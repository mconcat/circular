
use circular_core::GroundShape;
use circular_core::Value;
use circular_expr::EvalMode;
use circular_expr::shapes::ShapeEnv;
use circular_expr::snippet::{self, Snippet};
use std::collections::BTreeSet;

use crate::{Name, Shape};

pub const TRANSFORM: &str = "transform";

pub const EVENT_BINDING: &str = "event";

fn declared_bindings() -> BTreeSet<String> {
    BTreeSet::from([EVENT_BINDING.to_owned()])
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MapConfigError {
    NotAnObject,
    MissingTransform,
    Transform(String),
}

impl core::fmt::Display for MapConfigError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotAnObject => formatter.write_str("map config root is not an object"),
            Self::MissingTransform => {
                write!(formatter, "map config is missing `{TRANSFORM}`")
            }
            Self::Transform(detail) => {
                let detail = detail.strip_prefix("ConfigRejected: ").unwrap_or(detail);
                write!(formatter, "transform was rejected: {detail}")
            }
        }
    }
}

impl std::error::Error for MapConfigError {}

pub fn accept_transform(config: &Value) -> Result<Snippet, MapConfigError> {
    let root = config.as_object().ok_or(MapConfigError::NotAnObject)?;
    let value = root
        .get(TRANSFORM)
        .ok_or(MapConfigError::MissingTransform)?;
    snippet::from_config_value(value, &declared_bindings(), EvalMode::Transform)
        .map_err(|error| MapConfigError::Transform(error.to_string()))
}

#[must_use]
pub fn shape_env(item: &Shape) -> ShapeEnv {
    ShapeEnv::from([(
        EVENT_BINDING.to_owned(),
        crate::types::to_unnamed_shape(item),
    )])
}

#[must_use]
pub fn output_shape_of(transform: &Snippet, item: &Shape) -> Option<GroundShape<Name>> {
    let produced = transform.output_shape(&shape_env(item));
    GroundShape::try_new(crate::types::from_unnamed_shape(&produced)).ok()
}
