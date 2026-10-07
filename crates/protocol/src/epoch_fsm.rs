
use crate::ScopeCoverage;
use circular_core::Value;
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenEpoch<S, C> {
    scope: S,
    candidate: C,
}

impl<S, C> OpenEpoch<S, C> {
    #[must_use]
    pub const fn new(scope: S, empty_candidate: C) -> Self {
        Self {
            scope,
            candidate: empty_candidate,
        }
    }

    #[must_use]
    pub const fn scope(&self) -> &S {
        &self.scope
    }

    #[must_use]
    pub const fn candidate(&self) -> &C {
        &self.candidate
    }

    pub fn validate<V, E>(&self, validate: impl FnOnce(&C) -> Result<V, E>) -> Result<V, E> {
        validate(&self.candidate)
    }

    pub fn update<T, E>(&mut self, apply: impl FnOnce(&C) -> Result<(C, T), E>) -> Result<T, E> {
        let (candidate, answer) = apply(&self.candidate)?;
        self.candidate = candidate;
        Ok(answer)
    }

    #[must_use]
    pub fn into_candidate(self) -> C {
        self.candidate
    }
}

impl<S: ScopeCoverage, C> OpenEpoch<S, C> {
    pub fn apply_content<E>(
        &mut self,
        target: &S,
        apply: impl FnOnce(&C) -> Result<C, E>,
    ) -> Result<(), EpochContentError<E>> {
        if !self.scope.covers(target) {
            return Err(EpochContentError::OutsideScope);
        }
        self.update(|candidate| apply(candidate).map(|next| (next, ())))
            .map_err(EpochContentError::Rejected)
    }
}

circular_core::closed_table! {
    pub enum BeginEpochError {
        DuplicateEpoch => "duplicate_epoch",
        ScopeConflict => "scope_conflict",
    }
}

#[must_use]
pub fn encode_begin_epoch_rejection(error: BeginEpochError) -> Value {
    Value::String(error.as_str().to_owned())
}

pub fn decode_begin_epoch_rejection(
    value: &Value,
) -> Result<BeginEpochError, BeginEpochErrorCodecError> {
    let Value::String(spelling) = value else {
        return Err(BeginEpochErrorCodecError::WrongCarrier);
    };
    BeginEpochError::from_str(spelling).ok_or(BeginEpochErrorCodecError::UnknownSpelling)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BeginEpochErrorCodecError {
    WrongCarrier,
    UnknownSpelling,
}

impl fmt::Display for BeginEpochErrorCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongCarrier => formatter.write_str("begin-epoch rejection is not a String"),
            Self::UnknownSpelling => formatter.write_str("unknown begin-epoch rejection spelling"),
        }
    }
}

