
use crate::actor_registry::{ProductPayload, ProductValue};
use crate::{BaseShape, Shape};
use circular_core::GroundShape;
use std::error::Error;
use std::fmt;

#[must_use]
pub fn bang_event() -> ProductPayload {
    ProductPayload::new(
        GroundShape::try_new(Shape::Base(BaseShape::Null))
            .expect("Null pulse shape contains no variable"),
        ProductValue::Null,
    )
}

pub fn reject_nonempty_config(config: &ProductValue) -> Result<(), EmptyConfigFactoryError> {
    match config {
        ProductValue::Object(fields) if fields.is_empty() => Ok(()),
        _ => Err(EmptyConfigFactoryError::NonEmptyConfig),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmptyConfigFactoryError {
    NonEmptyConfig,
}

impl fmt::Display for EmptyConfigFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonEmptyConfig => formatter.write_str("config must be empty"),
        }
    }
}

impl Error for EmptyConfigFactoryError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bang_retained_schema_keeps_exact_ports_and_empty_config() {
        use crate::{Arity, EffectDeclaration, Flow, Presence};

        let spec = crate::get(crate::ActorType::Bang);
        let ports = spec.ports();
        assert!(ports.dynamic().is_empty());
        assert_eq!(ports.fixed().inlets().len(), 1);
        assert_eq!(ports.fixed().outlets().len(), 1);

        let inlet = &ports.fixed().inlets()[0];
        assert_eq!(inlet.id().as_str(), "event");
        assert_eq!(inlet.label().as_str(), "Event");
        assert_eq!(inlet.ty(), &Flow::Stream(Shape::Any));
        assert_eq!(inlet.arity(), Arity::Many);
        assert_eq!(inlet.presence(), &Presence::Required);
        assert!(inlet.primary());

        let outlet = &ports.fixed().outlets()[0];
        assert_eq!(outlet.id().as_str(), "pulse");
        assert_eq!(outlet.label().as_str(), "Pulse");
        assert_eq!(outlet.ty(), &Flow::Stream(Shape::Base(BaseShape::Null)));
        assert_eq!(outlet.arity(), Arity::Many);
        assert!(outlet.primary());

        assert!(spec.config().is_empty());
        assert!(spec.requires().is_empty());
        assert_eq!(spec.effect(), &EffectDeclaration::None);
        assert!(!spec.is_source());
    }

    #[test]
    fn canonical_pulse_value_matches_the_registered_null_shape() {
        let payload = bang_event();
        assert_eq!(payload.shape().as_shape(), &Shape::Base(BaseShape::Null));
        assert_eq!(payload.value(), &ProductValue::Null);
    }
}
