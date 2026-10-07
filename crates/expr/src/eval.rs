
use crate::EvalMode;
use crate::bridge::{LowerError, lift, lower};
use crate::checker::{Rejection, check};
use crate::surface::{PROFILE_ENV, SURFACE_NAMES};
use cel::common::ast::IdedExpr;
use cel::{Context, Value as CelValue};
use circular_core::{Value, ValueKind};
use std::collections::{BTreeMap, BTreeSet};

circular_core::closed_table! {
    pub enum EvaluationFailure {
        NoSuchKey => "no_such_key",
        NoSuchOverload => "no_such_overload",
        DivisionByZero => "division_by_zero",
        RemainderByZero => "remainder_by_zero",
        Overflow => "overflow",
        UndeclaredReference => "undeclared_reference",
        InvalidArgumentCount => "invalid_argument_count",
        UnsupportedTargetType => "unsupported_target_type",
        NotSupportedAsMethod => "not_supported_as_method",
        UnsupportedKeyType => "unsupported_key_type",
        UnexpectedType => "unexpected_type",
        MissingArgumentOrTarget => "missing_argument_or_target",
        ValuesNotComparable => "values_not_comparable",
        UnsupportedUnaryOperator => "unsupported_unary_operator",
        UnsupportedBinaryOperator => "unsupported_binary_operator",
        UnsupportedMapIndex => "unsupported_map_index",
        UnsupportedListIndex => "unsupported_list_index",
        UnsupportedIndex => "unsupported_index",
        UnsupportedFunctionCallIdentifierType => "unsupported_function_call_identifier_type",
        UnsupportedFieldsConstruction => "unsupported_fields_construction",
        FunctionError => "function_error",
        IndexOutOfBounds => "index_out_of_bounds",
        InternalError => "internal_error",
        Unclassified => "unclassified",
    }
}