impl Error for BeginEpochErrorCodecError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EpochContentError<E> {
    UnknownEpoch,
    OutsideScope,
    Rejected(E),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EpochTransitionError {
    UnknownEpoch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpochBook<I, S, C> {
    open: BTreeMap<I, OpenEpoch<S, C>>,
}

impl<I, S, C> EpochBook<I, S, C>
where
    I: Ord,
    S: ScopeCoverage,
{
    #[must_use]
    pub const fn new() -> Self {
        Self {
            open: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.open.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    pub fn begin(&mut self, id: I, scope: S, empty_candidate: C) -> Result<(), BeginEpochError> {
        if self.open.contains_key(&id) {
            return Err(BeginEpochError::DuplicateEpoch);
        }
        if self
            .open
            .values()
            .any(|epoch| epoch.scope.covers(&scope) || scope.covers(&epoch.scope))
        {
            return Err(BeginEpochError::ScopeConflict);
        }
        self.open.insert(id, OpenEpoch::new(scope, empty_candidate));
        Ok(())
    }

    #[must_use]
    pub fn get(&self, id: &I) -> Option<&OpenEpoch<S, C>> {
        self.open.get(id)
    }

    pub fn apply_content<E>(
        &mut self,
        id: &I,
        target: &S,
        apply: impl FnOnce(&C) -> Result<C, E>,
    ) -> Result<(), EpochContentError<E>> {
        self.open
            .get_mut(id)
            .ok_or(EpochContentError::UnknownEpoch)?
            .apply_content(target, apply)
    }

    pub fn validate<V, E>(
        &self,
        id: &I,
        validate: impl FnOnce(&C) -> Result<V, E>,
    ) -> Result<Result<V, E>, EpochTransitionError> {
        Ok(self
            .open
            .get(id)
            .ok_or(EpochTransitionError::UnknownEpoch)?
            .validate(validate))
    }

    pub fn abort(&mut self, id: &I) -> Result<OpenEpoch<S, C>, EpochTransitionError> {
        self.open
            .remove(id)
            .ok_or(EpochTransitionError::UnknownEpoch)
    }

    pub fn session_closed(&mut self) -> usize {
        let discarded = self.open.len();
        self.open.clear();
        discarded
    }
}

impl<I, S, C> Default for EpochBook<I, S, C>
where
    I: Ord,
    S: ScopeCoverage,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
    struct Scope(&'static [u8]);

    impl ScopeCoverage for Scope {
        fn covers(&self, target: &Self) -> bool {
            target.0.starts_with(self.0)
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Candidate {
        bytes: Vec<u8>,
    }

    impl Candidate {
        fn new(bytes: impl Into<Vec<u8>>) -> Self {
            Self {
                bytes: bytes.into(),
            }
        }
    }

    #[test]
    fn one_session_can_open_only_pairwise_disjoint_scopes() {
        let mut book = EpochBook::new();
        book.begin("left", Scope(&[1]), Candidate::new([]))
            .expect("first scope");
        book.begin("right", Scope(&[2]), Candidate::new([]))
            .expect("disjoint scope");
        assert_eq!(
            book.begin("child", Scope(&[1, 1]), Candidate::new([])),
            Err(BeginEpochError::ScopeConflict)
        );
        assert_eq!(book.len(), 2);
    }

    #[test]
    fn every_begin_rejection_round_trips_through_its_stable_spelling() {
        for error in BeginEpochError::ALL {
            assert_eq!(
                decode_begin_epoch_rejection(&encode_begin_epoch_rejection(error)),
                Ok(error)
            );
        }
        assert_eq!(
            decode_begin_epoch_rejection(&Value::String("other".to_owned())),
            Err(BeginEpochErrorCodecError::UnknownSpelling)
        );
        assert_eq!(
            decode_begin_epoch_rejection(&Value::Int(1)),
            Err(BeginEpochErrorCodecError::WrongCarrier)
        );
    }

    #[test]
    fn rejected_or_out_of_scope_content_keeps_the_candidate() {
        let mut book = EpochBook::new();
        book.begin("epoch", Scope(&[1]), Candidate::new([1_u8]))
            .expect("new epoch");
        book.apply_content(&"epoch", &Scope(&[1, 2]), |candidate| {
            let mut next = candidate.clone();
            next.bytes.push(2);
            Ok::<_, ()>(next)
        })
        .expect("names a descendant");
        assert_eq!(book.get(&"epoch").expect("open").candidate().bytes, [1, 2]);

        assert_eq!(
            book.apply_content(&"epoch", &Scope(&[1]), |_candidate| Err::<Candidate, _>(
                "bad"
            )),
            Err(EpochContentError::Rejected("bad"))
        );
        assert_eq!(
            book.apply_content(&"epoch", &Scope(&[3]), |candidate| Ok::<_, ()>(
                candidate.clone()
            )),
            Err(EpochContentError::OutsideScope)
        );
        assert_eq!(book.get(&"epoch").expect("open").candidate().bytes, [1, 2]);
    }

    #[test]
    fn validate_is_repeatable_and_does_not_close_the_epoch() {
        let mut book = EpochBook::new();
        book.begin("epoch", Scope(&[]), Candidate::new([1_u8, 2]))
            .expect("new epoch");
        let first = book
            .validate(&"epoch", |candidate| Ok::<_, ()>(candidate.bytes.len()))
            .expect("open epoch");
        let second = book
            .validate(&"epoch", |candidate| Ok::<_, ()>(candidate.bytes.len()))
            .expect("open epoch");
        assert_eq!(first, second);
        assert!(book.get(&"epoch").is_some());
    }

    #[test]
    fn abort_and_session_close_leave_no_candidate() {
        let mut book = EpochBook::new();
        book.begin("one", Scope(&[1]), Candidate::new([1_u8]))
            .expect("new epoch");
        book.begin("two", Scope(&[2]), Candidate::new([2_u8]))
            .expect("new epoch");
        let aborted = book.abort(&"one").expect("open epoch");
        assert_eq!(aborted.candidate(), &Candidate::new([1]));
        assert_eq!(book.session_closed(), 1);
        assert!(book.is_empty());
    }
}
