/*! Generated from the Rust declarations by crates/engine/src/closed_tables.rs. Do not edit.
 * Regenerate: cargo run --locked -p engine --example closed_tables */
export const Partition = [
  {
    "name": "SessionMechanics",
    "tag": 1
  },
  {
    "name": "Declaration",
    "tag": 2
  },
  {
    "name": "Query",
    "tag": 3
  },
  {
    "name": "Subscription",
    "tag": 4
  },
  {
    "name": "EventInjection",
    "tag": 5
  },
  {
    "name": "LedgerTransition",
    "tag": 6
  },
  {
    "name": "ReplayControl",
    "tag": 8
  },
  {
    "name": "Experimental",
    "tag": 9
  },
  {
    "name": "Lifecycle",
    "tag": 11
  }
];
export const RESERVED_CAPABILITY_PARTITION_TAG = 10;
export const StableVerb = [
  {
    "body": "Value",
    "name": "Hello",
    "partition": "SessionMechanics",
    "tag": 1
  },
  {
    "body": "Value",
    "name": "HelloAck",
    "partition": "SessionMechanics",
    "tag": 2
  },
  {
    "body": "Absent",
    "name": "Goodbye",
    "partition": "SessionMechanics",
    "tag": 3
  },
  {
    "body": "Value",
    "name": "BeginEpoch",
    "partition": "Declaration",
    "tag": 4
  },
  {
    "body": "Value",
    "name": "ValidateEpoch",
    "partition": "Declaration",
    "tag": 5
  },
  {
    "body": "Value",
    "name": "CommitEpoch",
    "partition": "Declaration",
    "tag": 6
  },
  {
    "body": "Value",
    "name": "AbortEpoch",
    "partition": "Declaration",
    "tag": 7
  },
  {
    "body": "Value",
    "name": "UpsertActor",
    "partition": "Declaration",
    "tag": 8
  },
  {
    "body": "Value",
    "name": "RetireActor",
    "partition": "Declaration",
    "tag": 9
  },
  {
    "body": "Value",
    "name": "UpsertEdge",
    "partition": "Declaration",
    "tag": 10
  },
  {
    "body": "Value",
    "name": "RetireEdge",
    "partition": "Declaration",
    "tag": 11
  },
  {
    "body": "Value",
    "name": "UpsertScope",
    "partition": "Declaration",
    "tag": 12
  },
  {
    "body": "Value",
    "name": "RetireScope",
    "partition": "Declaration",
    "tag": 13
  },
  {
    "body": "Value",
    "name": "MoveToScope",
    "partition": "Declaration",
    "tag": 80
  },
  {
    "body": "Value",
    "name": "UpsertExportMount",
    "partition": "Declaration",
    "tag": 14
  },
  {
    "body": "Value",
    "name": "RetireExportMount",
    "partition": "Declaration",
    "tag": 15
  },
  {
    "body": "Value",
    "name": "UpsertAnnotation",
    "partition": "Declaration",
    "tag": 16
  },
  {
    "body": "Value",
    "name": "RetireAnnotation",
    "partition": "Declaration",
    "tag": 17
  },
  {
    "body": "Value",
    "name": "SetPresentation",
    "partition": "Declaration",
    "tag": 18
  },
  {
    "body": "Value",
    "name": "SetFlags",
    "partition": "Declaration",
    "tag": 19
  },
  {
    "body": "Value",
    "name": "UpsertTemplate",
    "partition": "Declaration",
    "tag": 83
  },
  {
    "body": "Value",
    "name": "RetireTemplate",
    "partition": "Declaration",
    "tag": 84
  },
  {
    "body": "Value",
    "name": "CommandResult",
    "partition": "Declaration",
    "tag": 21
  },
  {
    "body": "Value",
    "name": "Query",
    "partition": "Query",
    "tag": 22
  },
  {
    "body": "Value",
    "name": "QueryResult",
    "partition": "Query",
    "tag": 23
  },
  {
    "body": "Absent",
    "name": "QueryClose",
    "partition": "Query",
    "tag": 85
  },
  {
    "body": "Value",
    "name": "Subscribe",
    "partition": "Subscription",
    "tag": 24
  },
  {
    "body": "Value",
    "name": "SubscribeAck",
    "partition": "Subscription",
    "tag": 25
  },
  {
    "body": "Value",
    "name": "Credit",
    "partition": "Subscription",
    "tag": 26
  },
  {
    "body": "Absent",
    "name": "Unsubscribe",
    "partition": "Subscription",
    "tag": 27
  },
  {
    "body": "Value",
    "name": "Frame",
    "partition": "Subscription",
    "tag": 28
  },
  {
    "body": "Value",
    "name": "SubscriptionEnded",
    "partition": "Subscription",
    "tag": 29
  },
  {
    "body": "Value",
    "name": "Inject",
    "partition": "EventInjection",
    "tag": 30
  },
  {
    "body": "Value",
    "name": "InjectAck",
    "partition": "EventInjection",
    "tag": 31
  },
  {
    "body": "Value",
    "name": "ApprovalDecide",
    "partition": "LedgerTransition",
    "tag": 32
  },
  {
    "body": "Value",
    "name": "SetObservationControl",
    "partition": "LedgerTransition",
    "tag": 33
  },
  {
    "body": "Value",
    "name": "TransitionResult",
    "partition": "LedgerTransition",
    "tag": 34
  },
  {
    "body": "Value",
    "name": "SetAgentHarness",
    "partition": "LedgerTransition",
    "tag": 86
  },
  {
    "body": "Value",
    "name": "ReplayStart",
    "partition": "ReplayControl",
    "tag": 41
  },
  {
    "body": "Value",
    "name": "ReplayRewind",
    "partition": "ReplayControl",
    "tag": 42
  },
  {
    "body": "Absent",
    "name": "ReplayEnd",
    "partition": "ReplayControl",
    "tag": 43
  },
  {
    "body": "Value",
    "name": "ReplayResult",
    "partition": "ReplayControl",
    "tag": 44
  },
  {
    "body": "Value",
    "name": "Resume",
    "partition": "Lifecycle",
    "tag": 77
  },
  {
    "body": "Value",
    "name": "Pause",
    "partition": "Lifecycle",
    "tag": 78
  },
  {
    "body": "Value",
    "name": "LifecycleResult",
    "partition": "Lifecycle",
    "tag": 79
  }
];
export const RESERVED_CAPABILITY_VERB_TAG_FIRST = 45;
export const RESERVED_CAPABILITY_VERB_TAG_COUNT = 32;
export const DeclarationCommand = [
  "BeginEpoch",
  "ValidateEpoch",
  "CommitEpoch",
  "AbortEpoch",
  "UpsertActor",
  "RetireActor",
  "UpsertEdge",
  "RetireEdge",
  "UpsertScope",
  "RetireScope",
  "MoveToScope",
  "UpsertExportMount",
  "RetireExportMount",
  "UpsertAnnotation",
  "RetireAnnotation",
  "SetPresentation",
  "SetFlags",
  "UpsertTemplate",
  "RetireTemplate"
];
export const RejectionReason = [
  {
    "name": "Malformed",
    "numbers": [
      1,
      1
    ]
  },
  {
    "name": "Unresolved",
    "numbers": [
      2,
      null
    ]
  },
  {
    "name": "MalformedPayload",
    "numbers": [
      3,
      1
    ]
  },
  {
    "name": "HelloRequired",
    "numbers": [
      4,
      10
    ]
  },
  {
    "name": "SessionClosed",
    "numbers": [
      5,
      11
    ]
  },
  {
    "name": "UnexpectedHello",
    "numbers": [
      6,
      null
    ]
  },
  {
    "name": "OutboundOnly",
    "numbers": [
      7,
      12
    ]
  },
  {
    "name": "RoleInsufficient",
    "numbers": [
      8,
      13
    ]
  },
  {
    "name": "ProtocolVersionMismatch",
    "numbers": [
      9,
      null
    ]
  },
  {
    "name": "IncompleteFeatures",
    "numbers": [
      10,
      null
    ]
  },
  {
    "name": "OwnerLocalRequired",
    "numbers": [
      11,
      null
    ]
  },
  {
    "name": "EntropyUnavailable",
    "numbers": [
      15,
      null
    ]
  },
  {
    "name": "GrantedUnrequestedRole",
    "numbers": [
      16,
      null
    ]
  },
  {
    "name": "LiveCorrelationUnavailable",
    "numbers": [
      17,
      14
    ]
  },
  {
    "name": "QueryResultEncodingFailed",
    "numbers": [
      19,
      null
    ]
  },
  {
    "name": "NoStandingPipeline",
    "numbers": [
      20,
      5
    ]
  },
  {
    "name": "InputNotAccepted",
    "numbers": [
      21,
      null
    ]
  },
  {
    "name": "ActivationFailed",
    "numbers": [
      22,
      null
    ]
  },
  {
    "name": "ArrivalRecorderStopped",
    "numbers": [
      23,
      null
    ]
  },
  {
    "name": "RevisionAdoptionFailed",
    "numbers": [
      24,
      null
    ]
  },
  {
    "name": "RevisionConflict",
    "numbers": [
      26,
      null
    ]
  },
  {
    "name": "JournalFormatRejected",
    "numbers": [
      28,
      null
    ]
  },
  {
    "name": "ClosedByClient",
    "numbers": [
      29,
      null
    ]
  },
  {
    "name": "RecoveryFailed",
    "numbers": [
      30,
      null
    ]
  },
  {
    "name": "EndedBeforeAnswering",
    "numbers": [
      31,
      null
    ]
  },
  {
    "name": "UnknownHarness",
    "numbers": [
      6401,
      null
    ]
  },
  {
    "name": "InvalidProgramPath",
    "numbers": [
      6402,
      null
    ]
  },
  {
    "name": "ProgramNotExecutable",
    "numbers": [
      6403,
      null
    ]
  },
  {
    "name": "AuthoringUnavailable",
    "numbers": [
      null,
      2
    ]
  },
  {
    "name": "AuthoringRevisionMismatch",
    "numbers": [
      null,
      3
    ]
  },
  {
    "name": "AlreadyRunning",
    "numbers": [
      null,
      4
    ]
  },
  {
    "name": "RuntimeOpenFailed",
    "numbers": [
      null,
      7
    ]
  },
  {
    "name": "LifecyclePersistenceFailed",
    "numbers": [
      null,
      8
    ]
  },
  {
    "name": "DesiredRunningUnavailable",
    "numbers": [
      null,
      9
    ]
  }
];
export const LifecycleWord = [
  {
    "as_str": "running",
    "pipeline_stands": true
  },
  {
    "as_str": "stopped",
    "pipeline_stands": true
  },
  {
    "as_str": "activation_failed",
    "pipeline_stands": false
  },
  {
    "as_str": "revision_adoption_failed",
    "pipeline_stands": true
  },
  {
    "as_str": "recovery_failed",
    "pipeline_stands": false
  }
];
export const BuiltinObservationName = [
  {
    "as_str": "DeliveryOccurrence",
    "name": "DeliveryOccurrence",
    "tag": 0
  },
  {
    "as_str": "DeliveryException",
    "name": "DeliveryException",
    "tag": 1
  },
  {
    "as_str": "ProcessingDisposition",
    "name": "ProcessingDisposition",
    "tag": 2
  },
  {
    "as_str": "AdmissionDelay",
    "name": "AdmissionDelay",
    "tag": 3
  },
  {
    "as_str": "DeadLetterEntry",
    "name": "DeadLetterEntry",
    "tag": 4
  },
  {
    "as_str": "EffectItem",
    "name": "EffectItem",
    "tag": 5
  },
  {
    "as_str": "EffectFailed",
    "name": "EffectFailed",
    "tag": 6
  },
  {
    "as_str": "IncarnationTransition",
    "name": "IncarnationTransition",
    "tag": 7
  },
  {
    "as_str": "ActivationOutcome",
    "name": "ActivationOutcome",
    "tag": 8
  },
  {
    "as_str": "ApprovalSettlement",
    "name": "ApprovalSettlement",
    "tag": 9
  },
  {
    "as_str": "OperationTransition",
    "name": "OperationTransition",
    "tag": 10
  },
  {
    "as_str": "CausalChain",
    "name": "CausalChain",
    "tag": 11
  },
  {
    "as_str": "BoundaryDelayExceeded",
    "name": "BoundaryDelayExceeded",
    "tag": 12
  },
  {
    "as_str": "EditAttribution",
    "name": "EditAttribution",
    "tag": 14
  },
  {
    "as_str": "RoutingDecision",
    "name": "RoutingDecision",
    "tag": 15
  },
  {
    "as_str": "CustodyClaim",
    "name": "CustodyClaim",
    "tag": 16
  },
  {
    "as_str": "SessionTransition",
    "name": "SessionTransition",
    "tag": 17
  },
  {
    "as_str": "SubscriptionTransition",
    "name": "SubscriptionTransition",
    "tag": 18
  },
  {
    "as_str": "StoreAppend",
    "name": "StoreAppend",
    "tag": 19
  },
  {
    "as_str": "RegistrationLoaded",
    "name": "RegistrationLoaded",
    "tag": 20
  },
  {
    "as_str": "DiagnosticOccurrence",
    "name": "DiagnosticOccurrence",
    "tag": 21
  },
  {
    "as_str": "actor",
    "name": "Actor",
    "tag": 22
  },
  {
    "as_str": "incarnation",
    "name": "Incarnation",
    "tag": 23
  },
  {
    "as_str": "scope",
    "name": "Scope",
    "tag": 24
  },
  {
    "as_str": "operation",
    "name": "Operation",
    "tag": 25
  },
  {
    "as_str": "replay_session",
    "name": "ReplaySession",
    "tag": 26
  },
  {
    "as_str": "tally",
    "name": "Tally",
    "tag": 27
  },
  {
    "as_str": "plane",
    "name": "Plane",
    "tag": 28
  },
  {
    "as_str": "progress",
    "name": "Progress",
    "tag": 29
  },
  {
    "as_str": "density",
    "name": "Density",
    "tag": 30
  },
  {
    "as_str": "floor",
    "name": "Floor",
    "tag": 31
  },
  {
    "as_str": "edge",
    "name": "Edge",
    "tag": 32
  },
  {
    "as_str": "binding",
    "name": "Binding",
    "tag": 33
  },
  {
    "as_str": "storage",
    "name": "Storage",
    "tag": 34
  },
  {
    "as_str": "boundary_growth",
    "name": "BoundaryGrowth",
    "tag": 35
  },
  {
    "as_str": "instance_transition",
    "name": "InstanceTransition",
    "tag": 36
  },
  {
    "as_str": "restart",
    "name": "Restart",
    "tag": 38
  },
  {
    "as_str": "daemon_shutdown",
    "name": "DaemonShutdown",
    "tag": 39
  },
  {
    "as_str": "stream_start",
    "name": "StreamStart",
    "tag": 40
  },
  {
    "as_str": "system_activation_outcome",
    "name": "SystemActivationOutcome",
    "tag": 41
  },
  {
    "as_str": "system_revision_adoption_outcome",
    "name": "SystemRevisionAdoptionOutcome",
    "tag": 42
  },
  {
    "as_str": "system_recovery_outcome",
    "name": "SystemRecoveryOutcome",
    "tag": 43
  },
  {
    "as_str": "system_pause_accepted",
    "name": "SystemPauseAccepted",
    "tag": 44
  },
  {
    "as_str": "system_resume_accepted",
    "name": "SystemResumeAccepted",
    "tag": 45
  }
];
export const ActorHealthState = [
  "running",
  "waiting",
  "backpressure",
  "failed",
  "stopped"
];
export const ActorHealthReasonCode = [
  "outcome_unclaimed",
  "destination_gone",
  "poisoned",
  "declared",
  "parameter_denied",
  "transport_terminal",
  "approval_required",
  "diverged",
  "endpoint_gone",
  "interpreter_fault",
  "peer",
  "source_failure",
  "capacity",
  "activation_failed",
  "activation_witness_missing",
  "activation_registration_failed",
  "recovery_refused",
  "kernel_fault",
  "harness_unbound",
  "harness_unusable"
];
export const ApprovalDecisionKind = [
  "approved",
  "denied"
];
export const PreprocessKind = [
  "map",
  "filter",
  "bang",
  "parse",
  "flatten"
];
export const DeadLetterReasonKind = [
  "processing",
  "outcome_unclaimed",
  "destination_gone",
  "poisoned",
  "actor_declared",
  "capacity"
];
export const LifecyclePhase = [
  {
    "name": "Draining",
    "tag": 1
  },
  {
    "name": "AdmissionClosed",
    "tag": 2
  },
  {
    "name": "Terminated",
    "tag": 3
  },
  {
    "name": "Prepared",
    "tag": 4
  },
  {
    "name": "Activated",
    "tag": 5
  },
  {
    "name": "Restarted",
    "tag": 6
  },
  {
    "name": "ConfigRestarted",
    "tag": 7
  },
  {
    "name": "Abandoned",
    "tag": 8
  },
  {
    "name": "ResumeDenied",
    "tag": 9
  },
  {
    "name": "ConfigApplied",
    "tag": 10
  }
];
export const SubscriptionFrame = [
  {
    "name": "Lossless",
    "tag": 1
  },
  {
    "name": "Conflated",
    "tag": 2
  },
  {
    "name": "Credit",
    "tag": 3
  },
  {
    "name": "RetentionComplete",
    "tag": 4
  }
];
export const FrameOrigin = [
  {
    "name": "Retained",
    "tag": 1
  },
  {
    "name": "Live",
    "tag": 2
  }
];
export const SubscriptionEndReason = [
  {
    "name": "ByClient",
    "tag": 1
  },
  {
    "name": "ConsumerBehind",
    "tag": 2
  },
  {
    "name": "TargetGone",
    "tag": 3
  },
  {
    "name": "Withdrawn",
    "tag": 4
  },
  {
    "name": "SessionClosed",
    "tag": 5
  },
  {
    "name": "ResetRequired",
    "tag": 6
  },
  {
    "name": "ScopeGone",
    "tag": 7
  },
  {
    "name": "IncompatibleClient",
    "tag": 8
  },
  {
    "name": "Complete",
    "tag": 9
  }
];
export const Ceilings = {
  "Identity": {
    "max_bytes": 1048576,
    "max_container_entries": 65535,
    "max_depth": 64,
    "max_string_bytes": 1048576
  },
  "Wire": {
    "max_bytes": 1048576,
    "max_container_entries": 65535,
    "max_depth": 64,
    "max_string_bytes": 1048576
  }
};
export const MAX_SEGMENT_BODY_BYTES = 65536;
export const MAX_REASSEMBLED_BODY_BYTES = 16777216;
export const QueryId = {
  "ActorCatalog": {
    "name": "actor.catalog",
    "paging": "None"
  },
  "ActorConfigurationAdmission": {
    "name": "actor.configure-admission",
    "paging": "None"
  },
  "ActorCreateAdmission": {
    "name": "actor.create-admission",
    "paging": "None"
  },
  "ActorCreateInputs": {
    "name": "actor.create-inputs",
    "paging": "None"
  },
  "ActorEvents": {
    "name": "actor.events",
    "paging": "OptionalImmutableCursor"
  },
  "AgentHarnessCandidates": {
    "name": "agent.harness-candidates",
    "paging": "None"
  },
  "AgentHarnesses": {
    "name": "agent.harnesses",
    "paging": "None"
  },
  "ArrivalScan": {
    "name": "arrival.scan",
    "paging": "OptionalImmutableCursor"
  },
  "AuthoringActorAccess": {
    "name": "authoring.actor-access",
    "paging": "None"
  },
  "AuthoringActorPorts": {
    "name": "authoring.actor-ports",
    "paging": "None"
  },
  "AuthoringSnapshot": {
    "name": "authoring-snapshot",
    "paging": "OptionalRetainedCursor"
  },
  "Catalog": {
    "name": "query.catalog",
    "paging": "None"
  },
  "DaemonHealth": {
    "name": "daemon.health",
    "paging": "None"
  },
  "DeadLetters": {
    "name": "dead.letters",
    "paging": "OptionalImmutableCursor"
  },
  "ObservationScan": {
    "name": "observation-scan",
    "paging": "OptionalImmutableCursor"
  },
  "Pipelines": {
    "name": "pipelines",
    "paging": "None"
  },
  "Presentation": {
    "name": "structure.presentation",
    "paging": "None"
  },
  "Records": {
    "name": "records",
    "paging": "OptionalImmutableCursor"
  },
  "Rollup": {
    "name": "display.rollup",
    "paging": "None"
  },
  "RuntimeApprovals": {
    "name": "runtime.approvals",
    "paging": "None"
  },
  "Timeline": {
    "name": "timeline",
    "paging": "OptionalImmutableCursor"
  },
  "TimelineAt": {
    "name": "timeline.at",
    "paging": "None"
  },
  "TimelineBins": {
    "name": "timeline.bins",
    "paging": "None"
  },
  "Transitions": {
    "name": "instance.transitions",
    "paging": "OptionalImmutableCursor"
  }
};
export const SubscriptionTarget = {
  "ActorEvents": {
    "delivery": "Credit",
    "name": "actor.events"
  },
  "AuthoringCommits": {
    "delivery": "Credit",
    "name": "authoring-commits"
  },
  "DisplayFrames": {
    "delivery": "Credit",
    "name": "display.frames"
  },
  "EdgeDepths": {
    "delivery": "Credit",
    "name": "edge.depths"
  },
  "Records": {
    "delivery": "Credit",
    "name": "records"
  }
};
export const TimelineMarkKind = [
  "edit",
  "restart",
  "pause",
  "resume"
];
export const BaseShape = [
  "null",
  "bool",
  "int",
  "float",
  "string",
  "bytes",
  "uint"
];
export const EvalMode = [
  "transform",
  "predicate",
  "number",
  "reduce"
];
