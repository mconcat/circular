
use circular_core::{
    EncodedPayload, NonZeroTicks, PayloadVersionTag, StreamIdentity, TicksPerSecond, TimeSourcePlan,
};
use std::fmt::Debug;

pub trait ManifestSchema: Clone + Debug + Eq + std::hash::Hash {
    type Stream: StreamIdentity;
    type Producer: circular_core::ProducerIdentity;
    type Placement: Clone + Debug + Eq;
    type Failure: Clone + Debug + Eq;
    type Versions: Clone + Debug + Eq;
    type RevisionId: Clone + Debug + Eq;
    type AuthoringCut: Clone + Debug + Eq;
    type ScopeId: Clone + Debug + Eq + std::hash::Hash;
    type GrantSet: Clone + Debug + Eq;
    type InputValue: Clone + Debug + Eq;

    type CadencePolicy: Clone + Debug + Eq;
    type TickOrigin: Clone + Debug + Eq;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimeParams<S: ManifestSchema> {
    resolution: TicksPerSecond,
    cadence: NonZeroTicks,
    cadence_policy: S::CadencePolicy,
    time_source_plan: TimeSourcePlan<S::Producer>,
    tick_origin: S::TickOrigin,
}

impl<S: ManifestSchema> TimeParams<S> {
    #[must_use]
    pub const fn new(
        resolution: TicksPerSecond,
        cadence: NonZeroTicks,
        cadence_policy: S::CadencePolicy,
        time_source_plan: TimeSourcePlan<S::Producer>,
        tick_origin: S::TickOrigin,
    ) -> Self {
        Self {
            resolution,
            cadence,
            cadence_policy,
            time_source_plan,
            tick_origin,
        }
    }

    #[must_use]
    pub const fn resolution(&self) -> TicksPerSecond {
        self.resolution
    }

    #[must_use]
    pub const fn cadence(&self) -> NonZeroTicks {
        self.cadence
    }

    #[must_use]
    pub const fn cadence_policy(&self) -> &S::CadencePolicy {
        &self.cadence_policy
    }

    #[must_use]
    pub const fn time_source_plan(&self) -> &TimeSourcePlan<S::Producer> {
        &self.time_source_plan
    }

