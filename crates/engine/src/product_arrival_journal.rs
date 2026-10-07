//! Product delivery and observation journal backed by the existing transaction/group-commit chain.
//!
//! Each receiving actor supplies its arrival stamp and ordinal. The writer returns a receipt
//! after SQLite FULL-sync. Producers enqueue emission bodies on the same FIFO before fanout. The
//! accepted `32 / 2 ms` group policy is injected unchanged.
//!
//! Observation payloads still use the provisional product `Value` encoding. The journal treats
//! them as opaque record bytes, so this on-disk representation may change after beta when the
//! canonical Value codec replaces it.

use crate::arrival_commit::{
    ArrivalCommitError, ArrivalCommitReceipt, EffectArrivalCommit, EventArrivalCommit,
};
use circular_actors::ProductPayload;
use circular_core::{ArrivalIndex, Boundary, Ceilings, EncodedPayload, EventId, PayloadVersionTag};
use circular_plan::{ActorId, NamedActorId};
use circular_runtime::ArrivalOrigin as RuntimeArrivalOrigin;
use circular_store::{
    ACCEPTED_GROUP_COMMIT, AppendBatch, AppendResult, ArrivalOrigin, ArrivalProjection,
    BoundaryFact, BoundaryKey, BoundaryRecord, CheckpointOwners, ClassKey, ColumnWriteContext,
    GroupCommitCoordinator, GroupCommitPolicy, JournalProjection, MemoryStore, OpaqueId,
    PagePolicy, ProductJournalCodec, ProductRecordCodec, ProductStore, ProductTransaction, Record,
    RecordOrigin, RecoveringSqliteTransactionStore, RecoveryTerminal, Store, StoreTransaction,
    StoreTransactionOp, StoreTransactionRef, StreamId, SubmissionOutcome, TransactionAppend,
    arrival_transaction, encode_record,
};
use std::collections::BTreeMap;
#[cfg(test)]
use std::convert::Infallible;
use std::path::Path;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicU64, AtomicUsize, Ordering},
    mpsc,
};
use std::time::Instant;

pub const PRODUCT_STORE_PAGE_MAXIMUM: std::num::NonZeroUsize =
    std::num::NonZeroUsize::new(64).expect("the product store page ceiling is positive");

/// One run journal's physical and logical growth.
///
/// `bytes` counts retained transaction bytes in this run namespace. `records` is
/// the ProductStore record-key prefix, so observation records count beside
/// arrivals instead of disappearing inside one group-commit transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProductArrivalJournalUsage {
    pub bytes: u64,
    pub records: u64,
}

pub(crate) struct ArrivalBootFold {
    journal: circular_store::ProductJournalFold,
    records: Option<(Vec<Record<ProductStore>>, Vec<u64>)>,
}

impl ArrivalBootFold {
    fn new(collect_records: bool) -> Self {
        Self::after_horizon(collect_records, Path::new(""), 0)
    }

    fn after_horizon(collect_records: bool, path: &Path, horizon: u64) -> Self {
        let page = PRODUCT_STORE_PAGE_MAXIMUM;
        Self {
            journal: circular_store::ProductJournalFold::after_horizon(
                crate::state_journal::ARRIVAL_JOURNAL_NAMESPACE.to_owned(),
                PagePolicy::new(page, page).expect("accepted page policy"),
                path.to_path_buf(),
                horizon,
            ),
            records: collect_records.then(|| (Vec::new(), Vec::new())),
        }
    }

    fn extend(&mut self, snapshot: &circular_store::SqliteJournalSnapshot) -> Result<(), String> {
        let Self { journal, records } = self;
        let mut observe = |commit: u64, record: &Record<ProductStore>| -> Result<(), String> {
            if let Some((rows, commits)) = records.as_mut() {
                rows.push(record.clone());
                commits.push(commit);
            }
            Ok(())
        };
        journal.extend(
            snapshot,
            &mut circular_store::JournalFoldHooks {
                facts: &mut |commit, transaction| checkpoint_facts(commit, transaction),
                record: &mut observe,
            },
        )
    }