impl EvaluationFailure {
    #[allow(deprecated)]
    fn classify(error: &cel::ExecutionError) -> Self {
        use cel::ExecutionError as E;
        match error {
            E::NoSuchKey(_) => Self::NoSuchKey,
            E::NoSuchOverload => Self::NoSuchOverload,
            E::DivisionByZero(_) => Self::DivisionByZero,
            E::RemainderByZero(_) => Self::RemainderByZero,
            E::Overflow(..) => Self::Overflow,
            E::UndeclaredReference(_) => Self::UndeclaredReference,
            E::InvalidArgumentCount { .. } => Self::InvalidArgumentCount,
            E::UnsupportedTargetType { .. } => Self::UnsupportedTargetType,
            E::NotSupportedAsMethod { .. } => Self::NotSupportedAsMethod,
            E::UnsupportedKeyType(_) => Self::UnsupportedKeyType,
            E::UnexpectedType { .. } => Self::UnexpectedType,
            E::MissingArgumentOrTarget => Self::MissingArgumentOrTarget,
            E::ValuesNotComparable(..) => Self::ValuesNotComparable,
            E::UnsupportedUnaryOperator(..) => Self::UnsupportedUnaryOperator,
            E::UnsupportedBinaryOperator(..) => Self::UnsupportedBinaryOperator,
            E::UnsupportedMapIndex(_) => Self::UnsupportedMapIndex,
            E::UnsupportedListIndex(_) => Self::UnsupportedListIndex,
            E::UnsupportedIndex(..) => Self::UnsupportedIndex,
            E::UnsupportedFunctionCallIdentifierType(_) => {
                Self::UnsupportedFunctionCallIdentifierType
            }
            E::UnsupportedFieldsConstruction(_) => Self::UnsupportedFieldsConstruction,
            E::FunctionError { .. } => Self::FunctionError,
            E::IndexOutOfBounds(_) => Self::IndexOutOfBounds,
            E::InternalError(_) => Self::InternalError,
            _ => Self::Unclassified,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum EvalError {
    NotAccepted(Vec<Rejection>),
    Evaluation(EvaluationFailure),
    Lower(LowerError),
    ModeMismatch {
        mode: EvalMode,
        got: ValueKind,
    },
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAccepted(rejections) => {
                write!(
                    formatter,
                    "a term that the admission boundary should have stopped: {rejections:?}"
                )
            }
            Self::Evaluation(failure) => write!(formatter, "evaluation failed: {failure}"),
            Self::Lower(error) => write!(formatter, "{error}"),
            Self::ModeMismatch { mode, got } => write!(
                formatter,
                "the {} field does not take {got:?}; refused rather than coerced",
                mode.as_str()
            ),
        }
    }
}

impl std::error::Error for EvalError {}

pub fn evaluate(
    term: &IdedExpr,
    bindings: &BTreeMap<String, Value>,
    mode: EvalMode,
) -> Result<Value, EvalError> {
    let declared: BTreeSet<String> = bindings.keys().cloned().collect();
    let rejections = check(term, &declared, &SURFACE_NAMES);
    if !rejections.is_empty() {
        return Err(EvalError::NotAccepted(rejections));
    }

    evaluate_unchecked(term, bindings, mode)
}

pub(crate) fn evaluate_unchecked(
    term: &IdedExpr,
    bindings: &BTreeMap<String, Value>,
    mode: EvalMode,
) -> Result<Value, EvalError> {
    let mut context = Context::with_env(PROFILE_ENV.clone());
    for (name, value) in bindings {
        context.add_variable_from_value(name.clone(), lift(value));
    }
    let mut lowered = term.clone();
    crate::comparison::lower(&mut lowered);
    let produced = CelValue::resolve(&lowered, &context)
        .map_err(|error| EvalError::Evaluation(EvaluationFailure::classify(&error)))?;

    let value = lower(&produced).map_err(EvalError::Lower)?;

    accept_in_mode(value, mode)
}

fn accept_in_mode(value: Value, mode: EvalMode) -> Result<Value, EvalError> {
    match mode {
        EvalMode::Predicate => match value.kind() {
            ValueKind::Bool => Ok(value),
            got => Err(EvalError::ModeMismatch { mode, got }),
        },
        EvalMode::Transform => Ok(value),
        EvalMode::Number => match value.kind() {
            ValueKind::Int | ValueKind::UInt | ValueKind::Float => Ok(value),
            got => Err(EvalError::ModeMismatch { mode, got }),
        },
        EvalMode::Reduce => Ok(value),
    }
}

#[cfg(test)]
mod tests {
    use super::{EvalError, EvaluationFailure, evaluate, evaluate_unchecked};
    use crate::EvalMode;
    use crate::depth::parse_bounded;
    use circular_core::{Value, ValueKind};
    use std::collections::BTreeMap;

    fn env(entries: &[(&str, Value)]) -> BTreeMap<String, Value> {
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect()
    }

    fn event(n: i64) -> BTreeMap<String, Value> {
        env(&[(
            "event",
            Value::object([("n", Value::int(n)), ("kind", Value::string("tick"))]).unwrap(),
        )])
    }

    #[test]
    fn a_predicate_snippet_runs() {
        let term = parse_bounded("event.n > 0").expect("accepted");
        assert_eq!(
            evaluate(&term, &event(3), EvalMode::Predicate),
            Ok(Value::bool(true))
        );
        assert_eq!(
            evaluate(&term, &event(-1), EvalMode::Predicate),
            Ok(Value::bool(false))
        );
    }

    #[test]
    fn a_transform_snippet_runs() {
        let term = parse_bounded("{'doubled': event.n * 2, 'kind': event.kind}").expect("accepted");
        let value = evaluate(&term, &event(21), EvalMode::Transform).expect("evaluate");

        let object = value.as_object().expect("object");
        assert_eq!(object.get("doubled"), Some(&Value::int(42)));
        assert_eq!(object.get("kind"), Some(&Value::string("tick")));
    }

