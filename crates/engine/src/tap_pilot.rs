
use crate::UnboundInvocationAuthority;
use crate::activation::{
    ActivationError, ActivationRequest, CapabilityAdmission, FactoryActivation, GrantBundleIssuer,
    RegisteredFactoryDispatcher, ScopeGrantView, StaticRequirementError,
    UnconditionalRequirementResolver, activate_registered,
};
use crate::activation_config::fold_config;
#[cfg(test)]
use crate::actor_registry::resolve_plan;
use crate::actor_registry::{PlanRegistryError, RegistryProfile};
use crate::authoring_assembly::projection::AuthoredProjection;
use circular_actors::{
    ActorType, ProductActor, ProductFactoryError, ProductPayload, product_actor_factory,
    registration,
};
use circular_plan::{ActorDecl, NamedActorId, ScopeRoleTable, admit_template};
use circular_runtime::{
    ActorTypes, AgentHarness, AgentHarnessAuthorityBearer, AgentHarnessGrant, Capability,
    FilesystemAuthorityBearer, FsRead, FsReadGrant, FsWrite, FsWriteGrant, Granted, HttpFetch,
    HttpFetchAuthorityBearer, HttpFetchGrant, InstanceAuthority, PeerAdvertise, PeerAdvertiseGrant,
    PeerAuthorityBearer, PeerDiscover, PeerDiscoverGrant, PeerReceive, PeerReceiveGrant, PeerSend,
    PeerSendGrant, ProcessAuthorityBearer, ProcessSpawn, UserNotify, UserNotifyAuthorityBearer,
    UserNotifyGrant, WorkspaceProcessGrant,
};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;

macro_rules! capability_grant_table {
    ( $( $field:ident : $capability:ident => $grant:ty { with: $with:ident } ),+ $(,)? ) => {
        #[derive(Clone, Debug, Default, Eq, PartialEq)]
        struct ProductActorCapabilityGrants {
            $( $field: Option<$grant>, )+
        }

        /// A supported capability carries the exact grant selected by the profile.
        /// This sum and its catalog insertion come from the same grant table.
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub(crate) enum ProductCapabilityGrant {
            $( $capability($grant), )+
        }

        impl ProductCapabilityGrant {
            pub(crate) fn insert(
                self,
                catalog: ProductGrantCatalog,
                actor: NamedActorId,
            ) -> ProductGrantCatalog {
                match self {
                    $( Self::$capability(grant) => catalog.$with(actor, grant), )+
                }
            }
        }

        #[derive(Clone, Debug, Default, Eq, PartialEq)]
        pub struct ProductGrantCatalog {
            actors: BTreeMap<NamedActorId, ProductActorCapabilityGrants>,
        }

        impl ProductGrantCatalog {
            pub(crate) fn for_actors<'a>(
                &self,
                actors: impl Iterator<Item = &'a NamedActorId>,
            ) -> Self {
                Self {
                    actors: actors.filter_map(|actor| {
                        self.actors.get(actor).map(|grants| (actor.clone(), grants.clone()))
                    }).collect(),
                }
            }

            #[must_use]
            pub const fn new() -> Self {
                Self {
                    actors: BTreeMap::new(),
                }
            }

            $(
                #[must_use]
                pub fn $with(mut self, actor: NamedActorId, grant: $grant) -> Self {
                    self.actors.entry(actor).or_default().$field = Some(grant);
                    self
                }
            )+

            fn get(&self, actor: &NamedActorId) -> Option<&ProductActorCapabilityGrants> {
                self.actors.get(actor)
            }

            /// Read-only policy projection used by product diagnostics.
            ///
            /// This does not issue [`Granted`] evidence. It reports whether the
            /// exact typed grant catalog that activation will consume contains
            /// the requested capability for `actor`.
            #[must_use]
            pub(crate) fn contains(&self, actor: &NamedActorId, capability: Capability) -> bool {
                let Some(grants) = self.get(actor) else {
                    return false;
                };
                match capability {
                    $( Capability::$capability => grants.$field.is_some(), )+
                    _ => false,
                }
            }
        }

        #[derive(Clone, Debug, Default, Eq, PartialEq)]
        struct TapPilotGrantView {
            catalog: ProductGrantCatalog,
        }

        impl TapPilotGrantView {
            fn handed_to_cell(&self, cell: &NamedActorId, prototype: &NamedActorId) -> Self {
                let mut catalog = ProductGrantCatalog::new();
                if let Some(grants) = self.catalog.get(prototype) {
                    catalog.actors.insert(cell.clone(), grants.clone());
                }
                Self { catalog }
            }
        }

        impl ScopeGrantView for TapPilotGrantView {
            fn contains(&self, actor: &NamedActorId, capability: Capability) -> bool {
                self.catalog.contains(actor, capability)
            }
        }

        pub struct TapPilotGrantBundle {
            actor: NamedActorId,
            instance: Option<InstanceAuthority<ProductInstanceSeal>>,
            $( $field: Option<Granted<$capability>>, )+
        }

        impl TapPilotGrantBundle {
            fn issue_typed(
                actor: NamedActorId,
                instance: Option<InstanceAuthority<ProductInstanceSeal>>,
                typed: Option<&ProductActorCapabilityGrants>,
                required: &circular_runtime::CapabilitySet,
            ) -> Self {
                Self {
                    actor,
                    instance,
                    $( $field: typed
                        .and_then(|grants| grants.$field.as_ref())
                        .and_then(|grant| grant.evidence_for(required)), )+
                }
            }
        }
    };
}

