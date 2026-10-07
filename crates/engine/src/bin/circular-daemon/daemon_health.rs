
use std::collections::BTreeMap;

use circular_core::{Boundary, Ceilings, Stamp, Value};
use circular_protocol::actor_events::ActorHealthTransition;
use circular_protocol::declaration_payload::{QueryPage, Terminal};
use circular_store::{ClassKey, ObservationFact, ObservationKey, Record};

/// Registered name. The daemon owns this spelling the way it owns
/// `pipelines`; nothing about it belongs to the protocol crate's vocabulary.
pub(crate) const DAEMON_HEALTH_QUERY: &str = "daemon.health";

/// Anchor body version. One number, so a reader never guesses which fields a
/// daemon of another vintage published.
const VERSION: u64 = 1;

use crate::kernel::system::{SystemBody, SystemFold};
use circular_protocol::lifecycle::LifecycleWord;

/// Facts this fold reads out of the arrival journal for one standing run.
#[derive(Default)]
struct RecordedHealth {
    /// Latest recorded transition per actor, keyed by the canonical encoding of
    /// its published identity so the answer's row order is deterministic.
    latest: BTreeMap<Vec<u8>, (Value, ActorHealthTransition, u64)>,
    dead_letter: Option<Stamp<circular_plan::ActorId>>,
    journal: Option<(ActorHealthTransition, u64)>,
    wall_clock: Option<(u64, u64)>,
}

/// The journal owner's identity as a health transition names it — the run's
/// health producer (`ledger::actor_health_producer`).
fn journal_owner() -> Result<circular_protocol::scope_identity::PlanActorKey, String> {
    let value = circular_store::named_actor_value(&crate::daemon::ledger::actor_health_producer())
        .map_err(|error| format!("journal owner identity: {error:?}"))?;
    circular_protocol::declaration_payload::decode_plan_actor_key(value)
        .map_err(|error| format!("journal owner identity: {error:?}"))
}

pub(crate) fn recorded_journal_ceiling(
    records: &circular_store::JournalView,
) -> Result<Option<String>, String> {
    let observed = fold_records(Some(records), None)?;
    Ok(observed
        .journal
        .as_ref()
        .and_then(|(transition, _)| ceiling_code(transition))
        .map(ToOwned::to_owned))
}

fn ceiling_code(transition: &ActorHealthTransition) -> Option<&str> {
    if transition.state != circular_protocol::actor_events::ActorHealthState::Backpressure {
        return None;
    }
    let reason = transition.reason.as_ref()?;
    if reason.code != circular_protocol::actor_events::ActorHealthReasonCode::Capacity {
        return None;
    }
    match reason.detail.as_object()?.get("code")? {
        Value::String(code) => Some(code.as_str()),
        _ => None,
    }
}

fn journal_value(
    recorded: Option<&(ActorHealthTransition, u64)>,
    usage: Option<&crate::daemon::runtime_arrival_retention::JournalMeasure>,
) -> Result<Value, String> {
    let ceiling = match recorded {
        Some((transition, ordinal)) => match ceiling_code(transition) {
            Some(code) => Value::object([
                ("code", Value::string(code)),
                ("record", Value::UInt(*ordinal)),
                ("since_ms", Value::UInt(transition.since_ms)),
            ])
            .map_err(|error| format!("journal ceiling: {error:?}"))?,
            None => Value::Null,
        },
        None => Value::Null,
    };
    Value::object([
        ("ceiling", ceiling),
        (
            "usage",
            usage
                .map(crate::daemon::runtime_arrival_retention::JournalMeasure::value)
                .transpose()?
                .unwrap_or(Value::Null),
        ),
    ])
    .map_err(|error| format!("journal: {error:?}"))
}

/// Read the System column from the same immutable journal prefix as actor health.
/// Diagnostic outcomes are not in the lifecycle index.
pub(crate) fn system_fold(
    records: Option<&circular_store::JournalView>,
    bound: Option<&crate::daemon::ledger::ReadBound>,
) -> Result<SystemFold, String> {
    let mut fold = SystemFold::default();
    let Some(records) = records else {
        return Ok(fold);
    };
    for row in records.all_rows()? {
        let row = row?;
        let record = row.record();
        if let Some(bound) = bound {
            if !bound.admits(row.position()) {
                continue;
            }
        }
        if let Some(body) = SystemBody::read(&record)? {
            fold.apply(record.header().at(), &body);
        }
    }
    Ok(fold)
}

