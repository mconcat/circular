#[path = "effect_outcome_record.rs"]
pub mod effect_outcome_record;

#[path = "activation.rs"]
pub(crate) mod activation;
#[path = "activation_config.rs"]
pub(crate) mod activation_config;
#[path = "activation_detail.rs"]
pub mod activation_detail;
#[path = "arrival_commit.rs"]
pub(crate) mod arrival_commit;
#[path = "http_close.rs"]
pub mod http_close;
#[path = "product_arrival_journal.rs"]
pub(crate) mod product_arrival_journal;
#[path = "state_journal.rs"]
pub mod state_journal;
#[path = "wake.rs"]
pub mod wake;
#[cfg(any(test, feature = "test-support"))]
#[path = "unique_temp_dir.rs"]
pub(crate) mod unique_temp_dir;
pub use product_arrival_journal::{
    ArrivalRecorderStop, ColumnJournal, PRODUCT_STORE_PAGE_MAXIMUM, ProductArrivalJournalUsage,
    ProductCheckpointReader, ProductDurableArrivalJournal, ProductReadOnlyArrivalJournal,
    arrival_manifest, journal_prefix, published_arrivals,
};
#[path = "actor_approval.rs"]
pub(crate) mod actor_approval;
#[path = "actor_capability.rs"]
pub(crate) mod actor_capability;
#[path = "actor_process.rs"]
pub(crate) mod actor_process;
#[path = "actor_records.rs"]
pub mod actor_records;
#[path = "authoring_assembly/mod.rs"]
pub mod authoring_assembly;
#[path = "checkpoint_fact.rs"]
pub mod checkpoint_fact;
#[path = "dead_letter_writer.rs"]
pub mod dead_letter_writer;
#[path = "declarations.rs"]
pub mod declarations;
#[path = "direct_effect.rs"]
pub(crate) mod direct_effect;
#[path = "display_writer.rs"]
pub mod display_writer;
 #[path = "execution_profile.rs"]
pub mod execution_profile;
#[path = "incarnation_transition.rs"]
pub(crate) mod incarnation_transition;
#[path = "recorded.rs"]
pub mod recorded;
#[path = "effect_retry.rs"]
pub mod effect_retry;
#[path = "process_slots.rs"]
pub(crate) mod process_slots;
#[path = "tap_pilot.rs"]
pub(crate) mod tap_pilot;
#[path = "actor_registry.rs"]
pub(crate) mod actor_registry;
#[path = "cli_agent.rs"]
pub(crate) mod cli_agent;
#[path = "harness_adapter.rs"]
pub(crate) mod harness_adapter;
#[path = "harness_boundary.rs"]
pub(crate) mod harness_boundary;
#[path = "harness_event.rs"]
pub(crate) mod harness_event;
#[path = "inlet_preprocess.rs"]
pub(crate) mod inlet_preprocess;
#[path = "instance_journal.rs"]
pub(crate) mod instance_journal;
#[path = "invocation_authority.rs"]
pub(crate) mod invocation_authority;
#[path = "kernel/mod.rs"]
pub(crate) mod kernel;
#[cfg(any(target_vendor = "apple", target_os = "linux"))]
#[path = "owner_local_daemon.rs"]
pub(crate) mod owner_local_daemon;
#[path = "peer_adapter.rs"]
pub mod peer_adapter;
#[path = "peer_bridges/mod.rs"]
pub mod peer_bridges;
#[path = "reference_agent.rs"]
pub(crate) mod reference_agent;
#[path = "run_graph.rs"]
pub(crate) mod run_graph;
#[path = "session_tokens.rs"]
pub(crate) mod session_tokens;
pub use session_tokens::OsSessionTokenSource;
#[path = "boot_tail.rs"]
pub mod boot_tail;
#[path = "restart_custody_codec.rs"]
pub mod restart_custody_codec;
#[path = "restart_journal.rs"]
pub mod restart_journal;
#[path = "run_reconcile.rs"]
pub(crate) mod run_reconcile;
 #[path = "runtime_approval.rs"]
pub(crate) mod runtime_approval;
#[path = "state_manifest.rs"]
pub mod state_manifest;