    fn into_writer(self, next_record_key: u64) -> (MemoryStore<ProductStore>, u64) {
        let next_record_key = next_record_key.max(self.journal.next_record_key());
        (self.journal.into_published(), next_record_key)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArrivalRecorderStop {
    reason: ArrivalCommitError,
}

impl ArrivalRecorderStop {
    #[must_use]
    pub const fn code(&self) -> circular_protocol::rejection_code::RejectionReason {
        circular_protocol::rejection_code::RejectionReason::ArrivalRecorderStopped
    }

    #[must_use]
    pub const fn reason(&self) -> &ArrivalCommitError {
        &self.reason
    }
}

#[derive(Clone)]
pub struct ProductDurableArrivalJournal {
    run: StreamId,
    journal_prefix: Option<Arc<circular_store::JournalPrefix>>,
    path: Arc<std::path::PathBuf>,
    sender: WriterSender,
    prefix: Arc<arc_swap::ArcSwap<MemoryStore<ProductStore>>>,
    record_count: Arc<AtomicU64>,
    checkpoint_horizon: Arc<AtomicU64>,
    /// This run namespace's retained bytes, republished by the writer after every
    /// group commit. The writer's journal handle carries the value forward, so
    /// neither an injection nor a publication reads the file to learn the size.
    namespace_bytes: Arc<AtomicU64>,
    recorder_stop: Arc<OnceLock<ArrivalRecorderStop>>,
    drained_through: Arc<AtomicUsize>,
    record_wake: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>,
    commit_boundary: Arc<std::sync::RwLock<()>>,
}

/// One producer's writing hand: every submission stamped by this producer goes
/// through it. The context owns the producer identity and its committed bases.
#[derive(Clone)]
pub struct ColumnJournal {
    context: ColumnWriteContext,
    journal: ProductDurableArrivalJournal,
}

#[derive(Clone)]
pub struct CommitBoundary {
    gate: Arc<std::sync::RwLock<()>>,
    prefix: Arc<arc_swap::ArcSwap<MemoryStore<ProductStore>>>,
    checkpoint_horizon: Arc<AtomicU64>,
}

/// A sealed checkpoint prefix read from the existing transaction journal.
#[derive(Clone)]
pub struct ProductCheckpointReader {
    path: Arc<std::path::PathBuf>,
    run: StreamId,
    last_checkpoint: Option<u64>,
    boundary: Option<CommitBoundary>,
}
impl ProductCheckpointReader {
    pub fn published_prefix(&self) -> Option<Arc<MemoryStore<ProductStore>>> {
        self.boundary
            .as_ref()
            .map(|boundary| boundary.prefix.load_full())
    }

    #[must_use]
    pub fn published_view(&self) -> Option<circular_store::JournalView> {
        self.published_prefix()
            .map(circular_store::JournalView::memory)
    }

    pub fn at(
        path: std::path::PathBuf,
        run: StreamId,
        checkpoints: &[circular_store::TransactionCheckpoint<ProductTransaction>],
    ) -> Self {
        Self {
            path: Arc::new(path),
            run,
            last_checkpoint: checkpoints.iter().map(|row| row.at().get()).max(),
            boundary: None,
        }
    }

    /// Reads transaction history on demand; no live checkpoint history is cached.
    pub fn read(
        &self,
    ) -> Result<Vec<circular_store::TransactionCheckpoint<ProductTransaction>>, String> {
        let snapshot = circular_store::SqliteJournal::read_only_namespace(
            self.path.as_path(),
            crate::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
        )
        .map_err(|e| e.to_string())?;
        checkpoint_rows(&snapshot).map(|rows| {
            rows.into_iter()
                .filter(|row| {
                    self.last_checkpoint
                        .is_some_and(|last| row.at().get() <= last)
                })
                .collect()
        })
    }
}

fn checkpoint_rows(
    snapshot: &circular_store::SqliteJournalSnapshot,
) -> Result<Vec<CheckpointRow>, String> {
    let mut rows = Vec::new();
    let mut reader = circular_store::ProductJournalReader::default();
    for entry in snapshot.entries() {
        let transaction = reader
            .read_entry(entry)
            .map_err(|e| format!("checkpoint history: {e}"))?;
        for operation in transaction.operations() {
            if let StoreTransactionOp::ReplaceCheckpoint(row) = operation {
                rows.push(row.clone());
            }
        }
    }
    Ok(rows)
}

pub fn published_arrivals(
    snapshot: &circular_store::SqliteJournalSnapshot,
) -> Result<MemoryStore<ProductStore>, String> {
    let page = PRODUCT_STORE_PAGE_MAXIMUM;
    circular_store::rehydrate_published_arrivals(
        snapshot,
        &ProductJournalCodec,
        &ArrivalProjection::new(),
        PagePolicy::new(page, page).expect("accepted page policy"),
        &mut |commit, transaction| checkpoint_facts(commit, transaction),
    )
    .map_err(|error| format!("product arrival read-only rehydrate failed: {error}"))
}

/// An owned read-only journal prefix with no arrival submission authority.
pub struct ProductReadOnlyArrivalJournal {
    records: Box<[Record<ProductStore>]>,
    commits: Box<[u64]>,
    checkpoint_facts: circular_store::RecordSegments<ProductStore>,
    applied_through: u64,
    custody: circular_store::ProductCustodySnapshot,
    checkpoints: Vec<CheckpointRow>,
    horizon: u64,
}

impl ProductReadOnlyArrivalJournal {
    #[must_use]
    pub const fn horizon(&self) -> u64 {
        self.horizon
    }

    /// Immutable transaction history, including superseded config handoffs.
    pub fn checkpoints(&self) -> &[circular_store::TransactionCheckpoint<ProductTransaction>] {
        &self.checkpoints
    }

    pub fn custody(&self) -> &circular_store::ProductCustodySnapshot {
        &self.custody
    }

    #[must_use]
    pub fn records(&self) -> &[Record<ProductStore>] {
        &self.records
    }

    #[must_use]
    pub fn into_records(self) -> Vec<Record<ProductStore>> {
        self.records.into_vec()
    }

    pub fn publish_into(&self, store: &mut MemoryStore<ProductStore>) -> Result<(), String> {
        if !self.records.is_empty() {
            let batch = AppendBatch::try_new(self.records.to_vec())
                .and_then(|batch| batch.with_commits(self.commits.to_vec()))
                .map_err(|e| format!("{e:?}"))?;
            if !matches!(
                store.append(batch),
                circular_store::AppendResult::Committed(_)
            ) {
                return Err("restored prefix append failed".to_owned());
            }
        }
        for row in self.checkpoint_facts.rows() {
            store.push_checkpoint_fact(row.record().into_owned(), row.commit());
        }
        store.note_commit(self.applied_through);
        Ok(())
    }
}

pub(crate) enum ApprovalMutation {
    Open(circular_runtime::EffectId, EncodedPayload),
    Approve(circular_runtime::EffectId),
    Settle(circular_runtime::EffectId, circular_store::ApprovalTerminal),
}

type CheckpointRow = circular_store::TransactionCheckpoint<ProductTransaction>;

#[derive(Clone)]
struct WriterSender(mpsc::Sender<WriterCommand>);
impl WriterSender {
    fn send(
        &self,
        command: Command,
        context: Option<ColumnWriteContext>,
    ) -> Result<(), mpsc::SendError<WriterCommand>> {
        let shutdown = matches!(&command, Command::Records(records, _, _)
            if matches!(records.as_ref(), [Record::Observation(row)]
                if matches!(row.header().key(), ClassKey::Observation(circular_store::ObservationKey::StreamItem(_, _, item))
                    if *item.kind() == circular_core::BuiltinObservationName::DaemonShutdown)));
        self.0.send(if shutdown {
            WriterCommand::Shutdown(command, context)
        } else {
            WriterCommand::Append(command, context)
        })
    }
}
enum WriterCommand {
    /// Carries System's finished submission, never stamp material. It is the
    /// final transaction; later queued commands cannot follow its exit fact.
    Shutdown(Command, Option<ColumnWriteContext>),
    Close(mpsc::SyncSender<Result<(), String>>),
    /// Every Append-bearing command comes from a [`ColumnJournal`] submission
    /// with that producer's own context. `None` is only the approval and
    /// checkpoint custody operations, which carry no Append.
    Append(Command, Option<ColumnWriteContext>),
}

pub(crate) type ArrivalAnswer = Result<ArrivalCommitReceipt, ArrivalCommitError>;
type ArrivalReply = tokio::sync::oneshot::Sender<ArrivalAnswer>;

enum Command {
    EmissionBody {
        at: circular_core::Stamp<ActorId>,
        body: circular_store::EmissionFact,
    },
    Checkpoint(
        CheckpointRow,
        mpsc::SyncSender<Result<CheckpointRow, String>>,
    ),
    Approval(ApprovalMutation, RecordsReply),
    Event(Box<EventArrivalCommit>, ArrivalReply),
    Effect(Box<EffectArrivalCommit>, ArrivalReply),
    Records(
        Box<[Record<ProductStore>]>,
        Vec<(usize, EncodedPayload)>,
        RecordsReply,
    ),
}

pub(crate) enum RecordsReply {
    #[cfg(test)]
    Blocking(mpsc::SyncSender<Result<(), String>>),
    Task(tokio::sync::oneshot::Sender<Result<(), String>>),
}

impl RecordsReply {
    fn send(self, result: Result<(), String>) {
        match self {
            #[cfg(test)]
            Self::Blocking(sender) => {
                let _ = sender.send(result);
            }
            Self::Task(sender) => {
                let _ = sender.send(result);
            }
        }
    }
}

impl ProductDurableArrivalJournal {
    /// Assembly opens a producer's writing hand for a life: a new segment of
    /// its column, starting at the first submission's CURRENT keyframe. Nothing
    /// is read from the journal; later submissions carry this context.
    pub fn open_column(&self, producer: &ActorId) -> Result<ColumnJournal, String> {
        let context = ColumnWriteContext::new(producer).map_err(|error| format!("{error:?}"))?;
        Ok(ColumnJournal {
            context,
            journal: self.clone(),
        })
    }

    pub fn checkpoint_reader(&self) -> ProductCheckpointReader {
        ProductCheckpointReader {
            path: self.path.clone(),
            run: self.run,
            last_checkpoint: self
                .checkpoint_horizon
                .load(Ordering::Acquire)
                .checked_sub(1),
            boundary: Some(CommitBoundary {
                gate: self.commit_boundary.clone(),
                prefix: self.prefix.clone(),
                checkpoint_horizon: self.checkpoint_horizon.clone(),
            }),
        }
    }

    pub fn set_record_wake(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        *self.record_wake.lock().expect("record wake") = Some(wake);
    }

    pub(crate) fn inherit_checkpoint(
        &self,
        actor: ActorId,
        owner: circular_store::IncarnationId,
        body: EncodedPayload,
    ) -> Result<CheckpointRow, String> {
        let (reply, answer) = mpsc::sync_channel(0);
        let row = CheckpointRow::new(actor, owner, OpaqueId::new(0), body, []);
        self.send_command(Command::Checkpoint(row, reply), None)
            .map_err(|(_, error)| format!("checkpoint commit failed: {error}"))?;
        answer
            .recv()
            .map_err(|_| self.closed_reason().to_string())?
    }

    pub(crate) fn submit_approval(
        &self,
        mutation: ApprovalMutation,
    ) -> tokio::sync::oneshot::Receiver<Result<(), String>> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        self.send_with_reply(Command::Approval(mutation, RecordsReply::Task(reply)), None);
        answer
    }

    pub(crate) fn approval_prefix(
        &self,
    ) -> Result<
        (
            circular_store::SqliteJournal,
            circular_store::SqliteJournalSnapshot,
        ),
        String,
    > {
        let source = circular_store::SqliteJournal::open_read_only_namespace(
            self.path.as_path(),
            crate::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
        )
        .map_err(|e| e.to_string())?;
        let snapshot = circular_store::SqliteJournal::read_only_namespace_after(
            self.path.as_path(),
            crate::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
            self.horizon(),
        )
        .map_err(|e| e.to_string())?;
        Ok((source, snapshot))
    }

    #[must_use]
    pub fn horizon(&self) -> u64 {
        self.journal_prefix
            .as_ref()
            .map_or(0, |prefix| prefix.through())
    }

    /// Open the state journal without creating a writer thread
    /// or retaining any writable SQLite handle. It folds the journal once from its first commit.
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<ProductReadOnlyArrivalJournal, String> {
        Self::open_read_only_after(path, 0)
    }

    pub fn open_read_only_after(
        path: impl AsRef<Path>,
        horizon: u64,
    ) -> Result<ProductReadOnlyArrivalJournal, String> {
        let path = path.as_ref();
        let snapshot = circular_store::SqliteJournal::read_only_namespace_after(
            path,
            crate::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
            horizon,
        )
        .map_err(|error| format!("product arrival read-only open failed: {error}"))?;
        let mut fold = ArrivalBootFold::after_horizon(true, path, horizon);
        if horizon > 0 {
            prior_manifest(path, horizon)?;
            fold.journal.note_prior_manifest();
        }
        fold.extend(&snapshot)
            .map_err(|error| format!("product arrival read-only rehydrate failed: {error}"))?;
        drop(snapshot);
        let custody = fold.journal.custody()?;
        let checkpoints = fold.journal.checkpoints().to_vec();
        let (records, commits) = fold.records.take().unwrap_or_default();
        Ok(ProductReadOnlyArrivalJournal {
            custody,
            checkpoints,
            records: records.into_boxed_slice(),
            commits: commits.into_boxed_slice(),
            checkpoint_facts: fold.journal.published().checkpoint_facts().clone(),
            applied_through: fold.journal.published().applied_through(),
            horizon,
        })
    }

    /// Open the writer without committing records. Assembly gives System its
    /// issuer and column; only System's first submission starts the stream.
    /// The writer folds the journal here, once, from its first commit; boot recovery reads the
    /// prefix it publishes ([`Self::read_prefix`]) instead of folding the journal again.
    pub fn open_unseeded(path: impl AsRef<Path>, run: StreamId) -> Result<Self, String> {
        let path = path.as_ref();
        let mut fold = ArrivalBootFold::new(false);
        let (journal, tail) = circular_store::SqliteJournal::open_namespace_after(
            path,
            crate::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
            fold.journal.through(),
        )
        .map_err(|error| format!("product arrival journal open failed: {error}"))?;
        fold.extend(&tail)
            .map_err(|error| format!("product arrival rehydrate failed: {error}"))?;
        drop(tail);
        let horizon = fold.journal.horizon();
        let mut next_record_key = fold.journal.next_record_key();
        let model = fold
            .journal
            .take_model()
            .ok_or("boot fold's transaction model was already taken")?;
        let recovering =
            RecoveringSqliteTransactionStore::from_recovered(journal, ProductJournalCodec, model);
        let owners = CheckpointOwners::try_new(std::iter::empty())
            .map_err(|e| format!("checkpoint recovery owners: {e:?}"))?;
        let recovered = recovering
            .recover_outboxes(
                &owners,
                |_, _| -> Result<Option<RecoveryTerminal<ProductTransaction>>, String> {
                    Err("an open outbox has no owner in this kernel".into())
                },
            )
            .map_err(|error| format!("product arrival recovery failed: {:?}", error.failure()))?;
        let (store, recovery_plan) = recovered.into_parts();
        if !recovery_plan.withhold_checkpoints().is_empty() {
            return Err("append-only arrival journal produced a custody recovery plan".to_owned());
        }

        let mut store = store;
        let snapshot = store.path().to_path_buf();
        let retained_bytes = store
            .retained_namespace_bytes()
            .ok_or_else(|| "arrival journal store routes no namespace".to_owned())?;
        let committed = store
            .journal_snapshot_since(fold.journal.through())
            .map_err(|error| format!("product arrival snapshot failed: {error:?}"))?;
        fold.extend(&committed)
            .map_err(|error| format!("product arrival rehydrate failed: {error}"))?;
        drop(committed);
        let (mut rehydrated, following_key) = fold.into_writer(next_record_key);
        next_record_key = following_key;
        rehydrated.seal_all();
        let drained_through = Arc::new(AtomicUsize::new(rehydrated.records().len()));
        let prefix = Arc::new(arc_swap::ArcSwap::from_pointee(
            rehydrated.published_prefix(),
        ));
        let record_count = Arc::new(AtomicU64::new(next_record_key));
        let checkpoint_horizon = Arc::new(AtomicU64::new(next_record_key));
        let namespace_bytes = Arc::new(AtomicU64::new(retained_bytes));
        let (sender, receiver) = mpsc::channel();
        let path = Arc::new(snapshot);
        let commit_boundary = Arc::new(std::sync::RwLock::new(()));
        let recorder_stop = Arc::new(OnceLock::new());
        let record_wake: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>> =
            Arc::new(Mutex::new(None));
        let journal_prefix = (horizon > 0).then(|| journal_prefix(&path, horizon));
        let shared = SharedWriterState {
            prefix: prefix.clone(),
            record_count: record_count.clone(),
            checkpoint_horizon: checkpoint_horizon.clone(),
            namespace_bytes: namespace_bytes.clone(),
            commit_boundary: commit_boundary.clone(),
            recorder_stop: recorder_stop.clone(),
            record_wake: record_wake.clone(),
        };
        std::thread::Builder::new()
            .name("circular-arrival-group-commit".to_owned())
            .spawn(move || {
                writer_loop(receiver, run, store, rehydrated, next_record_key, shared);
            })
            .map_err(|error| {
                format!(
                    "product arrival writer thread failed for {}: {error}",
                    path.display()
                )
            })?;
        let journal = Self {
            run,
            journal_prefix,
            path,
            sender: WriterSender(sender),
            prefix,
            record_count,
            checkpoint_horizon,
            namespace_bytes,
            recorder_stop,
            drained_through,
            record_wake,
            commit_boundary,
        };
        Ok(journal)
    }

    /// Read this run's namespace size and record-key prefix.
    ///
    /// Both are values the writer carries forward as it appends, so this answer
    /// costs two atomic loads. It measures only this run's namespace, never the
    /// shared file's other runs or authoring.
    #[must_use]
    pub fn usage(&self) -> ProductArrivalJournalUsage {
        ProductArrivalJournalUsage {
            bytes: self.namespace_bytes.load(Ordering::Acquire),
            records: self.record_count.load(Ordering::Acquire),
        }
    }

    pub fn file_bytes(&self) -> Result<u64, String> {
        crate::state_journal::state_journal_file_bytes(&self.path)
    }

    pub(crate) fn source_custody_root(&self) -> &Path {
        self.path.parent().expect("owned journal directory")
    }

    /// Drain submitted commands and close the writer without issuing a fact.
    /// Product shutdown instead closes with System's final stamped submission.
    pub fn close(&self) -> Result<(), String> {
        if let Some(stop) = self.recorder_stop() {
            return Err(stop.reason().to_string());
        }
        let (reply, answer) = mpsc::sync_channel(0);
        self.sender
            .0
            .send(WriterCommand::Close(reply))
            .map_err(|_| self.closed_reason().to_string())?;
        answer
            .recv()
            .map_err(|_| self.closed_reason().to_string())?
    }

    #[must_use]
    pub fn recorder_stop(&self) -> Option<ArrivalRecorderStop> {
        self.recorder_stop.get().cloned()
    }

    /// The writer publishes only a sealed, committed journal prefix. Reads take no lock.
    pub fn read_prefix(&self) -> Arc<MemoryStore<ProductStore>> {
        self.prefix.load_full()
    }

    #[must_use]
    pub fn read_view(&self) -> circular_store::JournalView {
        match &self.journal_prefix {
            Some(prefix) => {
                circular_store::JournalView::with_prefix(self.read_prefix(), prefix.clone())
            }
            None => circular_store::JournalView::memory(self.read_prefix()),
        }
    }
}

#[cfg(test)]
impl ProductDurableArrivalJournal {
    /// Fixture seeding is unavailable to product callers.
    pub(crate) fn open(
        path: impl AsRef<Path>,
        run: StreamId,
        seed: Vec<Record<ProductStore>>,
    ) -> Result<Self, String> {
        let journal = Self::open_unseeded(path, run)?;
        if journal.usage().records == 0 {
            if !seed.is_empty() {
                journal.append_records(
                    AppendBatch::try_new(seed).map_err(|e| format!("seed: {e:?}"))?,
                )?;
            }
        } else if let Some(expected) = seed.iter().find_map(|record| match record {
            Record::Structure(structure) => match structure.fact() {
                circular_store::StructureFact::RunManifest(manifest) => {
                    Some(manifest.groups().revision().start())
                }
                _ => None,
            },
            _ => None,
        }) {
            if prior_manifest(&journal.path, u64::MAX)?
                .groups()
                .revision()
                .start()
                != expected
            {
                return Err("arrival journal manifest authoring cut disagrees with the expected starting cut".into());
            }
        }
        Ok(journal)
    }