    #[must_use]
    pub const fn tick_origin(&self) -> &S::TickOrigin {
        &self.tick_origin
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlacementParams<T>(T);

impl<T> PlacementParams<T> {
    #[must_use]
    pub const fn from_validated(value: T) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(&self) -> &T {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FailureParams<T>(T);

impl<T> FailureParams<T> {
    #[must_use]
    pub const fn from_validated(value: T) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(&self) -> &T {
        &self.0
    }
}

/// The five recorded components of a project authoring cut.
///
/// Each component comes from its owner. No default, path-derived project, or
/// revision recomputation is supplied by the store.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthoringCut<P, C, E, A, T> {
    pub project: P,
    pub cursor: C,
    pub environment: E,
    pub authoring_revision: A,
    pub topology_revision: T,
}

/// The initial authoring cut of this state directory’s stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RevisionStart<S: ManifestSchema> {
    Fresh(S::AuthoringCut),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevisionContext<S: ManifestSchema> {
    start: RevisionStart<S>,
    grants: Box<[(S::ScopeId, S::GrantSet)]>,
}

impl<S: ManifestSchema> RevisionContext<S> {
    pub fn try_new(
        start: RevisionStart<S>,
        grants: Vec<(S::ScopeId, S::GrantSet)>,
    ) -> Result<Self, RevisionContextError> {
        for (index, (scope, _)) in grants.iter().enumerate() {
            if grants[..index].iter().any(|(earlier, _)| earlier == scope) {
                return Err(RevisionContextError::DuplicateScope);
            }
        }
        Ok(Self {
            start,
            grants: grants.into_boxed_slice(),
        })
    }

    #[must_use]
    pub const fn start(&self) -> &RevisionStart<S> {
        &self.start
    }

    #[must_use]
    pub fn grants(&self) -> &[(S::ScopeId, S::GrantSet)] {
        &self.grants
    }
}

#[cfg(test)]
mod revision_context_tests {
    use super::{ManifestSchema, RevisionContext, RevisionContextError, RevisionStart};

    #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    struct Id(u64);

    impl circular_core::StreamIdentity for Id {}

    impl circular_core::ProducerIdentity for Id {
        type EventProducer = Self;
        fn from_event_producer(producer: Self) -> Self {
            producer
        }
    }

    #[derive(Clone, Debug, Eq, Hash, PartialEq)]
    struct TestSchema;

    macro_rules! unit_types {
        ($($name:ident),* $(,)?) => { $(type $name = u64;)* };
    }

    impl ManifestSchema for TestSchema {
        type Stream = Id;
        type Producer = Id;
        unit_types!(
            Placement,
            Failure,
            Versions,
            RevisionId,
            AuthoringCut,
            ScopeId,
            GrantSet,
            InputValue,
            CadencePolicy,
            TickOrigin,
        );
    }

    #[test]
    fn a_duplicate_scope_is_refused_wherever_it_sits() {
        let far_apart = RevisionContext::<TestSchema>::try_new(
            RevisionStart::Fresh(1),
            vec![(7, 70), (1, 10), (4, 40), (7, 71)],
        );
        assert_eq!(far_apart, Err(RevisionContextError::DuplicateScope));
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevisionContextError {
    DuplicateScope,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunInputs<V>(V);

impl<V> RunInputs<V> {
    #[must_use]
    pub const fn from_primary_data(value: V) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(&self) -> &V {
        &self.0
    }
}

pub fn encode_run_inputs<V>(
    inputs: &RunInputs<V>,
    encode_value: impl FnOnce(&V) -> Vec<u8>,
) -> EncodedPayload {
    EncodedPayload::new(PayloadVersionTag::FIRST, &encode_value(inputs.value()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestGroups<S: ManifestSchema> {
    time: TimeParams<S>,
    placement: PlacementParams<S::Placement>,
    failure: FailureParams<S::Failure>,
    versions: S::Versions,
    revision: RevisionContext<S>,
    inputs: RunInputs<S::InputValue>,
}

impl<S: ManifestSchema> ManifestGroups<S> {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn new(
        time: TimeParams<S>,
        placement: PlacementParams<S::Placement>,
        failure: FailureParams<S::Failure>,
        versions: S::Versions,
        revision: RevisionContext<S>,
        inputs: RunInputs<S::InputValue>,
    ) -> Self {
        Self {
            time,
            placement,
            failure,
            versions,
            revision,
            inputs,
        }
    }

    #[must_use]
    pub const fn time(&self) -> &TimeParams<S> {
        &self.time
    }

    #[must_use]
    pub const fn placement(&self) -> &PlacementParams<S::Placement> {
        &self.placement
    }

    #[must_use]
    pub const fn failure(&self) -> &FailureParams<S::Failure> {
        &self.failure
    }

    #[must_use]
    pub const fn versions(&self) -> &S::Versions {
        &self.versions
    }

    #[must_use]
    pub const fn revision(&self) -> &RevisionContext<S> {
        &self.revision
    }

    #[must_use]
    pub const fn inputs(&self) -> &RunInputs<S::InputValue> {
        &self.inputs
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunManifest<S: ManifestSchema> {
    stream: S::Stream,
    groups: ManifestGroups<S>,
}

impl<S: ManifestSchema> RunManifest<S> {
    #[must_use]
    pub const fn new(stream: S::Stream, groups: ManifestGroups<S>) -> Self {
        Self { stream, groups }
    }

    #[must_use]
    pub const fn stream(&self) -> &S::Stream {
        &self.stream
    }

    #[must_use]
    pub const fn groups(&self) -> &ManifestGroups<S> {
        &self.groups
    }

    #[must_use]
    pub fn into_parts(self) -> (S::Stream, ManifestGroups<S>) {
        (self.stream, self.groups)
    }
}
