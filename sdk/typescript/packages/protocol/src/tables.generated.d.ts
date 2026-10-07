/*! Generated from the Rust declarations by crates/engine/src/closed_tables.rs. Do not edit.
 * Regenerate: cargo run --locked -p engine --example closed_tables */
export declare const Partition: readonly [
  {
    readonly "name": "SessionMechanics";
    readonly "tag": 1;
  },
  {
    readonly "name": "Declaration";
    readonly "tag": 2;
  },
  {
    readonly "name": "Query";
    readonly "tag": 3;
  },
  {
    readonly "name": "Subscription";
    readonly "tag": 4;
  },
  {
    readonly "name": "EventInjection";
    readonly "tag": 5;
  },
  {
    readonly "name": "LedgerTransition";
    readonly "tag": 6;
  },
  {
    readonly "name": "ReplayControl";
    readonly "tag": 8;
  },
  {
    readonly "name": "Experimental";
    readonly "tag": 9;
  },
  {
    readonly "name": "Lifecycle";
    readonly "tag": 11;
  },
];
export declare const RESERVED_CAPABILITY_PARTITION_TAG: 10;
export declare const StableVerb: readonly [
  {
    readonly "body": "Value";
    readonly "name": "Hello";
    readonly "partition": "SessionMechanics";
    readonly "tag": 1;
  },
  {
    readonly "body": "Value";
    readonly "name": "HelloAck";
    readonly "partition": "SessionMechanics";
    readonly "tag": 2;
  },
  {
    readonly "body": "Absent";
    readonly "name": "Goodbye";
    readonly "partition": "SessionMechanics";
    readonly "tag": 3;
  },
  {
    readonly "body": "Value";
    readonly "name": "BeginEpoch";
    readonly "partition": "Declaration";
    readonly "tag": 4;
  },
  {
    readonly "body": "Value";
    readonly "name": "ValidateEpoch";
    readonly "partition": "Declaration";
    readonly "tag": 5;
  },
  {
    readonly "body": "Value";
    readonly "name": "CommitEpoch";
    readonly "partition": "Declaration";
    readonly "tag": 6;
  },
  {
    readonly "body": "Value";
    readonly "name": "AbortEpoch";
    readonly "partition": "Declaration";
    readonly "tag": 7;
  },
  {
    readonly "body": "Value";
    readonly "name": "UpsertActor";
    readonly "partition": "Declaration";
    readonly "tag": 8;
  },
  {
    readonly "body": "Value";
    readonly "name": "RetireActor";
    readonly "partition": "Declaration";
    readonly "tag": 9;
  },
  {
    readonly "body": "Value";
    readonly "name": "UpsertEdge";
    readonly "partition": "Declaration";
    readonly "tag": 10;
  },
  {
    readonly "body": "Value";
    readonly "name": "RetireEdge";
    readonly "partition": "Declaration";
    readonly "tag": 11;
  },
  {
    readonly "body": "Value";
    readonly "name": "UpsertScope";
    readonly "partition": "Declaration";
    readonly "tag": 12;
  },
  {
    readonly "body": "Value";
    readonly "name": "RetireScope";
    readonly "partition": "Declaration";
    readonly "tag": 13;
  },
  {
    readonly "body": "Value";
    readonly "name": "MoveToScope";
    readonly "partition": "Declaration";
    readonly "tag": 80;
  },
  {
    readonly "body": "Value";
    readonly "name": "UpsertExportMount";
    readonly "partition": "Declaration";
    readonly "tag": 14;
  },
  {
    readonly "body": "Value";
    readonly "name": "RetireExportMount";
    readonly "partition": "Declaration";
    readonly "tag": 15;
  },
  {
    readonly "body": "Value";
    readonly "name": "UpsertAnnotation";
    readonly "partition": "Declaration";
    readonly "tag": 16;
  },
  {
    readonly "body": "Value";
    readonly "name": "RetireAnnotation";
    readonly "partition": "Declaration";
    readonly "tag": 17;
  },
  {
    readonly "body": "Value";
    readonly "name": "SetPresentation";
    readonly "partition": "Declaration";
    readonly "tag": 18;
  },
  {
    readonly "body": "Value";
    readonly "name": "SetFlags";
    readonly "partition": "Declaration";
    readonly "tag": 19;
  },
  {
    readonly "body": "Value";
    readonly "name": "UpsertTemplate";
    readonly "partition": "Declaration";
    readonly "tag": 83;
  },
  {
    readonly "body": "Value";
    readonly "name": "RetireTemplate";
    readonly "partition": "Declaration";
    readonly "tag": 84;
  },
  {
    readonly "body": "Value";
    readonly "name": "CommandResult";
    readonly "partition": "Declaration";
    readonly "tag": 21;
  },
  {
    readonly "body": "Value";
    readonly "name": "Query";
    readonly "partition": "Query";
    readonly "tag": 22;
  },
  {
    readonly "body": "Value";
    readonly "name": "QueryResult";
    readonly "partition": "Query";
    readonly "tag": 23;
  },
  {
    readonly "body": "Absent";
    readonly "name": "QueryClose";
    readonly "partition": "Query";
    readonly "tag": 85;
  },
  {
    readonly "body": "Value";
    readonly "name": "Subscribe";
    readonly "partition": "Subscription";
    readonly "tag": 24;
  },
  {
    readonly "body": "Value";
    readonly "name": "SubscribeAck";
    readonly "partition": "Subscription";
    readonly "tag": 25;
  },
  {
    readonly "body": "Value";
    readonly "name": "Credit";
    readonly "partition": "Subscription";
    readonly "tag": 26;
  },
  {
    readonly "body": "Absent";
    readonly "name": "Unsubscribe";
    readonly "partition": "Subscription";
    readonly "tag": 27;
  },
  {
    readonly "body": "Value";
    readonly "name": "Frame";
    readonly "partition": "Subscription";
    readonly "tag": 28;
  },
  {
    readonly "body": "Value";
    readonly "name": "SubscriptionEnded";
    readonly "partition": "Subscription";
    readonly "tag": 29;
  },
  {
    readonly "body": "Value";
    readonly "name": "Inject";
    readonly "partition": "EventInjection";
    readonly "tag": 30;
  },
  {
    readonly "body": "Value";
    readonly "name": "InjectAck";
    readonly "partition": "EventInjection";
    readonly "tag": 31;
  },
  {
    readonly "body": "Value";
    readonly "name": "ApprovalDecide";
    readonly "partition": "LedgerTransition";
    readonly "tag": 32;
  },
  {
    readonly "body": "Value";
    readonly "name": "SetObservationControl";
    readonly "partition": "LedgerTransition";
    readonly "tag": 33;
  },
  {
    readonly "body": "Value";
    readonly "name": "TransitionResult";
    readonly "partition": "LedgerTransition";
    readonly "tag": 34;
  },
  {
    readonly "body": "Value";
    readonly "name": "SetAgentHarness";
    readonly "partition": "LedgerTransition";
    readonly "tag": 86;
  },
  {
    readonly "body": "Value";
    readonly "name": "ReplayStart";
    readonly "partition": "ReplayControl";
    readonly "tag": 41;
  },
  {
    readonly "body": "Value";
    readonly "name": "ReplayRewind";
    readonly "partition": "ReplayControl";
    readonly "tag": 42;
  },
  {
    readonly "body": "Absent";
    readonly "name": "ReplayEnd";
    readonly "partition": "ReplayControl";
    readonly "tag": 43;
  },
  {
    readonly "body": "Value";
    readonly "name": "ReplayResult";
    readonly "partition": "ReplayControl";
    readonly "tag": 44;
  },
  {
    readonly "body": "Value";
    readonly "name": "Resume";
    readonly "partition": "Lifecycle";
    readonly "tag": 77;
  },
  {
    readonly "body": "Value";
    readonly "name": "Pause";
    readonly "partition": "Lifecycle";
    readonly "tag": 78;
  },
  {
    readonly "body": "Value";
    readonly "name": "LifecycleResult";
    readonly "partition": "Lifecycle";
    readonly "tag": 79;
  },
];
export declare const RESERVED_CAPABILITY_VERB_TAG_FIRST: 45;
export declare const RESERVED_CAPABILITY_VERB_TAG_COUNT: 32;
export declare const DeclarationCommand: readonly [
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
  "RetireTemplate",
];
export declare const RejectionReason: readonly [
  {
    readonly "name": "Malformed";
    readonly "numbers": readonly [
      1,
      1,
    ];
  },
  {
    readonly "name": "Unresolved";
    readonly "numbers": readonly [
      2,
      null,
    ];
  },
  {
    readonly "name": "MalformedPayload";
    readonly "numbers": readonly [
      3,
      1,
    ];
  },
  {
    readonly "name": "HelloRequired";
    readonly "numbers": readonly [
      4,
      10,
    ];
  },
  {
    readonly "name": "SessionClosed";
    readonly "numbers": readonly [
      5,
      11,
    ];
  },
  {
    readonly "name": "UnexpectedHello";
    readonly "numbers": readonly [
      6,
      null,
    ];
  },
  {
    readonly "name": "OutboundOnly";
    readonly "numbers": readonly [
      7,
      12,
    ];
  },
  {
    readonly "name": "RoleInsufficient";
    readonly "numbers": readonly [
      8,
      13,
    ];
  },
  {
    readonly "name": "ProtocolVersionMismatch";
    readonly "numbers": readonly [
      9,
      null,
    ];
  },
  {
    readonly "name": "IncompleteFeatures";
    readonly "numbers": readonly [
      10,
      null,
    ];
  },
  {
    readonly "name": "OwnerLocalRequired";
    readonly "numbers": readonly [
      11,
      null,
    ];
  },
  {
    readonly "name": "EntropyUnavailable";
    readonly "numbers": readonly [
      15,
      null,
    ];
  },
  {
    readonly "name": "GrantedUnrequestedRole";
    readonly "numbers": readonly [
      16,
      null,
    ];
  },
  {
    readonly "name": "LiveCorrelationUnavailable";
    readonly "numbers": readonly [
      17,
      14,
    ];
  },
  {
    readonly "name": "QueryResultEncodingFailed";
    readonly "numbers": readonly [
      19,
      null,
    ];
  },
  {
    readonly "name": "NoStandingPipeline";
    readonly "numbers": readonly [
      20,
      5,
    ];
  },
  {
    readonly "name": "InputNotAccepted";
    readonly "numbers": readonly [
      21,
      null,
    ];
  },
  {
    readonly "name": "ActivationFailed";
    readonly "numbers": readonly [
      22,
      null,
    ];
  },
  {
    readonly "name": "ArrivalRecorderStopped";
    readonly "numbers": readonly [
      23,
      null,
    ];
  },
  {
    readonly "name": "RevisionAdoptionFailed";
    readonly "numbers": readonly [
      24,
      null,
    ];
  },
  {
    readonly "name": "RevisionConflict";
    readonly "numbers": readonly [
      26,
      null,
    ];
  },
  {
    readonly "name": "JournalFormatRejected";
    readonly "numbers": readonly [
      28,
      null,
    ];
  },
  {
    readonly "name": "ClosedByClient";
    readonly "numbers": readonly [
      29,
      null,
    ];
  },
  {
    readonly "name": "RecoveryFailed";
    readonly "numbers": readonly [
      30,
      null,
    ];
  },
  {
    readonly "name": "EndedBeforeAnswering";
    readonly "numbers": readonly [
      31,
      null,
    ];
  },
  {
    readonly "name": "UnknownHarness";
    readonly "numbers": readonly [
      6401,
      null,
    ];
  },
  {
    readonly "name": "InvalidProgramPath";
    readonly "numbers": readonly [
      6402,
      null,
    ];
  },
  {
    readonly "name": "ProgramNotExecutable";
    readonly "numbers": readonly [
      6403,
      null,
    ];
  },
  {
    readonly "name": "AuthoringUnavailable";
    readonly "numbers": readonly [
      null,
      2,
    ];
  },
  {
    readonly "name": "AuthoringRevisionMismatch";
    readonly "numbers": readonly [
      null,
      3,
    ];
  },
  {
    readonly "name": "AlreadyRunning";
    readonly "numbers": readonly [
      null,
      4,
    ];
  },
  {
    readonly "name": "RuntimeOpenFailed";
    readonly "numbers": readonly [
      null,
      7,
    ];
  },
  {
    readonly "name": "LifecyclePersistenceFailed";
    readonly "numbers": readonly [
      null,
      8,
    ];
  },
  {
    readonly "name": "DesiredRunningUnavailable";
    readonly "numbers": readonly [
      null,
      9,
    ];
  },
];
export declare const LifecycleWord: readonly [
  {
    readonly "as_str": "running";
    readonly "pipeline_stands": true;
  },
  {
    readonly "as_str": "stopped";
    readonly "pipeline_stands": true;
  },
  {
    readonly "as_str": "activation_failed";
    readonly "pipeline_stands": false;
  },
  {
    readonly "as_str": "revision_adoption_failed";
    readonly "pipeline_stands": true;
  },
  {
    readonly "as_str": "recovery_failed";
    readonly "pipeline_stands": false;
  },
];
export declare const BuiltinObservationName: readonly [
  {
    readonly "as_str": "DeliveryOccurrence";
    readonly "name": "DeliveryOccurrence";
    readonly "tag": 0;
  },
  {
    readonly "as_str": "DeliveryException";
    readonly "name": "DeliveryException";
    readonly "tag": 1;
  },
  {
    readonly "as_str": "ProcessingDisposition";
    readonly "name": "ProcessingDisposition";
    readonly "tag": 2;
  },
  {
    readonly "as_str": "AdmissionDelay";
    readonly "name": "AdmissionDelay";
    readonly "tag": 3;
  },
  {
    readonly "as_str": "DeadLetterEntry";
    readonly "name": "DeadLetterEntry";
    readonly "tag": 4;
  },
  {
    readonly "as_str": "EffectItem";
    readonly "name": "EffectItem";
    readonly "tag": 5;
  },
  {
    readonly "as_str": "EffectFailed";
    readonly "name": "EffectFailed";
    readonly "tag": 6;
  },
  {
    readonly "as_str": "IncarnationTransition";
    readonly "name": "IncarnationTransition";
    readonly "tag": 7;
  },
  {
    readonly "as_str": "ActivationOutcome";
    readonly "name": "ActivationOutcome";
    readonly "tag": 8;
  },
  {
    readonly "as_str": "ApprovalSettlement";
    readonly "name": "ApprovalSettlement";
    readonly "tag": 9;
  },
  {
    readonly "as_str": "OperationTransition";
    readonly "name": "OperationTransition";
    readonly "tag": 10;
  },
  {
    readonly "as_str": "CausalChain";
    readonly "name": "CausalChain";
    readonly "tag": 11;
  },
  {
    readonly "as_str": "BoundaryDelayExceeded";
    readonly "name": "BoundaryDelayExceeded";
    readonly "tag": 12;
  },
  {
    readonly "as_str": "EditAttribution";
    readonly "name": "EditAttribution";
    readonly "tag": 14;
  },
  {
    readonly "as_str": "RoutingDecision";
    readonly "name": "RoutingDecision";
    readonly "tag": 15;
  },
  {
    readonly "as_str": "CustodyClaim";
    readonly "name": "CustodyClaim";
    readonly "tag": 16;
  },
  {
    readonly "as_str": "SessionTransition";
    readonly "name": "SessionTransition";
    readonly "tag": 17;
  },
  {
    readonly "as_str": "SubscriptionTransition";
    readonly "name": "SubscriptionTransition";
    readonly "tag": 18;
  },
  {
    readonly "as_str": "StoreAppend";
    readonly "name": "StoreAppend";
    readonly "tag": 19;
  },
  {
    readonly "as_str": "RegistrationLoaded";
    readonly "name": "RegistrationLoaded";
    readonly "tag": 20;
  },
  {
    readonly "as_str": "DiagnosticOccurrence";
    readonly "name": "DiagnosticOccurrence";
    readonly "tag": 21;
  },
  {
    readonly "as_str": "actor";
    readonly "name": "Actor";
    readonly "tag": 22;
  },
  {
    readonly "as_str": "incarnation";
    readonly "name": "Incarnation";
    readonly "tag": 23;
  },
  {
    readonly "as_str": "scope";
    readonly "name": "Scope";
    readonly "tag": 24;
  },
  {
    readonly "as_str": "operation";
    readonly "name": "Operation";
    readonly "tag": 25;
  },
  {
    readonly "as_str": "replay_session";
    readonly "name": "ReplaySession";
    readonly "tag": 26;
  },
  {
    readonly "as_str": "tally";
    readonly "name": "Tally";
    readonly "tag": 27;
  },
  {
    readonly "as_str": "plane";
    readonly "name": "Plane";
    readonly "tag": 28;
  },
  {
    readonly "as_str": "progress";
    readonly "name": "Progress";
    readonly "tag": 29;
  },
  {
    readonly "as_str": "density";
    readonly "name": "Density";
    readonly "tag": 30;
  },
  {
    readonly "as_str": "floor";
    readonly "name": "Floor";
    readonly "tag": 31;
  },
  {
    readonly "as_str": "edge";
    readonly "name": "Edge";
    readonly "tag": 32;
  },
  {
    readonly "as_str": "binding";
    readonly "name": "Binding";
    readonly "tag": 33;
  },
  {
    readonly "as_str": "storage";
    readonly "name": "Storage";
    readonly "tag": 34;
  },
  {
    readonly "as_str": "boundary_growth";
    readonly "name": "BoundaryGrowth";
    readonly "tag": 35;
  },
  {
    readonly "as_str": "instance_transition";
    readonly "name": "InstanceTransition";
    readonly "tag": 36;
  },
  {
    readonly "as_str": "restart";
    readonly "name": "Restart";
    readonly "tag": 38;
  },
  {
    readonly "as_str": "daemon_shutdown";
    readonly "name": "DaemonShutdown";
    readonly "tag": 39;
  },
  {
    readonly "as_str": "stream_start";
    readonly "name": "StreamStart";
    readonly "tag": 40;
  },
  {
    readonly "as_str": "system_activation_outcome";
    readonly "name": "SystemActivationOutcome";
    readonly "tag": 41;
  },
  {
    readonly "as_str": "system_revision_adoption_outcome";
    readonly "name": "SystemRevisionAdoptionOutcome";
    readonly "tag": 42;
  },
  {
    readonly "as_str": "system_recovery_outcome";
    readonly "name": "SystemRecoveryOutcome";
    readonly "tag": 43;
  },
  {
    readonly "as_str": "system_pause_accepted";
    readonly "name": "SystemPauseAccepted";
    readonly "tag": 44;
  },
  {
    readonly "as_str": "system_resume_accepted";
    readonly "name": "SystemResumeAccepted";
    readonly "tag": 45;
  },
];
export declare const ActorHealthState: readonly [
  "running",
  "waiting",
  "backpressure",
  "failed",
  "stopped",
];
export declare const ActorHealthReasonCode: readonly [
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
  "harness_unusable",
];
export declare const ApprovalDecisionKind: readonly [
  "approved",
  "denied",
];
export declare const PreprocessKind: readonly [
  "map",
  "filter",
  "bang",
  "parse",
  "flatten",
];
export declare const DeadLetterReasonKind: readonly [
  "processing",
  "outcome_unclaimed",
  "destination_gone",
  "poisoned",
  "actor_declared",
  "capacity",
];
export declare const LifecyclePhase: readonly [
  {
    readonly "name": "Draining";
    readonly "tag": 1;
  },
  {
    readonly "name": "AdmissionClosed";
    readonly "tag": 2;
  },
  {
    readonly "name": "Terminated";
    readonly "tag": 3;
  },
  {
    readonly "name": "Prepared";
    readonly "tag": 4;
  },
  {
    readonly "name": "Activated";
    readonly "tag": 5;
  },
  {
    readonly "name": "Restarted";
    readonly "tag": 6;
  },
  {
    readonly "name": "ConfigRestarted";
    readonly "tag": 7;
  },
  {
    readonly "name": "Abandoned";
    readonly "tag": 8;
  },
  {
    readonly "name": "ResumeDenied";
    readonly "tag": 9;
  },
  {
    readonly "name": "ConfigApplied";
    readonly "tag": 10;
  },
];
export declare const SubscriptionFrame: readonly [
  {
    readonly "name": "Lossless";
    readonly "tag": 1;
  },
  {
    readonly "name": "Conflated";
    readonly "tag": 2;
  },
  {
    readonly "name": "Credit";
    readonly "tag": 3;
  },
  {
    readonly "name": "RetentionComplete";
    readonly "tag": 4;
  },
];
export declare const FrameOrigin: readonly [
  {
    readonly "name": "Retained";
    readonly "tag": 1;
  },
  {
    readonly "name": "Live";
    readonly "tag": 2;
  },
];
export declare const SubscriptionEndReason: readonly [
  {
    readonly "name": "ByClient";
    readonly "tag": 1;
  },
  {
    readonly "name": "ConsumerBehind";
    readonly "tag": 2;
  },
  {
    readonly "name": "TargetGone";
    readonly "tag": 3;
  },
  {
    readonly "name": "Withdrawn";
    readonly "tag": 4;
  },
  {
    readonly "name": "SessionClosed";
    readonly "tag": 5;
  },
  {
    readonly "name": "ResetRequired";
    readonly "tag": 6;
  },
  {
    readonly "name": "ScopeGone";
    readonly "tag": 7;
  },
  {
    readonly "name": "IncompatibleClient";
    readonly "tag": 8;
  },
  {
    readonly "name": "Complete";
    readonly "tag": 9;
  },
];
export declare const Ceilings: {
  readonly "Identity": {
    readonly "max_bytes": 1048576;
    readonly "max_container_entries": 65535;
    readonly "max_depth": 64;
    readonly "max_string_bytes": 1048576;
  };
  readonly "Wire": {
    readonly "max_bytes": 1048576;
    readonly "max_container_entries": 65535;
    readonly "max_depth": 64;
    readonly "max_string_bytes": 1048576;
  };
};
export declare const MAX_SEGMENT_BODY_BYTES: 65536;
export declare const MAX_REASSEMBLED_BODY_BYTES: 16777216;
export declare const QueryId: {
  readonly "ActorCatalog": {
    readonly "name": "actor.catalog";
    readonly "paging": "None";
  };
  readonly "ActorConfigurationAdmission": {
    readonly "name": "actor.configure-admission";
    readonly "paging": "None";
  };
  readonly "ActorCreateAdmission": {
    readonly "name": "actor.create-admission";
    readonly "paging": "None";
  };
  readonly "ActorCreateInputs": {
    readonly "name": "actor.create-inputs";
    readonly "paging": "None";
  };
  readonly "ActorEvents": {
    readonly "name": "actor.events";
    readonly "paging": "OptionalImmutableCursor";
  };
  readonly "AgentHarnessCandidates": {
    readonly "name": "agent.harness-candidates";
    readonly "paging": "None";
  };
  readonly "AgentHarnesses": {
    readonly "name": "agent.harnesses";
    readonly "paging": "None";
  };
  readonly "ArrivalScan": {
    readonly "name": "arrival.scan";
    readonly "paging": "OptionalImmutableCursor";
  };
  readonly "AuthoringActorAccess": {
    readonly "name": "authoring.actor-access";
    readonly "paging": "None";
  };
  readonly "AuthoringActorPorts": {
    readonly "name": "authoring.actor-ports";
    readonly "paging": "None";
  };
  readonly "AuthoringSnapshot": {
    readonly "name": "authoring-snapshot";
    readonly "paging": "OptionalRetainedCursor";
  };
  readonly "Catalog": {
    readonly "name": "query.catalog";
    readonly "paging": "None";
  };
  readonly "DaemonHealth": {
    readonly "name": "daemon.health";
    readonly "paging": "None";
  };
  readonly "DeadLetters": {
    readonly "name": "dead.letters";
    readonly "paging": "OptionalImmutableCursor";
  };
  readonly "ObservationScan": {
    readonly "name": "observation-scan";
    readonly "paging": "OptionalImmutableCursor";
  };
  readonly "Pipelines": {
    readonly "name": "pipelines";
    readonly "paging": "None";
  };
  readonly "Presentation": {
    readonly "name": "structure.presentation";
    readonly "paging": "None";
  };
  readonly "Records": {
    readonly "name": "records";
    readonly "paging": "OptionalImmutableCursor";
  };
  readonly "Rollup": {
    readonly "name": "display.rollup";
    readonly "paging": "None";
  };
  readonly "RuntimeApprovals": {
    readonly "name": "runtime.approvals";
    readonly "paging": "None";
  };
  readonly "Timeline": {
    readonly "name": "timeline";
    readonly "paging": "OptionalImmutableCursor";
  };
  readonly "TimelineAt": {
    readonly "name": "timeline.at";
    readonly "paging": "None";
  };
  readonly "TimelineBins": {
    readonly "name": "timeline.bins";
    readonly "paging": "None";
  };
  readonly "Transitions": {
    readonly "name": "instance.transitions";
    readonly "paging": "OptionalImmutableCursor";
  };
};
export declare const SubscriptionTarget: {
  readonly "ActorEvents": {
    readonly "delivery": "Credit";
    readonly "name": "actor.events";
  };
  readonly "AuthoringCommits": {
    readonly "delivery": "Credit";
    readonly "name": "authoring-commits";
  };
  readonly "DisplayFrames": {
    readonly "delivery": "Credit";
    readonly "name": "display.frames";
  };
  readonly "EdgeDepths": {
    readonly "delivery": "Credit";
    readonly "name": "edge.depths";
  };
  readonly "Records": {
    readonly "delivery": "Credit";
    readonly "name": "records";
  };
};
export declare const TimelineMarkKind: readonly [
  "edit",
  "restart",
  "pause",
  "resume",
];
export declare const BaseShape: readonly [
  "null",
  "bool",
  "int",
  "float",
  "string",
  "bytes",
  "uint",
];
export declare const EvalMode: readonly [
  "transform",
  "predicate",
  "number",
  "reduce",
];
