use circular_core::{Stamp, Value};
use circular_protocol::declaration_payload::{QueryPage, Terminal};
use circular_store::{JournalProjection, ObservationFact, Record, SqliteJournal};
use std::path::Path;

const VERSION: u64 = 1;

pub(crate) fn capture_snapshot(
    directory: &Path,
) -> Result<crate::daemon::ledger::projection::ProjectionSnapshot, String> {
    let snapshot = SqliteJournal::read_only_namespace(
        engine::state_journal::state_journal_path(directory),
        engine::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
    )
    .map_err(|e| format!("restart records unavailable: {e}"))?;
    let mut items = Vec::new();
    let mut upto: Option<Stamp<circular_plan::ActorId>> = None;
    let mut reader = circular_store::ProductJournalReader::default();
    for entry in snapshot.entries() {
        let transaction = reader
            .read_entry(entry)
            .map_err(|e| format!("restart record transaction: {e}"))?;
        for record in circular_store::ArrivalProjection::new()
            .project(&transaction)
            .map_err(|e| format!("restart record projection: {e:?}"))?
        {
            let Record::Observation(record) = record else {
                continue;
            };
            let ObservationFact::Restart(payload) = record.fact() else {
                continue;
            };
            let body = circular_store::ProductRestartBody::decode(payload)?;
            if body.previous_sequence >= entry.sequence().get() {
                return Err("restart previous sequence does not precede its commit".into());
            }
            let stamp = record.header().at();
            if upto.as_ref().is_none_or(|last| stamp > last) {
                upto = Some(stamp.clone());
            }
            let witness = crate::daemon::ledger::record_witness(record.header())?;
            items.push(
                crate::daemon::ledger::projection::ProjectionRecord::recorded(
                    witness,
                    Value::array([stamp_value(stamp)?, Value::bytes(payload.as_bytes())]),
                ),
            );
        }
    }
    Ok(
        crate::daemon::ledger::projection::ProjectionSnapshot::recorded(
            Value::array([
                Value::UInt(VERSION),
                Value::UInt(u64::from(circular_store::SQLITE_FIXED_SCHEMA_VERSION)),
                Value::UInt(snapshot.through().map_or(0, |at| at.get())),
                upto.as_ref()
                    .map(stamp_value)
                    .transpose()?
                    .unwrap_or(Value::Null),
            ]),
            items,
            Vec::new(),
        ),
    )
}

#[cfg(test)]
pub(crate) fn capture(directory: &Path) -> Result<QueryPage, String> {
    let snapshot = capture_snapshot(directory)?;
    Ok(QueryPage {
        cut: None,
        folded_from: None,
        anchor: snapshot.anchor,
        items: snapshot
            .records
            .iter()
            .map(|row| row.value())
            .collect::<Result<_, _>>()?,
        terminal: Terminal::Complete,
        reached: None,
    })
}

