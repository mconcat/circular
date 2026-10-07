
use circular_core::{Stamp, StreamIdentity, Tick};
use circular_plan::{ActorId, EdgeId, Generation, Incarnation};

/// The two disjoint occasions prescribed by runtime/effects §6.1.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum EffectOccasion {
    Delivery(Option<EdgeId>, Stamp<ActorId>),
    Poll(Tick, u64),
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EffectId {
    actor: ActorId,
    generations: Box<[Generation]>,
    occasion: EffectOccasion,
    index: u64,
}

impl EffectId {
    pub fn from_components(
        actor: circular_plan::ScopedActorId,
        generations: impl Into<Box<[Generation]>>,
        occasion: EffectOccasion,
        index: u64,
    ) -> Result<Self, circular_plan::GenerationVectorError> {
        let generations = circular_plan::GenerationVector::for_actor(&actor, generations)?;
        Ok(Self {
            actor: actor.as_actor_id(),
            generations: generations.as_slice().into(),
            occasion,
            index,
        })
    }

    /// Issue a key at the original position in the entire hook output column.
    /// The caller supplies the actual occasion, before filtering internal terms.
    #[must_use]
    pub fn at_hook<R: StreamIdentity>(
        incarnation: &Incarnation<R>,
        occasion: EffectOccasion,
        index: u64,
    ) -> Self {
        Self {
            actor: incarnation.actor().as_actor_id(),
            generations: incarnation.generations().into(),
            occasion,
            index,
        }
    }

    #[must_use]
    pub const fn actor(&self) -> &ActorId {
        &self.actor
    }
    #[must_use]
    pub fn generations(&self) -> &[Generation] {
        &self.generations
    }
    #[must_use]
    pub const fn occasion(&self) -> &EffectOccasion {
        &self.occasion
    }
    #[must_use]
    pub const fn index(&self) -> u64 {
        self.index
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DivergenceKind, EffectFailure, EffectOutcome, LoggedOutcome, OutcomeLog,
        OutcomeLogBuildError,
    };
    use circular_core::{ArrivalIndex, Hlc, LogicalCounter, RevisionEpochId, Sequence};
    use circular_plan::{GenerationVector, Name, NamedActorId, ScopeId};
    use std::collections::BTreeSet;

    #[derive(Clone, Debug, Eq, Hash, PartialEq)]
    struct Run(u64);
    impl StreamIdentity for Run {}

    fn named_actor(name: &str) -> NamedActorId {
        NamedActorId::new(ScopeId::root(), Name::from_normalized(name))
    }
    fn incarnation(run: u64, name: &str, generation: u64) -> Incarnation<Run> {
        let actor = named_actor(name);
        let generations = GenerationVector::for_actor(
            actor.as_scoped(),
            vec![Generation::new(generation)].into_boxed_slice(),
        )
        .unwrap();
        Incarnation::new(Run(run), actor.into(), generations)
    }

    #[test]
    fn hook_identity_preserves_four_literal_components_and_excludes_run() {
        let expected = EffectId {
            actor: named_actor("source").as_actor_id(),
            generations: vec![Generation::new(7)].into_boxed_slice(),
            occasion: EffectOccasion::Poll(Tick::new(11), 2),
            index: 3,
        };
        for run in [1, 901] {
            assert_eq!(
                EffectId::at_hook(
                    &incarnation(run, "source", 7),
                    EffectOccasion::Poll(Tick::new(11), 2),
                    3
                ),
                expected
            );
        }
    }

    #[test]
    fn independent_actor_columns_do_not_depend_on_submission_interleaving() {
        let a = incarnation(1, "a", 0);
        let b = incarnation(1, "b", 0);
        for order in [[0, 1, 2, 3], [0, 2, 1, 3], [2, 3, 0, 1]] {
            let columns = [(&a, 1), (&a, 3), (&b, 1), (&b, 3)];
            let mut observed = BTreeSet::new();
            for i in order {
                let (actor, index) = columns[i];
                observed.insert(EffectId::at_hook(
                    actor,
                    EffectOccasion::Poll(Tick::new(5), 0),
                    index,
                ));
            }
            let expected = [("a", 1), ("a", 3), ("b", 1), ("b", 3)].map(|(name, index)| EffectId {
                actor: named_actor(name).as_actor_id(),
                generations: vec![Generation::new(0)].into_boxed_slice(),
                occasion: EffectOccasion::Poll(Tick::new(5), 0),
                index,
            });
            assert_eq!(observed, BTreeSet::from(expected));
            assert_eq!(observed.len(), 4);
        }
    }

    #[test]
    fn strict_lookup_uses_the_structural_key_and_the_term() {
        let actor = incarnation(19, "source", 2);
        let key = EffectId::at_hook(&actor, EffectOccasion::Poll(Tick::new(8), 0), 3);
        let record = LoggedOutcome::new(
            "recorded term",
            EffectOutcome::new(key.clone(), Err(EffectFailure::EndpointGone)),
        );
        let log = OutcomeLog::try_new(
            named_actor("source").as_actor_id(),
            [(ArrivalIndex::new(12), record.clone())],
        )
        .unwrap();
        let replay_key = EffectId::at_hook(
            &incarnation(20, "source", 2),
            EffectOccasion::Poll(Tick::new(8), 0),
            3,
        );
        assert_eq!(
            log.strict(&replay_key, &"recorded term")
                .at()
                .unwrap()
                .get(),
            12
        );
        assert_eq!(
            log.strict(&replay_key, &"different term").divergence(),
            Some(DivergenceKind::TermMismatch)
        );
        let missing = EffectId::at_hook(&actor, EffectOccasion::Poll(Tick::new(8), 0), 4);
        assert_eq!(
            log.strict(&missing, &"recorded term").divergence(),
            Some(DivergenceKind::KeyMiss)
        );
        assert!(
            matches!(OutcomeLog::try_new(named_actor("source").as_actor_id(), [
            (ArrivalIndex::new(12), record.clone()),
            (ArrivalIndex::new(13), record)]),
            Err(OutcomeLogBuildError::DuplicateCorrelation(duplicate)) if duplicate == key)
        );
    }
}

impl std::fmt::Display for EffectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
