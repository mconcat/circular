
use crate::RecordRowCodec;
use crate::memory::AppendBatch;
use crate::product_identity::{
    ProductIdentityError, actor_from_value, actor_parts_from_value, actor_value, edge_value,
    identity_bytes, identity_value, scope_value,
};
use crate::product_schema::{OpaqueId, ProductStore, ProductTransaction, StreamId};
use crate::record::{
    ArrivalKey, ArrivalOrigin, BoundaryKey, BoundaryRecord, ClassKey, Record, RecordOrigin,
    StructureKey,
};
use crate::record_codec::{
    EnvelopeOrigin, RecordCodecError, RecordIdentityCodec, decode_envelope, encode_record,
};
use crate::rehydrate::JournalProjection;
use crate::transaction::{
    StoreTransaction, StoreTransactionOp, TransactionAppend, TransactionCheckpoint,
    TransactionObservation,
};
use circular_core::{
    EncodedPayload, EventId, Hlc, LogicalCounter, RecordedInstant, Sequence, Stamp, Tick,
};
use circular_runtime::ActorId;

pub struct ProductRecordCodec;

fn take<'bytes>(bytes: &'bytes [u8], cursor: &mut usize) -> Option<&'bytes [u8]> {
    let length = u32::from_be_bytes(bytes.get(*cursor..*cursor + 4)?.try_into().ok()?) as usize;
    *cursor += 4;
    let slice = bytes.get(*cursor..*cursor + length)?;
    *cursor += length;
    Some(slice)
}

fn push_bytes(output: &mut Vec<u8>, bytes: &[u8]) {
    output.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    output.extend_from_slice(bytes);
}

fn actor_bytes(actor: &ActorId) -> Result<Vec<u8>, RecordCodecError> {
    actor_value(actor)
        .and_then(|value| identity_bytes(&value))
        .map_err(RecordCodecError::Identity)
}

fn scope_bytes(scope: &circular_runtime::ScopeId) -> Result<Vec<u8>, RecordCodecError> {
    scope_value(scope)
        .and_then(|value| identity_bytes(&value))
        .map_err(RecordCodecError::Identity)
}

fn observation_name_tag_or_reject(
    name: circular_core::BuiltinObservationName,
) -> Result<u8, RecordCodecError> {
    let tag = name.tag();
    if circular_core::BuiltinObservationName::from_tag(tag) == Some(name) {
        Ok(tag)
    } else {
        Err(RecordCodecError::Identity(
            ProductIdentityError::UnpublishedArm("BuiltinObservationName"),
        ))
    }
}

pub(crate) fn observation_name_of_tag(tag: u8) -> Option<circular_core::BuiltinObservationName> {
    circular_core::BuiltinObservationName::from_tag(tag)
}

pub fn display_key_bytes(
    key: &crate::product_schema::DisplayKey,
) -> Result<Vec<u8>, RecordCodecError> {
    let mut bytes = Vec::new();
    bytes.push(observation_name_tag_or_reject(key.name())?);
    push_bytes(
        &mut bytes,
        &actor_bytes(&ActorId::from(key.args().clone()))?,
    );
    match key.bucket() {
        None => bytes.push(0),
        Some(bucket) => {
            bytes.push(1);
            bytes.extend_from_slice(&bucket.get().get().to_be_bytes());
        }
    }
    Ok(bytes)
}

pub fn display_key_from_bytes(bytes: &[u8]) -> Option<crate::product_schema::DisplayKey> {
    let mut cursor = 0_usize;
    let name = observation_name_of_tag(*bytes.first()?)?;
    cursor += 1;
    let args = actor_parts_from_value(&identity_value(take(bytes, &mut cursor)?).ok()?).ok()?;
    let bucket = match *bytes.get(cursor)? {
        0 => {
            cursor += 1;
            None
        }
        1 => {
            cursor += 1;
            let bucket = circular_core::NonZeroMillis::new(u64::from_be_bytes(
                bytes.get(cursor..cursor + 8)?.try_into().ok()?,
            ))
            .ok()?;
            cursor += 8;
            Some(bucket)
        }
        _ => return None,
    };
    if cursor != bytes.len() {
        return None;
    }
    let key = crate::product_schema::DisplayKey::new(
        name,
        circular_runtime::NamedActorId::new(args.0, args.1),
        bucket,
    );
    (display_key_bytes(&key).ok()?.as_slice() == bytes).then_some(key)
}

impl RecordIdentityCodec<ProductStore> for ProductRecordCodec {
    fn producer(&self, producer: &ActorId) -> Result<Vec<u8>, RecordCodecError> {
        crate::product_identity::record_actor_value(producer)
            .and_then(|value| identity_bytes(&value))
            .map_err(RecordCodecError::Identity)
    }

    fn origin_body(
        &self,
        origin: &RecordOrigin<ProductStore>,
    ) -> Result<Vec<u8>, RecordCodecError> {
        Ok(match origin {
            RecordOrigin::Actor(incarnation) => incarnation.get().to_be_bytes().to_vec(),
            RecordOrigin::Stream => Vec::new(),
        })
    }

    fn class_key_body(&self, key: &ClassKey<ProductStore>) -> Result<Vec<u8>, RecordCodecError> {
        match key {
            ClassKey::Boundary(BoundaryKey::ScheduleReservation { effect, .. }) => {
                circular_runtime::EffectId::encode(effect).map_err(RecordCodecError::Identity)
            }
            ClassKey::Boundary(
                BoundaryKey::Arrival { actor, origin, .. }
                | BoundaryKey::Admission { actor, origin, .. },
            ) => {
                let mut bytes = Vec::new();
                push_bytes(&mut bytes, &actor_bytes(actor)?);
                match origin.as_ref() {
                    ArrivalKey::EdgeDelivery { edge, sender } => {
                        bytes.push(1);
                        push_bytes(
                            &mut bytes,
                            &edge_value(edge)
                                .and_then(|value| identity_bytes(&value))
                                .map_err(RecordCodecError::Identity)?,
                        );
                        push_stamp(&mut bytes, sender)?;
                    }
                    ArrivalKey::TimerFire { timer } => {
                        bytes.push(2);
                        push_bytes(
                            &mut bytes,
                            &circular_runtime::EffectId::encode(timer)
                                .map_err(RecordCodecError::Identity)?,
                        );
                    }
                    ArrivalKey::EffectOutcome { effect } => {
                        bytes.push(3);
                        push_bytes(
                            &mut bytes,
                            &circular_runtime::EffectId::encode(effect)
                                .map_err(RecordCodecError::Identity)?,
                        );
                    }
                    ArrivalKey::ExternalInject { origin, route_edge } => {
                        bytes.push(4);
                        push_bytes(&mut bytes, origin.as_bytes());
                        push_route_edge(&mut bytes, route_edge.as_ref())?;
                    }
                }
                Ok(bytes)
            }
            ClassKey::Structure(StructureKey::Revision(scope, _)) => scope_bytes(scope),
            ClassKey::Observation(key) => {
                let mut bytes = Vec::new();
                bytes.push(observation_name_tag_or_reject(*key.item().kind())?);
                bytes.extend_from_slice(&key.item().identity().get().to_be_bytes());
                Ok(bytes)
            }
            ClassKey::Display { key, .. } => display_key_bytes(key),
            _ => Ok(Vec::new()),
        }
    }

    fn payload(&self, record: &Record<ProductStore>) -> Result<Vec<u8>, RecordCodecError> {
        let body = match record {
            Record::Boundary(boundary) => match boundary.fact() {
                crate::BoundaryFact::EmissionBody {
                    port,
                    cause,
                    observed_at,
                    payload,
                } => emission_payload_bytes(port, *cause, *observed_at, payload),
                crate::BoundaryFact::ScheduleReservation { payload } => {
                    crate::schedule_reservation::read_schedule_reservation(boundary)
                        .map_err(|_| RecordCodecError::LengthOutOfRange)?;
                    payload.as_bytes().to_vec()
                }
                _ => arrival_payload_bytes(boundary.fact())?,
            },
            Record::Display(display) => display.payload().as_bytes().to_vec(),
            Record::Observation(observation) => match observation.fact() {
                crate::record::ObservationFact::Lifecycle(payload)
                | crate::record::ObservationFact::Diagnostic(payload)
                | crate::record::ObservationFact::Accounting(payload)
                | crate::record::ObservationFact::DeadLetter(payload)
                | crate::record::ObservationFact::ReplaySessionTransition(payload)
                | crate::record::ObservationFact::Restart(payload)
                | crate::record::ObservationFact::Checkpoint(payload) => {
                    payload.as_bytes().to_vec()
                }
            },
            Record::Structure(structure) => match structure.fact() {
                crate::record::StructureFact::RunManifest(manifest) => manifest_bytes(manifest)?,
                crate::record::StructureFact::GraphRevision(revision) => {
                    revision.as_bytes().to_vec()
                }
            },
        };
        Ok(body)
    }
}

pub fn push_stamp(output: &mut Vec<u8>, stamp: &Stamp<ActorId>) -> Result<(), RecordCodecError> {
    output.extend_from_slice(
        &circular_runtime::encode_effect_stamp(stamp).map_err(RecordCodecError::Identity)?,
    );
    Ok(())
}

pub fn take_stamp(bytes: &[u8], cursor: &mut usize) -> Option<Stamp<ActorId>> {
    let start = *cursor;
    let length = u32::from_be_bytes(bytes.get(start + 16..start + 20)?.try_into().ok()?) as usize;
    let end = start.checked_add(36)?.checked_add(length)?;
    let stamp = circular_runtime::decode_effect_stamp(bytes.get(start..end)?).ok()?;
    *cursor = end;
    Some(stamp)
}

const ARRIVAL_EVENT: u8 = 1;
const ARRIVAL_EFFECT_OUTCOME: u8 = 2;
const ARRIVAL_EVENT_EMITTED_REFERENCE: u8 = 4;

fn emission_payload_bytes(
    port: &circular_core::PortId,
    cause: circular_core::ArrivalIndex,
    observed_at: RecordedInstant,
    payload: &EncodedPayload,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    push_bytes(&mut bytes, port.as_str().as_bytes());
    bytes.extend_from_slice(&cause.get().to_be_bytes());
    bytes.extend_from_slice(&observed_at.millis().to_be_bytes());
    bytes.extend_from_slice(payload.as_bytes());
    bytes
}

fn emission_fact_from_bytes(bytes: &[u8]) -> Option<crate::EmissionFact> {
    let mut cursor = 0;
    let port = take(bytes, &mut cursor)?;
    let port =
        circular_core::PortId::try_derived(std::str::from_utf8(port).ok()?.to_owned()).ok()?;
    let mut number = || {
        let value = u64::from_be_bytes(bytes.get(cursor..cursor + 8)?.try_into().ok()?);
        cursor += 8;
        Some(value)
    };
    let cause = circular_core::ArrivalIndex::new(number()?);
    let observed_at = RecordedInstant::from_millis(number()?);
    let payload = EncodedPayload::from_bytes(bytes.get(cursor..)?).ok()?;
    Some(crate::EmissionFact {
        port,
        cause,
        observed_at,
        payload,
    })
}