    /// Each fixture submission uses its column owner's writing hand: the
    /// receiving actor for Arrival and Admission, otherwise the producer.
    pub(crate) fn append_records(&self, batch: AppendBatch<ProductStore>) -> Result<(), String> {
        let records: Vec<_> = batch.records().cloned().collect();
        for records in records
            .chunk_by(|a, b| circular_store::fixture_column(a) == circular_store::fixture_column(b))
        {
            self.open_column(circular_store::fixture_column(&records[0]))?
                .append_records(records.to_vec())?;
        }
        Ok(())
    }

    pub(crate) fn commit_event(
        &self,
        candidate: &EventArrivalCommit,
    ) -> Result<ArrivalCommitReceipt, ArrivalCommitError> {
        self.open_column(&candidate.actor().as_actor_id())
            .expect("test column opens")
            .submit_event(candidate.clone())
            .blocking_recv()
            .map_err(|_| ArrivalCommitError::CoordinatorGone)?
    }

    pub(crate) fn commit_effect(
        &self,
        candidate: &EffectArrivalCommit,
    ) -> Result<ArrivalCommitReceipt, ArrivalCommitError> {
        self.open_column(&candidate.actor().as_actor_id())
            .expect("test column opens")
            .submit_effect(candidate.clone())
            .blocking_recv()
            .map_err(|_| ArrivalCommitError::CoordinatorGone)?
    }
}

impl ProductDurableArrivalJournal {
    fn closed_reason(&self) -> ArrivalCommitError {
        self.recorder_stop()
            .map_or(ArrivalCommitError::CoordinatorGone, |stop| stop.reason)
    }

    fn send_command(
        &self,
        command: Command,
        context: Option<ColumnWriteContext>,
    ) -> Result<(), (Command, ArrivalCommitError)> {
        if let Some(stop) = self.recorder_stop() {
            return Err((command, stop.reason().clone()));
        }
        self.sender.send(command, context).map_err(|refused| {
            let (WriterCommand::Append(command, _) | WriterCommand::Shutdown(command, _)) =
                refused.0
            else {
                unreachable!("WriterSender::send wraps a submission")
            };
            (command, self.closed_reason())
        })
    }

    fn send_with_reply(&self, command: Command, context: Option<ColumnWriteContext>) {
        if let Err((command, error)) = self.send_command(command, context) {
            reply_failure(command, error);
        }
    }
}

impl ColumnJournal {
    #[cfg(test)]
    fn append_records(&self, records: Vec<Record<ProductStore>>) -> Result<(), String> {
        let (reply, answer) = mpsc::sync_channel(0);
        self.journal
            .send_command(
                Command::Records(
                    records.into_boxed_slice(),
                    Vec::new(),
                    RecordsReply::Blocking(reply),
                ),
                Some(self.context.clone()),
            )
            .map_err(|(_, error)| format!("product record append failed: {error}"))?;
        answer
            .recv()
            .map_err(|_| self.journal.closed_reason().to_string())?
    }

    /// Approval transactions are keyed by approval and use no column context.
    pub(crate) fn submit_approval(
        &self,
        mutation: ApprovalMutation,
    ) -> tokio::sync::oneshot::Receiver<Result<(), String>> {
        self.journal.submit_approval(mutation)
    }

    /// Status of the same writer queue used by this column.
    #[must_use]
    pub fn recorder_stop(&self) -> Option<ArrivalRecorderStop> {
        self.journal.recorder_stop()
    }

    /// The fixed custody path; this does not read the journal.
    pub(crate) fn source_custody_root(&self) -> &Path {
        self.journal.source_custody_root()
    }

    /// Queue a producer-owned emission before fanout; success is not a durable receipt. Every
    /// emission is recorded once, whether or not a wire leaves its outlet.
    pub fn submit_emission_body(
        &self,
        at: circular_core::Stamp<ActorId>,
        body: circular_store::EmissionFact,
    ) -> Result<(), ArrivalCommitError> {
        self.journal
            .sender
            .send(
                Command::EmissionBody { at, body },
                Some(self.context.clone()),
            )
            .map_err(|_| ArrivalCommitError::CoordinatorGone)
    }

