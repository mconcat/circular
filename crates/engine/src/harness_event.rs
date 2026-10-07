
use circular_core::Value;
use circular_runtime::{AgentPayload, AgentProgressRecord};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HarnessEventKind {
    BoundaryDenied,
}

impl HarnessEventKind {
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::BoundaryDenied => "boundary_denied",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HarnessBoundaryKind {
    Egress,
}

impl HarnessBoundaryKind {
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Egress => "egress",
        }
    }
}

#[must_use]
pub(crate) fn boundary_denied(
    boundary: HarnessBoundaryKind,
    at: &str,
    count: u64,
) -> AgentProgressRecord {
    let value = Value::object([
        (
            "kind",
            Value::string(HarnessEventKind::BoundaryDenied.as_str()),
        ),
        ("boundary", Value::string(boundary.as_str())),
        ("at", Value::string(at)),
        ("count", Value::UInt(count)),
    ])
    .expect("the four keys are distinct");
    let bytes = circular_core::encode(
        &value,
        circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
    )
    .expect("a boundary record is a small canonical value");
    AgentProgressRecord::new(AgentPayload::new(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_boundary_record_is_one_canonical_value_with_coded_fields() {
        let record = boundary_denied(HarnessBoundaryKind::Egress, "denied.example:443", 2);
        let decoded = circular_core::decode(
            record.payload().as_bytes(),
            circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
        )
        .expect("the record decodes");
        let expected = Value::object([
            ("kind", Value::string("boundary_denied")),
            ("boundary", Value::string("egress")),
            ("at", Value::string("denied.example:443")),
            ("count", Value::UInt(2)),
        ])
        .unwrap();
        assert_eq!(decoded, expected);
    }
}
