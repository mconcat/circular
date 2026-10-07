
use std::collections::BTreeMap;

use circular_core::{RecordedInstant, Stamp};
use circular_plan::{ActorId, EdgeId, NamedActorId};
use circular_protocol::actor_events::{
    ActorHealthReason, ActorHealthReasonCode, ActorHealthState, ActorHealthTransition, MailboxDepth,
};
use circular_runtime::DeadLetterReason;

use crate::mailbox_pressure::MailboxPressure;

#[derive(Clone, Debug, PartialEq)]
pub struct HealthFact {
    pub actor: NamedActorId,
    pub state: ActorHealthState,
    pub reason: Option<ActorHealthReason>,
    pub since: RecordedInstant,
    pub mailbox_depths: Option<Vec<(EdgeId, usize)>>,
    pub at: Stamp<ActorId>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct HealthEntry {
    pub(crate) state: ActorHealthState,
    pub(crate) reason: Option<ActorHealthReason>,
    pub(crate) since: RecordedInstant,
    pub(crate) mailbox_depths: Option<Vec<(EdgeId, usize)>>,
}

#[derive(Default)]
pub(crate) struct ActorHealth {
    recorded: Option<(ActorHealthState, Option<ActorHealthReason>)>,
    depths: BTreeMap<EdgeId, usize>,
    awaited: BTreeMap<circular_runtime::EffectId, RecordedInstant>,
    failures: Vec<(ActorHealthReason, RecordedInstant)>,
    arrived: Option<RecordedInstant>,
    harness: Option<ActorHealthReason>,
}

impl ActorHealth {
    pub(crate) fn submitted(&mut self, effect: circular_runtime::EffectId, at: RecordedInstant) {
        self.awaited.entry(effect).or_insert(at);
    }

    pub(crate) fn resumed(&mut self) {
        self.recorded = Some((ActorHealthState::Running, None));
    }

    pub(crate) fn settled(&mut self, effect: &circular_runtime::EffectId) {
        self.awaited.remove(effect);
    }

    pub(crate) fn arrived(&mut self, at: RecordedInstant) {
        self.arrived = Some(at);
    }

    pub(crate) fn failed(&mut self, reason: ActorHealthReason, at: RecordedInstant) {
        self.failures.push((reason, at));
    }

    fn depth_values(&self) -> Option<Vec<(EdgeId, usize)>> {
        (!self.depths.is_empty()).then(|| {
            self.depths
                .iter()
                .map(|(edge, depth)| (edge.clone(), *depth))
                .collect()
        })
    }

    fn record(
        &mut self,
        state: ActorHealthState,
        reason: Option<ActorHealthReason>,
        since: RecordedInstant,
        out: &mut Vec<HealthEntry>,
    ) {
        let fact = (state, reason);
        if self.recorded.as_ref() == Some(&fact) {
            return;
        }
        out.push(HealthEntry {
            state: fact.0,
            reason: fact.1.clone(),
            since,
            mailbox_depths: self.depth_values(),
        });
        self.recorded = Some(fact);
    }

    fn awaited_since(&self) -> Option<RecordedInstant> {
        self.awaited.values().min().copied()
    }

    fn waiting_reason(&self) -> Option<ActorHealthReason> {
        self.harness.clone()
    }

    pub(crate) fn harness_wait(&mut self, harness: Option<ActorHealthReason>) -> bool {
        let changed = self.harness != harness;
        self.harness = harness;
        changed
    }

    pub(crate) fn waiting(&mut self, restated: bool) -> Vec<HealthEntry> {
        let mut out = Vec::new();
        if let Some(since) = self.awaited_since() {
            if restated {
                self.recorded = None;
            }
            self.record(
                ActorHealthState::Waiting,
                self.waiting_reason(),
                since,
                &mut out,
            );
        }
        out
    }

    pub(crate) fn lifecycle(
        &mut self,
        state: ActorHealthState,
        reason: Option<ActorHealthReason>,
        at: RecordedInstant,
    ) -> Vec<HealthEntry> {
        let mut out = Vec::new();
        self.record(state, reason, at, &mut out);
        out
    }

