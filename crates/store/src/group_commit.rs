
use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::time::{Duration, Instant};

use crate::sqlite::SqliteGroupCommitCandidate;
use crate::transaction::{
    GroupCommitFailure, GroupTransactionPort, StoreTransaction, StoreTransactionReceipt,
    StoreTransactionReject, TransactionSchema,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GroupCommitPolicy {
    max_batch_size: NonZeroUsize,
    max_wait: Duration,
}

impl GroupCommitPolicy {
    #[must_use]
    pub const fn new(max_batch_size: NonZeroUsize, max_wait: Duration) -> Self {
        Self {
            max_batch_size,
            max_wait,
        }
    }

    #[must_use]
    pub const fn max_batch_size(self) -> NonZeroUsize {
        self.max_batch_size
    }

    #[must_use]
    pub const fn max_wait(self) -> Duration {
        self.max_wait
    }
}

impl From<SqliteGroupCommitCandidate> for GroupCommitPolicy {
    fn from(candidate: SqliteGroupCommitCandidate) -> Self {
        Self::new(candidate.max_batch_size(), candidate.max_wait())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcceptedValueProvenance {
    decision: &'static str,
    evidence: &'static str,
    measured_identity: &'static str,
    surface: &'static str,
}

impl AcceptedValueProvenance {
    #[must_use]
    pub const fn decision(self) -> &'static str {
        self.decision
    }

    #[must_use]
    pub const fn evidence(self) -> &'static str {
        self.evidence
    }

    #[must_use]
    pub const fn measured_identity(self) -> &'static str {
        self.measured_identity
    }

    #[must_use]
    pub const fn surface(self) -> &'static str {
        self.surface
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AcceptedGroupCommit {
    max_batch_size: NonZeroUsize,
    max_wait_millis: u32,
    provenance: AcceptedValueProvenance,
}

impl AcceptedGroupCommit {
    #[must_use]
    pub const fn max_batch_size(self) -> NonZeroUsize {
        self.max_batch_size
    }

    #[must_use]
    pub const fn max_wait_millis(self) -> u32 {
        self.max_wait_millis
    }

    #[must_use]
    pub const fn provenance(self) -> AcceptedValueProvenance {
        self.provenance
    }

    #[must_use]
    pub const fn candidate(self) -> SqliteGroupCommitCandidate {
        SqliteGroupCommitCandidate::new(
            self.max_batch_size,
            Duration::from_millis(self.max_wait_millis as u64),
        )
    }
}

pub const ACCEPTED_GROUP_COMMIT: AcceptedGroupCommit = AcceptedGroupCommit {
    max_batch_size: match NonZeroUsize::new(32) {
        Some(value) => value,
        None => unreachable!(),
    },
    max_wait_millis: 2,
    provenance: AcceptedValueProvenance {
        decision: "store-apply group-commit tuning, accepted 2026-08-18",
        evidence: "group-commit coordinator measurement",
        measured_identity: "coordinator-group-commit-356-v1",
        surface: "circular_store::GroupCommitCoordinator",
    },
};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SubmissionId(u64);

impl SubmissionId {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug)]
pub enum SubmissionOutcome<S: TransactionSchema, E> {
    Committed {
        receipt: StoreTransactionReceipt<S>,
        commit: u64,
    },
    Rejected {
        operation: usize,
        reason: StoreTransactionReject<S>,
    },
    Codec(E),
}

#[derive(Debug)]
pub struct SettledSubmission<S: TransactionSchema, E> {
    id: SubmissionId,
    outcome: SubmissionOutcome<S, E>,
}

impl<S: TransactionSchema, E> SettledSubmission<S, E> {
    #[must_use]
    pub const fn id(&self) -> SubmissionId {
        self.id
    }

    #[must_use]
    pub const fn outcome(&self) -> &SubmissionOutcome<S, E> {
        &self.outcome
    }

    #[must_use]
    pub fn into_parts(self) -> (SubmissionId, SubmissionOutcome<S, E>) {
        (self.id, self.outcome)
    }
}

pub type UnsettledSubmissions<S> = Box<[(SubmissionId, StoreTransaction<S>)]>;

