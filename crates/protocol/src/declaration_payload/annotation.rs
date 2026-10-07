
use circular_core::{Ceilings, Value, decode};

use crate::scope_identity::{
    AddressContext, AddressRef, PlanActorKey, ScopeSegment, decode_address, decode_plan_actor_key,
    decode_scope_identity,
};
use crate::wire_value::{
    PayloadRejection, decode_arm, decode_canonical_value_set, exhausted, object_fields,
    object_from_value, take, text_of,
};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct PlanAnnotationKey {
    pub scope: Vec<ScopeSegment>,
    pub local: String,
}

circular_core::closed_table! {
    pub enum AnnotationKind: i64 {
        Note = 1,
        Backdrop = 2,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnnotationDeclaration {
    pub kind: AnnotationKind,
    pub refs: Vec<PlanActorKey>,
    pub body: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpsertAnnotation {
    pub annotation: AddressRef<PlanAnnotationKey>,
    pub declaration: AnnotationDeclaration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetireAnnotation {
    pub annotation: AddressRef<PlanAnnotationKey>,
}

fn decode_plan_annotation_key(value: Value) -> Result<PlanAnnotationKey, PayloadRejection> {
    let mut fields = object_fields(value, "annotation")?;
    let local = text_of(take(&mut fields, "local")?, "local")?;
    let scope = decode_scope_identity(take(&mut fields, "scope")?)?;
    exhausted(fields)?;
    Ok(PlanAnnotationKey { scope, local })
}

pub fn decode_annotation_address(
    value: Value,
) -> Result<AddressRef<PlanAnnotationKey>, PayloadRejection> {
    decode_address(value, decode_plan_annotation_key)
}

fn decode_annotation_kind(value: Value) -> Result<AnnotationKind, PayloadRejection> {
    let (tag, arguments) = decode_arm(value, "kind")?;
    if !arguments.is_empty() {
        return Err(PayloadRejection::WrongCarrier { key: "kind" });
    }
    AnnotationKind::from_tag(tag).ok_or(PayloadRejection::UnknownArm { tag })
}

fn decode_annotation_declaration(value: Value) -> Result<AnnotationDeclaration, PayloadRejection> {
    let mut fields = object_fields(value, "declaration")?;
    let body = text_of(take(&mut fields, "body")?, "body")?;
    let kind = decode_annotation_kind(take(&mut fields, "kind")?)?;
    let refs =
        decode_canonical_value_set(take(&mut fields, "refs")?, "refs", decode_plan_actor_key)?;
    exhausted(fields)?;
    Ok(AnnotationDeclaration { kind, refs, body })
}

pub fn upsert_annotation_from_value(
    value: Value,
    context: AddressContext,
) -> Result<UpsertAnnotation, PayloadRejection> {
    let mut fields = object_from_value(value)?;
    let annotation = decode_annotation_address(take(&mut fields, "annotation")?)?;
    let declaration = decode_annotation_declaration(take(&mut fields, "declaration")?)?;
    exhausted(fields)?;

    if !context.admits(&annotation) {
        return Err(PayloadRejection::ArmNotAdmitted);
    }

    Ok(UpsertAnnotation {
        annotation,
        declaration,
    })
}

pub fn decode_upsert_annotation(
    bytes: &[u8],
    context: AddressContext,
    ceilings: Ceilings,
) -> Result<UpsertAnnotation, PayloadRejection> {
    upsert_annotation_from_value(
        decode(bytes, ceilings).map_err(PayloadRejection::Codec)?,
        context,
    )
}

pub fn retire_annotation_from_value(
    value: Value,
    context: AddressContext,
) -> Result<RetireAnnotation, PayloadRejection> {
    let mut fields = object_from_value(value)?;
    let annotation = decode_annotation_address(take(&mut fields, "annotation")?)?;
    exhausted(fields)?;

    if !context.admits(&annotation) {
        return Err(PayloadRejection::ArmNotAdmitted);
    }

    Ok(RetireAnnotation { annotation })
}

pub fn decode_retire_annotation(
    bytes: &[u8],
    context: AddressContext,
    ceilings: Ceilings,
) -> Result<RetireAnnotation, PayloadRejection> {
    retire_annotation_from_value(
        decode(bytes, ceilings).map_err(PayloadRejection::Codec)?,
        context,
    )
}

#[cfg(test)]
mod annotation_tests {
    use super::*;
    use crate::declaration_payload::presentation::decode_set_presentation;
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

    fn body(owner: Value, presentation: Value) -> Vec<u8> {
        let object = ObjectValue::try_from_entries([
            ("owner".to_owned(), owner),
            ("presentation".to_owned(), presentation),
        ])
        .expect("two keys");
        encode(&Value::Object(object), CEILINGS).expect("encodes")
    }

    fn axes(entries: Vec<(&str, Value)>) -> Value {
        Value::Object(
            ObjectValue::try_from_entries(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value)),
            )
            .expect("keys do not overlap"),
        )
    }

    fn presentation_value(entries: Vec<(&str, Value)>) -> Value {
        let mut entries = entries;
        if !entries.iter().any(|(key, _)| *key == "collapsed") {
            entries.push(("collapsed", Value::Bool(false)));
        }
        axes(entries)
    }

    fn annotation(scope: &[&str], local: &str) -> Value {
        Value::Array(vec![Value::Int(1), key_object(scope, local)])
    }

    fn annotation_body(address: Value, declaration: Value) -> Vec<u8> {
        let object = ObjectValue::try_from_entries([
            ("annotation".to_owned(), address),
            ("declaration".to_owned(), declaration),
        ])
        .expect("two keys");
        encode(&Value::Object(object), CEILINGS).expect("encodes")
    }

    fn refs(items: Vec<Value>) -> Value {
        let mut items = items;
        items.sort_by_key(|item| encode(item, CEILINGS).expect("encodes"));
        Value::Array(items)
    }

    #[test]
    fn an_annotation_opens_with_the_canonical_shape() {
        let decoded = decode_upsert_annotation(
            &annotation_body(
                annotation(&["gate"], "why-filtered"),
                axes(vec![
                    (
                        "body",
                        Value::String("filter out the noise here".to_owned()),
                    ),
                    ("kind", Value::Int(1)),
                    (
                        "refs",
                        refs(vec![
                            key_object(&["gate"], "filter"),
                            key_object(&["gate"], "map"),
                        ]),
                    ),
                ]),
            ),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("decodes");

        assert_eq!(decoded.declaration.kind, AnnotationKind::Note);
        assert_eq!(decoded.declaration.refs.len(), 2);
        let AddressRef::Absolute(key) = &decoded.annotation else {
            panic!("it is an absolute address");
        };
        assert_eq!(key.local, "why-filtered");
        assert_eq!(key.scope.len(), 1);
    }

    #[test]
    fn an_unassigned_annotation_kind_is_rejected() {
        assert_eq!(
            decode_upsert_annotation(
                &annotation_body(
                    annotation(&["gate"], "a"),
                    axes(vec![
                        ("body", Value::String("b".to_owned())),
                        ("kind", Value::Int(3)),
                        ("refs", Value::Array(Vec::new())),
                    ])
                ),
                AddressContext::Mutation,
                CEILINGS
            ),
            Err(PayloadRejection::UnknownArm { tag: 3 })
        );
    }

    #[test]
    fn an_annotation_key_keeps_its_segments() {
        let decoded = decode_retire_annotation(
            &{
                let object = ObjectValue::try_from_entries([(
                    "annotation".to_owned(),
                    annotation(&["fleet", "cell"], "note"),
                )])
                .expect("one key");
                encode(&Value::Object(object), CEILINGS).expect("encodes")
            },
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("decodes");

        let AddressRef::Absolute(key) = &decoded.annotation else {
            panic!("it is an absolute address");
        };
        assert_eq!(key.scope.len(), 2, "the two segments were flattened");
        assert_eq!(key.local, "note");
    }

    #[test]
    fn a_retire_annotation_carries_only_its_target() {
        assert_eq!(
            decode_retire_annotation(
                &annotation_body(
                    annotation(&["gate"], "a"),
                    axes(vec![
                        ("body", Value::String("b".to_owned())),
                        ("kind", Value::Int(1)),
                        ("refs", Value::Array(Vec::new())),
                    ])
                ),
                AddressContext::Mutation,
                CEILINGS
            ),
            Err(PayloadRejection::UnknownKey("declaration".to_owned()))
        );
    }

    #[test]
    fn the_context_partitions_the_arms() {
        let epoch_local = Value::Array(vec![Value::Int(2), key_object(&["gate"], "x")]);
        assert_eq!(
            decode_set_presentation(
                &body(
                    Value::object([("actor", epoch_local.clone())]).unwrap(),
                    presentation_value(Vec::new())
                ),
                AddressContext::AcceptedHistory,
                CEILINGS
            ),
            Err(PayloadRejection::ArmNotAdmitted)
        );
        assert_eq!(
            decode_upsert_annotation(
                &annotation_body(
                    epoch_local.clone(),
                    axes(vec![
                        ("body", Value::String("b".to_owned())),
                        ("kind", Value::Int(2)),
                        ("refs", Value::Array(Vec::new())),
                    ])
                ),
                AddressContext::AcceptedHistory,
                CEILINGS
            ),
            Err(PayloadRejection::ArmNotAdmitted)
        );
        let retire = {
            let object = ObjectValue::try_from_entries([("annotation".to_owned(), epoch_local)])
                .expect("one key");
            encode(&Value::Object(object), CEILINGS).expect("encodes")
        };
        assert_eq!(
            decode_retire_annotation(&retire, AddressContext::AcceptedHistory, CEILINGS),
            Err(PayloadRejection::ArmNotAdmitted)
        );
    }
}
