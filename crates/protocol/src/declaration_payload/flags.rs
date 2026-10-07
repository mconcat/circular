
use circular_core::{Ceilings, Value, decode};

#[cfg(test)]
use super::actor::decode_upsert_actor;
use crate::scope_identity::{AddressContext, AddressRef, PlanActorKey, decode_actor_address};
use crate::wire_value::{
    PayloadRejection, bool_of, exhausted, object_fields, object_from_value, take,
};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub struct ActorFlags {
    pub bypass: bool,
    pub mute: bool,
    pub pause: bool,
}

impl ActorFlags {
    #[must_use]
    pub const fn new(bypass: bool, mute: bool, pause: bool) -> Self {
        Self {
            bypass,
            mute,
            pause,
        }
    }

    #[must_use]
    pub const fn bypass(self) -> bool {
        self.bypass
    }

    #[must_use]
    pub const fn mute(self) -> bool {
        self.mute
    }

    #[must_use]
    pub const fn pause(self) -> bool {
        self.pause
    }
}

pub(crate) fn decode_flags(value: Value) -> Result<ActorFlags, PayloadRejection> {
    let mut fields = object_fields(value, "flags")?;
    let bypass = bool_of(take(&mut fields, "bypass")?, "bypass")?;
    let mute = bool_of(take(&mut fields, "mute")?, "mute")?;
    let pause = bool_of(take(&mut fields, "pause")?, "pause")?;
    exhausted(fields)?;
    Ok(ActorFlags {
        bypass,
        mute,
        pause,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetFlags {
    pub actor: AddressRef<PlanActorKey>,
    pub flags: ActorFlags,
}

pub fn set_flags_from_value(
    value: Value,
    context: AddressContext,
) -> Result<SetFlags, PayloadRejection> {
    let mut fields = object_from_value(value)?;
    let flags = decode_flags(take(&mut fields, "flags")?)?;
    let actor = decode_actor_address(take(&mut fields, "actor")?)?;
    exhausted(fields)?;

    if !context.admits(&actor) {
        return Err(PayloadRejection::ArmNotAdmitted);
    }

    Ok(SetFlags { actor, flags })
}

pub fn decode_set_flags(
    bytes: &[u8],
    context: AddressContext,
    ceilings: Ceilings,
) -> Result<SetFlags, PayloadRejection> {
    set_flags_from_value(
        decode(bytes, ceilings).map_err(PayloadRejection::Codec)?,
        context,
    )
}

#[cfg(test)]
mod set_flags_tests {
    use super::*;
    use circular_core::{ObjectValue, encode};

    const CEILINGS: Ceilings = Ceilings::for_boundary(circular_core::Boundary::Wire);

    fn actor(scope: &[&str], local: &str) -> Value {
        Value::Array(vec![
            Value::Int(1),
            Value::Object(
                ObjectValue::try_from_entries([
                    ("local".to_owned(), Value::String(local.to_owned())),
                    (
                        "scope".to_owned(),
                        Value::Array(
                            scope
                                .iter()
                                .map(|name| {
                                    Value::Array(vec![
                                        Value::Int(1),
                                        Value::String((*name).to_owned()),
                                    ])
                                })
                                .collect(),
                        ),
                    ),
                ])
                .expect("two keys"),
            ),
        ])
    }

    fn body(actor_value: Value, flags: Value) -> Vec<u8> {
        let object = ObjectValue::try_from_entries([
            ("flags".to_owned(), flags),
            ("actor".to_owned(), actor_value),
        ])
        .expect("two keys");
        encode(&Value::Object(object), CEILINGS).expect("encodes")
    }

    fn three(bypass: bool, mute: bool, pause: bool) -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                ("bypass".to_owned(), Value::Bool(bypass)),
                ("mute".to_owned(), Value::Bool(mute)),
                ("pause".to_owned(), Value::Bool(pause)),
            ])
            .expect("three keys"),
        )
    }

    #[test]
    fn a_set_flags_opens_with_the_canonical_shape() {
        let decoded = decode_set_flags(
            &body(
                actor(&["gate", "inner"], "filter"),
                three(true, false, true),
            ),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("decodes");

        assert_eq!(
            decoded.flags,
            ActorFlags {
                bypass: true,
                mute: false,
                pause: true,
            }
        );
        let AddressRef::Absolute(key) = &decoded.actor else {
            panic!("it is an absolute address");
        };
        assert_eq!(key.local, "filter");
        assert_eq!(key.scope.len(), 2, "the two-segment scope was flattened");
    }

    #[test]
    fn a_switch_spelled_as_a_number_is_rejected() {
        let numeric = Value::Object(
            ObjectValue::try_from_entries([
                ("bypass".to_owned(), Value::Int(1)),
                ("mute".to_owned(), Value::Bool(false)),
                ("pause".to_owned(), Value::Bool(false)),
            ])
            .expect("three keys"),
        );
        assert_eq!(
            decode_set_flags(
                &body(actor(&["gate"], "filter"), numeric),
                AddressContext::Mutation,
                CEILINGS
            ),
            Err(PayloadRejection::WrongCarrier { key: "bypass" })
        );
    }

    #[test]
    fn the_flag_carrier_is_the_one_upsert_actor_uses() {
        let switches = three(true, true, false);
        let by_set_flags = decode_set_flags(
            &body(actor(&["gate"], "filter"), switches.clone()),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("decodes")
        .flags;

        let declaration = Value::Object(
            ObjectValue::try_from_entries([
                (
                    "domain".to_owned(),
                    Value::Object(
                        ObjectValue::try_from_entries([
                            ("config".to_owned(), Value::Null),
                            ("actor_type".to_owned(), Value::String("filter".to_owned())),
                        ])
                        .expect("two keys"),
                    ),
                ),
                ("flags".to_owned(), switches),
            ])
            .expect("two parts"),
        );
        let upsert_body = encode(
            &Value::Object(
                ObjectValue::try_from_entries([
                    ("declaration".to_owned(), declaration),
                    ("actor".to_owned(), actor(&["gate"], "filter")),
                ])
                .expect("two keys"),
            ),
            CEILINGS,
        )
        .expect("encodes");
        let by_upsert = decode_upsert_actor(&upsert_body, AddressContext::Mutation, CEILINGS)
            .expect("decodes")
            .declaration
            .flags;

        assert_eq!(by_set_flags, by_upsert, "the two lengths disagree");
    }

    #[test]
    fn the_context_partitions_the_arms() {
        let epoch_local = Value::Array(vec![
            Value::Int(2),
            Value::Object(
                ObjectValue::try_from_entries([
                    ("local".to_owned(), Value::String("filter".to_owned())),
                    (
                        "scope".to_owned(),
                        Value::Array(vec![Value::Array(vec![
                            Value::Int(1),
                            Value::String("gate".to_owned()),
                        ])]),
                    ),
                ])
                .expect("two keys"),
            ),
        ]);
        let bytes = body(epoch_local, three(false, false, false));
        assert!(decode_set_flags(&bytes, AddressContext::Mutation, CEILINGS).is_ok());
        assert_eq!(
            decode_set_flags(&bytes, AddressContext::AcceptedHistory, CEILINGS),
            Err(PayloadRejection::ArmNotAdmitted)
        );
    }
}
