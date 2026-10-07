//! Effect outcome record placement: the existing summary payload and the exact runtime result body.
//! Store codecs keep these fields opaque. Strict replay identity/feed remains a separate step.

use circular_core::{
    ArrivalIndex, Boundary, Ceilings, EncodedPayload, EventId, ObjectValue, PayloadVersionTag,
    RecordedInstant, Stamp, Value,
};
use circular_plan::{ActorId, LocalKey, NamedActorId};
use circular_runtime::{
    AgentProgressRecord, EffectFailure, EffectTerm, OutcomePayload, PeerEffectTerm,
    PeerEventEnvelope, decode_outcome, decode_peer_envelope, decode_term, encode_outcome,
    encode_peer_envelope, try_encode_term,
};
use circular_store::{
    ArrivalOrigin, BoundaryFact, BoundaryKey, BoundaryRecord, ClassKey, ProductRecordCodec,
    ProductStore, Record, RecordIdentityCodec, RecordOrigin, StreamId,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EncodedOutcome {
    pub summary: Vec<u8>,
    pub term: Vec<u8>,
    pub body: Vec<u8>,
}

fn summary(
    actor: &NamedActorId,
    cause: &Stamp<ActorId>,
    result: &Result<OutcomePayload, EffectFailure>,
) -> Value {
    let (ok, kind) = match result {
        Ok(value) => (true, value.kind_tag()),
        Err(value) => (false, value.kind_tag()),
    };
    let mut entries = vec![
        (
            "cause_sequence".to_owned(),
            Value::UInt(cause.sequence().get()),
        ),
        ("kind".to_owned(), Value::String(kind.to_owned())),
        (
            "actor".to_owned(),
            Value::String(actor.name().as_str().to_owned()),
        ),
        ("ok".to_owned(), Value::Bool(ok)),
    ];
    match result {
        Ok(OutcomePayload::NotificationDelivered(value)) => entries.push((
            "channel".to_owned(),
            Value::String(value.channel().as_str().to_owned()),
        )),
        Ok(OutcomePayload::ProcessResult(value)) => {
            entries.push(("exit".to_owned(), Value::Int(value.exit_code().into())))
        }
        Err(EffectFailure::RetryExhausted { attempts }) => {
            entries.push(("attempts".to_owned(), Value::UInt(u64::from(*attempts))))
        }
        _ => {}
    }
    Value::Object(ObjectValue::try_from_entries(entries).expect("summary keys are distinct"))
}

fn validate_receive_envelope(
    term: &PeerEffectTerm,
    envelope: &PeerEventEnvelope,
) -> Result<(), String> {
    let PeerEffectTerm::Receive { binding, after } = term else {
        return Err("peer envelope requires its Receive term".into());
    };
    if envelope.binding() != binding || after.is_some_and(|cursor| envelope.cursor() <= cursor) {
        return Err("peer envelope differs from the recorded receive request".into());
    }
    Ok(())
}

fn encode_body(
    term: Option<&EffectTerm>,
    result: &Result<OutcomePayload, EffectFailure>,
    failure_progress: &[AgentProgressRecord],
) -> Result<Vec<u8>, String> {
    match (term, result) {
        (
            Some(EffectTerm::Peer(receive @ PeerEffectTerm::Receive { .. })),
            Ok(OutcomePayload::PeerEnvelope(envelope)),
        ) => {
            validate_receive_envelope(receive, envelope)?;
            encode_peer_envelope(envelope)
                .map_err(|error| format!("peer envelope encoding: {error:?}"))
        }
        (Some(EffectTerm::Peer(PeerEffectTerm::Receive { .. })), Ok(_)) => {
            Err("Receive completed with a non-envelope result".into())
        }
        _ => encode_outcome(result, failure_progress)
            .map_err(|error| format!("effect body encoding: {error:?}")),
    }
}

impl EncodedOutcome {
    pub(crate) fn new(
        actor: &NamedActorId,
        cause: &Stamp<ActorId>,
        term: Option<&EffectTerm>,
        result: &Result<OutcomePayload, EffectFailure>,
        failure_progress: &[AgentProgressRecord],
    ) -> Result<Self, String> {
        let summary = circular_core::encode(
            &summary(actor, cause, result),
            Ceilings::for_boundary(Boundary::Journal),
        )
        .map_err(|error| format!("effect summary encoding: {error:?}"))?;
        let body = encode_body(term, result, failure_progress)?;
        let term = term
            .map(try_encode_term)
            .transpose()
            .map_err(|error| format!("effect term encoding: {error:?}"))?
            .unwrap_or_default();

        Ok(Self {
            summary,
            term,
            body,
        })
    }
}

fn metadata_length(
    actor: usize,
    at_producer: usize,
    cause_producer: usize,
    effect_key: usize,
) -> Result<usize, String> {
    u16::try_from(at_producer).map_err(|_| "effect header producer exceeds u16")?;
    u32::try_from(actor).map_err(|_| "effect actor identity exceeds u32")?;
    u32::try_from(cause_producer).map_err(|_| "effect cause producer exceeds u32")?;
    u32::try_from(effect_key).map_err(|_| "effect key exceeds u32")?;
    [actor, at_producer, cause_producer, effect_key]
        .into_iter()
        .try_fold(153_usize, |size, length| {
            size.checked_add(length)
                .ok_or_else(|| "effect metadata length overflow".to_owned())
        })
}

pub(crate) fn check_record_lengths(
    actor: &NamedActorId,
    at_producer: &ActorId,
    cause: &Stamp<ActorId>,
    encoded: &EncodedOutcome,
    effect: &circular_runtime::EffectId,
) -> Result<(), String> {
    let identity = |actor: &ActorId| {
        ProductRecordCodec
            .producer(actor)
            .map_err(|error| format!("effect record identity encoding: {error:?}"))
    };
    let metadata = metadata_length(
        identity(&actor.as_actor_id())?.len(),
        identity(at_producer)?.len(),
        identity(cause.producer())?.len(),
        circular_runtime::EffectId::encode(effect)
            .map_err(|e| e.to_string())?
            .len(),
    )?;
    check_lengths(
        metadata,
        [
            encoded.summary.len(),
            encoded.term.len(),
            encoded.body.len(),
        ],
    )
}

fn check_lengths(empty_record: usize, lengths: [usize; 3]) -> Result<(), String> {
    let mut total = empty_record
        .checked_add(2)
        .ok_or("effect record length overflow")?;
    for length in lengths {
        let field = length
            .checked_add(2)
            .ok_or("effect field length overflow")?;
        u32::try_from(field).map_err(|_| "effect field exceeds u32")?;
        total = total
            .checked_add(length)
            .ok_or("effect record length overflow")?;
    }
    u32::try_from(total).map_err(|_| "effect record exceeds u32")?;
    Ok(())
}

/// Construct both product writers' records with their original supplied coordinates.
#[allow(clippy::too_many_arguments)]
pub fn effect_outcome_record(
    run: StreamId,
    actor: &NamedActorId,
    at: Stamp<ActorId>,
    index: ArrivalIndex,
    effect: circular_runtime::EffectId,
    cause: &Stamp<ActorId>,
    term: Option<&EffectTerm>,
    result: &Result<OutcomePayload, EffectFailure>,
    failure_progress: &[AgentProgressRecord],
    observed_at: RecordedInstant,
) -> Result<Record<ProductStore>, String> {
    let encoded = EncodedOutcome::new(actor, cause, term, result, failure_progress)?;
    let build = |summary: &[u8], term: &[u8], body: &[u8]| {
        Record::Boundary(BoundaryRecord::arrival(
            actor.as_actor_id(),
            at.clone(),
            RecordOrigin::Stream,
            ArrivalOrigin::EffectOutcome {
                effect: effect.clone(),
                term: EncodedPayload::new(PayloadVersionTag::FIRST, term),
                outcome: EncodedPayload::new(PayloadVersionTag::FIRST, body),
            },
            circular_store::ArrivalBody::Owned(EncodedPayload::new(
                PayloadVersionTag::FIRST,
                summary,
            )),
            index,
            Box::new([EventId::derive(run, cause.clone())]),
            observed_at,
            None,
        ))
    };
    check_record_lengths(actor, at.producer(), cause, &encoded, &effect)?;
    Ok(build(&encoded.summary, &encoded.term, &encoded.body))
}

/// Decoded body together with the borrowed original record; no synthesized inlet or stamp.
#[derive(Debug)]
pub struct RecordedEffectOutcome {
    pub record: Record<ProductStore>,
    pub actor: NamedActorId,
    pub effect: circular_runtime::EffectId,
    pub term: Option<EffectTerm>,
    pub result: Result<OutcomePayload, EffectFailure>,
    pub failure_progress: Vec<AgentProgressRecord>,
}

/// Query projection reads the summary, never the full body codec. The one exception is
/// [`effect_outcome_approval`]: the term head and an approval request's small decision body.
pub fn effect_outcome_summary(payload: &EncodedPayload) -> Result<Value, String> {
    if payload.version_tag() != PayloadVersionTag::FIRST {
        return Err("unsupported effect summary payload version".to_owned());
    }
    if payload.body().is_empty() {
        return Err("legacy summary-only effect record is unsupported".to_owned());
    }
    circular_core::decode(payload.body(), Ceilings::for_boundary(Boundary::Journal))
        .map_err(|error| format!("effect summary decoding: {error:?}"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecordedApproval {
    Decision {
        item: circular_runtime::EffectId,
        decision: circular_runtime::ApprovalRequestOutcome,
    },
    Ticket(circular_runtime::ApprovalTicket),
}

pub fn effect_outcome_approval(
    origin: &ArrivalOrigin<ProductStore>,
) -> Result<Option<RecordedApproval>, String> {
    let ArrivalOrigin::EffectOutcome {
        effect,
        term,
        outcome,
    } = origin
    else {
        return Ok(None);
    };
    if term.body().is_empty() {
        return Ok(None);
    }
    if term.version_tag() != PayloadVersionTag::FIRST
        || outcome.version_tag() != PayloadVersionTag::FIRST
    {
        return Err("unsupported effect term/body payload version".to_owned());
    }
    match circular_runtime::decode_term_approval(term.body())
        .map_err(|error| format!("effect term head decoding: {error:?}"))?
    {
        None => Ok(None),
        Some(circular_runtime::TermApproval::Ticket(ticket)) => {
            Ok(Some(RecordedApproval::Ticket(ticket)))
        }
        Some(circular_runtime::TermApproval::Request) => match decode_outcome(outcome.body())
            .map_err(|error| format!("effect body decoding: {error:?}"))?
            .0
        {
            Ok(OutcomePayload::Approval(decision)) => Ok(Some(RecordedApproval::Decision {
                item: effect.clone(),
                decision,
            })),
            Ok(_) => Err("approval request settled with a non-approval result".to_owned()),
            Err(_) => Ok(None),
        },
    }
}

/// Validate before any replay actor or external executor is constructed.
pub fn read_effect_outcome(
    record: &Record<ProductStore>,
) -> Result<Option<RecordedEffectOutcome>, String> {
    let Record::Boundary(boundary) = record else {
        return Ok(None);
    };
    let BoundaryFact::Arrival {
        origin,
        body,
        causal_parents,
        ..
    } = boundary.fact()
    else {
        return Ok(None);
    };
    let Some(payload) = body.payload() else {
        return Ok(None);
    };
    let ArrivalOrigin::EffectOutcome {
        effect,
        term,
        outcome,
    } = origin.as_ref()
    else {
        return Ok(None);
    };
    let ClassKey::Boundary(BoundaryKey::Arrival { actor, .. }) = boundary.header().key() else {
        return Err("effect record has no arrival identity".to_owned());
    };
    let ActorId::Scoped {
        scope,
        local: LocalKey::Named(name),
    } = actor
    else {
        return Err("effect record actor is unnamed".to_owned());
    };
    let actor = NamedActorId::new(scope.clone(), name.clone());
    if effect.actor() != &actor.as_actor_id() {
        return Err("effect identity belongs to another actor".into());
    }
    let recorded_summary = effect_outcome_summary(payload)?;
    if term.version_tag() != PayloadVersionTag::FIRST
        || outcome.version_tag() != PayloadVersionTag::FIRST
    {
        return Err("unsupported effect term/body payload version".to_owned());
    }
    if causal_parents.len() != 1 {
        return Err("effect record requires its original cause".to_owned());
    }
    let term = if term.body().is_empty() {
        None
    } else {
        Some(decode_term(term.body()).map_err(|error| format!("effect term decoding: {error:?}"))?)
    };
    let (result, failure_progress) =
        if let Some(EffectTerm::Peer(receive @ PeerEffectTerm::Receive { .. })) = &term {
            let Value::Object(fields) = &recorded_summary else {
                return Err("effect summary is not an object".into());
            };
            match fields.get("ok") {
                Some(Value::Bool(true)) => {
                    let envelope = decode_peer_envelope(outcome.body())
                        .map_err(|error| format!("peer envelope decoding: {error:?}"))?;
                    validate_receive_envelope(receive, &envelope)?;
                    (Ok(OutcomePayload::PeerEnvelope(envelope)), Vec::new())
                }
                Some(Value::Bool(false)) => {
                    let settled = decode_outcome(outcome.body())
                        .map_err(|error| format!("effect body decoding: {error:?}"))?;
                    if settled.0.is_ok() {
                        return Err("Receive failure summary has a successful body".into());
                    }
                    settled
                }
                _ => return Err("Receive summary has no success/failure fact".into()),
            }
        } else {
            decode_outcome(outcome.body())
                .map_err(|error| format!("effect body decoding: {error:?}"))?
        };
    if recorded_summary != summary(&actor, causal_parents[0].stamp(), &result) {
        return Err("effect summary conflicts with body/actor/cause".to_owned());
    }
    if let (
        Some(EffectTerm::AgentInvoke { invoke, .. }),
        Ok(OutcomePayload::AgentStepResult(result)),
    ) = (&term, &result)
    {
        if invoke.harness() != result.session().harness() {
            return Err(
                "effect result session differs from the recorded request harness".to_owned(),
            );
        }
    }
    Ok(Some(RecordedEffectOutcome {
        record: record.clone(),
        actor,
        effect: effect.clone(),
        term,
        result,
        failure_progress,
    }))
}

pub fn read_effect_outcomes<R: std::borrow::Borrow<Record<ProductStore>>>(
    records: impl IntoIterator<Item = R>,
) -> Result<Vec<RecordedEffectOutcome>, String> {
    let mut seen = BTreeSet::new();
    let mut outcomes = Vec::new();
    let mut indices = BTreeMap::<_, BTreeSet<_>>::new();
    for record in records {
        let record = record.borrow();
        if let Record::Boundary(boundary) = record {
            let BoundaryFact::Arrival { arrival_index, .. } = boundary.fact() else {
                continue;
            };
            let ClassKey::Boundary(BoundaryKey::Arrival { actor, .. }) = boundary.header().key()
            else {
                return Err("arrival record has no identity".to_owned());
            };
            if !indices
                .entry(actor.clone())
                .or_default()
                .insert(*arrival_index)
            {
                return Err("duplicate arrival index in recorded input".to_owned());
            }
        }
        if let Some(outcome) = read_effect_outcome(record)? {
            if !seen.insert((outcome.actor.clone(), outcome.effect.clone())) {
                return Err("duplicate effect outcome identity in recorded input".to_owned());
            }
            outcomes.push(outcome);
        }
    }
    for column in indices.values() {
        let base = column.first().map_or(0, |index| index.get());
        for (offset, index) in column.iter().enumerate() {
            let expected = u64::try_from(offset)
                .ok()
                .and_then(|offset| base.checked_add(offset))
                .ok_or("arrival index overflow")?;
            if index.get() != expected {
                return Err("recorded arrival column is not dense".to_owned());
            }
        }
    }
    Ok(outcomes)
}

#[cfg(test)]
mod tests {
    fn test_effect(index: u64) -> circular_runtime::EffectId {
        circular_runtime::EffectId::from_components(
            actor().into(),
            vec![circular_plan::Generation::new(0)].into_boxed_slice(),
            circular_runtime::EffectOccasion::Poll(circular_core::Tick::new(5), 0),
            index,
        )
        .unwrap()
    }

    use super::*;
    use circular_core::{RevisionEpochId, Sequence, Tick};
    use circular_plan::{Name, ScopeId};

    fn actor() -> NamedActorId {
        NamedActorId::new(ScopeId::root(), Name::from_normalized("sink"))
    }
    fn stamp(tick: u64, sequence: u64) -> Stamp<ActorId> {
        Stamp::from_event_producer(
            Tick::new(tick),
            actor(),
            Sequence::new(sequence).unwrap(),
            RevisionEpochId::new(1).unwrap(),
        )
    }

    fn record(
        index: u64,
        effect: circular_runtime::EffectId,
        result: &Result<OutcomePayload, EffectFailure>,
    ) -> Record<ProductStore> {
        effect_outcome_record(
            StreamId::new(31),
            &actor(),
            stamp(20 - index, index),
            ArrivalIndex::new(index),
            effect,
            &stamp(12, 0),
            None,
            result,
            &[],
            RecordedInstant::from_millis(19),
        )
        .unwrap()
    }
    fn fields(
        record: &mut Record<ProductStore>,
        mutate: impl FnOnce(&mut EncodedPayload, &mut EncodedPayload, &mut EncodedPayload),
    ) {
        let Record::Boundary(boundary) = record else {
            panic!()
        };
        let ClassKey::Boundary(BoundaryKey::Arrival { actor, .. }) = boundary.header().key() else {
            panic!()
        };
        let BoundaryFact::Arrival {
            origin,
            body: circular_store::ArrivalBody::Owned(payload),
            arrival_index,
            causal_parents,
            observed_at,
            ..
        } = boundary.fact()
        else {
            panic!("fixture requires an Arrival record");
        };
        let ArrivalOrigin::EffectOutcome {
            effect,
            term,
            outcome,
        } = origin.as_ref()
        else {
            panic!()
        };
        let (mut payload, mut term, mut outcome) = (payload.clone(), term.clone(), outcome.clone());
        mutate(&mut payload, &mut term, &mut outcome);
        *record = Record::Boundary(BoundaryRecord::arrival(
            actor.clone(),
            boundary.header().at().clone(),
            boundary.header().origin().clone(),
            ArrivalOrigin::EffectOutcome {
                effect: effect.clone(),
                term,
                outcome,
            },
            circular_store::ArrivalBody::Owned(payload),
            *arrival_index,
            causal_parents.clone(),
            *observed_at,
            None,
        ));
    }

    #[test]
    fn reader_rejects_legacy_versions_truncation_trailing_and_conflicting_summary() {
        let original = record(0, test_effect(41), &Err(EffectFailure::EndpointGone));
        for position in 0..3 {
            let mut invalid = original.clone();
            fields(&mut invalid, |summary, term, body| {
                let field = match position {
                    0 => summary,
                    1 => term,
                    _ => body,
                };
                *field = EncodedPayload::new(PayloadVersionTag::new(2).unwrap(), field.body());
            });
            assert!(
                read_effect_outcome(&invalid)
                    .unwrap_err()
                    .contains("version")
            );
        }
        let mut legacy = original.clone();
        fields(&mut legacy, |summary, _, body| {
            *body = summary.clone();
            *summary = EncodedPayload::new(PayloadVersionTag::FIRST, &[]);
        });
        assert!(read_effect_outcome(&legacy).unwrap_err().contains("legacy"));
        for bytes in [
            &[][..],
            &[1][..],
            &[1, 2][..],
            &[1, 2, 5, 0][..],
            &[2, 2, 5][..],
            &[1, 3, 5][..],
        ] {
            let mut invalid = original.clone();
            fields(&mut invalid, |_, _, body| {
                *body = EncodedPayload::new(PayloadVersionTag::FIRST, bytes)
            });
            assert!(
                read_effect_outcome(&invalid)
                    .unwrap_err()
                    .contains("body decoding")
            );
        }
        let mut invalid_term = original.clone();
        fields(&mut invalid_term, |_, term, _| {
            *term = EncodedPayload::new(PayloadVersionTag::FIRST, &[1, 5])
        });
        assert!(
            read_effect_outcome(&invalid_term)
                .unwrap_err()
                .contains("term decoding")
        );
        let mut conflict = original.clone();
        fields(&mut conflict, |_, _, body| {
            *body = EncodedPayload::new(PayloadVersionTag::FIRST, &[1, 2, 2])
        });
        assert!(
            read_effect_outcome(&conflict)
                .unwrap_err()
                .contains("conflicts")
        );
    }

    #[test]
    fn reader_rejects_summary_actor_cause_ok_kind_and_extra_field_disagreement() {
        for (key, changed) in [
            ("actor", Value::String("another-actor".to_owned())),
            ("cause_sequence", Value::UInt(1)),
            ("ok", Value::Bool(true)),
            ("kind", Value::String("transport_terminal".to_owned())),
            ("body", Value::String("invented".to_owned())),
        ] {
            let mut entries = vec![
                ("actor".to_owned(), Value::String("sink".to_owned())),
                ("cause_sequence".to_owned(), Value::UInt(0)),
                ("ok".to_owned(), Value::Bool(false)),
                ("kind".to_owned(), Value::String("endpoint_gone".to_owned())),
            ];
            entries.retain(|(name, _)| name != key);
            entries.push((key.to_owned(), changed));
            let bytes = circular_core::encode(
                &Value::object(entries).unwrap(),
                Ceilings::for_boundary(Boundary::Journal),
            )
            .unwrap();
            let mut record = record(0, test_effect(41), &Err(EffectFailure::EndpointGone));
            fields(&mut record, |summary, _, _| {
                *summary = EncodedPayload::new(PayloadVersionTag::FIRST, &bytes)
            });
            assert!(
                read_effect_outcome(&record)
                    .unwrap_err()
                    .contains("conflicts"),
                "{key}"
            );
        }
    }

    #[test]
    fn reader_compares_agent_session_to_recorded_request_harness() {
        use circular_runtime::{
            AgentHarnessName, AgentInvokeSpec, AgentPayload, AgentSessionId, AgentStepNext,
            AgentStepRequest, AgentStepResult,
        };
        let request_harness = AgentHarnessName::try_from_normalized("requested").unwrap();
        let result_harness = AgentHarnessName::try_from_normalized("returned").unwrap();
        let term = EffectTerm::AgentInvoke {
            ticket: None,
            invoke: AgentInvokeSpec::try_new(
                request_harness,
                None,
                AgentStepRequest::user_turn(AgentPayload::new(&b"turn"[..])),
            )
            .unwrap(),
        };
        let result = Ok(OutcomePayload::AgentStepResult(
            AgentStepResult::try_new(
                &result_harness,
                AgentSessionId::new(result_harness.clone(), &b"session"[..]),
                Vec::new(),
                AgentStepNext::Final {
                    output: AgentPayload::new(&b"result"[..]),
                    metadata: AgentPayload::new(&b"memory"[..]),
                },
            )
            .unwrap(),
        ));
        let record = effect_outcome_record(
            StreamId::new(31),
            &actor(),
            stamp(20, 0),
            ArrivalIndex::FIRST,
            test_effect(41),
            &stamp(12, 0),
            Some(&term),
            &result,
            &[],
            RecordedInstant::from_millis(19),
        )
        .unwrap();
        assert!(
            read_effect_outcome(&record)
                .unwrap_err()
                .contains("request harness")
        );
    }

    #[test]
    fn summary_projection_does_not_decode_or_invent_an_outcome_body() {
        let mut record = record(0, test_effect(41), &Err(EffectFailure::EndpointGone));
        fields(&mut record, |_, _, body| {
            *body = EncodedPayload::new(PayloadVersionTag::FIRST, &[255])
        });
        let Record::Boundary(boundary) = &record else {
            panic!()
        };
        let BoundaryFact::Arrival {
            body: circular_store::ArrivalBody::Owned(payload),
            ..
        } = boundary.fact()
        else {
            panic!("fixture requires an Arrival record");
        };
        assert_eq!(
            effect_outcome_summary(payload).unwrap(),
            Value::object(vec![
                ("cause_sequence".to_owned(), Value::UInt(0)),
                ("kind".to_owned(), Value::String("endpoint_gone".to_owned())),
                ("actor".to_owned(), Value::String("sink".to_owned())),
                ("ok".to_owned(), Value::Bool(false)),
            ])
            .unwrap()
        );
        assert!(read_effect_outcome(&record).is_err());
    }

    #[test]
    fn system_header_lengths_and_public_plan_rejection_match_the_record_codec() {
        let producer = ActorId::System(circular_plan::SystemActor::Pipeline);
        let carrier = circular_store::record_actor_value(&producer).unwrap();
        assert!(circular_protocol::scope_identity::decode_plan_actor_key(carrier).is_err());
        let user = NamedActorId::new(ScopeId::root(), Name::from_normalized("effect-outcomes"));
        let named_carrier = circular_store::actor_value(&user.as_actor_id()).unwrap();
        assert!(circular_protocol::scope_identity::decode_plan_actor_key(named_carrier).is_ok());
        let at = Stamp::from_system_record_producer_at(
            circular_core::Hlc::from_physical(Tick::new(13)),
            circular_plan::ActorId::System(circular_plan::SystemActor::Pipeline),
            Sequence::new(1).unwrap(),
            RevisionEpochId::new(1).unwrap(),
        )
        .unwrap();
        let cause = stamp(12, 0);
        let result = Err(EffectFailure::EndpointGone);
        let encoded = EncodedOutcome::new(&actor(), &cause, None, &result, &[]).unwrap();
        check_record_lengths(&actor(), &producer, &cause, &encoded, &test_effect(41)).unwrap();
        assert_eq!(metadata_length(37, 9, 37, 97).unwrap(), 333);
        let record = effect_outcome_record(
            StreamId::new(31),
            &actor(),
            at,
            ArrivalIndex::FIRST,
            test_effect(41),
            &cause,
            None,
            &result,
            &[],
            RecordedInstant::from_millis(19),
        )
        .unwrap();
        let bytes = circular_store::encode_record(&record, &ProductRecordCodec).unwrap();
        assert_eq!(
            bytes.len(),
            332 + encoded.summary.len() + encoded.term.len() + encoded.body.len()
        );
        assert!(circular_store::reencodes_identically(&record));
        assert_eq!(
            read_effect_outcome(&record).unwrap().unwrap().result,
            result
        );
    }

    #[test]
    fn actual_identity_codec_keeps_header_u16_and_cause_u32_limits_separate() {
        let named =
            |length| NamedActorId::new(ScopeId::root(), Name::from_normalized("n".repeat(length)));
        let at = |producer| {
            Stamp::from_event_producer(
                Tick::new(20),
                producer,
                Sequence::new(0).unwrap(),
                RevisionEpochId::new(1).unwrap(),
            )
        };
        let result = Err(EffectFailure::EndpointGone);
        let valid = effect_outcome_record(
            StreamId::new(31),
            &actor(),
            at(named(65_502)),
            ArrivalIndex::FIRST,
            test_effect(41),
            &stamp(12, 0),
            None,
            &result,
            &[],
            RecordedInstant::from_millis(19),
        )
        .unwrap();
        assert_eq!(
            circular_store::encode_record(&valid, &ProductRecordCodec)
                .unwrap()
                .len(),
            65_945
        );
        assert!(
            effect_outcome_record(
                StreamId::new(31),
                &actor(),
                at(named(65_503)),
                ArrivalIndex::FIRST,
                test_effect(41),
                &stamp(12, 0),
                None,
                &result,
                &[],
                RecordedInstant::from_millis(19)
            )
            .unwrap_err()
            .contains("u16")
        );
        let valid_cause = effect_outcome_record(
            StreamId::new(31),
            &actor(),
            stamp(20, 0),
            ArrivalIndex::FIRST,
            test_effect(41),
            &at(named(65_503)),
            None,
            &result,
            &[],
            RecordedInstant::from_millis(19),
        )
        .unwrap();
        assert_eq!(
            circular_store::encode_record(&valid_cause, &ProductRecordCodec)
                .unwrap()
                .len(),
            65_946
        );
    }

    #[test]
    fn checked_envelopes_include_payload_prefixes_and_combined_record_size() {
        let max = u32::MAX as usize;
        assert!(check_lengths(100, [max - 102, 0, 0]).is_ok());
        assert!(check_lengths(100, [max - 101, 0, 0]).is_err());
        assert!(check_lengths(0, [max - 1, 0, 0]).is_err());
        assert!(check_lengths(100, [max / 2, max / 2, 0]).is_err());
        assert!(check_lengths(100, [usize::MAX, 0, 0]).is_err());
        assert!(check_lengths(usize::MAX, [0, 0, 0]).is_err());
    }

    #[test]
    fn opaque_body_has_no_new_one_megabyte_ceiling() {
        let result = Ok(OutcomePayload::FileBytes(
            vec![255; 1024 * 1024 + 1].into_boxed_slice(),
        ));
        let record = record(0, test_effect(41), &result);
        assert_eq!(
            read_effect_outcome(&record).unwrap().unwrap().result,
            result
        );
    }
}

/// The strict lookup domain is exactly one recorded (run, actor) column.
/// Runtime owns key + term comparison and duplicate rejection; no scan-by-order fallback.
pub fn read_effect_log(
    records: &[Record<ProductStore>],
    actor: &NamedActorId,
) -> Result<circular_runtime::OutcomeLog<circular_runtime::EffectId, Option<EffectTerm>>, String> {
    let mut column = Vec::new();
    for recorded in read_effect_outcomes(records)? {
        if &recorded.actor != actor {
            continue;
        }
        let Record::Boundary(boundary) = recorded.record else {
            unreachable!()
        };
        let BoundaryFact::Arrival { arrival_index, .. } = boundary.fact() else {
            continue;
        };
        column.push((
            *arrival_index,
            circular_runtime::LoggedOutcome::new(
                recorded.term,
                circular_runtime::EffectOutcome::new(recorded.effect, recorded.result)
                    .with_failure_progress(recorded.failure_progress),
            ),
        ));
    }
    circular_runtime::OutcomeLog::try_new(actor.as_actor_id(), column)
        .map_err(|error| format!("strict effect column: {error:?}"))
}
