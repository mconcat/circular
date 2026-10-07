
use std::error::Error;
use std::fmt;

use crate::UnboundInvocationAuthority;
use circular_actors::{ActorSpec, Condition, FactoryArm, registration};
use circular_plan::{ActorDecl, ActorType, Config, NamedActorId};
use circular_runtime::{Capability, CapabilitySet};

pub(crate) struct ActivationAuthoritySeal {
    _private: (),
}

#[derive(Clone, Copy, Debug)]
pub struct ActivationRequest<'a> {
    actor: &'a NamedActorId,
    declaration: &'a ActorDecl,
    inlets: &'a circular_actors::ResolvedInletShapes,
}

impl<'a> ActivationRequest<'a> {
    #[must_use]
    pub const fn new(
        actor: &'a NamedActorId,
        declaration: &'a ActorDecl,
        inlets: &'a circular_actors::ResolvedInletShapes,
    ) -> Self {
        Self {
            actor,
            declaration,
            inlets,
        }
    }

    #[must_use]
    pub const fn actor(&self) -> &'a NamedActorId {
        self.actor
    }

    #[must_use]
    pub const fn declaration(&self) -> &'a ActorDecl {
        self.declaration
    }
}

pub trait RequirementResolver {
    type Error;

    fn condition_holds(
        &self,
        actor_type: ActorType,
        capability: Capability,
        condition: &Condition,
        config: &Config,
    ) -> Result<bool, Self::Error>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UnconditionalRequirementResolver;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StaticRequirementError {
    ConditionalRuleNeedsConfigAdapter {
        actor_type: ActorType,
        capability: Capability,
    },
}

impl fmt::Display for StaticRequirementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConditionalRuleNeedsConfigAdapter {
                actor_type,
                capability,
            } => write!(
                formatter,
                "registered actor type {actor_type} has a conditional {capability} requirement, but the plan-config adapter is not defined"
            ),
        }
    }
}

impl Error for StaticRequirementError {}

impl RequirementResolver for UnconditionalRequirementResolver {
    type Error = StaticRequirementError;

    fn condition_holds(
        &self,
        actor_type: ActorType,
        capability: Capability,
        condition: &Condition,
        _config: &Config,
    ) -> Result<bool, Self::Error> {
        if matches!(condition, Condition::Always) {
            Ok(true)
        } else {
            Err(StaticRequirementError::ConditionalRuleNeedsConfigAdapter {
                actor_type,
                capability,
            })
        }
    }
}

pub trait ScopeGrantView {
    fn contains(&self, actor: &NamedActorId, capability: Capability) -> bool;
}

pub struct CapabilityAdmission<'a, G: ?Sized> {
    actor: &'a NamedActorId,
    actor_type: ActorType,
    spec: &'static ActorSpec,
    config: &'a Config,
    required: &'a CapabilitySet,
    grants: &'a G,
}

impl<'a, G> CapabilityAdmission<'a, G>
where
    G: ScopeGrantView + ?Sized,
{
    #[must_use]
    pub const fn actor(&self) -> &'a NamedActorId {
        self.actor
    }

    #[must_use]
    pub const fn actor_type(&self) -> ActorType {
        self.actor_type
    }

    #[must_use]
    pub const fn spec(&self) -> &'static ActorSpec {
        self.spec
    }

    #[must_use]
    pub const fn config(&self) -> &'a Config {
        self.config
    }

    #[must_use]
    pub const fn required(&self) -> &'a CapabilitySet {
        self.required
    }

    #[must_use]
    pub const fn grants(&self) -> &'a G {
        self.grants
    }

    #[must_use]
    pub fn is_granted(&self, capability: Capability) -> bool {
        self.grants.contains(self.actor, capability)
    }
}

pub trait GrantBundleIssuer<G: ScopeGrantView + ?Sized> {
    type Bundle;
    type Error;

