
use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::fmt::Debug;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RecordDigest([u8; 32]);

impl RecordDigest {
    #[must_use]
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(
            ring::digest::digest(&ring::digest::SHA256, bytes)
                .as_ref()
                .try_into()
                .expect("SHA-256 has 32 bytes"),
        )
    }
}

pub trait TransactionSchema: Clone + Debug + Eq {
    /// Process-local encoder material carried by the submitting producer.
    type WriteContext: Clone + Debug;
    fn record_digest(record: &Self::Record) -> RecordDigest;

    fn observation_digest(observation: &Self::Observation) -> RecordDigest;

    type RecordKey: Clone + Debug + Eq + Ord;
    type Record: Clone + Debug + Eq;
    type EffectId: Clone + Debug + Eq + Ord;
    type Outbox: Clone + Debug + Eq;
    type ApprovalKey: Clone + Debug + Eq + Ord;
    type Approval: Clone + Debug + Eq;
    type ApprovalTicket: Clone + Debug + Eq;
    type ActorId: Clone + Debug + Eq + Ord;
    type Incarnation: Clone + Debug + Eq;
    type CheckpointStamp: Clone + Debug + Eq + Ord;
    type CheckpointState: Clone + Debug + Eq;
    type Outcome: Clone + Debug + Eq;
    type ObservationKey: Clone + Debug + Eq + Ord;
    type Observation: Clone + Debug + Eq;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionAppend<S: TransactionSchema> {
    key: S::RecordKey,
    record: S::Record,
}

impl<S: TransactionSchema> TransactionAppend<S> {
    #[must_use]
    pub const fn new(key: S::RecordKey, record: S::Record) -> Self {
        Self { key, record }
    }

    #[must_use]
    pub const fn key(&self) -> &S::RecordKey {
        &self.key
    }

