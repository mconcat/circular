use crate::{ProductStore, RestartReason};
use circular_core::{
    Boundary, BuiltinObservationName, Ceilings, EncodedPayload, PayloadVersionTag, Value,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductShutdownBody {
    pub reason: RestartReason,
}
impl ProductShutdownBody {
    pub fn encode(&self) -> Result<EncodedPayload, String> {
        if self.reason == RestartReason::Crash {
            return Err("crash has no shutdown body".into());
        }
        let value = Value::array([Value::string(self.reason.as_str())]);
        let bytes = circular_core::encode(&value, Ceilings::for_boundary(Boundary::Journal))
            .map_err(|e| e.to_string())?;
        Ok(EncodedPayload::new(PayloadVersionTag::FIRST, &bytes))
    }
    pub fn decode(payload: &EncodedPayload) -> Result<Self, String> {
        if payload.version_tag() != PayloadVersionTag::FIRST {
            return Err("unsupported shutdown body version".into());
        }
        let value =
            circular_core::decode(payload.body(), Ceilings::for_boundary(Boundary::Journal))
                .map_err(|e| e.to_string())?;
        let Some([Value::String(reason)]) = value.as_array() else {
            return Err("shutdown requires [reason String]".into());
        };
        let reason = RestartReason::parse(reason)?;
        if reason == RestartReason::Crash {
            return Err("crash has no shutdown body".into());
        }
        Ok(Self { reason })
    }
}

/// Construct the final lifecycle fact from the System actor's finished stamp.
pub fn shutdown_record(
    at: circular_core::Stamp<circular_runtime::ActorId>,
    body: &ProductShutdownBody,
) -> Result<crate::Record<ProductStore>, String> {
    if at.producer() != &circular_runtime::ActorId::System(circular_runtime::SystemActor::Pipeline)
    {
        return Err("shutdown is a System(Pipeline) record".into());
    }
    let identity = crate::OpaqueId::new(at.sequence().get());
    let bucket = crate::ObservationBucket::from_millis(at.physical_time().get());
    Ok(crate::Record::Observation(
        crate::ObservationRecord::lifecycle(
            at,
            bucket,
            crate::RecordOrigin::Stream,
            crate::ObservationItemKey::new(BuiltinObservationName::DaemonShutdown, identity),
            body.encode()?,
        ),
    ))
}

/// Read the final physical commit, not the last matching observation anywhere
/// in history. Missing evidence is crash; malformed evidence is an error.
/// `source` restores the System segment from its own last keyframe.
pub(crate) fn last_shutdown(
    snapshot: &crate::SqliteJournalSnapshot,
    source: &crate::SqliteJournal,
) -> Result<Option<ProductShutdownBody>, String> {
    use crate::JournalProjection;
    let Some(entry) = snapshot
        .entries()
        .last()
        .filter(|entry| Some(entry.sequence()) == snapshot.through())
    else {
        return Ok(None);
    };
    let transaction = crate::ProductJournalReader::default()
        .read_at(source, entry.sequence().get(), entry.payload())
        .map_err(|e| format!("shutdown transaction: {e}"))?;
    let records = crate::ArrivalProjection::new()
        .project(&transaction)
        .map_err(|e| format!("shutdown projection: {e:?}"))?;
    let Some(crate::Record::<ProductStore>::Observation(record)) = records.last() else {
        return Ok(None);
    };
    let crate::ClassKey::Observation(crate::ObservationKey::StreamItem(_, _, item)) =
        record.header().key()
    else {
        return Ok(None);
    };
    if *item.kind() != BuiltinObservationName::DaemonShutdown {
        return Ok(None);
    }
    if transaction.operations().len() != 1
        || !matches!(
            record.header().at().producer(),
            circular_runtime::ActorId::System(circular_runtime::SystemActor::Pipeline)
        )
        || record.header().origin() != &crate::RecordOrigin::Stream
    {
        return Err("shutdown record ownership or final commit is invalid".into());
    }
    let crate::ObservationFact::Lifecycle(payload) = record.fact() else {
        return Err("shutdown is not a Lifecycle observation".into());
    };
    ProductShutdownBody::decode(payload).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SqliteTransactionCodec;
    struct Directory(circular_testkit::temp::StateDir);
    impl Directory {
        fn new() -> Self {
            Self(circular_testkit::temp::StateDir::new("shutdown-store"))
        }
    }
    fn stamp() -> circular_core::Stamp<circular_runtime::ActorId> {
        circular_core::Stamp::from_system_record_producer_at(
            circular_core::Hlc::from_physical(circular_core::Tick::new(1)),
            circular_runtime::ActorId::System(circular_runtime::SystemActor::Pipeline),
            circular_core::Sequence::FIRST,
            circular_core::RevisionEpochId::new(1).unwrap(),
        )
        .unwrap()
    }
    #[test]
    fn corrupt_shutdown_evidence_is_not_reclassified_as_crash() {
        let dir = Directory::new();
        let mut journal =
            crate::SqliteJournal::open_namespace(dir.0.path().join("journal.sqlite3"), "fixture")
                .unwrap();
        let record =
            crate::Record::<ProductStore>::Observation(crate::ObservationRecord::lifecycle(
                stamp(),
                crate::ObservationBucket::from_millis(1),
                crate::RecordOrigin::Stream,
                crate::ObservationItemKey::new(
                    BuiltinObservationName::DaemonShutdown,
                    crate::OpaqueId::new(1),
                ),
                EncodedPayload::new(PayloadVersionTag::FIRST, &[0xff]),
            ));
        let bytes = crate::encode_record(&record, &crate::ProductRecordCodec).unwrap();
        let transaction = crate::StoreTransaction::<crate::ProductTransaction>::try_new(vec![
            crate::StoreTransactionOp::Append(crate::TransactionAppend::new(
                crate::OpaqueId::new(1),
                EncodedPayload::new(PayloadVersionTag::FIRST, &bytes),
            )),
        ])
        .unwrap();
        let column = crate::ColumnWriteContext::new(&circular_runtime::ActorId::System(
            circular_runtime::SystemActor::Pipeline,
        ))
        .unwrap();
        journal
            .commit(
                &crate::ProductJournalCodec
                    .encode(&column.bind(transaction))
                    .unwrap(),
            )
            .unwrap();
        let snapshot = journal.snapshot().unwrap();
        assert!(last_shutdown(&snapshot, &journal).is_err());
        assert!(crate::restart_facts(&snapshot, &journal, false).is_err());
        drop(journal);
        let after = crate::SqliteJournal::read_only_namespace(
            dir.0.path().join("journal.sqlite3"),
            "fixture",
        )
        .unwrap();
        assert_eq!(snapshot.through(), after.through());
    }
    fn table_49_effect_key() -> Vec<u8> {
        "07000000040800000002000000056c6f63616c0500000001730000000573636f706507000000000700000001090000000000000002070000000309000000000000000101060000004600000000000000050000000000000002000000220800000002000000056c6f63616c0500000001730000000573636f7065070000000000000000000000070000000000000001090000000000000007"
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
    fn open_outbox(journal: &mut crate::SqliteJournal, request: EncodedPayload) -> u64 {
        let transaction = crate::StoreTransaction::<crate::ProductTransaction>::try_new(vec![
            crate::StoreTransactionOp::OpenOutbox {
                effect: circular_runtime::EffectId::decode(&table_49_effect_key()).unwrap(),
                request,
            },
        ])
        .unwrap();
        journal
            .commit(&crate::ProductJournalCodec.encode(&transaction).unwrap())
            .unwrap()
            .sequence()
            .get()
    }
}