    fn issue(&mut self, admission: CapabilityAdmission<'_, G>)
    -> Result<Self::Bundle, Self::Error>;
}

pub struct FactoryActivation<'a, B: ?Sized> {
    actor: &'a NamedActorId,
    actor_type: ActorType,
    spec: &'static ActorSpec,
    config: &'a Config,
    required: &'a CapabilitySet,
    bundle: &'a B,
    inlets: &'a circular_actors::ResolvedInletShapes,
}

impl<'a, B: ?Sized> FactoryActivation<'a, B> {
    #[must_use]
    pub const fn actor(&self) -> &'a NamedActorId {
        self.actor
    }

    #[must_use]
    pub const fn actor_type(&self) -> ActorType {
        self.actor_type
    }

    #[must_use]
    pub const fn spec(&self) -> &'static ActorSpec {
        self.spec
    }

    #[must_use]
    pub const fn config(&self) -> &'a Config {
        self.config
    }

    #[must_use]
    pub const fn required(&self) -> &'a CapabilitySet {
        self.required
    }

    #[must_use]
    pub const fn bundle(&self) -> &'a B {
        self.bundle
    }

    #[must_use]
    pub const fn inlets(&self) -> &'a circular_actors::ResolvedInletShapes {
        self.inlets
    }
}

pub trait RegisteredFactoryDispatcher<B> {
    type Activated;
    type Error;

    fn activate_actor(
        &mut self,
        request: FactoryActivation<'_, B>,
    ) -> Result<Self::Activated, Self::Error>;

    fn activate_source(
        &mut self,
        request: FactoryActivation<'_, B>,
    ) -> Result<Self::Activated, Self::Error>;
}

pub type ActivationResult<T, R, I, F> = Result<T, ActivationError<R, I, F>>;

#[allow(clippy::type_complexity)]
pub fn activate_registered<R, G, I, D>(
    request: ActivationRequest<'_>,
    resolver: &R,
    grants: &G,
    issuer: &mut I,
    dispatcher: &mut D,
) -> ActivationResult<
    UnboundInvocationAuthority<D::Activated, I::Bundle>,
    R::Error,
    I::Error,
    D::Error,
>
where
    R: RequirementResolver,
    G: ScopeGrantView + ?Sized,
    I: GrantBundleIssuer<G>,
    D: RegisteredFactoryDispatcher<I::Bundle>,
{
    let actor = request.actor();
    let declaration = request.declaration();
    let actor_type = *declaration.domain().actor_type();
    let row = registration(actor_type);
    let spec = row.spec();
    let config = declaration.domain().config();

    let mut required = CapabilitySet::empty();
    for rule in spec.requires() {
        if !crate::actor_capability::required(declaration, rule.capability()) {
            continue;
        }
        let active = match rule.condition() {
            Condition::Always => true,
            condition => resolver
                .condition_holds(actor_type, rule.capability(), condition, config)
                .map_err(|source| ActivationError::RequirementResolution {
                    actor: actor.clone(),
                    actor_type,
                    source,
                })?,
        };
        if active {
            required = required.join(&CapabilitySet::singleton(rule.capability()));
        }
    }

    let missing = required
        .iter()
        .filter(|capability| !grants.contains(actor, *capability))
        .collect::<Vec<_>>()
        .into_boxed_slice();
    if !missing.is_empty() {
        return Err(ActivationError::CapabilityDenied {
            actor: actor.clone(),
            actor_type,
            missing,
        });
    }

    let bundle = issuer
        .issue(CapabilityAdmission {
            actor,
            actor_type,
            spec,
            config,
            required: &required,
            grants,
        })
        .map_err(|source| ActivationError::GrantIssuance {
            actor: actor.clone(),
            actor_type,
            source,
        })?;

    let factory = row.factory();
    let activation = FactoryActivation {
        actor,
        actor_type,
        spec,
        config,
        required: &required,
        bundle: &bundle,
        inlets: request.inlets,
    };
    let result = match factory {
        FactoryArm::Actor => dispatcher.activate_actor(activation),
        FactoryArm::Source => dispatcher.activate_source(activation),
    };
    let instance = result.map_err(|source| ActivationError::FactoryActivation {
        actor: actor.clone(),
        actor_type,
        factory,
        source,
    })?;
    Ok(UnboundInvocationAuthority::new(
        ActivationAuthoritySeal { _private: () },
        actor.clone(),
        declaration.clone(),
        required,
        bundle,
        instance,
    ))
}

