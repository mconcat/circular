
use crate::actor_registry::{ProductPayload, ProductValue};
use circular_expr::snippet::Snippet;

pub type FilterFailure = circular_expr::eval::EvalError;

pub fn filter_event(predicate: &Snippet, payload: &ProductPayload) -> Result<bool, FilterFailure> {
    evaluate_predicate(predicate, payload.value())
}

fn evaluate_predicate(predicate: &Snippet, value: &ProductValue) -> Result<bool, FilterFailure> {
    let bindings = std::collections::BTreeMap::from([("event".to_owned(), value.clone())]);
    let verdict = predicate.evaluate(&bindings)?;
    verdict.as_bool().ok_or(FilterFailure::ModeMismatch {
        mode: circular_expr::EvalMode::Predicate,
        got: verdict.kind(),
    })
}
