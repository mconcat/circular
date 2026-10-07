
use crate::authoring_assembly::projection::AuthoredProjection;
use crate::authoring_assembly::projection::fold_projection;
use crate::authoring_assembly::projection_diff::{ActorChange, ScopeChange, diff};
use crate::authoring_assembly::rejection::FoldRejection;
use crate::direct_effect::{DirectEffectExecutorRegistry, DirectExecutorRegistrationError};
use crate::tap_pilot::ProductCapabilityGrant;
pub use crate::tap_pilot::ProductGrantCatalog;
use circular_actors::Condition;
use circular_plan::{ActorDecl, NamedActorId};
use circular_runtime::{
    AgentHarnessName, Capability, Ceiling, CpuTime, FsReadGrant, FsWriteGrant, HttpFetchGrant,
    HttpHosts, MemoryBytes, NormalizedPath, NotificationChannel, NotificationChannels, PeerAdapter,
    PeerAdapterName, PeerAdvertisementScopes, PeerDiscoverGrant, PeerRealmScope, PeerSenderScopes,
    PeerTargetScopes, ProcessCount, ProcessTargets, ResourceCeilings, UserNotifyGrant,
    WorkspaceProcessGrant,
};
use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

pub type AgentExecutorFactory = std::sync::Arc<
    dyn Fn() -> Box<dyn circular_runtime::Interpreter<circular_runtime::EffectId> + Send>
        + Send
        + Sync,
>;

#[derive(Clone)]
struct AgentExecutor {
    factory: AgentExecutorFactory,
    program: Option<NormalizedPath>,
}

pub type PeerAdapterFactory = std::sync::Arc<dyn Fn() -> Box<dyn PeerAdapter + Send> + Send + Sync>;

pub const BUILT_IN_PEER_ADAPTER: &str = "memory";

#[derive(Clone)]
pub struct ProductExecutionProfile {
    http_fetch: HttpFetchGrant,
    http_executor: Option<HttpExecutorProfile>,
    secret_vault: Option<std::sync::Arc<crate::SecretVault>>,
    process_spawn: WorkspaceProcessGrant,
    process_executor: Option<ProcessExecutorProfile>,
    user_notify: UserNotifyGrant,
    notifications: BTreeMap<NotificationChannel, crate::NotificationEndpoint>,
    agents: BTreeMap<circular_runtime::AgentHarnessName, AgentExecutor>,
    agent_deadline: Option<Duration>,
    peer_adapters: BTreeMap<PeerAdapterName, PeerAdapterFactory>,
    publication_wake: Arc<std::sync::Mutex<Option<Arc<dyn Fn() + Send + Sync>>>>,
    config_defaults: Arc<[(Box<str>, u64)]>,
    effect_retry: crate::effect_retry::RetrySchedule,
    /// This assembly's boot identity is shared by stream-start and restart records.
    boot_id: [u8; 16],
}

/// Closed owner of one capability decision exposed by product diagnostics.
///
/// The registry remains the requirement source. This enum names the component
/// that made the allow/deny decision; clients must not infer policy from a
/// capability spelling or an actor type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProductCapabilityAuthority {
    ProductExecutionProfile,
    UnconditionalRequirementResolver,
}

/// Closed shape of the registry requirement. Config bodies and paths do not
/// cross this projection boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProductCapabilityCondition {
    Always,
    ConfigPresent,
    ConfigEquals,
}

/// Exact typed subject needed by a capability decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProductCapabilitySubject {
    AgentHarness(AgentHarnessName),
}

/// Product-profile result for one registry-owned requirement row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProductCapabilityDecision {
    Allowed,
    Denied { reason: String },
}

/// Actor-local inputs for one diagnostic or grant assembly. Folding and policy
/// validation are distinct: an agent/peer diagnostic must not acquire the
/// binding path's earlier capability-config rejection.
struct CapabilityInputs {
    config: circular_runtime::FoldedConfig,
    validation: Result<(), circular_actors::CreateInputAdmissionError>,
    peer: OnceCell<Result<circular_actors::peer_actor::PeerConfig, ProductExecutionProfileError>>,
}

impl CapabilityInputs {
    fn new(config: circular_runtime::FoldedConfig) -> Self {
        let validation =
            circular_actors::capability_config::validate(config.actor_type(), config.value());
        Self {
            config,
            validation,
            peer: OnceCell::new(),
        }
    }
}