capability_grant_table! {
    agent_harness: AgentHarness => AgentHarnessGrant { with: with_agent_harness },
    fs_read: FsRead => FsReadGrant { with: with_fs_read },
    fs_write: FsWrite => FsWriteGrant { with: with_fs_write },
    http_fetch: HttpFetch => HttpFetchGrant { with: with_http_fetch },
    process_spawn: ProcessSpawn => WorkspaceProcessGrant { with: with_process_spawn },
    user_notify: UserNotify => UserNotifyGrant { with: with_user_notify },
    peer_discover: PeerDiscover => PeerDiscoverGrant { with: with_peer_discover },
    peer_send: PeerSend => PeerSendGrant { with: with_peer_send },
    peer_advertise: PeerAdvertise => PeerAdvertiseGrant { with: with_peer_advertise },
    peer_receive: PeerReceive => PeerReceiveGrant { with: with_peer_receive },
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ProductInstanceSeal {}

impl AgentHarnessAuthorityBearer for TapPilotGrantBundle {
    fn agent_harness_authority(&self) -> Option<Granted<AgentHarness>> {
        self.agent_harness
    }
}

impl FilesystemAuthorityBearer for TapPilotGrantBundle {
    fn fs_read_authority(&self) -> Option<Granted<FsRead>> {
        self.fs_read
    }

    fn fs_write_authority(&self) -> Option<Granted<FsWrite>> {
        self.fs_write
    }
}

impl ProcessAuthorityBearer for TapPilotGrantBundle {
    fn process_spawn_authority(&self) -> Option<Granted<ProcessSpawn>> {
        self.process_spawn
    }
}

impl UserNotifyAuthorityBearer for TapPilotGrantBundle {
    fn user_notify_authority(&self) -> Option<Granted<UserNotify>> {
        self.user_notify
    }
}

impl HttpFetchAuthorityBearer for TapPilotGrantBundle {
    fn http_fetch_authority(&self) -> Option<Granted<HttpFetch>> {
        self.http_fetch
    }
}

impl PeerAuthorityBearer for TapPilotGrantBundle {
    fn peer_discover_authority(&self) -> Option<Granted<PeerDiscover>> {
        self.peer_discover
    }

    fn peer_send_authority(&self) -> Option<Granted<PeerSend>> {
        self.peer_send
    }

    fn peer_advertise_authority(&self) -> Option<Granted<PeerAdvertise>> {
        self.peer_advertise
    }

    fn peer_receive_authority(&self) -> Option<Granted<PeerReceive>> {
        self.peer_receive
    }
}

impl circular_runtime::InstanceAuthorityBearer for TapPilotGrantBundle {
    type Seal = ProductInstanceSeal;

    fn instance_authority(&self) -> Option<&circular_runtime::InstanceAuthority<Self::Seal>> {
        self.instance.as_ref()
    }
}

impl fmt::Debug for TapPilotGrantBundle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TapPilotGrantBundle")
            .field("actor", &self.actor)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TapPilotBoundaryError {
    UnexpectedActorType(ActorType),
    ConfigFold(String),
    UnexpectedRequirements,
    CanonicalSpecMismatch,
    BundleActorMismatch,
    UnexpectedSourceArm,
    StaleInstanceGrant,
    Factory(ProductFactoryError, String),
}

impl fmt::Display for TapPilotBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConfigFold(detail) => formatter.write_str(detail),
            Self::UnexpectedActorType(actor_type) => {
                write!(
                    formatter,
                    "actor type is not supported by bang pilot: {actor_type:?}"
                )
            }
            Self::UnexpectedRequirements => formatter
                .write_str("activation capability requirements differ from canonical registration"),
            Self::CanonicalSpecMismatch => {
                formatter.write_str("bang activation spec differs from canonical registration")
            }
            Self::BundleActorMismatch => {
                formatter.write_str("bang grant bundle and activation target different actors")
            }
            Self::UnexpectedSourceArm => {
                formatter.write_str("bang cannot be activated through a source factory variant")
            }
            Self::StaleInstanceGrant => {
                formatter.write_str("activation instance grant belongs to another actor")
            }
            Self::Factory(_, reason) => formatter.write_str(reason),
        }
    }
}