    pub(crate) fn pressure(&mut self, observed: Vec<MailboxPressure>) -> Vec<HealthEntry> {
        let Some(last) = observed.last().map(|pressure| pressure.observed_at) else {
            return Vec::new();
        };
        let mut overflow = false;
        for pressure in observed {
            overflow |= pressure.overflow;
            self.depths.insert(pressure.edge, pressure.depth);
        }
        let mut out = Vec::new();
        if overflow {
            self.record(
                ActorHealthState::Backpressure,
                Some(ActorHealthReason {
                    code: ActorHealthReasonCode::Capacity,
                    detail: circular_core::Value::Null,
                }),
                last,
                &mut out,
            );
        } else if self.depths.values().all(|depth| *depth == 0)
            && matches!(self.recorded, Some((ActorHealthState::Backpressure, _)))
        {
            match self.awaited_since() {
                Some(since) => self.record(
                    ActorHealthState::Waiting,
                    self.waiting_reason(),
                    since,
                    &mut out,
                ),
                None => self.record(ActorHealthState::Running, None, last, &mut out),
            }
        }
        out
    }

    pub(crate) fn turn_end(&mut self) -> Vec<HealthEntry> {
        let failures = std::mem::take(&mut self.failures);
        let arrived = self.arrived.take();
        let mut out = Vec::new();
        for (reason, at) in &failures {
            self.record(
                ActorHealthState::Failed,
                Some(reason.clone()),
                *at,
                &mut out,
            );
        }
        if let Some(since) = self.awaited_since() {
            self.record(
                ActorHealthState::Waiting,
                self.waiting_reason(),
                since,
                &mut out,
            );
            return out;
        }
        let queued_under_pressure =
            matches!(self.recorded, Some((ActorHealthState::Backpressure, _)))
                && self.depths.values().any(|depth| *depth > 0);
        if failures.is_empty()
            && !queued_under_pressure
            && let Some(at) = arrived
        {
            self.record(ActorHealthState::Running, None, at, &mut out);
        }
        out
    }
}

pub(crate) fn dead_letter_actor_failure(reason: &DeadLetterReason) -> Option<ActorHealthReason> {
    match reason {
        DeadLetterReason::Poisoned => Some(ActorHealthReason {
            code: ActorHealthReasonCode::Poisoned,
            detail: circular_core::Value::Null,
        }),
        DeadLetterReason::OutcomeUnclaimed
        | DeadLetterReason::DestinationGone
        | DeadLetterReason::ActorDeclared(_)
        | DeadLetterReason::Processing(_)
        | DeadLetterReason::Capacity => None,
    }
}

pub fn actor_health_record(
    at: Stamp<ActorId>,
    transition: &ActorHealthTransition,
) -> Result<circular_store::Record<circular_store::ProductStore>, String> {
    let value = transition
        .to_value()
        .map_err(|error| format!("actor-health transition does not encode: {error:?}"))?;
    let payload = circular_core::encode(
        &value,
        circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
    )
    .map_err(|error| format!("actor-health transition encoding: {error:?}"))?;
    Ok(circular_store::Record::Observation(
        circular_store::ObservationRecord::diagnostic(
            at,
            circular_store::ObservationBucket::from_millis(transition.since_ms),
            circular_store::RecordOrigin::Stream,
            circular_store::ObservationItemKey::new(
                circular_core::BuiltinObservationName::DiagnosticOccurrence,
                circular_store::OpaqueId::new(0),
            ),
            circular_core::EncodedPayload::new(circular_core::PayloadVersionTag::FIRST, &payload),
        ),
    ))
}

pub fn health_transition(fact: &HealthFact) -> Result<ActorHealthTransition, String> {
    let actor = circular_store::named_actor_value(&fact.actor)
        .map_err(|error| format!("health actor: {error}"))?;
    let actor = circular_protocol::declaration_payload::decode_plan_actor_key(actor)
        .map_err(|error| format!("health actor: {error:?}"))?;
    let mailbox_depths = fact
        .mailbox_depths
        .as_ref()
        .map(|rows| {
            rows.iter()
                .map(|(edge, depth)| {
                    Ok(MailboxDepth {
                        edge: circular_store::edge_value(edge)
                            .map_err(|error| format!("mailbox edge: {error}"))?,
                        depth: u64::try_from(*depth).map_err(|_| "mailbox depth exceeds u64")?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?;
    Ok(ActorHealthTransition {
        actor,
        state: fact.state,
        since_ms: fact.since.millis(),
        reason: fact.reason.clone(),
        mailbox_depths,
    })
}