    pub(crate) fn submit_event(
        &self,
        candidate: EventArrivalCommit,
    ) -> tokio::sync::oneshot::Receiver<ArrivalAnswer> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        self.journal.send_with_reply(
            Command::Event(Box::new(candidate), reply),
            Some(self.context.clone()),
        );
        answer
    }

    pub fn submit_records(
        &self,
        records: Vec<Record<ProductStore>>,
        live_bodies: Vec<(usize, EncodedPayload)>,
    ) -> tokio::sync::oneshot::Receiver<Result<(), String>> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        self.journal.send_with_reply(
            Command::Records(
                records.into_boxed_slice(),
                live_bodies,
                RecordsReply::Task(reply),
            ),
            Some(self.context.clone()),
        );
        answer
    }

    pub(crate) fn submit_effect(
        &self,
        candidate: EffectArrivalCommit,
    ) -> tokio::sync::oneshot::Receiver<ArrivalAnswer> {
        let (reply, answer) = tokio::sync::oneshot::channel();
        self.journal.send_with_reply(
            Command::Effect(Box::new(candidate), reply),
            Some(self.context.clone()),
        );
        answer
    }
}

struct PendingWrite {
    command: Command,
    transaction: Option<StoreTransaction<ProductTransaction>>,
    rows: Box<[circular_store::EncodedRow<ProductStore>]>,
    record_keys: Box<[OpaqueId]>,
    arrival: Option<PendingArrival>,
    restored: Vec<(usize, EncodedPayload)>,
}

struct PendingArrival {
    index: ArrivalIndex,
}

struct SharedWriterState {
    prefix: Arc<arc_swap::ArcSwap<MemoryStore<ProductStore>>>,
    record_count: Arc<AtomicU64>,
    checkpoint_horizon: Arc<AtomicU64>,
    namespace_bytes: Arc<AtomicU64>,
    commit_boundary: Arc<std::sync::RwLock<()>>,
    recorder_stop: Arc<OnceLock<ArrivalRecorderStop>>,
    record_wake: Arc<Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>,
}

fn stop_recording(shared: &SharedWriterState, reason: ArrivalCommitError) {
    let stop = ArrivalRecorderStop { reason };
    if shared.recorder_stop.set(stop.clone()).is_ok() {
        eprintln!(
            "circular: arrival recorder stopped — code {} ({}); this daemon records no further arrival: {}",
            stop.code().recorded_code(),
            stop.code().message(),
            stop.reason(),
        );
    }
}

pub(crate) fn checkpoint_facts(
    commit: u64,
    transaction: &StoreTransaction<ProductTransaction>,
) -> Result<Vec<Record<ProductStore>>, String> {
    let mut facts = Vec::new();
    for (index, operation) in transaction.operations().iter().enumerate() {
        let index = u32::try_from(index).map_err(|_| "checkpoint operation index overflow")?;
        if let Some(record) = crate::checkpoint_record(
            crate::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
            commit,
            index,
            operation,
        )? {
            facts.push(record);
        }
    }
    Ok(facts)
}

pub fn arrival_manifest(path: &Path) -> Result<circular_store::RunManifest<ProductStore>, String> {
    prior_manifest(path, u64::MAX)
}

pub(crate) fn prior_manifest(
    path: &Path,
    horizon: u64,
) -> Result<circular_store::RunManifest<ProductStore>, String> {
    let journal = circular_store::SqliteJournal::open_read_only_namespace(
        path,
        crate::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
    )
    .map_err(|error| format!("arrival journal manifest open: {error}"))?;
    let first = journal
        .namespace_first_commit()
        .map_err(|error| format!("arrival journal manifest read: {error}"))?
        .ok_or("arrival journal has no manifest commit")?;
    if first > horizon {
        return Err("arrival journal manifest is not before its checkpoint horizon".to_owned());
    }
    let (_, bytes) = journal
        .namespace_payload_at(first)
        .map_err(|error| format!("arrival journal manifest read: {error}"))?;
    let transaction = circular_store::SqliteTransactionCodec::decode(&ProductJournalCodec, &bytes)
        .map_err(|error| format!("arrival journal manifest commit: {error:?}"))?;
    ArrivalProjection::new()
        .project(&transaction)
        .map_err(|_| "arrival journal manifest commit does not project".to_owned())?
        .into_iter()
        .find_map(|record| match record {
            Record::Structure(structure) => match structure.fact() {
                circular_store::StructureFact::RunManifest(manifest) => Some(manifest.clone()),
                _ => None,
            },
            _ => None,
        })
        .ok_or_else(|| "arrival journal first commit is not this stream's manifest".to_owned())
}

#[must_use]
pub fn journal_prefix(path: &Path, through: u64) -> Arc<circular_store::JournalPrefix> {
    Arc::new(circular_store::JournalPrefix::new(
        path,
        crate::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
        through,
        circular_store::PrefixHooks {
            facts: Box::new(move |commit, transaction| checkpoint_facts(commit, transaction)),
        },
    ))
}

fn publish_prefix(shared: &SharedWriterState, rehydrated: &MemoryStore<ProductStore>) {
    shared.prefix.store(Arc::new(rehydrated.published_prefix()));
    let wake = shared.record_wake.lock().ok().and_then(|wake| wake.clone());
    if let Some(wake) = wake {
        wake();
    }
}

fn writer_loop<
    C: circular_store::SqliteTransactionCodec<
            ProductTransaction,
            Error = circular_store::TransactionCodecError,
        >,
>(
    receiver: mpsc::Receiver<WriterCommand>,
    run: StreamId,
    store: circular_store::RecoveredSqliteTransactionStore<ProductTransaction, C>,
    mut rehydrated: MemoryStore<ProductStore>,
    mut next_key: u64,
    shared: SharedWriterState,
) {
    let policy = GroupCommitPolicy::from(ACCEPTED_GROUP_COMMIT.candidate());
    let mut coordinator = GroupCommitCoordinator::new(store, policy);
    let mut deferred = None;
    loop {
        let first = match deferred.take().map_or_else(|| receiver.recv(), Ok) {
            Ok(first) => first,
            Err(_) => break,
        };
        let closing = matches!(&first, WriterCommand::Shutdown(..));
        let mut commands = match first {
            WriterCommand::Close(reply) => {
                drop(coordinator);
                let _ = reply.send(Ok(()));
                return;
            }
            WriterCommand::Append(command, context) | WriterCommand::Shutdown(command, context) => {
                vec![(command, context)]
            }
        };
        let deadline = Instant::now() + policy.max_wait();
        while !closing && !commands.is_empty() && commands.len() < policy.max_batch_size().get() {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match receiver.recv_timeout(deadline.saturating_duration_since(now)) {
                Ok(shutdown @ (WriterCommand::Close(_) | WriterCommand::Shutdown(..))) => {
                    deferred = Some(shutdown);
                    break;
                }
                Ok(WriterCommand::Append(command, context)) => commands.push((command, context)),
                Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                    break;
                }
            }
        }
        let mut pending = BTreeMap::new();
        let mut commands = commands.into_iter();
        while let Some((command, context)) = commands.next() {
            let non_arrival = matches!(
                &command,
                Command::Records(_, _, _)
                    | Command::Approval(_, _)
                    | Command::Checkpoint(..)
                    | Command::EmissionBody { .. }
            );
            let prepared = prepare_command(&rehydrated, run, command, next_key)
                .and_then(|prepared| match prepared {
                    Prepared::Write(mut write, mut transaction, following_key) => {
                        if let Some(context) = context {
                            transaction = Box::new(context.bind(*transaction));
                        }
                        match prepare_live_rows(&transaction, &write.restored) {
                            Ok(rows) => {
                                if matches!(write.command, Command::Records(..))
                                    && let Some(operation) = rows.iter().position(|row| {
                                        rehydrated.record_for_key(row.record().header().key())
                                            .is_some_and(|recorded| *recorded != *row.record())
                                    })
                                {
                                    return Err((write.command, ArrivalCommitError::ProjectionRejected { operation }));
                                }
                                if let Command::EmissionBody { at, .. } = &write.command
                                    && pending.values().any(|prior: &PendingWrite| matches!(&prior.command,
                                        Command::EmissionBody { at: prior, .. } if prior.producer() == at.producer() && prior.sequence() == at.sequence()))
                                {
                                    return Err((write.command, ArrivalCommitError::Rejected { operation: 0 }));
                                }
                                write.transaction = Some((*transaction).clone());
                                write.rows = rows;
                                Ok(Prepared::Write(write, transaction, following_key))
                            }
                            Err(error) => Err((write.command, error)),
                        }
                    }
                    folded => Ok(folded),
                });
            match prepared {
                Ok(Prepared::Folded(command, receipt)) => reply_success(command, Some(receipt)),
                Ok(Prepared::Write(write, transaction, following_key)) => {
                    let id = coordinator.submit(Instant::now(), *transaction);
                    pending.insert(id, *write);
                    next_key = following_key;
                }
                Err((command, error)) => {
                    if non_arrival {
                        stop_recording(&shared, error.clone());
                        reply_failure(command, error.clone());
                        for (_, write) in pending {
                            reply_failure(write.command, error.clone());
                        }
                        for (command, _) in commands {
                            reply_failure(command, error.clone());
                        }
                        reject_queued(&receiver, &error);
                        return;
                    }
                    reply_failure(command, error);
                }
            }
        }
        if pending.is_empty() {
            continue;
        }
        let _boundary = (!pending.is_empty()).then(|| {
            shared
                .commit_boundary
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        });
        let mut observation_failed = false;
        loop {
            let flushed = coordinator.flush_stopping_on(|id| {
                pending
                    .get(&id)
                    .is_some_and(|write| matches!(write.command, Command::EmissionBody { .. }))
            });
            publish_namespace_bytes(&coordinator, &shared);
            let settled = match flushed {
                Ok(settled) => settled,
                Err(failure) => {
                    let (settled, unsettled, _source) = failure.into_parts();
                    let reason = ArrivalCommitError::BackendAmbiguous;
                    stop_recording(&shared, reason.clone());
                    for item in settled {
                        settle_one(item, &mut pending, &mut rehydrated, &shared);
                    }
                    for (id, _) in unsettled {
                        if let Some(write) = pending.remove(&id) {
                            reply_failure(write.command, reason.clone());
                        }
                    }
                    for (_, write) in pending {
                        reply_failure(write.command, reason.clone());
                    }
                    return;
                }
            };
            let emission_failure = settled.iter().find_map(|item| {
                if !pending
                    .get(&item.id())
                    .is_some_and(|write| matches!(write.command, Command::EmissionBody { .. }))
                {
                    return None;
                }
                match item.outcome() {
                    SubmissionOutcome::Rejected { operation, .. } => {
                        Some(ArrivalCommitError::Rejected {
                            operation: *operation,
                        })
                    }
                    SubmissionOutcome::Codec(_) => Some(ArrivalCommitError::Codec),
                    _ => None,
                }
            });
            if let Some(reason) = emission_failure {
                stop_recording(&shared, reason.clone());
                for item in settled {
                    settle_one(item, &mut pending, &mut rehydrated, &shared);
                }
                for (_, write) in pending {
                    reply_failure(write.command, reason.clone());
                }
                reject_queued(&receiver, &reason);
                return;
            }
            for item in settled {
                observation_failed |= settle_one(item, &mut pending, &mut rehydrated, &shared);
            }
            if pending.is_empty() {
                break;
            }
            std::thread::sleep(policy.max_wait());
        }
        drop(_boundary);
        if observation_failed {
            stop_recording(&shared, ArrivalCommitError::BackendAmbiguous);
            return;
        }
        if closing {
            return;
        }
    }
}

