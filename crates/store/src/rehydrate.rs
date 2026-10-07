
use crate::memory::{AppendBatch, AppendFailure, AppendResult, MemoryStore, PagePolicy, Store};
use crate::record::{Record, StoreSchema};
use crate::sqlite::{SqliteCommitSequence, SqliteJournalSnapshot, SqliteTransactionCodec};
use crate::transaction::{StoreTransaction, TransactionSchema};

pub trait JournalProjection<T: TransactionSchema, S: StoreSchema> {
    fn project(
        &self,
        transaction: &StoreTransaction<T>,
    ) -> Result<Vec<Record<S>>, ProjectionRejection>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectionRejection {
    pub operation: usize,
}

#[derive(Debug)]
pub enum RehydrateError<S: StoreSchema, E> {
    Decode {
        sequence: SqliteCommitSequence,
        source: E,
    },
    Append {
        sequence: SqliteCommitSequence,
        failure: Box<AppendFailure<S>>,
    },
    EmptyBatch { sequence: SqliteCommitSequence },
    Project {
        sequence: SqliteCommitSequence,
        rejection: ProjectionRejection,
    },
}

pub fn rehydrate<T, S, C, P>(
    snapshot: &SqliteJournalSnapshot,
    codec: &C,
    projection: &P,
    page_policy: PagePolicy,
) -> Result<MemoryStore<S>, RehydrateError<S, C::Error>>
where
    T: TransactionSchema,
    S: StoreSchema,
    C: SqliteTransactionCodec<T>,
    P: JournalProjection<T, S>,
{
    let mut store = MemoryStore::new(page_policy);
    let mut reader = C::ReadContext::default();
    for entry in snapshot.entries() {
        let sequence = entry.sequence();
        let transaction = codec
            .decode_in(&mut reader, sequence.get(), entry.payload())
            .map_err(|source| RehydrateError::Decode { sequence, source })?;
        let records =
            projection
                .project(&transaction)
                .map_err(|rejection| RehydrateError::Project {
                    sequence,
                    rejection,
                })?;
        if records.is_empty() {
            continue;
        }
        let batch =
            AppendBatch::try_new(records).map_err(|_| RehydrateError::EmptyBatch { sequence })?;
        match store.append(batch) {
            AppendResult::Committed(_) => {}
            AppendResult::Failed(failure) => {
                return Err(RehydrateError::Append {
                    sequence,
                    failure: Box::new(failure),
                });
            }
        }
    }
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::{JournalProjection, rehydrate};
    use crate::memory::{PagePolicy, Store};
    use crate::query::{Bound, Query, ScanStart};
    use crate::record::{Record, RecordOrigin, StoreSchema};
    use crate::sqlite::{RecoveringSqliteTransactionStore, SqliteJournal, SqliteTransactionCodec};
    use crate::transaction::{
        CheckpointOwners, RecoveryTerminal, StoreTransaction, StoreTransactionOp,
        TransactionAppend, TransactionSchema,
    };
    use circular_core::{ProducerIdentity, Sequence, Stamp, StreamIdentity, Tick};
    use std::convert::Infallible;
    use std::num::NonZeroUsize;
    use std::path::PathBuf;

    struct Dir(circular_testkit::temp::StateDir);

    impl Dir {
        fn new() -> Self {
            Self(circular_testkit::temp::StateDir::new(
                "circular-store-rehydrate",
            ))
        }

        fn database(&self) -> PathBuf {
            self.0.path().join("journal.sqlite3")
        }
    }

    #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    struct Id(u64);

    impl StreamIdentity for Id {}

    impl ProducerIdentity for Id {
        type EventProducer = Self;
        fn from_event_producer(producer: Self) -> Self {
            producer
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Txn;

    macro_rules! u64_types {
        ($($name:ident),* $(,)?) => { $(type $name = u64;)* };
    }

    impl TransactionSchema for Txn {
        type WriteContext = ();
        fn record_digest(record: &Self::Record) -> crate::transaction::RecordDigest {
            crate::transaction::RecordDigest::of_bytes(&record.to_be_bytes())
        }

        fn observation_digest(observation: &Self::Observation) -> crate::transaction::RecordDigest {
            crate::transaction::RecordDigest::of_bytes(&observation.to_be_bytes())
        }

        u64_types!(
            RecordKey,
            Record,
            EffectId,
            Outbox,
            ApprovalKey,
            Approval,
            ApprovalTicket,
            ActorId,
            Incarnation,
            CheckpointStamp,
            CheckpointState,
            Outcome,
            ObservationKey,
            Observation,
        );
    }

    fn manifest_record(run: Id) -> Record<Store3> {
        use crate::manifest::{
            FailureParams, ManifestGroups, PlacementParams, RevisionContext, RevisionStart,
            RunInputs, RunManifest, TimeParams,
        };
        use circular_core::{NonZeroTicks, TicksPerSecond, TimeSourceKind, TimeSourcePlan};

        let revision = RevisionContext::try_new(RevisionStart::Fresh(1), Vec::new())
            .expect("an empty grant has no duplicates");
        let groups = ManifestGroups::new(
            TimeParams::new(
                TicksPerSecond::new(1_000).expect("fixed resolution"),
                NonZeroTicks::new(1).expect("fixed period"),
                1,
                TimeSourcePlan::Single(TimeSourceKind::Manual),
                1,
            ),
            PlacementParams::from_validated(1),
            FailureParams::from_validated(1),
            1,
            revision,
            RunInputs::from_primary_data(1),
        );
        Record::Structure(crate::record::StructureRecord::manifest(
            Stamp::from_event_producer(
                Tick::new(0),
                Id(1),
                Sequence::FIRST,
                circular_core::RevisionEpochId::new(1).expect("first revision"),
            ),
            RunManifest::new(run, groups),
        ))
    }

    struct ToArrival;

    impl JournalProjection<Txn, Store3> for ToArrival {
        fn project(
            &self,
            transaction: &StoreTransaction<Txn>,
        ) -> Result<Vec<Record<Store3>>, super::ProjectionRejection> {
            Ok(transaction
                .operations()
                .iter()
                .filter_map(|operation| match operation {
                    StoreTransactionOp::Append(append) if *append.key() == 1 => {
                        Some(manifest_record(Id(1)))
                    }
                    StoreTransactionOp::Append(append) => {
                        let index = *append.key();
                        Some(Record::Boundary(crate::record::BoundaryRecord::arrival(
                            7,
                            Stamp::from_event_producer(
                                Tick::new(index),
                                Id(7),
                                Sequence::new(index).expect("fixture sequence"),
                                circular_core::RevisionEpochId::new(1).expect("first revision"),
                            ),
                            RecordOrigin::Actor(7),
                            crate::record::ArrivalOrigin::TimerFire { timer: index },
                            crate::ArrivalBody::Owned(circular_core::EncodedPayload::new(
                                circular_core::PayloadVersionTag::FIRST,
                                &append.record().to_be_bytes(),
                            )),
                            circular_core::ArrivalIndex::new(index),
                            Box::new([]),
                            circular_core::RecordedInstant::from_millis(index),
                            None,
                        )))
                    }
                    _ => None,
                })
                .collect())
        }
    }

    #[derive(Clone, Debug, Eq, Hash, PartialEq)]
    struct Store3;

    impl crate::manifest::ManifestSchema for Store3 {
        type Stream = Id;
        type Producer = Id;
        u64_types!(
            Placement,
            Failure,
            Versions,
            RevisionId,
            AuthoringCut,
            ScopeId,
            GrantSet,
            InputValue,
            CadencePolicy,
            TickOrigin,
        );
    }

    impl StoreSchema for Store3 {
        u64_types!(
            Incarnation,
            ActorId,
            EdgeId,
            EffectId,
            TimerId,
            ObservationKindKey,
            ObservationIdentityKey,
            ExternalOrigin,
            EffectTerm,
            EffectOutcome,
            GraphRevision,
            ObservationPayload,
        );
        type DisplayKey = u64;
        type DisplayPayload = u64;
    }

    struct Codec;

    impl SqliteTransactionCodec<Txn> for Codec {
        type ReadContext = ();
        type Error = Infallible;

        fn encode(&self, transaction: &StoreTransaction<Txn>) -> Result<Vec<u8>, Self::Error> {
            let mut output = Vec::new();
            for operation in transaction.operations() {
                if let StoreTransactionOp::Append(append) = operation {
                    output.extend_from_slice(&append.key().to_be_bytes());
                    output.extend_from_slice(&append.record().to_be_bytes());
                }
            }
            Ok(output)
        }

        fn decode(&self, bytes: &[u8]) -> Result<StoreTransaction<Txn>, Self::Error> {
            let mut operations = Vec::new();
            for chunk in bytes.chunks_exact(16) {
                let key = u64::from_be_bytes(chunk[..8].try_into().expect("eight bytes"));
                let record = u64::from_be_bytes(chunk[8..].try_into().expect("eight bytes"));
                operations.push(StoreTransactionOp::Append(TransactionAppend::new(
                    key, record,
                )));
            }
            Ok(StoreTransaction::try_new(operations).expect("not empty"))
        }
    }

    #[test]
    fn what_survives_a_reopen_answers_the_published_query_vocabulary() {
        let directory = Dir::new();
        let database = directory.database();
        drop(SqliteJournal::create(&database).expect("create the journal"));

        {
            let recovering = RecoveringSqliteTransactionStore::<Txn, Codec>::open(&database, Codec)
                .expect("open");
            let mut store = match recovering.recover(
                &CheckpointOwners::default(),
                |_, _| -> Result<RecoveryTerminal<Txn>, Infallible> {
                    unreachable!("an append-only fixture has no submitted outbox")
                },
            ) {
                Ok(result) => result.into_parts().0,
                Err(_) => panic!("an empty journal recovers"),
            };
            for key in 1..=4_u64 {
                let transaction = StoreTransaction::try_new(vec![StoreTransactionOp::Append(
                    TransactionAppend::new(key, key * 10),
                )])
                .expect("one operation");
                store.commit_group(vec![transaction]).expect("commit");
            }
        }

        let mut reopened = SqliteJournal::open(&database).expect("reopen");
        let snapshot = reopened.snapshot().expect("snapshot");
        assert_eq!(
            snapshot.entries().len(),
            4,
            "what was written stayed on the medium"
        );

        let page_size = NonZeroUsize::new(16).expect("the constant is not zero");
        let mut store = rehydrate::<Txn, Store3, Codec, ToArrival>(
            &snapshot,
            &Codec,
            &ToArrival,
            PagePolicy::new(page_size, page_size).expect("the same value is valid"),
        )
        .expect("restore");
        store.seal_all();

        let page = store
            .query(&Query::ArrivalScan {
                actor: 7,
                start: ScanStart::Beginning,
                upto: Bound::EndOfSealed,
            })
            .expect("arrival lookup");
        let payloads = page
            .records()
            .iter()
            .map(|record| match record {
                Record::Boundary(boundary) => {
                    let crate::record::BoundaryFact::Arrival { body, .. } = boundary.fact() else {
                        panic!("fixture only contains arrivals");
                    };
                    let payload = body.payload().expect("owned body");
                    u64::from_be_bytes(payload.body().try_into().expect("eight bytes"))
                }
                _ => panic!("only Boundary was mapped"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            payloads,
            vec![20, 30, 40],
            "commit order is append order; the restored surface ordinals mean the same as the original"
        );
    }
}
