
use std::collections::{BTreeMap, BTreeSet};

use cel::common::ast::{Expr, IdedExpr, operators};

use crate::walk::{Structure, structure};

const LANGUAGE_CORE_FUNCTIONS: [&str; 21] = [
    operators::CONDITIONAL,
    operators::LOGICAL_AND,
    operators::LOGICAL_OR,
    operators::LOGICAL_NOT,
    operators::SUBSTRACT,
    operators::ADD,
    operators::MULTIPLY,
    operators::DIVIDE,
    operators::MODULO,
    operators::EQUALS,
    operators::NOT_EQUALS,
    operators::GREATER_EQUALS,
    operators::LESS_EQUALS,
    operators::GREATER,
    operators::LESS,
    operators::NEGATE,
    operators::INDEX,
    operators::OPT_INDEX,
    operators::OPT_SELECT,
    operators::NOT_STRICTLY_FALSE,
    operators::IN,
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rejection {
    pub node: u64,
    pub reason: RejectionReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RejectionReason {
    UndeclaredIdentifier(String),
    UnopenedFunction(String),
    WrongArity {
        name: String,
        expected: Box<[usize]>,
        actual: usize,
    },
    StructLiteral,
}

impl std::fmt::Display for RejectionReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UndeclaredIdentifier(name) => {
                write!(formatter, "`{name}` is not a declared variable")
            }
            Self::UnopenedFunction(name) => {
                write!(
                    formatter,
                    "`{name}` is not an available function; available functions: "
                )?;
                write_names(
                    formatter,
                    crate::surface::PROFILE_SURFACE
                        .iter()
                        .map(|(name, _)| *name),
                )
            }
            Self::WrongArity {
                name,
                expected,
                actual,
            } => {
                write!(
                    formatter,
                    "`{name}` was called with {actual} argument(s) counting the receiver; it takes "
                )?;
                for (index, count) in expected.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(" or ")?;
                    }
                    write!(formatter, "{count}")?;
                }
                Ok(())
            }
            Self::StructLiteral => {
                formatter.write_str("struct literals are not available in expressions")
            }
        }
    }
}

pub(crate) fn write_names<'a>(
    formatter: &mut std::fmt::Formatter<'_>,
    names: impl IntoIterator<Item = &'a str>,
) -> std::fmt::Result {
    for (index, name) in names.into_iter().enumerate() {
        if index > 0 {
            formatter.write_str(", ")?;
        }
        write!(formatter, "`{name}`")?;
    }
    Ok(())
}

#[must_use]
pub fn free_identifiers(term: &IdedExpr) -> BTreeMap<String, u64> {
    collect_references(term).0
}

#[must_use]
pub fn called_functions(term: &IdedExpr) -> BTreeMap<String, u64> {
    let (_, calls, _) = collect_references(term);
    let mut functions = BTreeMap::new();
    for call in calls {
        functions.entry(call.name).or_insert(call.node);
    }
    functions
}

#[must_use]
pub fn check(
    term: &IdedExpr,
    declared: &BTreeSet<String>,
    surface: &BTreeMap<String, Box<[usize]>>,
) -> Vec<Rejection> {
    let (identifiers, calls, struct_literals) = collect_references(term);
    let mut rejections: Vec<Rejection> = identifiers
        .into_iter()
        .filter(|(name, _)| !declared.contains(name))
        .map(|(name, node)| Rejection {
            node,
            reason: RejectionReason::UndeclaredIdentifier(name),
        })
        .collect();
    for call in calls {
        match surface.get(&call.name) {
            None => rejections.push(Rejection {
                node: call.node,
                reason: RejectionReason::UnopenedFunction(call.name),
            }),
            Some(expected) if !expected.contains(&call.actual) => rejections.push(Rejection {
                node: call.node,
                reason: RejectionReason::WrongArity {
                    name: call.name,
                    expected: expected.clone(),
                    actual: call.actual,
                },
            }),
            Some(_) => {}
        }
    }
    rejections.extend(struct_literals.into_iter().map(|node| Rejection {
        node,
        reason: RejectionReason::StructLiteral,
    }));
    rejections.sort_by_key(|rejection| rejection.node);
    rejections
}

fn is_language_core(name: &str) -> bool {
    LANGUAGE_CORE_FUNCTIONS.contains(&name)
}

enum WalkEvent<'a> {
    FreeIdent(&'a str, u64),
    Call {
        name: &'a str,
        node: u64,
        actual: usize,
    },
    StructLiteral(u64),
}

struct CallReference {
    name: String,
    node: u64,
    actual: usize,
}

fn collect_references(term: &IdedExpr) -> (BTreeMap<String, u64>, Vec<CallReference>, Vec<u64>) {
    let mut identifiers = BTreeMap::new();
    let mut calls = Vec::new();
    let mut struct_literals = Vec::new();
    let mut bound = Vec::new();
    walk_scoped(term, &mut bound, &mut |event| match event {
        WalkEvent::FreeIdent(name, node) => {
            identifiers.entry(name.to_owned()).or_insert(node);
        }
        WalkEvent::Call { name, node, actual } => calls.push(CallReference {
            name: name.to_owned(),
            node,
            actual,
        }),
        WalkEvent::StructLiteral(node) => struct_literals.push(node),
    });
    (identifiers, calls, struct_literals)
}

