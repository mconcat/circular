
use circular_core::{Ceilings, CodecError, Value, decode, encode};

use super::edge::{PortId, decode_endpoint};
use crate::scope_identity::{
    AddressContext, AddressRef, PlanActorKey, ScopeAddress, decode_actor_address,
    decode_scope_address, plan_actor_key_value, scope_identity_value,
};
#[cfg(test)]
use crate::scope_identity::{InstanceKey, ScalarKey, ScopeSegment};
use crate::wire_value::{
    PayloadRejection, decode_arm, exhausted, object_fields, object_from_value, take, text_of,
};

circular_core::closed_table! {
    pub enum ScopeRole: i64 {
        Concrete = 1,
        Template = 2,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeBinding {
    pub inner: (PlanActorKey, PortId),
    pub outer: PortId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeBoundary {
    pub inlets: Vec<ScopeBinding>,
    pub outlets: Vec<ScopeBinding>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeDeclaration {
    pub role: ScopeRole,
    pub boundary: ScopeBoundary,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpsertScope {
    pub scope: ScopeAddress,
    pub declaration: ScopeDeclaration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetireScope {
    pub scope: ScopeAddress,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoveToScope {
    pub actors: Vec<AddressRef<PlanActorKey>>,
    pub target: ScopeAddress,
}

fn decode_scope_role(value: Value) -> Result<ScopeRole, PayloadRejection> {
    let (tag, arguments) = decode_arm(value, "role")?;
    if !arguments.is_empty() {
        return Err(PayloadRejection::WrongCarrier { key: "role" });
    }
    ScopeRole::from_tag(tag).ok_or(PayloadRejection::UnknownArm { tag })
}

fn decode_scope_binding(value: Value) -> Result<ScopeBinding, PayloadRejection> {
    let mut fields = object_fields(value, "binding")?;
    let inner = decode_endpoint(take(&mut fields, "inner")?)?;
    let outer = text_of(take(&mut fields, "outer")?, "outer")?;
    exhausted(fields)?;
    Ok(ScopeBinding { inner, outer })
}

fn decode_boundary_side(
    value: Value,
    key: &'static str,
) -> Result<Vec<ScopeBinding>, PayloadRejection> {
    let Value::Array(items) = value else {
        return Err(PayloadRejection::WrongCarrier { key });
    };
    let mut previous: Option<String> = None;
    let mut bindings = Vec::with_capacity(items.len());
    for item in items {
        let binding = decode_scope_binding(item)?;
        if previous
            .as_ref()
            .is_some_and(|before| before.as_bytes() >= binding.outer.as_bytes())
        {
            return Err(PayloadRejection::NotCanonical { key });
        }
        previous = Some(binding.outer.clone());
        bindings.push(binding);
    }
    Ok(bindings)
}

fn decode_scope_boundary(value: Value) -> Result<ScopeBoundary, PayloadRejection> {
    let mut fields = object_fields(value, "boundary")?;
    let inlets = decode_boundary_side(take(&mut fields, "inlets")?, "inlets")?;
    let outlets = decode_boundary_side(take(&mut fields, "outlets")?, "outlets")?;
    exhausted(fields)?;
    Ok(ScopeBoundary { inlets, outlets })
}

fn decode_scope_declaration(value: Value) -> Result<ScopeDeclaration, PayloadRejection> {
    let mut fields = object_fields(value, "declaration")?;
    let boundary = decode_scope_boundary(take(&mut fields, "boundary")?)?;
    let role = decode_scope_role(take(&mut fields, "role")?)?;
    exhausted(fields)?;
    Ok(ScopeDeclaration { role, boundary })
}

pub fn decode_upsert_scope(
    bytes: &[u8],
    context: AddressContext,
    ceilings: Ceilings,
) -> Result<UpsertScope, PayloadRejection> {
    upsert_scope_from_value(
        decode(bytes, ceilings).map_err(PayloadRejection::Codec)?,
        context,
    )
}

pub fn upsert_scope_from_value(
    value: Value,
    context: AddressContext,
) -> Result<UpsertScope, PayloadRejection> {
    let mut fields = object_from_value(value)?;
    let declaration = decode_scope_declaration(take(&mut fields, "declaration")?)?;
    let scope = decode_scope_address(take(&mut fields, "scope")?)?;
    exhausted(fields)?;

    if !context.admits(&scope) {
        return Err(PayloadRejection::ArmNotAdmitted);
    }

    Ok(UpsertScope { scope, declaration })
}

pub fn retire_scope_from_value(
    value: Value,
    context: AddressContext,
) -> Result<RetireScope, PayloadRejection> {
    let mut fields = object_from_value(value)?;
    let scope = decode_scope_address(take(&mut fields, "scope")?)?;
    exhausted(fields)?;

    if !context.admits(&scope) {
        return Err(PayloadRejection::ArmNotAdmitted);
    }

    Ok(RetireScope { scope })
}

fn address_value<I>(address: &AddressRef<I>, encode_identity: impl FnOnce(&I) -> Value) -> Value {
    let (tag, addressed_identity) = match address {
        AddressRef::Absolute(identity) => (1, identity),
        AddressRef::EpochLocal(identity) => (2, identity),
        AddressRef::Relative(identity) => (3, identity),
    };
    Value::array([Value::Int(tag), encode_identity(addressed_identity)])
}

fn actor_address_value(actor: &AddressRef<PlanActorKey>) -> Value {
    address_value(actor, plan_actor_key_value)
}

fn scope_address_value(scope: &ScopeAddress) -> Value {
    address_value(scope, |segments| scope_identity_value(segments))
}

impl MoveToScope {
    #[must_use]
    pub fn to_value(&self) -> Value {
        Value::object([
            (
                "actors",
                Value::Array(self.actors.iter().map(actor_address_value).collect()),
            ),
            ("target", scope_address_value(&self.target)),
        ])
        .expect("move-to-scope fields are unique")
    }

    pub fn encode(&self, ceilings: Ceilings) -> Result<Vec<u8>, CodecError> {
        encode(&self.to_value(), ceilings)
    }
}

pub fn decode_move_to_scope(
    bytes: &[u8],
    context: AddressContext,
    ceilings: Ceilings,
) -> Result<MoveToScope, PayloadRejection> {
    move_to_scope_from_value(
        decode(bytes, ceilings).map_err(PayloadRejection::Codec)?,
        context,
    )
}

pub fn move_to_scope_from_value(
    value: Value,
    context: AddressContext,
) -> Result<MoveToScope, PayloadRejection> {
    let mut fields = object_from_value(value)?;
    let Value::Array(actor_values) = take(&mut fields, "actors")? else {
        return Err(PayloadRejection::WrongCarrier { key: "actors" });
    };
    let actors = actor_values
        .into_iter()
        .map(decode_actor_address)
        .collect::<Result<Vec<_>, _>>()?;
    let target = decode_scope_address(take(&mut fields, "target")?)?;
    exhausted(fields)?;

    if !context.admits(&target) || actors.iter().any(|actor| !context.admits(actor)) {
        return Err(PayloadRejection::ArmNotAdmitted);
    }

    Ok(MoveToScope { actors, target })
}

pub fn decode_retire_scope(
    bytes: &[u8],
    context: AddressContext,
    ceilings: Ceilings,
) -> Result<RetireScope, PayloadRejection> {
    retire_scope_from_value(
        decode(bytes, ceilings).map_err(PayloadRejection::Codec)?,
        context,
    )
}

#[cfg(test)]
mod scope_tests {
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

    fn binding(scope: &[&str], local: &str, port: &str, outer: &str) -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                (
                    "inner".to_owned(),
                    Value::Array(vec![
                        actor_key(scope, local),
                        Value::String(port.to_owned()),
                    ]),
                ),
                ("outer".to_owned(), Value::String(outer.to_owned())),
            ])
            .expect("two keys"),
        )
    }

    fn sorted(mut items: Vec<Value>) -> Value {
        items.sort_by_key(outer_of);
        Value::Array(items)
    }

    fn outer_of(item: &Value) -> String {
        let Value::Object(object) = item else {
            panic!("it is an object");
        };
        let Some(Value::String(outer)) = object.clone().into_map().remove("outer") else {
            panic!("the outer name is a string");
        };
        outer
    }

    fn boundary(inlets: Vec<Value>, outlets: Vec<Value>) -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                ("inlets".to_owned(), sorted(inlets)),
                ("outlets".to_owned(), sorted(outlets)),
            ])
            .expect("two keys"),
        )
    }

    fn declaration(role: i64, boundary_value: Value) -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                ("boundary".to_owned(), boundary_value),
                ("role".to_owned(), Value::Int(role)),
            ])
            .expect("two keys"),
        )
    }

    fn upsert_body(scope: Value, declaration_value: Value) -> Vec<u8> {
        let object = ObjectValue::try_from_entries([
            ("declaration".to_owned(), declaration_value),
            ("scope".to_owned(), scope),
        ])
        .expect("two keys");
        encode(&Value::Object(object), CEILINGS).expect("encodes")
    }

    fn retire_body(scope: Value) -> Vec<u8> {
        let object = ObjectValue::try_from_entries([("scope".to_owned(), scope)]).expect("one key");
        encode(&Value::Object(object), CEILINGS).expect("encodes")
    }

    fn address(tag: i64, names: &[&str]) -> Value {
        Value::Array(vec![
            Value::Int(tag),
            Value::Array(
                names
                    .iter()
                    .map(|name| {
                        Value::Array(vec![Value::Int(1), Value::String((*name).to_owned())])
                    })
                    .collect(),
            ),
        ])
    }

    #[test]
    fn an_instance_tailed_address_is_admitted() {
        let scope = Value::Array(vec![
            Value::Int(1),
            Value::Array(vec![
                Value::Array(vec![Value::Int(1), Value::String("fleet".to_owned())]),
                Value::Array(vec![
                    Value::Int(2),
                    Value::String("session-cell".to_owned()),
                    Value::String("s-1".to_owned()),
                ]),
            ]),
        ]);
        let bytes = upsert_body(scope, declaration(1, boundary(Vec::new(), Vec::new())));
        let decoded =
            decode_upsert_scope(&bytes, AddressContext::Mutation, CEILINGS).expect("decodes");

        let AddressRef::Absolute(segments) = &decoded.scope else {
            panic!("it is an absolute address");
        };
        assert_eq!(
            segments[1],
            ScopeSegment::Instance {
                of: "session-cell".to_owned(),
                key: InstanceKey::Scalar(ScalarKey::Text("s-1".to_owned())),
            }
        );
    }

    #[test]
    fn a_repeated_binding_is_rejected() {
        let one = binding(&["cell"], "a", "in", "alpha");
        let doubled = Value::Object(
            ObjectValue::try_from_entries([
                ("inlets".to_owned(), Value::Array(vec![one.clone(), one])),
                ("outlets".to_owned(), Value::Array(Vec::new())),
            ])
            .expect("two keys"),
        );
        assert_eq!(
            decode_upsert_scope(
                &upsert_body(address(1, &["cell"]), declaration(1, doubled)),
                AddressContext::Mutation,
                CEILINGS
            ),
            Err(PayloadRejection::NotCanonical { key: "inlets" })
        );
    }

    #[test]
    fn an_unassigned_role_tag_is_rejected() {
        assert_eq!(
            decode_upsert_scope(
                &upsert_body(
                    address(1, &["cell"]),
                    declaration(3, boundary(Vec::new(), Vec::new()))
                ),
                AddressContext::Mutation,
                CEILINGS
            ),
            Err(PayloadRejection::UnknownArm { tag: 3 })
        );
    }

    fn declaration_with_role(role: Value, boundary_value: Value) -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                ("boundary".to_owned(), boundary_value),
                ("role".to_owned(), role),
            ])
            .expect("two keys"),
        )
    }

    #[test]
    fn a_missing_boundary_list_is_not_an_empty_one() {
        let half = Value::Object(
            ObjectValue::try_from_entries([("inlets".to_owned(), Value::Array(Vec::new()))])
                .expect("one key"),
        );
        assert_eq!(
            decode_upsert_scope(
                &upsert_body(address(1, &["cell"]), declaration(1, half)),
                AddressContext::Mutation,
                CEILINGS
            ),
            Err(PayloadRejection::MissingKey("outlets"))
        );
    }

    #[test]
    fn the_inner_side_is_the_published_endpoint_carrier() {
        let flat = Value::Object(
            ObjectValue::try_from_entries([
                ("inner".to_owned(), Value::String("ingest.in".to_owned())),
                ("outer".to_owned(), Value::String("event".to_owned())),
            ])
            .expect("two keys"),
        );
        let boundary_value = Value::Object(
            ObjectValue::try_from_entries([
                ("inlets".to_owned(), Value::Array(vec![flat])),
                ("outlets".to_owned(), Value::Array(Vec::new())),
            ])
            .expect("two keys"),
        );
        assert_eq!(
            decode_upsert_scope(
                &upsert_body(address(1, &["cell"]), declaration(1, boundary_value)),
                AddressContext::Mutation,
                CEILINGS
            ),
            Err(PayloadRejection::WrongCarrier { key: "endpoint" })
        );
    }

    #[test]
    fn the_inner_actor_keeps_its_segments() {
        let bytes = upsert_body(
            address(1, &["fleet", "cell"]),
            declaration(
                2,
                boundary(
                    vec![binding(&["fleet", "cell"], "ingest", "in", "event")],
                    Vec::new(),
                ),
            ),
        );
        let decoded =
            decode_upsert_scope(&bytes, AddressContext::Mutation, CEILINGS).expect("decodes");
        assert_eq!(
            decoded.declaration.boundary.inlets[0].inner.0.scope.len(),
            2
        );
    }

    #[test]
    fn a_retire_carries_only_its_target() {
        let decoded = decode_retire_scope(
            &retire_body(address(1, &["fleet", "cell"])),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("decodes");
        assert_eq!(
            decoded.scope,
            AddressRef::Absolute(vec![
                ScopeSegment::Child("fleet".to_owned()),
                ScopeSegment::Child("cell".to_owned()),
            ])
        );

        let with_declaration = upsert_body(
            address(1, &["fleet", "cell"]),
            declaration(1, boundary(Vec::new(), Vec::new())),
        );
        assert_eq!(
            decode_retire_scope(&with_declaration, AddressContext::Mutation, CEILINGS),
            Err(PayloadRejection::UnknownKey("declaration".to_owned()))
        );
    }

    #[test]
    fn the_context_partitions_the_arms() {
        let bytes = upsert_body(
            address(2, &["draft"]),
            declaration(1, boundary(Vec::new(), Vec::new())),
        );
        assert!(decode_upsert_scope(&bytes, AddressContext::Mutation, CEILINGS).is_ok());
        assert_eq!(
            decode_upsert_scope(&bytes, AddressContext::AcceptedHistory, CEILINGS),
            Err(PayloadRejection::ArmNotAdmitted)
        );
        assert_eq!(
            decode_retire_scope(
                &retire_body(address(2, &["draft"])),
                AddressContext::AcceptedHistory,
                CEILINGS
            ),
            Err(PayloadRejection::ArmNotAdmitted)
        );
    }
    fn scope(tag: i64, names: &[&str]) -> ScopeAddress {
        let segments = names
            .iter()
            .map(|name| ScopeSegment::Child((*name).to_owned()))
            .collect();
        match tag {
            1 => AddressRef::Absolute(segments),
            2 => AddressRef::EpochLocal(segments),
            3 => AddressRef::Relative(segments),
            _ => panic!("test address tag is assigned"),
        }
    }

    fn actor(tag: i64, scope: &[&str], local: &str) -> AddressRef<PlanActorKey> {
        let key = PlanActorKey {
            scope: scope
                .iter()
                .map(|name| ScopeSegment::Child((*name).to_owned()))
                .collect(),
            local: crate::scope_identity::AuthoredLocal::try_new(local)
                .expect("test actor local is authored")
                .into(),
        };
        match tag {
            1 => AddressRef::Absolute(key),
            2 => AddressRef::EpochLocal(key),
            3 => AddressRef::Relative(key),
            _ => panic!("test address tag is assigned"),
        }
    }

    #[test]
    fn move_to_scope_round_trips_the_published_address_carriers() {
        let command = MoveToScope {
            actors: vec![
                actor(1, &["fleet"], "ingest"),
                actor(2, &["draft"], "meter"),
            ],
            target: scope(2, &["folded"]),
        };
        let bytes = command.encode(CEILINGS).expect("encodes");
        assert_eq!(
            decode_move_to_scope(&bytes, AddressContext::Mutation, CEILINGS),
            Ok(command)
        );
    }

    #[test]
    fn an_empty_move_set_is_well_formed_and_decoded() {
        let command = MoveToScope {
            actors: Vec::new(),
            target: scope(1, &["target"]),
        };
        assert_eq!(
            decode_move_to_scope(
                &command.encode(CEILINGS).expect("encodes"),
                AddressContext::Mutation,
                CEILINGS,
            ),
            Ok(command)
        );
    }

    #[test]
    fn target_and_every_actor_must_be_admitted_by_the_context() {
        let disallowed_target = MoveToScope {
            actors: vec![actor(1, &["root"], "one")],
            target: scope(2, &["draft"]),
        };
        assert_eq!(
            decode_move_to_scope(
                &disallowed_target.encode(CEILINGS).expect("encodes"),
                AddressContext::AcceptedHistory,
                CEILINGS,
            ),
            Err(PayloadRejection::ArmNotAdmitted)
        );

        let disallowed_second_actor = MoveToScope {
            actors: vec![actor(1, &["root"], "one"), actor(2, &["draft"], "two")],
            target: scope(1, &["target"]),
        };
        assert_eq!(
            decode_move_to_scope(
                &disallowed_second_actor.encode(CEILINGS).expect("encodes"),
                AddressContext::AcceptedHistory,
                CEILINGS,
            ),
            Err(PayloadRejection::ArmNotAdmitted)
        );

        let snapshot = MoveToScope {
            actors: vec![actor(3, &["root"], "one"), actor(3, &["root"], "two")],
            target: scope(3, &["target"]),
        };
        assert!(
            decode_move_to_scope(
                &snapshot.encode(CEILINGS).expect("encodes"),
                AddressContext::AuthoringSnapshot,
                CEILINGS,
            )
            .is_ok()
        );
    }

    fn raw_entry(bytes: &mut Vec<u8>, key: &str, value: &Value) {
        bytes.extend_from_slice(&u32::try_from(key.len()).expect("short key").to_be_bytes());
        bytes.extend_from_slice(key.as_bytes());
        bytes.extend_from_slice(&encode(value, CEILINGS).expect("value encodes"));
    }

    #[test]
    fn target_before_actors_is_rejected_as_a_noncanonical_key_order() {
        let command = MoveToScope {
            actors: vec![actor(1, &["root"], "one")],
            target: scope(1, &["target"]),
        };
        let Value::Object(object) = command.to_value() else {
            panic!("command is an object");
        };
        let mut fields = object.into_map();
        let actors = fields.remove("actors").expect("actors field");
        let target = fields.remove("target").expect("target field");

        let mut bytes = vec![8];
        bytes.extend_from_slice(&2_u32.to_be_bytes());
        raw_entry(&mut bytes, "target", &target);
        raw_entry(&mut bytes, "actors", &actors);

        assert_eq!(
            decode_move_to_scope(&bytes, AddressContext::Mutation, CEILINGS),
            Err(PayloadRejection::Codec(CodecError::ObjectKeyOrder))
        );
    }
}