    #[must_use]
    pub const fn record(&self) -> &S::Record {
        &self.record
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionObservation<S: TransactionSchema> {
    key: S::ObservationKey,
    value: S::Observation,
}

impl<S: TransactionSchema> TransactionObservation<S> {
    #[must_use]
    pub const fn new(key: S::ObservationKey, value: S::Observation) -> Self {
        Self { key, value }
    }

    #[must_use]
    pub const fn key(&self) -> &S::ObservationKey {
        &self.key
    }

    #[must_use]
    pub const fn value(&self) -> &S::Observation {
        &self.value
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionCheckpoint<S: TransactionSchema> {
    actor: S::ActorId,
    owner: S::Incarnation,
    at: S::CheckpointStamp,
    state: S::CheckpointState,
    pending: BTreeSet<S::EffectId>,
}

impl<S: TransactionSchema> TransactionCheckpoint<S> {
    #[must_use]
    pub fn new(
        actor: S::ActorId,
        owner: S::Incarnation,
        at: S::CheckpointStamp,
        state: S::CheckpointState,
        pending: impl IntoIterator<Item = S::EffectId>,
    ) -> Self {
        Self {
            actor,
            owner,
            at,
            state,
            pending: pending.into_iter().collect(),
        }
    }

    #[must_use]
    pub const fn actor(&self) -> &S::ActorId {
        &self.actor
    }

    #[must_use]
    pub const fn owner(&self) -> &S::Incarnation {
        &self.owner
    }

    #[must_use]
    pub const fn at(&self) -> &S::CheckpointStamp {
        &self.at
    }

    #[must_use]
    pub const fn state(&self) -> &S::CheckpointState {
        &self.state
    }

    #[must_use]
    pub const fn pending(&self) -> &BTreeSet<S::EffectId> {
        &self.pending
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransactionOutboxPhase {
    Committed,
    Submitted,
}

/// The only two states that can remain as an in-progress approval row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionApprovalPhase<T> {
    Requested,
    Approved(T),
}

/// Store-owned approval custody. Terminal decisions are represented only by
/// the atomic settlement observation and never by another live-row variant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionApproval<S: TransactionSchema> {
    request: S::Approval,
    phase: TransactionApprovalPhase<S::ApprovalTicket>,
}

impl<S: TransactionSchema> TransactionApproval<S> {
    #[must_use]
    pub const fn request(&self) -> &S::Approval {
        &self.request
    }

    #[must_use]
    pub const fn phase(&self) -> &TransactionApprovalPhase<S::ApprovalTicket> {
        &self.phase
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionOutbox<S: TransactionSchema> {
    request: S::Outbox,
    phase: TransactionOutboxPhase,
}

impl<S: TransactionSchema> TransactionOutbox<S> {
    #[must_use]
    pub const fn request(&self) -> &S::Outbox {
        &self.request
    }

    #[must_use]
    pub const fn phase(&self) -> TransactionOutboxPhase {
        self.phase
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreTransactionOp<S: TransactionSchema> {
    Append(TransactionAppend<S>),
    /// Append one immutable Observation that is not a custody settlement.
    ///
    /// Lifecycle, diagnostic, and accounting owners use this arm. Exact key
    /// retries are idempotent; the same key with a different value conflicts.
    AppendObservation(TransactionObservation<S>),
    OpenOutbox {
        effect: S::EffectId,
        request: S::Outbox,
    },
    SubmitOutbox {
        effect: S::EffectId,
    },
    AcquireOutboxDispatch {
        effect: S::EffectId,
    },
    SettleOutbox {
        effect: S::EffectId,
        outcome: S::Outcome,
        observation: TransactionObservation<S>,
    },
    /// Terminally cancel a `Committed` outbox row without claiming that an
    /// external adapter was contacted.
    ///
    /// This is the stop/drain counterpart of [`SettleOutbox`](Self::SettleOutbox).
    /// It accepts only the pre-submission phase, records the semantic terminal
    /// outcome and Observation atomically, and removes custody. A `Submitted`
    /// row must continue through `SettleOutbox`, where the semantic owner can
    /// account for the ambiguous external boundary instead of relabelling it
    /// as an unsubmitted cancellation.
    CancelCommittedOutbox {
        effect: S::EffectId,
        outcome: S::Outcome,
        observation: TransactionObservation<S>,
    },
    OpenApproval {
        key: S::ApprovalKey,
        approval: S::Approval,
    },
    ApproveApproval {
        key: S::ApprovalKey,
        ticket: S::ApprovalTicket,
    },
    SettleApproval {
        key: S::ApprovalKey,
        observation: TransactionObservation<S>,
    },
    ReplaceCheckpoint(TransactionCheckpoint<S>),
    SettleCheckpoint {
        actor: S::ActorId,
        at: S::CheckpointStamp,
        observation: TransactionObservation<S>,
    },
}

#[derive(Clone, Debug)]
pub struct StoreTransaction<S: TransactionSchema>(
    Box<[StoreTransactionOp<S>]>,
    Option<S::WriteContext>,
);

impl<S: TransactionSchema> PartialEq for StoreTransaction<S> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}
impl<S: TransactionSchema> Eq for StoreTransaction<S> {}

impl<S: TransactionSchema> StoreTransaction<S> {
    pub fn try_new(operations: Vec<StoreTransactionOp<S>>) -> Result<Self, EmptyTransaction> {
        if operations.is_empty() {
            Err(EmptyTransaction)
        } else {
            Ok(Self(operations.into_boxed_slice(), None))
        }
    }

    #[must_use]
    pub fn with_write_context(mut self, context: S::WriteContext) -> Self {
        self.1 = Some(context);
        self
    }

    pub fn write_context(&self) -> Option<&S::WriteContext> {
        self.1.as_ref()
    }

    pub const fn operations(&self) -> &[StoreTransactionOp<S>] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmptyTransaction;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreTransactionRef<S: TransactionSchema> {
    Appended(S::RecordKey),
    OutboxCommitted(S::EffectId),
    OutboxSubmitted(S::EffectId),
    OutboxDispatchAcquired(S::EffectId),
    OutboxSettled(S::EffectId),
    /// A `Committed` outbox row was terminally accounted without submission.
    OutboxCancelled(S::EffectId),
    ApprovalOpened(S::ApprovalKey),
    ApprovalApproved(S::ApprovalKey),
    ApprovalSettled(S::ApprovalKey),
    CheckpointCurrent {
        actor: S::ActorId,
        at: S::CheckpointStamp,
    },
    CheckpointSettled {
        actor: S::ActorId,
        at: S::CheckpointStamp,
    },
    Outcome(S::EffectId),
    Observation(S::ObservationKey),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreTransactionReceipt<S: TransactionSchema> {
    refs: Box<[StoreTransactionRef<S>]>,
}

impl<S: TransactionSchema> StoreTransactionReceipt<S> {
    fn new(refs: Vec<StoreTransactionRef<S>>) -> Self {
        debug_assert!(!refs.is_empty());
        Self {
            refs: refs.into_boxed_slice(),
        }
    }

    #[must_use]
    pub const fn refs(&self) -> &[StoreTransactionRef<S>] {
        &self.refs
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreTransactionReject<S: TransactionSchema> {
    AppendConflict {
        key: S::RecordKey,
    },
    OutboxConflict {
        effect: S::EffectId,
    },
    OutboxMissing {
        effect: S::EffectId,
    },
    OutboxAlreadySettled {
        effect: S::EffectId,
    },
    OutboxAlreadySubmitted {
        effect: S::EffectId,
    },
    OutboxNotSubmitted {
        effect: S::EffectId,
    },
    OutcomeConflict {
        effect: S::EffectId,
    },
    ApprovalConflict {
        key: S::ApprovalKey,
    },
    ApprovalMissing {
        key: S::ApprovalKey,
    },
    ApprovalDecisionConflict {
        key: S::ApprovalKey,
    },
    CheckpointStale {
        actor: S::ActorId,
        attempted: S::CheckpointStamp,
        current: S::CheckpointStamp,
    },
    CheckpointConflict {
        actor: S::ActorId,
        at: S::CheckpointStamp,
    },
    CheckpointNotCurrent {
        actor: S::ActorId,
        attempted: S::CheckpointStamp,
        current: Option<S::CheckpointStamp>,
    },
    ObservationConflict {
        key: S::ObservationKey,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreTransactionFailureReason<S: TransactionSchema, E> {
    Rejected {
        operation: usize,
        reason: StoreTransactionReject<S>,
    },
    Backend(E),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoreTransactionFailure<S: TransactionSchema, E> {
    transaction: Box<StoreTransaction<S>>,
    reason: StoreTransactionFailureReason<S, E>,
}

impl<S: TransactionSchema, E> StoreTransactionFailure<S, E> {
    #[must_use]
    pub const fn transaction(&self) -> &StoreTransaction<S> {
        &self.transaction
    }

    #[must_use]
    pub const fn reason(&self) -> &StoreTransactionFailureReason<S, E> {
        &self.reason
    }

    pub(crate) fn backend(transaction: StoreTransaction<S>, source: E) -> Self {
        Self {
            transaction: Box::new(transaction),
            reason: StoreTransactionFailureReason::Backend(source),
        }
    }

    pub(crate) fn rejected(
        transaction: StoreTransaction<S>,
        operation: usize,
        reason: StoreTransactionReject<S>,
    ) -> Self {
        Self {
            transaction: Box::new(transaction),
            reason: StoreTransactionFailureReason::Rejected { operation, reason },
        }
    }

    #[must_use]
    pub fn map_backend<T>(self, map: impl FnOnce(E) -> T) -> StoreTransactionFailure<S, T> {
        StoreTransactionFailure {
            transaction: self.transaction,
            reason: match self.reason {
                StoreTransactionFailureReason::Rejected { operation, reason } => {
                    StoreTransactionFailureReason::Rejected { operation, reason }
                }
                StoreTransactionFailureReason::Backend(source) => {
                    StoreTransactionFailureReason::Backend(map(source))
                }
            },
        }
    }

    #[must_use]
    pub fn into_parts(self) -> (StoreTransaction<S>, StoreTransactionFailureReason<S, E>) {
        (*self.transaction, self.reason)
    }
}

pub trait AtomicStoreTransactionPort<S: TransactionSchema> {
    type BackendError;

    fn commit(
        &mut self,
        transaction: StoreTransaction<S>,
    ) -> Result<StoreTransactionReceipt<S>, StoreTransactionFailure<S, Self::BackendError>>;
}

#[derive(Debug)]
pub enum GroupCommitFailure<S: TransactionSchema, CE, BE> {
    Rejected {
        transaction: usize,
        operation: usize,
        reason: StoreTransactionReject<S>,
    },
    Codec { transaction: usize, source: CE },
    Backend(BE),
    Contended,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommittedTransaction<S: TransactionSchema> {
    receipt: StoreTransactionReceipt<S>,
    commit: u64,
}

impl<S: TransactionSchema> CommittedTransaction<S> {
    #[must_use]
    pub const fn new(receipt: StoreTransactionReceipt<S>, commit: u64) -> Self {
        Self { receipt, commit }
    }

    #[must_use]
    pub const fn receipt(&self) -> &StoreTransactionReceipt<S> {
        &self.receipt
    }

    #[must_use]
    pub const fn commit(&self) -> u64 {
        self.commit
    }

    #[must_use]
    pub fn into_parts(self) -> (StoreTransactionReceipt<S>, u64) {
        (self.receipt, self.commit)
    }
}

pub trait GroupTransactionPort<S: TransactionSchema>: AtomicStoreTransactionPort<S> {
    type CodecError;

    fn commit_group(
        &mut self,
        transactions: Vec<StoreTransaction<S>>,
    ) -> Result<
        Box<[CommittedTransaction<S>]>,
        GroupCommitFailure<S, Self::CodecError, Self::BackendError>,
    >;
}

pub trait CrashDurableTransactionPort<S: TransactionSchema>: AtomicStoreTransactionPort<S> {}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ObservationOwner<S: TransactionSchema> {
    Independent,
    Outbox(S::EffectId),
    Approval(S::ApprovalKey),
    Checkpoint(S::ActorId, S::CheckpointStamp),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutboxTerminal {
    Settled,
    Cancelled,
}

trait LaneLiveValue<Req>: Clone + Eq {
    type Phase;

    fn open(request: Req, phase: Self::Phase) -> Self;
    fn request(&self) -> &Req;
    fn phase(&self) -> &Self::Phase;
    fn set_phase(&mut self, phase: Self::Phase);
}

impl<S: TransactionSchema> LaneLiveValue<S::Outbox> for TransactionOutbox<S> {
    type Phase = TransactionOutboxPhase;

    fn open(request: S::Outbox, phase: Self::Phase) -> Self {
        Self { request, phase }
    }

    fn request(&self) -> &S::Outbox {
        &self.request
    }

    fn phase(&self) -> &Self::Phase {
        &self.phase
    }

    fn set_phase(&mut self, phase: Self::Phase) {
        self.phase = phase;
    }
}

impl<S: TransactionSchema> LaneLiveValue<S::Approval> for TransactionApproval<S> {
    type Phase = TransactionApprovalPhase<S::ApprovalTicket>;

    fn open(request: S::Approval, phase: Self::Phase) -> Self {
        Self { request, phase }
    }

    fn request(&self) -> &S::Approval {
        &self.request
    }

    fn phase(&self) -> &Self::Phase {
        &self.phase
    }

    fn set_phase(&mut self, phase: Self::Phase) {
        self.phase = phase;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum LaneEntry<Req, Live, Terminal, Obs> {
    Live(Live),
    Settled {
        request: Req,
        terminal: Terminal,
        observation: Obs,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CustodyLane<K: Ord, Req, Live, Terminal, Obs> {
    entries: BTreeMap<K, LaneEntry<Req, Live, Terminal, Obs>>,
}

impl<K: Ord, Req, Live, Terminal, Obs> Default for CustodyLane<K, Req, Live, Terminal, Obs> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }
}

impl<K: Ord, Req, Live, Terminal, Obs> CustodyLane<K, Req, Live, Terminal, Obs> {
    fn entry(&self, key: &K) -> Option<&LaneEntry<Req, Live, Terminal, Obs>> {
        self.entries.get(key)
    }

    fn restore(&mut self, key: K, entry: Option<LaneEntry<Req, Live, Terminal, Obs>>) {
        match entry {
            Some(entry) => {
                self.entries.insert(key, entry);
            }
            None => {
                self.entries.remove(&key);
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum LaneReject<K> {
    Conflict { key: K },
    Missing { key: K },
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum LaneSettleReject<K, E> {
    Missing { key: K },
    Effects(E),
}

impl<K, Req, Live, Terminal, Obs> CustodyLane<K, Req, Live, Terminal, Obs>
where
    K: Clone + Ord,
    Req: Clone + Eq,
    Live: LaneLiveValue<Req>,
{
    fn open(&mut self, key: K, request: Req, initial: Live::Phase) -> Result<(), LaneReject<K>> {
        match self.entries.get(&key) {
            Some(LaneEntry::Live(row)) if row.request() != &request => {
                Err(LaneReject::Conflict { key })
            }
            Some(LaneEntry::Settled {
                request: original, ..
            }) if original != &request => Err(LaneReject::Conflict { key }),
            Some(_) => Ok(()),
            None => {
                self.entries
                    .insert(key, LaneEntry::Live(Live::open(request, initial)));
                Ok(())
            }
        }
    }

    fn transition(
        &mut self,
        key: &K,
        step: impl FnOnce(&Live::Phase) -> Result<Live::Phase, LaneReject<K>>,
    ) -> Result<(), LaneReject<K>> {
        match self.entries.get_mut(key) {
            Some(LaneEntry::Live(row)) => {
                let phase = step(row.phase())?;
                row.set_phase(phase);
                Ok(())
            }
            Some(LaneEntry::Settled { .. }) | None => Err(LaneReject::Missing { key: key.clone() }),
        }
    }

    fn settle<E>(
        &mut self,
        key: &K,
        terminal: Terminal,
        observation: Obs,
        effects: impl FnOnce(&Req) -> Result<(), E>,
    ) -> Result<(), LaneSettleReject<K, E>>
    where
        Terminal: Clone + Eq,
        Obs: Clone + Eq,
    {
        match self.entries.get_mut(key) {
            Some(entry @ LaneEntry::Live(_)) => {
                let request = match entry {
                    LaneEntry::Live(row) => {
                        effects(row.request()).map_err(LaneSettleReject::Effects)?;
                        row.request().clone()
                    }
                    LaneEntry::Settled { .. } => unreachable!(),
                };
                *entry = LaneEntry::Settled {
                    request,
                    terminal,
                    observation,
                };
                Ok(())
            }
            Some(LaneEntry::Settled {
                request,
                terminal: current_terminal,
                observation: current_observation,
            }) if current_terminal == &terminal && current_observation == &observation => {
                effects(request).map_err(LaneSettleReject::Effects)
            }
            Some(LaneEntry::Settled { .. }) | None => {
                Err(LaneSettleReject::Missing { key: key.clone() })
            }
        }
    }

    fn live(&self, key: &K) -> Option<&Live> {
        match self.entries.get(key) {
            Some(LaneEntry::Live(row)) => Some(row),
            Some(LaneEntry::Settled { .. }) | None => None,
        }
    }

    fn settled(&self, key: &K) -> Option<(&Req, &Terminal, &Obs)> {
        match self.entries.get(key) {
            Some(LaneEntry::Settled {
                request,
                terminal,
                observation,
            }) => Some((request, terminal, observation)),
            Some(LaneEntry::Live(_)) | None => None,
        }
    }

    fn live_entries(&self) -> LaneLiveIter<'_, K, Req, Live, Terminal, Obs> {
        LaneLiveIter {
            remaining: self
                .entries
                .values()
                .filter(|entry| matches!(entry, LaneEntry::Live(_)))
                .count(),
            entries: self.entries.iter(),
        }
    }
}

struct LaneLiveIter<'a, K, Req, Live, Terminal, Obs> {
    entries: std::collections::btree_map::Iter<'a, K, LaneEntry<Req, Live, Terminal, Obs>>,
    remaining: usize,
}

impl<'a, K, Req, Live, Terminal, Obs> Iterator for LaneLiveIter<'a, K, Req, Live, Terminal, Obs> {
    type Item = (&'a K, &'a Live);

    fn next(&mut self) -> Option<Self::Item> {
        for (key, entry) in self.entries.by_ref() {
            if let LaneEntry::Live(row) = entry {
                self.remaining -= 1;
                return Some((key, row));
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, Req, Live, Terminal, Obs> ExactSizeIterator
    for LaneLiveIter<'_, K, Req, Live, Terminal, Obs>
{
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TransactionImage<S: TransactionSchema> {
    records: BTreeMap<S::RecordKey, RecordDigest>,
    outbox: CustodyLane<
        S::EffectId,
        S::Outbox,
        TransactionOutbox<S>,
        OutboxTerminal,
        S::ObservationKey,
    >,
    approvals: CustodyLane<
        S::ApprovalKey,
        S::Approval,
        TransactionApproval<S>,
        TransactionApprovalPhase<S::ApprovalTicket>,
        S::ObservationKey,
    >,
    checkpoints: BTreeMap<S::ActorId, TransactionCheckpoint<S>>,
    checkpoint_high_water: BTreeMap<S::ActorId, TransactionCheckpoint<S>>,
    checkpoint_settlements: BTreeMap<(S::ActorId, S::CheckpointStamp), S::ObservationKey>,
    outcomes: BTreeMap<S::EffectId, S::Outcome>,
    observations: BTreeMap<S::ObservationKey, (ObservationOwner<S>, RecordDigest)>,
    after_horizon: bool,
}

type OutboxLaneEntry<S> = LaneEntry<
    <S as TransactionSchema>::Outbox,
    TransactionOutbox<S>,
    OutboxTerminal,
    <S as TransactionSchema>::ObservationKey,
>;
type ApprovalLaneEntry<S> = LaneEntry<
    <S as TransactionSchema>::Approval,
    TransactionApproval<S>,
    TransactionApprovalPhase<<S as TransactionSchema>::ApprovalTicket>,
    <S as TransactionSchema>::ObservationKey,
>;

pub(crate) struct CommitUndo<S: TransactionSchema>(Vec<ImageUndo<S>>);

impl<S: TransactionSchema> Default for CommitUndo<S> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

enum ImageUndo<S: TransactionSchema> {
    Record(S::RecordKey, Option<RecordDigest>),
    Observation(
        S::ObservationKey,
        Option<(ObservationOwner<S>, RecordDigest)>,
    ),
    Outbox(S::EffectId, Option<OutboxLaneEntry<S>>),
    Outcome(S::EffectId, Option<S::Outcome>),
    Approval(S::ApprovalKey, Option<ApprovalLaneEntry<S>>),
    Checkpoint(S::ActorId, Option<TransactionCheckpoint<S>>),
    CheckpointHighWater(S::ActorId, Option<TransactionCheckpoint<S>>),
    CheckpointSettlement((S::ActorId, S::CheckpointStamp), Option<S::ObservationKey>),
}

impl<S: TransactionSchema> Default for TransactionImage<S> {
    fn default() -> Self {
        Self {
            records: BTreeMap::new(),
            outbox: CustodyLane::default(),
            approvals: CustodyLane::default(),
            checkpoints: BTreeMap::new(),
            checkpoint_high_water: BTreeMap::new(),
            checkpoint_settlements: BTreeMap::new(),
            outcomes: BTreeMap::new(),
            observations: BTreeMap::new(),
            after_horizon: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveringTransactionModel<S: TransactionSchema> {
    image: TransactionImage<S>,
}

impl<S: TransactionSchema> RecoveringTransactionModel<S> {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            image: TransactionImage::default(),
        }
    }
}

impl<S: TransactionSchema> Default for RecoveringTransactionModel<S> {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveredTransactionModel<S: TransactionSchema> {
    image: TransactionImage<S>,
}

impl<S: TransactionSchema> RecoveredTransactionModel<S> {
    pub(crate) fn empty_for_journal_replay() -> Self {
        Self {
            image: TransactionImage::default(),
        }
    }

    pub(crate) fn empty_after_horizon() -> Self {
        Self {
            image: TransactionImage {
                after_horizon: true,
                ..TransactionImage::default()
            },
        }
    }

    pub fn fold_committed(
        mut self,
        transaction: StoreTransaction<S>,
    ) -> Result<(Self, StoreTransactionReceipt<S>), StoreTransactionFailure<S, Infallible>> {
        let mut refs = Vec::new();
        for (operation, change) in transaction.operations().iter().enumerate() {
            if let Err(reason) = self.image.apply(change, &mut refs) {
                return Err(StoreTransactionFailure::rejected(
                    transaction,
                    operation,
                    reason,
                ));
            }
        }
        Ok((self, StoreTransactionReceipt::new(refs)))
    }

    pub(crate) fn fold_undoable(
        &mut self,
        transaction: &StoreTransaction<S>,
        undo: &mut CommitUndo<S>,
    ) -> Result<StoreTransactionReceipt<S>, (usize, StoreTransactionReject<S>)> {
        let mut refs = Vec::new();
        for (operation, change) in transaction.operations().iter().enumerate() {
            self.image.undo_slots(change, &mut undo.0);
            if let Err(reason) = self.image.apply(change, &mut refs) {
                return Err((operation, reason));
            }
        }
        Ok(StoreTransactionReceipt::new(refs))
    }

    pub(crate) fn rewind(&mut self, undo: CommitUndo<S>) {
        self.image.rewind(undo.0);
    }

    #[must_use]
    pub fn reopen(self) -> RecoveringTransactionModel<S> {
        RecoveringTransactionModel { image: self.image }
    }

    #[must_use]
    pub fn record_digest(&self, key: &S::RecordKey) -> Option<RecordDigest> {
        self.image.records.get(key).copied()
    }

    #[must_use]
    pub fn outbox(&self, effect: &S::EffectId) -> Option<&TransactionOutbox<S>> {
        self.image.outbox.live(effect)
    }

    /// Current, non-terminal outbox custody in deterministic effect order.
    pub fn open_outboxes(
        &self,
    ) -> impl ExactSizeIterator<Item = (&S::EffectId, &TransactionOutbox<S>)> {
        self.image.outbox.live_entries()
    }

    #[must_use]
    pub fn approval(&self, key: &S::ApprovalKey) -> Option<&TransactionApproval<S>> {
        self.image.approvals.live(key)
    }

    #[must_use]
    pub fn checkpoint(&self, actor: &S::ActorId) -> Option<&TransactionCheckpoint<S>> {
        self.image.checkpoints.get(actor)
    }

    /// Current checkpoint custody in deterministic actor order.
    pub fn current_checkpoints(
        &self,
    ) -> impl ExactSizeIterator<Item = (&S::ActorId, &TransactionCheckpoint<S>)> {
        self.image.checkpoints.iter()
    }

    #[must_use]
    pub fn outcome(&self, effect: &S::EffectId) -> Option<&S::Outcome> {
        self.image.outcomes.get(effect)
    }

    #[must_use]
    pub fn observation_digest(&self, key: &S::ObservationKey) -> Option<RecordDigest> {
        self.image.observations.get(key).map(|(_, digest)| *digest)
    }
}

impl<S: TransactionSchema> AtomicStoreTransactionPort<S> for RecoveredTransactionModel<S> {
    type BackendError = Infallible;

    fn commit(
        &mut self,
        transaction: StoreTransaction<S>,
    ) -> Result<StoreTransactionReceipt<S>, StoreTransactionFailure<S, Self::BackendError>> {
        let mut undo = CommitUndo::default();
        match self.fold_undoable(&transaction, &mut undo) {
            Ok(receipt) => Ok(receipt),
            Err((operation, reason)) => {
                self.rewind(undo);
                Err(StoreTransactionFailure {
                    transaction: Box::new(transaction),
                    reason: StoreTransactionFailureReason::Rejected { operation, reason },
                })
            }
        }
    }
}

impl<S: TransactionSchema> TransactionImage<S> {
    fn undo_slots(&self, operation: &StoreTransactionOp<S>, into: &mut Vec<ImageUndo<S>>) {
        let observation_slot = |into: &mut Vec<ImageUndo<S>>, key: &S::ObservationKey| {
            into.push(ImageUndo::Observation(
                key.clone(),
                self.observations.get(key).cloned(),
            ));
        };
        match operation {
            StoreTransactionOp::Append(append) => into.push(ImageUndo::Record(
                append.key().clone(),
                self.records.get(append.key()).copied(),
            )),
            StoreTransactionOp::AppendObservation(observation) => {
                observation_slot(into, observation.key());
            }
            StoreTransactionOp::OpenOutbox { effect, .. }
            | StoreTransactionOp::SubmitOutbox { effect }
            | StoreTransactionOp::AcquireOutboxDispatch { effect } => into.push(ImageUndo::Outbox(
                effect.clone(),
                self.outbox.entry(effect).cloned(),
            )),
            StoreTransactionOp::SettleOutbox {
                effect,
                observation,
                ..
            }
            | StoreTransactionOp::CancelCommittedOutbox {
                effect,
                observation,
                ..
            } => {
                into.push(ImageUndo::Outbox(
                    effect.clone(),
                    self.outbox.entry(effect).cloned(),
                ));
                into.push(ImageUndo::Outcome(
                    effect.clone(),
                    self.outcomes.get(effect).cloned(),
                ));
                observation_slot(into, observation.key());
            }
            StoreTransactionOp::OpenApproval { key, .. }
            | StoreTransactionOp::ApproveApproval { key, .. } => into.push(ImageUndo::Approval(
                key.clone(),
                self.approvals.entry(key).cloned(),
            )),
            StoreTransactionOp::SettleApproval { key, observation } => {
                into.push(ImageUndo::Approval(
                    key.clone(),
                    self.approvals.entry(key).cloned(),
                ));
                observation_slot(into, observation.key());
            }
            StoreTransactionOp::ReplaceCheckpoint(checkpoint) => {
                let actor = checkpoint.actor();
                into.push(ImageUndo::Checkpoint(
                    actor.clone(),
                    self.checkpoints.get(actor).cloned(),
                ));
                into.push(ImageUndo::CheckpointHighWater(
                    actor.clone(),
                    self.checkpoint_high_water.get(actor).cloned(),
                ));
            }
            StoreTransactionOp::SettleCheckpoint {
                actor,
                at,
                observation,
            } => {
                into.push(ImageUndo::Checkpoint(
                    actor.clone(),
                    self.checkpoints.get(actor).cloned(),
                ));
                let settlement = (actor.clone(), at.clone());
                into.push(ImageUndo::CheckpointSettlement(
                    settlement.clone(),
                    self.checkpoint_settlements.get(&settlement).cloned(),
                ));
                observation_slot(into, observation.key());
            }
        }
    }

    fn rewind(&mut self, undo: Vec<ImageUndo<S>>) {
        fn restore_map<K: Ord, V>(map: &mut BTreeMap<K, V>, key: K, value: Option<V>) {
            match value {
                Some(value) => {
                    map.insert(key, value);
                }
                None => {
                    map.remove(&key);
                }
            }
        }
        for slot in undo.into_iter().rev() {
            match slot {
                ImageUndo::Record(key, value) => restore_map(&mut self.records, key, value),
                ImageUndo::Observation(key, value) => {
                    restore_map(&mut self.observations, key, value);
                }
                ImageUndo::Outbox(key, entry) => self.outbox.restore(key, entry),
                ImageUndo::Outcome(key, value) => restore_map(&mut self.outcomes, key, value),
                ImageUndo::Approval(key, entry) => self.approvals.restore(key, entry),
                ImageUndo::Checkpoint(key, value) => restore_map(&mut self.checkpoints, key, value),
                ImageUndo::CheckpointHighWater(key, value) => {
                    restore_map(&mut self.checkpoint_high_water, key, value);
                }
                ImageUndo::CheckpointSettlement(key, value) => {
                    restore_map(&mut self.checkpoint_settlements, key, value);
                }
            }
        }
    }

    fn apply(
        &mut self,
        operation: &StoreTransactionOp<S>,
        refs: &mut Vec<StoreTransactionRef<S>>,
    ) -> Result<(), StoreTransactionReject<S>> {
        match operation {
            StoreTransactionOp::Append(append) => self.append(append, refs),
            StoreTransactionOp::AppendObservation(observation) => {
                self.append_observation(observation, refs)
            }
            StoreTransactionOp::OpenOutbox { effect, request } => {
                self.open_outbox(effect, request, refs)
            }
            StoreTransactionOp::SubmitOutbox { effect } => self.submit_outbox(effect, refs),
            StoreTransactionOp::AcquireOutboxDispatch { effect } => {
                self.acquire_outbox_dispatch(effect, refs)
            }
            StoreTransactionOp::SettleOutbox {
                effect,
                outcome,
                observation,
            } => self.settle_outbox(effect, outcome, observation, refs),
            StoreTransactionOp::CancelCommittedOutbox {
                effect,
                outcome,
                observation,
            } => self.cancel_committed_outbox(effect, outcome, observation, refs),
            StoreTransactionOp::OpenApproval { key, approval } => {
                self.open_approval(key, approval, refs)
            }
            StoreTransactionOp::ApproveApproval { key, ticket } => {
                self.approve_approval(key, ticket, refs)
            }
            StoreTransactionOp::SettleApproval { key, observation } => {
                self.settle_approval(key, observation, refs)
            }
            StoreTransactionOp::ReplaceCheckpoint(checkpoint) => {
                self.replace_checkpoint(checkpoint, refs)
            }
            StoreTransactionOp::SettleCheckpoint {
                actor,
                at,
                observation,
            } => self.settle_checkpoint(actor, at, observation, refs),
        }
    }

    fn append(
        &mut self,
        append: &TransactionAppend<S>,
        refs: &mut Vec<StoreTransactionRef<S>>,
    ) -> Result<(), StoreTransactionReject<S>> {
        let digest = S::record_digest(append.record());
        match self.records.get(append.key()) {
            Some(current) if *current != digest => {
                return Err(StoreTransactionReject::AppendConflict {
                    key: append.key().clone(),
                });
            }
            Some(_) => {}
            None => {
                self.records.insert(append.key().clone(), digest);
            }
        }
        refs.push(StoreTransactionRef::Appended(append.key().clone()));
        Ok(())
    }

    fn append_observation(
        &mut self,
        observation: &TransactionObservation<S>,
        refs: &mut Vec<StoreTransactionRef<S>>,
    ) -> Result<(), StoreTransactionReject<S>> {
        Self::insert_observation(
            &mut self.observations,
            observation,
            ObservationOwner::Independent,
        )?;
        refs.push(StoreTransactionRef::Observation(observation.key().clone()));
        Ok(())
    }

    fn open_outbox(
        &mut self,
        effect: &S::EffectId,
        request: &S::Outbox,
        refs: &mut Vec<StoreTransactionRef<S>>,
    ) -> Result<(), StoreTransactionReject<S>> {
        self.outbox
            .open(
                effect.clone(),
                request.clone(),
                TransactionOutboxPhase::Committed,
            )
            .map_err(|reject| match reject {
                LaneReject::Conflict { key } => {
                    StoreTransactionReject::OutboxConflict { effect: key }
                }
                LaneReject::Missing { key } => {
                    StoreTransactionReject::OutboxMissing { effect: key }
                }
            })?;
        refs.push(StoreTransactionRef::OutboxCommitted(effect.clone()));
        Ok(())
    }

    fn submit_outbox(
        &mut self,
        effect: &S::EffectId,
        refs: &mut Vec<StoreTransactionRef<S>>,
    ) -> Result<(), StoreTransactionReject<S>> {
        if let Some((_, terminal, _)) = self.outbox.settled(effect) {
            match terminal {
                OutboxTerminal::Settled => {}
                OutboxTerminal::Cancelled => {
                    return Err(StoreTransactionReject::OutboxAlreadySettled {
                        effect: effect.clone(),
                    });
                }
            }
        } else {
            self.outbox
                .transition(effect, |_| Ok(TransactionOutboxPhase::Submitted))
                .map_err(|reject| match reject {
                    LaneReject::Conflict { key } | LaneReject::Missing { key } => {
                        StoreTransactionReject::OutboxMissing { effect: key }
                    }
                })?;
        }
        refs.push(StoreTransactionRef::OutboxSubmitted(effect.clone()));
        Ok(())
    }

    fn acquire_outbox_dispatch(
        &mut self,
        effect: &S::EffectId,
        refs: &mut Vec<StoreTransactionRef<S>>,
    ) -> Result<(), StoreTransactionReject<S>> {
        if self.outbox.settled(effect).is_some() {
            return Err(StoreTransactionReject::OutboxAlreadySettled {
                effect: effect.clone(),
            });
        }
        self.outbox
            .transition(effect, |phase| match phase {
                TransactionOutboxPhase::Committed => Ok(TransactionOutboxPhase::Submitted),
                TransactionOutboxPhase::Submitted => Err(LaneReject::Conflict {
                    key: effect.clone(),
                }),
            })
            .map_err(|reject| match reject {
                LaneReject::Conflict { key } => {
                    StoreTransactionReject::OutboxAlreadySubmitted { effect: key }
                }
                LaneReject::Missing { key } => {
                    StoreTransactionReject::OutboxMissing { effect: key }
                }
            })?;
        refs.push(StoreTransactionRef::OutboxDispatchAcquired(effect.clone()));
        Ok(())
    }

    fn settle_outbox(
        &mut self,
        effect: &S::EffectId,
        outcome: &S::Outcome,
        observation: &TransactionObservation<S>,
        refs: &mut Vec<StoreTransactionRef<S>>,
    ) -> Result<(), StoreTransactionReject<S>> {
        let owner = ObservationOwner::Outbox(effect.clone());
        if matches!(
            self.outbox.live(effect).map(|row| row.phase()),
            Some(TransactionOutboxPhase::Committed)
        ) {
            return Err(StoreTransactionReject::OutboxNotSubmitted {
                effect: effect.clone(),
            });
        }
        let terminal = match self.outbox.settled(effect) {
            Some((_, terminal, settled_observation))
                if settled_observation == observation.key() =>
            {
                *terminal
            }
            _ => OutboxTerminal::Settled,
        };
        let (outbox, outcomes, observations) =
            (&mut self.outbox, &mut self.outcomes, &mut self.observations);
        outbox
            .settle(effect, terminal, observation.key().clone(), |_request| {
                Self::insert_outcome(outcomes, effect, outcome)?;
                Self::insert_observation(observations, observation, owner)
            })
            .map_err(|reject| match reject {
                LaneSettleReject::Missing { key } => {
                    StoreTransactionReject::OutboxMissing { effect: key }
                }
                LaneSettleReject::Effects(reason) => reason,
            })?;
        refs.push(StoreTransactionRef::OutboxSettled(effect.clone()));
        refs.push(StoreTransactionRef::Outcome(effect.clone()));
        refs.push(StoreTransactionRef::Observation(observation.key().clone()));
        Ok(())
    }

    fn cancel_committed_outbox(
        &mut self,
        effect: &S::EffectId,
        outcome: &S::Outcome,
        observation: &TransactionObservation<S>,
        refs: &mut Vec<StoreTransactionRef<S>>,
    ) -> Result<(), StoreTransactionReject<S>> {
        let owner = ObservationOwner::Outbox(effect.clone());
        if matches!(
            self.outbox.live(effect).map(|row| row.phase()),
            Some(TransactionOutboxPhase::Submitted)
        ) {
            return Err(StoreTransactionReject::OutboxAlreadySubmitted {
                effect: effect.clone(),
            });
        }
        if let Some((_, terminal, settled_observation)) = self.outbox.settled(effect) {
            if terminal != &OutboxTerminal::Cancelled || settled_observation != observation.key() {
                return Err(StoreTransactionReject::OutboxAlreadySettled {
                    effect: effect.clone(),
                });
            }
        }
        let (outbox, outcomes, observations) =
            (&mut self.outbox, &mut self.outcomes, &mut self.observations);
        outbox
            .settle(
                effect,
                OutboxTerminal::Cancelled,
                observation.key().clone(),
                |_request| {
                    Self::insert_outcome(outcomes, effect, outcome)?;
                    Self::insert_observation(observations, observation, owner)
                },
            )
            .map_err(|reject| match reject {
                LaneSettleReject::Missing { key } => {
                    StoreTransactionReject::OutboxMissing { effect: key }
                }
                LaneSettleReject::Effects(reason) => reason,
            })?;
        refs.push(StoreTransactionRef::OutboxCancelled(effect.clone()));
        refs.push(StoreTransactionRef::Outcome(effect.clone()));
        refs.push(StoreTransactionRef::Observation(observation.key().clone()));
        Ok(())
    }

    fn open_approval(
        &mut self,
        key: &S::ApprovalKey,
        approval: &S::Approval,
        refs: &mut Vec<StoreTransactionRef<S>>,
    ) -> Result<(), StoreTransactionReject<S>> {
        self.approvals
            .open(
                key.clone(),
                approval.clone(),
                TransactionApprovalPhase::Requested,
            )
            .map_err(|reject| match reject {
                LaneReject::Conflict { key } => StoreTransactionReject::ApprovalConflict { key },
                LaneReject::Missing { key } => StoreTransactionReject::ApprovalMissing { key },
            })?;
        refs.push(StoreTransactionRef::ApprovalOpened(key.clone()));
        Ok(())
    }

    fn approve_approval(
        &mut self,
        key: &S::ApprovalKey,
        ticket: &S::ApprovalTicket,
        refs: &mut Vec<StoreTransactionRef<S>>,
    ) -> Result<(), StoreTransactionReject<S>> {
        if let Some((_, terminal, _)) = self.approvals.settled(key) {
            match terminal {
                TransactionApprovalPhase::Approved(previous) if previous == ticket => {}
                TransactionApprovalPhase::Approved(_) => {
                    return Err(StoreTransactionReject::ApprovalDecisionConflict {
                        key: key.clone(),
                    });
                }
                TransactionApprovalPhase::Requested => {
                    return Err(StoreTransactionReject::ApprovalMissing { key: key.clone() });
                }
            }
        } else {
            self.approvals
                .transition(key, |phase| match phase {
                    TransactionApprovalPhase::Requested => {
                        Ok(TransactionApprovalPhase::Approved(ticket.clone()))
                    }
                    TransactionApprovalPhase::Approved(previous) if previous == ticket => {
                        Ok(TransactionApprovalPhase::Approved(previous.clone()))
                    }
                    TransactionApprovalPhase::Approved(_) => {
                        Err(LaneReject::Conflict { key: key.clone() })
                    }
                })
                .map_err(|reject| match reject {
                    LaneReject::Conflict { key } => {
                        StoreTransactionReject::ApprovalDecisionConflict { key }
                    }
                    LaneReject::Missing { key } => StoreTransactionReject::ApprovalMissing { key },
                })?;
        }
        refs.push(StoreTransactionRef::ApprovalApproved(key.clone()));
        Ok(())
    }

    fn settle_approval(
        &mut self,
        key: &S::ApprovalKey,
        observation: &TransactionObservation<S>,
        refs: &mut Vec<StoreTransactionRef<S>>,
    ) -> Result<(), StoreTransactionReject<S>> {
        let owner = ObservationOwner::Approval(key.clone());
        let terminal = self
            .approvals
            .live(key)
            .map(|row| row.phase.clone())
            .or_else(|| {
                self.approvals
                    .settled(key)
                    .map(|(_, terminal, _)| terminal.clone())
            })
            .ok_or_else(|| StoreTransactionReject::ApprovalMissing { key: key.clone() })?;
        let (approvals, observations) = (&mut self.approvals, &mut self.observations);
        approvals
            .settle(key, terminal, observation.key().clone(), |_approval| {
                Self::insert_observation(observations, observation, owner)
            })
            .map_err(|reject| match reject {
                LaneSettleReject::Missing { key } => {
                    StoreTransactionReject::ApprovalMissing { key }
                }
                LaneSettleReject::Effects(reason) => reason,
            })?;
        refs.push(StoreTransactionRef::ApprovalSettled(key.clone()));
        refs.push(StoreTransactionRef::Observation(observation.key().clone()));
        Ok(())
    }

    fn replace_checkpoint(
        &mut self,
        checkpoint: &TransactionCheckpoint<S>,
        refs: &mut Vec<StoreTransactionRef<S>>,
    ) -> Result<(), StoreTransactionReject<S>> {
        let current = self.checkpoint_high_water.get(checkpoint.actor());
        let replace = match current.map(|row| (checkpoint.at().cmp(row.at()), row)) {
            None | Some((std::cmp::Ordering::Greater, _)) => true,
            Some((std::cmp::Ordering::Equal, row)) if row == checkpoint => false,
            Some((std::cmp::Ordering::Equal, _)) => {
                return Err(StoreTransactionReject::CheckpointConflict {
                    actor: checkpoint.actor().clone(),
                    at: checkpoint.at().clone(),
                });
            }
            Some((std::cmp::Ordering::Less, row)) => {
                return Err(StoreTransactionReject::CheckpointStale {
                    actor: checkpoint.actor().clone(),
                    attempted: checkpoint.at().clone(),
                    current: row.at().clone(),
                });
            }
        };
        if replace {
            self.checkpoint_high_water
                .insert(checkpoint.actor().clone(), checkpoint.clone());
            self.checkpoints
                .insert(checkpoint.actor().clone(), checkpoint.clone());
        }
        refs.push(StoreTransactionRef::CheckpointCurrent {
            actor: checkpoint.actor().clone(),
            at: checkpoint.at().clone(),
        });
        Ok(())
    }

    fn settle_checkpoint(
        &mut self,
        actor: &S::ActorId,
        at: &S::CheckpointStamp,
        observation: &TransactionObservation<S>,
        refs: &mut Vec<StoreTransactionRef<S>>,
    ) -> Result<(), StoreTransactionReject<S>> {
        let settlement_key = (actor.clone(), at.clone());
        let owner = ObservationOwner::Checkpoint(actor.clone(), at.clone());
        match self.checkpoints.get(actor) {
            Some(current) if current.at() == at => {
                Self::insert_observation(&mut self.observations, observation, owner)?;
                self.checkpoints.remove(actor);
                self.checkpoint_settlements
                    .insert(settlement_key, observation.key().clone());
            }
            Some(current) => {
                return Err(StoreTransactionReject::CheckpointNotCurrent {
                    actor: actor.clone(),
                    attempted: at.clone(),
                    current: Some(current.at().clone()),
                });
            }
            None if self.checkpoint_settlements.get(&settlement_key) == Some(observation.key()) => {
                Self::insert_observation(&mut self.observations, observation, owner)?;
            }
            None if self.after_horizon && !self.checkpoint_high_water.contains_key(actor) => {
                Self::insert_observation(&mut self.observations, observation, owner)?;
                self.checkpoint_settlements
                    .insert(settlement_key, observation.key().clone());
            }
            None => {
                return Err(StoreTransactionReject::CheckpointNotCurrent {
                    actor: actor.clone(),
                    attempted: at.clone(),
                    current: None,
                });
            }
        }
        refs.push(StoreTransactionRef::CheckpointSettled {
            actor: actor.clone(),
            at: at.clone(),
        });
        refs.push(StoreTransactionRef::Observation(observation.key().clone()));
        Ok(())
    }

    fn insert_outcome(
        outcomes: &mut BTreeMap<S::EffectId, S::Outcome>,
        effect: &S::EffectId,
        outcome: &S::Outcome,
    ) -> Result<(), StoreTransactionReject<S>> {
        match outcomes.get(effect) {
            Some(current) if current != outcome => Err(StoreTransactionReject::OutcomeConflict {
                effect: effect.clone(),
            }),
            Some(_) => Ok(()),
            None => {
                outcomes.insert(effect.clone(), outcome.clone());
                Ok(())
            }
        }
    }

    fn insert_observation(
        observations: &mut BTreeMap<S::ObservationKey, (ObservationOwner<S>, RecordDigest)>,
        observation: &TransactionObservation<S>,
        owner: ObservationOwner<S>,
    ) -> Result<(), StoreTransactionReject<S>> {
        let digest = S::observation_digest(observation.value());
        match observations.get(observation.key()) {
            Some((current_owner, current_digest))
                if current_owner == &owner && *current_digest == digest =>
            {
                Ok(())
            }
            Some(_) => Err(StoreTransactionReject::ObservationConflict {
                key: observation.key().clone(),
            }),
            None => {
                observations.insert(observation.key().clone(), (owner, digest));
                Ok(())
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointOwners<S: TransactionSchema>(BTreeMap<S::ActorId, S::Incarnation>);

impl<S: TransactionSchema> CheckpointOwners<S> {
    pub fn try_new(
        owners: impl IntoIterator<Item = (S::ActorId, S::Incarnation)>,
    ) -> Result<Self, CheckpointOwnerConflict<S>> {
        let mut result = BTreeMap::new();
        for (actor, owner) in owners {
            if let Some(current) = result.get(&actor) {
                if current != &owner {
                    return Err(CheckpointOwnerConflict { actor });
                }
            } else {
                result.insert(actor, owner);
            }
        }
        Ok(Self(result))
    }

    #[must_use]
    pub fn owner(&self, actor: &S::ActorId) -> Option<&S::Incarnation> {
        self.0.get(actor)
    }
}

impl<S: TransactionSchema> Default for CheckpointOwners<S> {
    fn default() -> Self {
        Self(BTreeMap::new())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckpointOwnerConflict<S: TransactionSchema> {
    actor: S::ActorId,
}

impl<S: TransactionSchema> CheckpointOwnerConflict<S> {
    #[must_use]
    pub const fn actor(&self) -> &S::ActorId {
        &self.actor
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryTerminal<S: TransactionSchema> {
    outcome: S::Outcome,
    observation: TransactionObservation<S>,
}

impl<S: TransactionSchema> RecoveryTerminal<S> {
    #[must_use]
    pub const fn new(outcome: S::Outcome, observation: TransactionObservation<S>) -> Self {
        Self {
            outcome,
            observation,
        }
    }

    #[must_use]
    pub const fn outcome(&self) -> &S::Outcome {
        &self.outcome
    }

    #[must_use]
    pub const fn observation(&self) -> &TransactionObservation<S> {
        &self.observation
    }

    #[must_use]
    pub fn into_parts(self) -> (S::Outcome, TransactionObservation<S>) {
        (self.outcome, self.observation)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionRecoveryPlan<S: TransactionSchema> {
    resubmit_outbox: Box<[S::EffectId]>,
    terminal_outbox: Box<[S::EffectId]>,
    preserve_approvals: Box<[S::ApprovalKey]>,
    restore_checkpoints: Box<[(S::ActorId, S::CheckpointStamp)]>,
    withhold_checkpoints: Box<[(S::ActorId, S::CheckpointStamp)]>,
}

impl<S: TransactionSchema> TransactionRecoveryPlan<S> {
    #[must_use]
    pub const fn resubmit_outbox(&self) -> &[S::EffectId] {
        &self.resubmit_outbox
    }

    #[must_use]
    pub const fn terminal_outbox(&self) -> &[S::EffectId] {
        &self.terminal_outbox
    }

    #[must_use]
    pub const fn preserve_approvals(&self) -> &[S::ApprovalKey] {
        &self.preserve_approvals
    }

    #[must_use]
    pub const fn restore_checkpoints(&self) -> &[(S::ActorId, S::CheckpointStamp)] {
        &self.restore_checkpoints
    }

    #[must_use]
    pub const fn withhold_checkpoints(&self) -> &[(S::ActorId, S::CheckpointStamp)] {
        &self.withhold_checkpoints
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionRecoveryFailure<S: TransactionSchema, E> {
    TerminalOutcome {
        effect: S::EffectId,
        source: E,
    },
    SettlementRejected {
        effect: S::EffectId,
        reason: StoreTransactionReject<S>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionRecoveryError<S: TransactionSchema, E> {
    recovering: Box<RecoveringTransactionModel<S>>,
    failure: TransactionRecoveryFailure<S, E>,
}

impl<S: TransactionSchema, E> TransactionRecoveryError<S, E> {
    #[must_use]
    pub const fn failure(&self) -> &TransactionRecoveryFailure<S, E> {
        &self.failure
    }

    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        RecoveringTransactionModel<S>,
        TransactionRecoveryFailure<S, E>,
    ) {
        (*self.recovering, self.failure)
    }

    #[must_use]
    pub fn into_recovering(self) -> RecoveringTransactionModel<S> {
        *self.recovering
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionRecoveryResult<S: TransactionSchema> {
    store: RecoveredTransactionModel<S>,
    plan: TransactionRecoveryPlan<S>,
}

impl<S: TransactionSchema> TransactionRecoveryResult<S> {
    #[must_use]
    pub const fn store(&self) -> &RecoveredTransactionModel<S> {
        &self.store
    }

    #[must_use]
    pub const fn plan(&self) -> &TransactionRecoveryPlan<S> {
        &self.plan
    }

    #[must_use]
    pub fn into_parts(self) -> (RecoveredTransactionModel<S>, TransactionRecoveryPlan<S>) {
        (self.store, self.plan)
    }
}

impl<S: TransactionSchema> RecoveringTransactionModel<S> {
    pub fn recover<E, F>(
        self,
        owners: &CheckpointOwners<S>,
        mut terminal: F,
    ) -> Result<TransactionRecoveryResult<S>, TransactionRecoveryError<S, E>>
    where
        F: FnMut(&S::EffectId, &S::Outbox) -> Result<RecoveryTerminal<S>, E>,
    {
        self.recover_outboxes(owners, |key, request| terminal(key, request).map(Some))
    }

    /// The request owner may retain a Submitted request for at-least-once retry.
    /// None preserves its exact phase/body and grants no live dispatch authority.
    pub fn recover_outboxes<E, F>(
        self,
        owners: &CheckpointOwners<S>,
        mut terminal: F,
    ) -> Result<TransactionRecoveryResult<S>, TransactionRecoveryError<S, E>>
    where
        F: FnMut(&S::EffectId, &S::Outbox) -> Result<Option<RecoveryTerminal<S>>, E>,
    {
        let mut candidate = self.image.clone();
        let preserve_approvals = candidate
            .approvals
            .live_entries()
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let mut resubmit_outbox = Vec::new();
        let mut terminal_outbox = Vec::new();

        let outbox_rows = candidate
            .outbox
            .live_entries()
            .map(|(effect, row)| (effect.clone(), row.clone()))
            .collect::<Vec<_>>();
        for (effect, row) in outbox_rows {
            match row.phase() {
                TransactionOutboxPhase::Committed => resubmit_outbox.push(effect),
                TransactionOutboxPhase::Submitted => {
                    let settlement = match terminal(&effect, row.request()) {
                        Ok(Some(settlement)) => settlement,
                        Ok(None) => {
                            resubmit_outbox.push(effect);
                            continue;
                        }
                        Err(source) => {
                            return Err(TransactionRecoveryError {
                                recovering: Box::new(self),
                                failure: TransactionRecoveryFailure::TerminalOutcome {
                                    effect,
                                    source,
                                },
                            });
                        }
                    };
                    let operation = StoreTransactionOp::SettleOutbox {
                        effect: effect.clone(),
                        outcome: settlement.outcome,
                        observation: settlement.observation,
                    };
                    if let Err(reason) = candidate.apply(&operation, &mut Vec::new()) {
                        return Err(TransactionRecoveryError {
                            recovering: Box::new(self),
                            failure: TransactionRecoveryFailure::SettlementRejected {
                                effect,
                                reason,
                            },
                        });
                    }
                    terminal_outbox.push(effect);
                }
            }
        }

        let mut restore_checkpoints = Vec::new();
        let mut withhold_checkpoints = Vec::new();
        for checkpoint in candidate.checkpoints.values() {
            let item = (checkpoint.actor().clone(), checkpoint.at().clone());
            if owners.owner(checkpoint.actor()) == Some(checkpoint.owner()) {
                restore_checkpoints.push(item);
            } else {
                withhold_checkpoints.push(item);
            }
        }

        Ok(TransactionRecoveryResult {
            store: RecoveredTransactionModel { image: candidate },
            plan: TransactionRecoveryPlan {
                resubmit_outbox: resubmit_outbox.into_boxed_slice(),
                terminal_outbox: terminal_outbox.into_boxed_slice(),
                preserve_approvals: preserve_approvals.into_boxed_slice(),
                restore_checkpoints: restore_checkpoints.into_boxed_slice(),
                withhold_checkpoints: withhold_checkpoints.into_boxed_slice(),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct TestSchema;

    impl TransactionSchema for TestSchema {
        type WriteContext = ();
        fn record_digest(record: &Self::Record) -> super::RecordDigest {
            super::RecordDigest::of_bytes(record.as_bytes())
        }

        fn observation_digest(observation: &Self::Observation) -> super::RecordDigest {
            super::RecordDigest::of_bytes(observation.as_bytes())
        }

        type RecordKey = u8;
        type Record = &'static str;
        type EffectId = u8;
        type Outbox = &'static str;
        type ApprovalKey = u8;
        type Approval = &'static str;
        type ApprovalTicket = &'static str;
        type ActorId = u8;
        type Incarnation = u8;
        type CheckpointStamp = u8;
        type CheckpointState = &'static str;
        type Outcome = &'static str;
        type ObservationKey = u8;
        type Observation = &'static str;
    }

    fn recover_empty() -> RecoveredTransactionModel<TestSchema> {
        RecoveringTransactionModel::empty()
            .recover(&CheckpointOwners::default(), |_, _| {
                Err::<RecoveryTerminal<TestSchema>, _>("unreachable")
            })
            .expect("empty recovery cannot call the terminalizer")
            .into_parts()
            .0
    }

    fn transaction(
        operations: Vec<StoreTransactionOp<TestSchema>>,
    ) -> StoreTransaction<TestSchema> {
        StoreTransaction::try_new(operations).expect("fixture transaction is non-empty")
    }

    fn checkpoint(
        actor: u8,
        owner: u8,
        at: u8,
        state: &'static str,
    ) -> TransactionCheckpoint<TestSchema> {
        TransactionCheckpoint::new(actor, owner, at, state, [at, at.saturating_add(10)])
    }

    #[test]
    fn independent_observation_append_is_idempotent_conflict_safe_and_iterable() {
        let mut store = recover_empty();
        let append = transaction(vec![StoreTransactionOp::AppendObservation(
            TransactionObservation::new(7, "lifecycle"),
        )]);

        let first = store
            .commit(append.clone())
            .expect("first observation append");
        assert_eq!(
            first.refs(),
            &[StoreTransactionRef::Observation(7)],
            "the receipt names exactly the durable observation row"
        );
        let committed = store.clone();
        assert_eq!(
            store.commit(append).expect("exact retry is idempotent"),
            first
        );
        assert_eq!(store, committed);

        let conflict = store
            .commit(transaction(vec![StoreTransactionOp::AppendObservation(
                TransactionObservation::new(7, "different"),
            )]))
            .expect_err("same key with another value must conflict");
        assert!(matches!(
            conflict.reason(),
            StoreTransactionFailureReason::Rejected {
                operation: 0,
                reason: StoreTransactionReject::ObservationConflict { key: 7 },
            }
        ));
        assert_eq!(store, committed, "a conflict cannot partially mutate state");
        assert_eq!(
            store.observation_digest(&7),
            Some(TestSchema::observation_digest(&"lifecycle"))
        );
    }

    #[test]
    fn batch_commit_is_all_or_nothing_and_exact_retry_has_the_same_receipt() {
        let mut store = recover_empty();
        let first = transaction(vec![
            StoreTransactionOp::Append(TransactionAppend::new(1, "record")),
            StoreTransactionOp::OpenOutbox {
                effect: 1,
                request: "outbox",
            },
        ]);
        let receipt = store.commit(first.clone()).expect("first atomic commit");
        let committed = store.clone();
        let duplicate = store.commit(first).expect("exact retry is idempotent");
        assert_eq!(duplicate, receipt);
        assert_eq!(store, committed);

        let conflict = transaction(vec![
            StoreTransactionOp::Append(TransactionAppend::new(2, "would-be-partial")),
            StoreTransactionOp::OpenOutbox {
                effect: 1,
                request: "different",
            },
        ]);
        let before = store.clone();
        let failure = store
            .commit(conflict)
            .expect_err("second operation conflicts");
        assert!(matches!(
            failure.reason(),
            StoreTransactionFailureReason::Rejected {
                operation: 1,
                reason: StoreTransactionReject::OutboxConflict { effect: 1 },
            }
        ));
        assert_eq!(store, before);
        assert_eq!(store.record_digest(&2), None);
    }

    #[test]
    fn outbox_outcome_and_observation_never_become_partial() {
        let mut store = recover_empty();
        store
            .commit(transaction(vec![
                StoreTransactionOp::OpenOutbox {
                    effect: 7,
                    request: "request",
                },
                StoreTransactionOp::SubmitOutbox { effect: 7 },
                StoreTransactionOp::OpenApproval {
                    key: 9,
                    approval: "approval",
                },
                StoreTransactionOp::SettleApproval {
                    key: 9,
                    observation: TransactionObservation::new(90, "approval-settled"),
                },
            ]))
            .expect("prepare a submitted row and an occupied observation key");

        let before = store.clone();
        let rejected = transaction(vec![StoreTransactionOp::SettleOutbox {
            effect: 7,
            outcome: "success",
            observation: TransactionObservation::new(90, "different-owner"),
        }]);
        assert!(store.commit(rejected).is_err());
        assert_eq!(store, before);
        assert_eq!(
            store.outbox(&7).map(TransactionOutbox::phase),
            Some(TransactionOutboxPhase::Submitted)
        );
        assert_eq!(store.outcome(&7), None);

        let settlement = transaction(vec![StoreTransactionOp::SettleOutbox {
            effect: 7,
            outcome: "success",
            observation: TransactionObservation::new(91, "outbox-settled"),
        }]);
        let receipt = store.commit(settlement.clone()).expect("atomic settlement");
        assert_eq!(store.outbox(&7), None);
        assert_eq!(store.outcome(&7), Some(&"success"));
        assert_eq!(
            store.observation_digest(&91),
            Some(TestSchema::observation_digest(&"outbox-settled"))
        );
        assert_eq!(store.commit(settlement).expect("settlement retry"), receipt);

        let settled = store.clone();
        let late_submit = transaction(vec![StoreTransactionOp::SubmitOutbox { effect: 7 }]);
        let receipt = store
            .commit(late_submit)
            .expect("historical submission retry is a state-preserving success");
        assert_eq!(receipt.refs(), &[StoreTransactionRef::OutboxSubmitted(7)]);
        assert_eq!(store, settled);
    }

    #[test]
    fn committed_outbox_can_be_cancelled_without_submission_but_submitted_cannot() {
        let mut store = recover_empty();
        store
            .commit(transaction(vec![StoreTransactionOp::OpenOutbox {
                effect: 21,
                request: "never-contacted",
            }]))
            .expect("committed outbox opens");

        let cancellation = transaction(vec![StoreTransactionOp::CancelCommittedOutbox {
            effect: 21,
            outcome: "cancelled-by-stop",
            observation: TransactionObservation::new(121, "outbox-cancelled"),
        }]);
        let receipt = store
            .commit(cancellation.clone())
            .expect("unsubmitted outbox is terminally cancelled");
        assert_eq!(
            receipt.refs(),
            &[
                StoreTransactionRef::OutboxCancelled(21),
                StoreTransactionRef::Outcome(21),
                StoreTransactionRef::Observation(121),
            ]
        );
        assert!(store.outbox(&21).is_none());
        assert_eq!(store.outcome(&21), Some(&"cancelled-by-stop"));
        assert_eq!(
            store
                .commit(cancellation)
                .expect("exact cancellation retry is idempotent"),
            receipt
        );

        store
            .commit(transaction(vec![
                StoreTransactionOp::OpenOutbox {
                    effect: 22,
                    request: "contacted",
                },
                StoreTransactionOp::AcquireOutboxDispatch { effect: 22 },
            ]))
            .expect("submitted control opens");
        let before = store.clone();
        let rejected = store
            .commit(transaction(vec![
                StoreTransactionOp::CancelCommittedOutbox {
                    effect: 22,
                    outcome: "must-not-be-relabeled",
                    observation: TransactionObservation::new(122, "wrong-terminal"),
                },
            ]))
            .expect_err("submitted custody needs ambiguity-aware settlement");
        assert!(matches!(
            rejected.reason(),
            StoreTransactionFailureReason::Rejected {
                operation: 0,
                reason: StoreTransactionReject::OutboxAlreadySubmitted { effect: 22 },
            }
        ));
        assert_eq!(store, before);
    }

    #[test]
    fn dispatch_acquire_is_one_shot_even_when_open_was_retried() {
        let mut store = recover_empty();
        let open = transaction(vec![StoreTransactionOp::OpenOutbox {
            effect: 11,
            request: "request",
        }]);
        store.commit(open.clone()).expect("first open");
        store.commit(open).expect("exact open retry");

        let acquire = transaction(vec![StoreTransactionOp::AcquireOutboxDispatch {
            effect: 11,
        }]);
        let receipt = store
            .commit(acquire.clone())
            .expect("the Committed row grants one dispatch acquisition");
        assert_eq!(
            receipt.refs(),
            &[StoreTransactionRef::OutboxDispatchAcquired(11)]
        );
        assert_eq!(
            store.outbox(&11).map(TransactionOutbox::phase),
            Some(TransactionOutboxPhase::Submitted)
        );

        let submitted = store.clone();
        let duplicate = store
            .commit(acquire)
            .expect_err("a Submitted row cannot mint a second dispatch permit");
        assert!(matches!(
            duplicate.reason(),
            StoreTransactionFailureReason::Rejected {
                operation: 0,
                reason: StoreTransactionReject::OutboxAlreadySubmitted { effect: 11 },
            }
        ));
        assert_eq!(store, submitted);
    }

    #[test]
    fn rejected_dispatch_acquire_rolls_back_companion_operations() {
        let mut store = recover_empty();
        store
            .commit(transaction(vec![
                StoreTransactionOp::OpenOutbox {
                    effect: 12,
                    request: "request",
                },
                StoreTransactionOp::AcquireOutboxDispatch { effect: 12 },
            ]))
            .expect("fixture row is already Submitted");
        let before = store.clone();

        let failure = store
            .commit(transaction(vec![
                StoreTransactionOp::Append(TransactionAppend::new(99, "must-roll-back")),
                StoreTransactionOp::AcquireOutboxDispatch { effect: 12 },
            ]))
            .expect_err("a duplicate dispatch acquisition rejects the whole transaction");
        assert!(matches!(
            failure.reason(),
            StoreTransactionFailureReason::Rejected {
                operation: 1,
                reason: StoreTransactionReject::OutboxAlreadySubmitted { effect: 12 },
            }
        ));
        assert_eq!(store, before);
        assert_eq!(store.record_digest(&99), None);
    }

    #[test]
    fn exact_open_submit_settle_batch_retry_is_a_noop_with_the_same_receipt() {
        let mut store = recover_empty();
        let batch = transaction(vec![
            StoreTransactionOp::OpenOutbox {
                effect: 8,
                request: "request",
            },
            StoreTransactionOp::SubmitOutbox { effect: 8 },
            StoreTransactionOp::SettleOutbox {
                effect: 8,
                outcome: "success",
                observation: TransactionObservation::new(80, "settled"),
            },
        ]);
        let first = store.commit(batch.clone()).expect("first batch");
        let settled = store.clone();
        let retry = store.commit(batch).expect("exact batch retry");
        assert_eq!(retry, first);
        assert_eq!(store, settled);
        assert_eq!(store.outbox(&8), None);
        assert_eq!(store.outcome(&8), Some(&"success"));
    }

    #[test]
    fn approval_custody_preserves_requested_and_approved_and_first_decision_wins() {
        let mut store = recover_empty();
        store
            .commit(transaction(vec![StoreTransactionOp::OpenApproval {
                key: 1,
                approval: "request",
            }]))
            .expect("request");
        assert_eq!(
            store.approval(&1).map(TransactionApproval::phase),
            Some(&TransactionApprovalPhase::Requested)
        );

        store
            .commit(transaction(vec![StoreTransactionOp::ApproveApproval {
                key: 1,
                ticket: "ticket-a",
            }]))
            .expect("approve");
        assert_eq!(
            store.approval(&1).map(TransactionApproval::phase),
            Some(&TransactionApprovalPhase::Approved("ticket-a"))
        );
        let approved = store.clone();
        assert!(matches!(
            store
                .commit(transaction(vec![StoreTransactionOp::ApproveApproval {
                    key: 1,
                    ticket: "ticket-b",
                }]))
                .expect_err("a second conflicting decision is rejected")
                .reason(),
            StoreTransactionFailureReason::Rejected {
                reason: StoreTransactionReject::ApprovalDecisionConflict { key: 1 },
                ..
            }
        ));
        assert_eq!(store, approved);

        let terminal_batch = transaction(vec![
            StoreTransactionOp::OpenApproval {
                key: 2,
                approval: "second-request",
            },
            StoreTransactionOp::ApproveApproval {
                key: 2,
                ticket: "ticket",
            },
            StoreTransactionOp::SettleApproval {
                key: 2,
                observation: TransactionObservation::new(2, "consumed"),
            },
        ]);
        let first = store
            .commit(terminal_batch.clone())
            .expect("approval terminal batch");
        let settled = store.clone();
        let retry = store
            .commit(terminal_batch)
            .expect("exact approval terminal retry");
        assert_eq!(retry, first);
        assert_eq!(store, settled);
        assert_eq!(store.approval(&2), None);
    }

    #[test]
    fn checkpoint_state_and_pending_replace_as_one_monotone_row() {
        let permutations = [
            [1, 2, 3],
            [1, 3, 2],
            [2, 1, 3],
            [2, 3, 1],
            [3, 1, 2],
            [3, 2, 1],
        ];
        for order in permutations {
            let mut store = recover_empty();
            for at in order {
                let state = match at {
                    1 => "one",
                    2 => "two",
                    3 => "three",
                    _ => unreachable!(),
                };
                let _ = store.commit(transaction(vec![StoreTransactionOp::ReplaceCheckpoint(
                    checkpoint(4, 8, at, state),
                )]));
            }
            let current = store.checkpoint(&4).expect("maximum checkpoint remains");
            assert_eq!(current.at(), &3);
            assert_eq!(current.state(), &"three");
            assert_eq!(current.pending(), &BTreeSet::from([3, 13]));

            let exact = transaction(vec![StoreTransactionOp::ReplaceCheckpoint(checkpoint(
                4, 8, 3, "three",
            ))]);
            let first = store.commit(exact.clone()).expect("exact latest retry");
            assert_eq!(store.commit(exact).expect("same retry"), first);

            let before = store.clone();
            let conflict = transaction(vec![StoreTransactionOp::ReplaceCheckpoint(
                TransactionCheckpoint::new(4, 8, 3, "conflict", [99]),
            )]);
            assert!(store.commit(conflict).is_err());
            assert_eq!(store, before);
        }
    }

    #[test]
    fn append_checkpoint_and_outcome_share_one_commit_point() {
        let mut store = recover_empty();
        store
            .commit(transaction(vec![
                StoreTransactionOp::OpenOutbox {
                    effect: 5,
                    request: "request",
                },
                StoreTransactionOp::SubmitOutbox { effect: 5 },
                StoreTransactionOp::OpenApproval {
                    key: 1,
                    approval: "approval",
                },
                StoreTransactionOp::SettleApproval {
                    key: 1,
                    observation: TransactionObservation::new(1, "occupied"),
                },
            ]))
            .expect("fixture setup");

        let before = store.clone();
        let rejected = transaction(vec![
            StoreTransactionOp::Append(TransactionAppend::new(4, "boundary")),
            StoreTransactionOp::ReplaceCheckpoint(checkpoint(3, 9, 6, "state")),
            StoreTransactionOp::SettleOutbox {
                effect: 5,
                outcome: "outcome",
                observation: TransactionObservation::new(1, "conflict"),
            },
        ]);
        assert!(store.commit(rejected).is_err());
        assert_eq!(store, before);
        assert_eq!(store.record_digest(&4), None);
        assert_eq!(store.checkpoint(&3), None);
        assert_eq!(store.outcome(&5), None);

        let committed = transaction(vec![
            StoreTransactionOp::Append(TransactionAppend::new(4, "boundary")),
            StoreTransactionOp::ReplaceCheckpoint(checkpoint(3, 9, 6, "state")),
            StoreTransactionOp::SettleOutbox {
                effect: 5,
                outcome: "outcome",
                observation: TransactionObservation::new(2, "settled"),
            },
        ]);
        store.commit(committed).expect("all three changes commit");
        assert_eq!(
            store.record_digest(&4),
            Some(TestSchema::record_digest(&"boundary"))
        );
        assert_eq!(
            store.checkpoint(&3).map(TransactionCheckpoint::at),
            Some(&6)
        );
        assert_eq!(store.outcome(&5), Some(&"outcome"));
    }

    #[test]
    fn recovery_is_idempotent_and_decomposes_the_three_subjects() {
        let mut store = recover_empty();
        store
            .commit(transaction(vec![
                StoreTransactionOp::OpenOutbox {
                    effect: 4,
                    request: "committed",
                },
                StoreTransactionOp::OpenOutbox {
                    effect: 5,
                    request: "submitted",
                },
                StoreTransactionOp::SubmitOutbox { effect: 5 },
                StoreTransactionOp::OpenApproval {
                    key: 6,
                    approval: "requested",
                },
                StoreTransactionOp::OpenApproval {
                    key: 8,
                    approval: "approved",
                },
                StoreTransactionOp::ApproveApproval {
                    key: 8,
                    ticket: "ticket",
                },
                StoreTransactionOp::ReplaceCheckpoint(checkpoint(7, 11, 8, "current")),
                StoreTransactionOp::ReplaceCheckpoint(checkpoint(9, 12, 10, "stale-owner")),
            ]))
            .expect("open all three custody subjects");

        let owners = CheckpointOwners::try_new([(7, 11)]).expect("one current owner");
        let recovered = store
            .reopen()
            .recover(&owners, |effect, _| {
                Ok::<_, Infallible>(RecoveryTerminal::new(
                    "terminal",
                    TransactionObservation::new(100 + *effect, "terminal-observation"),
                ))
            })
            .expect("recovery succeeds");
        assert_eq!(recovered.plan().resubmit_outbox(), &[4]);
        assert_eq!(recovered.plan().terminal_outbox(), &[5]);
        assert_eq!(recovered.plan().preserve_approvals(), &[6, 8]);
        assert_eq!(
            recovered
                .store()
                .approval(&6)
                .map(TransactionApproval::phase),
            Some(&TransactionApprovalPhase::Requested)
        );
        assert_eq!(
            recovered
                .store()
                .approval(&8)
                .map(TransactionApproval::phase),
            Some(&TransactionApprovalPhase::Approved("ticket"))
        );
        assert_eq!(recovered.plan().restore_checkpoints(), &[(7, 8)]);
        assert_eq!(recovered.plan().withhold_checkpoints(), &[(9, 10)]);
        assert_eq!(recovered.store().outbox(&5), None);
        assert_eq!(recovered.store().outcome(&5), Some(&"terminal"));

        let (once, _) = recovered.into_parts();
        let twice = once
            .clone()
            .reopen()
            .recover(
                &owners,
                |_, _| -> Result<RecoveryTerminal<TestSchema>, Infallible> {
                    panic!("a settled Submitted row cannot be terminalized twice")
                },
            )
            .expect("second recovery is a fixed point");
        assert!(twice.plan().terminal_outbox().is_empty());
        assert_eq!(twice.store(), &once);
    }

    #[test]
    fn failed_recovery_does_not_publish_earlier_terminal_settlements() {
        let mut store = recover_empty();
        store
            .commit(transaction(vec![
                StoreTransactionOp::OpenOutbox {
                    effect: 1,
                    request: "first",
                },
                StoreTransactionOp::SubmitOutbox { effect: 1 },
                StoreTransactionOp::OpenOutbox {
                    effect: 2,
                    request: "second",
                },
                StoreTransactionOp::SubmitOutbox { effect: 2 },
            ]))
            .expect("two submitted rows");

        let error = store
            .reopen()
            .recover(&CheckpointOwners::default(), |effect, _| {
                if *effect == 2 {
                    return Err("injected terminal materialization failure");
                }
                Ok(RecoveryTerminal::new(
                    "terminal",
                    TransactionObservation::new(100 + *effect, "terminal-observation"),
                ))
            })
            .expect_err("second row aborts the whole recovery");
        assert!(matches!(
            error.failure(),
            TransactionRecoveryFailure::TerminalOutcome {
                effect: 2,
                source: "injected terminal materialization failure",
            }
        ));

        let mut materialized = Vec::new();
        let recovered = error
            .into_recovering()
            .recover(&CheckpointOwners::default(), |effect, _| {
                materialized.push(*effect);
                Ok::<_, Infallible>(RecoveryTerminal::new(
                    "terminal",
                    TransactionObservation::new(100 + *effect, "terminal-observation"),
                ))
            })
            .expect("retry sees both original submitted rows");
        assert_eq!(materialized, vec![1, 2]);
        assert_eq!(recovered.plan().terminal_outbox(), &[1, 2]);
        assert_eq!(recovered.store().outcome(&1), Some(&"terminal"));
        assert_eq!(recovered.store().outcome(&2), Some(&"terminal"));
    }

    #[test]
    fn every_commit_crash_window_converges_by_exact_retry() {
        #[derive(Clone, Copy)]
        enum FaultPoint {
            BeforeCommit,
            AfterCommitBeforeReceipt,
            AfterReceipt,
        }

        let base = recover_empty();
        let change = transaction(vec![
            StoreTransactionOp::Append(TransactionAppend::new(1, "record")),
            StoreTransactionOp::ReplaceCheckpoint(checkpoint(3, 4, 5, "state")),
            StoreTransactionOp::OpenOutbox {
                effect: 6,
                request: "request",
            },
        ]);
        let mut canonical = base.clone();
        let expected_receipt = canonical
            .commit(change.clone())
            .expect("canonical commit receipt");

        for fault in [
            FaultPoint::BeforeCommit,
            FaultPoint::AfterCommitBeforeReceipt,
            FaultPoint::AfterReceipt,
        ] {
            let mut crashed = base.clone();
            if !matches!(fault, FaultPoint::BeforeCommit) {
                let _lost_receipt = crashed
                    .commit(change.clone())
                    .expect("commit happened before the simulated crash");
            }
            let (mut reopened, _) = crashed
                .reopen()
                .recover(&CheckpointOwners::default(), |_, _| {
                    Err::<RecoveryTerminal<TestSchema>, _>("no submitted row")
                })
                .expect("recovery does not invent work")
                .into_parts();
            let retried = reopened
                .commit(change.clone())
                .expect("same logical transaction retries safely");
            assert_eq!(retried, expected_receipt);
            assert_eq!(reopened, canonical);
        }
    }

    #[test]
    fn the_in_place_journal_fold_lands_exactly_where_the_copying_commit_landed() {
        let sequence = vec![
            transaction(vec![
                StoreTransactionOp::Append(TransactionAppend::new(1, "record-one")),
                StoreTransactionOp::AppendObservation(TransactionObservation::new(10, "opened")),
            ]),
            transaction(vec![
                StoreTransactionOp::OpenOutbox {
                    effect: 7,
                    request: "request",
                },
                StoreTransactionOp::SubmitOutbox { effect: 7 },
            ]),
            transaction(vec![StoreTransactionOp::SettleOutbox {
                effect: 7,
                outcome: "success",
                observation: TransactionObservation::new(12, "settled"),
            }]),
            transaction(vec![
                StoreTransactionOp::OpenApproval {
                    key: 9,
                    approval: "approval",
                },
                StoreTransactionOp::ApproveApproval {
                    key: 9,
                    ticket: "ticket",
                },
            ]),
            transaction(vec![StoreTransactionOp::SettleApproval {
                key: 9,
                observation: TransactionObservation::new(13, "approved"),
            }]),
            transaction(vec![StoreTransactionOp::ReplaceCheckpoint(checkpoint(
                4, 5, 6, "state",
            ))]),
            transaction(vec![StoreTransactionOp::SettleCheckpoint {
                actor: 4,
                at: 6,
                observation: TransactionObservation::new(14, "checkpointed"),
            }]),
            transaction(vec![StoreTransactionOp::Append(TransactionAppend::new(
                2,
                "record-two",
            ))]),
        ];

        let mut copying = recover_empty();
        let mut in_place = recover_empty();
        for (index, change) in sequence.into_iter().enumerate() {
            let expected = copying
                .commit(change.clone())
                .unwrap_or_else(|_| panic!("oracle fold accepts commit {index}"));
            let (next, actual) = in_place
                .fold_committed(change)
                .unwrap_or_else(|_| panic!("in-place fold accepts commit {index}"));
            in_place = next;
            assert_eq!(
                actual, expected,
                "receipt of commit {index} differs between the two folds"
            );
            assert_eq!(
                in_place, copying,
                "custody image after commit {index} differs between the two folds"
            );
        }
    }

    #[test]
    fn the_in_place_journal_fold_names_the_same_rejected_operation_as_the_copying_commit() {
        let opening = transaction(vec![StoreTransactionOp::AppendObservation(
            TransactionObservation::new(7, "lifecycle"),
        )]);
        let conflicting = transaction(vec![
            StoreTransactionOp::Append(TransactionAppend::new(1, "record")),
            StoreTransactionOp::AppendObservation(TransactionObservation::new(7, "different")),
        ]);

        let mut copying = recover_empty();
        copying.commit(opening.clone()).expect("oracle opens");
        let expected = copying
            .commit(conflicting.clone())
            .expect_err("the second operation conflicts");

        let in_place = recover_empty()
            .fold_committed(opening)
            .expect("in-place fold opens")
            .0;
        let actual = in_place
            .fold_committed(conflicting)
            .err()
            .expect("the second operation conflicts");

        assert_eq!(actual.transaction(), expected.transaction());
        assert!(
            matches!(
                actual.reason(),
                StoreTransactionFailureReason::Rejected {
                    operation: 1,
                    reason: StoreTransactionReject::ObservationConflict { key: 7 },
                }
            ),
            "in-place rejection must name operation 1 and the observation conflict"
        );
        assert!(matches!(
            expected.reason(),
            StoreTransactionFailureReason::Rejected {
                operation: 1,
                reason: StoreTransactionReject::ObservationConflict { key: 7 },
            }
        ));
    }
}