impl Error for TapPilotBoundaryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Factory(error, _) => Some(error),
            Self::ConfigFold(_)
            | Self::UnexpectedActorType(_)
            | Self::UnexpectedRequirements
            | Self::CanonicalSpecMismatch
            | Self::BundleActorMismatch
            | Self::UnexpectedSourceArm
            | Self::StaleInstanceGrant => None,
        }
    }
}

pub(crate) fn actor_factory<T>(
    profile: RegistryProfile,
    actor_type: ActorType,
) -> Option<circular_actors::ProductActorFactory<T>>
where
    T: ActorTypes,
{
    match profile {
        RegistryProfile::Published => product_actor_factory::<T>(actor_type),
        RegistryProfile::Fixture => circular_actors::fixture_actor_factory::<T>(actor_type),
    }
}

fn require_admitted<T>(
    profile: RegistryProfile,
    actor_type: ActorType,
) -> Result<(), TapPilotBoundaryError>
where
    T: ActorTypes<Grants = TapPilotGrantBundle, Payload = ProductPayload>,
{
    if actor_factory::<T>(profile, actor_type).is_some() {
        Ok(())
    } else {
        Err(TapPilotBoundaryError::UnexpectedActorType(actor_type))
    }
}

fn validate_registered_element(
    actor_type: ActorType,
    spec: &'static circular_actors::ActorSpec,
    required: &circular_runtime::CapabilitySet,
    config: &circular_plan::Config,
) -> Result<(), TapPilotBoundaryError> {
    if !std::ptr::eq(spec, registration(actor_type).spec()) {
        return Err(TapPilotBoundaryError::CanonicalSpecMismatch);
    }
    let declared = spec
        .requires()
        .iter()
        .filter(|rule| {
            crate::actor_capability::required_config(actor_type, config, rule.capability())
        })
        .fold(circular_runtime::CapabilitySet::empty(), |set, rule| {
            set.join(&circular_runtime::CapabilitySet::singleton(
                rule.capability(),
            ))
        });
    if required != &declared {
        return Err(TapPilotBoundaryError::UnexpectedRequirements);
    }
    Ok(())
}

fn validate_bundle_actor(
    bundle: &TapPilotGrantBundle,
    actor: &NamedActorId,
) -> Result<(), TapPilotBoundaryError> {
    if &bundle.actor == actor {
        Ok(())
    } else {
        Err(TapPilotBoundaryError::BundleActorMismatch)
    }
}

fn reject_source_arm<A>() -> Result<A, TapPilotBoundaryError> {
    Err(TapPilotBoundaryError::UnexpectedSourceArm)
}

#[derive(Debug, Default)]
struct TapPilotGrantIssuer {
    instance: Option<(NamedActorId, InstanceAuthority<ProductInstanceSeal>)>,
}

impl GrantBundleIssuer<TapPilotGrantView> for TapPilotGrantIssuer {
    type Bundle = TapPilotGrantBundle;
    type Error = TapPilotBoundaryError;

