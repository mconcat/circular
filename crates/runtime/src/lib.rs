#![forbid(unsafe_code)]

mod actor;
mod agent;
mod approval;
mod arrival;
mod capability;
mod effect;
mod effect_identity;
mod effect_identity_codec;
mod effect_term_codec;
mod failure;
mod folded_config;
mod instance;
mod interpreter;
mod keyed;
mod lifecycle;
mod lookup;
mod outcome;
mod outcome_codec;
mod peer;
mod peer_bridge;
pub mod product_identity;
mod reason;
mod routing_decision;

pub use effect_identity::{EffectId, EffectOccasion};
pub use effect_identity_codec::{decode_effect_stamp, encode_effect_stamp};

pub use actor::{
    Actor, ActorContext, ActorFactory, ActorInput, ActorLifecycle, ActorTypes, EditDisposition,
    EmittingActor, EmittingActorFactory, EnvelopeResult, SourceActor, SourceFactory, SourcePoll,
    WakeCondition,
};
pub use agent::{
    AgentHarnessName, AgentInvokeSpec, AgentInvokeSpecError, AgentPayload, AgentProgressRecord,
    AgentSessionId, AgentStepNext, AgentStepRequest, AgentStepRequestError, AgentStepResult,
    AgentStepResultError, AgentToolCall, AgentToolCallId, AgentToolResult,
    ConcreteExternalEffectTag, ConcreteOutcomeMismatch, ConcreteOutcomePayload, EmptyAgentName,
    EmptyAgentToolCallId, ToolName,
};
pub use approval::{
    ApprovalConsumeDisposition, ApprovalConsumeRejection, ApprovalDecision,
    ApprovalDecisionDisposition, ApprovalMachine, ApprovalRequestOutcome, ApprovalSpec,
    ApprovalState, ApprovalTicket, OpenApprovalState,
};
pub use arrival::{
    ActorArrivals, ArrivalOrigin, ArrivalSource, ArrivalStep, ArrivalStepError, ArrivalStepOutput,
    Checked, ConsumeStep, EmptyExternalOrigin, ExternalOrigin, IndexRule, IngressEdges, LogCut,
    RecordStep, RecordedArrival,
};
#[cfg(any(test, feature = "test-support"))]
pub use capability::GrantIssuer;
pub use capability::{
    AccessAllowance, AccessBoundary, AgentHarness, AgentHarnessAuthorityBearer, AgentHarnessGrant,
    AgentHarnessNames, ApprovalRequest, Capability, CapabilityGrant, CapabilitySet, Ceiling,
    CivilTime, CompleteAccessBoundary, CpuTime, EffectCapability, FilesystemAuthorityBearer,
    FsRead, FsReadGrant, FsWrite, FsWriteGrant, Granted, HostedDelegation, HttpFetch,
    HttpFetchAuthorityBearer, HttpFetchGrant, HttpHosts, IsolationBoundary, MemoryBytes,
    ModelProvider, NetworkListen, NetworkOutbound, NormalizedPath, NotificationChannels,
    PathNormalizationError, PathScope, PathScopes, PeerAdvertise, PeerAdvertiseGrant,
    PeerAdvertisementScope, PeerAdvertisementScopes, PeerAuthorityBearer, PeerConnect,
    PeerDiscover, PeerDiscoverGrant, PeerRealmScope, PeerRealmScopes, PeerReceive,
    PeerReceiveGrant, PeerSend, PeerSendGrant, PeerSenderScopes, PeerTargetScopes,
    ProcessAuthorityBearer, ProcessCount, ProcessSpawn, ProcessSpawnGrant, ProcessSpawnParameters,
    ProcessTargets, ResourceCeilings, ResourceUsage, UserNotify, UserNotifyAuthorityBearer,
    UserNotifyGrant, WorkspaceProcessAccess, WorkspaceProcessGrant, authority_admits,
    authority_entry_is_valid, authority_host,
};
pub use circular_core::{Millis, NonZeroMillis, RecordedInstant, ZeroMillisError};
pub use circular_plan::PreprocessKind;
pub use circular_plan::{
    ActorId, Config, EdgeId, Endpoint, InstanceKey, InstanceScalar, LocalKey, Name, NamedActorId,
    ScopeId, ScopeSeg, SystemActor,
};
pub use circular_plan::{AdmittedTemplate, CellDerivationError};
pub use effect::{
    ActorEffect, ActorEffects, ByteRange, Effect, EffectCtor, Effects, FileReadSpec, FileWriteMode,
    FileWriteSpec, HttpHeader, HttpHeaderError, HttpHeaderValue, HttpMethod, HttpRequestSpec,
    HttpRequestSpecError, HttpUrl, HttpUrlError, NotificationChannel, NotificationSpec,
    ProcessSpec, ProgramName, ScheduleCorrelation, ScheduleSpec, caps, required_capability,
};
pub use effect_term_codec::{
    TERM_CODEC_VERSION, TermApproval, TermCodecError, decode_term, decode_term_approval,
    encode_term, try_encode_term,
};
pub use failure::{
    DeadLetterLane, DeadLetterLaneActivationError, DeadLetterLanes, DeadLetterOrigin,
    DeadLetterReason, DeadLetterRecord, DeadLetterTarget, IncarnationState, IncarnationTransition,
    IncarnationTransitionError, PreprocessFailurePoint, ProcessingCause, ProcessingFailureEmission,
    ProcessingFailureRoute, RestartBudget, RestartBudgetError, RestartController,
    RestartDisposition, RestartParameters, RestartableActorFailure, classify_effect_failure,
    processing_letter, route_processing_failure,
};
pub use folded_config::{FoldedConfig, FoldedConfigMismatch};
pub use instance::{
    InstanceAuthority, InstanceAuthorityBearer, InstanceDisposition, InstanceGovernor,
    InstanceIntent, InstanceMutationSpec, InstanceRegistry,
};
pub use interpreter::{
    ApprovalGate, ApprovalGateDisposition, EffectTerm, Interpreter, LiveInterpreter,
    RecordedOutcome, SubmitError, VirtualizedBuildError, VirtualizedHistoryResidual,
    VirtualizedInterpreter,
};
pub use keyed::{KeyedDelivery, KeyedEmission, KeyedInlet};
pub use lifecycle::{
    ActorRestoreError, ActorState, Checkpoint, CheckpointRestore, ConfigChangeOutcome,
    EditableActor,
};
pub use lookup::{
    ConsumedSet, DivergenceKind, LoggedOutcome, LookupKey, LookupResult, OutcomeLog,
    OutcomeLogBuildError,
};
pub use outcome::{
    Divergence, EffectFailure, EffectOutcome, HttpResponse, InterpreterFault, NotificationReceipt,
    OutcomePayload, ProcessResult,
};
pub use outcome_codec::{OUTCOME_CODEC_VERSION, OutcomeCodecError, decode_outcome, encode_outcome};
pub use peer::{
    AdmittedPeerEvent, BindRequest, BindingLease, CrashDurablePeerOutcomeArrival,
    DeliveryDisposition, DiscoverRequest, DurablyCommittedPeerEvent, EmptyPeerIdentifier,
    ExternalPeerSend, InboundPolicy, MemoryPeerAdapter, MemoryPeerContactCounts,
    OpaqueProviderFields, Peer, PeerAcknowledgeError, PeerAcknowledgeFailure, PeerActorIncarnation,
    PeerAdapter, PeerAdapterCapabilities, PeerAdapterName, PeerAddress, PeerAdvertisementMode,
    PeerAvailability, PeerBinding, PeerBindingCapabilities, PeerBindingId, PeerBindingState,
    PeerBody, PeerCapabilities, PeerCursor, PeerDiagnostic, PeerDiscoveryMode, PeerDisplayName,
    PeerEffect, PeerEffectTerm, PeerEvent, PeerEventEnvelope, PeerEventStream, PeerFailure,
    PeerFailureKind, PeerId, PeerIngressCommitError, PeerKind, PeerMessage, PeerMessageId,
    PeerOutcomeArrivalReceipt, PeerOutcomeArrivalStore, PeerProvenance, PeerRealmId,
    PeerReceiveCursorMode, PeerReplayTape, PeerReplyAddressMode, PeerResolveError, PeerSnapshot,
    PeerStamp, PeerTextError, ProviderFieldsError, ProviderIngressReceipt, ProviderMessageId,
    SendRequest, SubmissionReceipt, SubmissionReceiptError, UnbindReceipt, commit_peer_event,
    decode_peer_envelope, encode_peer_envelope,
};
pub use peer_bridge::{
    BridgePeerAdapter, InvalidBridgeContractVersion, PeerBridgeContractVersion, ProviderPeerBridge,
    VersionedPeerBridge,
};
pub use reason::{
    DeadLettering, DeclaredReason, Reason, ReasonDecl, ReasonDeclError, Suppression,
    SuppressionReason,
};
pub use routing_decision::{
    CapacityDecision, DiscardedFlag, EmissionClass, EmissionDisposition, EmissionGate,
    InputFlagDecision, InputFlagDecisionError, decide_capacity, decide_input_flags,
};

#[cfg(test)]
pub(crate) fn test_effect(index: u64) -> EffectId {
    let actor = NamedActorId::new(ScopeId::root(), Name::from_normalized("s"));
    let arrival = circular_core::Stamp::from_event_producer_at(
        circular_core::Hlc::new(
            circular_core::Tick::new(5),
            circular_core::LogicalCounter::new(2),
        ),
        actor.clone(),
        circular_core::Sequence::new(7).expect("positive sequence"),
        circular_core::RevisionEpochId::new(1).expect("first revision"),
    );
    EffectId::from_components(
        actor.into(),
        vec![circular_plan::Generation::new(2)].into_boxed_slice(),
        EffectOccasion::Delivery(None, arrival),
        index,
    )
    .unwrap()
}

pub mod failure_value;