#[derive(Debug)]
pub enum ActivationError<R, I, F> {
    RequirementResolution {
        actor: NamedActorId,
        actor_type: ActorType,
        source: R,
    },
    CapabilityDenied {
        actor: NamedActorId,
        actor_type: ActorType,
        missing: Box<[Capability]>,
    },
    GrantIssuance {
        actor: NamedActorId,
        actor_type: ActorType,
        source: I,
    },
    FactoryActivation {
        actor: NamedActorId,
        actor_type: ActorType,
        factory: FactoryArm,
        source: F,
    },
}

impl<R, I, F> ActivationError<R, I, F> {
    #[must_use]
    pub const fn actor(&self) -> &NamedActorId {
        match self {
            Self::RequirementResolution { actor, .. }
            | Self::CapabilityDenied { actor, .. }
            | Self::GrantIssuance { actor, .. }
            | Self::FactoryActivation { actor, .. } => actor,
        }
    }

    #[must_use]
    pub const fn actor_type(&self) -> ActorType {
        match self {
            Self::RequirementResolution { actor_type, .. }
            | Self::CapabilityDenied { actor_type, .. }
            | Self::GrantIssuance { actor_type, .. }
            | Self::FactoryActivation { actor_type, .. } => *actor_type,
        }
    }
}

impl<R, I, F> fmt::Display for ActivationError<R, I, F>
where
    R: fmt::Display,
    I: fmt::Display,
    F: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequirementResolution {
                actor,
                actor_type,
                source,
            } => write!(
                formatter,
                "capability requirements for actor `{actor}` of type {actor_type} could not be resolved: {source}"
            ),
            Self::CapabilityDenied {
                actor,
                actor_type,
                missing,
            } => write!(
                formatter,
                "CapabilityDenied: activation denied for actor `{actor}` of type {actor_type}; missing capabilities: {}",
                missing
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::GrantIssuance {
                actor,
                actor_type,
                source,
            } => write!(
                formatter,
                "grant bundle issuance stopped for actor `{actor}` of type {actor_type}: {source}"
            ),
            Self::FactoryActivation {
                actor,
                actor_type,
                source,
                ..
            } => write!(
                formatter,
                "factory activation failed for actor `{actor}` of type {actor_type}: {source}"
            ),
        }
    }
}

impl<R, I, F> Error for ActivationError<R, I, F>
where
    R: Error + 'static,
    I: Error + 'static,
    F: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::RequirementResolution { source, .. } => Some(source),
            Self::GrantIssuance { source, .. } => Some(source),
            Self::FactoryActivation { source, .. } => Some(source),
            Self::CapabilityDenied { .. } => None,
        }
    }
}

