
use crate::{ActorId, EdgeId, ScopeId};
use circular_core::{Boundary, Ceilings, Value};
use circular_protocol::scope_identity as wire;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProductIdentityError {
    UnpublishedArm(&'static str),
    Codec,
    Malformed(&'static str),
}

impl fmt::Display for ProductIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnpublishedArm(arm) => {
                write!(
                    formatter,
                    "carrier does not yet support this variant: {arm}"
                )
            }
            Self::Codec => formatter.write_str("canonical codec rejected the identity value"),
            Self::Malformed(place) => write!(formatter, "invalid carrier shape: {place}"),
        }
    }
}

impl std::error::Error for ProductIdentityError {}

type Result<T> = std::result::Result<T, ProductIdentityError>;

#[must_use]
pub fn wire_scope(scope: &ScopeId) -> Vec<wire::ScopeSegment> {
    scope.segments().iter().map(wire_scope_segment).collect()
}

#[must_use]
pub fn wire_scope_segment(segment: &crate::ScopeSeg) -> wire::ScopeSegment {
    use crate::ScopeSeg;
    match segment {
        ScopeSeg::Child(name) => wire::ScopeSegment::Child(name.as_str().to_owned()),
        ScopeSeg::Instance { of, key } => wire::ScopeSegment::Instance {
            of: of.as_str().to_owned(),
            key: wire_instance_key(key),
        },
    }
}

#[must_use]
pub fn wire_instance_key(key: &crate::InstanceKey) -> wire::InstanceKey {
    use crate::{InstanceKey, InstanceScalar};
    let scalar = |value: &InstanceScalar| match value {
        InstanceScalar::Text(text) => wire::ScalarKey::Text(text.to_string()),
        InstanceScalar::Int(number) => wire::ScalarKey::Int(*number),
        InstanceScalar::Bool(flag) => wire::ScalarKey::Bool(*flag),
    };
    match key {
        InstanceKey::Scalar(value) => wire::InstanceKey::Scalar(scalar(value)),
        InstanceKey::Tuple(values) => wire::InstanceKey::Tuple(values.iter().map(scalar).collect()),
    }
}

pub fn scope_value(scope: &ScopeId) -> Result<Value> {
    Ok(wire::scope_identity_value(&wire_scope(scope)))
}

pub fn instance_key_value(key: &crate::InstanceKey) -> Value {
    wire_instance_key(key).to_value()
}

pub fn actor_value(actor: &ActorId) -> Result<Value> {
    match actor {
        ActorId::Scoped { scope, local } => {
            let name = match local {
                crate::LocalKey::Named(name) => name.as_str().to_owned(),
                crate::LocalKey::Ephemeral(_) => {
                    return Err(ProductIdentityError::UnpublishedArm("LocalKey::Ephemeral"));
                }
            };
            let object = Value::object(vec![
                ("local".to_owned(), Value::string(name)),
                ("scope".to_owned(), scope_value(scope)?),
            ])
            .map_err(|_| ProductIdentityError::Codec)?;
            Ok(object)
        }
        ActorId::System(_) => Err(ProductIdentityError::UnpublishedArm("ActorId::System")),
    }
}

pub fn record_actor_value(actor: &ActorId) -> Result<Value> {
    match actor {
        ActorId::System(crate::SystemActor::Stream) => Ok(Value::Int(1)),
        ActorId::System(crate::SystemActor::Pipeline) => Ok(Value::Int(2)),
        _ => actor_value(actor),
    }
}

/// Decode record identities without widening the public plan decoder.
pub fn record_actor_from_value(value: &Value) -> Result<ActorId> {
    match value {
        Value::Int(1) => Ok(ActorId::System(crate::SystemActor::Stream)),
        Value::Int(2) => Ok(ActorId::System(crate::SystemActor::Pipeline)),
        Value::Object(_) => actor_from_value(value),
        _ => Err(ProductIdentityError::Malformed("record actor arm")),
    }
}

