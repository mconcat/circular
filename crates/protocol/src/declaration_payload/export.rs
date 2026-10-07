
use circular_core::{Ceilings, Value, decode};

use super::edge::{PortId, decode_endpoint};
use crate::scope_identity::{
    AddressContext, AddressRef, PlanActorKey, ScopeSegment, decode_address, decode_scope_identity,
};
use crate::wire_value::{
    PayloadRejection, exhausted, object_fields, object_from_value, optional, take, text_of,
};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PlanExportKey {
    pub scope: Vec<ScopeSegment>,
    pub local: String,
}

impl PlanExportKey {
    #[must_use]
    pub fn root(local: impl Into<String>) -> Self {
        Self {
            scope: Vec::new(),
            local: local.into(),
        }
    }

    #[must_use]
    pub fn to_value(&self) -> Value {
        crate::scope_identity::plan_actor_key_value(&PlanActorKey {
            scope: self.scope.clone(),
            local: crate::scope_identity::ActorLocal::parse(self.local.as_str()),
        })
    }
}

impl std::fmt::Display for PlanExportKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::scope_identity::{InstanceKey, ScalarKey};
        let scalar = |formatter: &mut std::fmt::Formatter<'_>, key: &ScalarKey| match key {
            ScalarKey::Text(text) => write!(formatter, "{text:?}"),
            ScalarKey::Int(number) => write!(formatter, "{number}"),
            ScalarKey::Bool(flag) => write!(formatter, "{flag}"),
        };
        for segment in &self.scope {
            match segment {
                ScopeSegment::Child(name) => write!(formatter, "{name}/")?,
                ScopeSegment::Instance { of, key } => {
                    write!(formatter, "{of}[")?;
                    match key {
                        InstanceKey::Scalar(key) => scalar(formatter, key)?,
                        InstanceKey::Tuple(keys) => {
                            for (index, key) in keys.iter().enumerate() {
                                if index != 0 {
                                    formatter.write_str(", ")?;
                                }
                                scalar(formatter, key)?;
                            }
                        }
                    }
                    formatter.write_str("]/")?;
                }
            }
        }
        formatter.write_str(&self.local)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Default)]
