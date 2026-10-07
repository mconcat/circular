
use cel::common::ast::{EntryExpr, Expr, IdedExpr, LiteralValue};
use circular_core::{BaseShape, FieldMap, Shape};
use std::collections::BTreeMap;

pub type ShapeEnv = BTreeMap<String, Shape<String>>;

#[must_use]
pub fn merge(left: Shape<String>, right: Shape<String>) -> Shape<String> {
    if left == right { left } else { Shape::Any }
}

fn merge_all(mut shapes: impl Iterator<Item = Shape<String>>) -> Shape<String> {
    match shapes.next() {
        None => Shape::Any,
        Some(first) => shapes.fold(first, merge),
    }
}

#[must_use]
pub fn shape_of(term: &IdedExpr, env: &ShapeEnv) -> Shape<String> {
    match &term.expr {
        Expr::Ident(name) => env.get(name).cloned().unwrap_or(Shape::Any),

        Expr::Literal(literal) => literal_shape(literal),

        Expr::List(list) => Shape::Array(Box::new(merge_all(
            list.elements.iter().map(|item| shape_of(item, env)),
        ))),

        Expr::Map(map) => static_object(&map.entries, env),
        Expr::Struct(node) => static_object(&node.entries, env),

        Expr::Select(select) if select.test => Shape::Base(BaseShape::Bool),

        Expr::Select(select) => match shape_of(&select.operand, env) {
            Shape::Object { fields, .. } => fields
                .as_slice()
                .iter()
                .find(|(name, _)| *name == select.field)
                .map_or(Shape::Any, |(_, shape)| shape.clone()),
            _ => Shape::Any,
        },

        Expr::Call(call) => call_shape(call, env),

        Expr::Comprehension(node) => comprehension_shape(node, env),

        Expr::Unspecified => Shape::Any,
    }
}

fn literal_shape(literal: &LiteralValue) -> Shape<String> {
    match literal {
        LiteralValue::Null => Shape::Base(BaseShape::Null),
        LiteralValue::Boolean(_) => Shape::Base(BaseShape::Bool),
        LiteralValue::Int(_) => Shape::Base(BaseShape::Int),
        LiteralValue::Double(_) => Shape::Base(BaseShape::Float),
        LiteralValue::String(_) => Shape::Base(BaseShape::String),
        LiteralValue::Bytes(_) => Shape::Base(BaseShape::Bytes),
        LiteralValue::UInt(_) => Shape::Base(BaseShape::UInt),
    }
}

fn static_object(entries: &[cel::common::ast::IdedEntryExpr], env: &ShapeEnv) -> Shape<String> {
    let mut fields = Vec::with_capacity(entries.len());
    for entry in entries {
        match &entry.expr {
            EntryExpr::MapEntry(entry) => {
                let Expr::Literal(LiteralValue::String(key)) = &entry.key.expr else {
                    return Shape::Any;
                };
                fields.push((key.inner().to_owned(), shape_of(&entry.value, env)));
            }
            EntryExpr::StructField(field) => {
                fields.push((field.field.clone(), shape_of(&field.value, env)));
            }
        }
    }
    FieldMap::try_new(fields).map_or(Shape::Any, |fields| Shape::Object {
        fields,
        open: false,
    })
}

fn call_shape(call: &cel::common::ast::CallExpr, env: &ShapeEnv) -> Shape<String> {
    let args: Vec<Shape<String>> = call
        .target
        .iter()
        .map(|target| shape_of(target, env))
        .chain(call.args.iter().map(|arg| shape_of(arg, env)))
        .collect();

    match (call.func_name.as_str(), args.len()) {
        ("_==_" | "_!=_" | "_<_" | "_<=_" | "_>_" | "_>=_" | "@in", 2)
        | ("_&&_" | "_||_", 2)
        | ("!_", 1) => Shape::Base(BaseShape::Bool),

        ("_?_:_", 3) => merge(args[1].clone(), args[2].clone()),

        ("_+_" | "_-_" | "_*_" | "_/_" | "_%_", 2) => merge(args[0].clone(), args[1].clone()),

        ("-_", 1) => args[0].clone(),

        ("_[_]", 2) => match &args[0] {
            Shape::Array(item) => item.as_ref().clone(),
            _ => Shape::Any,
        },

        ("size", 1 | 2) => Shape::Base(BaseShape::Int),
        ("startsWith" | "endsWith" | "contains", 2) => Shape::Base(BaseShape::Bool),
        ("int", 1) => Shape::Base(BaseShape::Int),
        ("uint", 1) => Shape::Base(BaseShape::UInt),
        ("double", 1) => Shape::Base(BaseShape::Float),
        ("string", 1) => Shape::Base(BaseShape::String),
        ("bytes", 1) => Shape::Base(BaseShape::Bytes),

        _ => Shape::Any,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KindSplit {
    pub operator: &'static str,
    pub left: BaseShape,
    pub right: BaseShape,
}

impl std::fmt::Display for KindSplit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} has no overload for {} and {}",
            self.operator, self.left, self.right
        )
    }
}

const HOMOGENEOUS_BINARY: [&str; 11] = [
    "_+_", "_-_", "_*_", "_/_", "_%_", "_<_", "_<=_", "_>_", "_>=_", "_==_", "_!=_",
];