struct CapabilityRejection {
    reason: String,
    subject: Option<ProductCapabilitySubject>,
}

impl CapabilityRejection {
    fn without_subject(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            subject: None,
        }
    }
}

/// Diagnostics are a one-way projection. Issuance consumes the typed grant,
/// never an `Allowed` flag paired with optional reconstruction data.
fn capability_diagnostic(
    result: Result<ProductCapabilityGrant, CapabilityRejection>,
) -> (ProductCapabilityDecision, Option<ProductCapabilitySubject>) {
    match result {
        Ok(grant) => {
            let subject = match grant {
                ProductCapabilityGrant::AgentHarness(grant) => grant
                    .parameters()
                    .iter()
                    .next()
                    .cloned()
                    .map(ProductCapabilitySubject::AgentHarness),
                _ => None,
            };
            (ProductCapabilityDecision::Allowed, subject)
        }
        Err(CapabilityRejection { reason, subject }) => {
            (ProductCapabilityDecision::Denied { reason }, subject)
        }
    }
}

/// One lossless `ActorSpec::requires()` row and its product decision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductCapabilityAccess {
    pub rule_index: u64,
    pub capability: Capability,
    pub condition: ProductCapabilityCondition,
    pub decision: ProductCapabilityDecision,
    pub authority: ProductCapabilityAuthority,
    pub subject: Option<ProductCapabilitySubject>,
}

/// Access facts for one exact authored plan identity. Actors with no
/// requirements deliberately remain present with an empty `requirements`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductActorAccess {
    pub actor: NamedActorId,
    pub requirements: Vec<ProductCapabilityAccess>,
}

pub const CONDITIONAL_CAPABILITY_REQUIREMENT_REASON: &str = "conditional capability algebra is deferred; this daemon rejects conditional activation requirements";
pub const UNSUPPORTED_PROFILE_CAPABILITY_REASON: &str =
    "capability algebra is deferred; this daemon issues no grant for this capability";
pub const MISSING_AGENT_HARNESS_REASON: &str =
    "agent config does not declare a canonical harness name";

