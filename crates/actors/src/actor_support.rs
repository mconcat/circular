
use crate::{BaseShape, GroundShape, ProductPayload, ProductValue, Shape};
use circular_core::{Event, EventPayload, OperationIdentity, ProducerIdentity, StreamIdentity};

/// Recorded envelope time visible to actor mechanisms, never a body binding.
pub trait StampedEvent {
    fn recorded_instant(&self) -> Option<circular_core::RecordedInstant> {
        None
    }
}

impl<R: StreamIdentity, P: ProducerIdentity, D: EventPayload, O: OperationIdentity> StampedEvent
    for Event<R, P, D, O>
{
    fn recorded_instant(&self) -> Option<circular_core::RecordedInstant> {
        Event::recorded_instant(self)
    }
}

impl StampedEvent for ProductPayload {}

pub(crate) fn error_payload(message: impl Into<String>) -> ProductPayload {
    ProductPayload::new(
        GroundShape::try_new(Shape::Base(BaseShape::String))
            .expect("actor error shape has no variables"),
        ProductValue::String(message.into()),
    )
}

pub(crate) enum ConfigUnsigned {
    Value(u64),
    Negative(i64),
    NotAnInteger,
}

pub(crate) fn read_config_unsigned(value: &ProductValue) -> ConfigUnsigned {
    match value {
        ProductValue::UInt(value) => ConfigUnsigned::Value(*value),
        ProductValue::Int(value) => match u64::try_from(*value) {
            Ok(value) => ConfigUnsigned::Value(value),
            Err(_) => ConfigUnsigned::Negative(*value),
        },
        _ => ConfigUnsigned::NotAnInteger,
    }
}

pub(crate) fn config_unsigned(value: &ProductValue) -> Option<u64> {
    match read_config_unsigned(value) {
        ConfigUnsigned::Value(value) => Some(value),
        ConfigUnsigned::Negative(_) | ConfigUnsigned::NotAnInteger => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_unsigned_reads_both_integer_variants_and_rejects_the_rest() {
        assert_eq!(config_unsigned(&ProductValue::UInt(7)), Some(7));
        assert_eq!(config_unsigned(&ProductValue::Int(7)), Some(7));
        assert_eq!(config_unsigned(&ProductValue::Int(-1)), None);
        assert_eq!(config_unsigned(&ProductValue::String("7".to_owned())), None);
    }
}
