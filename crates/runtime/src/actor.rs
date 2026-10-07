
use crate::{ActorEffects, EditableActor, EffectOutcome, Effects};
use circular_core::{ArrivalIndex, RecordedInstant, Stamp, StreamIdentity, Tick};
use circular_plan::{ActorId, ActorType, Config, EdgeId, Incarnation, PortId};

pub trait ActorTypes {
    type Stream: StreamIdentity;
    type Event;
    type Payload;
    type EffectId: Clone + Ord;
    type StateVersion: Clone;
    type Observation;
    type Grants: ?Sized;

    fn payload(event: &Self::Event) -> &Self::Payload;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EnvelopeResult {
    Ok,
    Err {
        reason: crate::DeadLetterReason,
        failure_point: Option<crate::PreprocessFailurePoint>,
    },
}

/// Recorded receiving coordinates are present together, or the input is a
/// detached authoring/probe value. A source arrival legitimately has no edge.
#[derive(Clone, Debug, Eq, PartialEq)]
struct InputReceipt {
    index: ArrivalIndex,
    route_edge: Option<EdgeId>,
    origin: crate::ArrivalOrigin<ActorId, std::convert::Infallible, crate::EffectId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActorInput<E> {
    inlet: PortId,
    event: E,
    result: EnvelopeResult,
    receipt: Option<InputReceipt>,
}

impl<E> ActorInput<E> {
    #[must_use]
    pub const fn new(inlet: PortId, event: E) -> Self {
        Self {
            inlet,
            event,
            result: EnvelopeResult::Ok,
            receipt: None,
        }
    }

    /// Attach the recorded receiving coordinates; detached inputs have none.
    #[must_use]
    pub fn with_arrival(
        mut self,
        index: ArrivalIndex,
        route_edge: Option<EdgeId>,
        origin: crate::ArrivalOrigin<ActorId, std::convert::Infallible, crate::EffectId>,
    ) -> Self {
        self.receipt = Some(InputReceipt {
            index,
            route_edge,
            origin,
        });
        self
    }

    #[must_use]
    pub const fn arrival_index(&self) -> Option<ArrivalIndex> {
        match &self.receipt {
            Some(receipt) => Some(receipt.index),
            None => None,
        }
    }

    #[must_use]
    pub fn route_edge(&self) -> Option<&EdgeId> {
        self.receipt
            .as_ref()
            .and_then(|receipt| receipt.route_edge.as_ref())
    }

    #[must_use]
    pub fn origin(
        &self,
    ) -> Option<&crate::ArrivalOrigin<ActorId, std::convert::Infallible, crate::EffectId>> {
        self.receipt.as_ref().map(|receipt| &receipt.origin)
    }

    #[must_use]
    pub fn with_result(mut self, result: EnvelopeResult) -> Self {
        self.result = result;
        self
    }

    #[must_use]
    pub const fn result(&self) -> &EnvelopeResult {
        &self.result
    }

    #[must_use]
    pub const fn inlet(&self) -> &PortId {
        &self.inlet
    }

    #[must_use]
    pub const fn event(&self) -> &E {
        &self.event
    }

    #[must_use]
    pub fn payload<T: ActorTypes<Event = E>>(&self) -> &T::Payload {
        T::payload(&self.event)
    }

    #[must_use]
    pub fn into_parts(self) -> (PortId, E) {
        (self.inlet, self.event)
    }
}

/// Coordinates supplied by durable admission for one outcome receipt.
struct ContextReceipt {
    index: ArrivalIndex,
    stamp: Stamp<ActorId>,
    instant: RecordedInstant,
}

pub struct ActorContext<'call, R: StreamIdentity, G: ?Sized> {
    id: &'call ActorId,
    incarnation: &'call Incarnation<R>,
    config: &'call Config,
    grants: &'call G,
    receipt: Option<ContextReceipt>,
}

impl<'call, R: StreamIdentity, G: ?Sized> ActorContext<'call, R, G> {
    #[must_use]
    pub const fn new(
        id: &'call ActorId,
        incarnation: &'call Incarnation<R>,
        config: &'call Config,
        grants: &'call G,
    ) -> Self {
        Self {
            id,
            incarnation,
            config,
            grants,
            receipt: None,
        }
    }