fn arrival_payload_bytes(
    fact: &crate::record::BoundaryFact<ProductStore>,
) -> Result<Vec<u8>, RecordCodecError> {
    let (crate::record::BoundaryFact::Arrival {
        origin,
        body,
        causal_parents,
        inlet,
        route_edge,
        result,
        ..
    }
    | crate::record::BoundaryFact::Admission {
        origin,
        body,
        causal_parents,
        inlet,
        route_edge,
        result,
        ..
    }) = fact
    else {
        return Err(RecordCodecError::LengthOutOfRange);
    };
    let outcome = matches!(origin.as_ref(), ArrivalOrigin::EffectOutcome { .. });
    let mut bytes = Vec::new();
    match body {
        crate::record::ArrivalBody::Owned(payload) => {
            bytes.push(if outcome {
                ARRIVAL_EFFECT_OUTCOME
            } else {
                ARRIVAL_EVENT
            });
            push_bytes(&mut bytes, payload.as_bytes());
        }
        crate::record::ArrivalBody::Emitted { .. } if outcome => {
            return Err(RecordCodecError::LengthOutOfRange);
        }
        crate::record::ArrivalBody::Emitted { producer, sequence } => {
            let ArrivalOrigin::EdgeDelivery { sender, .. } = origin.as_ref() else {
                return Err(RecordCodecError::OriginFactMismatch);
            };
            if sender.producer() != producer || sender.sequence() != *sequence {
                return Err(RecordCodecError::OriginFactMismatch);
            }
            bytes.push(ARRIVAL_EVENT_EMITTED_REFERENCE);
            push_bytes(&mut bytes, &ProductRecordCodec.producer(producer)?);
            bytes.extend_from_slice(&sequence.get().to_be_bytes());
        }
    }
    let parent_count =
        u32::try_from(causal_parents.len()).map_err(|_| RecordCodecError::LengthOutOfRange)?;
    bytes.extend_from_slice(&parent_count.to_be_bytes());
    for parent in causal_parents.iter() {
        bytes.extend_from_slice(&parent.stream().get().to_be_bytes());
        push_stamp(&mut bytes, parent.stamp())?;
    }
    if let ArrivalOrigin::EffectOutcome { term, outcome, .. } = origin.as_ref() {
        push_bytes(&mut bytes, term.as_bytes());
        push_bytes(&mut bytes, outcome.as_bytes());
    }
    match (origin.as_ref(), inlet) {
        (ArrivalOrigin::EffectOutcome { .. }, None) => push_bytes(&mut bytes, &[]),
        (ArrivalOrigin::EffectOutcome { .. }, Some(_)) | (_, None) => {
            return Err(RecordCodecError::LengthOutOfRange);
        }
        (_, Some(inlet)) => push_bytes(&mut bytes, inlet.as_str().as_bytes()),
    }
    if !route_matches_origin(origin, route_edge.as_ref()) {
        return Err(RecordCodecError::LengthOutOfRange);
    }
    if !matches!(origin.as_ref(), ArrivalOrigin::EffectOutcome { .. }) {
        push_bytes(&mut bytes, &crate::arrival_result::encode(result)?);
    } else if !matches!(result, circular_runtime::EnvelopeResult::Ok) {
        return Err(RecordCodecError::LengthOutOfRange);
    }
    Ok(bytes)
}

fn push_route_edge(
    bytes: &mut Vec<u8>,
    route: Option<&circular_runtime::EdgeId>,
) -> Result<(), RecordCodecError> {
    match route {
        None => bytes.push(0),
        Some(edge) => {
            bytes.push(1);
            push_bytes(
                bytes,
                &edge_value(edge)
                    .and_then(|v| identity_bytes(&v))
                    .map_err(RecordCodecError::Identity)?,
            );
        }
    }
    Ok(())
}

fn take_route_edge(bytes: &[u8], cursor: &mut usize) -> Option<Option<circular_runtime::EdgeId>> {
    let tag = *bytes.get(*cursor)?;
    *cursor += 1;
    match tag {
        0 => Some(None),
        1 => Some(Some(
            crate::product_identity::edge_from_value(&identity_value(take(bytes, cursor)?).ok()?)
                .ok()?,
        )),
        _ => None,
    }
}

fn route_matches_origin(
    origin: &ArrivalOrigin<ProductStore>,
    route: Option<&circular_runtime::EdgeId>,
) -> bool {
    match origin {
        ArrivalOrigin::EdgeDelivery { edge, .. } => route == Some(edge),
        ArrivalOrigin::ExternalInject { .. } => true,
        ArrivalOrigin::TimerFire { .. } | ArrivalOrigin::EffectOutcome { .. } => route.is_none(),
    }
}

struct RebuiltArrivalPayload {
    body: crate::record::ArrivalBody<ActorId>,
    causal_parents: Box<[EventId<StreamId, ActorId>]>,
    effect: Option<(EncodedPayload, EncodedPayload)>,
    inlet: Option<circular_core::PortId>,
    result: circular_runtime::EnvelopeResult,
}

fn rebuild_arrival_payload(bytes: &[u8]) -> Option<RebuiltArrivalPayload> {
    let mut cursor = 1;
    let kind = *bytes.first()?;
    let body = match kind {
        ARRIVAL_EVENT_EMITTED_REFERENCE => {
            let producer = crate::product_identity::record_actor_from_value(
                &identity_value(take(bytes, &mut cursor)?).ok()?,
            )
            .ok()?;
            let sequence = Sequence::new(u64::from_be_bytes(
                bytes.get(cursor..cursor + 8)?.try_into().ok()?,
            ))
            .ok()?;
            cursor += 8;
            crate::ArrivalBody::Emitted { producer, sequence }
        }
        ARRIVAL_EVENT | ARRIVAL_EFFECT_OUTCOME => {
            crate::ArrivalBody::Owned(EncodedPayload::from_bytes(take(bytes, &mut cursor)?).ok()?)
        }
        _ => return None,
    };
    let count_end = cursor.checked_add(4)?;
    let count = u32::from_be_bytes(bytes.get(cursor..count_end)?.try_into().ok()?) as usize;
    cursor = count_end;
    if count > bytes.get(cursor..)?.len() / (8 + 8 + 8 + 4 + 8 + 8) {
        return None;
    }
    let mut causal_parents = Vec::with_capacity(count);
    for _ in 0..count {
        let run = StreamId::new(u64::from_be_bytes(
            bytes.get(cursor..cursor + 8)?.try_into().ok()?,
        ));
        cursor += 8;
        causal_parents.push(EventId::derive(run, take_stamp(bytes, &mut cursor)?));
    }
    let effect = match kind {
        ARRIVAL_EVENT | ARRIVAL_EVENT_EMITTED_REFERENCE => None,
        ARRIVAL_EFFECT_OUTCOME => Some((
            EncodedPayload::from_bytes(take(bytes, &mut cursor)?).ok()?,
            EncodedPayload::from_bytes(take(bytes, &mut cursor)?).ok()?,
        )),
        _ => return None,
    };
    let port = take(bytes, &mut cursor)?;
    let inlet = if kind == ARRIVAL_EFFECT_OUTCOME {
        if !port.is_empty() {
            return None;
        }
        None
    } else {
        Some(circular_core::PortId::try_derived(std::str::from_utf8(port).ok()?.to_owned()).ok()?)
    };
    let result = if kind == ARRIVAL_EFFECT_OUTCOME {
        circular_runtime::EnvelopeResult::Ok
    } else {
        crate::arrival_result::decode(take(bytes, &mut cursor)?)?
    };
    (cursor == bytes.len()).then_some(RebuiltArrivalPayload {
        result,
        body,
        causal_parents: causal_parents.into_boxed_slice(),
        effect,
        inlet,
    })
}

fn time_source_kind_tag(kind: circular_core::TimeSourceKind) -> u8 {
    match kind {
        circular_core::TimeSourceKind::WallClockAnchored => 1,
        circular_core::TimeSourceKind::LogDriven => 2,
        circular_core::TimeSourceKind::Manual => 3,
    }
}

fn time_source_kind_of_tag(tag: u8) -> Option<circular_core::TimeSourceKind> {
    match tag {
        1 => Some(circular_core::TimeSourceKind::WallClockAnchored),
        2 => Some(circular_core::TimeSourceKind::LogDriven),
        3 => Some(circular_core::TimeSourceKind::Manual),
        _ => None,
    }
}

/// Canonical five-field authoring-cut value. The existing value, environment,
/// project-text, cursor and revision carriers are reused without recomputation.
pub fn authoring_cut_bytes(
    cut: &crate::product_schema::ProductAuthoringCut,
) -> Result<Vec<u8>, RecordCodecError> {
    let environment = circular_protocol::authoring_snapshot::environment_value(&cut.environment)
        .map_err(|_| RecordCodecError::Identity(ProductIdentityError::Codec))?;
    let value = circular_core::Value::object([
        ("project", circular_core::Value::bytes(cut.project)),
        ("cursor", circular_core::Value::UInt(cut.cursor)),
        ("environment", environment),
        (
            "authoring_revision",
            circular_core::Value::bytes(*cut.authoring_revision.as_bytes()),
        ),
        (
            "topology_revision",
            circular_core::Value::bytes(*cut.topology_revision.as_bytes()),
        ),
    ])
    .map_err(|_| RecordCodecError::Identity(ProductIdentityError::Codec))?;
    identity_bytes(&value).map_err(RecordCodecError::Identity)
}

/// Read all five components, rejecting old opaque revisions, missing/extra
/// fields, wrong carriers and wrong digest widths. There is no compatibility
/// fallback that fabricates the absent project or other cut components.
pub fn authoring_cut_from_bytes(
    bytes: &[u8],
) -> Option<crate::product_schema::ProductAuthoringCut> {
    use circular_core::Value;
    use circular_protocol::{AuthoringRevision, RevisionDigest, TopologyRevision};

    let Value::Object(object) = identity_value(bytes).ok()? else {
        return None;
    };
    let mut fields = object.into_map();
    let Value::Bytes(project) = fields.remove("project")? else {
        return None;
    };
    let cursor = match fields.remove("cursor")? {
        Value::UInt(cursor) => cursor,
        _ => return None,
    };
    let environment =
        circular_protocol::declaration_payload::decode_environment(fields.remove("environment")?)
            .ok()?;
    let Value::Bytes(authoring_revision) = fields.remove("authoring_revision")? else {
        return None;
    };
    let Value::Bytes(topology_revision) = fields.remove("topology_revision")? else {
        return None;
    };
    if !fields.is_empty() {
        return None;
    }
    Some(crate::manifest::AuthoringCut {
        project: project.try_into().ok()?,
        cursor,
        environment,
        authoring_revision: RevisionDigest::<AuthoringRevision>::try_from_bytes(
            &authoring_revision,
        )
        .ok()?,
        topology_revision: RevisionDigest::<TopologyRevision>::try_from_bytes(&topology_revision)
            .ok()?,
    })
}

