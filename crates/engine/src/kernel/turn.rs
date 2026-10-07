
use super::ProductEvent;
use super::inlet::Recorded;
use super::share::InletWire;
use crate::inlet_preprocess::{Element, InletVerdict};
use circular_actors::ProductPayload;
use circular_core::{ActorType, Emission, EventId};
use circular_plan::{ActorId, Config, Incarnation, PortId};
use circular_runtime::{
    ActorContext, ActorEffect, ActorInput, ArrivalOrigin, EffectId, EmittingActor, EnvelopeResult,
};
use circular_store::StreamId;
use std::panic::{AssertUnwindSafe, catch_unwind};

pub(crate) struct ProductTypes;

impl circular_runtime::ActorTypes for ProductTypes {
    type Stream = StreamId;
    type Event = super::ProductEvent;
    type Payload = ProductPayload;
    type EffectId = circular_runtime::EffectId;
    type StateVersion = u16;
    type Observation = circular_actors::ProductObservation;
    type Grants = Grants;

    fn payload(event: &Self::Event) -> &Self::Payload {
        event.payload()
    }
}
pub(crate) type ProductActor = circular_actors::ProductActor<ProductTypes>;
pub(crate) type Grants = crate::tap_pilot::TapPilotGrantBundle;
pub(crate) type InputOrigin = ArrivalOrigin<ActorId, std::convert::Infallible, EffectId>;

pub(crate) struct Behavior {
    pub(crate) id: ActorId,
    pub(crate) actor_type: ActorType,
    pub(crate) incarnation: Incarnation<StreamId>,
    pub(crate) config: Config,
    pub(crate) grants: Grants,
    pub(crate) actor: ProductActor,
    pub(crate) effects: Option<super::effect::EffectPort>,
    pub(crate) timers: super::effect::Timers,
    pub(crate) approval: crate::actor_approval::ApprovalDemand,
    pub(crate) source: Option<super::source::Plan>,
}

pub(crate) fn preprocess(
    wire: &InletWire,
    arrival: &Recorded,
    actor_type: ActorType,
) -> InletVerdict<Element<ProductPayload>> {
    wire.shape.program.apply_result_to_inlet(
        arrival.event.payload(),
        &arrival.result,
        actor_type,
        &arrival.inlet,
    )
}

pub(crate) enum Hooked {
    Effects(Vec<ActorEffect<ProductPayload>>),
    Panicked,
}

impl Behavior {
    pub(crate) fn on_arrival(
        &mut self,
        arrival: &Recorded,
        origin: &InputOrigin,
        inputs: Vec<(ProductPayload, EnvelopeResult)>,
    ) -> Hooked {
        let stream = self.incarnation.stream().clone();
        let inputs: Vec<ActorInput<ProductEvent>> = inputs
            .into_iter()
            .map(|(payload, result)| {
                let event = project(&stream, &arrival.event, payload)
                    .with_recorded_instant(Some(arrival.observed_at));
                ActorInput::new(arrival.inlet.clone(), event)
                    .with_arrival(arrival.index, arrival.route.clone(), origin.clone())
                    .with_result(result)
            })
            .collect();
        let context = ActorContext::new(&self.id, &self.incarnation, &self.config, &self.grants);
        let actor = &mut self.actor;
        match catch_unwind(AssertUnwindSafe(|| {
            inputs
                .iter()
                .flat_map(|input| {
                    <ProductActor as EmittingActor<ProductTypes, ProductPayload>>::on_event(
                        actor, input, &context,
                    )
                    .into_iter()
                })
                .collect::<Vec<_>>()
        })) {
            Ok(effects) => Hooked::Effects(effects),
            Err(_) => Hooked::Panicked,
        }
    }

