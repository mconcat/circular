//! First-class, root-relative compacted DeclarationCommand values.
use crate::scope_identity::AddressContext;
use crate::wire_value::{PayloadRejection, exhausted, object_from_value, take, text_of};
use circular_core::{Ceilings, Value, decode};

#[derive(Clone, Debug, PartialEq)]
pub struct UpsertTemplate {
    pub name: String,
    pub commands: Vec<Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetireTemplate {
    pub name: String,
}

pub fn upsert_template_from_value(value: Value) -> Result<UpsertTemplate, PayloadRejection> {
    let mut fields = object_from_value(value)?;
    let name = text_of(take(&mut fields, "name")?, "name")?;
    let Value::Array(commands) = take(&mut fields, "commands")? else {
        return Err(PayloadRejection::WrongCarrier { key: "commands" });
    };
    exhausted(fields)?;
    for command in &commands {
        crate::authoring_snapshot::decode_compacted(command, AddressContext::AuthoringSnapshot)
            .map_err(|_| PayloadRejection::ArmNotAdmitted)?;
    }
    Ok(UpsertTemplate { name, commands })
}

pub fn decode_upsert_template(
    bytes: &[u8],
    ceilings: Ceilings,
) -> Result<UpsertTemplate, PayloadRejection> {
    upsert_template_from_value(decode(bytes, ceilings).map_err(PayloadRejection::Codec)?)
}

pub fn retire_template_from_value(value: Value) -> Result<RetireTemplate, PayloadRejection> {
    let mut fields = object_from_value(value)?;
    let name = text_of(take(&mut fields, "name")?, "name")?;
    exhausted(fields)?;
    Ok(RetireTemplate { name })
}

pub fn decode_retire_template(
    bytes: &[u8],
    ceilings: Ceilings,
) -> Result<RetireTemplate, PayloadRejection> {
    retire_template_from_value(decode(bytes, ceilings).map_err(PayloadRejection::Codec)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DeclarationCommand;
    use crate::authoring_snapshot::{AuthoringSnapshotEncoder, decode_compacted};
    use circular_core::Boundary;
    use circular_core::encode;

    const EMPTY_UPSERT: &[u8] = b"\x08\x00\x00\x00\x02\x00\x00\x00\x08commands\x07\x00\x00\x00\x00\x00\x00\x00\x04name\x05\x00\x00\x00\x06worker";
    const RETIRE: &[u8] = b"\x08\x00\x00\x00\x01\x00\x00\x00\x04name\x05\x00\x00\x00\x06worker";

    fn object(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
        Value::object(fields).unwrap()
    }
    fn body(commands: Vec<Value>) -> Value {
        object([
            ("name", Value::string("worker")),
            ("commands", Value::Array(commands)),
        ])
    }
    fn tap(address_tag: i64) -> Value {
        object([
            ("kind", Value::string("UpsertActor")),
            (
                "actor",
                Value::Array(vec![
                    Value::Int(address_tag),
                    object([
                        ("scope", Value::Array(vec![])),
                        ("local", Value::string("tap")),
                    ]),
                ]),
            ),
            (
                "declaration",
                object([
                    (
                        "domain",
                        object([
                            ("actor_type", Value::string("tap")),
                            ("config", Value::Null),
                        ]),
                    ),
                    (
                        "flags",
                        object([
                            ("bypass", Value::Bool(false)),
                            ("mute", Value::Bool(false)),
                            ("pause", Value::Bool(false)),
                        ]),
                    ),
                ]),
            ),
        ])
    }

    #[test]
    fn relative_and_minted_boundary_identities_match_independent_literals() {
        use crate::boundary_port::{
            BoundaryActorGeneration, BoundaryPortDirection, BoundaryPortId,
        };
        use crate::declaration_payload::{AuthoredLocal, PlanActorKey, ScopeSegment};
        for (scope, expected) in [
            (vec![], "_bi1_fe2ee876177f7808c584a56064"),
            (
                vec![ScopeSegment::Child("host".into())],
                "_bi1_112439e93290e8b6e40d2dee49",
            ),
        ] {
            let actor = PlanActorKey {
                scope,
                local: AuthoredLocal::try_new("incoming").unwrap().into(),
            };
            assert_eq!(
                BoundaryPortId::derive(
                    BoundaryPortDirection::Inlet,
                    &actor,
                    BoundaryActorGeneration::initial()
                )
                .unwrap()
                .as_port_id()
                .as_str(),
                expected
            );
        }
    }

    #[test]
    fn template_payloads_match_independent_literal_bytes() {
        assert_eq!(
            decode_upsert_template(EMPTY_UPSERT, Ceilings::for_boundary(Boundary::Wire)).unwrap(),
            UpsertTemplate {
                name: "worker".to_owned(),
                commands: vec![]
            }
        );
        assert_eq!(
            encode(&body(vec![]), Ceilings::for_boundary(Boundary::Wire)).unwrap(),
            EMPTY_UPSERT
        );
        assert_eq!(
            decode_retire_template(RETIRE, Ceilings::for_boundary(Boundary::Wire)).unwrap(),
            RetireTemplate {
                name: "worker".to_owned()
            }
        );
        assert_eq!(
            encode(
                &object([("name", Value::string("worker"))]),
                Ceilings::for_boundary(Boundary::Wire)
            )
            .unwrap(),
            RETIRE
        );
    }

    #[test]
    fn template_commands_use_the_existing_relative_declaration_union() {
        let commands = vec![tap(3)];
        let admitted = upsert_template_from_value(body(commands.clone())).unwrap();
        assert_eq!(admitted.commands, commands);
        let compacted = AuthoringSnapshotEncoder::new(&[])
            .upsert_template("worker", &commands)
            .unwrap();
        assert_eq!(
            decode_compacted(&compacted, AddressContext::AuthoringSnapshot).unwrap(),
            DeclarationCommand::UpsertTemplate {
                epoch: None,
                name: "worker".to_owned(),
                commands
            }
        );
    }

    #[test]
    fn template_body_rejects_absolute_epoch_local_and_mutation_only_commands() {
        for commands in [
            vec![tap(1)],
            vec![tap(2)],
            vec![object([("kind", Value::string("CommitEpoch"))])],
            vec![object([
                ("kind", Value::string("RetireActor")),
                ("actor", Value::Null),
            ])],
        ] {
            assert_eq!(
                upsert_template_from_value(body(commands)),
                Err(PayloadRejection::ArmNotAdmitted)
            );
        }
    }

    #[test]
    fn template_fields_are_closed_and_commands_are_an_array() {
        assert_eq!(
            upsert_template_from_value(object([("name", Value::string("worker"))])),
            Err(PayloadRejection::MissingKey("commands"))
        );
        assert_eq!(
            upsert_template_from_value(object([
                ("name", Value::string("worker")),
                ("commands", Value::Null)
            ])),
            Err(PayloadRejection::WrongCarrier { key: "commands" })
        );
        assert_eq!(
            retire_template_from_value(body(vec![])),
            Err(PayloadRejection::UnknownKey("commands".to_owned()))
        );
    }
}
