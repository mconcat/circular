
use crate::EvalMode;
use crate::checker::{Rejection, RejectionReason, check};
use crate::depth::{ParseRejection, parse_bounded};
use crate::eval::{EvalError, evaluate_unchecked};
use crate::printer::print;
use crate::shapes::{ShapeEnv, shape_of};
use crate::surface::SURFACE_NAMES;
use cel::common::ast::IdedExpr;
use circular_core::{BaseShape, Shape};
use circular_core::{Value, ValueKind};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug)]
pub struct Snippet {
    canonical: String,
    term: IdedExpr,
    mode: EvalMode,
}

impl Snippet {
    #[must_use]
    pub fn canonical_text(&self) -> &str {
        &self.canonical
    }

    #[must_use]
    pub const fn mode(&self) -> EvalMode {
        self.mode
    }

    #[must_use]
    pub fn output_shape(&self, env: &ShapeEnv) -> Shape<String> {
        shape_of(&self.term, env)
    }

    #[must_use]
    pub fn kind_split(&self, env: &ShapeEnv) -> Option<crate::shapes::KindSplit> {
        crate::shapes::kind_split(&self.term, env)
    }

    /// Whether the admitted term references a free input binding. Macro-local
    /// binders are excluded by the same collector used at admission.
    #[must_use]
    pub fn references_input(&self, name: &str) -> bool {
        crate::checker::free_identifiers(&self.term).contains_key(name)
    }

    pub fn evaluate(&self, bindings: &BTreeMap<String, Value>) -> Result<Value, EvalError> {
        evaluate_unchecked(&self.term, bindings, self.mode)
    }
}

#[derive(Debug)]
pub enum AcceptError {
    NotText {
        got: ValueKind,
    },
    NotATerm(ParseRejection),
    NotInScope(Vec<Rejection>, Box<[String]>),
    /// The statically known result cannot inhabit the requested evaluation mode.
    ModeMismatch { mode: EvalMode, got: Shape<String> },
}