    #[test]
    fn a_predicate_place_does_not_coerce() {
        let term = parse_bounded("event.kind").expect("accepted");
        assert_eq!(
            evaluate(&term, &event(1), EvalMode::Predicate),
            Err(EvalError::ModeMismatch {
                mode: EvalMode::Predicate,
                got: ValueKind::String
            })
        );

        assert_eq!(
            evaluate(&term, &event(1), EvalMode::Transform),
            Ok(Value::string("tick"))
        );
    }

    #[test]
    fn an_undeclared_name_is_charged_to_the_check_not_the_input() {
        let term = parse_bounded("stranger.n > 0").expect("parses");
        assert!(matches!(
            evaluate(&term, &event(1), EvalMode::Predicate),
            Err(EvalError::NotAccepted(_))
        ));
    }

    #[test]
    fn an_unopened_function_is_rejected_before_evaluation() {
        let term = parse_bounded("'x'.matches('y')").expect("parses");
        assert!(matches!(
            evaluate(&term, &event(1), EvalMode::Predicate),
            Err(EvalError::NotAccepted(_))
        ));
    }

    #[test]
    fn a_data_failure_is_charged_to_the_input() {
        let term = parse_bounded("event.missing > 0").expect("accepted");
        assert_eq!(
            evaluate(&term, &event(1), EvalMode::Predicate),
            Err(EvalError::Evaluation(EvaluationFailure::NoSuchKey))
        );
    }

    #[test]
    fn each_data_failure_kind_is_a_closed_arm() {
        let cases = [
            (
                "event.missing > 0",
                EvalMode::Predicate,
                EvaluationFailure::NoSuchKey,
            ),
            (
                "event.n < 1u",
                EvalMode::Predicate,
                EvaluationFailure::NoSuchOverload,
            ),
            (
                "event.n / 0",
                EvalMode::Transform,
                EvaluationFailure::DivisionByZero,
            ),
            (
                "event.n + 9223372036854775807",
                EvalMode::Transform,
                EvaluationFailure::Overflow,
            ),
            (
                "event.n % 0",
                EvalMode::Transform,
                EvaluationFailure::RemainderByZero,
            ),
        ];
        for (source, mode, expected) in cases {
            let term = parse_bounded(source).expect("accepted");
            assert_eq!(
                evaluate(&term, &event(1), mode),
                Err(EvalError::Evaluation(expected)),
                "{source}"
            );
        }
        let term = parse_bounded("event.n > 0").expect("accepted");
        assert_eq!(
            evaluate_unchecked(&term, &BTreeMap::new(), EvalMode::Predicate),
            Err(EvalError::Evaluation(
                EvaluationFailure::UndeclaredReference
            ))
        );
    }

    #[test]
    fn a_number_slot_takes_every_number_kind_and_coerces_none() {
        for (source, expected) in [
            ("event.n", Value::int(3)),
            ("event.n * 2", Value::int(6)),
            ("1.5", Value::float(1.5)),
            ("1u", Value::uint(1)),
        ] {
            let term = parse_bounded(source).expect("accepted");
            assert_eq!(
                evaluate(&term, &event(3), EvalMode::Number),
                Ok(expected),
                "{source}"
            );
        }

        for (source, got) in [
            ("event.kind", ValueKind::String),
            ("event.n > 0", ValueKind::Bool),
            ("[event.n]", ValueKind::Array),
        ] {
            let term = parse_bounded(source).expect("accepted");
            assert_eq!(
                evaluate(&term, &event(3), EvalMode::Number),
                Err(EvalError::ModeMismatch {
                    mode: EvalMode::Number,
                    got
                }),
                "{source}"
            );
        }
    }

    #[test]
    fn the_repaired_size_is_visible_through_the_entry_point() {
        let term = parse_bounded("size(event.kind)").expect("accepted");
        let bindings = env(&[(
            "event",
            Value::object([("kind", Value::string("€→✓"))]).unwrap(),
        )]);
        assert_eq!(
            evaluate(&term, &bindings, EvalMode::Transform),
            Ok(Value::int(3)),
            "3 code points, not 9 UTF-8 bytes"
        );
    }
}