/// Republish the namespace size the committing handle already carries.
///
/// The writer is the only appender of this namespace, so the value it holds
/// after a group commit is the whole namespace's retained size. Publishing it
/// beside the prefix is what lets `usage` answer without reading the file.
fn publish_namespace_bytes<
    C: circular_store::SqliteTransactionCodec<
            ProductTransaction,
            Error = circular_store::TransactionCodecError,
        >,
>(
    coordinator: &GroupCommitCoordinator<
        ProductTransaction,
        circular_store::RecoveredSqliteTransactionStore<ProductTransaction, C>,
    >,
    shared: &SharedWriterState,
) {
    if let Some(bytes) = coordinator.store().retained_namespace_bytes() {
        shared.namespace_bytes.store(bytes, Ordering::Release);
    }
}

enum Prepared {
    Folded(Command, ArrivalCommitReceipt),
    Write(
        Box<PendingWrite>,
        Box<StoreTransaction<ProductTransaction>>,
        u64,
    ),
}

fn prepare_command(
    committed: &MemoryStore<ProductStore>,
    run: StreamId,
    command: Command,
    record_key: u64,
) -> Result<Prepared, (Command, ArrivalCommitError)> {
    let command = match command {
        Command::Checkpoint(row, reply) => {
            let Some(following) = record_key.checked_add(1) else {
                return Err((
                    Command::Checkpoint(row, reply),
                    ArrivalCommitError::RecordKeyExhausted,
                ));
            };
            let row = CheckpointRow::new(
                row.actor().clone(),
                *row.owner(),
                OpaqueId::new(record_key),
                row.state().clone(),
                row.pending().clone(),
            );
            let transaction =
                StoreTransaction::try_new(vec![StoreTransactionOp::ReplaceCheckpoint(row.clone())])
                    .expect("one checkpoint replacement");
            return Ok(Prepared::Write(
                Box::new(PendingWrite {
                    transaction: None,
                    command: Command::Checkpoint(row, reply),
                    rows: Box::new([]),
                    record_keys: Box::new([]),
                    arrival: None,
                    restored: Vec::new(),
                }),
                Box::new(transaction),
                following,
            ));
        }
        Command::Approval(mutation, reply) => {
            let (operation, following) = match &mutation {
                ApprovalMutation::Open(key, payload) => (
                    StoreTransactionOp::OpenApproval {
                        key: key.clone(),
                        approval: payload.clone(),
                    },
                    record_key,
                ),
                ApprovalMutation::Approve(key) => (
                    StoreTransactionOp::ApproveApproval {
                        key: key.clone(),
                        ticket: key.clone(),
                    },
                    record_key,
                ),
                ApprovalMutation::Settle(key, terminal) => {
                    let payload =
                        match crate::restart_custody_codec::encode_approval_terminal(*terminal) {
                            Ok(payload) => payload,
                            Err(_) => {
                                return Err((
                                    Command::Approval(mutation, reply),
                                    ArrivalCommitError::RecordEncoding,
                                ));
                            }
                        };
                    let Some(following) = record_key.checked_add(1) else {
                        return Err((
                            Command::Approval(mutation, reply),
                            ArrivalCommitError::RecordEncoding,
                        ));
                    };
                    (
                        StoreTransactionOp::SettleApproval {
                            key: key.clone(),
                            observation: circular_store::TransactionObservation::new(
                                OpaqueId::new(record_key),
                                payload,
                            ),
                        },
                        following,
                    )
                }
            };
            let transaction =
                StoreTransaction::try_new(vec![operation]).expect("one approval operation");
            return Ok(Prepared::Write(
                Box::new(PendingWrite {
                    transaction: None,
                    record_keys: if matches!(&mutation, ApprovalMutation::Settle(_, _)) {
                        vec![OpaqueId::new(record_key)].into_boxed_slice()
                    } else {
                        Box::new([])
                    },
                    command: Command::Approval(mutation, reply),
                    rows: Box::new([]),
                    arrival: None,
                    restored: Vec::new(),
                }),
                Box::new(transaction),
                following,
            ));
        }
        Command::EmissionBody { at, body } => {
            let record = Record::Boundary(BoundaryRecord::emission_body(
                at.clone(),
                RecordOrigin::Stream,
                body.clone(),
            ));
            if committed.record_for_key(record.header().key()).is_some() {
                return Err((
                    Command::EmissionBody { at, body },
                    ArrivalCommitError::Rejected { operation: 0 },
                ));
            }
            let command = Command::EmissionBody { at, body };
            let Some(following) = record_key.checked_add(1) else {
                return Err((command, ArrivalCommitError::RecordKeyExhausted));
            };
            let transaction = match record_transaction(record_key, &record) {
                Ok(transaction) => transaction,
                Err(error) => return Err((command, error)),
            };
            return Ok(Prepared::Write(
                Box::new(PendingWrite {
                    command,
                    transaction: None,
                    rows: Box::new([]),
                    record_keys: vec![OpaqueId::new(record_key)].into_boxed_slice(),
                    arrival: None,
                    restored: Vec::new(),
                }),
                Box::new(transaction),
                following,
            ));
        }
        Command::Records(records, live_bodies, reply) => {
            return prepare_records_command(records, live_bodies, reply, record_key);
        }
        command => command,
    };
    let (actor, expected) = match &command {
        Command::Event(candidate, _) => (candidate.actor().clone(), candidate.expected_horizon()),
        Command::Effect(candidate, _) => (candidate.actor().clone(), candidate.expected_horizon()),
        Command::Records(_, _, _)
        | Command::Approval(_, _)
        | Command::Checkpoint(..)
        | Command::EmissionBody { .. } => {
            unreachable!("non-arrival records return above")
        }
    };
    let identity = match arrival_identity(&command) {
        Ok(identity) => identity,
        Err(error) => return Err((command, error)),
    };
    if let Some(recorded) = committed.record_for_key(&identity) {
        let matches = match same_facts(run, &recorded, &command) {
            Ok(matches) => matches,
            Err(error) => return Err((command, error)),
        };
        if !matches {
            return Err((
                command,
                ArrivalCommitError::IdentityCarriedDifferentFacts {
                    actor: actor.clone(),
                },
            ));
        }
        let receipt = match folded_receipt(&recorded, &actor, expected) {
            Ok(receipt) => receipt,
            Err(error) => return Err((command, error)),
        };
        return Ok(Prepared::Folded(command, receipt));
    }
    let index = expected;
    let record_result = match &command {
        Command::Event(candidate, _) => event_record(run, candidate, index),
        Command::Effect(candidate, _) => effect_record(run, candidate, index),
        Command::Records(_, _, _)
        | Command::Approval(_, _)
        | Command::Checkpoint(..)
        | Command::EmissionBody { .. } => {
            unreachable!("non-arrival records return above")
        }
    };
    let record = match record_result {
        Ok(record) => record,
        Err(error) => return Err((command, error)),
    };
    let Some(following_key) = record_key.checked_add(1) else {
        return Err((command, ArrivalCommitError::RecordKeyExhausted));
    };
    let key = OpaqueId::new(record_key);
    let restored = match &command {
        Command::Event(candidate, _)
            if matches!(
                candidate.body().expect("event_record checked the body"),
                circular_store::ArrivalBody::Emitted { .. }
            ) =>
        {
            match candidate.encoded() {
                Ok(payload) => vec![(0, payload.clone())],
                Err(error) => return Err((command, error)),
            }
        }
        _ => Vec::new(),
    };
    let transaction = match record_transaction(record_key, &record) {
        Ok(transaction) => transaction,
        Err(error) => return Err((command, error)),
    };
    Ok(Prepared::Write(
        Box::new(PendingWrite {
            transaction: None,
            command,
            rows: Box::new([]),
            record_keys: vec![key].into_boxed_slice(),
            arrival: Some(PendingArrival { index }),
            restored,
        }),
        Box::new(transaction),
        following_key,
    ))
}

