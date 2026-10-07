//! Comparison operands of different value kinds are evaluation errors.
//! Lower only comparison calls after admission. CEL still owns evaluation order,
//! short-circuiting and comprehension bindings; these private names are not syntax.
use cel::common::ast::{EntryExpr, Expr, IdedExpr};
use cel::common::types::{CelBool, DYN_TYPE};
use cel::common::value::Val;
use cel::{Env, ExecutionError};
use std::borrow::Cow;

const CALLS: [(&str, &str); 6] = [
    ("_==_", "#comparison.eq"),
    ("_!=_", "#comparison.ne"),
    ("_<_", "#comparison.lt"),
    ("_<=_", "#comparison.le"),
    ("_>_", "#comparison.gt"),
    ("_>=_", "#comparison.ge"),
];

pub(crate) fn lower(term: &mut IdedExpr) {
    match &mut term.expr {
        Expr::Call(call) => {
            if let Some((_, name)) = CALLS.iter().find(|(op, _)| *op == call.func_name) {
                call.func_name = (*name).to_owned();
            }
            if let Some(target) = &mut call.target {
                lower(target);
            }
            for arg in &mut call.args {
                lower(arg);
            }
        }
        Expr::Select(select) => lower(&mut select.operand),
        Expr::List(list) => {
            for item in &mut list.elements {
                lower(item);
            }
        }
        Expr::Map(map) => lower_entries(&mut map.entries),
        Expr::Struct(value) => lower_entries(&mut value.entries),
        Expr::Comprehension(c) => {
            for term in [
                &mut c.iter_range,
                &mut c.accu_init,
                &mut c.loop_cond,
                &mut c.loop_step,
                &mut c.result,
            ] {
                lower(term);
            }
        }
        Expr::Ident(_) | Expr::Literal(_) | Expr::Unspecified => {}
    }
}
fn lower_entries(entries: &mut [cel::common::ast::IdedEntryExpr]) {
    for entry in entries {
        match &mut entry.expr {
            EntryExpr::MapEntry(e) => {
                lower(&mut e.key);
                lower(&mut e.value);
            }
            EntryExpr::StructField(e) => lower(&mut e.value),
        }
    }
}

fn compare<'a>(args: Vec<Cow<'a, dyn Val>>, op: usize) -> Result<Cow<'a, dyn Val>, ExecutionError> {
    let [left, right] = args.as_slice() else {
        return Err(ExecutionError::NoSuchOverload);
    };
    if left.get_type() != right.get_type() {
        return Err(ExecutionError::NoSuchOverload);
    }
    let result = match op {
        0 => left.eq(right),
        1 => left.ne(right),
        _ => {
            let order = left
                .as_comparer()
                .ok_or(ExecutionError::NoSuchOverload)?
                .compare(right.as_ref())
                .map_err(|_| unordered(left.as_ref(), right.as_ref()))?;
            match op {
                2 => order.is_lt(),
                3 => order.is_le(),
                4 => order.is_gt(),
                5 => order.is_ge(),
                _ => unreachable!(),
            }
        }
    };
    Ok(Cow::<dyn Val>::Owned(Box::new(CelBool::from(result))))
}

/// Two operands of one kind that have no order between them.
///
/// The kinds already agree here, so the pair has an ordering overload. Within the value
/// model the only such pair without an order holds a NaN `Float`. CEL does not order NaN:
/// the reference runtime answers "NaN values cannot be ordered" (cel-go `Double.Compare`),
/// and langdef "Ordering" requires `e1 <= e2` to equal `!(e1 > e2)`, which no Bool answer
/// satisfies for NaN. The pinned crate names the same fact `ValuesNotComparable` in its
/// own `max`/`min`. Its `Double` comparer instead reports `NoSuchOverload`, which says a
/// different thing: that the kinds have no comparison at all.
fn unordered(left: &dyn Val, right: &dyn Val) -> ExecutionError {
    match (cel::Value::try_from(left), cel::Value::try_from(right)) {
        (Ok(left), Ok(right)) => ExecutionError::ValuesNotComparable(left, right),
        (Err(error), _) | (_, Err(error)) => error,
    }
}

pub(crate) fn register(env: &mut Env) {
    let functions: [cel::common::functions::Function; 6] = [
        |args| compare(args, 0),
        |args| compare(args, 1),
        |args| compare(args, 2),
        |args| compare(args, 3),
        |args| compare(args, 4),
        |args| compare(args, 5),
    ];
    for ((_, name), function) in CALLS.into_iter().zip(functions) {
        env.add_overload(name, name, vec![DYN_TYPE, DYN_TYPE], function)
            .expect("private comparison overloads are distinct");
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        EvalMode,
        depth::parse_bounded,
        eval::{EvalError, evaluate},
    };
    use circular_core::Value;

    /// If this breaks, NaN is again read as a Bool by an ordering operator, or its
    /// failure again says "no overload for these kinds". Expected values are the cel-spec
    /// vectors `not_eq_double_nan` and `not_ne_double_nan` (comparisons.textproto), and
    /// cel-go's answer for ordering ("NaN values cannot be ordered").
    #[test]
    fn nan_is_unequal_to_itself_and_has_no_order() {
        use crate::eval::EvaluationFailure;
        let nan = [("event".to_owned(), Value::float(f64::NAN))].into();
        for (code, expected) in [("event == event", false), ("event != event", true)] {
            let term = parse_bounded(code).unwrap();
            assert_eq!(
                evaluate(&term, &nan, EvalMode::Predicate).unwrap(),
                Value::bool(expected),
                "{code}"
            );
        }
        for op in ["<", "<=", ">", ">="] {
            for code in [format!("event {op} 0.05"), format!("0.05 {op} event")] {
                let term = parse_bounded(&code).unwrap();
                assert_eq!(
                    evaluate(&term, &nan, EvalMode::Predicate),
                    Err(EvalError::Evaluation(
                        EvaluationFailure::ValuesNotComparable
                    )),
                    "{code}"
                );
            }
        }
    }

    #[test]
    fn mixed_kind_comparisons_fail_and_same_kind_comparisons_work() {
        for op in ["==", "!=", "<", "<=", ">", ">="] {
            let bindings = [("event".to_owned(), Value::uint(0))].into();
            let term = parse_bounded(&format!("event {op} 0")).unwrap();
            assert!(
                matches!(
                    evaluate(&term, &bindings, EvalMode::Predicate),
                    Err(EvalError::Evaluation(_))
                ),
                "{op}"
            );
        }
        for (code, expected) in [
            ("0u == 0u", true),
            ("0u != 0u", false),
            ("1 < 2", true),
            ("'a' != 'b'", true),
        ] {
            assert_eq!(
                evaluate(
                    &parse_bounded(code).unwrap(),
                    &Default::default(),
                    EvalMode::Predicate
                )
                .unwrap(),
                Value::bool(expected)
            );
        }
    }
    #[test]
    fn comparisons_keep_short_circuiting_and_comprehension_bindings() {
        for (code, expected) in [
            ("false && 0u != 0", false),
            ("true || 0u == 0", true),
            ("true ? true : 0u == 0", true),
            ("[0u, 1u].all(x, x >= 0u)", true),
        ] {
            assert_eq!(
                evaluate(
                    &parse_bounded(code).unwrap(),
                    &Default::default(),
                    EvalMode::Predicate
                )
                .unwrap(),
                Value::bool(expected),
                "{code}"
            );
        }
        assert!(
            evaluate(
                &parse_bounded("[0u].all(x, x != 0)").unwrap(),
                &Default::default(),
                EvalMode::Predicate
            )
            .is_err()
        );
    }
}