pub fn manifest_bytes(
    manifest: &crate::manifest::RunManifest<ProductStore>,
) -> Result<Vec<u8>, RecordCodecError> {
    use crate::manifest::RevisionStart;

    let groups = manifest.groups();
    let time = groups.time();
    let mut bytes = vec![0; 4];
    bytes.extend_from_slice(&manifest.stream().get().to_be_bytes());

    bytes.extend_from_slice(&time.resolution().get().to_be_bytes());
    bytes.extend_from_slice(&time.cadence().get().get().to_be_bytes());
    push_bytes(&mut bytes, time.cadence_policy().as_bytes());
    match time.time_source_plan() {
        circular_core::TimeSourcePlan::Single(kind) => {
            bytes.push(1);
            bytes.push(time_source_kind_tag(*kind));
        }
        circular_core::TimeSourcePlan::Switched { head, at, tail } => {
            bytes.push(2);
            bytes.push(time_source_kind_tag(*head));
            push_stamp(&mut bytes, at)?;
            bytes.push(time_source_kind_tag(*tail));
        }
    }
    bytes.extend_from_slice(&time.tick_origin().get().to_be_bytes());
    push_bytes(&mut bytes, groups.placement().value().as_bytes());
    push_bytes(&mut bytes, groups.failure().value().as_bytes());
    push_bytes(&mut bytes, groups.versions().as_bytes());

    match groups.revision().start() {
        RevisionStart::Fresh(authoring) => {
            bytes.push(1);
            push_bytes(&mut bytes, &authoring_cut_bytes(authoring)?);
        }
    }
    let grants = groups.revision().grants();
    bytes.extend_from_slice(&(grants.len() as u32).to_be_bytes());
    for (scope, grant) in grants {
        push_bytes(&mut bytes, &scope_bytes(scope)?);
        push_bytes(&mut bytes, grant.as_bytes());
    }

    push_bytes(&mut bytes, groups.inputs().value().as_bytes());
    Ok(bytes)
}

pub fn manifest_from_bytes(bytes: &[u8]) -> Option<crate::manifest::RunManifest<ProductStore>> {
    use crate::manifest::{
        FailureParams, ManifestGroups, PlacementParams, RevisionContext, RevisionStart, RunInputs,
        RunManifest, TimeParams,
    };

    if bytes.get(..4)? != [0; 4] {
        return None;
    }
    let mut cursor = 4_usize;
    let u64_at = |cursor: &mut usize| -> Option<u64> {
        let value = u64::from_be_bytes(bytes.get(*cursor..*cursor + 8)?.try_into().ok()?);
        *cursor += 8;
        Some(value)
    };

    let run = StreamId::new(u64_at(&mut cursor)?);
    let resolution = circular_core::TicksPerSecond::new(u32::from_be_bytes(
        bytes.get(cursor..cursor + 4)?.try_into().ok()?,
    ))
    .ok()?;
    cursor += 4;
    let cadence = circular_core::NonZeroTicks::new(u64_at(&mut cursor)?).ok()?;
    let cadence_policy = EncodedPayload::from_bytes(take(bytes, &mut cursor)?).ok()?;
    let plan = match *bytes.get(cursor)? {
        1 => {
            cursor += 1;
            let kind = time_source_kind_of_tag(*bytes.get(cursor)?)?;
            cursor += 1;
            circular_core::TimeSourcePlan::Single(kind)
        }
        2 => {
            cursor += 1;
            let head = time_source_kind_of_tag(*bytes.get(cursor)?)?;
            cursor += 1;
            let at = take_stamp(bytes, &mut cursor)?;
            let tail = time_source_kind_of_tag(*bytes.get(cursor)?)?;
            cursor += 1;
            circular_core::TimeSourcePlan::Switched { head, at, tail }
        }
        _ => return None,
    };
    let tick_origin = OpaqueId::new(u64_at(&mut cursor)?);
    let placement = EncodedPayload::from_bytes(take(bytes, &mut cursor)?).ok()?;
    let failure = EncodedPayload::from_bytes(take(bytes, &mut cursor)?).ok()?;
    let versions = EncodedPayload::from_bytes(take(bytes, &mut cursor)?).ok()?;

    let start = match *bytes.get(cursor)? {
        1 => {
            cursor += 1;
            RevisionStart::Fresh(authoring_cut_from_bytes(take(bytes, &mut cursor)?)?)
        }
        _ => return None,
    };
    let count = u32::from_be_bytes(bytes.get(cursor..cursor + 4)?.try_into().ok()?) as usize;
    cursor += 4;
    let mut grants = Vec::with_capacity(count);
    for _ in 0..count {
        let scope = crate::product_identity::scope_from_value(
            &identity_value(take(bytes, &mut cursor)?).ok()?,
        )
        .ok()?;
        let grant = EncodedPayload::from_bytes(take(bytes, &mut cursor)?).ok()?;
        grants.push((scope, grant));
    }
    let revision = RevisionContext::try_new(start, grants).ok()?;

    let inputs = EncodedPayload::from_bytes(take(bytes, &mut cursor)?).ok()?;
    if cursor != bytes.len() {
        return None;
    }

    Some(RunManifest::new(
        run,
        ManifestGroups::new(
            TimeParams::new(resolution, cadence, cadence_policy, plan, tick_origin),
            PlacementParams::from_validated(placement),
            FailureParams::from_validated(failure),
            versions,
            revision,
            RunInputs::from_primary_data(inputs),
        ),
    ))
}

