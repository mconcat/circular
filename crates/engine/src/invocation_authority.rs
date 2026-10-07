
use crate::activation::ActivationAuthoritySeal;
use circular_core::{ArrivalIndex, Event, EventPayload, OperationIdentity, StreamIdentity, Tick};
use circular_plan::{ActorDecl, ActorFlags, ActorId, ActorType, Config, Incarnation, NamedActorId};
#[cfg(test)]
use circular_runtime::{Actor, ActorState};
use circular_runtime::{
    ActorArrivals, ActorContext, ActorTypes, CapabilitySet, CheckpointRestore, EditableActor,
    EffectOutcome, EmittingActor, IncarnationState, IncarnationTransition,
    IncarnationTransitionError, IngressEdges, RestartBudget, RestartController,
    RestartableActorFailure,
};
use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::BTreeSet;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvocationLifecycle {
    LifecycleUnbound,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvocationReadiness {
    DomainUnbound,
}

#[derive(Debug)]
struct ActivationMetadata {
    actor: NamedActorId,
    declaration: ActorDecl,
    required: CapabilitySet,
}

pub struct UnboundInvocationAuthority<A, G> {
    metadata: ActivationMetadata,
    grants: G,
    actor: A,
}

impl<A, G> fmt::Debug for UnboundInvocationAuthority<A, G> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnboundInvocationAuthority")
            .field("actor", &self.metadata.actor)
            .field("actor_type", &self.actor_type())
            .field("flags", &self.flags())
            .field("required", &self.metadata.required)
            .field("grants", &"<redacted>")
            .field("actor", &"<redacted>")
            .field("lifecycle", &InvocationLifecycle::LifecycleUnbound)
            .field("readiness", &InvocationReadiness::DomainUnbound)
            .finish()
    }
}

impl<A, G> UnboundInvocationAuthority<A, G> {
    pub(crate) fn into_actor_with_grants(self) -> (A, G) {
        (self.actor, self.grants)
    }

    pub(crate) fn new(
        _seal: ActivationAuthoritySeal,
        actor_id: NamedActorId,
        declaration: ActorDecl,
        required: CapabilitySet,
        grants: G,
        actor: A,
    ) -> Self {
        Self {
            metadata: ActivationMetadata {
                actor: actor_id,
                declaration,
                required,
            },
            grants,
            actor,
        }
    }

    #[must_use]
    pub const fn actor_id(&self) -> &NamedActorId {
        &self.metadata.actor
    }

    #[must_use]
    pub const fn actor_type(&self) -> ActorType {
        *self.metadata.declaration.domain().actor_type()
    }

    #[must_use]
    #[cfg(test)]
    pub const fn config(&self) -> &Config {
        self.metadata.declaration.domain().config()
    }

    #[must_use]
    pub const fn flags(&self) -> ActorFlags {
        self.metadata.declaration.flags()
    }

    #[must_use]
    #[cfg(test)]
    pub const fn required(&self) -> &CapabilitySet {
        &self.metadata.required
    }

    #[must_use]
    #[cfg(test)]
    pub const fn lifecycle(&self) -> InvocationLifecycle {
        InvocationLifecycle::LifecycleUnbound
    }

    #[must_use]
    #[cfg(test)]
    pub const fn readiness(&self) -> InvocationReadiness {
        InvocationReadiness::DomainUnbound
    }

    #[cfg(test)]
    pub(crate) const fn actor(&self) -> &A {
        &self.actor
    }

    #[cfg(test)]
    pub(crate) const fn grants(&self) -> &G {
        &self.grants
    }
}