impl fmt::Debug for ProductExecutionProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProductExecutionProfile")
            .field("http_fetch", &self.http_fetch)
            .field("http_executor", &self.http_executor)
            .field(
                "secret_vault_entries",
                &self.secret_vault.as_ref().map_or(0, |vault| vault.len()),
            )
            .field("process_spawn", &self.process_spawn)
            .field("process_executor", &self.process_executor)
            .field(
                "peer_adapters",
                &self.peer_adapters.keys().collect::<Vec<_>>(),
            )
            .field("notification_channels", &self.notifications.len())
            .field(
                "agents",
                &self
                    .agents
                    .keys()
                    .map(|name| name.as_str())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[derive(Clone, Debug)]
struct ProcessExecutorProfile {
    slots: Arc<crate::process_slots::ProcessSlots>,
    workspace: NormalizedPath,
    max_output_bytes: usize,
    deadline: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HttpExecutorProfile {
    max_body_bytes: usize,
    timeout: Duration,
}

/// HTTP response body ceiling carried over from the measured v1 executor.
pub const HTTP_MAX_BODY_BYTES: usize = 1024 * 1024;

/// HTTP request deadline carried over from the measured v1 executor.
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(20);

impl Default for ProductExecutionProfile {
    fn default() -> Self {
        Self::new()
    }
}

impl ProductExecutionProfile {
    pub(crate) fn process_slots(&self) -> Option<Arc<crate::process_slots::ProcessSlots>> {
        self.process_executor
            .as_ref()
            .map(|process| process.slots.clone())
    }

    pub(crate) fn effect_retry(&self) -> &crate::effect_retry::RetrySchedule {
        &self.effect_retry
    }

    #[must_use]
    pub fn with_effect_retry(mut self, schedule: crate::effect_retry::RetrySchedule) -> Self {
        self.effect_retry = schedule;
        self
    }

    pub fn with_config_defaults(
        mut self,
        defaults: impl IntoIterator<Item = (String, u64)>,
    ) -> Self {
        self.config_defaults = defaults
            .into_iter()
            .map(|(key, value)| (key.into_boxed_str(), value))
            .collect();
        self
    }

    pub fn config_defaults(&self) -> &[(Box<str>, u64)] {
        &self.config_defaults
    }

    pub fn set_publication_wake(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        *self.publication_wake.lock().expect("publication observer") = Some(wake);
    }

    pub fn publication_wake(&self) -> Arc<dyn Fn() + Send + Sync> {
        let observer = self.publication_wake.clone();
        Arc::new(move || {
            if let Some(wake) = observer.lock().expect("publication observer").as_ref() {
                wake();
            }
        })
    }

    #[must_use]
    /// Configure executor adapters. Filesystem authority belongs to actor config.
    pub fn new() -> Self {
        let process_spawn = WorkspaceProcessGrant::workspace_process(
            ProcessTargets::default(),
            circular_runtime::PathScopes::new([]),
            default_process_resource_ceilings(),
        );
        let memory =
            PeerAdapterName::try_new(BUILT_IN_PEER_ADAPTER).expect("fixed peer adapter name");
        let name = memory.clone();
        let factory: PeerAdapterFactory = std::sync::Arc::new(move || {
            Box::new(circular_runtime::MemoryPeerAdapter::new(name.clone()))
        });
        Self {
            peer_adapters: BTreeMap::from([(memory, factory)]),
            http_fetch: HttpFetchGrant::http_fetch(HttpHosts::default()),
            http_executor: None,
            secret_vault: None,
            process_spawn,
            process_executor: None,
            user_notify: UserNotifyGrant::user_notify(NotificationChannels::default()),
            notifications: BTreeMap::new(),
            agents: BTreeMap::new(),
            agent_deadline: None,
            publication_wake: Arc::new(std::sync::Mutex::new(None)),
            config_defaults: Arc::from([]),
            effect_retry: crate::effect_retry::RetrySchedule::creator_default(),
            boot_id: {
                let mut boot_id = [0; 16];
                getrandom::fill(&mut boot_id).expect("OS entropy for this assembly's boot id");
                boot_id
            },
        }
    }

    /// The boot identity used by this assembly's stream-start and restart records.
    pub fn boot_id(&self) -> [u8; 16] {
        self.boot_id
    }

    #[must_use]
    pub fn with_http_fetch(
        mut self,
        hosts: HttpHosts,
        max_body_bytes: usize,
        timeout: Duration,
        vault: Option<std::sync::Arc<crate::SecretVault>>,
    ) -> Self {
        self.http_fetch = HttpFetchGrant::http_fetch(hosts);
        self.http_executor = Some(HttpExecutorProfile {
            max_body_bytes,
            timeout,
        });
        self.secret_vault = vault;
        self
    }

    /// All concrete executor outcomes share the same non-recording vault boundary.
    #[must_use]
    pub fn with_secret_vault(mut self, vault: Option<std::sync::Arc<crate::SecretVault>>) -> Self {
        self.secret_vault = vault;
        self
    }

    #[must_use]
    pub fn with_notification_endpoint(
        mut self,
        channel: NotificationChannel,
        endpoint: crate::NotificationEndpoint,
    ) -> Self {
        self.notifications.insert(channel, endpoint);
        self.user_notify = UserNotifyGrant::user_notify(NotificationChannels::exact(
            self.notifications.keys().cloned(),
        ));
        self
    }

    #[must_use]
    pub fn with_process_executor(
        mut self,
        targets: ProcessTargets,
        workspace: NormalizedPath,
        max_output_bytes: usize,
        deadline: Duration,
        max_concurrent: NonZeroUsize,
        queue_capacity: Option<NonZeroUsize>,
    ) -> Self {
        self.process_spawn = WorkspaceProcessGrant::workspace_process(
            targets,
            circular_runtime::PathScopes::new([]),
            default_process_resource_ceilings(),
        );
        self.process_executor = Some(ProcessExecutorProfile {
            slots: crate::process_slots::ProcessSlots::new(max_concurrent, queue_capacity),
            workspace,
            max_output_bytes,
            deadline,
        });
        self
    }

    pub fn with_agent_executor(
        mut self,
        harness: circular_runtime::AgentHarnessName,
        factory: AgentExecutorFactory,
    ) -> Result<Self, ProductExecutionProfileError> {
        if self.agents.contains_key(&harness) {
            return Err(ProductExecutionProfileError::DuplicateAgentHarness(harness));
        }
        self.agents.insert(
            harness,
            AgentExecutor {
                factory,
                program: None,
            },
        );
        Ok(self)
    }

    /// The configured CLI deadline that saved harness bindings run under.
    pub fn with_agent_deadline(mut self, deadline: Duration) -> Self {
        self.agent_deadline = Some(deadline);
        self
    }

    pub fn agent_deadline(&self) -> Option<Duration> {
        self.agent_deadline
    }

    #[must_use]
    pub fn with_agent_binding(
        mut self,
        harness: AgentHarnessName,
        binding: Option<(NormalizedPath, AgentExecutorFactory)>,
    ) -> Self {
        match binding {
            Some((program, factory)) => {
                self.agents.insert(
                    harness,
                    AgentExecutor {
                        factory,
                        program: Some(program),
                    },
                );
            }
            None => {
                self.agents.remove(&harness);
            }
        }
        self
    }

    pub fn bound_programs(&self) -> impl Iterator<Item = (&AgentHarnessName, &NormalizedPath)> {
        self.agents
            .iter()
            .filter_map(|(harness, executor)| Some((harness, executor.program.as_ref()?)))
    }

    /// Register one adapter factory; duplicate names never replace an implementation.
    pub fn with_peer_adapter(
        mut self,
        name: PeerAdapterName,
        factory: PeerAdapterFactory,
    ) -> Result<Self, ProductExecutionProfileError> {
        if self.peer_adapters.contains_key(&name) {
            return Err(ProductExecutionProfileError::DuplicatePeerAdapter(name));
        }
        self.peer_adapters.insert(name, factory);
        Ok(self)
    }

    fn peer_config(
        &self,
        actor: &NamedActorId,
        declaration: &ActorDecl,
        folded: Result<&circular_runtime::FoldedConfig, &String>,
    ) -> Result<circular_actors::peer_actor::PeerConfig, ProductExecutionProfileError> {
        let invalid = |reason: String| ProductExecutionProfileError::PeerConfigRejected {
            actor: actor.clone(),
            reason: crate::activation_config::config_rejection(
                actor,
                "config",
                declaration.domain().config(),
                reason,
            ),
        };
        let value = folded
            .map_err(Clone::clone)
            .and_then(|folded| {
                folded
                    .for_type(*declaration.domain().actor_type())
                    .map_err(|error| error.to_string())
            })
            .map_err(invalid)?;
        self.registered_peer_adapter(actor, value)?;
        circular_actors::peer_actor::PeerConfig::from_value(value)
            .map_err(|error| invalid(error.to_string()))
    }

    fn registered_peer_adapter(
        &self,
        actor: &NamedActorId,
        value: &circular_core::Value,
    ) -> Result<(), ProductExecutionProfileError> {
        if let circular_core::Value::Object(object) = value
            && let Some(name) = object
                .get(PEER_ADAPTER_SLOT)
                .and_then(circular_core::Value::as_str)
            && let Ok(adapter) = PeerAdapterName::try_new(name)
            && !self.peer_adapters.contains_key(&adapter)
        {
            return Err(ProductExecutionProfileError::PeerAdapterUnregistered {
                actor: actor.clone(),
                adapter,
                registered: self.peer_adapters.keys().cloned().collect(),
            });
        }
        Ok(())
    }

    pub fn admit_standing_actors(
        &self,
        plan: &AuthoredProjection,
        previous: impl FnOnce() -> Result<AuthoredProjection, FoldRejection>,
    ) -> Result<(), FoldRejection> {
        let inlets = crate::actor_registry::resolve_plan_inlets(
            plan,
            crate::actor_registry::RegistryProfile::Published,
        )
        .map_err(|error| FoldRejection::Registry(Box::new(error)))?;
        let unresolved = circular_actors::ResolvedInletShapes::default();
        let admit = |actor: &NamedActorId, declaration: &ActorDecl| -> Result<(), FoldRejection> {
            self.admit_peer(actor, declaration)?;
            crate::activation_config::admit_activation_config(
                actor,
                declaration,
                inlets.get(actor).unwrap_or(&unresolved),
            )
        };
        if admit_every(plan, &admit).is_ok() {
            return Ok(());
        }
        let patch = diff(&previous()?, plan);
        let mut pending = vec![(&patch, plan)];
        while let Some((patch, after)) = pending.pop() {
            for (actor, change) in patch.actors() {
                let declaration = match change {
                    ActorChange::Added(declaration) | ActorChange::Replaced(declaration) => {
                        declaration
                    }
                    ActorChange::ConfigTransition { .. } => &after.graph().actors()[actor],
                    ActorChange::Removed | ActorChange::FlagsUpdated { .. } => continue,
                };
                admit(actor, declaration)?;
            }
            for (segment, change) in patch.scopes() {
                match change {
                    ScopeChange::Added(scope) => admit_every(scope, &admit)?,
                    ScopeChange::Changed(child) => {
                        pending.push((child, &after.graph.scopes[segment]))
                    }
                    ScopeChange::Removed => {}
                }
            }
        }
        Ok(())
    }

    fn admit_peer(
        &self,
        actor: &NamedActorId,
        declaration: &ActorDecl,
    ) -> Result<(), FoldRejection> {
        let actor_type = *declaration.domain().actor_type();
        if !binds_peer(actor_type) {
            return Ok(());
        }
        let Ok(value) = fold_peer_value(declaration) else {
            return Ok(());
        };
        self.registered_peer_adapter(actor, &value)
            .map_err(|error| unregistered_peer_rejection(actor_type, &error))
    }

    pub(crate) fn peer_adapter(
        &self,
        declaration: &ActorDecl,
    ) -> Result<Option<Box<dyn PeerAdapter + Send>>, crate::activation_detail::RegistrationFailure>
    {
        use crate::activation_detail::{RegistrationFailure, activation};
        if !binds_peer(*declaration.domain().actor_type()) {
            return Ok(None);
        }
        let value = fold_peer_value(declaration)
            .map_err(|reason| RegistrationFailure::new(activation::CONFIG_FOLD, reason))?;
        let config =
            circular_actors::peer_actor::PeerConfig::from_value(&value).map_err(|error| {
                let message = error.to_string();
                RegistrationFailure::new(
                    circular_actors::ProductFactoryError::Peer(error).detail(),
                    message,
                )
            })?;
        let factory = self.peer_adapters.get(&config.adapter).ok_or_else(|| {
            RegistrationFailure::new(
                activation::CAPABILITY_DENIED,
                format!(
                    "peer adapter `{}` is not registered in this daemon",
                    config.adapter.as_str()
                ),
            )
        })?;
        Ok(Some(factory()))
    }

    pub fn registered_peer_adapters(&self) -> impl ExactSizeIterator<Item = &str> {
        self.peer_adapters.keys().map(PeerAdapterName::as_str)
    }

    pub fn agent_harnesses(
        &self,
    ) -> impl ExactSizeIterator<Item = &circular_runtime::AgentHarnessName> {
        self.agents.keys()
    }

    pub(crate) fn declared_agent_harness(
        &self,
        declaration: &ActorDecl,
    ) -> Result<AgentHarnessName, String> {
        Self::agent_harness_from_config(crate::actor_capability::config(declaration).as_ref())
    }

    fn agent_harness_from_config(
        folded: Result<&circular_runtime::FoldedConfig, &String>,
    ) -> Result<AgentHarnessName, String> {
        let folded = folded
            .map_err(|error| format!("agent config cannot be folded canonically: {error}"))?;
        circular_actors::agent_actor::declared_harness(folded)
            .ok_or_else(|| MISSING_AGENT_HARNESS_REASON.to_owned())
    }

    fn accepts_agent_harness(&self, harness: &AgentHarnessName) -> bool {
        crate::cli_agent::adapter_names().any(|name| name == harness.as_str())
            || self.agents.contains_key(harness)
    }

    fn declared_capability_grant(
        &self,
        actor: &NamedActorId,
        declaration: &ActorDecl,
        inputs: Result<&CapabilityInputs, &String>,
        capability: Capability,
    ) -> Result<ProductCapabilityGrant, CapabilityRejection> {
        use ProductCapabilityGrant as Grant;
        let rejected = |reason: String| {
            CapabilityRejection::without_subject(format!(
                "ConfigRejected: actor `{actor}`; config.capabilities: {reason}"
            ))
        };
        let policy = || {
            let inputs = inputs.map_err(|reason| rejected(reason.clone()))?;
            inputs
                .validation
                .as_ref()
                .map_err(|error| rejected(error.to_string()))?;
            if !crate::actor_capability::required_folded(&inputs.config, capability) {
                return Err(CapabilityRejection::without_subject(
                    "authored tools do not require this capability",
                ));
            }
            Ok(&inputs.config)
        };
        let peer = || match inputs {
            Ok(inputs) => inputs
                .peer
                .get_or_init(|| self.peer_config(actor, declaration, Ok(&inputs.config)))
                .as_ref()
                .map_err(|error| CapabilityRejection::without_subject(error.to_string())),
            Err(reason) => Err(CapabilityRejection::without_subject(
                ProductExecutionProfileError::PeerConfigRejected {
                    actor: actor.clone(),
                    reason: crate::activation_config::config_rejection(
                        actor,
                        "config",
                        declaration.domain().config(),
                        reason,
                    ),
                }
                .to_string(),
            )),
        };
        Ok(match capability {
            Capability::FsRead => Grant::FsRead(FsReadGrant::fs_read(
                crate::actor_capability::roots(policy()?, capability).map_err(rejected)?,
            )),
            Capability::FsWrite => Grant::FsWrite(FsWriteGrant::fs_write(
                crate::actor_capability::roots(policy()?, capability).map_err(rejected)?,
            )),
            Capability::HttpFetch => {
                policy()?;
                Grant::HttpFetch(self.http_fetch.clone())
            }
            Capability::ProcessSpawn => {
                let folded = policy()?;
                let declared = crate::actor_process::declared_targets_folded(folded);
                let targets = ProcessTargets::exact(
                    declared
                        .iter()
                        .filter(|program| self.process_spawn.parameters().targets().allows(program))
                        .cloned(),
                );
                Grant::ProcessSpawn(WorkspaceProcessGrant::workspace_process(
                    targets,
                    crate::actor_capability::filesystem(folded)
                        .1
                        .into_parameters(),
                    default_process_resource_ceilings(),
                ))
            }
            Capability::UserNotify => {
                policy()?;
                Grant::UserNotify(self.user_notify.clone())
            }
            Capability::PeerDiscover => {
                let config = peer()?;
                Grant::PeerDiscover(PeerDiscoverGrant::peer_discover([PeerRealmScope::new(
                    config.adapter.clone(),
                    config.realm.clone(),
                )]))
            }
            Capability::PeerSend => {
                let config = peer()?;
                Grant::PeerSend(
                    PeerTargetScopes::realm(config.adapter.clone(), config.realm.clone()).into(),
                )
            }
            Capability::PeerAdvertise => {
                let config = peer()?;
                Grant::PeerAdvertise(
                    PeerAdvertisementScopes::realm(
                        config.adapter.clone(),
                        config.realm.clone(),
                        config.name.clone(),
                    )
                    .into(),
                )
            }
            Capability::PeerReceive => {
                let config = peer()?;
                Grant::PeerReceive(
                    PeerSenderScopes::realm(config.adapter.clone(), config.realm.clone()).into(),
                )
            }
            Capability::AgentHarness => {
                let harness = Self::agent_harness_from_config(inputs.map(|inputs| &inputs.config))
                    .map_err(CapabilityRejection::without_subject)?;
                if !self.accepts_agent_harness(&harness) {
                    return Err(CapabilityRejection {
                        reason: format!(
                            "ProductExecutionProfile has no executor for agent harness {:?}",
                            harness.as_str()
                        ),
                        subject: Some(ProductCapabilitySubject::AgentHarness(harness)),
                    });
                }
                Grant::AgentHarness(circular_runtime::AgentHarnessGrant::agent_harness([
                    harness,
                ]))
            }
            _ => {
                return Err(CapabilityRejection::without_subject(
                    UNSUPPORTED_PROFILE_CAPABILITY_REASON,
                ));
            }
        })
    }

    #[must_use]
    pub fn actor_capability_access(&self, plan: &AuthoredProjection) -> Vec<ProductActorAccess> {
        fold_projection(plan, |layer| {
            let local = layer
                .actors()
                .iter()
                .map(|(actor, declaration)| {
                    let actor_type = *declaration.domain().actor_type();
                    let inputs =
                        crate::actor_capability::config(declaration).map(CapabilityInputs::new);
                    let requirements = circular_actors::registration(actor_type)
                        .spec()
                        .requires()
                        .iter()
                        .enumerate()
                        .map(|(rule_index, rule)| {
                            let condition = match rule.condition() {
                                Condition::Always => ProductCapabilityCondition::Always,
                                Condition::ConfigPresent(_) => {
                                    ProductCapabilityCondition::ConfigPresent
                                }
                                Condition::ConfigEquals { .. } => {
                                    ProductCapabilityCondition::ConfigEquals
                                }
                            };
                            let (decision, authority, subject) = match rule.condition() {
                                Condition::Always => {
                                    let (decision, subject) =
                                        capability_diagnostic(self.declared_capability_grant(
                                            actor,
                                            declaration,
                                            inputs.as_ref(),
                                            rule.capability(),
                                        ));
                                    (
                                        decision,
                                        ProductCapabilityAuthority::ProductExecutionProfile,
                                        subject,
                                    )
                                }
                                Condition::ConfigPresent(_) | Condition::ConfigEquals { .. } => (
                                    ProductCapabilityDecision::Denied {
                                        reason: CONDITIONAL_CAPABILITY_REQUIREMENT_REASON
                                            .to_owned(),
                                    },
                                    ProductCapabilityAuthority::UnconditionalRequirementResolver,
                                    None,
                                ),
                            };
                            ProductCapabilityAccess {
                                rule_index: u64::try_from(rule_index)
                                    .expect("registry requirement count fits u64"),
                                capability: rule.capability(),
                                condition,
                                decision,
                                authority,
                                subject,
                            }
                        })
                        .collect();
                    ProductActorAccess {
                        actor: actor.clone(),
                        requirements,
                    }
                })
                .collect::<Vec<_>>();
            let mut actors = layer
                .into_scopes()
                .into_values()
                .flatten()
                .collect::<Vec<_>>();
            actors.extend(local);
            actors.sort_by(|left, right| left.actor.cmp(&right.actor));
            actors
        })
    }

    pub(crate) fn effect_registry(
        &self,
        filesystem: &(
            circular_runtime::FsReadGrant,
            circular_runtime::FsWriteGrant,
        ),
        harness: Option<&AgentHarnessName>,
        uses: impl Fn(circular_runtime::EffectCtor) -> bool,
    ) -> Result<DirectEffectExecutorRegistry, ProductExecutionProfileError> {
        use circular_runtime::EffectCtor;
        let mut registry = if uses(EffectCtor::FileRead) || uses(EffectCtor::FileWrite) {
            DirectEffectExecutorRegistry::live_filesystem(&filesystem.0, &filesystem.1)
                .map_err(ProductExecutionProfileError::Executor)?
        } else {
            DirectEffectExecutorRegistry::new()
        };
        registry.bind_secret_vault(self.secret_vault.clone());
        if uses(EffectCtor::Http)
            && let Some(http) = &self.http_executor
        {
            registry
                .register_http(
                    &self.http_fetch,
                    http.max_body_bytes,
                    http.timeout,
                    self.secret_vault.clone(),
                )
                .map_err(ProductExecutionProfileError::Executor)?;
        }
        if uses(EffectCtor::Spawn)
            && let Some(process) = &self.process_executor
        {
            registry.with_process_leases();
            registry
                .register_process(
                    &self.process_spawn,
                    process.workspace.clone(),
                    process.max_output_bytes,
                    process.deadline,
                )
                .map_err(ProductExecutionProfileError::Executor)?;
        }
        if uses(EffectCtor::Notify) {
            registry
                .register_notifications(&self.user_notify, self.notifications.clone())
                .map_err(ProductExecutionProfileError::Executor)?;
        }
        if uses(EffectCtor::AgentInvoke)
            && let Some(harness) = harness
            && let Some(executor) = self.agents.get(harness)
        {
            registry
                .register_boxed(
                    [crate::DirectExecutorSelector::AgentHarness(harness.clone())],
                    (executor.factory)(),
                )
                .map_err(ProductExecutionProfileError::Executor)?;
        }
        Ok(registry)
    }

    pub(crate) fn capability_grants(
        &self,
        graph: &crate::run_graph::RunGraph,
    ) -> ProductGrantCatalog {
        let mut grants = ProductGrantCatalog::new();
        let prototypes = graph
            .templates()
            .iter()
            .flat_map(|template| template.actors.iter());
        for (actor, declaration) in graph.actors().iter().chain(prototypes) {
            let actor_type = *declaration.domain().actor_type();
            let Ok(inputs) = admitted_capability_config(actor, declaration) else {
                continue;
            };
            for rule in circular_actors::registration(actor_type).spec().requires() {
                if !crate::actor_capability::required_folded(&inputs.config, rule.capability()) {
                    continue;
                }
                if !matches!(rule.condition(), circular_actors::Condition::Always) {
                    continue;
                }
                if let Ok(grant) = self.declared_capability_grant(
                    actor,
                    declaration,
                    Ok(&inputs),
                    rule.capability(),
                ) {
                    grants = grant.insert(grants, actor.clone());
                }
            }
        }
        grants
    }
}

fn admitted_capability_config(
    actor: &NamedActorId,
    declaration: &ActorDecl,
) -> Result<CapabilityInputs, crate::activation_detail::RegistrationFailure> {
    use crate::activation_detail::{RegistrationFailure, activation};
    let inputs = crate::actor_capability::config(declaration)
        .map(CapabilityInputs::new)
        .map_err(|reason| RegistrationFailure::new(activation::CONFIG_FOLD, reason))?;
    if let Err(error) = &inputs.validation {
        return Err(RegistrationFailure::new(
            activation::CAPABILITIES_ADMISSION,
            error.rejection_message(actor.name().as_str(), inputs.config.value()),
        ));
    }
    Ok(inputs)
}

pub(crate) fn refused_capability_config(
    actor: &NamedActorId,
    declaration: &ActorDecl,
) -> Option<crate::activation_detail::RegistrationFailure> {
    admitted_capability_config(actor, declaration).err()
}

pub(crate) const fn default_process_resource_ceilings() -> ResourceCeilings {
    ResourceCeilings::new(
        Ceiling::AtMost(CpuTime::from_millis(30_000)),
        Ceiling::AtMost(MemoryBytes::from_bytes(512 * 1024 * 1024)),
        Ceiling::AtMost(ProcessCount::new(8)),
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProductExecutionProfileError {
    Executor(DirectExecutorRegistrationError),
    DuplicateAgentHarness(circular_runtime::AgentHarnessName),
    DuplicatePeerAdapter(PeerAdapterName),
    PeerAdapterUnregistered {
        actor: NamedActorId,
        adapter: PeerAdapterName,
        registered: Vec<PeerAdapterName>,
    },
    PeerConfigRejected {
        actor: NamedActorId,
        reason: String,
    },
}

impl fmt::Display for ProductExecutionProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Executor(error) => write!(formatter, "effect executor: {error}"),
            Self::DuplicatePeerAdapter(adapter) => {
                write!(
                    formatter,
                    "peer adapter `{}` is already registered",
                    adapter.as_str()
                )
            }
            Self::PeerAdapterUnregistered {
                actor,
                adapter,
                registered,
            } => formatter.write_str(&circular_actors::config::config_rejection(
                actor,
                "config.adapter",
                Some(&circular_core::Value::string(adapter.as_str())),
                format_args!(
                    "peer adapter is not registered; {}",
                    circular_core::spelling::allowed(
                        registered
                            .iter()
                            .map(|name| circular_core::spelling::Quoted(name.as_str()))
                    )
                ),
            )),
            Self::PeerConfigRejected { reason, .. } => formatter.write_str(reason),
            Self::DuplicateAgentHarness(harness) => {
                write!(
                    formatter,
                    "executor for agent harness {:?} is already registered",
                    harness.as_str()
                )
            }
        }
    }
}

impl std::error::Error for ProductExecutionProfileError {}

const PEER_ADAPTER_SLOT: &str = "adapter";

fn binds_peer(actor_type: circular_plan::ActorType) -> bool {
    circular_actors::get(actor_type)
        .effect()
        .stand_ins()
        .is_some_and(|stand_ins| {
            stand_ins
                .get(circular_runtime::EffectCtor::PeerBind)
                .is_some()
        })
}

fn admit_every(
    plan: &AuthoredProjection,
    admit: &dyn Fn(&NamedActorId, &ActorDecl) -> Result<(), FoldRejection>,
) -> Result<(), FoldRejection> {
    fold_projection::<Result<(), FoldRejection>>(plan, |layer| {
        layer
            .actors()
            .iter()
            .try_for_each(|(actor, declaration)| admit(actor, declaration))?;
        layer.into_scopes().into_values().collect()
    })
}

fn unregistered_peer_rejection(
    actor_type: circular_plan::ActorType,
    error: &ProductExecutionProfileError,
) -> FoldRejection {
    FoldRejection::Registry(Box::new(
        crate::actor_registry::PlanRegistryError::ConfigFold {
            actor_type,
            detail: error.to_string(),
            admission: Some(Box::new(
                circular_actors::CreateInputAdmissionError::outside_space(
                    circular_actors::ConfigPath::root().join_key(PEER_ADAPTER_SLOT),
                ),
            )),
        },
    ))
}

fn fold_peer_value(declaration: &ActorDecl) -> Result<circular_core::Value, String> {
    let actor_type = *declaration.domain().actor_type();
    crate::activation_config::fold_config(actor_type, declaration.domain().config())
        .map_err(|error| error.to_string())?
        .for_type(actor_type)
        .cloned()
        .map_err(|error| error.to_string())
}