pub type FlushResult<S, CE, BE> =
    Result<Vec<SettledSubmission<S, CE>>, GroupCommitBackendFailure<S, CE, BE>>;

pub type BackendFailureParts<S, CE, BE> =
    (Box<[SettledSubmission<S, CE>]>, UnsettledSubmissions<S>, BE);

#[derive(Debug)]
pub struct GroupCommitBackendFailure<S: TransactionSchema, CE, BE> {
    settled: Box<[SettledSubmission<S, CE>]>,
    unsettled: UnsettledSubmissions<S>,
    source: BE,
}

impl<S: TransactionSchema, CE, BE> GroupCommitBackendFailure<S, CE, BE> {
    #[must_use]
    pub const fn settled(&self) -> &[SettledSubmission<S, CE>] {
        &self.settled
    }

    #[must_use]
    pub const fn unsettled(&self) -> &[(SubmissionId, StoreTransaction<S>)] {
        &self.unsettled
    }

    #[must_use]
    pub const fn source(&self) -> &BE {
        &self.source
    }

    #[must_use]
    pub fn into_parts(self) -> BackendFailureParts<S, CE, BE> {
        (self.settled, self.unsettled, self.source)
    }
}

struct QueuedSubmission<S: TransactionSchema> {
    id: SubmissionId,
    at: Instant,
    transaction: StoreTransaction<S>,
}

pub struct GroupCommitCoordinator<S, P>
where
    S: TransactionSchema,
    P: GroupTransactionPort<S>,
{
    store: P,
    policy: GroupCommitPolicy,
    queue: VecDeque<QueuedSubmission<S>>,
    next_id: u64,
}