    pub(crate) fn on_outcome(
        &mut self,
        outcome: &circular_runtime::EffectOutcome<EffectId>,
    ) -> Hooked {
        let context = ActorContext::new(&self.id, &self.incarnation, &self.config, &self.grants);
        let actor = &mut self.actor;
        match catch_unwind(AssertUnwindSafe(|| {
            <ProductActor as EmittingActor<ProductTypes, ProductPayload>>::on_outcome(
                actor, outcome, &context,
            )
            .into_iter()
            .collect::<Vec<_>>()
        })) {
            Ok(effects) => Hooked::Effects(effects),
            Err(_) => Hooked::Panicked,
        }
    }

    pub(crate) fn on_lifecycle(&mut self, life: circular_runtime::ActorLifecycle) -> Hooked {
        let context = ActorContext::new(&self.id, &self.incarnation, &self.config, &self.grants);
        let actor = &mut self.actor;
        match catch_unwind(AssertUnwindSafe(|| {
            <ProductActor as EmittingActor<ProductTypes, ProductPayload>>::on_lifecycle(
                actor, life, &context,
            )
            .into_iter()
            .collect::<Vec<_>>()
        })) {
            Ok(effects) => Hooked::Effects(effects),
            Err(_) => Hooked::Panicked,
        }
    }

    pub(crate) fn absorbs(&mut self, config: &Config) -> bool {
        let Ok(folded) = crate::activation_config::fold_config(self.actor_type, config) else {
            return false;
        };
        <ProductActor as circular_runtime::EditableActor>::on_config_change(
            &mut self.actor,
            &folded,
        ) == circular_runtime::ConfigChangeOutcome::Absorbed
    }

    pub(crate) fn hand_over(&self, next: &mut Self) -> Result<(), String> {
        let Some(state) =
            <ProductActor as circular_runtime::EditableActor>::checkpoint(&self.actor)
        else {
            return Ok(());
        };
        next.restore_state(state)
    }

    pub(crate) fn state(&self) -> Result<Option<circular_runtime::ActorState<u16>>, &'static str> {
        let state = <ProductActor as circular_runtime::EditableActor>::try_checkpoint(&self.actor)
            .map_err(|_| "hook_state_encode_failed")?;
        if state.is_none()
            && !<ProductActor as circular_runtime::EditableActor>::stateless(&self.actor)
        {
            return Err("hook_state_unavailable");
        }
        Ok(state)
    }

    pub(crate) fn restore_state(
        &mut self,
        state: circular_runtime::ActorState<u16>,
    ) -> Result<(), String> {
        <ProductActor as circular_runtime::EditableActor>::restore(&mut self.actor, state)
            .map_err(|error| format!("{error:?}"))
    }

    pub(crate) fn observed_slots(&self) -> Box<[circular_core::Value]> {
        <ProductActor as EmittingActor<ProductTypes, ProductPayload>>::observe(&self.actor)
            .map_or_else(
                || Box::new([]) as Box<[circular_core::Value]>,
                |observed| observed.slots(),
            )
    }

    pub(crate) fn instance_disposition(
        &mut self,
        intent: &circular_runtime::InstanceIntent,
        disposition: circular_runtime::InstanceDisposition,
    ) -> Option<circular_runtime::ScheduleSpec> {
        <ProductActor as circular_runtime::EditableActor>::on_instance_disposition(
            &mut self.actor,
            intent,
            disposition,
        )
    }

    pub(crate) fn accepts(&self, inlet: &PortId) -> bool {
        <ProductActor as EmittingActor<ProductTypes, ProductPayload>>::accepts(&self.actor, inlet)
    }

    pub(crate) fn inlet_type(&self) -> ActorType {
        self.actor_type
    }
}

fn project(stream: &StreamId, original: &ProductEvent, payload: ProductPayload) -> ProductEvent {
    if original.payload() == &payload {
        return original.clone();
    }
    circular_core::admit(
        stream.clone(),
        Emission::from_runtime(
            circular_core::emit(payload),
            original.causality().clone(),
            original.operation().cloned(),
        ),
        original.stamp().clone(),
    )
    .expect("a projection keeps the admitted causality of its original")
}