pub(crate) fn lifecycle_word(fold: &SystemFold) -> LifecycleWord {
    let receipts = [
        (&fold.last_activation, LifecycleWord::ActivationFailed),
        (
            &fold.last_revision_adoption,
            LifecycleWord::RevisionAdoptionFailed,
        ),
        (&fold.last_recovery, LifecycleWord::RecoveryFailed),
    ]
    .map(|(receipt, failed)| {
        receipt
            .as_ref()
            .map(|(at, result)| (at, result.is_ok(), failed))
    });
    let [activation, _, recovery] = receipts;
    if let Some((_, false, failed)) = [activation, recovery]
        .into_iter()
        .flatten()
        .max_by_key(|(at, _, _)| at.sequence())
    {
        return failed;
    }
    let covered = |failure: &Stamp<circular_plan::ActorId>| {
        receipts.iter().flatten().any(|(success, ok, _)| {
            *ok && success.sequence() > failure.sequence()
                && success.revision() >= failure.revision()
        })
    };
    let uncovered = receipts
        .iter()
        .flatten()
        .filter(|(at, ok, _)| !ok && !covered(at))
        .max_by_key(|(at, _, _)| at.sequence())
        .map(|(_, _, failed)| *failed);
    match uncovered {
        Some(failed) => failed,
        None if fold.pause.is_some() => LifecycleWord::Stopped,
        None => LifecycleWord::Running,
    }
}

#[cfg(test)]
pub(crate) fn health_page(
    records: &circular_store::JournalView,
    stream: Option<circular_store::StreamId>,
    storage: Option<circular_protocol::rejection_code::RejectionReason>,
) -> Result<QueryPage, String> {
    health_page_within(Some(records), stream, storage, &[], None, None)
}