pub struct ExportRoles {
    pub request: Option<(PlanActorKey, PortId)>,
    pub progress: Option<(PlanActorKey, PortId)>,
    pub result: Option<(PlanActorKey, PortId)>,
    pub error: Option<(PlanActorKey, PortId)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExportDeclaration {
    pub roles: ExportRoles,
    pub operations: Option<Value>,
    pub surface: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UpsertExportMount {
    pub mount: AddressRef<PlanExportKey>,
    pub declaration: ExportDeclaration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetireExportMount {
    pub mount: AddressRef<PlanExportKey>,
}

pub fn decode_plan_export_key(value: Value) -> Result<PlanExportKey, PayloadRejection> {
    let mut fields = object_fields(value, "mount")?;
    let local = text_of(take(&mut fields, "local")?, "local")?;
    let scope = decode_scope_identity(take(&mut fields, "scope")?)?;
    exhausted(fields)?;
    Ok(PlanExportKey { scope, local })
}

pub fn decode_export_address(value: Value) -> Result<AddressRef<PlanExportKey>, PayloadRejection> {
    decode_address(value, decode_plan_export_key)
}

fn decode_export_roles(value: Value) -> Result<ExportRoles, PayloadRejection> {
    let mut fields = object_fields(value, "roles")?;
    let error = optional(&mut fields, "error")
        .map(decode_endpoint)
        .transpose()?;
    let progress = optional(&mut fields, "progress")
        .map(decode_endpoint)
        .transpose()?;
    let request = optional(&mut fields, "request")
        .map(decode_endpoint)
        .transpose()?;
    let result = optional(&mut fields, "result")
        .map(decode_endpoint)
        .transpose()?;
    exhausted(fields)?;
    Ok(ExportRoles {
        request,
        progress,
        result,
        error,
    })
}

fn decode_export_declaration(value: Value) -> Result<ExportDeclaration, PayloadRejection> {
    let mut fields = object_fields(value, "declaration")?;
    let operations = optional(&mut fields, "operations");
    let surface = optional(&mut fields, "surface");
    let roles = decode_export_roles(take(&mut fields, "roles")?)?;
    exhausted(fields)?;
    Ok(ExportDeclaration {
        roles,
        operations,
        surface,
    })
}

pub fn upsert_export_mount_from_value(
    value: Value,
    context: AddressContext,
) -> Result<UpsertExportMount, PayloadRejection> {
    let mut fields = object_from_value(value)?;
    let declaration = decode_export_declaration(take(&mut fields, "declaration")?)?;
    let mount = decode_export_address(take(&mut fields, "mount")?)?;
    exhausted(fields)?;

    if !context.admits(&mount) {
        return Err(PayloadRejection::ArmNotAdmitted);
    }

    Ok(UpsertExportMount { mount, declaration })
}

pub fn decode_upsert_export_mount(
    bytes: &[u8],
    context: AddressContext,
    ceilings: Ceilings,
) -> Result<UpsertExportMount, PayloadRejection> {
    upsert_export_mount_from_value(
        decode(bytes, ceilings).map_err(PayloadRejection::Codec)?,
        context,
    )
}

pub fn retire_export_mount_from_value(
    value: Value,
    context: AddressContext,
) -> Result<RetireExportMount, PayloadRejection> {
    let mut fields = object_from_value(value)?;
    let mount = decode_export_address(take(&mut fields, "mount")?)?;
    exhausted(fields)?;

    if !context.admits(&mount) {
        return Err(PayloadRejection::ArmNotAdmitted);
    }

    Ok(RetireExportMount { mount })
}

pub fn decode_retire_export_mount(
    bytes: &[u8],
    context: AddressContext,
    ceilings: Ceilings,
) -> Result<RetireExportMount, PayloadRejection> {
    retire_export_mount_from_value(
        decode(bytes, ceilings).map_err(PayloadRejection::Codec)?,
        context,
    )
}

#[cfg(test)]
mod export_tests {
    use super::*;
    use circular_core::{ObjectValue, encode};

    const CEILINGS: Ceilings = Ceilings::for_boundary(circular_core::Boundary::Wire);

    fn key_object(scope: &[&str], local: &str) -> Value {
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

    fn endpoint(scope: &[&str], local: &str, port: &str) -> Value {
        Value::Array(vec![
            key_object(scope, local),
            Value::String(port.to_owned()),
        ])
    }

    fn object_of(entries: Vec<(&str, Value)>) -> Value {
        Value::Object(
            ObjectValue::try_from_entries(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value)),
            )
            .expect("keys do not overlap"),
        )
    }

    fn body(mount: Value, declaration: Value) -> Vec<u8> {
        encode(
            &object_of(vec![("declaration", declaration), ("mount", mount)]),
            CEILINGS,
        )
        .expect("encodes")
    }

    fn mount(scope: &[&str], local: &str) -> Value {
        Value::Array(vec![Value::Int(1), key_object(scope, local)])
    }

    #[test]
    fn every_role_opens() {
        let decoded = decode_upsert_export_mount(
            &body(
                mount(&["gate"], "run"),
                object_of(vec![
                    ("operations", Value::Null),
                    (
                        "surface",
                        Value::object([
                            ("$circular", Value::string("export-definition")),
                            ("roles", Value::object([] as [(&str, Value); 0]).unwrap()),
                            ("params", Value::object([] as [(&str, Value); 0]).unwrap()),
                            ("operations", Value::Null),
                            (
                                "surfaces",
                                Value::array([Value::object([
                                    ("mark", Value::string("window")),
                                    (
                                        "spec",
                                        Value::object([("title", Value::string("Run"))]).unwrap(),
                                    ),
                                    ("children", Value::array([])),
                                ])
                                .unwrap()]),
                            ),
                        ])
                        .unwrap(),
                    ),
                    (
                        "roles",
                        object_of(vec![
                            ("error", endpoint(&["gate"], "fail", "out")),
                            ("progress", endpoint(&["gate"], "tap", "out")),
                            ("request", endpoint(&["gate", "input"], "bang", "out")),
                            ("result", endpoint(&["gate"], "sink", "out")),
                        ]),
                    ),
                ]),
            ),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("decodes");

        let roles = &decoded.declaration.roles;
        assert_eq!(roles.request.as_ref().expect("request").1, "out");
        assert_eq!(
            roles.request.as_ref().expect("request").0.scope.len(),
            2,
            "the two segments were flattened"
        );
        assert!(roles.progress.is_some() && roles.result.is_some() && roles.error.is_some());
        assert_eq!(decoded.declaration.operations, Some(Value::Null));
        assert_eq!(
            decoded.declaration.surface,
            Some(
                Value::object([
                    ("$circular", Value::string("export-definition")),
                    ("roles", Value::object([] as [(&str, Value); 0]).unwrap()),
                    ("params", Value::object([] as [(&str, Value); 0]).unwrap()),
                    ("operations", Value::Null),
                    (
                        "surfaces",
                        Value::array([Value::object([
                            ("mark", Value::string("window")),
                            (
                                "spec",
                                Value::object([("title", Value::string("Run"))]).unwrap()
                            ),
                            ("children", Value::array([])),
                        ])
                        .unwrap()])
                    ),
                ])
                .unwrap()
            )
        );
    }

    #[test]
    fn export_may_reference_a_synth_boundary_actor() {
        let synth = "_bni1_0123456789abcdef0123456789";
        let decoded = decode_upsert_export_mount(
            &body(
                mount(&["gate"], "run"),
                object_of(vec![(
                    "roles",
                    object_of(vec![("request", endpoint(&["gate"], synth, "out"))]),
                )]),
            ),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("export decodes");
        let actor = &decoded.declaration.roles.request.expect("request role").0;
        assert!(matches!(
            &actor.local,
            crate::scope_identity::ActorLocal::Synth(_)
        ));
        assert_eq!(actor.local.as_str(), synth);
    }

    #[test]
    fn an_unbound_role_is_an_absent_key_and_not_a_sentinel() {
        let decoded = decode_upsert_export_mount(
            &body(
                mount(&["gate"], "run"),
                object_of(vec![(
                    "roles",
                    object_of(vec![(
                        "request",
                        endpoint(&["gate", "input"], "bang", "out"),
                    )]),
                )]),
            ),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("an export with only one role bound is a value too");

        assert!(decoded.declaration.roles.request.is_some());
        assert_eq!(decoded.declaration.roles.progress, None);
        assert_eq!(decoded.declaration.operations, None);
    }

    #[test]
    fn the_roles_record_itself_is_required() {
        assert_eq!(
            decode_upsert_export_mount(
                &body(mount(&["gate"], "run"), object_of(Vec::new())),
                AddressContext::Mutation,
                CEILINGS
            ),
            Err(PayloadRejection::MissingKey("roles"))
        );
        assert!(
            decode_upsert_export_mount(
                &body(
                    mount(&["gate"], "run"),
                    object_of(vec![("roles", object_of(Vec::new()))])
                ),
                AddressContext::Mutation,
                CEILINGS
            )
            .is_ok(),
            "an export with no role bound is representable"
        );
    }

    #[test]
    fn a_role_slot_carries_the_published_endpoint() {
        assert_eq!(
            decode_upsert_export_mount(
                &body(
                    mount(&["gate"], "run"),
                    object_of(vec![(
                        "roles",
                        object_of(vec![("request", Value::String("bang.out".to_owned()))])
                    )])
                ),
                AddressContext::Mutation,
                CEILINGS
            ),
            Err(PayloadRejection::WrongCarrier { key: "endpoint" })
        );
    }

    #[test]
    fn a_retire_carries_only_its_target() {
        let retire = encode(
            &object_of(vec![("mount", mount(&["gate", "inner"], "run"))]),
            CEILINGS,
        )
        .expect("encodes");
        let decoded = decode_retire_export_mount(&retire, AddressContext::Mutation, CEILINGS)
            .expect("decodes");
        let AddressRef::Absolute(key) = &decoded.mount else {
            panic!("it is an absolute address");
        };
        assert_eq!(key.local, "run");
        assert_eq!(key.scope.len(), 2);

        assert_eq!(
            decode_retire_export_mount(
                &body(
                    mount(&["gate"], "run"),
                    object_of(vec![("roles", object_of(Vec::new()))])
                ),
                AddressContext::Mutation,
                CEILINGS
            ),
            Err(PayloadRejection::UnknownKey("declaration".to_owned()))
        );
    }

    #[test]
    fn the_context_partitions_the_arms() {
        let epoch_local = Value::Array(vec![Value::Int(2), key_object(&["gate"], "run")]);
        let upsert = body(
            epoch_local.clone(),
            object_of(vec![("roles", object_of(Vec::new()))]),
        );
        assert!(decode_upsert_export_mount(&upsert, AddressContext::Mutation, CEILINGS).is_ok());
        assert_eq!(
            decode_upsert_export_mount(&upsert, AddressContext::AcceptedHistory, CEILINGS),
            Err(PayloadRejection::ArmNotAdmitted)
        );

        let retire = encode(&object_of(vec![("mount", epoch_local)]), CEILINGS).expect("encodes");
        assert_eq!(
            decode_retire_export_mount(&retire, AddressContext::AcceptedHistory, CEILINGS),
            Err(PayloadRejection::ArmNotAdmitted)
        );
    }
}