pub(crate) fn emission_event(
    stream: &StreamId,
    cause: &ProductEvent,
    payload: ProductPayload,
    stamp: circular_core::Stamp<ActorId>,
) -> Result<ProductEvent, circular_core::AdmissionError<ActorId>> {
    let parent: EventId<StreamId, ActorId> = cause.id();
    circular_core::admit(
        stream.clone(),
        Emission::from_runtime(
            circular_core::emit(payload),
            circular_core::Causality::Derived(parent),
            cause.operation().cloned(),
        ),
        stamp,
    )
}

pub(crate) fn null_payload() -> ProductPayload {
    ProductPayload::new(
        circular_actors::GroundShape::try_new(circular_actors::Shape::Any)
            .expect("Any has no type variable"),
        circular_actors::ProductValue::Null,
    )
}

pub(crate) fn correlation_payload(correlation: u64) -> ProductPayload {
    ProductPayload::new(
        circular_actors::GroundShape::try_new(circular_actors::Shape::Base(
            circular_actors::BaseShape::UInt,
        ))
        .expect("UInt is ground"),
        circular_actors::ProductValue::UInt(correlation),
    )
}

pub(crate) fn lifecycle_event(
    stream: StreamId,
    at: circular_core::Stamp<ActorId>,
    payload: ProductPayload,
    parents: &[circular_core::Stamp<ActorId>],
) -> Result<ProductEvent, String> {
    let parents: Vec<_> = parents
        .iter()
        .map(|at| EventId::derive(stream, at.clone()))
        .collect();
    let causality = match parents.as_slice() {
        [parent] => circular_core::Causality::Derived(parent.clone()),
        _ => circular_core::Causality::Aggregated(
            circular_core::CausalParents::try_new(parents)
                .map_err(|error| format!("lifecycle parents: {error:?}"))?,
        ),
    };
    circular_core::admit(
        stream,
        Emission::from_runtime(circular_core::emit(payload), causality, None),
        at,
    )
    .map_err(|error| format!("lifecycle event: {error:?}"))
}

pub(crate) fn lifecycle_payload(life: &super::actor::Life) -> ProductPayload {
    use super::actor::Life;
    use circular_actors::ProductValue as V;
    let fields = match life {
        Life::Activate => vec![V::UInt(0)],
        Life::Become(_) => vec![V::UInt(1)],
        Life::Stop(consumed) => vec![V::UInt(2), V::UInt(consumed.get())],
        Life::Pause { force, consumed } => {
            vec![V::UInt(3), V::Bool(*force), V::UInt(consumed.get())]
        }
        Life::Resume => vec![V::UInt(4)],
    };
    ProductPayload::new(
        circular_actors::GroundShape::try_new(circular_actors::Shape::Any)
            .expect("Any has no type variable"),
        V::Array(fields),
    )
}

pub(crate) fn lifecycle(
    inlet: &PortId,
    value: &circular_actors::ProductValue,
) -> Result<Option<super::actor::Life>, String> {
    use super::actor::Life;
    use circular_actors::ProductValue as V;
    if !super::actor::is_lifecycle(inlet) {
        return Ok(None);
    }
    let V::Array(fields) = value else {
        return Err("lifecycle arrival requires a tagged body".into());
    };
    let life = match fields.as_slice() {
        [V::UInt(0)] => Life::Activate,
        [V::UInt(1)] => Life::Become(None),
        [V::UInt(2), V::UInt(consumed)] => Life::Stop(circular_core::ArrivalIndex::new(*consumed)),
        [V::UInt(3), V::Bool(force), V::UInt(consumed)] => Life::Pause {
            force: *force,
            consumed: circular_core::ArrivalIndex::new(*consumed),
        },
        [V::UInt(4)] => Life::Resume,
        _ => return Err("lifecycle arrival has an unknown kind or invalid fields".into()),
    };
    Ok(Some(life))
}