/// Restore a record stamp through the same core-owned creation boundary as writers.
pub fn record_stamp(
    hlc: circular_core::Hlc,
    producer: ActorId,
    sequence: circular_core::Sequence,
    revision: circular_core::RevisionEpochId,
) -> Result<circular_core::Stamp<ActorId>> {
    use crate::{LocalKey, NamedActorId, SystemActor};
    match producer {
        ActorId::Scoped {
            scope,
            local: LocalKey::Named(name),
        } => Ok(circular_core::Stamp::from_event_producer_at(
            hlc,
            NamedActorId::new(scope, name),
            sequence,
            revision,
        )),
        producer @ ActorId::System(SystemActor::Stream | SystemActor::Pipeline) => {
            circular_core::Stamp::from_system_record_producer_at(hlc, producer, sequence, revision)
                .ok_or(ProductIdentityError::Malformed("record producer"))
        }
        _ => Err(ProductIdentityError::UnpublishedArm(
            "record stamp producer",
        )),
    }
}

fn wire_endpoint(endpoint: &crate::Endpoint) -> (wire::PlanActorKey, String) {
    (
        wire_named_actor(endpoint.actor()),
        endpoint.port().as_str().to_owned(),
    )
}

#[must_use]
pub fn wire_named_actor(actor: &crate::NamedActorId) -> wire::PlanActorKey {
    wire::PlanActorKey {
        scope: wire_scope(actor.scope()),
        local: wire::ActorLocal::parse(actor.name().as_str()),
    }
}

#[must_use]
pub fn wire_edge(edge: &EdgeId) -> circular_protocol::declaration_payload::EdgeKey {
    use circular_protocol::declaration_payload::{DeclaredEdgeKey, EdgeKey};
    match edge {
        EdgeId::Declared { from, to, ordinal } => EdgeKey::Declared(DeclaredEdgeKey {
            from: wire_endpoint(from),
            to: wire_endpoint(to),
            ordinal: *ordinal,
        }),
        EdgeId::Outcome { target } => EdgeKey::Outcome(wire_named_actor(target)),
    }
}

pub fn edge_value(edge: &EdgeId) -> Result<Value> {
    Ok(circular_protocol::declaration_payload::edge_identity_value(
        &wire_edge(edge),
    ))
}

pub fn named_actor_value(actor: &crate::NamedActorId) -> Result<Value> {
    Value::object(vec![
        (
            "local".to_owned(),
            Value::string(actor.name().as_str().to_owned()),
        ),
        ("scope".to_owned(), scope_value(actor.scope())?),
    ])
    .map_err(|_| ProductIdentityError::Codec)
}

pub fn identity_bytes(value: &Value) -> Result<Vec<u8>> {
    circular_core::encode(value, Ceilings::for_boundary(Boundary::Identity))
        .map_err(|_| ProductIdentityError::Codec)
}

pub fn scope_from_value(value: &Value) -> Result<ScopeId> {
    let segments = wire::decode_scope_identity(value.clone())
        .map_err(|_| ProductIdentityError::Malformed("scope"))?;
    let rebuilt = segments
        .iter()
        .map(scope_segment_from_wire)
        .collect::<Vec<_>>();
    ScopeId::from_segments(rebuilt).map_err(|_| ProductIdentityError::Malformed("scope limit"))
}

#[must_use]
pub fn scope_segment_from_wire(segment: &wire::ScopeSegment) -> crate::ScopeSeg {
    use crate::{Name, ScopeSeg};
    match segment {
        wire::ScopeSegment::Child(name) => ScopeSeg::Child(Name::from_normalized(name.as_str())),
        wire::ScopeSegment::Instance { of, key } => ScopeSeg::Instance {
            of: Name::from_normalized(of.as_str()),
            key: instance_key_from_wire(key),
        },
    }
}

#[must_use]
pub fn instance_key_from_wire(key: &wire::InstanceKey) -> crate::InstanceKey {
    use crate::{InstanceKey, InstanceScalar};
    let scalar = |value: &wire::ScalarKey| match value {
        wire::ScalarKey::Text(text) => InstanceScalar::Text(text.clone().into_boxed_str()),
        wire::ScalarKey::Int(number) => InstanceScalar::Int(*number),
        wire::ScalarKey::Bool(flag) => InstanceScalar::Bool(*flag),
    };
    match key {
        wire::InstanceKey::Scalar(value) => InstanceKey::Scalar(scalar(value)),
        wire::InstanceKey::Tuple(values) => InstanceKey::Tuple(values.iter().map(scalar).collect()),
    }
}

pub fn instance_key_from_value(value: &Value) -> Result<crate::InstanceKey> {
    let key = wire::decode_instance_key(value.clone())
        .map_err(|_| ProductIdentityError::Malformed("instance scalar"))?;
    Ok(instance_key_from_wire(&key))
}

