
use crate::Kind;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(transparent)]
pub struct CorrelationId<C>(C);

impl<C> CorrelationId<C> {
    #[must_use]
    pub const fn from_value(value: C) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(&self) -> &C {
        &self.0
    }

    #[must_use]
    pub fn into_value(self) -> C {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Envelope<C, P, X> {
    kind: Kind<X>,
    correlation: CorrelationId<C>,
    payload: P,
}

impl<C, P, X> Envelope<C, P, X> {
    #[must_use]
    pub const fn new(kind: Kind<X>, correlation: CorrelationId<C>, payload: P) -> Self {
        Self {
            kind,
            correlation,
            payload,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> &Kind<X> {
        &self.kind
    }

    #[must_use]
    pub const fn correlation(&self) -> &CorrelationId<C> {
        &self.correlation
    }

    #[must_use]
    pub const fn payload(&self) -> &P {
        &self.payload
    }

    #[must_use]
    pub fn into_parts(self) -> (Kind<X>, CorrelationId<C>, P) {
        (self.kind, self.correlation, self.payload)
    }

    #[must_use]
    pub fn map_payload<Q>(self, map: impl FnOnce(P) -> Q) -> Envelope<C, Q, X> {
        Envelope {
            kind: self.kind,
            correlation: self.correlation,
            payload: map(self.payload),
        }
    }
}

mod private {
    pub trait Sealed {}
}

pub trait ProtocolEnvelope: private::Sealed {}

impl<C, P, X> private::Sealed for Envelope<C, P, X> {}
impl<C, P, X> ProtocolEnvelope for Envelope<C, P, X> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Kind, QueryVerb};

    #[test]
    fn payload_mapping_preserves_kind_and_correlation() {
        let envelope = Envelope::new(
            Kind::<()>::Query(QueryVerb::Query),
            CorrelationId::from_value("request-1"),
            7_u8,
        );
        let mapped = envelope.map_payload(|value| value.to_string());

        assert_eq!(mapped.kind(), &Kind::Query(QueryVerb::Query));
        assert_eq!(mapped.correlation().value(), &"request-1");
        assert_eq!(mapped.payload(), "7");
    }
}