fn prepare_records_command(
    records: Box<[Record<ProductStore>]>,
    live_bodies: Vec<(usize, EncodedPayload)>,
    reply: RecordsReply,
    record_key: u64,
) -> Result<Prepared, (Command, ArrivalCommitError)> {
    let count = match u64::try_from(records.len()) {
        Ok(count) => count,
        Err(_) => {
            return Err((
                Command::Records(records, live_bodies, reply),
                ArrivalCommitError::RecordKeyExhausted,
            ));
        }
    };
    let following_key = match record_key.checked_add(count) {
        Some(following_key) => following_key,
        None => {
            return Err((
                Command::Records(records, live_bodies, reply),
                ArrivalCommitError::RecordKeyExhausted,
            ));
        }
    };
    let transaction = match records_transaction(record_key, &records) {
        Ok(transaction) => transaction,
        Err(error) => return Err((Command::Records(records, live_bodies, reply), error)),
    };
    let record_keys = (record_key..following_key)
        .map(OpaqueId::new)
        .collect::<Vec<_>>()
        .into_boxed_slice();
    Ok(Prepared::Write(
        Box::new(PendingWrite {
            transaction: None,
            command: Command::Records(records, Vec::new(), reply),
            rows: Box::new([]),
            record_keys,
            arrival: None,
            restored: live_bodies,
        }),
        Box::new(transaction),
        following_key,
    ))
}

fn settle_one(
    item: circular_store::SettledSubmission<
        ProductTransaction,
        circular_store::TransactionCodecError,
    >,
    pending: &mut BTreeMap<circular_store::SubmissionId, PendingWrite>,
    rehydrated: &mut MemoryStore<ProductStore>,
    shared: &SharedWriterState,
) -> bool {
    let (id, outcome) = item.into_parts();
    let Some(write) = pending.remove(&id) else {
        return false;
    };
    if let SubmissionOutcome::Committed { commit, .. } = &outcome {
        if let Some(transaction) = &write.transaction {
            match checkpoint_facts(*commit, transaction) {
                Ok(facts) => {
                    for fact in facts {
                        rehydrated.push_checkpoint_fact(fact, *commit);
                    }
                }
                Err(_) => {
                    reply_failure(write.command, ArrivalCommitError::RecordDecoding);
                    return true;
                }
            }
        }
        rehydrated.note_commit(*commit);
        if matches!(
            write.command,
            Command::Checkpoint(..) | Command::Approval(..)
        ) {
            publish_prefix(shared, rehydrated);
        }
    }
    if let Command::Checkpoint(row, _) = &write.command {
        let expected = [StoreTransactionRef::CheckpointCurrent {
            actor: row.actor().clone(),
            at: *row.at(),
        }];
        return match outcome {
            SubmissionOutcome::Committed { receipt, .. } if receipt.refs() == expected => {
                shared
                    .checkpoint_horizon
                    .store(row.at().get() + 1, Ordering::Release);
                reply_success(write.command, None);
                false
            }
            _ => {
                reply_failure(write.command, ArrivalCommitError::BackendAmbiguous);
                true
            }
        };
    }
    if let Command::Approval(mutation, _) = &write.command {
        let expected = match mutation {
            ApprovalMutation::Open(key, _) => {
                vec![StoreTransactionRef::ApprovalOpened(key.clone())]
            }
            ApprovalMutation::Approve(key) => {
                vec![StoreTransactionRef::ApprovalApproved(key.clone())]
            }
            ApprovalMutation::Settle(key, _) => vec![
                StoreTransactionRef::ApprovalSettled(key.clone()),
                StoreTransactionRef::Observation(write.record_keys[0]),
            ],
        };
        match outcome {
            SubmissionOutcome::Committed { receipt, .. }
                if receipt.refs() == expected.as_slice() =>
            {
                reply_success(write.command, None)
            }
            _ => {
                reply_failure(write.command, ArrivalCommitError::BackendAmbiguous);
                return true;
            }
        }
        return false;
    }

    let observation = write.arrival.is_none();
    let expected_refs = write
        .record_keys
        .iter()
        .cloned()
        .map(StoreTransactionRef::Appended)
        .collect::<Vec<_>>();
    match outcome {
        SubmissionOutcome::Committed { receipt, commit }
            if receipt.refs() == expected_refs.as_slice() =>
        {
            let batch = AppendBatch::try_new_rows(write.rows.into_vec())
                .expect("a pending write always contains records")
                .at_commit(commit);
            if !matches!(rehydrated.append(batch), AppendResult::Committed(_)) {
                reply_failure(
                    write.command,
                    ArrivalCommitError::RehydratedProjectionFailed,
                );
                return observation;
            }
            rehydrated.seal_all();
            publish_prefix(shared, rehydrated);
            if let Some(following) = write
                .record_keys
                .last()
                .and_then(|key| key.get().checked_add(1))
            {
                shared.record_count.fetch_max(following, Ordering::Release);
            }
            if let Some(arrival) = write.arrival {
                let (at, observed_at) = match &write.command {
                    Command::Event(candidate, _) => {
                        (candidate.at().clone(), candidate.observed_at())
                    }
                    Command::Effect(candidate, _) => {
                        (candidate.at().clone(), candidate.observed_at())
                    }
                    Command::Records(_, _, _)
                    | Command::Approval(_, _)
                    | Command::Checkpoint(..)
                    | Command::EmissionBody { .. } => {
                        unreachable!("arrival metadata has an arrival command")
                    }
                };
                reply_success(
                    write.command,
                    Some(ArrivalCommitReceipt::recorded_event(
                        arrival.index,
                        at,
                        observed_at,
                    )),
                );
            } else {
                reply_success(write.command, None);
            }
            false
        }
        SubmissionOutcome::Committed { .. } => {
            reply_failure(write.command, ArrivalCommitError::UnexpectedReceipt);
            observation
        }
        SubmissionOutcome::Rejected {
            operation,
            reason: _,
        } => {
            let emission = matches!(write.command, Command::EmissionBody { .. });
            reply_failure(write.command, ArrivalCommitError::Rejected { operation });
            emission
        }
        SubmissionOutcome::Codec(_error) => {
            let emission = matches!(write.command, Command::EmissionBody { .. });
            reply_failure(write.command, ArrivalCommitError::Codec);
            emission
        }
    }
}

fn reply_success(command: Command, receipt: Option<ArrivalCommitReceipt>) {
    match command {
        Command::EmissionBody { .. } => {}
        Command::Checkpoint(row, reply) => {
            let _ = reply.send(Ok(row));
        }
        Command::Event(_, sender) | Command::Effect(_, sender) => {
            let _ = sender.send(Ok(receipt.expect("arrival success carries a receipt")));
        }
        Command::Records(_, _, reply) | Command::Approval(_, reply) => {
            debug_assert!(receipt.is_none());
            reply.send(Ok(()));
        }
    }
}

fn reply_failure(command: Command, error: ArrivalCommitError) {
    match command {
        Command::EmissionBody { .. } => {}
        Command::Checkpoint(_, reply) => {
            let _ = reply.send(Err(format!("checkpoint commit failed: {error}")));
        }
        Command::Event(_, sender) | Command::Effect(_, sender) => {
            let _ = sender.send(Err(error));
        }
        Command::Records(_, _, reply) | Command::Approval(_, reply) => {
            reply.send(Err(format!("product record append failed: {error}")));
        }
    }
}

fn prepare_live_rows(
    transaction: &StoreTransaction<ProductTransaction>,
    live_bodies: &[(usize, EncodedPayload)],
) -> Result<Box<[circular_store::EncodedRow<ProductStore>]>, ArrivalCommitError> {
    let mut rows = transaction
        .operations()
        .iter()
        .filter_map(|operation| match operation {
            StoreTransactionOp::Append(append) => Some(append.record().clone()),
            _ => None,
        })
        .map(|bytes| circular_store::EncodedRow::decoded(&circular_store::ProductRowCodec, bytes))
        .collect::<Option<Vec<_>>>()
        .ok_or(ArrivalCommitError::RecordDecoding)?;
    if !publish_live_bodies(&mut rows, live_bodies) {
        return Err(ArrivalCommitError::RecordDecoding);
    }
    Ok(rows.into_boxed_slice())
}

fn reject_queued(receiver: &mpsc::Receiver<WriterCommand>, reason: &ArrivalCommitError) {
    for queued in receiver.try_iter() {
        match queued {
            WriterCommand::Append(command, _) | WriterCommand::Shutdown(command, _) => {
                reply_failure(command, reason.clone())
            }
            WriterCommand::Close(reply) => {
                let _ = reply.send(Err(reason.to_string()));
            }
        }
    }
}

fn publish_live_bodies(
    rows: &mut [circular_store::EncodedRow<ProductStore>],
    restored: &[(usize, EncodedPayload)],
) -> bool {
    for (index, payload) in restored {
        match rows
            .get_mut(*index)
            .map(circular_store::EncodedRow::record_mut)
        {
            Some(Record::Boundary(boundary)) => {
                if !boundary.resolve_arrival_body(payload.clone()) {
                    return false;
                }
            }
            _ => return false,
        }
    }
    rows.iter().all(|row| {
        !matches!(row.record(), Record::Boundary(boundary)
        if matches!(boundary.arrival_body(), Some(circular_store::ArrivalBody::Emitted { .. })))
    })
}

pub(crate) fn record_transaction(
    key: u64,
    record: &Record<ProductStore>,
) -> Result<StoreTransaction<ProductTransaction>, ArrivalCommitError> {
    let bytes = encode_record(record, &ProductRecordCodec)
        .map_err(|_| ArrivalCommitError::RecordEncoding)?;
    StoreTransaction::try_new(vec![StoreTransactionOp::Append(TransactionAppend::new(
        OpaqueId::new(key),
        EncodedPayload::new(PayloadVersionTag::FIRST, &bytes),
    ))])
    .map_err(|_| ArrivalCommitError::RecordEncoding)
}

