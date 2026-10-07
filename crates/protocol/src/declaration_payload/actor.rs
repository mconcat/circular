
use circular_core::{Ceilings, Value, decode};

use super::flags::{ActorFlags, decode_flags};
#[cfg(test)]
use crate::scope_identity::ScopeSegment;
use crate::scope_identity::{
    ActorLocal, AddressContext, AddressRef, PlanActorKey, decode_actor_address,
};
use crate::wire_value::{
    PayloadRejection, exhausted, object_fields, object_from_value, take, text_of,
};

#[derive(Clone, Debug, PartialEq)]
pub struct ActorDeclaration {
    pub actor_type: String,
    pub config: Value,
    pub flags: ActorFlags,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UpsertActor {
    pub actor: AddressRef<PlanActorKey>,
    pub declaration: ActorDeclaration,
}

fn decode_actor_declaration(value: Value) -> Result<ActorDeclaration, PayloadRejection> {
    let mut parts = object_fields(value, "declaration")?;
    let domain = take(&mut parts, "domain")?;
    let flags = decode_flags(take(&mut parts, "flags")?)?;
    exhausted(parts)?;

    let mut domain_fields = object_fields(domain, "domain")?;
    let config = take(&mut domain_fields, "config")?;
    let actor_type = text_of(take(&mut domain_fields, "actor_type")?, "actor_type")?;
    exhausted(domain_fields)?;

    Ok(ActorDeclaration {
        actor_type,
        config,
        flags,
    })
}

pub fn upsert_actor_from_value(
    value: Value,
    context: AddressContext,
) -> Result<UpsertActor, PayloadRejection> {
    let mut fields = object_from_value(value)?;
    let declaration = decode_actor_declaration(take(&mut fields, "declaration")?)?;
    let actor = decode_actor_address(take(&mut fields, "actor")?)?;
    exhausted(fields)?;

    if !context.admits(&actor) {
        return Err(PayloadRejection::ArmNotAdmitted);
    }
    let local = match &actor {
        AddressRef::Absolute(key) | AddressRef::EpochLocal(key) | AddressRef::Relative(key) => {
            &key.local
        }
    };
    if matches!(local, ActorLocal::Synth(_)) {
        return Err(PayloadRejection::ReservedLocalSpelling);
    }

    Ok(UpsertActor { actor, declaration })
}

pub fn decode_upsert_actor(
    bytes: &[u8],
    context: AddressContext,
    ceilings: Ceilings,
) -> Result<UpsertActor, PayloadRejection> {
    upsert_actor_from_value(
        decode(bytes, ceilings).map_err(PayloadRejection::Codec)?,
        context,
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetireActor {
    pub actor: AddressRef<PlanActorKey>,
}

pub fn retire_actor_from_value(
    value: Value,
    context: AddressContext,
) -> Result<RetireActor, PayloadRejection> {
    let mut fields = object_from_value(value)?;
    let actor = decode_actor_address(take(&mut fields, "actor")?)?;
    exhausted(fields)?;

    if !context.admits(&actor) {
        return Err(PayloadRejection::ArmNotAdmitted);
    }

    Ok(RetireActor { actor })
}

pub fn decode_retire_actor(
    bytes: &[u8],
    context: AddressContext,
    ceilings: Ceilings,
) -> Result<RetireActor, PayloadRejection> {
    retire_actor_from_value(
        decode(bytes, ceilings).map_err(PayloadRejection::Codec)?,
        context,
    )
}

#[cfg(test)]
mod upsert_actor_tests {
    use super::*;
    use circular_core::{ObjectValue, encode};

    const CEILINGS: Ceilings = Ceilings::for_boundary(circular_core::Boundary::Wire);

    fn actor_key(scope: &[&str], local: &str) -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                ("local".to_owned(), Value::String(local.to_owned())),
                (
                    "scope".to_owned(),
                    Value::Array(
                        scope
                            .iter()
                            .map(|name| {
                                Value::Array(vec![Value::Int(1), Value::String((*name).to_owned())])
                            })
                            .collect(),
                    ),
                ),
            ])
            .expect("two keys"),
        )
    }

    fn flags(bypass: bool) -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                ("bypass".to_owned(), Value::Bool(bypass)),
                ("mute".to_owned(), Value::Bool(false)),
                ("pause".to_owned(), Value::Bool(false)),
            ])
            .expect("three keys"),
        )
    }

    fn declaration(actor_type: &str, config: Value) -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                (
                    "domain".to_owned(),
                    Value::Object(
                        ObjectValue::try_from_entries([
                            ("config".to_owned(), config),
                            (
                                "actor_type".to_owned(),
                                Value::String(actor_type.to_owned()),
                            ),
                        ])
                        .expect("two keys"),
                    ),
                ),
                ("flags".to_owned(), flags(false)),
            ])
            .expect("two parts"),
        )
    }

    fn body(actor: Value, declaration_value: Value) -> Vec<u8> {
        let object = ObjectValue::try_from_entries([
            ("declaration".to_owned(), declaration_value),
            ("actor".to_owned(), actor),
        ])
        .expect("two keys");
        encode(&Value::Object(object), CEILINGS).expect("encodes")
    }

    #[test]
    fn an_upsert_actor_opens_with_the_canonical_shape() {
        let bytes = body(
            Value::Array(vec![Value::Int(1), actor_key(&["outer"], "bang")]),
            declaration("bang", Value::Null),
        );
        let decoded =
            decode_upsert_actor(&bytes, AddressContext::Mutation, CEILINGS).expect("decodes");
        assert_eq!(
            decoded.actor,
            AddressRef::Absolute(PlanActorKey {
                scope: vec![ScopeSegment::Child("outer".to_owned())],
                local: crate::scope_identity::AuthoredLocal::try_new("bang")
                    .expect("test actor local is authored")
                    .into(),
            })
        );
        assert_eq!(decoded.declaration.actor_type, "bang");
        assert!(!decoded.declaration.flags.bypass);
    }

    #[test]
    fn an_upsert_actor_rejects_the_synth_local_arm() {
        let bytes = body(
            Value::Array(vec![
                Value::Int(1),
                actor_key(&[], "_bni1_0123456789abcdef0123456789"),
            ]),
            declaration("input", Value::Null),
        );
        assert_eq!(
            decode_upsert_actor(&bytes, AddressContext::Mutation, CEILINGS),
            Err(PayloadRejection::ReservedLocalSpelling)
        );
    }

    #[test]
    fn config_passes_through_uninterpreted() {
        let config = Value::Object(
            ObjectValue::try_from_entries([("period".to_owned(), Value::Int(250))])
                .expect("one key"),
        );
        let bytes = body(
            Value::Array(vec![Value::Int(1), actor_key(&["s"], "n")]),
            declaration("bang", config.clone()),
        );
        let decoded =
            decode_upsert_actor(&bytes, AddressContext::Mutation, CEILINGS).expect("decodes");
        assert_eq!(decoded.declaration.config, config);
    }

    #[test]
    fn the_flag_set_is_closed_at_three() {
        let two = Value::Object(
            ObjectValue::try_from_entries([
                ("bypass".to_owned(), Value::Bool(false)),
                ("mute".to_owned(), Value::Bool(false)),
            ])
            .expect("two keys"),
        );
        let partial = Value::Object(
            ObjectValue::try_from_entries([
                (
                    "domain".to_owned(),
                    Value::Object(
                        ObjectValue::try_from_entries([
                            ("config".to_owned(), Value::Null),
                            ("actor_type".to_owned(), Value::String("b".to_owned())),
                        ])
                        .expect("two keys"),
                    ),
                ),
                ("flags".to_owned(), two),
            ])
            .expect("two parts"),
        );
        let bytes = body(
            Value::Array(vec![Value::Int(1), actor_key(&["s"], "n")]),
            partial,
        );
        assert_eq!(
            decode_upsert_actor(&bytes, AddressContext::Mutation, CEILINGS),
            Err(PayloadRejection::MissingKey("pause"))
        );
    }
}
