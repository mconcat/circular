
use crate::EffectOutcome;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DivergenceKind {
    KeyMiss,
    TermMismatch,
    ContentMiss,
}

use circular_core::ArrivalIndex;
use circular_plan::ActorId;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum LookupKey {
    Strict,
    Content,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoggedOutcome<I, T> {
    term: T,
    outcome: EffectOutcome<I>,
}

impl<I, T> LoggedOutcome<I, T> {
    #[must_use]
    pub const fn new(term: T, outcome: EffectOutcome<I>) -> Self {
        Self { term, outcome }
    }

    #[must_use]
    pub const fn term(&self) -> &T {
        &self.term
    }

    #[must_use]
    pub const fn outcome(&self) -> &EffectOutcome<I> {
        &self.outcome
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum LookupResult<'log, I> {
    Found {
        at: ArrivalIndex,
        outcome: &'log EffectOutcome<I>,
    },
    Diverged(DivergenceKind),
}

impl<I> Clone for LookupResult<'_, I> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<I> Copy for LookupResult<'_, I> {}

impl<'log, I> LookupResult<'log, I> {
    #[must_use]
    pub const fn is_found(&self) -> bool {
        matches!(self, Self::Found { .. })
    }

    #[must_use]
    pub const fn at(&self) -> Option<ArrivalIndex> {
        match self {
            Self::Found { at, .. } => Some(*at),
            Self::Diverged(_) => None,
        }
    }

    #[must_use]
    pub const fn outcome(&self) -> Option<&'log EffectOutcome<I>> {
        match self {
            Self::Found { outcome, .. } => Some(outcome),
            Self::Diverged(_) => None,
        }
    }

    #[must_use]
    pub const fn divergence(&self) -> Option<DivergenceKind> {
        match self {
            Self::Found { .. } => None,
            Self::Diverged(kind) => Some(*kind),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConsumedSet {
    refs: BTreeSet<ArrivalIndex>,
}

impl ConsumedSet {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            refs: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn contains(&self, at: ArrivalIndex) -> bool {
        self.refs.contains(&at)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.refs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.refs.is_empty()
    }

    pub fn refs(&self) -> impl Iterator<Item = ArrivalIndex> + '_ {
        self.refs.iter().copied()
    }

    fn mark(&mut self, at: ArrivalIndex) -> bool {
        self.refs.insert(at)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OutcomeLogBuildError<I> {
    DuplicateArrivalIndex(ArrivalIndex),
    DuplicateCorrelation(I),
}

impl<I: fmt::Debug> fmt::Display for OutcomeLogBuildError<I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateArrivalIndex(at) => {
                write!(formatter, "duplicate arrival ordinal {}", at.get())
            }
            Self::DuplicateCorrelation(key) => {
                write!(formatter, "correlation key {key:?} received two outcomes")
            }
        }
    }
}

impl<I: fmt::Debug> std::error::Error for OutcomeLogBuildError<I> {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutcomeLog<I, T> {
    actor: ActorId,
    records: BTreeMap<ArrivalIndex, LoggedOutcome<I, T>>,
    by_key: BTreeMap<I, ArrivalIndex>,
}

impl<I: Clone + Ord, T> OutcomeLog<I, T> {
    pub fn try_new(
        actor: ActorId,
        records: impl IntoIterator<Item = (ArrivalIndex, LoggedOutcome<I, T>)>,
    ) -> Result<Self, OutcomeLogBuildError<I>> {
        let mut by_index = BTreeMap::new();
        let mut by_key = BTreeMap::new();
        for (at, record) in records {
            let key = record.outcome.correlation().clone();
            if by_key.insert(key.clone(), at).is_some() {
                return Err(OutcomeLogBuildError::DuplicateCorrelation(key));
            }
            if by_index.insert(at, record).is_some() {
                return Err(OutcomeLogBuildError::DuplicateArrivalIndex(at));
            }
        }
        Ok(Self {
            actor,
            records: by_index,
            by_key,
        })
    }

    #[must_use]
    pub const fn actor(&self) -> &ActorId {
        &self.actor
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    #[must_use]
    pub fn get(&self, at: ArrivalIndex) -> Option<&LoggedOutcome<I, T>> {
        self.records.get(&at)
    }

    pub fn refs(&self) -> impl Iterator<Item = ArrivalIndex> + '_ {
        self.records.keys().copied()
    }

    pub fn records(&self) -> impl Iterator<Item = (ArrivalIndex, &LoggedOutcome<I, T>)> + '_ {
        self.records.iter().map(|(at, record)| (*at, record))
    }

    pub fn residual<'a>(
        &'a self,
        consumed: &'a ConsumedSet,
    ) -> impl Iterator<Item = (ArrivalIndex, &'a LoggedOutcome<I, T>)> + 'a {
        self.records()
            .filter(move |(at, _)| !consumed.contains(*at))
    }
}

impl<I: Ord, T: Eq> OutcomeLog<I, T> {
    pub fn strict(&self, key: &I, term: &T) -> LookupResult<'_, I> {
        let Some(at) = self.by_key.get(key).copied() else {
            return LookupResult::Diverged(DivergenceKind::KeyMiss);
        };
        let record = self.records.get(&at).expect(
            "the correlation key index and the sequence index come from the same record set",
        );
        if record.term == *term {
            LookupResult::Found {
                at,
                outcome: &record.outcome,
            }
        } else {
            LookupResult::Diverged(DivergenceKind::TermMismatch)
        }
    }

    pub fn content(&self, term: &T, consumed: &mut ConsumedSet) -> LookupResult<'_, I> {
        for (at, record) in &self.records {
            if consumed.contains(*at) || record.term != *term {
                continue;
            }
            consumed.mark(*at);
            return LookupResult::Found {
                at: *at,
                outcome: &record.outcome,
            };
        }
        LookupResult::Diverged(DivergenceKind::ContentMiss)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EffectFailure, OutcomePayload};
    use circular_plan::{Name, NamedActorId, ScopeId};

    fn actor(name: &str) -> ActorId {
        NamedActorId::new(ScopeId::root(), Name::from_normalized(name)).as_actor_id()
    }

    fn at(index: u64) -> ArrivalIndex {
        ArrivalIndex::new(index)
    }

    fn ok(correlation: u64, written: u64) -> EffectOutcome<u64> {
        EffectOutcome::new(correlation, Ok(OutcomePayload::WrittenLength(written)))
    }

    fn record(correlation: u64, term: &str, written: u64) -> LoggedOutcome<u64, String> {
        LoggedOutcome::new(term.to_owned(), ok(correlation, written))
    }

    fn log(
        entries: impl IntoIterator<Item = (u64, u64, &'static str, u64)>,
    ) -> OutcomeLog<u64, String> {
        OutcomeLog::try_new(
            actor("a"),
            entries
                .into_iter()
                .map(|(index, correlation, term, written)| {
                    (at(index), record(correlation, term, written))
                }),
        )
        .expect("unique sequence and unique correlation key")
    }

    #[test]
    fn strict_never_touches_the_consumed_set() {
        let log = log([(0, 1, "write", 5)]);
        let mut consumed = ConsumedSet::new();

        assert!(log.content(&"write".to_owned(), &mut consumed).is_found());
        assert_eq!(
            log.strict(&1, &"write".to_owned()).at(),
            Some(at(0)),
            "a strict lookup still yields an already consumed record; the key makes it unique"
        );
    }

    #[test]
    fn neither_key_splits_success_from_failure() {
        let failed = LoggedOutcome::new(
            "write".to_owned(),
            EffectOutcome::new(1_u64, Err(EffectFailure::EndpointGone)),
        );
        let log = OutcomeLog::try_new(actor("a"), [(at(0), failed)]).expect("one record");
        let mut consumed = ConsumedSet::new();

        assert!(
            log.strict(&1, &"write".to_owned()).is_found(),
            "the answer to a term whose original failed is also in the record, and the lookup yields it unchanged"
        );
        assert!(log.content(&"write".to_owned(), &mut consumed).is_found());
    }

    #[test]
    fn the_residual_is_ordered_by_arrival_index() {
        let log = log([(5, 3, "c", 0), (1, 1, "a", 0), (3, 2, "b", 0)]);
        let consumed = ConsumedSet::new();
        assert_eq!(
            log.residual(&consumed)
                .map(|(at, _)| at)
                .collect::<Vec<_>>(),
            vec![at(1), at(3), at(5)]
        );
    }
}