fn records_transaction(
    first_key: u64,
    records: &[Record<ProductStore>],
) -> Result<StoreTransaction<ProductTransaction>, ArrivalCommitError> {
    let batch =
        AppendBatch::try_new(records.to_vec()).map_err(|_| ArrivalCommitError::RecordEncoding)?;
    let encoded = arrival_transaction(&batch).map_err(|_| ArrivalCommitError::RecordEncoding)?;
    let operations = encoded
        .operations()
        .iter()
        .enumerate()
        .map(|(offset, operation)| {
            let StoreTransactionOp::Append(append) = operation else {
                unreachable!("arrival_transaction emits append operations only")
            };
            let offset =
                u64::try_from(offset).map_err(|_| ArrivalCommitError::RecordKeyExhausted)?;
            let key = first_key
                .checked_add(offset)
                .ok_or(ArrivalCommitError::RecordKeyExhausted)?;
            Ok(StoreTransactionOp::Append(TransactionAppend::new(
                OpaqueId::new(key),
                append.record().clone(),
            )))
        })
        .collect::<Result<Vec<_>, ArrivalCommitError>>()?;
    StoreTransaction::try_new(operations).map_err(|_| ArrivalCommitError::RecordEncoding)
}

fn arrival_identity(command: &Command) -> Result<ClassKey<ProductStore>, ArrivalCommitError> {
    let (actor, origin) = match command {
        Command::Event(candidate, _) => {
            let origin = event_arrival_origin(candidate)?;
            let mut key = origin.key();
            if let circular_store::ArrivalKey::ExternalInject { route_edge, .. } = &mut key {
                *route_edge = candidate.route_edge().cloned();
            }
            (candidate.actor().as_actor_id(), key)
        }
        Command::Effect(candidate, _) => (
            candidate.actor().as_actor_id(),
            circular_store::ArrivalKey::EffectOutcome {
                effect: candidate.effect(),
            },
        ),
        Command::Records(_, _, _)
        | Command::Approval(_, _)
        | Command::Checkpoint(..)
        | Command::EmissionBody { .. } => {
            unreachable!("non-arrival records return before identity")
        }
    };
    Ok(ClassKey::Boundary(BoundaryKey::Arrival {
        actor,
        origin: Box::new(origin),
    }))
}

fn same_facts(
    run: StreamId,
    record: &Record<ProductStore>,
    command: &Command,
) -> Result<bool, ArrivalCommitError> {
    let Record::Boundary(boundary) = record else {
        return Ok(false);
    };
    let BoundaryFact::Arrival {
        result,
        origin,
        inlet,
        route_edge,
        body,
        causal_parents,
        ..
    } = boundary.fact()
    else {
        return Ok(false);
    };
    let Some(payload) = body.payload() else {
        return Ok(false);
    };
    Ok(match command {
        Command::Event(candidate, _) => {
            let encoded = circular_core::encode(
                candidate.payload().value(),
                Ceilings::for_boundary(Boundary::Journal),
            )
            .unwrap_or_default();
            let parents = candidate
                .causal_parents()
                .iter()
                .cloned()
                .map(|stamp| EventId::derive(run, stamp))
                .collect::<Vec<_>>();
            result == candidate.result()
                && route_edge.as_ref() == candidate.route_edge()
                && inlet.as_ref() == Some(candidate.inlet())
                && payload.body() == encoded
                && causal_parents.as_ref() == parents.as_slice()
                && runtime_origin_matches(origin, candidate.origin())
        }
        Command::Effect(candidate, _) => match origin.as_ref() {
            ArrivalOrigin::EffectOutcome {
                effect,
                term,
                outcome,
            } => {
                let encoded = crate::effect_outcome_record::EncodedOutcome::new(
                    candidate.actor(),
                    candidate.cause(),
                    candidate.term(),
                    candidate.result(),
                    candidate.failure_progress(),
                )
                .map_err(|_| ArrivalCommitError::RecordEncoding)?;
                *effect == candidate.effect()
                    && term.version_tag() == PayloadVersionTag::FIRST
                    && outcome.version_tag() == PayloadVersionTag::FIRST
                    && payload.version_tag() == PayloadVersionTag::FIRST
                    && term.body() == encoded.term
                    && payload.body() == encoded.summary
                    && outcome.body() == encoded.body
                    && causal_parents.as_ref() == [EventId::derive(run, candidate.cause().clone())]
            }
            _ => false,
        },
        Command::Records(_, _, _)
        | Command::Approval(_, _)
        | Command::Checkpoint(..)
        | Command::EmissionBody { .. } => false,
    })
}

fn runtime_origin_matches(
    stored: &ArrivalOrigin<ProductStore>,
    runtime: &RuntimeArrivalOrigin<ActorId, circular_runtime::EffectId, circular_runtime::EffectId>,
) -> bool {
    match (stored, runtime) {
        (
            ArrivalOrigin::EdgeDelivery { edge, sender },
            RuntimeArrivalOrigin::EdgeDelivery {
                edge: candidate_edge,
                stamp,
            },
        ) => edge == candidate_edge && sender == stamp,
        (
            ArrivalOrigin::TimerFire { timer },
            RuntimeArrivalOrigin::TimerFire {
                timer: candidate_timer,
            },
        ) => timer == candidate_timer,
        (
            ArrivalOrigin::ExternalInject { origin },
            RuntimeArrivalOrigin::ExternalInject {
                origin: candidate_origin,
            },
        ) => origin.body() == candidate_origin.as_bytes(),
        _ => false,
    }
}

fn folded_receipt(
    recorded: &Record<ProductStore>,
    actor: &NamedActorId,
    mailbox_horizon: ArrivalIndex,
) -> Result<ArrivalCommitReceipt, ArrivalCommitError> {
    let Record::Boundary(boundary) = recorded else {
        return Err(ArrivalCommitError::MissingBoundaryArrival);
    };
    let BoundaryFact::Arrival {
        arrival_index,
        observed_at,
        ..
    } = boundary.fact()
    else {
        return Err(ArrivalCommitError::MissingBoundaryArrival);
    };
    let recorded_index = *arrival_index;
    if mailbox_horizon < recorded_index {
        return Err(ArrivalCommitError::FoldedHorizonMismatch {
            actor: actor.clone(),
            recorded: recorded_index,
            mailbox: mailbox_horizon,
        });
    }
    Ok(ArrivalCommitReceipt::folded_event(
        recorded_index,
        if mailbox_horizon == recorded_index {
            crate::arrival_commit::ArrivalCommitDisposition::Recovered
        } else {
            crate::arrival_commit::ArrivalCommitDisposition::Folded
        },
        recorded.header().at().clone(),
        *observed_at,
    ))
}

fn event_arrival_origin(
    candidate: &EventArrivalCommit,
) -> Result<ArrivalOrigin<ProductStore>, ArrivalCommitError> {
    Ok(match candidate.origin() {
        RuntimeArrivalOrigin::EdgeDelivery { edge, stamp } => ArrivalOrigin::EdgeDelivery {
            edge: edge.clone(),
            sender: stamp.clone(),
        },
        RuntimeArrivalOrigin::TimerFire { timer } => ArrivalOrigin::TimerFire {
            timer: timer.clone(),
        },
        RuntimeArrivalOrigin::ExternalInject { origin } => ArrivalOrigin::ExternalInject {
            origin: EncodedPayload::new(PayloadVersionTag::FIRST, origin.as_bytes()),
        },
        RuntimeArrivalOrigin::EffectOutcome { .. } => {
            return Err(ArrivalCommitError::WrongRecordConstructor);
        }
    })
}

pub(crate) fn encode_event_payload(
    payload: &ProductPayload,
) -> Result<EncodedPayload, ArrivalCommitError> {
    circular_core::encode(payload.value(), Ceilings::for_boundary(Boundary::Journal))
        .map(|bytes| EncodedPayload::new(PayloadVersionTag::FIRST, &bytes))
        .map_err(|_| ArrivalCommitError::RecordEncoding)
}

