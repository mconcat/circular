
use super::inlet::Recorded;
use super::turn::InputOrigin;
use crate::recorded::RecordedArrival;
use circular_core::{CausalParents, Causality, EventId};
use circular_plan::PortId;
use circular_runtime::ArrivalOrigin;
use circular_store::StreamId;

#[derive(Debug)]
pub(crate) struct Unreadable(pub(crate) String);

pub(crate) fn recorded(stream: StreamId, row: &RecordedArrival) -> Result<Recorded, Unreadable> {
    let payload = row.payload().decode().map_err(Unreadable)?;
    let (origin, event_stamp, causality): (InputOrigin, _, _) = match row.origin() {
        ArrivalOrigin::EdgeDelivery { edge, stamp } => {
            let parents = row
                .causal_parents()
                .iter()
                .map(|parent| EventId::derive(stream, parent.clone()))
                .collect::<Vec<_>>();
            let causality = match parents.len() {
                1 => Causality::Derived(parents.into_iter().next().expect("one parent")),
                _ => Causality::Aggregated(
                    CausalParents::try_new(parents)
                        .map_err(|error| Unreadable(format!("causal parents: {error:?}")))?,
                ),
            };
            (
                ArrivalOrigin::EdgeDelivery {
                    edge: edge.clone(),
                    stamp: stamp.clone(),
                },
                stamp.clone(),
                causality,
            )
        }
        ArrivalOrigin::ExternalInject { origin } => (
            ArrivalOrigin::ExternalInject {
                origin: origin.clone(),
            },
            row.stamp().clone(),
            Causality::Source,
        ),
        ArrivalOrigin::TimerFire { timer } => (
            ArrivalOrigin::TimerFire {
                timer: timer.clone(),
            },
            row.stamp().clone(),
            Causality::Aggregated(CausalParents::try_new(Vec::new()).expect("empty parents")),
        ),
        ArrivalOrigin::EffectOutcome { .. } => {
            return Err(Unreadable(
                "an effect outcome belongs to the outcome turn".to_owned(),
            ));
        }
    };
    let event = circular_core::admit(
        stream,
        circular_core::Emission::from_runtime(circular_core::emit(payload), causality, None),
        event_stamp,
    )
    .map_err(|error| Unreadable(format!("recorded event: {error:?}")))?;
    Ok(Recorded {
        index: row.index(),
        at: row.stamp().clone(),
        observed_at: row.observed_at(),
        route: row.route_edge().cloned(),
        origin,
        inlet: row.inlet().clone(),
        revision: event.stamp().revision(),
        event,
        emission_body: row.route_edge().map(|_| row.payload().bytes().clone()),
        result: row.result().clone(),
        outcome: None,
    })
}

pub(crate) struct Outcome {
    pub(crate) index: circular_core::ArrivalIndex,
    pub(crate) at: circular_core::Stamp<circular_plan::ActorId>,
    pub(crate) observed_at: circular_core::RecordedInstant,
    pub(crate) outcome: circular_runtime::EffectOutcome<circular_runtime::EffectId>,
}

pub(crate) fn outcome(
    stream: StreamId,
    actor: circular_plan::NamedActorId,
    recorded: Outcome,
) -> Result<Recorded, Unreadable> {
    let event = super::turn::lifecycle_event(
        stream,
        recorded.at.clone(),
        super::turn::null_payload(),
        &[],
    )
    .map_err(Unreadable)?;
    Ok(Recorded {
        index: recorded.index,
        revision: recorded.at.revision(),
        observed_at: recorded.observed_at,
        route: None,
        origin: ArrivalOrigin::EdgeDelivery {
            edge: circular_plan::EdgeId::outcome(actor),
            stamp: recorded.at.clone(),
        },
        inlet: super::actor::outcome_inlet(),
        at: recorded.at,
        event,
        emission_body: None,
        result: circular_runtime::EnvelopeResult::Ok,
        outcome: Some(Box::new(recorded.outcome)),
    })
}

pub(crate) fn timer_fire(
    stream: StreamId,
    index: circular_core::ArrivalIndex,
    at: circular_core::Stamp<circular_plan::ActorId>,
    observed_at: circular_core::RecordedInstant,
    timer: circular_runtime::EffectId,
    payload: circular_actors::ProductPayload,
) -> Result<Recorded, Unreadable> {
    let event =
        super::turn::lifecycle_event(stream, at.clone(), payload, &[]).map_err(Unreadable)?;
    Ok(Recorded {
        index,
        revision: at.revision(),
        observed_at,
        route: None,
        origin: ArrivalOrigin::TimerFire { timer },
        inlet: PortId::try_derived(circular_actors::TIMER_PORT_NAME.to_owned())
            .map_err(|error| Unreadable(format!("timer inlet: {error:?}")))?,
        at,
        event,
        emission_body: None,
        result: circular_runtime::EnvelopeResult::Ok,
        outcome: None,
    })
}

pub(crate) struct Admitted {
    pub(crate) edge: circular_plan::EdgeId,
    pub(crate) inlet: circular_plan::PortId,
    pub(crate) sender: circular_core::Stamp<circular_plan::ActorId>,
    pub(crate) parents: Box<[circular_core::Stamp<circular_plan::ActorId>]>,
    pub(crate) payload: crate::recorded::RecordedPayload,
    pub(crate) result: circular_runtime::EnvelopeResult,
    pub(crate) observed_at: circular_core::RecordedInstant,
}

pub(crate) fn admitted(
    stream: StreamId,
    admitted: &Admitted,
    credits: &std::collections::BTreeMap<circular_plan::EdgeId, super::share::Credit>,
) -> Result<super::Delivery, Unreadable> {
    let payload = admitted.payload.decode().map_err(Unreadable)?;
    let parents = admitted
        .parents
        .iter()
        .map(|parent| EventId::derive(stream, parent.clone()))
        .collect::<Vec<_>>();
    let causality = match parents.len() {
        1 => Causality::Derived(parents.into_iter().next().expect("one parent")),
        _ => Causality::Aggregated(
            CausalParents::try_new(parents)
                .map_err(|error| Unreadable(format!("causal parents: {error:?}")))?,
        ),
    };
    let event = circular_core::admit(
        stream,
        circular_core::Emission::from_runtime(circular_core::emit(payload), causality, None),
        admitted.sender.clone(),
    )
    .map_err(|error| Unreadable(format!("admitted event: {error:?}")))?;
    Ok(super::Delivery {
        edge: admitted.edge.clone(),
        inlet: admitted.inlet.clone(),
        event,
        encoded: admitted.payload.bytes().clone(),
        result: admitted.result.clone(),
        credit: credits
            .get(&admitted.edge)
            .and_then(|credit| credit.clone().try_acquire_owned().ok()),
    })
}