impl<S, P> GroupCommitCoordinator<S, P>
where
    S: TransactionSchema,
    P: GroupTransactionPort<S>,
{
    #[must_use]
    pub const fn new(store: P, policy: GroupCommitPolicy) -> Self {
        Self {
            store,
            policy,
            queue: VecDeque::new(),
            next_id: 1,
        }
    }

    #[must_use]
    pub const fn policy(&self) -> GroupCommitPolicy {
        self.policy
    }

    #[must_use]
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    #[must_use]
    pub const fn store(&self) -> &P {
        &self.store
    }

    pub fn into_parts(mut self) -> (P, UnsettledSubmissions<S>) {
        let pending = self
            .queue
            .drain(..)
            .map(|queued| (queued.id, queued.transaction))
            .collect();
        (self.store, pending)
    }

    pub fn submit(&mut self, at: Instant, transaction: StoreTransaction<S>) -> SubmissionId {
        let id = SubmissionId(self.next_id);
        self.next_id += 1;
        self.queue.push_back(QueuedSubmission {
            id,
            at,
            transaction,
        });
        id
    }

    #[must_use]
    pub fn deadline(&self) -> Option<Instant> {
        self.queue
            .front()
            .and_then(|queued| queued.at.checked_add(self.policy.max_wait()))
    }

    #[must_use]
    pub fn is_ready(&self, now: Instant) -> bool {
        if self.queue.is_empty() {
            return false;
        }
        if self.queue.len() >= self.policy.max_batch_size().get() {
            return true;
        }
        self.deadline().is_some_and(|deadline| now >= deadline)
    }

    pub fn flush(&mut self) -> FlushResult<S, P::CodecError, P::BackendError> {
        self.flush_stopping_on(|_| false)
    }

    /// Stop before retrying any survivors when a dependency-producing submission fails.
    /// The caller must end recording on that failure; the remaining queue is uncommitted.
    pub fn flush_stopping_on(
        &mut self,
        stop: impl Fn(SubmissionId) -> bool,
    ) -> FlushResult<S, P::CodecError, P::BackendError> {
        let take = self.queue.len().min(self.policy.max_batch_size().get());
        let mut pending = self.queue.drain(..take).collect::<Vec<_>>();
        let mut settled = Vec::with_capacity(take);

        while !pending.is_empty() {
            let transactions = pending
                .iter()
                .map(|queued| queued.transaction.clone())
                .collect::<Vec<_>>();
            match self.store.commit_group(transactions) {
                Ok(receipts) => {
                    debug_assert_eq!(receipts.len(), pending.len());
                    for (queued, committed) in pending.drain(..).zip(receipts.into_vec()) {
                        let (receipt, commit) = committed.into_parts();
                        settled.push(SettledSubmission {
                            id: queued.id,
                            outcome: SubmissionOutcome::Committed { receipt, commit },
                        });
                    }
                }
                Err(failure) => match failure {
                    GroupCommitFailure::Rejected {
                        transaction,
                        operation,
                        reason,
                    } => {
                        let queued = pending.remove(transaction);
                        settled.push(SettledSubmission {
                            id: queued.id,
                            outcome: SubmissionOutcome::Rejected { operation, reason },
                        });
                        if stop(queued.id) {
                            for queued in pending.into_iter().rev() {
                                self.queue.push_front(queued);
                            }
                            settled.sort_by_key(SettledSubmission::id);
                            return Ok(settled);
                        }
                    }
                    GroupCommitFailure::Codec {
                        transaction,
                        source,
                    } => {
                        let queued = pending.remove(transaction);
                        settled.push(SettledSubmission {
                            id: queued.id,
                            outcome: SubmissionOutcome::Codec(source),
                        });
                        if stop(queued.id) {
                            for queued in pending.into_iter().rev() {
                                self.queue.push_front(queued);
                            }
                            settled.sort_by_key(SettledSubmission::id);
                            return Ok(settled);
                        }
                    }
                    GroupCommitFailure::Backend(source) => {
                        settled.sort_by_key(SettledSubmission::id);
                        return Err(GroupCommitBackendFailure {
                            settled: settled.into_boxed_slice(),
                            unsettled: pending
                                .into_iter()
                                .map(|queued| (queued.id, queued.transaction))
                                .collect(),
                            source,
                        });
                    }
                    GroupCommitFailure::Contended => {
                        for queued in pending.into_iter().rev() {
                            self.queue.push_front(queued);
                        }
                        settled.sort_by_key(SettledSubmission::id);
                        return Ok(settled);
                    }
                },
            }
        }

        settled.sort_by_key(SettledSubmission::id);
        Ok(settled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite::{
        RecoveredSqliteTransactionStore, RecoveringSqliteTransactionStore, SqliteTransactionCodec,
    };
    use crate::transaction::{
        AtomicStoreTransactionPort, CheckpointOwners, RecoveredTransactionModel, RecoveryTerminal,
        StoreTransactionFailure, StoreTransactionFailureReason, StoreTransactionOp,
        TransactionAppend,
    };
    use std::convert::Infallible;
    use std::num::NonZeroUsize;
    use std::path::PathBuf;
    use std::time::Duration;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct TestSchema;

    impl TransactionSchema for TestSchema {
        type WriteContext = ();
        fn record_digest(record: &Self::Record) -> crate::transaction::RecordDigest {
            crate::transaction::RecordDigest::of_bytes(&record.to_be_bytes())
        }

        fn observation_digest(observation: &Self::Observation) -> crate::transaction::RecordDigest {
            crate::transaction::RecordDigest::of_bytes(&observation.to_be_bytes())
        }

        type RecordKey = u64;
        type Record = u64;
        type EffectId = u64;
        type Outbox = u64;
        type ApprovalKey = u64;
        type Approval = u64;
        type ApprovalTicket = u64;
        type ActorId = u64;
        type Incarnation = u64;
        type CheckpointStamp = u64;
        type CheckpointState = u64;
        type Outcome = u64;
        type ObservationKey = u64;
        type Observation = u64;
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum ScriptedBackend {
        Ambiguous,
    }

    enum ScriptedGroupOutcome {
        Commit,
        Reject {
            transaction: usize,
            operation: usize,
            reason: StoreTransactionReject<TestSchema>,
        },
        Backend(ScriptedBackend),
    }

    struct ScriptedGroupPort {
        model: RecoveredTransactionModel<TestSchema>,
        outcomes: VecDeque<ScriptedGroupOutcome>,
        group_sizes: Vec<usize>,
        committed: u64,
    }

    impl ScriptedGroupPort {
        fn new(outcomes: impl IntoIterator<Item = ScriptedGroupOutcome>) -> Self {
            Self {
                model: RecoveredTransactionModel::empty_for_journal_replay(),
                outcomes: outcomes.into_iter().collect(),
                group_sizes: Vec::new(),
                committed: 0,
            }
        }
    }

    impl AtomicStoreTransactionPort<TestSchema> for ScriptedGroupPort {
        type BackendError = ScriptedBackend;

        fn commit(
            &mut self,
            transaction: StoreTransaction<TestSchema>,
        ) -> Result<
            StoreTransactionReceipt<TestSchema>,
            StoreTransactionFailure<TestSchema, Self::BackendError>,
        > {
            self.model
                .commit(transaction)
                .map_err(|failure| failure.map_backend(|never: Infallible| match never {}))
        }
    }

    impl GroupTransactionPort<TestSchema> for ScriptedGroupPort {
        type CodecError = Infallible;

        fn commit_group(
            &mut self,
            transactions: Vec<StoreTransaction<TestSchema>>,
        ) -> Result<
            Box<[crate::transaction::CommittedTransaction<TestSchema>]>,
            GroupCommitFailure<TestSchema, Self::CodecError, Self::BackendError>,
        > {
            self.group_sizes.push(transactions.len());
            match self.outcomes.pop_front() {
                Some(ScriptedGroupOutcome::Reject {
                    transaction,
                    operation,
                    reason,
                }) => {
                    return Err(GroupCommitFailure::Rejected {
                        transaction,
                        operation,
                        reason,
                    });
                }
                Some(ScriptedGroupOutcome::Backend(source)) => {
                    return Err(GroupCommitFailure::Backend(source));
                }
                Some(ScriptedGroupOutcome::Commit) | None => {}
            }

            let mut candidate = self.model.clone();
            let mut receipts = Vec::with_capacity(transactions.len());
            for (transaction, value) in transactions.into_iter().enumerate() {
                match candidate.commit(value) {
                    Ok(receipt) => {
                        self.committed += 1;
                        receipts.push(crate::transaction::CommittedTransaction::new(
                            receipt,
                            self.committed,
                        ));
                    }
                    Err(failure) => {
                        let (_, reason) = failure.into_parts();
                        let (operation, reason) = match reason {
                            StoreTransactionFailureReason::Rejected { operation, reason } => {
                                (operation, reason)
                            }
                            StoreTransactionFailureReason::Backend(never) => match never {},
                        };
                        return Err(GroupCommitFailure::Rejected {
                            transaction,
                            operation,
                            reason,
                        });
                    }
                }
            }
            self.model = candidate;
            Ok(receipts.into_boxed_slice())
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TestCodecError {
        Refused(u64),
        Unsupported,
        Truncated,
        Trailing,
    }

    #[derive(Clone, Copy, Debug)]
    struct TestCodec {
        refuse_record: Option<u64>,
    }

    impl TestCodec {
        const fn plain() -> Self {
            Self {
                refuse_record: None,
            }
        }

        const fn refusing(record: u64) -> Self {
            Self {
                refuse_record: Some(record),
            }
        }
    }

    impl SqliteTransactionCodec<TestSchema> for TestCodec {
        type ReadContext = ();
        type Error = TestCodecError;

        fn encode(
            &self,
            transaction: &StoreTransaction<TestSchema>,
        ) -> Result<Vec<u8>, Self::Error> {
            let mut output = Vec::new();
            output.extend_from_slice(&(transaction.operations().len() as u64).to_be_bytes());
            for operation in transaction.operations() {
                match operation {
                    StoreTransactionOp::Append(append) => {
                        if self.refuse_record == Some(*append.record()) {
                            return Err(TestCodecError::Refused(*append.record()));
                        }
                        output.push(1);
                        output.extend_from_slice(&append.key().to_be_bytes());
                        output.extend_from_slice(&append.record().to_be_bytes());
                    }
                    StoreTransactionOp::OpenOutbox { effect, request } => {
                        output.push(2);
                        output.extend_from_slice(&effect.to_be_bytes());
                        output.extend_from_slice(&request.to_be_bytes());
                    }
                    _ => return Err(TestCodecError::Unsupported),
                }
            }
            Ok(output)
        }

        fn decode(&self, bytes: &[u8]) -> Result<StoreTransaction<TestSchema>, Self::Error> {
            let count = u64::from_be_bytes(
                bytes
                    .get(..8)
                    .ok_or(TestCodecError::Truncated)?
                    .try_into()
                    .expect("eight bytes were checked"),
            );
            let mut at = 8_usize;
            let mut operations = Vec::new();
            for _ in 0..count {
                let tag = *bytes.get(at).ok_or(TestCodecError::Truncated)?;
                let key = u64::from_be_bytes(
                    bytes
                        .get(at + 1..at + 9)
                        .ok_or(TestCodecError::Truncated)?
                        .try_into()
                        .expect("eight bytes were checked"),
                );
                let value = u64::from_be_bytes(
                    bytes
                        .get(at + 9..at + 17)
                        .ok_or(TestCodecError::Truncated)?
                        .try_into()
                        .expect("eight bytes were checked"),
                );
                at += 17;
                operations.push(match tag {
                    1 => StoreTransactionOp::Append(TransactionAppend::new(key, value)),
                    2 => StoreTransactionOp::OpenOutbox {
                        effect: key,
                        request: value,
                    },
                    _ => return Err(TestCodecError::Unsupported),
                });
            }
            if at != bytes.len() {
                return Err(TestCodecError::Trailing);
            }
            StoreTransaction::try_new(operations).map_err(|_| TestCodecError::Truncated)
        }
    }

    struct TestDirectory(circular_testkit::temp::StateDir);

    impl TestDirectory {
        fn new() -> Self {
            Self(circular_testkit::temp::StateDir::new(
                "circular-store-group-commit",
            ))
        }

        fn database(&self) -> PathBuf {
            self.0.path().join("store.sqlite3")
        }
    }

    fn coordinator(
        directory: &TestDirectory,
        codec: TestCodec,
        batch: usize,
        wait: Duration,
    ) -> GroupCommitCoordinator<TestSchema, RecoveredSqliteTransactionStore<TestSchema, TestCodec>>
    {
        let recovering = RecoveringSqliteTransactionStore::create(directory.database(), codec)
            .expect("create the group-commit fixture");
        let store = match recovering.recover(
            &CheckpointOwners::default(),
            |_, _| -> Result<RecoveryTerminal<TestSchema>, Infallible> {
                unreachable!("the append-only fixture has no submitted outbox")
            },
        ) {
            Ok(result) => result.into_parts().0,
            Err(_) => panic!("the append-only fixture must recover"),
        };
        GroupCommitCoordinator::new(
            store,
            SqliteGroupCommitCandidate::new(
                NonZeroUsize::new(batch).expect("fixture batch is nonzero"),
                wait,
            )
            .into(),
        )
    }

    fn append(key: u64, record: u64) -> StoreTransaction<TestSchema> {
        StoreTransaction::try_new(vec![StoreTransactionOp::Append(TransactionAppend::new(
            key, record,
        ))])
        .expect("fixture transactions are nonempty")
    }

    fn open_outbox(effect: u64, request: u64) -> StoreTransaction<TestSchema> {
        StoreTransaction::try_new(vec![StoreTransactionOp::OpenOutbox { effect, request }])
            .expect("fixture transactions are nonempty")
    }

    fn ids<E>(settled: &[SettledSubmission<TestSchema, E>]) -> Vec<u64> {
        settled
            .iter()
            .map(|item| item.id().get())
            .collect::<Vec<_>>()
    }

    #[test]
    fn coordinator_policy_and_rebatch_are_backend_free() {
        let port = ScriptedGroupPort::new([
            ScriptedGroupOutcome::Reject {
                transaction: 1,
                operation: 0,
                reason: StoreTransactionReject::AppendConflict { key: 2 },
            },
            ScriptedGroupOutcome::Commit,
            ScriptedGroupOutcome::Backend(ScriptedBackend::Ambiguous),
        ]);
        let policy = GroupCommitPolicy::new(
            NonZeroUsize::new(3).expect("the policy batch is not zero"),
            Duration::from_millis(5),
        );
        let mut coordinator = GroupCommitCoordinator::new(port, policy);
        let now = Instant::now();

        assert!(!coordinator.is_ready(now));
        let first = coordinator.submit(now, append(1, 10));
        coordinator.submit(now, append(2, 20));
        assert!(!coordinator.is_ready(now));
        let third = coordinator.submit(now, append(3, 30));
        assert!(coordinator.is_ready(now));

        let settled = coordinator.flush().expect("scripted backend success");
        assert_eq!(ids(&settled), vec![first.get(), 2, third.get()]);
        assert!(matches!(
            settled[1].outcome(),
            SubmissionOutcome::Rejected {
                operation: 0,
                reason: StoreTransactionReject::AppendConflict { key: 2 },
            }
        ));
        assert!(matches!(
            settled[0].outcome(),
            SubmissionOutcome::Committed { .. }
        ));
        assert!(matches!(
            settled[2].outcome(),
            SubmissionOutcome::Committed { .. }
        ));
        assert_eq!(coordinator.store().group_sizes, vec![3, 2]);

        let fourth = coordinator.submit(now, append(4, 40));
        let fifth = coordinator.submit(now, append(5, 50));
        let failure = coordinator.flush().expect_err("scripted backend ambiguity");
        assert!(failure.settled().is_empty());
        assert_eq!(
            failure
                .unsettled()
                .iter()
                .map(|(id, _)| id.get())
                .collect::<Vec<_>>(),
            vec![fourth.get(), fifth.get()]
        );
        assert_eq!(failure.source(), &ScriptedBackend::Ambiguous);
    }

    #[test]
    fn one_submitters_rejection_does_not_fail_the_rest_of_the_batch() {
        let directory = TestDirectory::new();
        let mut coordinator = coordinator(&directory, TestCodec::plain(), 8, Duration::ZERO);
        let now = Instant::now();

        let first = coordinator.submit(now, open_outbox(1, 100));
        let second = coordinator.submit(now, append(2, 20));
        let conflicting = coordinator.submit(now, open_outbox(1, 200));
        let fourth = coordinator.submit(now, append(3, 30));

        let settled = coordinator.flush().expect("no backend failure");

        assert_eq!(
            ids(&settled),
            vec![first.get(), second.get(), conflicting.get(), fourth.get()],
            "outcomes are in submission order, exactly one per batched submission"
        );
        assert!(matches!(
            settled[2].outcome(),
            SubmissionOutcome::Rejected { .. }
        ));
        for index in [0, 1, 3] {
            assert!(
                matches!(
                    settled[index].outcome(),
                    SubmissionOutcome::Committed { .. }
                ),
                "submission {index} failed because of someone else's refusal"
            );
        }

        let (mut store, pending) = coordinator.into_parts();
        assert!(pending.is_empty());
        assert_eq!(
            store.model().record_digest(&2),
            Some(TestSchema::record_digest(&20))
        );
        assert_eq!(
            store.model().record_digest(&3),
            Some(TestSchema::record_digest(&30))
        );
        let snapshot = store.journal_snapshot().expect("durable prefix");
        assert_eq!(snapshot.entries().len(), 3);
    }

    #[test]
    fn one_submitters_codec_failure_does_not_fail_the_rest_of_the_batch() {
        let directory = TestDirectory::new();
        let mut coordinator = coordinator(&directory, TestCodec::refusing(999), 8, Duration::ZERO);
        let now = Instant::now();

        let first = coordinator.submit(now, append(1, 10));
        let refused = coordinator.submit(now, append(2, 999));
        let third = coordinator.submit(now, append(3, 30));

        let settled = coordinator.flush().expect("no backend failure");

        assert_eq!(ids(&settled), vec![first.get(), refused.get(), third.get()]);
        assert!(matches!(
            settled[1].outcome(),
            SubmissionOutcome::Codec(TestCodecError::Refused(999))
        ));
        assert!(matches!(
            settled[0].outcome(),
            SubmissionOutcome::Committed { .. }
        ));
        assert!(matches!(
            settled[2].outcome(),
            SubmissionOutcome::Committed { .. }
        ));

        let (store, _) = coordinator.into_parts();
        assert_eq!(store.model().record_digest(&2), None);
    }

    #[test]
    fn every_rejection_is_peeled_until_the_remaining_group_commits() {
        let directory = TestDirectory::new();
        let mut coordinator = coordinator(&directory, TestCodec::refusing(777), 8, Duration::ZERO);
        let now = Instant::now();

        coordinator.submit(now, open_outbox(1, 100));
        coordinator.submit(now, open_outbox(1, 200));
        coordinator.submit(now, append(2, 777));
        coordinator.submit(now, open_outbox(1, 300));
        coordinator.submit(now, append(3, 30));

        let settled = coordinator.flush().expect("no backend failure");
        assert_eq!(ids(&settled), vec![1, 2, 3, 4, 5]);
        let committed = settled
            .iter()
            .filter(|item| matches!(item.outcome(), SubmissionOutcome::Committed { .. }))
            .count();
        assert_eq!(
            committed, 2,
            "only the two that did not conflict are confirmed"
        );
        assert!(matches!(
            settled[2].outcome(),
            SubmissionOutcome::Codec(TestCodecError::Refused(777))
        ));
    }

    #[test]
    fn batching_does_not_change_which_submissions_commit() {
        let inputs = || {
            vec![
                open_outbox(1, 100),
                append(2, 20),
                open_outbox(1, 200),
                append(3, 30),
            ]
        };

        let one_at_a_time = TestDirectory::new();
        let mut single = coordinator(&one_at_a_time, TestCodec::plain(), 1, Duration::ZERO);
        let mut single_outcomes = Vec::new();
        for transaction in inputs() {
            single.submit(Instant::now(), transaction);
            let settled = single.flush().expect("no backend failure");
            assert_eq!(settled.len(), 1);
            single_outcomes.push(matches!(
                settled[0].outcome(),
                SubmissionOutcome::Committed { .. }
            ));
        }

        let batched_directory = TestDirectory::new();
        let mut batched = coordinator(&batched_directory, TestCodec::plain(), 8, Duration::ZERO);
        let now = Instant::now();
        for transaction in inputs() {
            batched.submit(now, transaction);
        }
        let batched_outcomes = batched
            .flush()
            .expect("no backend failure")
            .iter()
            .map(|item| matches!(item.outcome(), SubmissionOutcome::Committed { .. }))
            .collect::<Vec<_>>();

        assert_eq!(single_outcomes, batched_outcomes);

        let (mut single_store, _) = single.into_parts();
        let (mut batched_store, _) = batched.into_parts();
        assert_eq!(
            single_store.model().record_digest(&2),
            batched_store.model().record_digest(&2)
        );
        assert_eq!(
            single_store
                .journal_snapshot()
                .expect("single prefix")
                .entries()
                .len(),
            batched_store
                .journal_snapshot()
                .expect("batched prefix")
                .entries()
                .len()
        );
    }

    #[test]
    fn the_injected_candidate_alone_decides_readiness() {
        let directory = TestDirectory::new();
        let mut coordinator =
            coordinator(&directory, TestCodec::plain(), 3, Duration::from_millis(5));
        let now = Instant::now();

        assert!(
            !coordinator.is_ready(now),
            "an empty queue does not require a flush"
        );
        assert_eq!(coordinator.deadline(), None);

        coordinator.submit(now, append(1, 10));
        assert!(
            !coordinator.is_ready(now),
            "the window remains and the batch is not full"
        );
        assert_eq!(
            coordinator.deadline(),
            now.checked_add(Duration::from_millis(5))
        );
        assert!(
            coordinator.is_ready(now + Duration::from_millis(5)),
            "once the window passes, a flush is required even if the batch is not full"
        );

        coordinator.submit(now + Duration::from_millis(1), append(2, 20));
        assert!(!coordinator.is_ready(now));
        coordinator.submit(now + Duration::from_millis(2), append(3, 30));
        assert!(
            coordinator.is_ready(now),
            "a full batch does not wait for the window"
        );
        assert_eq!(
            coordinator.deadline(),
            now.checked_add(Duration::from_millis(5)),
            "the deadline belongs to the submission that waited longest"
        );
    }

    #[test]
    fn a_zero_window_candidate_is_ready_with_one_submission() {
        let directory = TestDirectory::new();
        let mut coordinator = coordinator(&directory, TestCodec::plain(), 32, Duration::ZERO);
        let now = Instant::now();
        assert!(!coordinator.is_ready(now));
        coordinator.submit(now, append(1, 10));
        assert!(coordinator.is_ready(now));
    }

    #[test]
    fn one_flush_settles_at_most_the_candidate_batch_size() {
        let directory = TestDirectory::new();
        let mut coordinator = coordinator(&directory, TestCodec::plain(), 2, Duration::ZERO);
        let now = Instant::now();
        for key in 1..=5_u64 {
            coordinator.submit(now, append(key, key * 10));
        }
        assert_eq!(coordinator.queued(), 5);

        let settled = coordinator.flush().expect("no backend failure");
        assert_eq!(ids(&settled), vec![1, 2]);
        assert_eq!(coordinator.queued(), 3);

        let settled = coordinator.flush().expect("no backend failure");
        assert_eq!(ids(&settled), vec![3, 4]);
        assert_eq!(coordinator.queued(), 1);
    }

    #[test]
    fn flushing_an_empty_queue_writes_nothing() {
        let directory = TestDirectory::new();
        let mut coordinator = coordinator(&directory, TestCodec::plain(), 4, Duration::ZERO);
        assert!(coordinator.flush().expect("no backend failure").is_empty());
        let (mut store, pending) = coordinator.into_parts();
        assert!(pending.is_empty());
        assert!(
            store
                .journal_snapshot()
                .expect("empty prefix")
                .entries()
                .is_empty()
        );
    }

    #[test]
    fn dismantling_the_coordinator_returns_unflushed_submissions_without_an_outcome() {
        let directory = TestDirectory::new();
        let mut coordinator = coordinator(&directory, TestCodec::plain(), 8, Duration::ZERO);
        let now = Instant::now();
        let first = coordinator.submit(now, append(1, 10));
        let second = coordinator.submit(now, append(2, 20));

        let (mut store, pending) = coordinator.into_parts();
        assert_eq!(
            pending.iter().map(|(id, _)| id.get()).collect::<Vec<_>>(),
            vec![first.get(), second.get()]
        );
        assert!(
            store
                .journal_snapshot()
                .expect("empty prefix")
                .entries()
                .is_empty(),
            "submission alone makes nothing durable"
        );
    }

    #[test]
    fn a_full_batch_is_ready_before_its_window_elapses() {
        let directory = TestDirectory::new();
        let accepted = ACCEPTED_GROUP_COMMIT.candidate();
        let mut coordinator = coordinator(
            &directory,
            TestCodec::plain(),
            accepted.max_batch_size().get(),
            accepted.max_wait(),
        );
        let now = Instant::now();

        for index in 0..31 {
            coordinator.submit(now, append(index, index * 10));
        }
        assert!(
            !coordinator.is_ready(now),
            "31 entries do not fill the batch and the window has not passed"
        );

        coordinator.submit(now, append(31, 310));
        assert!(
            coordinator.is_ready(now),
            "the moment the 32nd fills the batch it is ready, regardless of the window"
        );
    }

    #[test]
    fn one_short_of_the_batch_waits_for_the_exact_window() {
        let directory = TestDirectory::new();
        let accepted = ACCEPTED_GROUP_COMMIT.candidate();
        let mut coordinator = coordinator(
            &directory,
            TestCodec::plain(),
            accepted.max_batch_size().get(),
            accepted.max_wait(),
        );
        let now = Instant::now();

        for index in 0..31 {
            coordinator.submit(now, append(index, index * 10));
        }

        let deadline = coordinator.deadline().expect("the queue is not empty");
        assert_eq!(
            deadline.duration_since(now),
            accepted.max_wait(),
            "the deadline is one window after the submission that waited longest"
        );
        assert!(
            !coordinator.is_ready(deadline - Duration::from_nanos(1)),
            "not yet just before the window"
        );
        assert!(
            coordinator.is_ready(deadline),
            "ready the moment the window is reached"
        );
    }
}