pub(crate) fn event_record(
    run: StreamId,
    candidate: &EventArrivalCommit,
    index: ArrivalIndex,
) -> Result<Record<ProductStore>, ArrivalCommitError> {
    let origin = event_arrival_origin(candidate)?;
    Ok(Record::Boundary(
        BoundaryRecord::arrival(
            candidate.actor().as_actor_id(),
            candidate.at().clone(),
            RecordOrigin::Stream,
            origin,
            candidate.body()?.clone(),
            index,
            candidate
                .causal_parents()
                .iter()
                .cloned()
                .map(|stamp| EventId::derive(run, stamp))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            candidate.observed_at(),
            Some(candidate.inlet().clone()),
        )
        .with_route_edge(candidate.route_edge().cloned())
        .with_result(candidate.result().clone()),
    ))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn admission_record(
    run: StreamId,
    actor: &NamedActorId,
    inlet: &circular_core::PortId,
    edge: &circular_plan::EdgeId,
    sender: &circular_core::Stamp<ActorId>,
    parents: &[circular_core::Stamp<ActorId>],
    body: circular_store::ArrivalBody<ActorId>,
    result: &circular_runtime::EnvelopeResult,
    observed_at: circular_core::RecordedInstant,
) -> Result<Record<ProductStore>, ArrivalCommitError> {
    Ok(Record::Boundary(
        BoundaryRecord::admission(
            actor.as_actor_id(),
            sender.clone(),
            RecordOrigin::Stream,
            ArrivalOrigin::EdgeDelivery {
                edge: edge.clone(),
                sender: sender.clone(),
            },
            body,
            parents
                .iter()
                .cloned()
                .map(|stamp| EventId::derive(run, stamp))
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            observed_at,
            Some(inlet.clone()),
        )
        .with_result(result.clone()),
    ))
}

fn effect_record(
    run: StreamId,
    candidate: &EffectArrivalCommit,
    index: ArrivalIndex,
) -> Result<Record<ProductStore>, ArrivalCommitError> {
    crate::effect_outcome_record::effect_outcome_record(
        run,
        candidate.actor(),
        candidate.at().clone(),
        index,
        candidate.effect(),
        candidate.cause(),
        candidate.term(),
        candidate.result(),
        candidate.failure_progress(),
        candidate.observed_at(),
    )
    .map_err(|_| ArrivalCommitError::RecordEncoding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_actors::ProductValue;
    use circular_core::Value;
    use circular_core::{
        BuiltinObservationName, GroundShape, NonZeroTicks, Sequence, Shape, Stamp, Tick,
        TicksPerSecond, TimeSourceKind, TimeSourcePlan,
    };
    use circular_plan::{Name, ScopeId};
    use circular_runtime::{
        EffectFailure, EffectTerm, ExternalOrigin, NotificationChannel, NotificationSpec,
        decode_term,
    };
    use circular_store::rehydrate;
    use circular_store::{
        AtomicStoreTransactionPort, FailureParams, ManifestGroups, ObservationBucket,
        ObservationItemKey, ObservationRecord, PlacementParams, RevisionContext, RevisionStart,
        RunInputs, RunManifest, SqliteTransactionCodec, StructureRecord, TimeParams,
    };
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(1);

    fn actor() -> NamedActorId {
        NamedActorId::new(ScopeId::root(), Name::from_normalized("sink"))
    }

    fn numbered_actor(ordinal: u64) -> NamedActorId {
        NamedActorId::new(
            ScopeId::root(),
            Name::from_normalized(format!("sink_{ordinal}")),
        )
    }

    fn seed(run: StreamId) -> Record<ProductStore> {
        let payload = || EncodedPayload::new(PayloadVersionTag::FIRST, b"");
        let revision = RevisionContext::try_new(
            RevisionStart::Fresh(circular_store::manifest_test_support::authoring_cut()),
            Vec::new(),
        )
        .expect("revision");
        let groups = ManifestGroups::new(
            TimeParams::new(
                TicksPerSecond::new(1_000).expect("resolution"),
                NonZeroTicks::new(1).expect("cadence"),
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
        Record::Structure(StructureRecord::manifest(
            Stamp::from_event_producer(
                Tick::ZERO,
                actor(),
                Sequence::FIRST,
                circular_core::RevisionEpochId::new(1).expect("first revision"),
            ),
            RunManifest::new(run, groups),
        ))
    }

    fn candidate(expected: ArrivalIndex) -> EventArrivalCommit {
        candidate_for(
            actor(),
            expected,
            ExternalOrigin::try_new(b"durable-request".to_vec()).expect("origin"),
        )
    }

    fn candidate_for(
        actor: NamedActorId,
        expected: ArrivalIndex,
        origin: ExternalOrigin,
    ) -> EventArrivalCommit {
        EventArrivalCommit::new(
            actor.clone(),
            circular_core::PortId::try_new("event").unwrap(),
            expected,
            Stamp::from_event_producer(
                Tick::new(expected.get()),
                actor,
                Sequence::new(expected.get()).expect("arrival index is a sequence"),
                circular_core::RevisionEpochId::new(1).expect("first revision"),
            ),
            RuntimeArrivalOrigin::ExternalInject { origin },
            Box::new([]),
            ProductPayload::new(
                GroundShape::try_new(Shape::Any).expect("shape"),
                ProductValue::Null,
            ),
            circular_core::RecordedInstant::from_millis(7),
        )
    }

    fn observation_records(run: StreamId) -> Vec<Record<ProductStore>> {
        let producer = numbered_actor(70);
        let at = |ordinal: u64| {
            Stamp::from_event_producer(
                Tick::new(ordinal),
                producer.clone(),
                Sequence::new(ordinal).expect("positive observation sequence"),
                circular_core::RevisionEpochId::new(1).expect("first revision"),
            )
        };
        let payload = |body: &'static [u8]| EncodedPayload::new(PayloadVersionTag::FIRST, body);
        let by = || RecordOrigin::Stream;
        vec![
            Record::Observation(ObservationRecord::dead_letter(
                at(1),
                ObservationBucket::from_millis(1),
                by(),
                ObservationItemKey::new(BuiltinObservationName::DeadLetterEntry, OpaqueId::new(1)),
                payload(b"dead-letter"),
            )),
            Record::Observation(ObservationRecord::diagnostic(
                at(2),
                ObservationBucket::from_millis(2),
                by(),
                ObservationItemKey::new(
                    BuiltinObservationName::DiagnosticOccurrence,
                    OpaqueId::new(2),
                ),
                payload(b"actor-health-transition"),
            )),
        ]
    }

    fn carrier_record(actor: NamedActorId, ordinal: u64) -> Record<ProductStore> {
        crate::display_writer::project_display::<ProductStore, _>(
            circular_core::Stamp::from_event_producer(
                circular_core::Tick::new(ordinal),
                actor.clone(),
                circular_core::Sequence::new(ordinal).unwrap(),
                circular_core::RevisionEpochId::new(1).unwrap(),
            ),
            circular_store::DisplayKey::new(
                circular_core::BuiltinObservationName::Tally,
                actor,
                None,
            ),
            &(),
            |()| vec![0],
        )
    }

    fn records_after_reopen(path: &Path, run: StreamId) -> Vec<Record<ProductStore>> {
        let mut recovering = RecoveringSqliteTransactionStore::from_journal(
            circular_store::SqliteJournal::open_namespace(
                path,
                crate::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
            )
            .unwrap(),
            ProductJournalCodec,
        )
        .expect("journal reopens for projection");
        let snapshot = recovering.journal_snapshot().expect("journal snapshot");
        let page = std::num::NonZeroUsize::new(64).expect("page size");
        rehydrate::<ProductTransaction, ProductStore, _, _>(
            &snapshot,
            &ProductJournalCodec,
            &ArrivalProjection::new(),
            PagePolicy::new(page, page).expect("page policy"),
        )
        .expect("the committed journal rehydrates")
        .records()
        .to_vec()
    }

    fn quantile(values: &[u128], percentile: usize) -> u128 {
        let mut ordered = values.to_vec();
        ordered.sort_unstable();
        let rank = percentile
            .checked_mul(ordered.len())
            .and_then(|value| value.checked_add(99))
            .expect("small measurement rank")
            / 100;
        ordered[rank.max(1) - 1]
    }

    fn outcome_candidate(
        index: u64,
        effect: circular_runtime::EffectId,
        result: Result<circular_runtime::OutcomePayload, EffectFailure>,
    ) -> EffectArrivalCommit {
        EffectArrivalCommit::new(
            actor(),
            ArrivalIndex::new(index),
            Stamp::from_event_producer(
                Tick::new(index + 10),
                actor(),
                Sequence::new(index).unwrap(),
                circular_core::RevisionEpochId::new(1).unwrap(),
            ),
            effect,
            Stamp::from_event_producer(
                Tick::new(9),
                actor(),
                Sequence::new(0).unwrap(),
                circular_core::RevisionEpochId::new(1).unwrap(),
            ),
            None,
            result,
            circular_core::RecordedInstant::from_millis(29),
        )
    }

    #[test]
    fn delivery_transaction_round_trips_at_and_sender_origin() {
        let run = StreamId::new(23);
        let receiver = actor();
        let upstream = numbered_actor(9);
        let index = ArrivalIndex::new(4);
        let at = Stamp::from_event_producer(
            Tick::new(11),
            receiver.clone(),
            Sequence::new(index.get()).unwrap(),
            circular_core::RevisionEpochId::new(1).expect("first revision"),
        );
        let sender = Stamp::from_event_producer(
            Tick::new(10),
            upstream,
            Sequence::new(2).unwrap(),
            circular_core::RevisionEpochId::new(1).expect("first revision"),
        );
        let origin = RuntimeArrivalOrigin::EdgeDelivery {
            edge: circular_plan::EdgeId::outcome(receiver.clone()),
            stamp: sender.clone(),
        };
        let payload = ProductPayload::new(
            GroundShape::try_new(Shape::Any).expect("shape"),
            ProductValue::Null,
        );
        let observed_at = circular_core::RecordedInstant::from_millis(17);
        let candidate = EventArrivalCommit::new(
            receiver.clone(),
            circular_core::PortId::try_new("event").unwrap(),
            index,
            at.clone(),
            origin.clone(),
            Box::new([]),
            payload.clone(),
            observed_at,
        )
        .with_body(circular_store::ArrivalBody::emitted(&sender));
        let durable = event_record(run, &candidate, index).expect("delivery transaction record");

        let transaction = record_transaction(7, &durable).expect("record encodes");
        let reopened = ArrivalProjection::new()
            .project(&transaction)
            .expect("record decodes through the product projection");
        let [Record::Boundary(reopened)] = reopened.as_slice() else {
            panic!("exactly one Boundary arrival round-trips")
        };
        let BoundaryFact::Arrival {
            origin: reopened_origin,
            body,
            ..
        } = reopened.fact()
        else {
            panic!("fixture requires an Arrival record");
        };
        assert!(matches!(
            reopened_origin.as_ref(),
            ArrivalOrigin::EdgeDelivery {
                sender: reopened_sender,
                ..
            } if reopened_sender == &sender
        ));
        assert_eq!(reopened.header().at(), &at);
        assert_eq!(
            body,
            &circular_store::ArrivalBody::Emitted {
                producer: numbered_actor(9).as_actor_id(),
                sequence: Sequence::new(2).unwrap(),
            }
        );
    }
}