pub(crate) fn health_page_within(
    records: Option<&circular_store::JournalView>,
    stream: Option<circular_store::StreamId>,
    storage: Option<circular_protocol::rejection_code::RejectionReason>,
    config_defaults: &[(Box<str>, u64)],
    bound: Option<&crate::daemon::ledger::ReadBound>,
    journal_usage: Option<crate::daemon::runtime_arrival_retention::JournalMeasure>,
) -> Result<QueryPage, String> {
    let system = system_fold(records, bound)?;
    let word = stream.map(|_| lifecycle_word(&system));
    let observed = fold_records(records, bound)?;
    let config_defaults = config_defaults
        .iter()
        .map(|(key, value)| {
            Value::object([
                ("key", Value::string(key.as_ref())),
                ("value", Value::UInt(*value)),
            ])
            .map_err(|error| format!("daemon health config default: {error:?}"))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let anchor = Value::object([
        ("config_defaults", Value::array(config_defaults)),
        (
            "dead_letter",
            observed
                .dead_letter
                .as_ref()
                .map(crate::daemon::restart_query::stamp_value)
                .transpose()?
                .unwrap_or(Value::Null),
        ),
        (
            "journal",
            journal_value(observed.journal.as_ref(), journal_usage.as_ref())?,
        ),
        (
            "lifecycle",
            word.map_or(Value::Null, |word| Value::string(word.as_str())),
        ),
        (
            "wall_clock",
            observed
                .wall_clock
                .map_or(Ok(Value::Null), |(wall_ms, at_ms)| {
                    Value::object([
                        ("at_ms", Value::UInt(at_ms)),
                        ("wall_ms", Value::UInt(wall_ms)),
                    ])
                    .map_err(|error| format!("daemon health wall clock: {error:?}"))
                })?,
        ),
        (
            "storage",
            storage.map_or(Value::Null, |reason| {
                Value::UInt(u64::from(reason.recorded_code()))
            }),
        ),
        ("version", Value::UInt(VERSION)),
    ])
    .map_err(|error| format!("daemon health anchor: {error:?}"))?;
    let items = observed
        .latest
        .into_values()
        .map(|(actor, transition, ordinal)| {
            let detail = transition
                .reason
                .as_ref()
                .map_or(Ok(Value::Null), |reason| row_detail(&reason.detail))?;
            Value::object([
                ("actor", actor),
                ("detail", detail),
                ("record", Value::UInt(ordinal)),
                (
                    "reason",
                    transition
                        .reason
                        .as_ref()
                        .map_or(Value::Null, |reason| Value::string(reason.code.as_str())),
                ),
                ("since_ms", Value::UInt(transition.since_ms)),
                ("state", Value::string(transition.state.as_str())),
            ])
            .map_err(|error| format!("daemon health row: {error:?}"))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(QueryPage {
        cut: None,
        folded_from: None,
        reached: None,
        anchor,
        items,
        terminal: Terminal::Complete,
    })
}

fn row_detail(detail: &Value) -> Result<Value, String> {
    let Some(fields) = detail.as_object() else {
        return Ok(Value::Null);
    };
    let shaped = fields.len() == 2
        && matches!(fields.get("code"), Some(Value::String(_)))
        && matches!(fields.get("slot"), Some(Value::String(_) | Value::Null));
    if shaped {
        Ok(detail.clone())
    } else {
        Err("daemon health reason detail is an object but not {code, slot}".to_owned())
    }
}

fn fold_records(
    records: Option<&circular_store::JournalView>,
    bound: Option<&crate::daemon::ledger::ReadBound>,
) -> Result<RecordedHealth, String> {
    let mut observed = RecordedHealth::default();
    let Some(records) = records else {
        return Ok(observed);
    };
    let owner = journal_owner()?;
    for row in records.all_rows()? {
        let row = row?;
        if bound.is_some_and(|bound| !bound.admits(row.position())) {
            break;
        }
        let ordinal = records.ordinal(row.position())?.get();
        if !crate::daemon::record_rows::keeps(&row, health_candidate)? {
            continue;
        }
        let slot = crate::daemon::record_rows::rebuild(&row);
        let Record::Observation(record) = &*slot else {
            continue;
        };
        if let ClassKey::Observation(ObservationKey::StreamItem(_, _, item)) = record.header().key()
        {
            let wall_millis = match record.fact() {
                ObservationFact::Restart(payload) => Some(
                    circular_store::ProductRestartBody::decode(payload)
                        .map_err(|error| format!("daemon health restart body: {error}"))?
                        .wall_millis,
                ),
                ObservationFact::Lifecycle(payload)
                    if *item.kind() == circular_core::BuiltinObservationName::StreamStart =>
                {
                    Some(
                        circular_store::ProductStreamStartBody::decode(payload)
                            .map_err(|error| format!("daemon health stream start body: {error}"))?
                            .wall_millis,
                    )
                }
                _ => None,
            };
            if let Some(wall_millis) = wall_millis {
                observed.wall_clock =
                    Some((wall_millis, record.header().at().physical_time().get()));
                continue;
            }
        }
        let ClassKey::Observation(ObservationKey::StreamItem(_, _, item)) = record.header().key()
        else {
            continue;
        };
        if !crate::daemon::subscription::records::published_fact(
            *item.kind(),
            circular_store::observation_fact_tag(record.fact()),
        ) {
            continue;
        }
        match record.fact() {
            ObservationFact::DeadLetter(_) => {
                let at = record.header().at();
                if observed.dead_letter.as_ref().is_none_or(|last| at > last) {
                    observed.dead_letter = Some(at.clone());
                }
            }
            ObservationFact::Diagnostic(payload) => {
                let body = circular_core::decode(
                    payload.body(),
                    Ceilings::for_boundary(Boundary::Journal),
                )
                .map_err(|error| format!("daemon health diagnostic body: {error:?}"))?;
                if !matches!(
                    body.as_object().and_then(|fields| fields.get("kind")),
                    Some(Value::String(kind)) if kind == engine::ACTOR_HEALTH_TRANSITION_KIND
                ) {
                    continue;
                }
                let transition =
                    circular_protocol::actor_events::decode_actor_health_transition(body)
                        .map_err(|error| format!("daemon health transition: {error:?}"))?
                        .ok_or("recorded diagnostic is not a health transition")?;
                if transition.actor == owner {
                    observed.journal = Some((transition, ordinal));
                    continue;
                }
                let actor =
                    circular_protocol::boundary_port::encode_boundary_actor_key(&transition.actor)
                        .map_err(|error| format!("daemon health actor: {error:?}"))?;
                let key = circular_core::encode(&actor, Ceilings::for_boundary(Boundary::Identity))
                    .map_err(|error| format!("daemon health actor identity: {error:?}"))?;
                observed.latest.insert(key, (actor, transition, ordinal));
            }
            _ => {}
        }
    }
    Ok(observed)
}

fn health_candidate(envelope: &circular_store::RecordEnvelope<'_>) -> bool {
    let Some(kind) = crate::daemon::record_rows::observation_kind(envelope) else {
        return false;
    };
    let Some(fact) = envelope.observation_fact_tag else {
        return false;
    };
    if (kind == circular_core::BuiltinObservationName::Restart
        && fact == circular_store::RESTART_FACT_TAG)
        || (kind == circular_core::BuiltinObservationName::StreamStart
            && fact == circular_store::LIFECYCLE_FACT_TAG)
    {
        return true;
    }
    matches!(
        fact,
        circular_store::DEAD_LETTER_FACT_TAG | circular_store::DIAGNOSTIC_FACT_TAG
    ) && crate::daemon::subscription::records::published_fact(kind, fact)
}

#[cfg(test)]
pub(crate) fn restored_prefix(state_directory: &std::path::Path) -> circular_store::JournalView {
    use circular_store::Store as _;
    let path = engine::state_journal::state_journal_path(state_directory);
    let journal = engine::ProductDurableArrivalJournal::open_read_only(&path)
        .expect("restored arrival prefix");
    let page = engine::PRODUCT_STORE_PAGE_MAXIMUM;
    let mut store = circular_store::MemoryStore::<circular_store::ProductStore>::new(
        circular_store::PagePolicy::new(page, page).expect("accepted page policy"),
    );
    let records = journal.records().to_vec();
    if !records.is_empty() {
        match store.append(circular_store::AppendBatch::try_new(records).expect("restored batch")) {
            circular_store::AppendResult::Committed(_) => store.seal_all(),
            other => panic!("restored prefix append failed: {other:?}"),
        }
    }
    circular_store::JournalView::memory(std::sync::Arc::new(store))
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::{
        BuiltinObservationName, EncodedPayload, PayloadVersionTag, RevisionEpochId, Sequence, Tick,
    };
    use circular_plan::{Name, NamedActorId, ScopeId};
    use circular_store::{
        AppendBatch, ObservationBucket, ObservationItemKey, ObservationRecord, OpaqueId,
        RecordOrigin,
    };
    use circular_store::{AppendResult, MemoryStore, PagePolicy, Store};

    struct Fixture {
        store: circular_store::JournalView,
    }

    fn stamp(sequence: u64) -> Stamp<circular_plan::ActorId> {
        Stamp::from_event_producer(
            Tick::new(10),
            NamedActorId::new(ScopeId::root(), Name::from_normalized("agent")),
            Sequence::new(sequence).unwrap(),
            RevisionEpochId::new(1).unwrap(),
        )
    }

    impl Fixture {
        fn new() -> Self {
            let page = engine::PRODUCT_STORE_PAGE_MAXIMUM;
            Self {
                store: circular_store::JournalView::memory(std::sync::Arc::new(MemoryStore::new(
                    PagePolicy::new(page, page).unwrap(),
                ))),
            }
        }

        fn append(&mut self, records: Vec<Record<circular_store::ProductStore>>) {
            let batch = AppendBatch::try_new(records).unwrap();
            let store = self.store.memory_mut().unwrap();
            match store.append(batch) {
                AppendResult::Committed(_) => store.seal_all(),
                other => panic!("prefix append failed: {other:?}"),
            }
        }

        fn records(&self) -> &circular_store::JournalView {
            &self.store
        }
    }

    fn health(
        sequence: u64,
        local: &str,
        state: &str,
        reason: Option<&str>,
    ) -> Record<circular_store::ProductStore> {
        health_with_detail(sequence, local, state, reason, Value::Null)
    }

    fn health_with_detail(
        sequence: u64,
        local: &str,
        state: &str,
        reason: Option<&str>,
        detail: Value,
    ) -> Record<circular_store::ProductStore> {
        let mut fields = vec![
            (
                "kind",
                Value::String(engine::ACTOR_HEALTH_TRANSITION_KIND.into()),
            ),
            (
                "actor",
                Value::object([
                    ("scope", Value::Array(vec![])),
                    ("local", Value::String(local.into())),
                ])
                .unwrap(),
            ),
            ("state", Value::String(state.into())),
            ("since_ms", Value::Int(i64::try_from(sequence).unwrap())),
        ];
        if let Some(reason) = reason {
            fields.push((
                "reason",
                Value::object([("code", Value::String(reason.into())), ("detail", detail)])
                    .unwrap(),
            ));
        }
        let body = Value::object(fields).unwrap();
        Record::Observation(ObservationRecord::diagnostic(
            stamp(sequence),
            ObservationBucket::from_millis(10),
            RecordOrigin::Stream,
            ObservationItemKey::new(
                BuiltinObservationName::DiagnosticOccurrence,
                OpaqueId::new(sequence),
            ),
            EncodedPayload::new(
                PayloadVersionTag::FIRST,
                &circular_core::encode(&body, Ceilings::for_boundary(Boundary::Journal)).unwrap(),
            ),
        ))
    }

    fn dead_letter(sequence: u64) -> Record<circular_store::ProductStore> {
        Record::Observation(ObservationRecord::dead_letter(
            stamp(sequence),
            ObservationBucket::from_millis(10),
            RecordOrigin::Stream,
            ObservationItemKey::new(
                BuiltinObservationName::DeadLetterEntry,
                OpaqueId::new(sequence),
            ),
            EncodedPayload::new(
                PayloadVersionTag::FIRST,
                &circular_core::encode(&Value::Null, Ceilings::for_boundary(Boundary::Journal))
                    .unwrap(),
            ),
        ))
    }

    fn field<'a>(page: &'a QueryPage, name: &str) -> &'a Value {
        page.anchor
            .as_object()
            .expect("anchor is an object")
            .get(name)
            .expect("anchor field")
    }

    fn system_observation(sequence: u64, body: Value) -> Record<circular_store::ProductStore> {
        system_observation_at(sequence, 1, body)
    }

    fn system_observation_at(
        sequence: u64,
        revision: u64,
        body: Value,
    ) -> Record<circular_store::ProductStore> {
        use circular_core::{EncodedPayload, PayloadVersionTag};
        use circular_store::{
            ObservationBucket, ObservationItemKey, ObservationRecord, OpaqueId, RecordOrigin,
        };
        let tag = match &body.as_array().unwrap()[0] {
            Value::UInt(tag) => *tag,
            _ => panic!("tag"),
        };
        let name = circular_core::BuiltinObservationName::from_tag(tag as u8).unwrap();
        let at = Stamp::from_system_record_producer_at(
            circular_core::Hlc::from_physical(circular_core::Tick::new(sequence)),
            circular_plan::ActorId::System(circular_plan::SystemActor::Pipeline),
            circular_core::Sequence::new(sequence).unwrap(),
            circular_core::RevisionEpochId::new(revision).unwrap(),
        )
        .unwrap();
        let bytes =
            circular_core::encode(&body, Ceilings::for_boundary(Boundary::Journal)).unwrap();
        let payload = EncodedPayload::new(PayloadVersionTag::FIRST, &bytes);
        let origin = RecordOrigin::Stream;
        let item = ObservationItemKey::new(name, OpaqueId::new(0));
        Record::Observation(if tag <= 43 {
            ObservationRecord::diagnostic(
                at,
                ObservationBucket::from_millis(sequence),
                origin,
                item,
                payload,
            )
        } else {
            ObservationRecord::lifecycle(
                at,
                ObservationBucket::from_millis(sequence),
                origin,
                item,
                payload,
            )
        })
    }

    fn running() -> Option<circular_store::StreamId> {
        Some(circular_store::StreamId::new(1))
    }

    #[test]
    fn the_answer_is_the_latest_transition_per_actor_with_a_closed_reason_code() {
        let mut fixture = Fixture::new();
        fixture.append(vec![
            health(2, "alpha", "failed", Some("poisoned")),
            health(3, "beta", "backpressure", Some("capacity")),
            health(4, "gamma", "waiting", None),
        ]);
        fixture.append(vec![
            health(5, "alpha", "running", None),
            health(6, "delta", "failed", Some("interpreter_fault")),
        ]);
        let page = health_page(fixture.records(), running(), None).unwrap();
        let expected = |local: &str, state: &str, reason: Option<&str>, since: u64, record: u64| {
            Value::object([
                (
                    "actor",
                    Value::object([
                        ("local", Value::String(local.into())),
                        ("scope", Value::Array(vec![])),
                    ])
                    .unwrap(),
                ),
                ("detail", Value::Null),
                ("reason", reason.map_or(Value::Null, Value::string)),
                ("record", Value::UInt(record)),
                ("since_ms", Value::UInt(since)),
                ("state", Value::String(state.into())),
            ])
            .unwrap()
        };
        assert_eq!(
            page.items,
            vec![
                expected("beta", "backpressure", Some("capacity"), 3, 2),
                expected("alpha", "running", None, 5, 4),
                expected("delta", "failed", Some("interpreter_fault"), 6, 5),
                expected("gamma", "waiting", None, 4, 3),
            ],
            "every recorded state remains, in canonical actor identity order"
        );
        assert_eq!(field(&page, "dead_letter"), &Value::Null);
    }

    fn journal_ceiling(code: &str, record: u64, since_ms: u64) -> Value {
        Value::object([
            ("code", Value::string(code)),
            ("record", Value::UInt(record)),
            ("since_ms", Value::UInt(since_ms)),
        ])
        .unwrap()
    }

    fn journal(ceiling: Value, usage: Value) -> Value {
        Value::object([("ceiling", ceiling), ("usage", usage)]).unwrap()
    }

    #[test]
    fn journal_usage_is_the_handles_measure() {
        let fixture = Fixture::new();
        let measure = crate::daemon::runtime_arrival_retention::JournalMeasure {
            bytes: 7,
            records: 3,
            file_bytes: 11,
            limits: crate::daemon::runtime_arrival_retention::RuntimeArrivalLimits {
                arrivals_max_bytes: 5,
                arrivals_max_records: 100,
                total_max_bytes: 10,
            },
        };
        let page = health_page_within(
            Some(fixture.records()),
            running(),
            None,
            &[],
            None,
            Some(measure),
        )
        .unwrap();
        let usage = Value::object([
            ("bytes", Value::UInt(7)),
            ("file_bytes", Value::UInt(11)),
            ("records", Value::UInt(3)),
            ("arrivals_max_bytes", Value::UInt(5)),
            ("arrivals_max_records", Value::UInt(100)),
            ("total_max_bytes", Value::UInt(10)),
        ])
        .unwrap();
        assert_eq!(field(&page, "journal"), &journal(Value::Null, usage));
        assert_eq!(
            measure.exceeded().map(circular_actors::FailureDetail::code),
            Some("journal.arrivals_bytes_and_total_bytes_exceeded")
        );
    }

    #[test]
    fn the_fold_never_appends_and_repeats_itself() {
        let mut fixture = Fixture::new();
        fixture.append(vec![
            health(2, "alpha", "failed", Some("poisoned")),
            health(3, "beta", "running", None),
            health(4, "gamma", "waiting", None),
        ]);
        let before = fixture.records().tail().records().to_vec();
        let first = health_page(fixture.records(), running(), None).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let second = health_page(fixture.records(), running(), None).unwrap();
        assert_eq!(first.items, second.items);
        assert_eq!(first.anchor, second.anchor);
        assert_eq!(fixture.records().tail().records().to_vec(), before);
    }
}