pub fn arrival_transaction(
    batch: &AppendBatch<ProductStore>,
) -> Result<StoreTransaction<ProductTransaction>, RecordCodecError> {
    let mut operations = Vec::new();
    for (index, record) in batch.records().enumerate() {
        let bytes = encode_record(record, &ProductRecordCodec)?;
        operations.push(StoreTransactionOp::Append(TransactionAppend::new(
            OpaqueId::new(index as u64),
            EncodedPayload::new(circular_core::PayloadVersionTag::FIRST, &bytes),
        )));
    }
    Ok(StoreTransaction::try_new(operations)
        .expect("an arrival batch is not empty; AppendBatch already enforces that"))
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ArrivalProjection;

impl ArrivalProjection {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl ArrivalProjection {
    /// Rebuild one selected record after the caller has projected its envelope.
    /// The record codec and rejection remain identical to full journal replay.
    pub fn record(
        &self,
        bytes: &[u8],
    ) -> Result<Record<ProductStore>, crate::rehydrate::ProjectionRejection> {
        rebuild(bytes).ok_or(crate::rehydrate::ProjectionRejection { operation: 0 })
    }
    /// Decode the existing envelope while retaining its main arrival body in
    /// the same allocation as the transaction model's immutable record.
    pub fn shared_record(
        &self,
        bytes: &EncodedPayload,
    ) -> Result<Record<ProductStore>, crate::rehydrate::ProjectionRejection> {
        let mut record = self.record(bytes.body())?;
        if let Record::Boundary(boundary) = &mut record
            && boundary
                .arrival_body()
                .is_some_and(|body| body.payload().is_some())
            && matches!(boundary.fact(), crate::BoundaryFact::Arrival { origin, .. }
                | crate::BoundaryFact::Admission { origin, .. }
                if !matches!(origin.as_ref(), ArrivalOrigin::EffectOutcome { .. }))
        {
            let envelope = decode_envelope(bytes.body())
                .map_err(|_| crate::rehydrate::ProjectionRejection { operation: 0 })?;
            let mut cursor = 1;
            let field = take(envelope.payload, &mut cursor)
                .ok_or(crate::rehydrate::ProjectionRejection { operation: 0 })?;
            let shared = bytes
                .shared_subpayload(field)
                .ok_or(crate::rehydrate::ProjectionRejection { operation: 0 })?;
            boundary.share_arrival_payload(shared);
        }
        Ok(record)
    }
}

impl ArrivalProjection {
    pub fn project_positions(
        &self,
        transaction: &StoreTransaction<ProductTransaction>,
    ) -> Result<Vec<PositionedRecord>, crate::rehydrate::ProjectionRejection> {
        let mut records = Vec::new();
        for (operation_index, operation) in transaction.operations().iter().enumerate() {
            let StoreTransactionOp::Append(append) = operation else {
                continue;
            };
            let row = crate::record::EncodedRow::decoded(&ProductRowCodec, append.record().clone())
                .ok_or(crate::rehydrate::ProjectionRejection {
                    operation: operation_index,
                })?;
            let position = u32::try_from(operation_index).map_err(|_| {
                crate::rehydrate::ProjectionRejection {
                    operation: operation_index,
                }
            })?;
            records.push((position, row));
        }
        Ok(records)
    }
}

pub type PositionedRecord = (u32, crate::record::EncodedRow<ProductStore>);

pub trait PositionedProjection {
    fn project_positions(
        &self,
        transaction: &StoreTransaction<ProductTransaction>,
    ) -> Result<Vec<PositionedRecord>, crate::rehydrate::ProjectionRejection>;
}

impl PositionedProjection for ArrivalProjection {
    fn project_positions(
        &self,
        transaction: &StoreTransaction<ProductTransaction>,
    ) -> Result<Vec<PositionedRecord>, crate::rehydrate::ProjectionRejection> {
        Self::project_positions(self, transaction)
    }
}

/// The payload `record` owns when it is the EmissionBody of `(producer, sequence)`.
pub(crate) fn emission_body_of(
    record: &Record<ProductStore>,
    producer: &ActorId,
    sequence: Sequence,
) -> Option<EncodedPayload> {
    let Record::Boundary(boundary) = record else {
        return None;
    };
    let crate::BoundaryFact::EmissionBody { payload, .. } = boundary.fact() else {
        return None;
    };
    let at = boundary.header().at();
    (at.producer() == producer && at.sequence() == sequence).then(|| payload.clone())
}

/// The EmissionBody of `(producer, sequence)` on a surface the fold writes — answered by
/// that surface's own key index ([`crate::MemoryStore::record_for_key`]). The body record's
/// key is `BoundaryKey::EmissionBody { producer, sequence }`, the pair a reference carries,
/// so one lookup answers it. `None` when the surface holds no such record.
pub(crate) fn emission_body_in(
    store: &crate::MemoryStore<ProductStore>,
    producer: &ActorId,
    sequence: Sequence,
) -> Option<EncodedPayload> {
    let key = ClassKey::Boundary(BoundaryKey::EmissionBody {
        producer: producer.clone(),
        sequence,
    });
    store
        .record_for_key(&key)
        .and_then(|record| emission_body_of(&record, producer, sequence))
}

pub(crate) fn resolve_emitted_body(
    row: &mut crate::EncodedRow<ProductStore>,
    resolve: impl FnOnce(&ActorId, Sequence) -> Result<Option<EncodedPayload>, String>,
) -> Result<(), String> {
    let reference = match row.record() {
        Record::Boundary(boundary) => match boundary.arrival_body() {
            Some(crate::ArrivalBody::Emitted { producer, sequence }) => {
                Some((producer.clone(), *sequence))
            }
            _ => None,
        },
        _ => None,
    };
    if let Some((producer, sequence)) = reference {
        let payload = resolve(&producer, sequence)?.ok_or_else(|| {
            format!(
                "emitted body {producer:?}/{} has no owner in the preceding journal prefix",
                sequence.get()
            )
        })?;
        let Record::Boundary(boundary) = row.record_mut() else {
            unreachable!()
        };
        boundary.resolve_arrival_body(payload);
    }
    Ok(())
}

/// Resolve a known emission in this journal, strictly before the referencing operation.
/// It decodes the namespace from its first commit up to `before`: SQLite keys a commit by its
/// sequence only, so the cost grows with the reference's position.
pub fn read_emission_body(
    path: &std::path::Path,
    namespace: &str,
    before: (u64, u32),
    producer: &ActorId,
    sequence: Sequence,
) -> Result<Option<EncodedPayload>, String> {
    let journal = crate::SqliteJournal::open_read_only_namespace(path, namespace)
        .map_err(|error| error.to_string())?;
    emission_body_before(&journal, before, producer, sequence)
}

pub(crate) fn emission_body_before(
    journal: &crate::SqliteJournal,
    before: (u64, u32),
    producer: &ActorId,
    sequence: Sequence,
) -> Result<Option<EncodedPayload>, String> {
    let mut found = None;
    let mut reader = crate::ProductJournalReader::default();
    journal
        .visit_namespace(0, before.0, |commit, bytes| -> Result<(), String> {
            let transaction = reader
                .read_at(journal, commit.get(), bytes)
                .map_err(|error| error.to_string())?;
            let rows = ArrivalProjection::new()
                .project_positions(&transaction)
                .map_err(|error| {
                    ProductJournalCodec
                        .projection_error_at(commit.get(), &transaction, error)
                        .to_string()
                })?;
            for (operation, row) in rows {
                if (commit.get(), operation) >= before {
                    break;
                }
                if let Some(payload) = emission_body_of(row.record(), producer, sequence) {
                    if found.is_some() {
                        return Err("duplicate emission owner in producer column".to_owned());
                    }
                    found = Some(payload);
                }
            }
            Ok(())
        })
        .map_err(|error| error.to_string())??;
    Ok(found)
}

/// Replay logical records, resolving emission references from the already folded prefix.
pub fn rehydrate_arrivals<C, P>(
    snapshot: &crate::SqliteJournalSnapshot,
    codec: &C,
    projection: &P,
    page_policy: crate::PagePolicy,
) -> Result<crate::MemoryStore<ProductStore>, String>
where
    C: crate::SqliteTransactionCodec<ProductTransaction, Error = crate::TransactionCodecError>,
    P: PositionedProjection,
{
    rehydrate_published_arrivals(snapshot, codec, projection, page_policy, &mut |_, _| {
        Ok(Vec::new())
    })
}

pub fn rehydrate_published_arrivals<C, P>(
    snapshot: &crate::SqliteJournalSnapshot,
    codec: &C,
    projection: &P,
    page_policy: crate::PagePolicy,
    facts: &mut dyn FnMut(
        u64,
        &StoreTransaction<ProductTransaction>,
    ) -> Result<Vec<Record<ProductStore>>, String>,
) -> Result<crate::MemoryStore<ProductStore>, String>
where
    C: crate::SqliteTransactionCodec<ProductTransaction, Error = crate::TransactionCodecError>,
    P: PositionedProjection,
{
    use crate::Store;
    let mut store = crate::MemoryStore::new(page_policy);
    let mut reader = C::ReadContext::default();
    for entry in snapshot.entries() {
        let sequence = entry.sequence().get();
        let transaction = codec
            .decode_in(&mut reader, sequence, entry.payload())
            .map_err(|error| {
                ProductJournalCodec
                    .decode_error_at(sequence, error)
                    .to_string()
            })?;
        let positioned = projection
            .project_positions(&transaction)
            .map_err(|rejection| {
                ProductJournalCodec
                    .projection_error(entry, &transaction, rejection)
                    .to_string()
            })?;
        let mut rows: Vec<crate::EncodedRow<ProductStore>> = Vec::with_capacity(positioned.len());
        for (_, mut row) in positioned {
            resolve_emitted_body(&mut row, |producer, emission| {
                Ok(rows
                    .iter()
                    .find_map(|row| emission_body_of(row.record(), producer, emission))
                    .or_else(|| emission_body_in(&store, producer, emission)))
            })?;
            rows.push(row);
        }
        for fact in facts(sequence, &transaction)? {
            store.push_checkpoint_fact(fact, sequence);
        }
        store.note_commit(sequence);
        if !rows.is_empty() {
            let batch = crate::AppendBatch::try_new_rows(rows)
                .map_err(|_| "empty batch")?
                .at_commit(sequence);
            if let crate::AppendResult::Failed(failure) = store.append(batch) {
                return Err(format!("arrival journal append {sequence}: {failure:?}"));
            }
        }
    }
    if let Some(through) = snapshot.through() {
        store.note_commit(through.get());
    }
    Ok(store)
}

impl JournalProjection<ProductTransaction, ProductStore> for ArrivalProjection {
    fn project(
        &self,
        transaction: &StoreTransaction<ProductTransaction>,
    ) -> Result<Vec<Record<ProductStore>>, crate::rehydrate::ProjectionRejection> {
        let mut records = Vec::new();
        for (operation_index, operation) in transaction.operations().iter().enumerate() {
            let StoreTransactionOp::Append(append) = operation else {
                continue;
            };
            let record = ProductRowCodec.decode(append.record()).ok_or_else(|| {
                crate::rehydrate::ProjectionRejection {
                    operation: operation_index,
                }
            })?;
            records.push(record);
        }
        Ok(records)
    }
}

fn rebuild(bytes: &[u8]) -> Option<Record<ProductStore>> {
    let envelope = decode_envelope(bytes).ok()?;
    let producer = match envelope.origin {
        EnvelopeOrigin::Stamped { producer, .. } => Some(
            crate::product_identity::record_actor_from_value(&identity_value(producer).ok()?)
                .ok()?,
        ),
        EnvelopeOrigin::OperationCoordinate { .. } => None,
    };
    let record_origin = match envelope.attribution_tag {
        1 => RecordOrigin::Actor(crate::product_schema::IncarnationId::new(
            u64::from_be_bytes(envelope.attribution_body.try_into().ok()?),
        )),
        2 if envelope.attribution_body.is_empty() => RecordOrigin::Stream,
        _ => return None,
    };

    if envelope.class == crate::record::Class::Observation {
        return rebuild_observation(&envelope, record_origin, producer);
    }
    let producer = producer?;
    let at = envelope_stamp(&envelope, producer.clone())?;

    if envelope.class == crate::record::Class::Structure {
        return rebuild_structure(&envelope, record_origin, producer);
    }
    if envelope.class == crate::record::Class::Display {
        return rebuild_display(&envelope, record_origin, producer);
    }

    if envelope.class == crate::record::Class::Boundary && envelope.class_key_tag == 5 {
        if !envelope.class_key_body.is_empty() {
            return None;
        }
        return Some(Record::Boundary(BoundaryRecord::emission_body(
            at,
            record_origin,
            emission_fact_from_bytes(envelope.payload)?,
        )));
    }
    if envelope.class == crate::record::Class::Boundary && envelope.class_key_tag == 3 {
        let key = circular_runtime::EffectId::decode(envelope.class_key_body).ok()?;
        let payload = EncodedPayload::from_bytes(envelope.payload).ok()?;
        let record = BoundaryRecord::schedule_reservation(key, at, record_origin, payload);
        crate::schedule_reservation::read_schedule_reservation(&record).ok()?;
        return Some(Record::Boundary(record));
    }
    let admission = envelope.class_key_tag == 4;
    if envelope.class != crate::record::Class::Boundary
        || !(envelope.class_key_tag == 1 || admission)
    {
        return None;
    }
    let body = envelope.class_key_body;
    let mut cursor = 0_usize;
    let actor = actor_from_value(&identity_value(take(body, &mut cursor)?).ok()?).ok()?;
    let tag = *body.get(cursor)?;
    cursor += 1;
    let rebuilt_payload = rebuild_arrival_payload(envelope.payload)?;
    let mut key_route = None;
    let origin = match tag {
        1 => {
            let edge = take(body, &mut cursor)?;
            ArrivalOrigin::EdgeDelivery {
                edge: crate::product_identity::edge_from_value(&identity_value(edge).ok()?).ok()?,
                sender: take_stamp(body, &mut cursor)?,
            }
        }
        2 => ArrivalOrigin::TimerFire {
            timer: circular_runtime::EffectId::decode(take(body, &mut cursor)?).ok()?,
        },
        3 => {
            let effect = circular_runtime::EffectId::decode(take(body, &mut cursor)?).ok()?;
            let (term, outcome) = rebuilt_payload.effect.clone()?;
            ArrivalOrigin::EffectOutcome {
                effect,
                term,
                outcome,
            }
        }
        4 => {
            let origin = EncodedPayload::from_bytes(take(body, &mut cursor)?).ok()?;
            key_route = take_route_edge(body, &mut cursor)?;
            ArrivalOrigin::ExternalInject { origin }
        }
        _ => return None,
    };

    let route_edge: Option<circular_runtime::EdgeId> = match &origin {
        ArrivalOrigin::<ProductStore>::EdgeDelivery { edge, .. } => Some(edge.clone()),
        ArrivalOrigin::ExternalInject { .. } => key_route,
        ArrivalOrigin::TimerFire { .. } | ArrivalOrigin::EffectOutcome { .. } => None,
    };
    if cursor != body.len() || !route_matches_origin(&origin, route_edge.as_ref()) {
        return None;
    }
    if (tag == 3) != rebuilt_payload.effect.is_some() {
        return None;
    }
    if let crate::ArrivalBody::Emitted { producer, sequence } = &rebuilt_payload.body {
        let ArrivalOrigin::EdgeDelivery { sender, .. } = &origin else {
            return None;
        };
        if sender.producer() != producer || sender.sequence() != *sequence {
            return None;
        }
    }
    let rebuilt = if admission {
        BoundaryRecord::admission(
            actor,
            at,
            record_origin,
            origin,
            rebuilt_payload.body,
            rebuilt_payload.causal_parents,
            envelope.observed_at?,
            rebuilt_payload.inlet,
        )
    } else {
        BoundaryRecord::arrival(
            actor,
            at,
            record_origin,
            origin,
            rebuilt_payload.body,
            envelope.arrival_index?,
            rebuilt_payload.causal_parents,
            envelope.observed_at?,
            rebuilt_payload.inlet,
        )
    }
    .with_route_edge(route_edge)
    .with_result(rebuilt_payload.result);
    Some(Record::Boundary(rebuilt))
}

fn envelope_stamp(
    envelope: &crate::record_codec::RecordEnvelope<'_>,
    producer: ActorId,
) -> Option<Stamp<ActorId>> {
    let EnvelopeOrigin::Stamped {
        l,
        c,
        sequence,
        revision,
        ..
    } = envelope.origin
    else {
        return None;
    };
    crate::product_identity::record_stamp(
        Hlc::new(Tick::new(l), LogicalCounter::new(c)),
        producer,
        Sequence::new(sequence).ok()?,
        revision,
    )
    .ok()
}

fn rebuild_display(
    envelope: &crate::record_codec::RecordEnvelope<'_>,
    record_origin: RecordOrigin<ProductStore>,
    producer: ActorId,
) -> Option<Record<ProductStore>> {
    let at = envelope_stamp(envelope, producer)?;
    let key = display_key_from_bytes(envelope.class_key_body)?;
    Some(Record::Display(crate::record::DisplayRecord::new(
        at,
        record_origin,
        key,
        EncodedPayload::from_bytes(envelope.payload).ok()?,
    )))
}

fn rebuild_observation(
    envelope: &crate::record_codec::RecordEnvelope<'_>,
    record_origin: RecordOrigin<ProductStore>,
    producer: Option<ActorId>,
) -> Option<Record<ProductStore>> {
    use crate::record::ObservationRecord;

    let bucket = envelope.observation_bucket?;
    let body = envelope.class_key_body;
    let kind = observation_name_of_tag(*body.first()?)?;
    let identity = OpaqueId::new(u64::from_be_bytes(body.get(1..9)?.try_into().ok()?));
    let item = crate::record::ObservationItemKey::new(kind, identity);
    let payload = EncodedPayload::from_bytes(envelope.payload).ok()?;

    if let EnvelopeOrigin::OperationCoordinate {
        namespace,
        commit,
        operation,
        at,
    } = envelope.origin
    {
        let coordinate = crate::record::OperationCoordinate::new(
            std::str::from_utf8(namespace).ok()?.to_owned(),
            commit,
            operation,
            at,
        );
        return Some(Record::Observation(ObservationRecord::checkpoint(
            coordinate,
            bucket,
            record_origin,
            item,
            payload,
        )));
    }

    let at = envelope_stamp(envelope, producer?)?;

    Some(Record::Observation(match envelope.observation_fact_tag? {
        1 => ObservationRecord::lifecycle(at, bucket, record_origin, item, payload),
        2 => ObservationRecord::diagnostic(at, bucket, record_origin, item, payload),
        3 => match envelope.class_key_tag {
            1 => ObservationRecord::accounting(at, bucket, record_origin, item, payload),
            2 => ObservationRecord::global_accounting(at, bucket, record_origin, item, payload),
            _ => return None,
        },
        4 => ObservationRecord::dead_letter(at, bucket, record_origin, item, payload),
        6 => ObservationRecord::replay_session_transition(at, bucket, record_origin, item, payload),
        7 => ObservationRecord::restart(at, bucket, record_origin, item, payload),
        _ => return None,
    }))
}

fn rebuild_structure(
    envelope: &crate::record_codec::RecordEnvelope<'_>,
    record_origin: RecordOrigin<ProductStore>,
    producer: ActorId,
) -> Option<Record<ProductStore>> {
    let at = envelope_stamp(envelope, producer)?;
    if !matches!(record_origin, RecordOrigin::Stream) {
        return None;
    }
    let body = envelope.class_key_body;
    match envelope.class_key_tag {
        1 => Some(Record::Structure(crate::record::StructureRecord::manifest(
            at,
            manifest_from_bytes(envelope.payload)?,
        ))),
        2 => {
            let scope =
                crate::product_identity::scope_from_value(&identity_value(body).ok()?).ok()?;
            Some(Record::Structure(
                crate::record::StructureRecord::graph_revision(
                    scope,
                    at,
                    EncodedPayload::from_bytes(envelope.payload).ok()?,
                ),
            ))
        }
        _ => None,
    }
}

pub struct ProductRowCodec;

impl crate::record::RecordRowCodec<ProductStore> for ProductRowCodec {
    fn encode(&self, record: &Record<ProductStore>) -> Option<EncodedPayload> {
        let bytes = encode_record(record, &ProductRecordCodec).ok()?;
        Some(EncodedPayload::new(
            circular_core::PayloadVersionTag::FIRST,
            &bytes,
        ))
    }

    fn decode(&self, bytes: &EncodedPayload) -> Option<Record<ProductStore>> {
        ArrivalProjection::new().shared_record(bytes).ok()
    }
}

#[must_use]
pub fn reencodes_identically(record: &Record<ProductStore>) -> bool {
    let Ok(bytes) = encode_record(record, &ProductRecordCodec) else {
        return false;
    };
    rebuild(&bytes)
        .and_then(|rebuilt| encode_record(&rebuilt, &ProductRecordCodec).ok())
        .is_some_and(|again| again == bytes)
}

pub struct ProductTransactionCodec;

fn opaque(value: &OpaqueId) -> Vec<u8> {
    value.get().to_be_bytes().to_vec()
}

fn payload_bytes(value: &EncodedPayload) -> Vec<u8> {
    value.as_bytes().to_vec()
}

fn transaction_fields<'bytes, const N: usize>(
    parts: crate::transaction_codec::TransactionParts<'bytes>,
) -> std::result::Result<[&'bytes [u8]; N], crate::transaction_codec::TransactionCodecError> {
    parts.fields.try_into().map_err(|_| {
        crate::transaction_codec::TransactionCodecError::UnexpectedFieldCount {
            tag: parts.tag,
            expected: u8::try_from(N).expect("the operation field count is a u8"),
            actual: u8::try_from(parts.fields.len())
                .expect("the wire field_count limits the field count to a u8"),
        }
    })
}

fn malformed_field(tag: u8, field: u8) -> crate::transaction_codec::TransactionCodecError {
    crate::transaction_codec::TransactionCodecError::MalformedField { tag, field }
}

fn opaque_from_field(
    tag: u8,
    field: u8,
    bytes: &[u8],
) -> std::result::Result<OpaqueId, crate::transaction_codec::TransactionCodecError> {
    let raw = u64::from_be_bytes(bytes.try_into().map_err(|_| malformed_field(tag, field))?);
    Ok(OpaqueId::new(raw))
}

fn payload_from_field(
    tag: u8,
    field: u8,
    bytes: &[u8],
) -> std::result::Result<EncodedPayload, crate::transaction_codec::TransactionCodecError> {
    EncodedPayload::from_bytes(bytes).map_err(|_| malformed_field(tag, field))
}

fn actor_from_field(
    tag: u8,
    field: u8,
    bytes: &[u8],
) -> std::result::Result<ActorId, crate::transaction_codec::TransactionCodecError> {
    identity_value(bytes)
        .and_then(|value| actor_from_value(&value))
        .map_err(|_| malformed_field(tag, field))
}

fn incarnation_from_field(
    tag: u8,
    field: u8,
    bytes: &[u8],
) -> std::result::Result<
    crate::product_schema::IncarnationId,
    crate::transaction_codec::TransactionCodecError,
> {
    let raw = u64::from_be_bytes(bytes.try_into().map_err(|_| malformed_field(tag, field))?);
    Ok(crate::product_schema::IncarnationId::new(raw))
}

fn checkpoint_pending_from_field(
    tag: u8,
    field: u8,
    bytes: &[u8],
) -> std::result::Result<
    Vec<circular_runtime::EffectId>,
    crate::transaction_codec::TransactionCodecError,
> {
    let invalid = || malformed_field(tag, field);
    let mut cursor = 0_usize;
    let count_end = cursor.checked_add(4).ok_or_else(invalid)?;
    let count = u32::from_be_bytes(
        bytes
            .get(cursor..count_end)
            .ok_or_else(invalid)?
            .try_into()
            .map_err(|_| invalid())?,
    );
    cursor = count_end;

    let mut pending = Vec::new();
    for _ in 0..count {
        let length_end = cursor.checked_add(4).ok_or_else(invalid)?;
        let length = u32::from_be_bytes(
            bytes
                .get(cursor..length_end)
                .ok_or_else(invalid)?
                .try_into()
                .map_err(|_| invalid())?,
        ) as usize;
        cursor = length_end;
        let effect_end = cursor.checked_add(length).ok_or_else(invalid)?;
        let effect = bytes.get(cursor..effect_end).ok_or_else(invalid)?;
        pending.push(effect_from_field(tag, field, effect)?);
        cursor = effect_end;
    }
    if cursor != bytes.len() {
        return Err(invalid());
    }
    Ok(pending)
}

impl crate::transaction_codec::TransactionValueCodec<ProductTransaction>
    for ProductTransactionCodec
{
    type Error = crate::transaction_codec::TransactionCodecError;

    fn record_key(&self, value: &OpaqueId) -> Vec<u8> {
        opaque(value)
    }
    fn record(&self, value: &EncodedPayload) -> Vec<u8> {
        payload_bytes(value)
    }
    fn effect_id(&self, value: &circular_runtime::EffectId) -> Vec<u8> {
        circular_runtime::EffectId::encode(value).expect("admitted canonical effect identity")
    }
    fn outbox(&self, value: &EncodedPayload) -> Vec<u8> {
        payload_bytes(value)
    }
    fn approval_key(&self, value: &circular_runtime::EffectId) -> Vec<u8> {
        circular_runtime::EffectId::encode(value).expect("admitted canonical effect identity")
    }
    fn approval(&self, value: &EncodedPayload) -> Vec<u8> {
        payload_bytes(value)
    }
    fn approval_ticket(&self, value: &circular_runtime::EffectId) -> Vec<u8> {
        circular_runtime::EffectId::encode(value).expect("admitted canonical effect identity")
    }
    fn actor_id(
        &self,
        value: &ActorId,
    ) -> Result<Vec<u8>, crate::transaction_codec::TransactionCodecError> {
        actor_bytes(value).map_err(|_| {
            crate::transaction_codec::TransactionCodecError::UnencodableIdentity("ActorId")
        })
    }
    fn incarnation(&self, value: &crate::product_schema::IncarnationId) -> Vec<u8> {
        value.get().to_be_bytes().to_vec()
    }
    fn checkpoint_stamp(&self, value: &OpaqueId) -> Vec<u8> {
        opaque(value)
    }
    fn checkpoint_state(&self, value: &EncodedPayload) -> Vec<u8> {
        payload_bytes(value)
    }
    fn outcome(&self, value: &EncodedPayload) -> Vec<u8> {
        payload_bytes(value)
    }
    fn observation_key(&self, value: &OpaqueId) -> Vec<u8> {
        opaque(value)
    }
    fn observation(&self, value: &EncodedPayload) -> Vec<u8> {
        payload_bytes(value)
    }

    fn decode(
        &self,
        parts: crate::transaction_codec::TransactionParts<'_>,
    ) -> std::result::Result<StoreTransactionOp<ProductTransaction>, Self::Error> {
        let observation = |tag, key_field, value_field, key, value| {
            Ok(TransactionObservation::new(
                opaque_from_field(tag, key_field, key)?,
                payload_from_field(tag, value_field, value)?,
            ))
        };

        Ok(match parts.tag {
            1 => {
                let [key, record] = transaction_fields(parts)?;
                StoreTransactionOp::Append(TransactionAppend::new(
                    opaque_from_field(1, 0, key)?,
                    payload_from_field(1, 1, record)?,
                ))
            }
            2 => {
                let [key, value] = transaction_fields(parts)?;
                StoreTransactionOp::AppendObservation(observation(2, 0, 1, key, value)?)
            }
            5 => {
                let [effect, request] = transaction_fields(parts)?;
                StoreTransactionOp::OpenOutbox {
                    effect: effect_from_field(5, 0, effect)?,
                    request: payload_from_field(5, 1, request)?,
                }
            }
            6 => {
                let [effect] = transaction_fields(parts)?;
                StoreTransactionOp::SubmitOutbox {
                    effect: effect_from_field(6, 0, effect)?,
                }
            }
            7 => {
                let [effect] = transaction_fields(parts)?;
                StoreTransactionOp::AcquireOutboxDispatch {
                    effect: effect_from_field(7, 0, effect)?,
                }
            }
            8 => {
                let [effect, outcome, observation_key, observation_value] =
                    transaction_fields(parts)?;
                StoreTransactionOp::SettleOutbox {
                    effect: effect_from_field(8, 0, effect)?,
                    outcome: payload_from_field(8, 1, outcome)?,
                    observation: observation(8, 2, 3, observation_key, observation_value)?,
                }
            }
            9 => {
                let [effect, outcome, observation_key, observation_value] =
                    transaction_fields(parts)?;
                StoreTransactionOp::CancelCommittedOutbox {
                    effect: effect_from_field(9, 0, effect)?,
                    outcome: payload_from_field(9, 1, outcome)?,
                    observation: observation(9, 2, 3, observation_key, observation_value)?,
                }
            }
            10 => {
                let [key, approval] = transaction_fields(parts)?;
                StoreTransactionOp::OpenApproval {
                    key: effect_from_field(10, 0, key)?,
                    approval: payload_from_field(10, 1, approval)?,
                }
            }
            11 => {
                let [key, ticket] = transaction_fields(parts)?;
                StoreTransactionOp::ApproveApproval {
                    key: effect_from_field(11, 0, key)?,
                    ticket: effect_from_field(11, 1, ticket)?,
                }
            }
            12 => {
                let [key, observation_key, observation_value] = transaction_fields(parts)?;
                StoreTransactionOp::SettleApproval {
                    key: effect_from_field(12, 0, key)?,
                    observation: observation(12, 1, 2, observation_key, observation_value)?,
                }
            }
            13 => {
                let [actor, owner, at, state, pending] = transaction_fields(parts)?;
                StoreTransactionOp::ReplaceCheckpoint(TransactionCheckpoint::new(
                    actor_from_field(13, 0, actor)?,
                    incarnation_from_field(13, 1, owner)?,
                    opaque_from_field(13, 2, at)?,
                    payload_from_field(13, 3, state)?,
                    checkpoint_pending_from_field(13, 4, pending)?,
                ))
            }
            14 => {
                let [actor, at, observation_key, observation_value] = transaction_fields(parts)?;
                StoreTransactionOp::SettleCheckpoint {
                    actor: actor_from_field(14, 0, actor)?,
                    at: opaque_from_field(14, 1, at)?,
                    observation: observation(14, 2, 3, observation_key, observation_value)?,
                }
            }
            tag => {
                return Err(crate::transaction_codec::TransactionCodecError::UnknownOperation(tag));
            }
        })
    }
}

pub struct ProductJournalCodec;

impl ProductJournalCodec {
    pub(crate) fn decode_error_at(
        &self,
        sequence: u64,
        error: crate::TransactionCodecError,
    ) -> crate::SqliteJournalError {
        match error {
            crate::TransactionCodecError::UnsupportedRecordFormat { vocabulary, found } => {
                crate::SqliteJournalError::UnsupportedRecordFormat {
                    entry: sequence,
                    vocabulary,
                    found,
                }
            }
            crate::TransactionCodecError::UnknownOperation(found) => {
                crate::SqliteJournalError::UnsupportedRecordFormat {
                    entry: sequence,
                    vocabulary: "operation",
                    found: u64::from(found),
                }
            }
            other => crate::SqliteJournalError::Integrity {
                detail: format!("entry={sequence} custody decode: {other:?}"),
            },
        }
    }

    pub(crate) fn projection_error(
        &self,
        entry: &crate::SqliteJournalEntry,
        transaction: &StoreTransaction<ProductTransaction>,
        rejection: crate::rehydrate::ProjectionRejection,
    ) -> crate::SqliteJournalError {
        self.projection_error_at(entry.sequence().get(), transaction, rejection)
    }

    pub(crate) fn projection_error_at(
        &self,
        sequence: u64,
        transaction: &StoreTransaction<ProductTransaction>,
        rejection: crate::rehydrate::ProjectionRejection,
    ) -> crate::SqliteJournalError {
        if let Some(StoreTransactionOp::Append(append)) =
            transaction.operations().get(rejection.operation)
            && let Ok(envelope) = decode_envelope(append.record().body())
            && envelope.arrival_origin_tag.is_some()
            && envelope.payload.first() == Some(&3)
        {
            return crate::SqliteJournalError::UnsupportedRecordFormat {
                entry: sequence,
                vocabulary: "arrival_body",
                found: 3,
            };
        }
        if let Some(StoreTransactionOp::Append(append)) =
            transaction.operations().get(rejection.operation)
            && let Err(error) = decode_envelope(append.record().body())
        {
            let unknown = match error {
                RecordCodecError::UnknownVersion(n) => Some(("record_version", n)),
                RecordCodecError::UnknownClass(n) => Some(("record_class", n)),
                RecordCodecError::UnknownArrivalOrigin(n) => Some(("arrival_origin", n)),
                RecordCodecError::UnknownObservationFact(n) => Some(("observation_fact", n)),
                RecordCodecError::UnknownOrigin(n) => Some(("record_origin", n)),
                _ => None,
            };
            if let Some((vocabulary, found)) = unknown {
                return crate::SqliteJournalError::UnsupportedRecordFormat {
                    entry: sequence,
                    vocabulary,
                    found: u64::from(found),
                };
            }
        }
        crate::SqliteJournalError::Integrity {
            detail: format!("entry={} custody record: {rejection:?}", sequence),
        }
    }
}

impl crate::sqlite::SqliteTransactionCodec<ProductTransaction> for ProductJournalCodec {
    type Error = crate::transaction_codec::TransactionCodecError;
    type ReadContext = crate::ProductJournalReader;

    fn encode(
        &self,
        transaction: &StoreTransaction<ProductTransaction>,
    ) -> Result<Vec<u8>, Self::Error> {
        Ok(
            crate::column_codec::prepare(std::slice::from_ref(transaction))
                .map_err(|(_, error)| error)?
                .payloads
                .remove(0),
        )
    }
    fn prepare(
        &self,
        transactions: &[StoreTransaction<ProductTransaction>],
    ) -> Result<crate::sqlite::PreparedSqliteBatch, (usize, Self::Error)> {
        crate::column_codec::prepare(transactions)
    }
    fn decode(&self, bytes: &[u8]) -> Result<StoreTransaction<ProductTransaction>, Self::Error> {
        crate::ProductJournalReader::default().decode_at(1, bytes)
    }
    fn decode_in(
        &self,
        context: &mut Self::ReadContext,
        sequence: u64,
        bytes: &[u8],
    ) -> Result<StoreTransaction<ProductTransaction>, Self::Error> {
        context.decode_at(sequence, bytes)
    }
    fn checks_canonical_on_decode(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    fn effect(index: u64) -> circular_runtime::EffectId {
        let hex = "07000000040800000002000000056c6f63616c0500000001730000000573636f706507000000000700000001090000000000000002070000000309000000000000000101060000004600000000000000050000000000000002000000220800000002000000056c6f63616c0500000001730000000573636f7065070000000000000000000000070000000000000001090000000000000003";
        let mut bytes: Vec<u8> = hex
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect();
        let last = bytes.len() - 8;
        bytes[last..].copy_from_slice(&index.to_be_bytes());
        circular_runtime::EffectId::decode(&bytes).unwrap()
    }

    use super::{
        ArrivalProjection, EnvelopeOrigin, ProductJournalCodec, ProductRecordCodec,
        arrival_transaction, manifest_bytes, manifest_from_bytes, push_stamp,
        reencodes_identically, take_stamp,
    };
    use crate::memory::{AppendBatch, PagePolicy, Store};
    use crate::product_schema::{IncarnationId, OpaqueId, ProductStore, StreamId};
    use crate::query::{Bound, Query, ScanStart};
    use crate::record::{
        ArrivalOrigin, BoundaryRecord, ObservationBucket, ObservationItemKey, ObservationRecord,
        Record, RecordOrigin,
    };
    use crate::transaction::{StoreTransaction, StoreTransactionOp, TransactionAppend};
    use circular_core::{Boundary, BuiltinObservationName, Ceilings, PortId, Value};
    use circular_core::{
        EncodedPayload, PayloadVersionTag, RecordedInstant, Sequence, Stamp, Tick,
    };
    use circular_runtime::{ActorId, EdgeId, Endpoint, LocalKey, Name, NamedActorId, ScopeId};
    use std::num::NonZeroUsize;

    fn actor(text: &str) -> ActorId {
        ActorId::Scoped {
            scope: ScopeId::root(),
            local: LocalKey::Named(Name::from_normalized(text)),
        }
    }

    #[test]
    fn a_stamp_round_trips_with_its_revision() {
        let producer = NamedActorId::new(ScopeId::root(), Name::from_normalized("sender"));
        let stamp = Stamp::from_event_producer(
            Tick::new(7),
            producer,
            Sequence::new(3).unwrap(),
            circular_core::RevisionEpochId::new(4).unwrap(),
        );
        let mut bytes = Vec::new();
        push_stamp(&mut bytes, &stamp).unwrap();
        let mut cursor = 0;
        assert_eq!(take_stamp(&bytes, &mut cursor), Some(stamp));
        assert_eq!(cursor, bytes.len());
    }

    #[test]
    fn an_unpublished_identity_refuses_instead_of_converging_to_empty_bytes() {
        use crate::record_codec::{RecordCodecError, RecordIdentityCodec};
        assert_eq!(
            super::ProductRecordCodec
                .producer(&ActorId::System(circular_runtime::SystemActor::Heartbeat)),
            Err(RecordCodecError::Identity(
                crate::product_identity::ProductIdentityError::UnpublishedArm("ActorId::System"),
            ))
        );
    }

    #[test]
    fn user_effect_outcomes_name_and_system_record_have_distinct_store_columns() {
        let revision = circular_core::RevisionEpochId::new(1).unwrap();
        let named = Stamp::from_event_producer(
            Tick::new(9),
            NamedActorId::new(ScopeId::root(), Name::from_normalized("effect-outcomes")),
            Sequence::new(1).unwrap(),
            revision,
        );
        let system = Stamp::from_system_record_producer_at(
            circular_core::Hlc::from_physical(Tick::new(9)),
            ActorId::System(circular_runtime::SystemActor::Pipeline),
            Sequence::new(1).unwrap(),
            revision,
        )
        .unwrap();
        assert_ne!(named.producer(), system.producer());
        let observation = |at| {
            Record::Observation(ObservationRecord::accounting(
                at,
                ObservationBucket::from_millis(9),
                RecordOrigin::Stream,
                ObservationItemKey::new(BuiltinObservationName::Tally, OpaqueId::new(1)),
                EncodedPayload::new(PayloadVersionTag::FIRST, b"same facts"),
            ))
        };
        let size = NonZeroUsize::new(16).unwrap();
        let mut store =
            crate::memory::MemoryStore::<ProductStore>::new(PagePolicy::new(size, size).unwrap());
        let records = vec![manifest_record(), observation(named), observation(system)];
        for record in &records {
            assert!(reencodes_identically(record));
        }
        assert!(matches!(
            store.append(AppendBatch::try_new(records).unwrap()),
            crate::AppendResult::Committed(_)
        ));
        assert_eq!(
            store
                .records()
                .iter()
                .filter(|r| matches!(&**r, Record::Observation(_)))
                .count(),
            2
        );
    }

    pub(super) fn arrival(index: u64) -> Record<ProductStore> {
        Record::Boundary(BoundaryRecord::arrival(
            actor("counter"),
            Stamp::from_event_producer(
                Tick::new(index),
                NamedActorId::new(ScopeId::root(), Name::from_normalized("counter")),
                Sequence::new(index).expect("fixture sequence"),
                circular_core::RevisionEpochId::new(1).expect("first revision"),
            ),
            RecordOrigin::Actor(IncarnationId::new(1)),
            ArrivalOrigin::EdgeDelivery {
                edge: EdgeId::Declared {
                    from: Endpoint::new(
                        NamedActorId::new(ScopeId::root(), Name::from_normalized("map")),
                        PortId::try_new("out").expect("fixture port"),
                    ),
                    to: Endpoint::new(
                        NamedActorId::new(ScopeId::root(), Name::from_normalized("counter")),
                        PortId::try_new("in").expect("fixture port"),
                    ),
                    ordinal: 0,
                },
                sender: Stamp::from_event_producer(
                    Tick::new(index),
                    NamedActorId::new(ScopeId::root(), Name::from_normalized("map")),
                    Sequence::new(index).expect("fixture sequence"),
                    circular_core::RevisionEpochId::new(1).expect("first revision"),
                ),
            },
            crate::ArrivalBody::Owned(EncodedPayload::new(
                PayloadVersionTag::FIRST,
                &index.to_be_bytes(),
            )),
            circular_core::ArrivalIndex::new(index),
            Box::new([]),
            RecordedInstant::from_millis(index * 100),
            Some(PortId::try_new("in").unwrap()),
        ))
    }

    fn manifest_record() -> Record<ProductStore> {
        Record::Structure(crate::record::StructureRecord::manifest(
            Stamp::from_event_producer(
                Tick::new(0),
                NamedActorId::new(ScopeId::root(), Name::from_normalized("counter")),
                Sequence::FIRST,
                circular_core::RevisionEpochId::new(1).expect("first revision"),
            ),
            manifest_groups(),
        ))
    }

    fn display_key(
        name: circular_core::BuiltinObservationName,
        args: &str,
        bucket_millis: Option<u64>,
    ) -> crate::product_schema::DisplayKey {
        crate::product_schema::DisplayKey::new(
            name,
            NamedActorId::new(ScopeId::root(), Name::from_normalized(args)),
            bucket_millis
                .map(|millis| circular_core::NonZeroMillis::new(millis).expect("non-zero tick")),
        )
    }

    #[test]
    fn a_display_key_survives_the_bytes_without_an_issuance_table() {
        use circular_core::BuiltinObservationName::{DeliveryOccurrence, Tally};

        for key in [
            display_key(DeliveryOccurrence, "counter", Some(1_000)),
            display_key(Tally, "map", Some(60_000)),
        ] {
            let bytes = super::display_key_bytes(&key).expect("a published name and actor");
            assert_eq!(
                super::display_key_from_bytes(&bytes).expect("rebuild"),
                key,
                "the bytes lost the coordinate"
            );
        }
    }

    #[test]
    fn each_place_of_a_display_key_changes_the_bytes() {
        use circular_core::BuiltinObservationName::{DeliveryOccurrence, Tally};

        let base =
            super::display_key_bytes(&display_key(DeliveryOccurrence, "counter", Some(1_000)))
                .expect("a published name and actor");
        let other_name = super::display_key_bytes(&display_key(Tally, "counter", Some(1_000)))
            .expect("a published name and actor");
        let other_args =
            super::display_key_bytes(&display_key(DeliveryOccurrence, "map", Some(1_000)))
                .expect("a published name and actor");
        let other_bucket =
            super::display_key_bytes(&display_key(DeliveryOccurrence, "counter", Some(60_000)))
                .expect("a published name and actor");
        let snapshot = super::display_key_bytes(&display_key(DeliveryOccurrence, "counter", None))
            .expect("a published name and actor");

        let seen = [base, other_name, other_args, other_bucket, snapshot];
        for (index, left) in seen.iter().enumerate() {
            for right in seen.iter().skip(index + 1) {
                assert_ne!(left, right, "two coordinates folded into one byte string");
            }
        }
    }

    #[test]
    fn the_display_key_bytes_have_a_fixed_layout() {
        let bytes = super::display_key_bytes(&display_key(
            circular_core::BuiltinObservationName::DeliveryOccurrence,
            "counter",
            Some(60_000),
        ))
        .expect("a published name and actor");
        let hex = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            hex,
            concat!(
                "00",
                "00000028",
                "0800000002000000056c6f63616c0500000007636f756e7465720000000573636f7065",
                "0700000000",
                "01",
                "000000000000ea60",
            )
        );
    }

    #[test]
    fn a_snapshot_display_key_with_no_bucket_round_trips() {
        let key = display_key(
            circular_core::BuiltinObservationName::Tally,
            "snapshot",
            None,
        );
        let bytes = super::display_key_bytes(&key).expect("a published name and actor");
        assert_eq!(super::display_key_from_bytes(&bytes), Some(key));
    }

    #[test]
    fn display_key_bytes_with_an_unknown_suffix_are_rejected() {
        let key = display_key(
            circular_core::BuiltinObservationName::Tally,
            "counter",
            Some(1_000),
        );
        let mut bytes = super::display_key_bytes(&key).expect("a published name and actor");
        bytes.push(0);
        assert_eq!(super::display_key_from_bytes(&bytes), None);
    }

    #[test]
    fn observations_survive_the_journal_including_process_series() {
        use circular_core::BuiltinObservationName::{DeadLetterEntry, Tally};

        let by = || RecordOrigin::Stream;
        let at = |millis: u64| {
            Stamp::from_event_producer(
                Tick::new(millis),
                NamedActorId::new(ScopeId::root(), Name::from_normalized("counter")),
                Sequence::new(millis).expect("fixture sequence"),
                circular_core::RevisionEpochId::new(1).expect("first revision"),
            )
        };
        let payload = || EncodedPayload::new(PayloadVersionTag::FIRST, b"x");

        let failure = Record::Observation(crate::record::ObservationRecord::dead_letter(
            at(2),
            crate::record::ObservationBucket::from_millis(2),
            by(),
            crate::record::ObservationItemKey::new(DeadLetterEntry, OpaqueId::new(5)),
            payload(),
        ));
        let series = Record::Observation(crate::record::ObservationRecord::global_accounting(
            at(3),
            crate::record::ObservationBucket::from_millis(3),
            by(),
            crate::record::ObservationItemKey::new(Tally, OpaqueId::new(6)),
            payload(),
        ));

        let batch = AppendBatch::<ProductStore>::try_new(vec![
            manifest_record(),
            failure.clone(),
            series.clone(),
        ])
        .expect("three");
        let transaction = arrival_transaction(&batch).expect("carry");
        let rebuilt = <ArrivalProjection as crate::rehydrate::JournalProjection<
            crate::product_schema::ProductTransaction,
            ProductStore,
        >>::project(&ArrivalProjection::new(), &transaction)
        .expect("all of them are rebuilt");

        assert_eq!(rebuilt.len(), 3, "all three come back");
        assert_eq!(
            rebuilt[1], failure,
            "equal to what the failure observation carried"
        );
        assert_eq!(
            rebuilt[2], series,
            "equal to what the global series carried"
        );
        assert!(reencodes_identically(&failure));
        assert!(reencodes_identically(&series));
    }

    #[test]
    fn a_graph_revision_survives_the_journal() {
        let revision = Record::Structure(crate::record::StructureRecord::graph_revision(
            ScopeId::from_segments(vec![circular_runtime::ScopeSeg::Child(
                Name::from_normalized("fleet"),
            )])
            .expect("one segment"),
            Stamp::from_event_producer(
                Tick::new(3),
                NamedActorId::new(ScopeId::root(), Name::from_normalized("gov")),
                Sequence::new(3).expect("fixture sequence"),
                circular_core::RevisionEpochId::new(1).expect("first revision"),
            ),
            EncodedPayload::new(PayloadVersionTag::FIRST, b"rev"),
        ));
        let batch = AppendBatch::<ProductStore>::try_new(vec![manifest_record(), revision.clone()])
            .expect("two");
        let transaction = arrival_transaction(&batch).expect("carry");
        let rebuilt = <ArrivalProjection as crate::rehydrate::JournalProjection<
            crate::product_schema::ProductTransaction,
            ProductStore,
        >>::project(&ArrivalProjection::new(), &transaction)
        .expect("all of them are rebuilt");
        assert_eq!(rebuilt.len(), 2);
        assert_eq!(rebuilt[1], revision, "equal to what the revision carried");
        assert!(reencodes_identically(&revision));
    }

    #[test]
    fn the_instance_transition_lifecycle_occurrence_has_fixed_bytes() {
        use crate::record_codec::encode_record;
        use circular_runtime::{InstanceKey, InstanceScalar};

        let scope = ScopeId::root();
        let key = InstanceKey::Scalar(InstanceScalar::Int(7));
        let carrier = Value::array([
            Value::int(1),
            crate::scope_value(&scope).expect("root scope is publishable"),
            crate::instance_key_value(&key),
        ]);
        let carrier = circular_core::encode(&carrier, Ceilings::for_boundary(Boundary::Journal))
            .expect("small canonical carrier");
        let record = Record::Observation(ObservationRecord::lifecycle(
            Stamp::from_event_producer(
                Tick::new(13),
                NamedActorId::new(scope, Name::from_normalized("instantiate")),
                Sequence::new(1).expect("positive sequence"),
                circular_core::RevisionEpochId::new(1).expect("first revision"),
            ),
            ObservationBucket::from_millis(13),
            RecordOrigin::Actor(IncarnationId::new(4)),
            ObservationItemKey::new(BuiltinObservationName::InstanceTransition, OpaqueId::new(0)),
            EncodedPayload::new(PayloadVersionTag::FIRST, &carrier),
        ));
        assert!(reencodes_identically(&record));
        let bytes = encode_record(&record, &ProductRecordCodec).expect("encode");
        let hex = bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        assert_eq!(
            hex,
            concat!(
                "01000000960500000000000000000d0000000000",
                "000000002c0800000002000000056c6f63616c050000000b696e7374616e7469",
                "6174650000000573636f706507000000000000000000000001000000000000000101000000080000",
                "000000000004010100000009240000000000000000000000000000000d000000",
                "1e000107000000030300000000000000010700000000030000000000000007",
            )
        );
    }

    fn parent_count_effect_arrival() -> Record<ProductStore> {
        let producer = NamedActorId::new(ScopeId::root(), Name::from_normalized("effects"));
        let stamp = Stamp::from_event_producer_at(
            circular_core::Hlc::new(Tick::new(13), circular_core::LogicalCounter::new(17)),
            producer.clone(),
            Sequence::new(19).expect("fixture sequence"),
            circular_core::RevisionEpochId::new(37).expect("recorded revision"),
        );
        let parent = circular_core::EventId::derive(
            StreamId::new(1),
            Stamp::from_event_producer(
                Tick::new(11),
                producer,
                Sequence::new(12).expect("parent sequence"),
                circular_core::RevisionEpochId::new(1).expect("first revision"),
            ),
        );
        Record::Boundary(BoundaryRecord::effect_outcome_arrival(
            actor("counter"),
            stamp,
            RecordOrigin::Actor(IncarnationId::new(1)),
            circular_core::ArrivalIndex::new(23),
            effect(29),
            EncodedPayload::new(PayloadVersionTag::FIRST, b"term"),
            EncodedPayload::new(PayloadVersionTag::FIRST, b"outcome"),
            vec![parent].into_boxed_slice(),
            RecordedInstant::from_millis(31),
        ))
    }

    #[test]
    fn an_effect_arrival_preserves_both_coordinates_and_comparison_terms() {
        let record = parent_count_effect_arrival();
        assert!(reencodes_identically(&record));
    }

    fn parent_count_payload(record: &Record<ProductStore>) -> (Vec<u8>, usize) {
        let Record::Boundary(record) = record else {
            panic!("takes only the arrival fixture");
        };
        let bytes = super::arrival_payload_bytes(record.fact()).expect("arrival payload encoding");
        let mut cursor = 1;
        super::take(&bytes, &mut cursor).expect("event payload");
        (bytes, cursor)
    }

    #[test]
    fn parent_count_rejects_a_truncated_count_field() {
        let (bytes, count_at) = parent_count_payload(&arrival(1));
        for width in 0..4 {
            assert!(super::rebuild_arrival_payload(&bytes[..count_at + width]).is_none());
        }
    }

    #[test]
    fn parent_count_rejects_a_truncated_parent_body() {
        let (bytes, count_at) = parent_count_payload(&parent_count_effect_arrival());
        assert_eq!(&bytes[count_at..count_at + 4], &1_u32.to_be_bytes());
        let parent_start = count_at + 4;
        let mut parent_end = parent_start + 8;
        take_stamp(&bytes, &mut parent_end).expect("parent stamp");
        for end in parent_start..parent_end {
            assert!(
                super::rebuild_arrival_payload(&bytes[..end]).is_none(),
                "truncated at parent body byte {}",
                end - parent_start
            );
        }
    }

    #[test]
    fn parent_count_rejects_max_count_in_the_journal_projection() {
        use crate::sqlite::SqliteTransactionCodec;

        let mut bytes =
            super::encode_record(&arrival(1), &ProductRecordCodec).expect("arrival encoding");
        let count_at = {
            let envelope = super::decode_envelope(&bytes).expect("valid outer envelope");
            let mut cursor = 1;
            super::take(envelope.payload, &mut cursor).expect("event payload");
            envelope.total_len - envelope.payload.len() + cursor
        };
        bytes[count_at..count_at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        let transaction =
            StoreTransaction::try_new(vec![StoreTransactionOp::Append(TransactionAppend::new(
                OpaqueId::new(0),
                EncodedPayload::new(PayloadVersionTag::FIRST, &bytes),
            ))])
            .expect("one append");
        let column = crate::ColumnWriteContext::new(&actor("counter")).unwrap();
        assert!(
            ProductJournalCodec
                .encode(&column.bind(transaction))
                .is_err()
        );
    }

    #[test]
    fn parent_count_preserves_zero_parent_event_and_one_parent_effect() {
        use crate::rehydrate::JournalProjection;
        use crate::sqlite::SqliteTransactionCodec;

        let batch = AppendBatch::try_new(vec![arrival(1), parent_count_effect_arrival()])
            .expect("event and effect arrivals");
        let transaction = arrival_transaction(&batch).expect("carries the arrival");
        let column = crate::ColumnWriteContext::new(&actor("counter")).unwrap();
        let encoded = ProductJournalCodec
            .encode(&column.bind(transaction))
            .expect("transaction encoding");
        let decoded = ProductJournalCodec
            .decode(&encoded)
            .expect("read the transaction back");
        let rebuilt = ArrivalProjection::new()
            .project(&decoded)
            .expect("rebuilds an arrival with the right parent count");
        assert!(rebuilt.iter().eq(batch.records()));
    }

    fn manifest_groups() -> crate::manifest::RunManifest<ProductStore> {
        use crate::manifest::{
            FailureParams, ManifestGroups, PlacementParams, RevisionContext, RevisionStart,
            RunInputs, RunManifest, TimeParams,
        };
        use circular_core::{NonZeroTicks, TicksPerSecond, TimeSourceKind, TimeSourcePlan};

        let payload = || EncodedPayload::new(PayloadVersionTag::FIRST, b"");
        let revision = RevisionContext::try_new(
            RevisionStart::Fresh(crate::manifest_test_support::authoring_cut()),
            Vec::new(),
        )
        .expect("empty grant");
        let groups = ManifestGroups::new(
            TimeParams::new(
                TicksPerSecond::new(1_000).expect("resolution"),
                NonZeroTicks::new(1).expect("period"),
                payload(),
                TimeSourcePlan::Single(TimeSourceKind::Manual),
                OpaqueId::new(0),
            ),
            PlacementParams::from_validated(payload()),
            FailureParams::from_validated(payload()),
            payload(),
            revision,
            RunInputs::from_primary_data(payload()),
        );
        RunManifest::new(StreamId::new(1), groups)
    }

    #[test]
    fn two_arrivals_that_differ_only_in_their_origin_do_not_collide() {
        use crate::record_codec::encode_record;

        let keyed = |record: &Record<ProductStore>| {
            let bytes = encode_record(record, &super::ProductRecordCodec).expect("encode");
            crate::record_codec::decode_envelope(&bytes)
                .expect("decode")
                .class_key_body
                .to_vec()
        };

        let edge = keyed(&arrival(1));
        let timer = keyed(&Record::Boundary(BoundaryRecord::arrival(
            actor("counter"),
            Stamp::from_event_producer(
                Tick::new(1),
                NamedActorId::new(ScopeId::root(), Name::from_normalized("counter")),
                Sequence::new(1).expect("sequence"),
                circular_core::RevisionEpochId::new(1).expect("first revision"),
            ),
            RecordOrigin::Actor(IncarnationId::new(1)),
            ArrivalOrigin::TimerFire { timer: effect(1) },
            crate::ArrivalBody::Owned(EncodedPayload::new(PayloadVersionTag::FIRST, b"")),
            circular_core::ArrivalIndex::new(1),
            Box::new([]),
            RecordedInstant::from_millis(0),
            Some(PortId::try_new("timer").unwrap()),
        )));

        assert_ne!(edge, timer, "a different class gives a different key");
        assert_ne!(
            edge,
            keyed(&arrival(2)),
            "even within one class, a different sent stamp gives a different key"
        );
    }

    #[test]
    fn removed_retirement_carriers_are_refused() {
        use crate::{ProductRowCodec, RecordRowCodec};
        let bytes = ProductRowCodec.encode(&arrival(1)).unwrap();
        for presence in [3, 4] {
            let mut old = bytes.body().to_vec();
            old[6] = presence;
            assert_eq!(
                crate::decode_envelope(&old),
                Err(crate::RecordCodecError::InvalidPositionTag(presence))
            );
            assert!(ArrivalProjection::new().record(&old).is_err());
            assert!(
                ProductRowCodec
                    .decode(&EncodedPayload::new(PayloadVersionTag::FIRST, &old))
                    .is_none()
            );
        }
    }

    #[test]
    fn manifest_wire_has_no_origin_run_and_refuses_fork() {
        let literal = concat!(
            "000000000000000000000001000003e800000000000000010000000200010103000000000000000000000002000100000002000100000002",
            "000101000000ff080000000500000012617574686f72696e675f7265766973696f6e0600000020a3a3a3a3a3a3a3a3a3",
            "a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a300000006637572736f720900000000000000010000000b656e",
            "7669726f6e6d656e740800000002000000126465636c61726174696f6e5f736368656d610600000001a1000000087370",
            "65635f7365740600000001a20000000770726f6a65637406000000201010101010101010101010101010101010101010",
            "10101010101010101010101000000011746f706f6c6f67795f7265766973696f6e0600000020a4a4a4a4a4a4a4a4a4a4",
            "a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a4a400000000000000020001",
        );
        let bytes: Vec<u8> = literal
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect();
        assert_eq!(manifest_bytes(&manifest_groups()).unwrap(), bytes);
        assert_eq!(manifest_from_bytes(&bytes), Some(manifest_groups()));
        for origin in [vec![0], vec![1, 0, 0, 0, 0, 0, 0, 0, 9]] {
            let mut old = bytes.clone();
            old.splice(40..40, origin);
            assert!(manifest_from_bytes(&old).is_none());
        }
        let mut fork = bytes;
        fork[58] = 2;
        assert!(manifest_from_bytes(&fork).is_none());
    }

    #[test]
    fn manifest_rejects_removed_retention_declarations() {
        let manifest = manifest_groups();
        let bytes = manifest_bytes(&manifest).unwrap();
        assert_eq!(manifest_from_bytes(&bytes), Some(manifest));
        for suffix in [&[2][..], &[1, 0, 0, 0, 2, 0, 1][..]] {
            let mut old = bytes.clone();
            old.extend_from_slice(suffix);
            assert!(manifest_from_bytes(&old).is_none());
        }
    }

    #[test]
    fn display_payload_versions_round_trip() {
        use crate::{ProductRowCodec, RecordRowCodec};
        for version in [1, 0x0601, u16::MAX] {
            let record = Record::Display(crate::DisplayRecord::new(
                Stamp::from_event_producer(
                    Tick::new(1),
                    NamedActorId::new(ScopeId::root(), Name::from_normalized("sender")),
                    Sequence::new(1).unwrap(),
                    circular_core::RevisionEpochId::new(1).unwrap(),
                ),
                RecordOrigin::Actor(IncarnationId::new(1)),
                crate::product_schema::DisplayKey::new(
                    BuiltinObservationName::Tally,
                    NamedActorId::new(ScopeId::root(), Name::from_normalized("receiver")),
                    None,
                ),
                EncodedPayload::new(PayloadVersionTag::new(version).unwrap(), &[6, 0, 1]),
            ));
            let bytes = ProductRowCodec.encode(&record).unwrap();
            assert_eq!(ProductRowCodec.decode(&bytes), Some(record.clone()));
        }
    }
}

#[cfg(test)]
mod inlet_record_tests {
    use super::*;
    use circular_core::{ArrivalIndex, PayloadVersionTag, RecordedInstant};

    const EVENT: &[u8] = &[
        1, 0, 0, 0, 2, 0, 1, 0, 0, 0, 0, 0, 0, 0, 5, b'e', b'v', b'e', b'n', b't',
        0, 0, 0, 14, 7, 0, 0, 0, 1, 9, 0, 0, 0, 0, 0, 0, 0, 1,
    ];
    const FLUSH: &[u8] = &[
        1, 0, 0, 0, 2, 0, 1, 0, 0, 0, 0, 0, 0, 0, 5, b'f', b'l', b'u', b's', b'h',
        0, 0, 0, 14, 7, 0, 0, 0, 1, 9, 0, 0, 0, 0, 0, 0, 0, 1,
    ];

    const ROUTED_LEGACY: &[u8] = &[
        1, 0, 0, 0, 2, 0, 1, 0, 0, 0, 0, 0, 0, 0, 5, 101, 118, 101, 110, 116, 1, 0, 0, 0, 155, 7,
        0, 0, 0, 4, 3, 0, 0, 0, 0, 0, 0, 0, 1, 8, 0, 0, 0, 2, 0, 0, 0, 5, 97, 99, 116, 111, 114, 8,
        0, 0, 0, 2, 0, 0, 0, 5, 108, 111, 99, 97, 108, 5, 0, 0, 0, 1, 97, 0, 0, 0, 5, 115, 99, 111,
        112, 101, 7, 0, 0, 0, 0, 0, 0, 0, 4, 112, 111, 114, 116, 5, 0, 0, 0, 5, 101, 118, 101, 110,
        116, 8, 0, 0, 0, 2, 0, 0, 0, 5, 97, 99, 116, 111, 114, 8, 0, 0, 0, 2, 0, 0, 0, 5, 108, 111,
        99, 97, 108, 5, 0, 0, 0, 1, 98, 0, 0, 0, 5, 115, 99, 111, 112, 101, 7, 0, 0, 0, 0, 0, 0, 0,
        4, 112, 111, 114, 116, 5, 0, 0, 0, 5, 101, 118, 101, 110, 116, 3, 0, 0, 0, 0, 0, 0, 0, 7,
        0, 0, 0, 14, 7, 0, 0, 0, 1, 9, 0, 0, 0, 0, 0, 0, 0, 1,
    ];

    #[test]
    fn arrival_payload_rejects_unassigned_tags() {
        for tag in [0, 4, 5, 6] {
            assert!(super::rebuild_arrival_payload(&[tag, 0, 0, 0, 0]).is_none());
        }
    }
}

fn effect_from_field(
    tag: u8,
    field: u8,
    bytes: &[u8],
) -> Result<circular_runtime::EffectId, crate::transaction_codec::TransactionCodecError> {
    circular_runtime::EffectId::decode(bytes).map_err(|_| malformed_field(tag, field))
}