#[allow(unused_imports)]
pub(crate) use activation::ActivationError;
pub use activation::UnconditionalRequirementResolver;
pub use activation_config::{CANONICAL_VALUE_TAG, fold_config};
pub(crate) use actor_registry::resolve_declared_ports_at;
pub use actor_registry::{
    RequestExportIngress, observation_mount_actor, request_mount_actors, resolve_declared_ports,
    validate_published_plan,
};
pub use actor_registry::{ResolvedRevisionPorts, recorded_port_shape};
pub use checkpoint_fact::{CheckpointValueRejection, checkpoint_record, checkpoint_value};
pub use circular_core::RevisionEpochId;
pub use circular_protocol::actor_events::{
    ACTOR_HEALTH_TRANSITION_KIND, ActorHealthReason, ActorHealthReasonCode, ActorHealthState,
    ActorHealthTransition, ActorHealthTransitionRejection, decode_actor_health_transition,
};
pub use circular_protocol::dead_letter::{
    DeadLetterReason, DeadLetterReasonRejection, decode_dead_letter_reason,
};
pub use cli_agent::{
    CliAgentExecutor, CliHarnessSpec, adapter_candidates as cli_adapter_candidates,
    adapter_for as cli_adapter_for, adapter_names as cli_adapter_names,
    program_is_executable as cli_program_is_executable,
};
pub use dead_letter_writer::{DeadLetterValueRejection, dead_letter_value};
pub use declarations::RevisionDeclarations;
pub use direct_effect::NotificationEndpoint;
pub(crate) use direct_effect::{DirectEffectExecutorRegistry, DirectExecutorSelector};
pub use engine_secrets::SecretVault;
pub use incarnation_transition::incarnation_transition_value;
pub use instance_journal::{
    InstanceLifecycleReplay, instance_transition_record, instance_transition_value,
    minted_instance_scope,
};
#[allow(unused_imports)]
pub(crate) use invocation_authority::{
    InvocationLifecycle, InvocationReadiness, UnboundInvocationAuthority,
};
#[cfg(any(target_vendor = "apple", target_os = "linux"))]
pub use owner_local_daemon::{ClaimedOwnerLocalDaemon, DaemonStartError};
pub use reference_agent::{ReferenceAgentContacts, ReferenceAgentExecutor};
pub use run_graph::{RunGraph, fold_revision};
pub use run_reconcile::{
    RevisionReconciliation, RunReconciliation, metadata_only_plan_change,
    preprocess_only_plan_change,
};
pub use runtime_approval::{
    RuntimeApprovalDecision, RuntimeApprovalDecisionError, RuntimeApprovalInvariantError,
    RuntimeApprovalPersistence, RuntimeApprovalPersistenceUnavailable, RuntimeApprovalQueue,
    RuntimeApprovalQueueSnapshot, RuntimeApprovalRow, RuntimeApprovalSummary,
    RuntimeApprovalSummaryUnavailable,
};

#[path = "otlp_ingress/mod.rs"]
pub mod otlp_ingress;

/// Test-only fixed coordinate carrier; never used by a product issuer.
#[cfg(any(test, feature = "test-support"))]
pub fn test_effect(index: u64) -> circular_runtime::EffectId {
    test_effect_for(
        circular_plan::NamedActorId::new(
            circular_plan::ScopeId::root(),
            circular_plan::Name::from_normalized("test-effect"),
        ),
        index,
    )
}
#[cfg(any(test, feature = "test-support"))]
pub fn test_effect_for(
    actor: circular_plan::NamedActorId,
    index: u64,
) -> circular_runtime::EffectId {
    circular_runtime::EffectId::from_components(
        actor.into(),
        vec![circular_plan::Generation::new(0)].into_boxed_slice(),
        circular_runtime::EffectOccasion::Poll(circular_core::Tick::new(5), 0),
        index,
    )
    .unwrap()
}

#[path = "mailbox_pressure.rs"]
pub mod mailbox_pressure;

#[path = "actor_time.rs"]
pub(crate) mod actor_time;
pub use actor_time::TimeActor;