#[must_use]
pub fn kind_split(term: &IdedExpr, env: &ShapeEnv) -> Option<KindSplit> {
    if let Expr::Call(call) = &term.expr
        && call.target.is_none()
        && call.args.len() == 2
        && let Some(operator) = HOMOGENEOUS_BINARY
            .iter()
            .find(|name| **name == call.func_name.as_str())
        && let Shape::Base(left) = shape_of(&call.args[0], env)
        && let Shape::Base(right) = shape_of(&call.args[1], env)
        && left != right
    {
        return Some(KindSplit {
            operator,
            left,
            right,
        });
    }
    match crate::walk::structure(term) {
        crate::walk::Structure::Leaf | crate::walk::Structure::Comprehension { .. } => None,
        crate::walk::Structure::Node(children) => children
            .into_iter()
            .find_map(|child| kind_split(child, env)),
    }
}

fn comprehension_shape(
    node: &cel::common::ast::ComprehensionExpr,
    env: &ShapeEnv,
) -> Shape<String> {
    match shape_of(&node.accu_init, env) {
        Shape::Base(BaseShape::Bool) | Shape::Base(BaseShape::Int) => {
            Shape::Base(BaseShape::Bool)
        }
        _ => Shape::Array(Box::new(Shape::Any)),
    }
}

#[cfg(test)]
mod tests {
    use super::{ShapeEnv, merge, shape_of};
    use crate::depth::parse_bounded;
    use circular_core::{BaseShape, Shape};

    fn shape(source: &str, env: &ShapeEnv) -> Shape<String> {
        shape_of(&parse_bounded(source).expect(source), env)
    }

    fn event_env() -> ShapeEnv {
        let fields = circular_core::FieldMap::try_new(vec![
            ("n".to_owned(), Shape::Base(BaseShape::Int)),
            ("kind".to_owned(), Shape::Base(BaseShape::String)),
        ])
        .expect("the field name is unique");
        [(
            "event".to_owned(),
            Shape::Object {
                fields,
                open: false,
            },
        )]
        .into_iter()
        .collect()
    }

    #[test]
    fn the_environment_key_is_the_inlet_id_itself() {
        let int = Shape::Base(BaseShape::Int);
        let named =
            |key: &str| -> ShapeEnv { [(key.to_owned(), int.clone())].into_iter().collect() };

        assert_eq!(shape("event", &named("event")), int);
        assert_eq!(shape("event", &named("input")), Shape::Any);
    }

    #[test]
    fn merging_prefers_soundness_over_precision() {
        let int = Shape::Base(BaseShape::Int);
        let float = Shape::Base(BaseShape::Float);

        assert_eq!(merge(int.clone(), int.clone()), int);
        assert_eq!(merge(int, float), Shape::Any);
    }

    #[test]
    fn a_static_map_literal_is_a_closed_object() {
        let Shape::Object { fields, open } = shape("{'n': 1, 'kind': 'tick'}", &ShapeEnv::new())
        else {
            panic!("must be an object");
        };
        assert!(!open, "no execution path adds more keys to the value");
        assert_eq!(
            fields.as_slice(),
            [
                ("n".to_owned(), Shape::Base(BaseShape::Int)),
                ("kind".to_owned(), Shape::Base(BaseShape::String)),
            ]
        );
    }

    #[test]
    fn a_dynamic_key_collapses_the_object_to_any() {
        let env = event_env();
        assert_eq!(shape("{event.kind: 1}", &env), Shape::Any);
    }

    #[test]
    fn a_static_field_select_reads_the_environment() {
        let env = event_env();
        assert_eq!(shape("event.n", &env), Shape::Base(BaseShape::Int));
        assert_eq!(shape("event.kind", &env), Shape::Base(BaseShape::String));
        assert_eq!(
            shape("event.missing", &env),
            Shape::Any,
            "an unknown field is Any"
        );
    }

    #[test]
    fn arithmetic_preserves_the_kind_and_mixing_collapses() {
        let env = event_env();
        assert_eq!(shape("1 + 2", &env), Shape::Base(BaseShape::Int));
        assert_eq!(shape("1.0 * 2.0", &env), Shape::Base(BaseShape::Float));
        assert_eq!(
            shape("1 + 2.0", &env),
            Shape::Any,
            "differing kinds give Any"
        );
    }

    #[test]
    fn a_conditional_merges_its_branches() {
        let env = event_env();
        assert_eq!(
            shape("event.n > 0 ? 1 : 2", &env),
            Shape::Base(BaseShape::Int)
        );
        assert_eq!(
            shape("event.n > 0 ? 1 : 'x'", &env),
            Shape::Any,
            "differing branches give Any"
        );
    }

    #[test]
    fn a_list_is_an_array_of_the_merged_element() {
        let env = event_env();
        assert_eq!(
            shape("[1, 2]", &env),
            Shape::Array(Box::new(Shape::Base(BaseShape::Int)))
        );
        assert_eq!(
            shape("[1, 'x']", &env),
            Shape::Array(Box::new(Shape::Any)),
            "differing elements give an Any element shape"
        );
        assert_eq!(
            shape("[]", &env),
            Shape::Array(Box::new(Shape::Any)),
            "merge of an empty sequence is Any"
        );
    }

    #[test]
    fn the_open_surface_function_carries_its_shape() {
        let env = event_env();
        assert_eq!(shape("size(event.kind)", &env), Shape::Base(BaseShape::Int));
    }

    #[test]
    fn predicate_macros_yield_a_bool() {
        let env = ShapeEnv::new();
        assert_eq!(
            shape("[1,2].all(x, x > 0)", &env),
            Shape::Base(BaseShape::Bool)
        );
        assert_eq!(
            shape("[1,2].exists(x, x > 0)", &env),
            Shape::Base(BaseShape::Bool)
        );
    }
}