pub fn actor_parts_from_value(value: &Value) -> Result<(ScopeId, crate::Name)> {
    use crate::Name;
    let Value::Object(object) = value else {
        return Err(ProductIdentityError::Malformed(
            "actor key must be a product",
        ));
    };
    let local = object
        .get("local")
        .and_then(Value::as_str)
        .ok_or(ProductIdentityError::Malformed("local"))?;
    let scope = object
        .get("scope")
        .ok_or(ProductIdentityError::Malformed("scope"))?;
    Ok((scope_from_value(scope)?, Name::from_normalized(local)))
}

pub fn actor_from_value(value: &Value) -> Result<ActorId> {
    use crate::{LocalKey, Name};
    let Value::Object(object) = value else {
        return Err(ProductIdentityError::Malformed(
            "actor key must be a product",
        ));
    };
    let local = object
        .get("local")
        .and_then(Value::as_str)
        .ok_or(ProductIdentityError::Malformed("local"))?;
    let scope = object
        .get("scope")
        .ok_or(ProductIdentityError::Malformed("scope"))?;
    Ok(ActorId::Scoped {
        scope: scope_from_value(scope)?,
        local: LocalKey::Named(Name::from_normalized(local)),
    })
}

pub fn identity_value(bytes: &[u8]) -> Result<Value> {
    circular_core::decode(bytes, Ceilings::for_boundary(Boundary::Identity))
        .map_err(|_| ProductIdentityError::Codec)
}

pub fn named_actor_from_wire(key: &wire::PlanActorKey) -> Result<crate::NamedActorId> {
    use crate::{Name, NamedActorId, ScopeId};
    let scope = ScopeId::from_segments(
        key.scope
            .iter()
            .map(scope_segment_from_wire)
            .collect::<Vec<_>>(),
    )
    .map_err(|_| ProductIdentityError::Malformed("scope limit"))?;
    Ok(NamedActorId::new(
        scope,
        Name::from_normalized(key.local.as_str()),
    ))
}

pub fn edge_from_wire(edge: &circular_protocol::declaration_payload::EdgeKey) -> Result<EdgeId> {
    use circular_core::PortId;
    use circular_protocol::declaration_payload::EdgeKey;
    let endpoint = |(actor, port): &(wire::PlanActorKey, String)| -> Result<crate::Endpoint> {
        Ok(crate::Endpoint::new(
            named_actor_from_wire(actor)?,
            PortId::try_new(port.as_str())
                .map_err(|_| ProductIdentityError::Malformed("port name"))?,
        ))
    };
    Ok(match edge {
        EdgeKey::Declared(key) => EdgeId::Declared {
            from: endpoint(&key.from)?,
            to: endpoint(&key.to)?,
            ordinal: key.ordinal,
        },
        EdgeKey::Outcome(target) => EdgeId::Outcome {
            target: named_actor_from_wire(target)?,
        },
    })
}

pub fn edge_from_value(value: &Value) -> Result<EdgeId> {
    let key = circular_protocol::declaration_payload::decode_edge_identity(value.clone())
        .map_err(|_| ProductIdentityError::Malformed("edge identity"))?;
    edge_from_wire(&key)
}

#[cfg(test)]
mod tests {
    use super::{
        ProductIdentityError, actor_from_value, actor_value, identity_bytes, identity_value,
        scope_value,
    };
    use crate::{
        ActorId, InstanceKey, InstanceScalar, LocalKey, Name, ScopeId, ScopeSeg, SystemActor,
    };
    use circular_core::{Boundary, Ceilings, Value};

    fn name(text: &str) -> Name {
        Name::from_normalized(text)
    }

    #[test]
    fn a_scope_carries_its_segments_as_a_sequence() {
        let scope = ScopeId::from_segments(vec![
            ScopeSeg::Child(name("outer")),
            ScopeSeg::Instance {
                of: name("worker"),
                key: InstanceKey::Scalar(InstanceScalar::Int(7)),
            },
        ])
        .expect("the two segments are under the ceiling");

        let value = scope_value(&scope).expect("only published arms");
        let expected = Value::array(vec![
            Value::array(vec![Value::int(1), Value::string("outer".to_owned())]),
            Value::array(vec![
                Value::int(2),
                Value::string("worker".to_owned()),
                Value::int(7),
            ]),
        ]);
        assert_eq!(value, expected, "order is hierarchy, so it is not sorted");
    }