    #[must_use]
    pub fn with_arrival(
        mut self,
        index: ArrivalIndex,
        stamp: Stamp<ActorId>,
        instant: RecordedInstant,
    ) -> Self {
        self.receipt = Some(ContextReceipt {
            index,
            stamp,
            instant,
        });
        self
    }

    #[must_use]
    pub const fn arrival_index(&self) -> Option<ArrivalIndex> {
        match &self.receipt {
            Some(receipt) => Some(receipt.index),
            None => None,
        }
    }

    #[must_use]
    pub const fn recorded_instant(&self) -> Option<RecordedInstant> {
        match &self.receipt {
            Some(receipt) => Some(receipt.instant),
            None => None,
        }
    }

    #[must_use]
    pub const fn id(&self) -> &ActorId {
        self.id
    }

    #[must_use]
    pub const fn incarnation(&self) -> &Incarnation<R> {
        self.incarnation
    }

    #[must_use]
    pub const fn config(&self) -> &Config {
        self.config
    }

    #[must_use]
    pub const fn grants(&self) -> &G {
        self.grants
    }
}

pub trait Actor<T: ActorTypes>:
    EditableActor<StateVersion = T::StateVersion, EffectId = T::EffectId>
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> Effects;

    fn on_outcome(
        &mut self,
        outcome: &EffectOutcome<T::EffectId>,
        context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> Effects;

    fn observe(&self) -> Option<T::Observation> {
        None
    }
}

pub trait EmittingActor<T: ActorTypes, P>:
    EditableActor<StateVersion = T::StateVersion, EffectId = T::EffectId>
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<P>;

    fn on_outcome(
        &mut self,
        _outcome: &EffectOutcome<T::EffectId>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<P> {
        ActorEffects::empty()
    }

    fn on_lifecycle(
        &mut self,
        _life: ActorLifecycle,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<P> {
        ActorEffects::empty()
    }

    fn accepts(&self, _inlet: &PortId) -> bool {
        true
    }

    fn observe(&self) -> Option<T::Observation> {
        None
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActorLifecycle {
    Opened,
    Closing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WakeCondition<I> {
    Tick(Tick),
    Outcome(I),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourcePoll<I> {
    Ready(Effects),
    Pending { wake: WakeCondition<I> },
    Exhausted,
}

pub trait SourceActor<T: ActorTypes>: Actor<T> {
    fn poll(&mut self, context: &ActorContext<'_, T::Stream, T::Grants>)
    -> SourcePoll<T::EffectId>;
}

pub trait ActorFactory {
    const TYPE: ActorType;
    type Grants: ?Sized;
    type Types: ActorTypes<Grants = Self::Grants>;
    type Instance: Actor<Self::Types>;
    type Error;

    fn create(config: &Config, grants: &Self::Grants) -> Result<Self::Instance, Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditDisposition {
    Absorbs,
    AbsorbsSome,
    Restarts,
}

pub trait EmittingActorFactory<P> {
    const TYPE: ActorType;
    const DISPOSITION: EditDisposition;
    const CHECKPOINTS: bool;
    type Grants: ?Sized;
    type Types: ActorTypes<Grants = Self::Grants>;
    type Instance: EmittingActor<Self::Types, P>;
    type Error;

    fn create(
        config: &crate::FoldedConfig,
        grants: &Self::Grants,
    ) -> Result<Self::Instance, Self::Error>;
}

pub trait SourceFactory {
    const TYPE: ActorType;
    type Grants: ?Sized;
    type Types: ActorTypes<Grants = Self::Grants>;
    type Instance: SourceActor<Self::Types>;
    type Error;

    fn create(config: &Config, grants: &Self::Grants) -> Result<Self::Instance, Self::Error>;
}
