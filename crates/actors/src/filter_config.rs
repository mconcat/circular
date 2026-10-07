
use circular_core::Value;
use circular_expr::EvalMode;
use circular_expr::snippet::{self, Snippet};
use std::collections::BTreeSet;

pub const PREDICATE: &str = "predicate";

pub const EVENT_BINDING: &str = "event";

fn declared_bindings() -> BTreeSet<String> {
    BTreeSet::from([EVENT_BINDING.to_owned()])
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FilterConfigError {
    NotAnObject,
    MissingPredicate,
    Predicate(String),
}

impl core::fmt::Display for FilterConfigError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotAnObject => formatter.write_str("filter config root is not an object"),
            Self::MissingPredicate => {
                write!(formatter, "filter config is missing `{PREDICATE}`")
            }
            Self::Predicate(detail) => {
                let detail = detail.strip_prefix("ConfigRejected: ").unwrap_or(detail);
                write!(formatter, "predicate was rejected: {detail}")
            }
        }
    }
}

impl std::error::Error for FilterConfigError {}

pub fn accept_predicate(config: &Value) -> Result<Snippet, FilterConfigError> {
    let root = config.as_object().ok_or(FilterConfigError::NotAnObject)?;
    let value = root
        .get(PREDICATE)
        .ok_or(FilterConfigError::MissingPredicate)?;
    snippet::from_config_value(value, &declared_bindings(), EvalMode::Predicate)
        .map_err(|error| FilterConfigError::Predicate(error.to_string()))
}
