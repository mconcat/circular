
use crate::RecordedInstant;
use crate::identity::{OperationIdentity, ProducerIdentity, StreamIdentity};
use crate::ordering::Stamp;
use std::fmt;

crate::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum BaseShape {
        Null => "null",
        Bool => "bool",
        Int => "int",
        Float => "float",
        String => "string",
        Bytes => "bytes",
        UInt => "uint",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Shape<N> {
    Any,
    Base(BaseShape),
    Object { fields: FieldMap<N>, open: bool },
    Array(Box<Shape<N>>),
    Var(N),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldMap<N>(Vec<(N, Shape<N>)>);

impl<N: Eq> FieldMap<N> {
    pub fn try_new(fields: Vec<(N, Shape<N>)>) -> Result<Self, FieldMapError> {
        for (index, (name, _)) in fields.iter().enumerate() {
            if fields[..index].iter().any(|(previous, _)| previous == name) {
                return Err(FieldMapError::DuplicateName);
            }
        }
        Ok(Self(fields))
    }
}

impl<N> FieldMap<N> {
    #[must_use]
    pub fn as_slice(&self) -> &[(N, Shape<N>)] {
        &self.0
    }

    fn values(&self) -> impl Iterator<Item = &Shape<N>> {
        self.0.iter().map(|(_, shape)| shape)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FieldMapError {
    DuplicateName,
}

impl fmt::Display for FieldMapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("field names within one Object shape must be unique")
    }
}

impl std::error::Error for FieldMapError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroundShape<N>(Shape<N>);

impl<N> GroundShape<N> {
    pub fn try_new(shape: Shape<N>) -> Result<Self, GroundShapeError> {
        if contains_variable(&shape) {
            Err(GroundShapeError::ContainsVariable)
        } else {
            Ok(Self(shape))
        }
    }

    #[must_use]
    pub const fn as_shape(&self) -> &Shape<N> {
        &self.0
    }
}

impl<N> TryFrom<Shape<N>> for GroundShape<N> {
    type Error = GroundShapeError;

    fn try_from(shape: Shape<N>) -> Result<Self, Self::Error> {
        Self::try_new(shape)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroundShapeError {
    ContainsVariable,
}

impl fmt::Display for GroundShapeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the shape of an accepted payload cannot keep an unsubstituted Var")
    }
}

impl std::error::Error for GroundShapeError {}

fn contains_variable<N>(shape: &Shape<N>) -> bool {
    let mut pending = vec![shape];
    while let Some(current) = pending.pop() {
        match current {
            Shape::Var(_) => return true,
            Shape::Array(item) => pending.push(item),
            Shape::Object { fields, .. } => pending.extend(fields.values()),
            Shape::Any | Shape::Base(_) => {}
        }
    }
    false
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Payload<N, V> {
    shape: GroundShape<N>,
    value: std::sync::Arc<V>,
}

impl<N, V> Payload<N, V> {
    #[must_use]
    pub fn new(shape: GroundShape<N>, value: V) -> Self {
        Self {
            shape,
            value: std::sync::Arc::new(value),
        }
    }

    #[must_use]
    pub const fn shape(&self) -> &GroundShape<N> {
        &self.shape
    }

    #[must_use]
    pub fn value(&self) -> &V {
        &self.value
    }
}

mod sealed {
    pub trait EventPayload {}
}

pub trait EventPayload: sealed::EventPayload {}

impl<N, V> sealed::EventPayload for Payload<N, V> {}
impl<N, V> EventPayload for Payload<N, V> {}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PayloadVersionTag(u16);

impl PayloadVersionTag {
    pub const FIRST: Self = Self(1);

    pub const fn new(value: u16) -> Result<Self, PayloadVersionTagError> {
        if value == 0 {
            Err(PayloadVersionTagError::Reserved)
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadVersionTagError {
    Reserved,
}

impl fmt::Display for PayloadVersionTagError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("0 is not a value of any PayloadVersionTag entry")
    }
}

impl std::error::Error for PayloadVersionTagError {}

#[derive(Clone)]
pub struct EncodedPayload(std::sync::Arc<[u8]>, std::ops::Range<usize>);
impl fmt::Debug for EncodedPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("EncodedPayload")
            .field(&self.as_bytes())
            .finish()
    }
}
impl PartialEq for EncodedPayload {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}
impl Eq for EncodedPayload {}
impl std::hash::Hash for EncodedPayload {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(self.as_bytes(), state);
    }
}
impl EncodedPayload {
    #[must_use]
    pub fn new(tag: PayloadVersionTag, body: &[u8]) -> Self {
        let mut bytes = Vec::with_capacity(2 + body.len());
        bytes.extend_from_slice(&tag.get().to_be_bytes());
        bytes.extend_from_slice(body);
        let len = bytes.len();
        Self(bytes.into(), 0..len)
    }
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0[self.1.clone()]
    }
    #[must_use]
    pub fn version_tag(&self) -> PayloadVersionTag {
        let bytes = self.as_bytes();
        PayloadVersionTag(u16::from_be_bytes([bytes[0], bytes[1]]))
    }
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.as_bytes()[2..]
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PayloadVersionTagError> {
        let (Some(high), Some(low)) = (bytes.first(), bytes.get(1)) else {
            return Err(PayloadVersionTagError::Reserved);
        };
        PayloadVersionTag::new(u16::from_be_bytes([*high, *low]))?;
        Ok(Self(bytes.into(), 0..bytes.len()))
    }
    /// Retain an already-parsed, independently tagged field of this payload.
    /// The slice must borrow this very allocation; equal foreign bytes do not
    /// establish ownership. Identity and hashing remain exactly the visible bytes.
    pub fn shared_subpayload(&self, field: &[u8]) -> Option<Self> {
        let relative = (field.as_ptr() as usize).checked_sub(self.as_bytes().as_ptr() as usize)?;
        let end = relative.checked_add(field.len())?;
        if end > self.as_bytes().len() || field.len() < 2 {
            return None;
        }
        PayloadVersionTag::new(u16::from_be_bytes([field[0], field[1]])).ok()?;
        Some(Self(
            self.0.clone(),
            self.1.start + relative..self.1.start + end,
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventId<R: StreamIdentity, P: ProducerIdentity> {
    stream: R,
    stamp: Stamp<P>,
}

impl<R: StreamIdentity, P: ProducerIdentity> EventId<R, P> {
    #[must_use]
    pub const fn derive(stream: R, stamp: Stamp<P>) -> Self {
        Self { stream, stamp }
    }

    #[must_use]
    pub const fn stream(&self) -> &R {
        &self.stream
    }

    #[must_use]
    pub const fn stamp(&self) -> &Stamp<P> {
        &self.stamp
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CausalParents<R: StreamIdentity, P: ProducerIdentity>(Vec<EventId<R, P>>);

impl<R: StreamIdentity, P: ProducerIdentity> CausalParents<R, P> {
    pub fn try_new(parents: Vec<EventId<R, P>>) -> Result<Self, CausalParentsError<P>> {
        for pair in parents.windows(2) {
            if pair[0].stream != pair[1].stream {
                return Err(CausalParentsError::MixedStreams);
            }
            if !pair[0].stamp.timeline_cmp(&pair[1].stamp).is_lt() {
                return Err(CausalParentsError::NotStrictlyIncreasing {
                    previous: pair[0].stamp.clone(),
                    next: pair[1].stamp.clone(),
                });
            }
        }
        Ok(Self(parents))
    }

    #[must_use]
    pub fn as_slice(&self) -> &[EventId<R, P>] {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CausalParentsError<P: ProducerIdentity> {
    MixedStreams,
    NotStrictlyIncreasing { previous: Stamp<P>, next: Stamp<P> },
}

impl<P: ProducerIdentity> fmt::Display for CausalParentsError<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MixedStreams => {
                formatter.write_str("one causal parent list must belong to one stream")
            }
            Self::NotStrictlyIncreasing { previous, next } => write!(
                formatter,
                "causal parents must be in ascending stamp order without duplicates; violation: {previous:?} then {next:?}"
            ),
        }
    }
}

impl<P: ProducerIdentity> std::error::Error for CausalParentsError<P> {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Causality<R: StreamIdentity, P: ProducerIdentity> {
    Source,
    Derived(EventId<R, P>),
    Aggregated(CausalParents<R, P>),
}

impl<R: StreamIdentity, P: ProducerIdentity> Causality<R, P> {
    #[must_use]
    pub fn parents(&self) -> &[EventId<R, P>] {
        match self {
            Self::Source => &[],
            Self::Derived(parent) => std::slice::from_ref(parent),
            Self::Aggregated(parents) => parents.as_slice(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmissionDraft<D: EventPayload> {
    payload: D,
}

#[must_use]
pub const fn emit<D: EventPayload>(payload: D) -> EmissionDraft<D> {
    EmissionDraft { payload }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Emission<R: StreamIdentity, P: ProducerIdentity, D: EventPayload, O: OperationIdentity> {
    causality: Causality<R, P>,
    payload: D,
    operation: Option<O>,
}

impl<R: StreamIdentity, P: ProducerIdentity, D: EventPayload, O: OperationIdentity>
    Emission<R, P, D, O>
{
    #[must_use]
    pub fn from_runtime(
        draft: EmissionDraft<D>,
        causality: Causality<R, P>,
        operation: Option<O>,
    ) -> Self {
        Self {
            causality,
            payload: draft.payload,
            operation,
        }
    }

    #[must_use]
    pub const fn causality(&self) -> &Causality<R, P> {
        &self.causality
    }

    #[must_use]
    pub const fn payload(&self) -> &D {
        &self.payload
    }

    #[must_use]
    pub const fn operation(&self) -> Option<&O> {
        self.operation.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Event<R: StreamIdentity, P: ProducerIdentity, D: EventPayload, O: OperationIdentity> {
    stream: R,
    stamp: Stamp<P>,
    causality: Causality<R, P>,
    payload: D,
    operation: Option<O>,
    recorded_instant: Option<RecordedInstant>,
}

impl<R: StreamIdentity, P: ProducerIdentity, D: EventPayload, O: OperationIdentity>
    Event<R, P, D, O>
{
    /// Time from this recipient's arrival record; never sampled while consuming.
    #[must_use]
    pub const fn recorded_instant(&self) -> Option<RecordedInstant> {
        self.recorded_instant
    }

    /// Attach existing arrival metadata without changing the producer stamp.
    #[must_use]
    pub fn with_recorded_instant(mut self, instant: Option<RecordedInstant>) -> Self {
        self.recorded_instant = instant;
        self
    }

    #[must_use]
    pub fn id(&self) -> EventId<R, P> {
        EventId::derive(self.stream.clone(), self.stamp.clone())
    }

    #[must_use]
    pub const fn stamp(&self) -> &Stamp<P> {
        &self.stamp
    }

    #[must_use]
    pub const fn causality(&self) -> &Causality<R, P> {
        &self.causality
    }

    #[must_use]
    pub const fn payload(&self) -> &D {
        &self.payload
    }

    #[must_use]
    pub const fn operation(&self) -> Option<&O> {
        self.operation.as_ref()
    }
}

pub fn admit<R: StreamIdentity, P: ProducerIdentity, D: EventPayload, O: OperationIdentity>(
    stream: R,
    emission: Emission<R, P, D, O>,
    stamp: Stamp<P>,
) -> Result<Event<R, P, D, O>, AdmissionError<P>> {
    for parent in emission.causality.parents() {
        if parent.stream != stream {
            return Err(AdmissionError::CrossStreamParent);
        }
        if !parent.stamp.timeline_cmp(&stamp).is_lt() {
            return Err(AdmissionError::ParentNotBeforeChild {
                parent: parent.stamp.clone(),
                child: stamp.clone(),
            });
        }
    }

    Ok(Event {
        stream,
        stamp,
        causality: emission.causality,
        payload: emission.payload,
        operation: emission.operation,
        recorded_instant: None,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdmissionError<P: ProducerIdentity> {
    CrossStreamParent,
    ParentNotBeforeChild { parent: Stamp<P>, child: Stamp<P> },
}

impl<P: ProducerIdentity> fmt::Display for AdmissionError<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CrossStreamParent => {
                formatter.write_str("a causal parent must be in the same stream as its child")
            }
            Self::ParentNotBeforeChild { parent, child } => {
                write!(
                    formatter,
                    "a causal parent's stamp must be strictly less than its child's; parent {parent:?}, child {child:?}"
                )
            }
        }
    }
}

impl<P: ProducerIdentity> std::error::Error for AdmissionError<P> {}

#[cfg(test)]
mod encoded_payload_custody_tests {
    use super::*;
    #[test]
    fn immutable_payload_clone_shares_storage_and_preserves_existing_bytes() {
        let payload = EncodedPayload::new(PayloadVersionTag::FIRST, &[7, 8, 9]);
        let retained = payload.clone();
        assert_eq!(retained.as_bytes(), &[0, 1, 7, 8, 9]);
        assert_eq!(retained.as_bytes().as_ptr(), payload.as_bytes().as_ptr());
        drop(payload);
        assert_eq!(retained.body(), &[7, 8, 9]);
    }
}

#[cfg(test)]
mod shared_payload_tests {
    use super::*;
    use std::hash::{Hash, Hasher};

    #[test]
    fn a_shared_tagged_field_keeps_literal_bytes_identity_and_ownership() {
        let outer = EncodedPayload::new(PayloadVersionTag::FIRST, &[0, 2, 0x61, 0x62, 0, 3, 0x7a]);
        let field = outer.shared_subpayload(&outer.body()[..4]).unwrap();
        assert_eq!(field.as_bytes(), &[0, 2, 0x61, 0x62]);
        assert_eq!(field.version_tag().get(), 2);
        assert_eq!(field.body(), b"ab");
        assert_eq!(field.as_bytes().as_ptr(), outer.body().as_ptr());
        let independent = EncodedPayload::from_bytes(&[0, 2, 0x61, 0x62]).unwrap();
        assert_eq!(field, independent);
        let hash = |value: &EncodedPayload| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            value.hash(&mut h);
            h.finish()
        };
        assert_eq!(hash(&field), hash(&independent));
        assert!(outer.shared_subpayload(independent.as_bytes()).is_none());
        assert!(outer.shared_subpayload(&outer.body()[..1]).is_none());
        drop(outer);
        assert_eq!(field.body(), b"ab");
    }
}