    fn issue(
        &mut self,
        admission: CapabilityAdmission<'_, TapPilotGrantView>,
    ) -> Result<Self::Bundle, Self::Error> {
        validate_registered_element(
            admission.actor_type(),
            admission.spec(),
            admission.required(),
            admission.config(),
        )?;
        let instance = match self.instance.take() {
            Some((actor, authority)) if &actor == admission.actor() => Some(authority),
            Some(_) => return Err(TapPilotBoundaryError::StaleInstanceGrant),
            None => None,
        };
        Ok(TapPilotGrantBundle::issue_typed(
            admission.actor().clone(),
            instance,
            admission.grants().catalog.get(admission.actor()),
            admission.required(),
        ))
    }
}

struct TapPilotFactoryDispatcher<T> {
    registry: RegistryProfile,
    marker: PhantomData<fn() -> T>,
}

impl<T> Default for TapPilotFactoryDispatcher<T> {
    fn default() -> Self {
        Self {
            registry: RegistryProfile::Published,
            marker: PhantomData,
        }
    }
}

impl<T> RegisteredFactoryDispatcher<TapPilotGrantBundle> for TapPilotFactoryDispatcher<T>
where
    T: ActorTypes<Grants = TapPilotGrantBundle, Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
    T::Event: circular_actors::StampedEvent,
    T::Event: circular_actors::StampedEvent,
{
    type Activated = ProductActor<T>;
    type Error = TapPilotBoundaryError;

    fn activate_actor(
        &mut self,
        request: FactoryActivation<'_, TapPilotGrantBundle>,
    ) -> Result<Self::Activated, Self::Error> {
        validate_bundle_actor(request.bundle(), request.actor())?;
        let factory = actor_factory::<T>(self.registry, request.actor_type()).ok_or(
            TapPilotBoundaryError::UnexpectedActorType(request.actor_type()),
        )?;
        let folded = fold_config(request.actor_type(), request.config()).map_err(|error| {
            TapPilotBoundaryError::ConfigFold(crate::activation_config::config_rejection(
                request.actor().name().as_str(),
                "config",
                request.config(),
                error,
            ))
        })?;
        factory
            .create(&folded, request.bundle(), request.inlets())
            .map_err(|error| {
                let reason = crate::activation_config::factory_rejection_reason(
                    request.actor().name().as_str(),
                    &folded,
                    &error,
                );
                TapPilotBoundaryError::Factory(error, reason)
            })
    }

    fn activate_source(
        &mut self,
        request: FactoryActivation<'_, TapPilotGrantBundle>,
    ) -> Result<Self::Activated, Self::Error> {
        if !matches!(request.actor_type(), ActorType::Otlp | ActorType::Listener) {
            return reject_source_arm();
        }
        self.activate_actor(request)
    }
}

pub(crate) fn instance_grant(
    scopes: &ScopeRoleTable,
    actor: &NamedActorId,
    declaration: &ActorDecl,
) -> Result<Option<(NamedActorId, circular_plan::AdmittedTemplate)>, TapPilotActivationError> {
    if *declaration.domain().actor_type() != ActorType::Replicator {
        return Ok(None);
    }
    let admitted = admit_template(scopes, actor.scope(), actor.name())
        .map_err(TapPilotActivationError::InstanceAdmission)?;
    Ok(Some((actor.clone(), admitted)))
}

type RegisteredBangActivationError =
    ActivationError<StaticRequirementError, TapPilotBoundaryError, TapPilotBoundaryError>;

#[derive(Debug)]
pub enum TapPilotActivationError {
    MissingActor(NamedActorId),
    Boundary(TapPilotBoundaryError),
    AuthoredProjection(PlanRegistryError),
    Registered(RegisteredBangActivationError),
    InstanceAdmission(circular_plan::ScopeAdmissionError),
    /// This actor has no durable Activated lifecycle witness.
    MissingActivationWitness,
    /// This actor's registration or source preparation was refused.
    Registration(crate::activation_detail::RegistrationFailure),
}

impl fmt::Display for TapPilotActivationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingActor(actor) => {
                write!(
                    formatter,
                    "bang activation target is missing from the plan: {actor:?}"
                )
            }
            Self::Boundary(error) => error.fmt(formatter),
            Self::AuthoredProjection(error) => error.fmt(formatter),
            Self::Registered(error) => error.fmt(formatter),
            Self::InstanceAdmission(error) => {
                write!(formatter, "replicator template rejected: {error:?}")
            }
            Self::MissingActivationWitness => {
                formatter.write_str("has no Activated lifecycle witness")
            }
            Self::Registration(detail) => {
                write!(formatter, "activation registration failed: {detail}")
            }
        }
    }
}

