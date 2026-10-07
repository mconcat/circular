#![forbid(unsafe_code)]

pub mod actor_events;
mod admission;
pub mod agent_harness_payload;
pub mod approval_payload;
pub mod authored_value;
pub mod authoring_snapshot;
pub mod boundary_port;
mod command_fsm;
pub mod dead_letter;
pub mod declaration_payload;
mod dispatch_handler;
mod dispatcher;
mod envelope;
mod epoch_fsm;
mod identity;
pub mod injection_payload;
mod kind;
pub mod lifecycle;
pub mod lifecycle_payload;
mod message;
pub mod move_to_scope;
mod payload;
pub mod port_type;
mod query_fsm;
pub mod query_payload;
pub mod rejection_code;
pub mod replay_payload;
pub mod scope_identity;
mod session;
mod session_fsm;
pub mod session_payload;
mod subscription_fsm;
pub mod subscription_payload;
pub mod timeline;
mod wire;
mod wire_value;
pub use wire_value::{WireError, WireErrorKind};

pub use admission::{
    AdmissionRegistry, AdmissionRejection, AdmittedHandler, KindRegistrationOf, Outcome,
    ServerSessionOf, SessionAdmission,
};
pub use authoring_snapshot::{CompactedDeclarationRejection, decode_compacted};
pub use command_fsm::{InjectionLedger, InjectionLookup, InjectionTransitionError};
pub use declaration_payload::{
    replace_authoring_environment_from_value, retire_actor_from_value,
    retire_annotation_from_value, retire_edge_from_value, retire_export_mount_from_value,
    retire_scope_from_value, set_flags_from_value, set_presentation_from_value,
    upsert_actor_from_value, upsert_annotation_from_value, upsert_edge_from_value,
    upsert_export_mount_from_value, upsert_scope_from_value,
};
pub use dispatch_handler::DispatchHandler;
pub use dispatcher::{
    DeclarationPort, EventInjectionPort, LedgerTransitionPort, LifecyclePort, QueryPort,
    SemanticDispatcher, SubscriptionPort,
};
pub use envelope::{CorrelationId, Envelope, ProtocolEnvelope};
pub use epoch_fsm::{
    BeginEpochError, BeginEpochErrorCodecError, EpochBook, EpochContentError, EpochTransitionError,
    OpenEpoch, decode_begin_epoch_rejection, encode_begin_epoch_rejection,
};
pub use identity::{
    AuthoringRevision, REVISION_DIGEST_BYTES, RevisionDigest, RevisionKind, RevisionWidthMismatch,
    SESSION_TOKEN_BITS, SESSION_TOKEN_BYTES, SessionToken, SessionTokenIssueError,
    SessionTokenIssuer, SessionTokenSource, SessionTokenWidthMismatch, SessionTokensExhausted,
    TopologyRevision,
};
pub use kind::{
    DeclarationVerb, EventInjectionVerb, Kind, LedgerTransitionVerb, LifecycleVerb, Partition,
    QueryVerb, ReplayControlVerb, STABLE_VERB_COUNT, SessionMechanicsVerb, SubscriptionVerb,
};
pub use lifecycle_payload::{
    LifecyclePayloadRejection, LifecycleRequest, LifecycleResult, WireLifecycleDomain,
    decode_pause, decode_resume,
};
pub use message::{
    Anchor, CommandResult, CreditError, Cursor, DeclarationCommand, DeclarationDomain,
    DeliveryDiscipline, FrameOrigin, Injection, LedgerDomain, LedgerTransition, Lifecycle,
    LifecycleAccepted, LifecycleDomain, NoTerminationPayload, PageEnd, PageStep, PauseMode,
    PositiveCredit, PositiveFrameCount, QueryPage, QueryRequest, QueryResult, Subscribe,
    SubscriptionEndReason, SubscriptionEnded, SubscriptionFrame, SubscriptionTerminationDomain,
    Target,
};
pub use payload::{
    EstablishedSessionOf, ExperimentalPayload, HelloOf, LifecycleAcceptedOf, QueryPageOf,
    QueryRequestOf, ReplayControlPayload, SessionDomain, SessionEnvelope, SessionPayload,
};
pub use query_fsm::{QueryCursorError, QueryCursorFsm, QueryPhase};
pub use session::{
    DuplicateFeature, EstablishedSession, EstablishedSessionValue, FeatureSet, Hello, HelloAck,
    MissingReader, SessionRole, SessionRoles, SessionState, TransportTrust,
};
pub use session_fsm::{
    EstablishmentPolicy, EstablishmentRejection, IncompleteFeatureSet, KindRegistration,
    KindRegistrationError, ReceiveHelloResult, RoleRequirement, ScopeCoverage, ServerHelloAck,
    ServerSession, SessionPhase, SessionTransitionError, accepts, require_complete_features,
};
pub use subscription_fsm::{
    CreditBalance, SubscriptionAction, SubscriptionFsm, SubscriptionPhase, SubscriptionTermination,
    SubscriptionTransitionError,
};
pub use wire::{
    Body, CORRELATION_BYTES, DecodedEnvelopeFrame, ENVELOPE_HEADER_BYTES, EnvelopeHeader,
    FrameRejection, HeaderRejection, INITIAL_PROTOCOL_VERSION, LiveCorrelations,
    MAX_LIVE_CORRELATIONS, PARTITION_TAG_BYTES, PAYLOAD_LENGTH_BYTES, PROTOCOL_VERSION_BYTES,
    RESERVED_CAPABILITY_PARTITION_TAG, RESERVED_CAPABILITY_VERB_TAG_COUNT,
    RESERVED_CAPABILITY_VERB_TAG_END, RESERVED_CAPABILITY_VERB_TAG_FIRST, RETIRED_PARTITION_TAGS,
    RETIRED_VERB_TAGS, StableVerb, VERB_TAG_APPEND_FRONTIER, VERB_TAG_BYTES,
    VERSION_INVARIANT_PREFIX_BYTES, VersionInvariantPrefix, decode_envelope_frame, decode_header,
    decode_version_invariant_prefix, encode_envelope_frame, encode_header, partition_tag,
    verb_from_tag, verb_tag,
};