impl std::fmt::Display for AcceptError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ConfigRejected: ")?;
        match self {
            Self::NotText { got } => write!(
                formatter,
                "an expression position takes one CEL source string; got {got:?}"
            ),
            Self::NotATerm(rejection) => write!(formatter, "{rejection}"),
            Self::ModeMismatch { mode, got } => write!(
                formatter,
                "{} requires a compatible result type; expression produces {got:?}",
                mode.as_str()
            ),
            Self::NotInScope(rejections, declared) => {
                let Some(first) = rejections.first() else {
                    return formatter.write_str(
                        "the expression uses a name outside its declared variables and available functions",
                    );
                };
                write!(formatter, "{}", first.reason)?;
                if matches!(first.reason, RejectionReason::UndeclaredIdentifier(_)) {
                    if declared.is_empty() {
                        formatter.write_str("; this expression declares no variables")?;
                    } else {
                        formatter.write_str("; available variables: ")?;
                        crate::checker::write_names(
                            formatter,
                            declared.iter().map(String::as_str),
                        )?;
                    }
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for AcceptError {}

pub fn from_config_value(
    value: &Value,
    declared: &BTreeSet<String>,
    mode: EvalMode,
) -> Result<Snippet, AcceptError> {
    let Some(source) = value.as_str() else {
        return Err(AcceptError::NotText { got: value.kind() });
    };
    accept(source, declared, mode)
}

pub fn accept(
    source: &str,
    declared: &BTreeSet<String>,
    mode: EvalMode,
) -> Result<Snippet, AcceptError> {
    let term = parse_bounded(source).map_err(AcceptError::NotATerm)?;

    let rejections = check(&term, declared, &SURFACE_NAMES);
    if !rejections.is_empty() {
        return Err(AcceptError::NotInScope(
            rejections,
            declared.iter().cloned().collect(),
        ));
    }

    let got = shape_of(&term, &ShapeEnv::new());
    let compatible = match mode {
        EvalMode::Predicate => matches!(got, Shape::Any | Shape::Base(BaseShape::Bool)),
        EvalMode::Number => matches!(
            got,
            Shape::Any | Shape::Base(BaseShape::Int | BaseShape::UInt | BaseShape::Float)
        ),
        EvalMode::Transform | EvalMode::Reduce => true,
    };
    if !compatible {
        return Err(AcceptError::ModeMismatch { mode, got });
    }

    let canonical = print(&term).expect("a parsed term that passed admission has canonical text");
    Ok(Snippet {
        canonical,
        term,
        mode,
    })
}

#[cfg(test)]
mod tests {
    use super::{AcceptError, accept, from_config_value};
    use crate::EvalMode;
    use circular_core::{Value, ValueKind};
    use std::collections::{BTreeMap, BTreeSet};

    fn declared() -> BTreeSet<String> {
        ["event".to_owned(), "items".to_owned()]
            .into_iter()
            .collect()
    }

    fn event(n: i64) -> BTreeMap<String, Value> {
        [(
            "event".to_owned(),
            Value::object([("n", Value::int(n)), ("kind", Value::string("tick"))]).unwrap(),
        )]
        .into_iter()
        .collect()
    }

    #[test]
    fn a_config_string_becomes_a_running_term() {
        let stored = Value::string("event.n > 0");
        let snippet =
            from_config_value(&stored, &declared(), EvalMode::Predicate).expect("accepted");

        assert_eq!(snippet.evaluate(&event(3)), Ok(Value::bool(true)));
        assert_eq!(snippet.evaluate(&event(-1)), Ok(Value::bool(false)));
    }

    #[test]
    fn spellings_collapse_to_one_stored_text() {
        let spellings = ["event.n>0", "event.n > 0", "((event.n) > (0))"];
        let texts: Vec<String> = spellings
            .iter()
            .map(|source| {
                accept(source, &declared(), EvalMode::Predicate)
                    .expect(source)
                    .canonical_text()
                    .to_owned()
            })
            .collect();

        assert!(
            texts.windows(2).all(|pair| pair[0] == pair[1]),
            "the spellings did not fold: {texts:?}"
        );
    }

    #[test]
    fn the_stored_text_is_a_fixed_point() {
        let once = accept("event.n>0", &declared(), EvalMode::Predicate).expect("accepted");
        let twice =
            accept(once.canonical_text(), &declared(), EvalMode::Predicate).expect("re-admit");

        assert_eq!(once.canonical_text(), twice.canonical_text());
    }

    #[test]
    fn admission_diagnostics_are_english_and_name_what_is_available() {
        let message = |source: &str, declared: &[&str]| {
            let declared = declared
                .iter()
                .map(|name| (*name).to_owned())
                .collect::<BTreeSet<_>>();
            accept(source, &declared, EvalMode::Predicate)
                .unwrap_err()
                .to_string()
        };
        assert_eq!(
            message("value.level == 'error'", &["event"]),
            "ConfigRejected: `value` is not a declared variable; available variables: `event`"
        );
        assert_eq!(
            message("x > 0", &[]),
            "ConfigRejected: `x` is not a declared variable; this expression declares no variables"
        );
        assert_eq!(
            message("['a', 'b'].join(',') == 'a,b'", &["event"]),
            "ConfigRejected: `join` is not an available function; available functions: \
             `size`, `startsWith`, `endsWith`, `contains`, `int`, `double`, `string`, `bytes`, `uint`"
        );
        assert_eq!(
            message("size(event, event) > 0", &["event"]),
            "ConfigRejected: `size` was called with 2 argument(s) counting the receiver; it takes 1"
        );
        assert_eq!(
            message("Message{field: 1}", &["event"]),
            "ConfigRejected: struct literals are not available in expressions"
        );
        assert_eq!(
            from_config_value(&Value::int(1), &declared(), EvalMode::Predicate)
                .unwrap_err()
                .to_string(),
            "ConfigRejected: an expression position takes one CEL source string; got Int"
        );
    }

    #[test]
    fn undeclared_names_are_caught_at_acceptance() {
        assert!(matches!(
            accept("stranger.n > 0", &declared(), EvalMode::Predicate),
            Err(AcceptError::NotInScope(..))
        ));
        assert!(matches!(
            accept(
                "event.kind.matches('t.*')",
                &declared(),
                EvalMode::Predicate
            ),
            Err(AcceptError::NotInScope(..))
        ));
    }

    #[test]
    fn struct_literals_are_rejected_at_acceptance() {
        let Err(AcceptError::NotInScope(rejections, _)) =
            accept("Message{field: 1}", &declared(), EvalMode::Transform)
        else {
            panic!("a struct literal must not pass admission");
        };
        assert_eq!(
            rejections
                .into_iter()
                .map(|rejection| rejection.reason)
                .collect::<Vec<_>>(),
            [crate::checker::RejectionReason::StructLiteral]
        );
    }

    #[test]
    fn the_depth_gates_stand_at_acceptance() {
        let lethal = format!("event{}", ".f".repeat(crate::depth::MAX_TERM_DEPTH + 8));
        assert!(matches!(
            accept(&lethal, &declared(), EvalMode::Predicate),
            Err(AcceptError::NotATerm(_))
        ));
    }

    #[test]
    fn a_transform_place_accepts_and_runs() {
        let stored = Value::string("{'doubled': event.n * 2, 'kind': event.kind}");
        let snippet =
            from_config_value(&stored, &declared(), EvalMode::Transform).expect("accepted");

        let value = snippet.evaluate(&event(21)).expect("evaluate");
        let object = value.as_object().expect("object");
        assert_eq!(object.get("doubled"), Some(&Value::int(42)));
    }

    #[test]
    fn the_place_carries_its_mode() {
        let snippet = accept("event.kind", &declared(), EvalMode::Transform).expect("accepted");
        assert_eq!(snippet.mode(), EvalMode::Transform);
        assert_eq!(snippet.evaluate(&event(1)), Ok(Value::string("tick")));
    }

    #[test]
    fn an_accepted_snippet_yields_its_output_shape() {
        use circular_core::{BaseShape, FieldMap, Shape};

        let snippet = accept(
            "{'doubled': event.n * 2, 'kind': event.kind}",
            &declared(),
            EvalMode::Transform,
        )
        .expect("accepted");

        let fields = FieldMap::try_new(vec![
            ("n".to_owned(), Shape::Base(BaseShape::Int)),
            ("kind".to_owned(), Shape::Base(BaseShape::String)),
        ])
        .expect("the field is unique");
        let env: crate::shapes::ShapeEnv = [(
            "event".to_owned(),
            Shape::Object {
                fields,
                open: false,
            },
        )]
        .into_iter()
        .collect();

        let Shape::Object { fields, open } = snippet.output_shape(&env) else {
            panic!("must be a closed object; with Any, the term output check refuses it");
        };
        assert!(!open);
        assert_eq!(
            fields.as_slice(),
            [
                ("doubled".to_owned(), Shape::Base(BaseShape::Int)),
                ("kind".to_owned(), Shape::Base(BaseShape::String)),
            ]
        );

        let empty = snippet.output_shape(&crate::shapes::ShapeEnv::new());
        let Shape::Object { fields, .. } = empty else {
            panic!("object");
        };
        assert_eq!(
            fields.as_slice()[0].1,
            Shape::Any,
            "if the environment does not know event, it does not know the arithmetic kind either"
        );
    }

    #[test]
    fn value_kind_is_reported_verbatim_on_rejection() {
        assert!(matches!(
            from_config_value(&Value::bool(true), &declared(), EvalMode::Predicate),
            Err(AcceptError::NotText {
                got: ValueKind::Bool
            })
        ));
    }
}

#[cfg(test)]
mod shape_input_dependency_tests {
    use super::*;

    #[test]
    fn references_input_respects_macro_binders_and_literal_keys() {
        let declared = BTreeSet::from(["event".to_owned()]);
        for (source, expected) in [
            ("event.n", true),
            ("{'event': 1}", false),
            ("[1, 2].map(event, event + 1)", false),
            ("[1, 2].map(x, x + event.n)", true),
        ] {
            let snippet = accept(source, &declared, EvalMode::Transform).unwrap();
            assert_eq!(snippet.references_input("event"), expected, "{source}");
        }
    }
}