impl Error for TapPilotActivationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Boundary(error) => Some(error),
            Self::AuthoredProjection(error) => Some(error),
            Self::Registered(error) => Some(error),
            Self::InstanceAdmission(error) => Some(error),
            Self::MissingActivationWitness | Self::Registration(_) | Self::MissingActor(_) => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StandingDeclarations {
    inlets: std::collections::BTreeMap<NamedActorId, circular_actors::ResolvedInletShapes>,
    scopes: ScopeRoleTable,
    refused:
        std::collections::BTreeMap<NamedActorId, crate::activation_detail::RegistrationFailure>,
}

impl StandingDeclarations {
    pub(crate) fn for_actors<'a>(&self, actors: impl Iterator<Item = &'a NamedActorId>) -> Self {
        let mut scopes = ScopeRoleTable::new();
        let mut inlets = BTreeMap::new();
        let mut refused = BTreeMap::new();
        for actor in actors {
            if let Some(shape) = self.inlets.get(actor) {
                inlets.insert(actor.clone(), shape.clone());
            }
            if let Some(failure) = self.refused.get(actor) {
                refused.insert(actor.clone(), failure.clone());
            }
            let segments = actor.scope().segments();
            for depth in 1..=segments.len() {
                let scope = circular_plan::ScopeId::from_segments(segments[..depth].to_vec())
                    .expect("a declared scope prefix");
                if let Some(role) = self.scopes.role_at(&scope) {
                    scopes.declare(scope, role);
                }
            }
        }
        Self {
            inlets,
            scopes,
            refused,
        }
    }

    pub(crate) fn admit(
        plan: &AuthoredProjection,
        registry: RegistryProfile,
    ) -> Result<Self, TapPilotActivationError> {
        let inlets = crate::actor_registry::resolve_plan_inlets(plan, registry)
            .map_err(TapPilotActivationError::AuthoredProjection)?;
        Ok(Self {
            scopes: crate::authoring_assembly::projection::scope_roles(plan),
            inlets,
            refused: plan
                .refused()
                .iter()
                .map(|(actor, refused)| (actor.clone(), refused.failure()))
                .collect(),
        })
    }

    pub(crate) fn refused(
        &self,
        actor: &NamedActorId,
    ) -> Option<&crate::activation_detail::RegistrationFailure> {
        self.refused
            .get(actor)
            .or_else(|| self.refused.get(&self.prototype_of(actor)?))
    }

    pub(crate) fn inlets_for(&self, actor: &NamedActorId) -> &circular_actors::ResolvedInletShapes {
        static UNRESOLVED: std::sync::LazyLock<circular_actors::ResolvedInletShapes> =
            std::sync::LazyLock::new(circular_actors::ResolvedInletShapes::default);
        self.inlets
            .get(actor)
            .or_else(|| self.inlets.get(&self.prototype_of(actor)?))
            .unwrap_or(&UNRESOLVED)
    }

    pub(crate) fn prototype_of(&self, actor: &NamedActorId) -> Option<NamedActorId> {
        let admitted = circular_plan::admit_runtime_scope(&self.scopes, actor.scope()).ok()?;
        let prototype = NamedActorId::new(admitted.declared().clone(), actor.name().clone());
        (&prototype != actor).then_some(prototype)
    }

    pub(crate) fn grant_for(
        &self,
        actor: &NamedActorId,
        declaration: &ActorDecl,
    ) -> Result<Option<(NamedActorId, circular_plan::AdmittedTemplate)>, TapPilotActivationError>
    {
        instance_grant(&self.scopes, actor, declaration)
    }
}

pub struct TapPilotActivationProfile<T> {
    registry: RegistryProfile,
    grants: TapPilotGrantView,
    issuer: TapPilotGrantIssuer,
    dispatcher: TapPilotFactoryDispatcher<T>,
}

impl<T> Default for TapPilotActivationProfile<T> {
    fn default() -> Self {
        Self::on(RegistryProfile::Published)
    }
}

impl<T> TapPilotActivationProfile<T> {
    #[must_use]
    pub(crate) fn on(registry: RegistryProfile) -> Self {
        Self::on_with_grants(registry, ProductGrantCatalog::new())
    }

