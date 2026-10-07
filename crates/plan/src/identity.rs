
use circular_core::{ProducerIdentity, RecordProducerIdentity, StreamIdentity};
use std::fmt;

pub const MAX_SCOPE_DEPTH: usize = 64;

pub use circular_protocol::authored_value::Name;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Uuid([u8; 16]);

impl Uuid {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InstanceScalar {
    Text(Box<str>),
    Int(i64),
    Bool(bool),
}

impl InstanceScalar {
    #[must_use]
    pub fn normalized_text(value: impl Into<Box<str>>) -> Self {
        Self::Text(value.into())
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InstanceKey {
    Scalar(InstanceScalar),
    Tuple(Box<[InstanceScalar]>),
}

impl InstanceKey {
    fn display_key(&self) -> String {
        fn scalar(value: &InstanceScalar, output: &mut String) {
            match value {
                InstanceScalar::Text(text) => {
                    output.push('t');
                    output.push_str(&text.len().to_string());
                    output.push(':');
                    output.push_str(text);
                }
                InstanceScalar::Int(integer) => {
                    output.push('n');
                    output.push_str(&integer.to_string());
                    output.push(';');
                }
                InstanceScalar::Bool(value) => {
                    output.push('b');
                    output.push(if *value { '1' } else { '0' });
                    output.push(';');
                }
            }
        }

        let mut output = String::new();
        match self {
            Self::Scalar(value) => {
                output.push('s');
                scalar(value, &mut output);
            }
            Self::Tuple(values) => {
                output.push('u');
                output.push_str(&values.len().to_string());
                output.push(':');
                for value in values {
                    scalar(value, &mut output);
                }
            }
        }
        output
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ScopeSeg {
    Child(Name),
    Instance { of: Name, key: InstanceKey },
}

#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ScopeId(Box<[ScopeSeg]>);

impl ScopeId {
    #[must_use]
    pub fn root() -> Self {
        Self(Box::new([]))
    }

    pub fn from_segments(segments: impl Into<Box<[ScopeSeg]>>) -> Result<Self, ScopeIdError> {
        let segments = segments.into();
        if segments.len() > MAX_SCOPE_DEPTH {
            Err(ScopeIdError {
                attempted: segments.len(),
                limit: MAX_SCOPE_DEPTH,
            })
        } else {
            Ok(Self(segments))
        }
    }

    #[must_use]
    pub fn segments(&self) -> &[ScopeSeg] {
        &self.0
    }

    #[must_use]
    pub fn depth(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        let parent_len = self.0.len().checked_sub(1)?;
        Some(Self(self.0[..parent_len].to_vec().into_boxed_slice()))
    }

    #[must_use]
    pub fn is_ancestor_of(&self, other: &Self) -> bool {
        other.0.starts_with(&self.0)
    }

    pub fn append_segment(&self, segment: ScopeSeg) -> Result<Self, ScopeIdError> {
        let attempted = self.depth() + 1;
        if attempted > MAX_SCOPE_DEPTH {
            return Err(ScopeIdError {
                attempted,
                limit: MAX_SCOPE_DEPTH,
            });
        }
        let mut segments = self.0.to_vec();
        segments.push(segment);
        Ok(Self(segments.into_boxed_slice()))
    }
}

impl fmt::Display for ScopeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return formatter.write_str("/");
        }
        for segment in &self.0 {
            match segment {
                ScopeSeg::Child(name) => {
                    write!(formatter, "/c{}:{}", name.as_str().len(), name)?;
                }
                ScopeSeg::Instance { of, key } => {
                    let key = key.display_key();
                    write!(
                        formatter,
                        "/i{}:{}{}:{}",
                        of.as_str().len(),
                        of,
                        key.len(),
                        key
                    )?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScopeIdError {
    attempted: usize,
    limit: usize,
}

impl ScopeIdError {
    #[must_use]
    pub const fn attempted(self) -> usize {
        self.attempted
    }

    #[must_use]
    pub const fn limit(self) -> usize {
        self.limit
    }
}

impl fmt::Display for ScopeIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "scope depth {} exceeds limit {}",
            self.attempted, self.limit
        )
    }
}

impl std::error::Error for ScopeIdError {}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum LocalKey {
    Named(Name),
    Ephemeral(Uuid),
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SystemActor {
    Stream,
    Heartbeat,
    Pipeline,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ActorId {
    Scoped { scope: ScopeId, local: LocalKey },
    System(SystemActor),
}

impl ProducerIdentity for ActorId {
    type EventProducer = NamedActorId;

    fn from_event_producer(producer: Self::EventProducer) -> Self {
        producer.into()
    }
}

/// System records do not widen the event producer domain.
///
/// ```
/// use circular_core::{Hlc, Stamp, Tick, Sequence, RevisionEpochId};
/// use plan::{ActorId, SystemActor};
/// let stamp: Stamp<ActorId> = Stamp::from_record_producer_at(
///     Hlc::from_physical(Tick::ZERO), Sequence::new(0).unwrap(),
///     RevisionEpochId::new(1).unwrap(),
/// );
/// assert_eq!(stamp.producer(), &ActorId::System(SystemActor::Stream));
/// ```
///
/// ```compile_fail
/// use circular_core::{Stamp, Tick, Sequence, RevisionEpochId};
/// use plan::{ActorId, SystemActor};
/// let _: Stamp<ActorId> = Stamp::from_event_producer(
///     Tick::ZERO, ActorId::System(SystemActor::Stream),
///     Sequence::new(0).unwrap(), RevisionEpochId::new(1).unwrap(),
/// );
/// ```
impl RecordProducerIdentity for ActorId {
    fn is_record_producer(&self) -> bool {
        matches!(
            self,
            Self::System(SystemActor::Stream | SystemActor::Pipeline)
        )
    }

    fn record_producer() -> Self {
        Self::System(SystemActor::Stream)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ScopedActorId {
    scope: ScopeId,
    local: LocalKey,
}

impl ScopedActorId {
    #[must_use]
    pub fn new(scope: ScopeId, local: LocalKey) -> Self {
        Self { scope, local }
    }

    #[must_use]
    pub fn scope(&self) -> &ScopeId {
        &self.scope
    }

    #[must_use]
    pub fn local(&self) -> &LocalKey {
        &self.local
    }

    #[must_use]
    pub fn as_actor_id(&self) -> ActorId {
        ActorId::Scoped {
            scope: self.scope.clone(),
            local: self.local.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NamedActorId(ScopedActorId);

impl NamedActorId {
    #[must_use]
    pub fn new(scope: ScopeId, name: Name) -> Self {
        Self(ScopedActorId::new(scope, LocalKey::Named(name)))
    }

    #[must_use]
    pub fn scope(&self) -> &ScopeId {
        self.0.scope()
    }

    #[must_use]
    pub fn name(&self) -> &Name {
        match self.0.local() {
            LocalKey::Named(name) => name,
            LocalKey::Ephemeral(_) => unreachable!("the sealed invariant of NamedActorId"),
        }
    }

    #[must_use]
    pub fn as_scoped(&self) -> &ScopedActorId {
        &self.0
    }

    #[must_use]
    pub fn as_actor_id(&self) -> ActorId {
        self.0.as_actor_id()
    }
}

impl From<NamedActorId> for ScopedActorId {
    fn from(value: NamedActorId) -> Self {
        value.0
    }
}

impl From<NamedActorId> for ActorId {
    fn from(value: NamedActorId) -> Self {
        value.0.as_actor_id()
    }
}

impl From<ScopedActorId> for ActorId {
    fn from(value: ScopedActorId) -> Self {
        value.as_actor_id()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Generation(u64);

impl Generation {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenerationVector(Box<[Generation]>);

impl GenerationVector {
    pub fn for_actor(
        actor: &ScopedActorId,
        values: impl Into<Box<[Generation]>>,
    ) -> Result<Self, GenerationVectorError> {
        let values = values.into();
        let expected = actor.scope().depth() + 1;
        if values.len() == expected {
            Ok(Self(values))
        } else {
            Err(GenerationVectorError {
                expected,
                actual: values.len(),
            })
        }
    }

    #[must_use]
    pub fn as_slice(&self) -> &[Generation] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerationVectorError {
    expected: usize,
    actual: usize,
}

impl GenerationVectorError {
    #[must_use]
    pub const fn expected(self) -> usize {
        self.expected
    }

    #[must_use]
    pub const fn actual(self) -> usize {
        self.actual
    }
}

impl fmt::Display for GenerationVectorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "generation vector length must equal actor depth + 1 (expected: {}, received: {})",
            self.expected, self.actual
        )
    }
}

impl std::error::Error for GenerationVectorError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Incarnation<R: StreamIdentity> {
    stream: R,
    actor: ScopedActorId,
    generations: GenerationVector,
}

impl<R: StreamIdentity> Incarnation<R> {
    #[must_use]
    pub fn new(stream: R, actor: ScopedActorId, generations: GenerationVector) -> Self {
        debug_assert_eq!(generations.as_slice().len(), actor.scope().depth() + 1);
        Self {
            stream,
            actor,
            generations,
        }
    }

    #[must_use]
    pub const fn stream(&self) -> &R {
        &self.stream
    }

    #[must_use]
    pub const fn actor(&self) -> &ScopedActorId {
        &self.actor
    }

    #[must_use]
    pub fn generations(&self) -> &[Generation] {
        self.generations.as_slice()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    struct Run(u8);

    impl StreamIdentity for Run {}

    fn name(value: &str) -> Name {
        Name::from_normalized(value)
    }

    #[test]
    fn specified_total_order() {
        let root = ScopeId::root();
        let child = root.append_segment(ScopeSeg::Child(name("a"))).unwrap();
        let values = [
            ActorId::Scoped {
                scope: root,
                local: LocalKey::Named(name("z")),
            },
            ActorId::Scoped {
                scope: child,
                local: LocalKey::Named(name("a")),
            },
            ActorId::System(SystemActor::Stream),
            ActorId::System(SystemActor::Heartbeat),
        ];
        assert!(values.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(values.into_iter().collect::<BTreeSet<_>>().len(), 4);
    }
}