    #[test]
    fn identity_bytes_are_the_canonical_bytes_of_the_carrier() {
        let actor = ActorId::Scoped {
            scope: ScopeId::root(),
            local: LocalKey::Named(name("tap")),
        };
        let value = actor_value(&actor).expect("published arm");
        let bytes = identity_bytes(&value).expect("canonical encoding");

        assert_eq!(
            bytes,
            circular_core::encode(&value, Ceilings::for_boundary(Boundary::Identity))
                .expect("same codec"),
        );
        assert_eq!(
            circular_core::decode(&bytes, Ceilings::for_boundary(Boundary::Identity))
                .expect("round trip"),
            value,
        );
    }

    #[test]
    fn two_instances_of_one_template_take_different_bytes() {
        let actor = |key: InstanceKey| {
            let actor = ActorId::Scoped {
                scope: ScopeId::from_segments(vec![ScopeSeg::Instance {
                    of: name("session-cell"),
                    key,
                }])
                .expect("one segment"),
                local: LocalKey::Named(name("tokens")),
            };
            identity_bytes(&actor_value(&actor).expect("published arm"))
                .expect("canonical encoding")
        };
        let first = actor(InstanceKey::Scalar(InstanceScalar::Text("s-1".into())));
        let second = actor(InstanceKey::Scalar(InstanceScalar::Text("s-2".into())));
        let tuple = actor(InstanceKey::Tuple(Box::new([InstanceScalar::Text(
            "s-1".into(),
        )])));
        assert_ne!(first, second, "different keys folded into one byte string");
        assert_ne!(
            first, tuple,
            "a scalar and a one-element tuple folded into one byte string; the two arms must stay apart"
        );
    }

    #[test]
    fn the_instance_scoped_actor_identity_has_fixed_bytes() {
        let actor = ActorId::Scoped {
            scope: ScopeId::from_segments(vec![ScopeSeg::Instance {
                of: name("session-cell"),
                key: InstanceKey::Scalar(InstanceScalar::Int(7)),
            }])
            .expect("one segment"),
            local: LocalKey::Named(name("tokens")),
        };
        let bytes = identity_bytes(&actor_value(&actor).expect("published arm"))
            .expect("canonical encoding");
        let hex = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            hex,
            concat!(
                "0800000002000000056c6f63616c0500000006746f6b656e73",
                "0000000573636f7065070000000107000000030300000000000000",
                "02050000000c73657373696f6e2d63656c6c030000000000000007",
            )
        );
    }

    #[test]
    fn record_system_carrier_is_disjoint_and_does_not_expand_plan_identities() {
        use super::{record_actor_from_value, record_actor_value, record_stamp};
        use circular_core::Value;
        let system = ActorId::System(SystemActor::Stream);
        assert_eq!(record_actor_value(&system), Ok(Value::Int(1)));
        assert_eq!(record_actor_from_value(&Value::Int(1)), Ok(system));
        let bytes = [3, 0, 0, 0, 0, 0, 0, 0, 1];
        assert_eq!(identity_bytes(&Value::Int(1)).unwrap(), bytes);
        for end in 0..bytes.len() {
            assert!(identity_value(&bytes[..end]).is_err());
        }
        for bad in [
            Value::Int(0),
            Value::Int(3),
            Value::Int(-1),
            Value::UInt(1),
            Value::array([Value::Int(1)]),
            Value::Null,
        ] {
            assert!(record_actor_from_value(&bad).is_err(), "{bad:?}");
        }
        assert!(actor_from_value(&Value::Int(1)).is_err());
        assert!(super::actor_parts_from_value(&Value::Int(1)).is_err());
        let heartbeat = ActorId::System(SystemActor::Heartbeat);
        assert!(record_actor_value(&heartbeat).is_err());
        assert!(
            record_stamp(
                circular_core::Hlc::from_physical(circular_core::Tick::ZERO),
                heartbeat,
                circular_core::Sequence::new(0).unwrap(),
                circular_core::RevisionEpochId::new(1).unwrap()
            )
            .is_err()
        );
    }

    #[test]
    fn an_unpublished_arm_is_refused_rather_than_invented() {
        assert_eq!(
            actor_value(&ActorId::System(SystemActor::Stream)),
            Err(ProductIdentityError::UnpublishedArm("ActorId::System")),
        );
    }
}