fn walk_scoped<'a>(
    term: &'a IdedExpr,
    bound: &mut Vec<&'a str>,
    visit: &mut impl FnMut(WalkEvent<'a>),
) {
    match &term.expr {
        Expr::Ident(name) => {
            if !bound.contains(&name.as_str()) {
                visit(WalkEvent::FreeIdent(name, term.id));
            }
        }
        Expr::Call(call) if !is_language_core(&call.func_name) => {
            visit(WalkEvent::Call {
                name: &call.func_name,
                node: term.id,
                actual: call.args.len() + usize::from(call.target.is_some()),
            });
        }
        Expr::Struct(_) => visit(WalkEvent::StructLiteral(term.id)),
        _ => {}
    }

    match structure(term) {
        Structure::Leaf => {}
        Structure::Node(children) => {
            for child in children {
                walk_scoped(child, bound, visit);
            }
        }
        Structure::Comprehension {
            iter_range,
            accu_init,
            loop_cond,
            loop_step,
            result,
            iter_var,
            iter_var2,
            accu_var,
        } => {
            walk_scoped(iter_range, bound, visit);
            walk_scoped(accu_init, bound, visit);

            let depth = bound.len();
            bound.push(iter_var);
            if let Some(second) = iter_var2 {
                bound.push(second);
            }
            bound.push(accu_var);

            walk_scoped(loop_cond, bound, visit);
            walk_scoped(loop_step, bound, visit);
            walk_scoped(result, bound, visit);

            bound.truncate(depth);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        LANGUAGE_CORE_FUNCTIONS, RejectionReason, called_functions, check, free_identifiers,
    };
    use crate::surface::PROFILE_SURFACE;
    use cel::common::ast::{CallExpr, Expr, IdedExpr};
    use cel::parser::Parser;
    use std::collections::{BTreeMap, BTreeSet};

    fn free(source: &str) -> Vec<String> {
        let term = Parser::new().parse(source).expect("parse");
        free_identifiers(&term).into_keys().collect()
    }

    fn calls(source: &str) -> Vec<String> {
        let term = Parser::new().parse(source).expect("parse");
        called_functions(&term).into_keys().collect()
    }

    fn names(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    fn surface(items: &[(&str, &[usize])]) -> BTreeMap<String, Box<[usize]>> {
        items
            .iter()
            .map(|(name, arities)| ((*name).to_owned(), Box::from(*arities)))
            .collect()
    }

    #[test]
    fn plain_identifiers_are_free() {
        assert_eq!(free("a"), ["a"]);
        assert_eq!(free("a + b"), ["a", "b"]);
        assert_eq!(free("a.b.c"), ["a"], "a field name is not an identifier");
        assert_eq!(free("m['k']"), ["m"], "a string key is not an identifier");
        assert_eq!(free("1 + 2"), Vec::<String>::new());
    }

    #[test]
    fn binders_do_not_escape_their_comprehension() {
        assert_eq!(free("l.map(x, x) + [x]"), ["l", "x"]);
        assert_eq!(free("l.map(x, x.map(y, y + x))"), ["l"]);
    }

    #[test]
    fn range_and_seed_are_evaluated_outside() {
        assert_eq!(free("x.map(x, x)"), ["x"]);
    }

    #[test]
    fn free_names_from_the_environment_survive() {
        assert_eq!(free("l.map(x, x * scale)"), ["l", "scale"]);
    }

    #[test]
    fn operators_and_macro_internals_are_not_functions() {
        assert_eq!(calls("a + b"), Vec::<String>::new());
        assert_eq!(calls("a in l"), Vec::<String>::new());
        assert_eq!(calls("a ? b : c"), Vec::<String>::new());
        assert_eq!(calls("!a"), Vec::<String>::new());
        assert_eq!(calls("a[0]"), Vec::<String>::new());
        assert_eq!(calls("l.all(x, x > 0)"), Vec::<String>::new());
    }

    #[test]
    fn profile_functions_are_collected_in_both_call_forms() {
        assert_eq!(calls("size(s)"), ["size"]);
        assert_eq!(calls("s.size()"), ["size"]);
        assert_eq!(calls("f(g(a))"), ["f", "g"]);
    }

    #[test]
    fn undeclared_identifiers_are_rejected() {
        let term = Parser::new()
            .parse("event.value > threshold")
            .expect("parse");
        let rejections = check(&term, &names(&["event"]), &surface(&[]));
        assert_eq!(rejections.len(), 1);
        assert_eq!(
            rejections[0].reason,
            RejectionReason::UndeclaredIdentifier("threshold".to_owned())
        );
    }

    #[test]
    fn unopened_functions_are_rejected() {
        let term = Parser::new()
            .parse("size(event) + f(event)")
            .expect("parse");
        let rejections = check(&term, &names(&["event"]), &surface(&[("size", &[1])]));
        assert_eq!(rejections.len(), 1);
        assert_eq!(
            rejections[0].reason,
            RejectionReason::UnopenedFunction("f".to_owned())
        );
    }

    #[test]
    fn underscore_shaped_author_names_are_rejected() {
        for name in ["_foo", "foo_", "_"] {
            let term = Parser::new()
                .parse(&format!("{name}()"))
                .unwrap_or_else(|error| panic!("{name} did not parse: {error}"));
            assert_eq!(
                check(&term, &names(&[]), &surface(&[]))
                    .into_iter()
                    .map(|rejection| rejection.reason)
                    .collect::<Vec<_>>(),
                [RejectionReason::UnopenedFunction(name.to_owned())],
                "{name}"
            );
        }
    }

    #[test]
    fn every_pinned_cel_core_call_name_passes() {
        for (id, name) in LANGUAGE_CORE_FUNCTIONS.iter().enumerate() {
            let term = IdedExpr {
                id: id as u64,
                expr: Expr::Call(CallExpr {
                    func_name: (*name).to_owned(),
                    target: None,
                    args: Vec::new(),
                }),
            };
            assert_eq!(check(&term, &names(&[]), &surface(&[])), [], "{name}");
        }
    }

    #[test]
    fn every_profile_function_accepts_both_call_forms_at_its_declared_arity() {
        let profile = surface(&PROFILE_SURFACE);
        let declared = names(&["a"]);

        for (name, arities) in PROFILE_SURFACE {
            for arity in arities {
                let global_args = vec!["a"; *arity].join(", ");
                let global = Parser::new()
                    .parse(&format!("{name}({global_args})"))
                    .unwrap_or_else(|error| panic!("{name} global form failed to parse: {error}"));
                assert_eq!(
                    check(&global, &declared, &profile),
                    [],
                    "{name} global form"
                );

                let member_args = vec!["a"; arity - 1].join(", ");
                let member = Parser::new()
                    .parse(&format!("a.{name}({member_args})"))
                    .unwrap_or_else(|error| {
                        panic!("{name} receiver form failed to parse: {error}")
                    });
                assert_eq!(
                    check(&member, &declared, &profile),
                    [],
                    "{name} receiver form"
                );
            }
        }
    }

    #[test]
    fn every_profile_function_rejects_one_fewer_and_one_more_argument() {
        let profile = surface(&PROFILE_SURFACE);
        let declared = names(&["a"]);

        for (name, arities) in PROFILE_SURFACE {
            assert_eq!(
                arities.len(),
                1,
                "this test's boundary value must be updated: {name}"
            );
            let expected = Box::<[usize]>::from(arities);
            for actual in [arities[0] - 1, arities[0] + 1] {
                let args = vec!["a"; actual].join(", ");
                let term = Parser::new()
                    .parse(&format!("{name}({args})"))
                    .unwrap_or_else(|error| panic!("{name}/{actual} failed to parse: {error}"));
                assert_eq!(
                    check(&term, &declared, &profile)
                        .into_iter()
                        .map(|rejection| rejection.reason)
                        .collect::<Vec<_>>(),
                    [RejectionReason::WrongArity {
                        name: name.to_owned(),
                        expected: expected.clone(),
                        actual,
                    }],
                    "{name}/{actual}"
                );
            }
        }
    }

    #[test]
    fn receiver_calls_count_the_receiver_as_an_argument() {
        let profile = surface(&PROFILE_SURFACE);
        let declared = names(&["a"]);

        let correct = Parser::new()
            .parse("a.startsWith(a)")
            .expect("parse of the correct receiver form");
        assert_eq!(check(&correct, &declared, &profile), []);

        let too_few = Parser::new()
            .parse("a.startsWith()")
            .expect("parse of a receiver form with too few arguments");
        assert!(matches!(
            check(&too_few, &declared, &profile).as_slice(),
            [super::Rejection {
                reason: RejectionReason::WrongArity {
                    name,
                    expected,
                    actual: 1,
                },
                ..
            }] if name == "startsWith" && expected.as_ref() == [2]
        ));
    }

    #[test]
    fn every_call_site_is_checked_for_arity() {
        let term = Parser::new()
            .parse("size(a) + size()")
            .expect("parse of two calls with the same name");
        let rejections = check(&term, &names(&["a"]), &surface(&[("size", &[1])]));
        assert!(matches!(
            rejections.as_slice(),
            [super::Rejection {
                reason: RejectionReason::WrongArity { actual: 0, .. },
                ..
            }]
        ));
    }

    #[test]
    fn a_term_using_macros_passes_with_only_its_real_environment() {
        let term = Parser::new()
            .parse("event.items.all(x, x.size() > 0)")
            .expect("parse");
        assert_eq!(
            check(&term, &names(&["event"]), &surface(&[("size", &[1])])),
            []
        );
    }

    #[test]
    fn rejections_are_ordered_by_node() {
        let term = Parser::new().parse("a + b + c").expect("parse");
        let rejections = check(&term, &names(&[]), &surface(&[]));
        let nodes: Vec<_> = rejections.iter().map(|rejection| rejection.node).collect();
        let mut sorted = nodes.clone();
        sorted.sort_unstable();
        assert_eq!(nodes, sorted);
    }
}
