
use crate::{ObjectValue, PayloadVersionTag, Value};
use std::fmt;

pub const WIRE_PROTOCOL_VERSION: u16 = 1;
pub const WIRE_FRAMING_VERSION: u8 = 1;
pub const RECORD_FORMAT_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AxisValue<V> {
    Declared(V),
    Undeclared,
}

/// No authoring API version has been declared. This type invents no values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthoringApiVersion {}

/// No actor specification version has been declared.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActorSpecVersion {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WireVersions {
    pub protocol: u16,
    pub framing: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecordVersions {
    pub record: u8,
    pub payload_tag: u16,
}

/// Fixed axis order; platform and negotiated partition minors are not axes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompatibilityAxes {
    pub authoring_api: AxisValue<AuthoringApiVersion>,
    pub actor_spec: AxisValue<ActorSpecVersion>,
    pub wire: AxisValue<WireVersions>,
    pub record: AxisValue<RecordVersions>,
}

#[must_use]
pub const fn current() -> CompatibilityAxes {
    CompatibilityAxes {
        authoring_api: AxisValue::Undeclared,
        actor_spec: AxisValue::Undeclared,
        wire: AxisValue::Declared(WireVersions {
            protocol: WIRE_PROTOCOL_VERSION,
            framing: WIRE_FRAMING_VERSION,
        }),
        record: AxisValue::Declared(RecordVersions {
            record: RECORD_FORMAT_VERSION,
            payload_tag: PayloadVersionTag::FIRST.get(),
        }),
    }
}

fn object(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Object(ObjectValue::try_from_entries(entries).expect("unique axis fields"))
}

fn axis_value<V>(
    axis: &AxisValue<V>,
    fields: impl FnOnce(&V) -> Vec<(&'static str, Value)>,
) -> Value {
    let (state, mut entries) = match axis {
        AxisValue::Declared(value) => ("declared", fields(value)),
        AxisValue::Undeclared => ("undeclared", Vec::new()),
    };
    entries.push(("state", Value::String(state.into())));
    object(entries)
}

impl CompatibilityAxes {
    #[must_use]
    pub fn to_value(&self) -> Value {
        object([
            (
                "authoring-api",
                axis_value(&self.authoring_api, |v| match *v {}),
            ),
            ("actor-spec", axis_value(&self.actor_spec, |v| match *v {})),
            (
                "wire",
                axis_value(&self.wire, |v| {
                    vec![
                        ("protocol", Value::UInt(v.protocol.into())),
                        ("framing", Value::UInt(v.framing.into())),
                    ]
                }),
            ),
            (
                "record",
                axis_value(&self.record, |v| {
                    vec![
                        ("format", Value::UInt(v.record.into())),
                        ("payload", Value::UInt(v.payload_tag.into())),
                    ]
                }),
            ),
        ])
    }
}

impl fmt::Display for WireVersions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "protocol {}, framing {}", self.protocol, self.framing)
    }
}

impl fmt::Display for RecordVersions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "format {}, payload {}", self.record, self.payload_tag)
    }
}

impl fmt::Display for AuthoringApiVersion {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {}
    }
}

impl fmt::Display for ActorSpecVersion {
    fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {}
    }
}

impl<V: fmt::Display> fmt::Display for AxisValue<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Declared(value) => value.fmt(f),
            Self::Undeclared => f.write_str("undeclared"),
        }
    }
}

impl fmt::Display for CompatibilityAxes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "authoring-api: {}", self.authoring_api)?;
        writeln!(f, "actor-spec: {}", self.actor_spec)?;
        writeln!(f, "wire: {}", self.wire)?;
        writeln!(f, "record: {}", self.record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_axes_use_their_owners() {
        let axes = current();
        assert_eq!(axes.authoring_api, AxisValue::Undeclared);
        assert_eq!(axes.actor_spec, AxisValue::Undeclared);
        assert_eq!(
            axes.wire,
            AxisValue::Declared(WireVersions {
                protocol: WIRE_PROTOCOL_VERSION,
                framing: WIRE_FRAMING_VERSION,
            })
        );
        assert_eq!(
            axes.record,
            AxisValue::Declared(RecordVersions {
                record: RECORD_FORMAT_VERSION,
                payload_tag: PayloadVersionTag::FIRST.get(),
            })
        );
    }

    #[test]
    fn axes_survive_the_value_codec_without_implicit_absence() {
        for axes in [
            current(),
            CompatibilityAxes {
                wire: AxisValue::Undeclared,
                record: AxisValue::Undeclared,
                ..current()
            },
            CompatibilityAxes {
                wire: AxisValue::Declared(WireVersions {
                    protocol: u16::MAX,
                    framing: u8::MAX,
                }),
                record: AxisValue::Declared(RecordVersions {
                    record: u8::MAX,
                    payload_tag: u16::MAX,
                }),
                ..current()
            },
        ] {
            let value = axes.to_value();
            let bytes = crate::encode(&value, crate::Ceilings::for_boundary(crate::Boundary::Wire))
                .unwrap();
            let decoded =
                crate::decode(&bytes, crate::Ceilings::for_boundary(crate::Boundary::Wire))
                    .unwrap();
            assert_eq!(decoded, value);
            fn no_null(value: &Value) {
                match value {
                    Value::Null => panic!("absence must have an explicit state"),
                    Value::Object(fields) => fields.iter().for_each(|(_, v)| no_null(v)),
                    Value::Array(values) => values.iter().for_each(no_null),
                    _ => {}
                }
            }
            no_null(&value);
        }
    }
}
