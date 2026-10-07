#![forbid(unsafe_code)]

mod approval;
pub mod atomic_file;
mod column_codec;
pub use column_codec::{ColumnWriteContext, KeyframePolicy, ProductJournalReader};
#[cfg(any(test, feature = "test-support"))]
pub use column_codec::{commit_fixture_transactions, fixture_column};
mod group_commit;
mod journal_fold;
mod journal_view;
mod live_feed;
mod manifest;
mod memory;
mod outbox_request;
mod port;
mod product_identity;
mod product_journal;
mod product_schema;
mod query;
#[cfg(feature = "test-support")]
pub mod read_measurement;
mod record;
mod record_codec;
mod record_stamp_value;
mod rehydrate;
mod restart;
mod schedule_reservation;
mod shutdown;
mod stream_start;
pub use schedule_reservation::{ProductScheduleReservation, read_schedule_reservation};
mod sqlite;
mod surface_index;
mod transaction;
mod transaction_codec;

#[cfg(feature = "test-support")]
pub mod scripted_port;

pub use outbox_request::{
    CustodyFold, ProductCustodySnapshot, ProductOutboxCustody, ProductOutboxRequest,
};
#[cfg(any(test, feature = "test-support"))]
pub mod manifest_test_support;

pub use approval::{
    ApprovalLedger, ApprovalLedgerDecision, ApprovalLedgerDiagnostic, ApprovalLedgerSnapshot,
    ApprovalRequest, ApprovalRequestDisposition, ApprovalRow, ApprovalTerminal,
    ApprovalTerminalObservation,
};
pub use group_commit::{
    ACCEPTED_GROUP_COMMIT, AcceptedGroupCommit, AcceptedValueProvenance, BackendFailureParts,
    FlushResult, GroupCommitBackendFailure, GroupCommitCoordinator, GroupCommitPolicy,
    SettledSubmission, SubmissionId, SubmissionOutcome, UnsettledSubmissions,
};
pub use journal_fold::{JournalFoldHooks, ProductJournalFold};
pub use journal_view::{JournalPrefix, JournalView, PREFIX_ORIGIN, PrefixHooks, ViewRow, ViewRows};
pub use live_feed::LiveFrame;
pub use manifest::{
    AuthoringCut, FailureParams, ManifestGroups, ManifestSchema, PlacementParams, RevisionContext,
    RevisionContextError, RevisionStart, RunInputs, RunManifest, TimeParams, encode_run_inputs,
};
pub use memory::{
    AppendBatch, AppendBatchError, AppendFailure, AppendRejectReason, AppendResult, MemoryStore,
    PagePolicy, PagePolicyError, Receipt, RecordRef,
};
pub use port::Store;
pub use product_identity::{
    ProductIdentityError, actor_from_value, actor_parts_from_value, actor_value, edge_value,
    identity_bytes, instance_key_from_value, instance_key_value, named_actor_value,
    record_actor_from_value, record_actor_value, record_stamp, scope_from_value, scope_value,
};
pub use product_journal::{
    ArrivalProjection, PositionedProjection, PositionedRecord, ProductJournalCodec,
    ProductRecordCodec, ProductRowCodec, ProductTransactionCodec, arrival_transaction,
    authoring_cut_bytes, authoring_cut_from_bytes, display_key_bytes, display_key_from_bytes,
    manifest_bytes, manifest_from_bytes, push_stamp, read_emission_body, reencodes_identically,
    rehydrate_arrivals, rehydrate_published_arrivals, take_stamp,
};
pub use product_schema::{
    DisplayKey, IncarnationId, OpaqueId, OperationId, ProductAuthoringCut, ProductStore,
    ProductTransaction, StreamId,
};
pub use query::{
    Bound, BucketRange, BucketRangeError, Cursor, CursorRejection, ObservationScope, Page, Query,
    ScanStart, StoreError, SurfaceSequence,
};
pub use record::{
    ArrivalBody, ArrivalKey, ArrivalOrigin, BoundaryFact, BoundaryKey, BoundaryRecord, Class,
    ClassKey, DisplayRecord, EmissionFact, EncodedRow, ObservationBucket, ObservationFact,
    ObservationItemKey, ObservationKey, ObservationRecord, OperationCoordinate, Record,
    RecordHeader, RecordOrigin, RecordPosition, RecordRowCodec, StoreSchema, StructureFact,
    StructureKey, StructureRecord,
};
pub use record_codec::{
    ACCOUNTING_FACT_TAG, CHECKPOINT_FACT_TAG, CHECKPOINT_KEY_TAG, DEAD_LETTER_FACT_TAG,
    DIAGNOSTIC_FACT_TAG, EnvelopeOrigin, LIFECYCLE_FACT_TAG, OpaqueWitness, RECORD_VERSION,
    REPLAY_SESSION_FACT_TAG, RESTART_FACT_TAG, RecordCodecError, RecordEnvelope,
    RecordIdentityCodec, decode_envelope, decode_many, encode_record, observation_fact_tag,
};
pub use record_stamp_value::{record_stamp_from_value, record_stamp_value};
pub use rehydrate::{JournalProjection, ProjectionRejection, RehydrateError, rehydrate};
pub use sqlite::{
    JOURNAL_FORMAT_VERSION, PreparedSqliteBatch, RecoveredSqliteTransactionStore,
    RecoveringSqliteTransactionStore, SQLITE_FIXED_SCHEMA_VERSION, SqliteCommitError,
    SqliteCommitGroupReceipt, SqliteCommitReceipt, SqliteCommitSequence,
    SqliteGroupCommitCandidate, SqliteJournal, SqliteJournalEntry, SqliteJournalError,
    SqliteJournalSnapshot, SqlitePayloadChecksum, SqliteRecoveryError, SqliteRecoveryFailure,
    SqliteRecoveryResult, SqliteTransactionCodec, SqliteTransactionGroupFailure,
    SqliteTransactionGroupFailureReason, SqliteTransactionGroupReceipt, SqliteTransactionOpenError,
};
pub use transaction::{
    AtomicStoreTransactionPort, CheckpointOwnerConflict, CheckpointOwners, CommittedTransaction,
    CrashDurableTransactionPort, EmptyTransaction, GroupCommitFailure, GroupTransactionPort,
    RecordDigest, RecoveredTransactionModel, RecoveringTransactionModel, RecoveryTerminal,
    StoreTransaction, StoreTransactionFailure, StoreTransactionFailureReason, StoreTransactionOp,
    StoreTransactionReceipt, StoreTransactionRef, StoreTransactionReject, TransactionAppend,
    TransactionApproval, TransactionApprovalPhase, TransactionCheckpoint, TransactionObservation,
    TransactionOutbox, TransactionOutboxPhase, TransactionRecoveryError,
    TransactionRecoveryFailure, TransactionRecoveryPlan, TransactionRecoveryResult,
    TransactionSchema,
};
pub use transaction_codec::{
    APPEND_FRONTIER, TransactionCodecError, TransactionParts, TransactionValueCodec,
    decode_transaction, encode_transaction, op_tag,
};

pub use circular_core::EncodedPayload;

pub use restart::{ProductRestartBody, RestartFacts, RestartReason, restart_facts, restart_record};
pub use shutdown::{ProductShutdownBody, shutdown_record};
pub use stream_start::{ProductStreamStartBody, stream_start_record};

mod record_segments;
pub use record_segments::{RecordRow, RecordSegments, RecordSlice, RowIter, StoredRow};

mod arrival_result;

mod query_positions;