pub(crate) fn stamp_value(stamp: &Stamp<circular_plan::ActorId>) -> Result<Value, String> {
    circular_store::record_stamp_value(stamp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::{Boundary, Ceilings, RevisionEpochId};
    use circular_protocol::declaration_payload::{PageRequest, Query, QueryResult};
    use circular_store::{ProductShutdownBody, RestartReason, StreamId};
    use std::{
        collections::BTreeMap,
        num::NonZeroU64,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "circular-restart-query-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn journal(&self) -> SqliteJournal {
            SqliteJournal::open_namespace(
                engine::state_journal::state_journal_path(&self.0),
                engine::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
            )
            .unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn stamp(sequence: u64, revision: u64) -> Value {
        Value::array([
            Value::UInt(0),
            Value::UInt(0),
            Value::Int(2),
            Value::UInt(sequence),
            Value::UInt(revision),
        ])
    }
    fn expected_body(previous: u64, boot: u8, wall: u64, reason: &str) -> Value {
        let mut bytes = vec![0, 1];
        bytes.extend(
            circular_core::encode(
                &Value::array([
                    Value::UInt(previous),
                    Value::bytes([boot; 16]),
                    Value::UInt(wall),
                    Value::string(reason),
                    Value::Array(vec![]),
                ]),
                Ceilings::for_boundary(Boundary::Journal),
            )
            .unwrap(),
        );
        Value::bytes(bytes)
    }
    fn request(cursor: Option<Value>) -> Query {
        Query {
            lens: None,
            since: None,
            upto: None,
            name: "observation-scan".into(),
            args: Value::Null,
            page: Some(PageRequest {
                limit: NonZeroU64::new(1).unwrap(),
                cursor,
            }),
        }
    }
    fn append_restart_fixture(
        journal: &mut SqliteJournal,
        revision: RevisionEpochId,
        boot_id: [u8; 16],
        wall_millis: u64,
    ) -> Result<(), String> {
        use circular_store::SqliteTransactionCodec;
        let snapshot = journal.snapshot().map_err(|e| e.to_string())?;
        let previous = snapshot.through().map_or(0, |at| at.get());
        let at = circular_core::Stamp::from_system_record_producer_at(
            circular_core::Hlc::from_physical(circular_core::Tick::ZERO),
            circular_plan::ActorId::System(circular_plan::SystemActor::Pipeline),
            circular_core::Sequence::new(previous + 1).unwrap(),
            revision,
        )
        .unwrap();
        let record = circular_store::restart_record(
            at,
            &circular_store::ProductRestartBody {
                previous_sequence: previous,
                boot_id,
                wall_millis,
                reason: circular_store::RestartReason::Crash,
                unsettled: vec![],
            },
        )?;
        let transaction =
            circular_store::StoreTransaction::<circular_store::ProductTransaction>::try_new(vec![
                circular_store::StoreTransactionOp::Append(circular_store::TransactionAppend::new(
                    circular_store::OpaqueId::new(previous),
                    circular_core::EncodedPayload::new(
                        circular_core::PayloadVersionTag::FIRST,
                        &circular_store::encode_record(
                            &record,
                            &circular_store::ProductRecordCodec,
                        )
                        .map_err(|e| format!("{e:?}"))?,
                    ),
                )),
            ])
            .map_err(|e| format!("{e:?}"))?;
        let column = circular_store::ColumnWriteContext::new(&circular_plan::ActorId::System(
            circular_plan::SystemActor::Pipeline,
        ))
        .map_err(|e| format!("{e:?}"))?;
        journal
            .commit(
                &circular_store::ProductJournalCodec
                    .encode(&column.bind(transaction))
                    .map_err(|e| format!("{e:?}"))?,
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[test]
    fn restart_query_retains_exact_prefix_across_append_and_journal_replacement() {
        let f = Fixture::new();
        let mut journal = f.journal();
        for n in 1..=2 {
            append_restart_fixture(
                &mut journal,
                RevisionEpochId::new(7).unwrap(),
                [n; 16],
                n as u64,
            )
            .unwrap();
        }
        let mut open = crate::daemon::query::OpenQueries::default();
        let QueryResult::Page(first) = crate::daemon::query::prepare_record_query(
            7,
            &request(None),
            &f.0,
            &mut open,
            no_sources,
        ) else {
            panic!("page")
        };
        assert_eq!(
            first.items,
            vec![Value::array([stamp(1, 7), expected_body(0, 1, 1, "crash")])]
        );
        let Terminal::More(cursor) = &first.terminal else {
            panic!("immutable cursor")
        };
        let fields = cursor.as_object().unwrap();
        assert_eq!(fields.get("anchor"), Some(&first.anchor));
        assert_eq!(
            fields.get("domain"),
            Some(&Value::string("observation-scan"))
        );
        assert!(matches!(fields.get("position"), Some(Value::Bytes(_))));
        append_restart_fixture(&mut journal, RevisionEpochId::new(9).unwrap(), [3; 16], 3).unwrap();
        drop(journal);
        let path = engine::state_journal::state_journal_path(&f.0);
        std::fs::remove_file(&path).unwrap();
        SqliteJournal::create(&path).unwrap();
        let QueryResult::Page(last) = crate::daemon::query::prepare_record_query(
            7,
            &request(Some(cursor.clone())),
            &f.0,
            &mut open,
            no_sources,
        ) else {
            panic!("page")
        };
        assert_eq!(last.anchor, first.anchor);
        assert_eq!(
            last.items,
            vec![Value::array([stamp(2, 7), expected_body(1, 2, 2, "crash")])]
        );
        assert_eq!(last.terminal, Terminal::Complete);
        assert!(capture(&f.0).unwrap().items.is_empty());
    }
    fn no_sources() -> crate::daemon::subscription::records::Sources {
        unreachable!("observation-scan does not read the records sources")
    }
    #[test]
    fn restart_query_rejects_missing_store_bad_arguments_and_unknown_cursors() {
        let f = Fixture::new();
        assert!(capture(&f.0).is_err());
        assert!(!engine::state_journal::state_journal_path(&f.0).exists());
        let _journal = f.journal();
        let mut open = crate::daemon::query::OpenQueries::default();
        let mut bad = request(None);
        bad.args = Value::UInt(1);
        assert!(matches!(
            crate::daemon::query::prepare_record_query(1, &bad, &f.0, &mut open, no_sources),
            QueryResult::Rejected(_)
        ));
        assert!(matches!(
            crate::daemon::query::prepare_record_query(
                1,
                &request(Some(Value::Int(0))),
                &f.0,
                &mut open,
                no_sources
            ),
            QueryResult::Rejected(_)
        ));
        let empty = capture(&f.0).unwrap();
        assert_eq!(
            empty.anchor,
            Value::array([Value::UInt(1), Value::UInt(1), Value::UInt(0), Value::Null])
        );
        assert!(empty.items.is_empty());
    }
    #[test]
    fn restart_query_corrupt_record_lane_is_not_an_empty_success() {
        let f = Fixture::new();
        let mut journal = f.journal();
        journal.commit(b"invalid transaction").unwrap();
        assert!(capture(&f.0).unwrap_err().contains("transaction"));
    }
    #[test]
    fn restart_query_refuses_unknown_body_version_and_malformed_restart() {
        use circular_core::{EncodedPayload, PayloadVersionTag};
        use circular_store::*;
        for version in [1, 2] {
            let f = Fixture::new();
            let mut journal = f.journal();
            let at = circular_core::Stamp::from_system_record_producer_at(
                circular_core::Hlc::new(
                    circular_core::Tick::ZERO,
                    circular_core::LogicalCounter::ZERO,
                ),
                circular_plan::ActorId::System(circular_plan::SystemActor::Pipeline),
                circular_core::Sequence::new(1).unwrap(),
                RevisionEpochId::new(7).unwrap(),
            )
            .unwrap();
            let record = Record::<ProductStore>::Observation(ObservationRecord::restart(
                at,
                ObservationBucket::from_millis(100),
                RecordOrigin::Stream,
                ObservationItemKey::new(
                    circular_core::BuiltinObservationName::Restart,
                    OpaqueId::new(1),
                ),
                EncodedPayload::new(PayloadVersionTag::new(version).unwrap(), &[0]),
            ));
            let bytes = encode_record(&record, &ProductRecordCodec).unwrap();
            let transaction =
                StoreTransaction::<ProductTransaction>::try_new(vec![StoreTransactionOp::Append(
                    TransactionAppend::new(
                        OpaqueId::new(0),
                        EncodedPayload::new(PayloadVersionTag::FIRST, &bytes),
                    ),
                )])
                .unwrap();
            let column = ColumnWriteContext::new(&circular_plan::ActorId::System(
                circular_plan::SystemActor::Pipeline,
            ))
            .unwrap();
            journal
                .commit(
                    &ProductJournalCodec
                        .encode(&column.bind(transaction))
                        .unwrap(),
                )
                .unwrap();
            assert!(capture(&f.0).is_err());
        }
    }
}
