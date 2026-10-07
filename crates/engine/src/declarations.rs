
use crate::actor_registry::RegistryProfile;
use crate::authoring_assembly::projection::AuthoredProjection;
use crate::run_graph::flatten;
use circular_core::StreamIdentity;
use circular_plan::{Generation, GenerationVector, Incarnation, NamedActorId};

#[derive(Debug)]
pub enum DeclarationsError {
    Declarations(Box<crate::tap_pilot::TapPilotActivationError>),
    Graph(crate::run_graph::GraphError),
}

impl std::fmt::Display for DeclarationsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Declarations(error) => write!(formatter, "{error}"),
            Self::Graph(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for DeclarationsError {}

#[derive(Clone, Debug)]
pub struct RevisionDeclarations {
    graph: crate::run_graph::RunGraph,
    standing: crate::tap_pilot::StandingDeclarations,
    registry: RegistryProfile,
}

impl RevisionDeclarations {
    pub(crate) fn parts(
        &self,
    ) -> (
        &crate::run_graph::RunGraph,
        &crate::tap_pilot::StandingDeclarations,
        RegistryProfile,
    ) {
        (&self.graph, &self.standing, self.registry)
    }

    pub fn admit(
        plan: &AuthoredProjection,
        registry: RegistryProfile,
    ) -> Result<Self, DeclarationsError> {
        let standing = crate::tap_pilot::StandingDeclarations::admit(plan, registry)
            .map_err(|error| DeclarationsError::Declarations(Box::new(error)))?;
        Ok(Self {
            graph: flatten(plan).map_err(DeclarationsError::Graph)?,
            standing,
            registry,
        })
    }

    pub fn published(plan: &AuthoredProjection) -> Result<Self, DeclarationsError> {
        Self::admit(plan, RegistryProfile::Published)
    }

    pub fn fixture(plan: &AuthoredProjection) -> Result<Self, DeclarationsError> {
        Self::admit(plan, RegistryProfile::Fixture)
    }

    #[must_use]
    pub const fn graph(&self) -> &crate::run_graph::RunGraph {
        &self.graph
    }
}

pub(crate) fn fresh_incarnation<R: StreamIdentity>(
    run: R,
    actor: &NamedActorId,
    generation: u64,
) -> Incarnation<R> {
    let mut generations = vec![Generation::new(0); actor.scope().depth() + 1];
    if let Some(own) = generations.last_mut() {
        *own = Generation::new(generation);
    }
    Incarnation::new(
        run,
        actor.as_scoped().clone(),
        GenerationVector::for_actor(actor.as_scoped(), generations)
            .expect("one generation per ancestor and actor"),
    )
}