#[cfg(test)]
fn capability_set(capabilities: impl IntoIterator<Item = Capability>) -> CapabilitySet {
    capabilities
        .into_iter()
        .fold(CapabilitySet::empty(), |set, capability| {
            set.join(&CapabilitySet::singleton(capability))
        })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::convert::Infallible;
    use std::rc::Rc;

    use circular_plan::{ActorDomain, ActorFlags, Name, ScopeId};

    use super::*;

    fn actor_id(value: &str) -> NamedActorId {
        NamedActorId::new(ScopeId::root(), Name::from_normalized(value))
    }

    fn declaration(actor_type: ActorType) -> ActorDecl {
        ActorDecl::new(
            ActorDomain::new(
                actor_type,
                crate::actor_capability::tests::fixture_config(actor_type, Config::default()),
            ),
            ActorFlags::default(),
        )
    }

    struct TestGrants(CapabilitySet);

    impl ScopeGrantView for TestGrants {
        fn contains(&self, _actor: &NamedActorId, capability: Capability) -> bool {
            self.0.contains(capability)
        }
    }

    #[derive(Debug, Eq, PartialEq)]
    struct TestBundle(Box<[Capability]>);

    #[derive(Default)]
    struct RecordingIssuer {
        calls: usize,
    }

    impl GrantBundleIssuer<TestGrants> for RecordingIssuer {
        type Bundle = TestBundle;
        type Error = Infallible;

        fn issue(
            &mut self,
            admission: CapabilityAdmission<'_, TestGrants>,
        ) -> Result<Self::Bundle, Self::Error> {
            self.calls += 1;
            let required = admission.required().iter().collect::<Vec<_>>();
            assert!(
                required
                    .iter()
                    .all(|capability| admission.is_granted(*capability))
            );
            Ok(TestBundle(required.into_boxed_slice()))
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Activated {
        arm: FactoryArm,
        actor_type: ActorType,
        required: Box<[Capability]>,
    }

    #[derive(Default)]
    struct RecordingDispatcher {
        actor_calls: usize,
        source_calls: usize,
    }

    impl RegisteredFactoryDispatcher<TestBundle> for RecordingDispatcher {
        type Activated = Activated;
        type Error = Infallible;

        fn activate_actor(
            &mut self,
            request: FactoryActivation<'_, TestBundle>,
        ) -> Result<Self::Activated, Self::Error> {
            self.actor_calls += 1;
            let actor_type = request.actor_type();
            let required = request.bundle().0.clone();
            Ok(Activated {
                arm: FactoryArm::Actor,
                actor_type,
                required,
            })
        }

        fn activate_source(
            &mut self,
            request: FactoryActivation<'_, TestBundle>,
        ) -> Result<Self::Activated, Self::Error> {
            self.source_calls += 1;
            let actor_type = request.actor_type();
            let required = request.bundle().0.clone();
            Ok(Activated {
                arm: FactoryArm::Source,
                actor_type,
                required,
            })
        }
    }

    #[test]
    fn denial_reports_all_missing_capabilities_before_issuance_or_dispatch() {
        let actor = actor_id("tap");
        let declaration = declaration(ActorType::FixtureTap);
        let grants = TestGrants(CapabilitySet::empty());
        let mut issuer = RecordingIssuer::default();
        let mut dispatcher = RecordingDispatcher::default();

        let error = activate_registered(
            ActivationRequest::new(
                &actor,
                &declaration,
                &circular_actors::ResolvedInletShapes::default(),
            ),
            &UnconditionalRequirementResolver,
            &grants,
            &mut issuer,
            &mut dispatcher,
        )
        .unwrap_err();

        match error {
            ActivationError::CapabilityDenied {
                actor: denied,
                actor_type,
                missing,
            } => {
                assert_eq!(denied, actor);
                assert_eq!(actor_type, ActorType::FixtureTap);
                assert_eq!(&*missing, &[Capability::FsRead, Capability::FsWrite]);
            }
            other => panic!("unexpected activation error: {other:?}"),
        }
        assert_eq!(issuer.calls, 0);
        assert_eq!(dispatcher.actor_calls, 0);
        assert_eq!(dispatcher.source_calls, 0);
    }

    #[test]
    fn admitted_registration_reaches_exactly_its_actor_factory_arm() {
        let actor = actor_id("tap");
        let declaration = declaration(ActorType::FixtureTap);
        let grants = TestGrants(capability_set([Capability::FsRead, Capability::FsWrite]));
        let mut issuer = RecordingIssuer::default();
        let mut dispatcher = RecordingDispatcher::default();

        let activated = activate_registered(
            ActivationRequest::new(
                &actor,
                &declaration,
                &circular_actors::ResolvedInletShapes::default(),
            ),
            &UnconditionalRequirementResolver,
            &grants,
            &mut issuer,
            &mut dispatcher,
        )
        .unwrap();

        assert_eq!(
            activated.actor(),
            &Activated {
                arm: FactoryArm::Actor,
                actor_type: ActorType::FixtureTap,
                required: vec![Capability::FsRead, Capability::FsWrite].into_boxed_slice(),
            }
        );
        assert_eq!(issuer.calls, 1);
        assert_eq!(dispatcher.actor_calls, 1);
        assert_eq!(dispatcher.source_calls, 0);
    }

    #[test]
    fn source_registration_reaches_only_the_source_factory_arm() {
        let actor = actor_id("input");
        let declaration = declaration(ActorType::FixtureInput);
        let grants = TestGrants(CapabilitySet::empty());
        let mut issuer = RecordingIssuer::default();
        let mut dispatcher = RecordingDispatcher::default();

        let activated = activate_registered(
            ActivationRequest::new(
                &actor,
                &declaration,
                &circular_actors::ResolvedInletShapes::default(),
            ),
            &UnconditionalRequirementResolver,
            &grants,
            &mut issuer,
            &mut dispatcher,
        )
        .unwrap();

        assert_eq!(activated.actor().arm, FactoryArm::Source);
        assert_eq!(activated.actor_type(), ActorType::FixtureInput);
        assert_eq!(activated.required().iter().count(), 0);
        assert_eq!(issuer.calls, 1);
        assert_eq!(dispatcher.actor_calls, 0);
        assert_eq!(dispatcher.source_calls, 1);
    }

    struct RejectEveryConditionalRule {
        calls: Cell<usize>,
    }

    impl RequirementResolver for RejectEveryConditionalRule {
        type Error = Infallible;

        fn condition_holds(
            &self,
            _actor_type: ActorType,
            _capability: Capability,
            _condition: &Condition,
            _config: &Config,
        ) -> Result<bool, Self::Error> {
            self.calls.set(self.calls.get() + 1);
            Ok(false)
        }
    }

    #[test]
    fn activation_owns_unconditional_rules_instead_of_the_resolver() {
        let actor = actor_id("tap");
        let declaration = declaration(ActorType::FixtureTap);
        let grants = TestGrants(capability_set(Capability::ALL));
        let resolver = RejectEveryConditionalRule {
            calls: Cell::new(0),
        };
        let mut issuer = RecordingIssuer::default();
        let mut dispatcher = RecordingDispatcher::default();

        let activated = activate_registered(
            ActivationRequest::new(
                &actor,
                &declaration,
                &circular_actors::ResolvedInletShapes::default(),
            ),
            &resolver,
            &grants,
            &mut issuer,
            &mut dispatcher,
        )
        .expect("unconditional requirements are collected without resolver delegation");

        assert_eq!(resolver.calls.get(), 0);
        assert_eq!(
            activated.required().iter().collect::<Vec<_>>(),
            vec![Capability::FsRead, Capability::FsWrite]
        );
        assert_eq!(issuer.calls, 1);
        assert_eq!(dispatcher.actor_calls, 1);
        assert_eq!(dispatcher.source_calls, 0);
    }

    struct TrackedBundle {
        drops: Rc<Cell<usize>>,
    }

    impl fmt::Debug for TrackedBundle {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("SENSITIVE_GRANT_SENTINEL")
        }
    }

    impl Drop for TrackedBundle {
        fn drop(&mut self) {
            self.drops.set(self.drops.get() + 1);
        }
    }

    struct TrackedIssuer {
        drops: Rc<Cell<usize>>,
        calls: usize,
    }

    impl GrantBundleIssuer<TestGrants> for TrackedIssuer {
        type Bundle = TrackedBundle;
        type Error = Infallible;

        fn issue(
            &mut self,
            _admission: CapabilityAdmission<'_, TestGrants>,
        ) -> Result<Self::Bundle, Self::Error> {
            self.calls += 1;
            Ok(TrackedBundle {
                drops: Rc::clone(&self.drops),
            })
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct FactoryRejected;

    impl fmt::Display for FactoryRejected {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("fixture factory rejected")
        }
    }

    impl Error for FactoryRejected {}

    struct TrackedDispatcher {
        reject: bool,
        calls: usize,
    }

    struct TrackedActor;

    impl fmt::Debug for TrackedActor {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("SENSITIVE_ACTOR_SENTINEL")
        }
    }

    impl RegisteredFactoryDispatcher<TrackedBundle> for TrackedDispatcher {
        type Activated = TrackedActor;
        type Error = FactoryRejected;

        fn activate_actor(
            &mut self,
            request: FactoryActivation<'_, TrackedBundle>,
        ) -> Result<Self::Activated, Self::Error> {
            self.dispatch(request)
        }

        fn activate_source(
            &mut self,
            request: FactoryActivation<'_, TrackedBundle>,
        ) -> Result<Self::Activated, Self::Error> {
            self.dispatch(request)
        }
    }

    impl TrackedDispatcher {
        fn dispatch(
            &mut self,
            request: FactoryActivation<'_, TrackedBundle>,
        ) -> Result<TrackedActor, FactoryRejected> {
            self.calls += 1;
            assert_eq!(request.bundle().drops.get(), 0);
            if self.reject {
                Err(FactoryRejected)
            } else {
                Ok(TrackedActor)
            }
        }
    }

    #[test]
    fn successful_activation_keeps_actor_and_bundle_in_one_unbound_authority() {
        let actor = actor_id("input");
        let declaration = declaration(ActorType::FixtureInput);
        let grants = TestGrants(CapabilitySet::empty());
        let drops = Rc::new(Cell::new(0));
        let mut issuer = TrackedIssuer {
            drops: Rc::clone(&drops),
            calls: 0,
        };
        let mut dispatcher = TrackedDispatcher {
            reject: false,
            calls: 0,
        };

        let authority = activate_registered(
            ActivationRequest::new(
                &actor,
                &declaration,
                &circular_actors::ResolvedInletShapes::default(),
            ),
            &UnconditionalRequirementResolver,
            &grants,
            &mut issuer,
            &mut dispatcher,
        )
        .expect("fixture input activation succeeds");

        assert_eq!(authority.actor_id(), &actor);
        assert_eq!(authority.actor_type(), ActorType::FixtureInput);
        assert_eq!(authority.config(), declaration.domain().config());
        assert_eq!(authority.flags(), declaration.flags());
        assert_eq!(
            authority.lifecycle(),
            crate::InvocationLifecycle::LifecycleUnbound
        );
        assert_eq!(
            authority.readiness(),
            crate::InvocationReadiness::DomainUnbound
        );
        assert_eq!(authority.grants().drops.get(), 0);
        let rendered = format!("{authority:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(!rendered.contains("SENSITIVE_GRANT_SENTINEL"));
        assert!(!rendered.contains("SENSITIVE_ACTOR_SENTINEL"));
        assert_eq!(issuer.calls, 1);
        assert_eq!(dispatcher.calls, 1);
        assert_eq!(
            drops.get(),
            0,
            "the authority keeps owning the grant bundle"
        );

        drop(authority);
        assert_eq!(drops.get(), 1);
    }

    #[test]
    fn factory_failure_returns_no_authority_and_drops_the_unpublished_bundle() {
        let actor = actor_id("input");
        let declaration = declaration(ActorType::FixtureInput);
        let grants = TestGrants(CapabilitySet::empty());
        let drops = Rc::new(Cell::new(0));
        let mut issuer = TrackedIssuer {
            drops: Rc::clone(&drops),
            calls: 0,
        };
        let mut dispatcher = TrackedDispatcher {
            reject: true,
            calls: 0,
        };

        let result = activate_registered(
            ActivationRequest::new(
                &actor,
                &declaration,
                &circular_actors::ResolvedInletShapes::default(),
            ),
            &UnconditionalRequirementResolver,
            &grants,
            &mut issuer,
            &mut dispatcher,
        );

        assert!(matches!(
            result,
            Err(ActivationError::FactoryActivation {
                source: FactoryRejected,
                ..
            })
        ));
        assert_eq!(issuer.calls, 1);
        assert_eq!(dispatcher.calls, 1);
        assert_eq!(
            drops.get(),
            1,
            "a failed actor keeps no grant and no canonical lifecycle"
        );
    }
}
