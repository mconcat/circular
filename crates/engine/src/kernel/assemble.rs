
use super::turn::{Behavior, ProductTypes};
use crate::actor_registry::RegistryProfile;
use crate::tap_pilot::{ProductGrantCatalog, StandingDeclarations, TapPilotActivationProfile};
use circular_plan::{ActorDecl, NamedActorId};
use circular_store::StreamId;
use std::sync::Arc;

pub(crate) struct Assembly {
    stream: StreamId,
    registry: RegistryProfile,
    profile: TapPilotActivationProfile<ProductTypes>,
    standing: StandingDeclarations,
    generation: u64,
    execution: Option<Arc<crate::execution_profile::ProductExecutionProfile>>,
}

#[derive(Debug)]
pub(crate) struct Unassembled {
    pub(crate) actor: NamedActorId,
    pub(crate) reason: String,
    pub(crate) detail: Option<circular_actors::FailureDetail>,
}

impl Unassembled {
    pub(crate) fn refused(
        actor: &NamedActorId,
        step: &str,
        failure: &crate::activation_detail::RegistrationFailure,
    ) -> Self {
        Self {
            actor: actor.clone(),
            reason: format!("{step}: {failure}"),
            detail: Some(failure.detail()),
        }
    }
}

impl Assembly {
    pub(crate) fn new(
        stream: StreamId,
        registry: RegistryProfile,
        grants: ProductGrantCatalog,
        standing: StandingDeclarations,
        generation: u64,
    ) -> Self {
        Self {
            stream,
            registry,
            profile: TapPilotActivationProfile::on_with_grants(registry, grants),
            standing,
            generation,
            execution: None,
        }
    }

    pub(crate) fn executing(
        mut self,
        execution: Arc<crate::execution_profile::ProductExecutionProfile>,
    ) -> Self {
        self.execution = Some(execution);
        self
    }

    pub(crate) fn boarded(&self, actor_type: circular_core::ActorType) -> bool {
        crate::tap_pilot::actor_factory::<ProductTypes>(self.registry, actor_type).is_some()
    }

    pub(crate) fn behavior(
        &mut self,
        actor: &NamedActorId,
        declaration: &ActorDecl,
    ) -> Result<Behavior, Unassembled> {
        if let Some(failure) = self.standing.refused(actor) {
            return Err(Unassembled::refused(
                actor,
                "declaration admission",
                failure,
            ));
        }
        if let Some(failure) =
            crate::execution_profile::refused_capability_config(actor, declaration)
        {
            return Err(Unassembled::refused(actor, "capability config", &failure));
        }
        let authority = self
            .profile
            .activate_declared(&self.standing, actor, declaration)
            .map_err(|error| Unassembled {
                actor: actor.clone(),
                reason: format!("{error:?}"),
                detail: Some(error.detail()),
            })?;
        let (instance, grants) = authority.into_actor_with_grants();
        let source = super::source::Plan::of(declaration, &instance)
            .map_err(|failure| Unassembled::refused(actor, "source", &failure))?;
        let effects = match &self.execution {
            Some(execution) => super::effect::EffectPort::of(execution, declaration)
                .map_err(|failure| Unassembled::refused(actor, "effect executors", &failure))?,
            None => None,
        };
        Ok(Behavior {
            id: actor.as_actor_id(),
            actor_type: *declaration.domain().actor_type(),
            incarnation: crate::declarations::fresh_incarnation(
                self.stream,
                actor,
                self.generation,
            ),
            config: declaration.domain().config().clone(),
            grants,
            actor: instance,
            effects,
            timers: super::effect::Timers::default(),
            approval: crate::actor_approval::ApprovalDemand::fold(declaration),
            source,
        })
    }
}