    #[must_use]
    pub(crate) fn on_with_grants(registry: RegistryProfile, catalog: ProductGrantCatalog) -> Self {
        Self {
            registry,
            grants: TapPilotGrantView { catalog },
            issuer: TapPilotGrantIssuer::default(),
            dispatcher: TapPilotFactoryDispatcher {
                registry,
                marker: PhantomData,
            },
        }
    }
}

impl<T> TapPilotActivationProfile<T>
where
    T: ActorTypes<Grants = TapPilotGrantBundle, Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
    T::Event: circular_actors::StampedEvent,
{
    #[cfg(test)]
    pub fn activate_from_plan(
        &mut self,
        plan: &AuthoredProjection,
        actor: &NamedActorId,
    ) -> Result<
        UnboundInvocationAuthority<ProductActor<T>, TapPilotGrantBundle>,
        TapPilotActivationError,
    > {
        let declaration = plan
            .graph()
            .actors()
            .get(actor)
            .ok_or_else(|| TapPilotActivationError::MissingActor(actor.clone()))?
            .clone();
        let standing = StandingDeclarations::admit(plan, self.registry)?;
        self.activate_declared(&standing, actor, &declaration)
    }

    pub(crate) fn activate_declared(
        &mut self,
        standing: &StandingDeclarations,
        actor: &NamedActorId,
        declaration: &ActorDecl,
    ) -> Result<
        UnboundInvocationAuthority<ProductActor<T>, TapPilotGrantBundle>,
        TapPilotActivationError,
    > {
        require_admitted::<T>(self.registry, *declaration.domain().actor_type())
            .map_err(TapPilotActivationError::Boundary)?;
        self.issuer.instance = standing
            .grant_for(actor, declaration)?
            .map(|(actor, admitted)| (actor, InstanceAuthority::granted(&admitted)));
        let handed;
        let grants = match standing.prototype_of(actor) {
            Some(prototype) => {
                handed = self.grants.handed_to_cell(actor, &prototype);
                &handed
            }
            None => &self.grants,
        };
        activate_registered(
            ActivationRequest::new(actor, declaration, standing.inlets_for(actor)),
            &UnconditionalRequirementResolver,
            grants,
            &mut self.issuer,
            &mut self.dispatcher,
        )
        .map_err(TapPilotActivationError::Registered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor_registry::default_plan_port;
    use crate::authoring_assembly::projection::AuthoredProjectionBuilder;
    use circular_actors::{FieldMap, Name as ShapeName, ProductPayload, ProductValue, Shape, Side};
    use circular_core::Value;
    use circular_core::{
        BaseShape, Causality, Event, EventId, GroundShape, ManualTimeSource, NonZeroTicks,
        OperationIdentity, Sequence, Stamp, StreamIdentity, Tick, TickSource, Ticks,
        TicksPerSecond,
    };
    use circular_plan::{
        ActorDomain, ActorFlags, ActorId, Config, ConfigValue, Delivery, EdgeAttrs, EdgeId,
        Endpoint, Generation, GenerationVector, Incarnation, Name, NonContainerActorDecl, PortId,
        PositiveCapacity, ScopeRole, WirePolicy,
    };
    use circular_runtime::{
        Capability, CapabilitySet, IncarnationState, IngressEdges, RestartBudget, RestartParameters,
    };
    use std::collections::BTreeMap;
    use std::num::NonZeroUsize;
    use std::sync::Arc;

    fn assert_peer_grant(
        capability: Capability,
        configure: impl FnOnce(ProductGrantCatalog, NamedActorId) -> ProductGrantCatalog,
    ) {
        let actor = NamedActorId::new(
            circular_plan::ScopeId::root(),
            Name::from_normalized("peer"),
        );
        let catalog = configure(ProductGrantCatalog::new(), actor.clone());
        assert!(catalog.contains(&actor, capability));
        for required in [CapabilitySet::singleton(capability), CapabilitySet::empty()] {
            let bundle = TapPilotGrantBundle::issue_typed(
                actor.clone(),
                None,
                catalog.get(&actor),
                &required,
            );
            for (candidate, present) in [
                (
                    Capability::PeerDiscover,
                    bundle.peer_discover_authority().is_some(),
                ),
                (Capability::PeerSend, bundle.peer_send_authority().is_some()),
                (
                    Capability::PeerAdvertise,
                    bundle.peer_advertise_authority().is_some(),
                ),
                (
                    Capability::PeerReceive,
                    bundle.peer_receive_authority().is_some(),
                ),
            ] {
                assert_eq!(
                    present,
                    required.contains(candidate),
                    "grant {capability:?}, required {required:?}, bearer {candidate:?}",
                );
            }
        }
    }

    #[test]
    fn peer_discover_grant_issues_only_requested_discover_authority() {
        assert_peer_grant(Capability::PeerDiscover, |catalog, actor| {
            catalog.with_peer_discover(actor, PeerDiscoverGrant::peer_discover([]))
        });
    }

    #[test]
    fn peer_send_grant_issues_only_requested_send_authority() {
        assert_peer_grant(Capability::PeerSend, |catalog, actor| {
            catalog.with_peer_send(actor, PeerSendGrant::peer_send([]))
        });
    }

    #[test]
    fn peer_advertise_grant_issues_only_requested_advertise_authority() {
        assert_peer_grant(Capability::PeerAdvertise, |catalog, actor| {
            catalog.with_peer_advertise(actor, PeerAdvertiseGrant::peer_advertise([]))
        });
    }

    #[test]
    fn peer_receive_grant_issues_only_requested_receive_authority() {
        assert_peer_grant(Capability::PeerReceive, |catalog, actor| {
            catalog.with_peer_receive(actor, PeerReceiveGrant::peer_receive([]))
        });
    }

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    struct PilotRun(u64);

    impl StreamIdentity for PilotRun {}

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct PilotOperation(circular_plan::ActorId, Sequence);

    impl OperationIdentity for PilotOperation {}

    use circular_core::ProducerLocalOperation as _;

    impl circular_core::ProducerLocalOperation<circular_plan::ActorId> for PilotOperation {
        fn issue(producer: &circular_plan::ActorId, sequence: Sequence) -> Self {
            Self(producer.clone(), sequence)
        }
    }

    struct PilotTypes;

    impl ActorTypes for PilotTypes {
        type Stream = PilotRun;
        type Event = Event<PilotRun, ActorId, ProductPayload, PilotOperation>;
        type Payload = ProductPayload;
        type EffectId = circular_runtime::EffectId;
        type StateVersion = u16;
        type Observation = circular_actors::ProductObservation;
        type Grants = TapPilotGrantBundle;

        fn payload(event: &Self::Event) -> &Self::Payload {
            event.payload()
        }
    }

    struct PublishedBangPlan {
        first: NamedActorId,
        second: NamedActorId,
    }

    fn declaration(actor_type: ActorType, config: Config) -> NonContainerActorDecl {
        NonContainerActorDecl::try_new(ActorDomain::new(actor_type, config), ActorFlags::default())
            .expect("pilot test declaration is not a container")
    }

    fn bang_edge_attrs(policy: WirePolicy) -> EdgeAttrs {
        EdgeAttrs::new(Ticks::ZERO, policy).with_preprocess(circular_plan::PreprocessChain::new(
            vec![circular_plan::PreprocessStep::new(
                circular_plan::PreprocessKind::Bang,
                Config::default(),
            )],
        ))
    }

    fn published_bang_plan() -> PublishedBangPlan {
        let source_outlet = default_plan_port(ActorType::FixtureInput, Side::Outlet).unwrap();
        let bang_inlet = default_plan_port(ActorType::Tap, Side::Inlet).unwrap();
        let bang_outlet = default_plan_port(ActorType::Tap, Side::Outlet).unwrap();
        let policy = WirePolicy::new(Delivery::Lossless, Some(PositiveCapacity::new(1).unwrap()));
        let mut builder = AuthoredProjectionBuilder::new();
        let source = builder
            .add_actor(
                Name::from_normalized("source"),
                declaration(ActorType::FixtureInput, Config::default()),
            )
            .unwrap();
        let first = builder
            .add_actor(
                Name::from_normalized("first_bang"),
                declaration(ActorType::Tap, Config::default()),
            )
            .unwrap();
        let second = builder
            .add_actor(
                Name::from_normalized("second_bang"),
                declaration(ActorType::Tap, Config::default()),
            )
            .unwrap();
        builder
            .add_edge(
                Endpoint::new(source, source_outlet),
                Endpoint::new(first.clone(), bang_inlet.clone()),
                0,
                EdgeAttrs::new(Ticks::ZERO, policy),
            )
            .unwrap();
        builder
            .add_edge(
                Endpoint::new(first.clone(), bang_outlet),
                Endpoint::new(second.clone(), bang_inlet),
                0,
                bang_edge_attrs(policy),
            )
            .unwrap();
        let plan = builder.finish().unwrap();
        resolve_plan(&plan, RegistryProfile::Fixture).unwrap();
        PublishedBangPlan { first, second }
    }

    #[test]
    fn registered_relay_validation_rejects_foreign_spec_and_nonempty_requirements() {
        assert_eq!(
            validate_registered_element(
                ActorType::Tap,
                registration(ActorType::Route).spec(),
                &CapabilitySet::empty(),
                &Config::default(),
            ),
            Err(TapPilotBoundaryError::CanonicalSpecMismatch)
        );
        assert_eq!(
            validate_registered_element(
                ActorType::Tap,
                registration(ActorType::Tap).spec(),
                &CapabilitySet::singleton(Capability::FsRead),
                &Config::default(),
            ),
            Err(TapPilotBoundaryError::UnexpectedRequirements)
        );
    }

    #[test]
    fn dispatcher_helpers_reject_source_arm_and_foreign_bundle_actor() {
        let PublishedBangPlan { first, second, .. } = published_bang_plan();
        let bundle = TapPilotGrantBundle {
            actor: first.clone(),
            instance: None,
            agent_harness: None,
            fs_read: None,
            fs_write: None,
            http_fetch: None,
            process_spawn: None,
            user_notify: None,
            peer_discover: None,
            peer_send: None,
            peer_advertise: None,
            peer_receive: None,
        };

        assert_eq!(validate_bundle_actor(&bundle, &first), Ok(()));
        assert_eq!(
            validate_bundle_actor(&bundle, &second),
            Err(TapPilotBoundaryError::BundleActorMismatch)
        );
        assert!(matches!(
            reject_source_arm::<ProductActor<PilotTypes>>(),
            Err(TapPilotBoundaryError::UnexpectedSourceArm)
        ));
    }

    #[test]
    fn published_plan_gate_rejects_fixture_actors_before_relay_activation() {
        let mut builder = AuthoredProjectionBuilder::new();
        builder
            .add_actor(
                Name::from_normalized("fixture"),
                declaration(ActorType::FixtureInput, Config::default()),
            )
            .unwrap();
        let bang = builder
            .add_actor(
                Name::from_normalized("bang"),
                declaration(ActorType::Tap, Config::default()),
            )
            .unwrap();
        let plan = builder.finish().unwrap();
        let mut profile = TapPilotActivationProfile::<PilotTypes>::default();

        assert!(matches!(
            profile.activate_from_plan(&plan, &bang),
            Err(TapPilotActivationError::AuthoredProjection(
                PlanRegistryError::FixtureLocalActorInPublishedPlan { .. }
            ))
        ));
    }

    #[test]
    fn nonempty_relay_config_is_rejected_through_registered_activation() {
        let config = Config::try_new(vec![(
            Name::from_normalized("unexpected"),
            ConfigValue::List(Box::new([])),
        )])
        .unwrap();
        let mut builder = AuthoredProjectionBuilder::new();
        let bang = builder
            .add_actor(
                Name::from_normalized("bang"),
                declaration(ActorType::Tap, config),
            )
            .unwrap();
        let plan = builder.finish().unwrap();
        let declaration: &ActorDecl = plan.graph().actors().get(&bang).unwrap();
        assert_ne!(declaration.domain().config(), &Config::default());
        let mut profile = TapPilotActivationProfile::<PilotTypes>::default();

        let result = profile.activate_from_plan(&plan, &bang);
        let reason = result.as_ref().err().unwrap().to_string();
        assert!(
            reason.contains("ConfigRejected: actor `bang`; config.unexpected = []"),
            "{reason}"
        );
        assert!(matches!(
            result,
            Err(TapPilotActivationError::Registered(
                ActivationError::FactoryActivation {
                    source: TapPilotBoundaryError::Factory(
                        ProductFactoryError::EmptyConfig(
                            circular_actors::EmptyConfigFactoryError::NonEmptyConfig
                        ),
                        _
                    ),
                    ..
                }
            ))
        ));
    }
}
