
use circular_core::{
    AdmissionError, ArrivalIndex, Emission, Event, EventPayload, OperationIdentity,
    ProducerIdentity, RecordedInstant, Stamp, StreamIdentity, admit as admit_event,
};
use circular_plan::{ActorId, EdgeId};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ArrivalOrigin<P, C, T>
where
    P: ProducerIdentity,
{
    EdgeDelivery { edge: EdgeId, stamp: Stamp<P> },
    TimerFire { timer: T },
    EffectOutcome { correlation: C },
    ExternalInject { origin: ExternalOrigin },
}

impl<P, C, T> ArrivalOrigin<P, C, T>
where
    P: ProducerIdentity,
{
    #[must_use]
    pub const fn producer_stamp(&self) -> Option<&Stamp<P>> {
        match self {
            Self::EdgeDelivery { stamp, .. } => Some(stamp),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ExternalOrigin(Box<[u8]>);

impl ExternalOrigin {
    pub fn try_new(bytes: impl Into<Box<[u8]>>) -> Result<Self, EmptyExternalOrigin> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            Err(EmptyExternalOrigin)
        } else {
            Ok(Self(bytes))
        }
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmptyExternalOrigin;

impl fmt::Display for EmptyExternalOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("external injection target cannot be empty")
    }
}

impl std::error::Error for EmptyExternalOrigin {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct LogCut {
    components: BTreeMap<ActorId, ArrivalIndex>,
}

impl LogCut {
    #[must_use]
    pub fn new(components: impl IntoIterator<Item = (ActorId, ArrivalIndex)>) -> Self {
        Self {
            components: components.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn empty() -> Self {
        Self {
            components: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn get(&self, actor: &ActorId) -> Option<ArrivalIndex> {
        self.components.get(actor).copied()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.components.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.components.is_empty()
    }

    pub fn components(&self) -> impl Iterator<Item = (&ActorId, ArrivalIndex)> {
        self.components.iter().map(|(actor, index)| (actor, *index))
    }

    #[must_use]
    pub fn precedes(&self, other: &Self) -> Option<bool> {
        if self.components.len() != other.components.len() {
            return None;
        }
        let mut all_le = true;
        for (actor, index) in &self.components {
            let their = other.components.get(actor)?;
            if index > their {
                all_le = false;
            }
        }
        Some(all_le)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IngressEdges {
    entries: BTreeSet<EdgeId>,
}

impl IngressEdges {
    #[must_use]
    pub fn new(entries: impl IntoIterator<Item = EdgeId>) -> Self {
        Self {
            entries: entries.into_iter().collect(),
        }
    }

    #[must_use]
    pub fn empty() -> Self {
        Self {
            entries: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn contains_edge(&self, edge: &EdgeId) -> bool {
        self.entries.contains(edge)
    }

    pub fn iter(&self) -> impl Iterator<Item = &EdgeId> {
        self.entries.iter()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArrivalSource<T> {
    Edge(EdgeId),
    Timer(T),
    External(ExternalOrigin),
}

#[derive(Clone, Debug)]
pub struct RecordStep<R, P, D, O, T>
where
    R: StreamIdentity,
    P: ProducerIdentity,
    D: EventPayload,
    O: OperationIdentity,
{
    index: ArrivalIndex,
    stamp: Stamp<P>,
    source: ArrivalSource<T>,
    recorded_instant: Option<RecordedInstant>,
    emission: Emission<R, P, D, O>,
}

impl<R, P, D, O, T> RecordStep<R, P, D, O, T>
where
    R: StreamIdentity,
    P: ProducerIdentity,
    D: EventPayload,
    O: OperationIdentity,
{
    #[must_use]
    pub const fn edge_delivery(
        index: ArrivalIndex,
        edge: EdgeId,
        stamp: Stamp<P>,
        emission: Emission<R, P, D, O>,
    ) -> Self {
        Self {
            index,
            stamp,
            recorded_instant: None,
            source: ArrivalSource::Edge(edge),
            emission,
        }
    }

    #[must_use]
    pub const fn external_inject(
        index: ArrivalIndex,
        origin: ExternalOrigin,
        stamp: Stamp<P>,
        emission: Emission<R, P, D, O>,
    ) -> Self {
        Self {
            index,
            stamp,
            recorded_instant: None,
            source: ArrivalSource::External(origin),
            emission,
        }
    }

    #[must_use]
    pub const fn timer_fire(
        index: ArrivalIndex,
        timer: T,
        stamp: Stamp<P>,
        emission: Emission<R, P, D, O>,
    ) -> Self {
        Self {
            index,
            stamp,
            recorded_instant: None,
            source: ArrivalSource::Timer(timer),
            emission,
        }
    }

    /// Carry the durable arrival time through the queue to ActorInput.
    #[must_use]
    pub fn with_recorded_instant(mut self, instant: Option<RecordedInstant>) -> Self {
        self.recorded_instant = instant;
        self
    }

    #[must_use]
    pub const fn source(&self) -> &ArrivalSource<T> {
        &self.source
    }

    #[must_use]
    pub const fn index(&self) -> ArrivalIndex {
        self.index
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConsumeStep;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndexRule {
    Next,
    AlreadyCounted,
}

#[derive(Debug)]
pub struct Checked<R, P, D, O, C, T>
where
    R: StreamIdentity,
    P: ProducerIdentity,
    D: EventPayload,
    O: OperationIdentity,
{
    arrival: RecordedArrival<R, P, D, O, C, T>,
    advance_to: Option<ArrivalIndex>,
}

impl<R, P, D, O, C, T> Checked<R, P, D, O, C, T>
where
    R: StreamIdentity,
    P: ProducerIdentity,
    D: EventPayload,
    O: OperationIdentity,
{
    #[must_use]
    pub const fn index(&self) -> ArrivalIndex {
        self.arrival.index
    }

    #[must_use]
    pub fn stamp(&self) -> &Stamp<P> {
        self.arrival.event.stamp()
    }
}

#[derive(Clone, Debug)]
pub enum ArrivalStep<R, P, D, O, T>
where
    R: StreamIdentity,
    P: ProducerIdentity,
    D: EventPayload,
    O: OperationIdentity,
{
    Record(RecordStep<R, P, D, O, T>),
    Consume(ConsumeStep),
}

#[derive(Clone, Debug)]
pub enum ArrivalStepOutput<R, P, D, O, C, T>
where
    R: StreamIdentity,
    P: ProducerIdentity,
    D: EventPayload,
    O: OperationIdentity,
{
    Recorded {
        index: ArrivalIndex,
        stamp: Stamp<P>,
    },
    Consumed {
        arrival: Option<RecordedArrival<R, P, D, O, C, T>>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordedArrival<R, P, D, O, C, T>
where
    R: StreamIdentity,
    P: ProducerIdentity,
    D: EventPayload,
    O: OperationIdentity,
{
    index: ArrivalIndex,
    origin: ArrivalOrigin<P, C, T>,
    event: Event<R, P, D, O>,
}

impl<R, P, D, O, C, T> RecordedArrival<R, P, D, O, C, T>
where
    R: StreamIdentity,
    P: ProducerIdentity,
    D: EventPayload,
    O: OperationIdentity,
{
    #[must_use]
    pub const fn index(&self) -> ArrivalIndex {
        self.index
    }

    #[must_use]
    pub const fn origin(&self) -> &ArrivalOrigin<P, C, T> {
        &self.origin
    }

    #[must_use]
    pub const fn edge(&self) -> Option<&EdgeId> {
        match &self.origin {
            ArrivalOrigin::EdgeDelivery { edge, .. } => Some(edge),
            ArrivalOrigin::TimerFire { .. }
            | ArrivalOrigin::EffectOutcome { .. }
            | ArrivalOrigin::ExternalInject { .. } => None,
        }
    }

    #[must_use]
    pub const fn event(&self) -> &Event<R, P, D, O> {
        &self.event
    }

    /// Move the recorded identity and event together across the consumption boundary.
    #[must_use]
    pub fn into_parts(self) -> (ArrivalIndex, ArrivalOrigin<P, C, T>, Event<R, P, D, O>) {
        (self.index, self.origin, self.event)
    }
}

#[derive(Clone, Debug)]
pub struct ActorArrivals<R, P, D, O, C, T>
where
    R: StreamIdentity,
    P: ProducerIdentity,
    D: EventPayload,
    O: OperationIdentity,
{
    run: R,
    ingress: IngressEdges,
    pending: VecDeque<RecordedArrival<R, P, D, O, C, T>>,
    next_index: ArrivalIndex,
    consumed_count: usize,
    last_consumed: Option<(ArrivalIndex, Stamp<P>)>,
}

impl<R, P, D, O, C, T> ActorArrivals<R, P, D, O, C, T>
where
    R: StreamIdentity,
    P: ProducerIdentity,
    D: EventPayload + Clone,
    O: OperationIdentity,
{
    #[must_use]
    pub fn new(run: R, ingress: IngressEdges) -> Self {
        Self::new_at(run, ingress, ArrivalIndex::FIRST)
    }

    #[must_use]
    pub fn new_at(run: R, ingress: IngressEdges, next_index: ArrivalIndex) -> Self {
        Self {
            run,
            ingress,
            pending: VecDeque::new(),
            next_index,
            consumed_count: 0,
            last_consumed: None,
        }
    }

    #[must_use]
    pub const fn stream(&self) -> &R {
        &self.run
    }

    #[must_use]
    pub const fn ingress(&self) -> &IngressEdges {
        &self.ingress
    }

    #[must_use]
    pub fn pending_arrivals(&self) -> usize {
        self.pending.len()
    }

    pub fn pending(&self) -> impl Iterator<Item = &RecordedArrival<R, P, D, O, C, T>> {
        self.pending.iter()
    }

    /// Prepare against a borrowed arrival, then consume that same item only on
    /// success. A failed preparation preserves the queue and consumption cursor.
    /// Timer-only selection is the existing pause rule, not a different queue.
    #[allow(clippy::type_complexity)]
    pub fn try_consume<A, E>(
        &mut self,
        timer_only: bool,
        prepare: impl FnOnce(&RecordedArrival<R, P, D, O, C, T>) -> Result<A, E>,
    ) -> Result<Option<(A, RecordedArrival<R, P, D, O, C, T>)>, E> {
        let position = if timer_only {
            self.pending
                .iter()
                .position(|arrival| matches!(arrival.origin, ArrivalOrigin::TimerFire { .. }))
        } else {
            (!self.pending.is_empty()).then_some(0)
        };
        let Some(position) = position else {
            return Ok(None);
        };
        let prepared = prepare(&self.pending[position])?;
        let arrival = self
            .pending
            .remove(position)
            .expect("prepared arrival is still queued");
        self.consumed_count += 1;
        self.last_consumed = Some((arrival.index, arrival.event.stamp().clone()));
        Ok(Some((prepared, arrival)))
    }

    pub fn consume_reserved_timer(mut self) -> (Self, Option<RecordedArrival<R, P, D, O, C, T>>) {
        let Ok(arrival) = self.try_consume(true, |_| Ok::<_, std::convert::Infallible>(()));
        let arrival = arrival.map(|((), arrival)| arrival);
        (self, arrival)
    }

    #[must_use]
    pub const fn consumed_count(&self) -> usize {
        self.consumed_count
    }

    #[must_use]
    pub const fn last_consumed(&self) -> Option<&(ArrivalIndex, Stamp<P>)> {
        self.last_consumed.as_ref()
    }

    #[must_use]
    pub const fn horizon(&self) -> ArrivalIndex {
        self.next_index
    }

    pub fn advance_committed_index(
        &mut self,
        index: ArrivalIndex,
    ) -> Result<(), ArrivalStepError<P>> {
        if index != self.next_index {
            return Err(ArrivalStepError::NonContiguousIndex {
                expected: self.next_index,
                actual: index,
            });
        }
        self.next_index = index.next().ok_or(ArrivalStepError::IndexExhausted)?;
        Ok(())
    }

    #[allow(clippy::type_complexity)]
    pub fn check(
        &self,
        step: RecordStep<R, P, D, O, T>,
        rule: IndexRule,
    ) -> Result<Checked<R, P, D, O, C, T>, ArrivalStepError<P>> {
        let index = step.index;
        let advance_to = match rule {
            IndexRule::Next => {
                if index != self.next_index {
                    return Err(ArrivalStepError::NonContiguousIndex {
                        expected: self.next_index,
                        actual: index,
                    });
                }
                Some(index.next().ok_or(ArrivalStepError::IndexExhausted)?)
            }
            IndexRule::AlreadyCounted => None,
        };
        if let ArrivalSource::Edge(edge) = &step.source
            && !self.ingress.contains_edge(edge)
        {
            return Err(ArrivalStepError::UnknownEdge(edge.clone()));
        }
        let stamp = step.stamp;
        let event = admit_event(self.run.clone(), step.emission, stamp.clone())
            .map_err(ArrivalStepError::Admission)?
            .with_recorded_instant(step.recorded_instant);
        let origin = match step.source {
            ArrivalSource::Edge(edge) => ArrivalOrigin::EdgeDelivery { edge, stamp },
            ArrivalSource::Timer(timer) => ArrivalOrigin::TimerFire { timer },
            ArrivalSource::External(origin) => ArrivalOrigin::ExternalInject { origin },
        };
        Ok(Checked {
            arrival: RecordedArrival {
                index,
                origin,
                event,
            },
            advance_to,
        })
    }

    pub fn commit(&mut self, checked: Checked<R, P, D, O, C, T>) -> ArrivalIndex {
        let index = checked.arrival.index;
        if let Some(next) = checked.advance_to {
            self.next_index = next;
        }
        self.pending.push_back(checked.arrival);
        index
    }

    #[allow(clippy::type_complexity)]
    pub fn fold(
        mut self,
        step: ArrivalStep<R, P, D, O, T>,
    ) -> Result<(Self, ArrivalStepOutput<R, P, D, O, C, T>), ArrivalStepError<P>> {
        match step {
            ArrivalStep::Record(step) => {
                let checked = self.check(step, IndexRule::Next)?;
                let stamp = checked.stamp().clone();
                let index = self.commit(checked);
                Ok((self, ArrivalStepOutput::Recorded { index, stamp }))
            }
            ArrivalStep::Consume(_) => {
                let Ok(arrival) =
                    self.try_consume(false, |_| Ok::<_, std::convert::Infallible>(()));
                Ok((
                    self,
                    ArrivalStepOutput::Consumed {
                        arrival: arrival.map(|((), arrival)| arrival),
                    },
                ))
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArrivalStepError<P: ProducerIdentity> {
    UnknownEdge(EdgeId),
    Admission(AdmissionError<P>),
    IndexExhausted,
    NonContiguousIndex {
        expected: ArrivalIndex,
        actual: ArrivalIndex,
    },
}

impl<P: ProducerIdentity> fmt::Display for ArrivalStepError<P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownEdge(edge) => {
                write!(formatter, "not an ingress edge of this actor: {edge:?}")
            }
            Self::Admission(error) => write!(formatter, "event admission failed: {error}"),
            Self::IndexExhausted => formatter.write_str("actor arrival ordinal space exhausted"),
            Self::NonContiguousIndex { expected, actual } => write!(
                formatter,
                "durable arrival ordinal differs from the actor horizon: expected={}, actual={}",
                expected.get(),
                actual.get()
            ),
        }
    }
}

impl<P: ProducerIdentity> std::error::Error for ArrivalStepError<P> {}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::{BaseShape, GroundShape, Payload, Shape};
    use circular_plan::{ActorId, Name, NamedActorId, ScopeId};

    type TestPayload = Payload<Name, u64>;

    fn actor(name: &str) -> ActorId {
        NamedActorId::new(ScopeId::root(), Name::from_normalized(name)).as_actor_id()
    }

    fn payload(value: u64) -> TestPayload {
        Payload::new(
            GroundShape::try_new(Shape::Base(BaseShape::Int)).expect("Int is a ground shape"),
            value,
        )
    }

    fn timer(id: u64) -> ArrivalOrigin<ActorId, u64, u64> {
        ArrivalOrigin::TimerFire { timer: id }
    }

    #[test]
    fn only_edge_delivery_carries_a_producer_stamp() {
        let fire: ArrivalOrigin<ActorId, u64, u64> = timer(1);
        assert!(fire.producer_stamp().is_none());

        let outcome: ArrivalOrigin<ActorId, u64, u64> =
            ArrivalOrigin::EffectOutcome { correlation: 7 };
        assert!(outcome.producer_stamp().is_none());

        let injected: ArrivalOrigin<ActorId, u64, u64> = ArrivalOrigin::ExternalInject {
            origin: ExternalOrigin::try_new(vec![1u8]).expect("origin"),
        };
        assert!(injected.producer_stamp().is_none());
    }

    #[test]
    fn external_origin_rejects_empty_designation() {
        assert_eq!(
            ExternalOrigin::try_new(Vec::new()),
            Err(EmptyExternalOrigin)
        );
    }

    #[test]
    fn log_cut_compares_component_wise() {
        let early = LogCut::new([
            (actor("a"), ArrivalIndex::new(1)),
            (actor("b"), ArrivalIndex::new(3)),
        ]);
        let late = LogCut::new([
            (actor("a"), ArrivalIndex::new(2)),
            (actor("b"), ArrivalIndex::new(5)),
        ]);
        assert_eq!(early.precedes(&late), Some(true));
        assert_eq!(late.precedes(&early), Some(false));

        let crossed = LogCut::new([
            (actor("a"), ArrivalIndex::new(4)),
            (actor("b"), ArrivalIndex::new(0)),
        ]);
        assert_eq!(early.precedes(&crossed), Some(false));
        assert_eq!(crossed.precedes(&early), Some(false));

        let narrower = LogCut::new([(actor("a"), ArrivalIndex::new(1))]);
        assert_eq!(
            early.precedes(&narrower),
            None,
            "with different actor sets, the comparison is undefined"
        );
    }

    #[test]
    fn absent_component_is_not_read_as_first() {
        let cut = LogCut::new([(actor("a"), ArrivalIndex::new(2))]);
        assert_eq!(cut.get(&actor("a")), Some(ArrivalIndex::new(2)));
        assert_eq!(cut.get(&actor("b")), None);
    }

    #[test]
    fn reversed_instants_do_not_silently_fold_to_zero() {
        let later = RecordedInstant::from_millis(10);
        let earlier = RecordedInstant::from_millis(40);
        assert_eq!(later.interval_since(earlier), None);
    }
}
