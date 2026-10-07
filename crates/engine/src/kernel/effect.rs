
use circular_core::{Stamp, Tick};
use circular_plan::{ActorDecl, ActorId};
use circular_runtime::{
    AgentHarnessName, ApprovalRequestOutcome, ApprovalSpec, ApprovalTicket, Effect, EffectFailure,
    EffectId, EffectOutcome, EffectTerm,
};
use std::collections::BTreeMap;
use std::task::{Context, Poll};

pub(crate) struct Pending {
    cause: Stamp<ActorId>,
    term: Option<EffectTerm>,
    effect: Effect,
    state: Waiting,
    retried: Option<u32>,
}

enum Waiting {
    Unsubmitted,
    Submitted,
    Answered,
    Approval,
    Failed(EffectFailure),
    Retry(Tick),
    Slot(std::sync::Arc<crate::process_slots::ProcessPermit>),
    Harness(HarnessWait),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HarnessWait {
    Unbound,
    ProgramNotExecutable,
}

impl HarnessWait {
    pub(crate) fn reason(self) -> circular_protocol::actor_events::ActorHealthReason {
        use circular_protocol::actor_events::{ActorHealthReason, ActorHealthReasonCode};
        match self {
            Self::Unbound => ActorHealthReason {
                code: ActorHealthReasonCode::HarnessUnbound,
                detail: circular_core::Value::Null,
            },
            Self::ProgramNotExecutable => ActorHealthReason {
                code: ActorHealthReasonCode::HarnessUnusable,
                detail: crate::activation_detail::agent_harness::PROGRAM_NOT_EXECUTABLE.to_value(),
            },
        }
    }
}

struct Request {
    cause: Stamp<ActorId>,
    term: EffectTerm,
    target: EffectId,
}

pub(crate) enum Decided {
    Approved { standing: Option<EffectId> },
    Failed(EffectOutcome<EffectId>),
}

pub(crate) struct EffectPort {
    registry: crate::DirectEffectExecutorRegistry,
    pause: Option<bool>,
    listener: bool,
    pending: BTreeMap<EffectId, Pending>,
    requests: BTreeMap<EffectId, Request>,
    schedule: crate::effect_retry::RetrySchedule,
    draining: Vec<crate::DirectEffectExecutorRegistry>,
    slots: Option<std::sync::Arc<crate::process_slots::ProcessSlots>>,
    settled: std::collections::VecDeque<EffectOutcome<EffectId>>,
    peer: Option<super::peer::PeerPort>,
    harness: Option<AgentHarnessName>,
}

fn dispatch(
    pause: Option<bool>,
    harness: Option<&AgentHarnessName>,
    registry: &mut crate::DirectEffectExecutorRegistry,
    slots: Option<&std::sync::Arc<crate::process_slots::ProcessSlots>>,
    settled: &mut std::collections::VecDeque<EffectOutcome<EffectId>>,
    id: &EffectId,
    pending: &mut Pending,
) {
    match pause {
        Some(true) => {
            interrupt(settled, id, pending);
            return;
        }
        Some(false) => return,
        None => {}
    }
    pending.state = Waiting::Submitted;
    let submitted = match (&pending.effect, slots) {
        (Effect::Spawn { .. }, Some(slots)) => match slots.reserve(id, usize::MAX) {
            None => {
                settled.push_back(EffectOutcome::new(
                    id.clone(),
                    Err(EffectFailure::InterpreterFault(
                        circular_runtime::InterpreterFault::ResourceExhausted,
                    )),
                ));
                return;
            }
            Some(permit) if !permit.ready() => {
                pending.state = Waiting::Slot(permit);
                return;
            }
            Some(permit) => registry.submit_leased(id.clone(), &pending.effect, permit),
        },
        _ => registry.submit(id.clone(), &pending.effect),
    };
    match submitted {
        Ok(()) => {}
        Err(crate::direct_effect::DirectExecutorSubmitError::MissingExecutor(
            crate::DirectExecutorSelector::AgentHarness(missing),
        )) if harness == Some(&missing) => pending.state = Waiting::Harness(HarnessWait::Unbound),
        Err(crate::direct_effect::DirectExecutorSubmitError::ExecutorRejected(
            circular_runtime::SubmitError::ProgramNotExecutable,
        )) if harness.is_some() => {
            pending.state = Waiting::Harness(HarnessWait::ProgramNotExecutable);
        }
        Err(error) => refused(settled, id, &error),
    }
}

fn interrupt(
    settled: &mut std::collections::VecDeque<EffectOutcome<EffectId>>,
    id: &EffectId,
    pending: &mut Pending,
) {
    settled.push_back(EffectOutcome::new(
        id.clone(),
        Err(EffectFailure::InterpreterFault(
            circular_runtime::InterpreterFault::Interrupted,
        )),
    ));
    pending.state = Waiting::Submitted;
}

fn refused(
    settled: &mut std::collections::VecDeque<EffectOutcome<EffectId>>,
    id: &EffectId,
    error: &dyn std::fmt::Display,
) {
    eprintln!("circular-kernel: effect {id:?} was not submitted: {error}");
    settled.push_back(EffectOutcome::new(
        id.clone(),
        Err(EffectFailure::InterpreterFault(
            circular_runtime::InterpreterFault::Other,
        )),
    ));
}

impl EffectPort {
    pub(crate) fn of(
        execution: &crate::execution_profile::ProductExecutionProfile,
        declaration: &ActorDecl,
    ) -> Result<Option<Self>, crate::activation_detail::RegistrationFailure> {
        use crate::activation_detail::{RegistrationFailure, activation};
        let spec = circular_actors::get(*declaration.domain().actor_type());
        let Some(stand_ins) = spec.effect().stand_ins() else {
            return Ok(None);
        };
        let folded = crate::actor_capability::config(declaration)
            .map_err(|reason| RegistrationFailure::new(activation::CONFIG_FOLD, reason))?;
        let filesystem = crate::actor_capability::filesystem(&folded);
        let listener = *declaration.domain().actor_type() == circular_core::ActorType::Listener;
        let harness = stand_ins
            .get(circular_runtime::EffectCtor::AgentInvoke)
            .map(|_| execution.declared_agent_harness(declaration))
            .transpose()
            .map_err(|reason| {
                RegistrationFailure::new(
                    circular_actors::ProductFactoryError::Agent(
                        circular_actors::AgentFactoryError::Config(
                            circular_actors::config::ConfigRejection::Missing("harness"),
                        ),
                    )
                    .detail(),
                    reason,
                )
            })?;
        let registry = execution
            .effect_registry(&filesystem, harness.as_ref(), |effect| {
                stand_ins.get(effect).is_some()
                    && !(listener && effect == circular_runtime::EffectCtor::FileRead)
            })
            .map_err(|error| {
                RegistrationFailure::new(
                    activation::EFFECT_EXECUTOR_REGISTRATION,
                    error.to_string(),
                )
            })?;
        let schedule = match circular_actors::retry_config::declared(folded.value())
            .map_err(|reason| RegistrationFailure::new(activation::RETRY_ADMISSION, reason))?
        {
            Some(waits) => crate::effect_retry::RetrySchedule::from_checked_ms(waits),
            None => execution.effect_retry().clone(),
        };
        Ok(Some(Self {
            registry,
            pause: None,
            listener,
            pending: BTreeMap::new(),
            requests: BTreeMap::new(),
            schedule,
            draining: Vec::new(),
            slots: stand_ins
                .get(circular_runtime::EffectCtor::Spawn)
                .and_then(|_| execution.process_slots()),
            settled: std::collections::VecDeque::new(),
            peer: execution
                .peer_adapter(declaration)?
                .map(super::peer::PeerPort::new),
            harness,
        }))
    }

    pub(crate) fn bind_listener(
        &mut self,
        plan: super::source::ListenerPlan,
        entrance: super::system::Entrance,
        paused: bool,
    ) {
        self.registry
            .register(
                [crate::DirectExecutorSelector::Constructor(
                    circular_runtime::EffectCtor::FileRead,
                )],
                super::source::ListenerInterpreter::new(plan, entrance, paused),
            )
            .expect("the listener reserves its FileRead executor at assembly");
    }

    pub(crate) fn stands(&self, effect: circular_runtime::EffectCtor) -> bool {
        effect.is_external_entry()
            || (self.listener && effect == circular_runtime::EffectCtor::FileRead)
    }

    pub(crate) fn begin(
        &mut self,
        id: EffectId,
        effect: Effect,
        cause: Stamp<ActorId>,
        live: bool,
    ) {
        let term = EffectTerm::from_effect(&effect);
        let retried =
            (crate::effect_retry::may_retry(&effect) && !self.schedule.is_empty()).then_some(0);
        let mut pending = Pending {
            cause,
            term,
            effect,
            state: Waiting::Unsubmitted,
            retried,
        };
        if live {
            dispatch(
                self.pause,
                self.harness.as_ref(),
                &mut self.registry,
                self.slots.as_ref(),
                &mut self.settled,
                &id,
                &mut pending,
            );
        }
        self.pending.insert(id, pending);
    }

    pub(crate) fn begin_peer(
        &mut self,
        id: EffectId,
        effect: circular_runtime::PeerEffect,
        cause: Stamp<ActorId>,
        live: bool,
    ) -> bool {
        let Some(peer) = self.peer.as_mut() else {
            return false;
        };
        peer.begin(id, effect, cause, live);
        true
    }

    pub(crate) fn acknowledge_peer(&mut self, id: &EffectId, index: circular_core::ArrivalIndex) {
        if let Some(peer) = self.peer.as_mut() {
            peer.acknowledge(id, index);
        }
    }

    pub(crate) fn knows(&self, id: &EffectId) -> bool {
        self.recorded(id).is_some()
    }

    pub(crate) fn hold(
        &mut self,
        request: EffectId,
        target: EffectId,
        effect: Effect,
        cause: Stamp<ActorId>,
    ) -> ApprovalSpec {
        let spec = ApprovalSpec::new(target.clone());
        self.requests.insert(
            request.clone(),
            Request {
                cause: cause.clone(),
                term: EffectTerm::RequestApproval(spec.clone()),
                target: target.clone(),
            },
        );
        self.pending.insert(
            target,
            Pending {
                cause,
                term: EffectTerm::from_effect(&effect),
                retried: (crate::effect_retry::may_retry(&effect) && !self.schedule.is_empty())
                    .then_some(0),
                effect,
                state: Waiting::Approval,
            },
        );
        spec
    }

    pub(crate) fn recorded(&self, id: &EffectId) -> Option<(Stamp<ActorId>, Option<EffectTerm>)> {
        if let Some(pending) = self.pending.get(id) {
            return Some((pending.cause.clone(), pending.term.clone()));
        }
        if let Some(recorded) = self.peer.as_ref().and_then(|peer| peer.recorded(id)) {
            return Some(recorded);
        }
        self.requests
            .get(id)
            .map(|request| (request.cause.clone(), Some(request.term.clone())))
    }

    pub(crate) fn requests(&self) -> impl Iterator<Item = &EffectId> {
        self.requests.keys()
    }

    pub(crate) fn is_request(&self, id: &EffectId) -> bool {
        self.requests.contains_key(id)
    }

    pub(crate) fn decided(
        &mut self,
        request: &EffectId,
        decision: Result<ApprovalRequestOutcome<EffectId>, EffectFailure>,
        live: bool,
    ) -> Option<Decided> {
        let Some(Request { target, .. }) = self.requests.remove(request) else {
            return None;
        };
        let standing = self
            .pending
            .get(&target)
            .is_some_and(|pending| self.stands(pending.effect.constructor()));
        let Some(pending) = self.pending.get_mut(&target) else {
            return None;
        };
        if !matches!(pending.state, Waiting::Approval) {
            return None;
        }
        match decision {
            Ok(ApprovalRequestOutcome::Approved(ticket)) => {
                attach_ticket(&mut pending.effect, ticket);
                pending.term = EffectTerm::from_effect(&pending.effect);
                pending.state = Waiting::Unsubmitted;
                if live {
                    dispatch(
                        self.pause,
                        self.harness.as_ref(),
                        &mut self.registry,
                        self.slots.as_ref(),
                        &mut self.settled,
                        &target,
                        pending,
                    );
                }
                Some(Decided::Approved {
                    standing: standing.then_some(target),
                })
            }
            Ok(ApprovalRequestOutcome::Denied) => {
                pending.state = Waiting::Failed(EffectFailure::ApprovalRequired);
                Some(Decided::Failed(EffectOutcome::new(
                    target,
                    Err(EffectFailure::ApprovalRequired),
                )))
            }
            Err(failure) => {
                pending.state = Waiting::Failed(failure.clone());
                Some(Decided::Failed(EffectOutcome::new(target, Err(failure))))
            }
        }
    }

    pub(crate) fn adopt(&mut self, mut before: Self, live: bool) {
        if let Some(force) = before.pause {
            self.pause(force);
        }
        if live && before.listener {
            before.pause(true);
            before.registry.begin_cancel();
        }
        let Self {
            registry,
            pending,
            requests,
            draining,
            settled,
            ..
        } = before;
        self.pending.extend(pending);
        self.requests.extend(requests);
        self.draining.extend(draining);
        self.settled.extend(settled);
        if self.awaiting() {
            self.draining.push(registry);
        }
        if live {
            self.release_harness_waits();
        }
    }

    pub(crate) fn rebind(
        &mut self,
        harness: &AgentHarnessName,
        executor: Option<&crate::execution_profile::AgentExecutorFactory>,
    ) -> bool {
        if self.harness.as_ref() != Some(harness) {
            return false;
        }
        let mut registry = self.registry.empty_on_same_vault();
        if let Some(factory) = executor {
            registry
                .register_boxed(
                    [crate::DirectExecutorSelector::AgentHarness(harness.clone())],
                    factory(),
                )
                .expect("an empty registry takes one harness executor");
        }
        if let Some(force) = self.pause {
            registry.pause(force);
        }
        let before = std::mem::replace(&mut self.registry, registry);
        if self.awaiting() {
            self.draining.push(before);
        }
        self.release_harness_waits();
        true
    }

    fn release_harness_waits(&mut self) {
        for (id, pending) in &mut self.pending {
            if matches!(pending.state, Waiting::Harness(_)) {
                pending.state = Waiting::Unsubmitted;
                dispatch(
                    self.pause,
                    self.harness.as_ref(),
                    &mut self.registry,
                    self.slots.as_ref(),
                    &mut self.settled,
                    id,
                    pending,
                );
            }
        }
    }

    pub(crate) fn harness_wait(&self) -> Option<HarnessWait> {
        self.pending
            .values()
            .find_map(|pending| match pending.state {
                Waiting::Harness(wait) => Some(wait),
                _ => None,
            })
    }

    pub(crate) fn harness(&self) -> Option<&AgentHarnessName> {
        self.harness.as_ref()
    }

    pub(crate) fn answered(&mut self, id: &EffectId) {
        if let Some(pending) = self.pending.get_mut(id) {
            pending.state = Waiting::Answered;
        }
        if let Some(peer) = self.peer.as_mut() {
            peer.answered(id);
        }
    }

    pub(crate) fn settle(&mut self, id: &EffectId) {
        self.pending.remove(id);
        self.requests.remove(id);
        if let Some(peer) = self.peer.as_mut() {
            peer.settle(id);
        }
        if !self.awaiting() {
            self.draining.clear();
        }
    }

    pub(crate) fn settle_or_retry(
        &mut self,
        outcome: EffectOutcome<EffectId>,
        now: Tick,
        ticks_per_second: u32,
    ) -> Option<EffectOutcome<EffectId>> {
        let id = outcome.correlation().clone();
        if let Some(request) = self.requests.get(&id)
            && !self
                .pending
                .get(&request.target)
                .is_some_and(|pending| matches!(pending.state, Waiting::Approval))
        {
            return Some(EffectOutcome::new(id, Err(EffectFailure::EndpointGone)));
        }
        let Some(pending) = self.pending.get_mut(&id) else {
            return Some(outcome);
        };
        let Some(retried) = pending.retried else {
            return Some(outcome);
        };
        let Some(server_after) = crate::effect_retry::transient(&pending.effect, outcome.result())
        else {
            return Some(outcome);
        };
        let attempts = retried.saturating_add(1);
        let Some(wait) = self.schedule.wait_before(attempts) else {
            return Some(EffectOutcome::new(
                id,
                Err(EffectFailure::RetryExhausted { attempts }),
            ));
        };
        let after = wait.max(server_after.unwrap_or(0).saturating_mul(1000));
        eprintln!(
            "circular: effect {id} attempt {attempts} failed transiently ({}); retry {attempts} of {} after {after}ms",
            match outcome.result() {
                Ok(circular_runtime::OutcomePayload::HttpResponse(response)) =>
                    format!("status {}", response.status()),
                Ok(payload) => payload.kind_tag().to_owned(),
                Err(failure) => failure.kind_tag().to_owned(),
            },
            self.schedule.ms().len(),
        );
        pending.retried = Some(attempts);
        pending.state = Waiting::Retry(Tick::new(
            now.get().saturating_add(
                after
                    .saturating_mul(u64::from(ticks_per_second))
                    .div_ceil(1000),
            ),
        ));
        None
    }

    pub(crate) fn next_retry(&self) -> Option<Tick> {
        if self.pause.is_some() {
            return None;
        }
        self.pending
            .values()
            .filter_map(|pending| match pending.state {
                Waiting::Retry(at) => Some(at),
                _ => None,
            })
            .min()
    }

    pub(crate) fn release_retries(&mut self, now: Tick) {
        for (id, pending) in &mut self.pending {
            if matches!(pending.state, Waiting::Retry(at) if at <= now) {
                dispatch(
                    self.pause,
                    self.harness.as_ref(),
                    &mut self.registry,
                    self.slots.as_ref(),
                    &mut self.settled,
                    id,
                    pending,
                );
            }
        }
    }

    pub(crate) fn pause(&mut self, force: bool) {
        self.pause = Some(force);
        self.registry.pause(force);
        if let Some(peer) = self.peer.as_mut() {
            peer.pause();
        }
        for registry in &mut self.draining {
            registry.pause(force);
        }
        for (id, pending) in &mut self.pending {
            let cancelled = match pending.state {
                Waiting::Retry(_) => true,
                Waiting::Unsubmitted if self.listener => false,
                Waiting::Unsubmitted
                | Waiting::Slot(_)
                | Waiting::Approval
                | Waiting::Harness(_) => force,
                _ => false,
            };
            if cancelled {
                interrupt(&mut self.settled, id, pending);
            }
        }
    }

    pub(crate) fn replay_force_pause(&mut self, consumed: circular_core::ArrivalIndex) {
        for pending in self.pending.values_mut() {
            if pending.cause.sequence().get() >= consumed.get() {
                continue;
            }
            match pending.state {
                Waiting::Unsubmitted if self.listener => {}
                Waiting::Answered | Waiting::Failed(_) => {}
                _ => {
                    pending.state = Waiting::Failed(EffectFailure::InterpreterFault(
                        circular_runtime::InterpreterFault::Interrupted,
                    ));
                }
            }
        }
    }

    pub(crate) fn resume(&mut self) {
        let held = self.pause.take().is_some();
        self.registry.resume();
        if let Some(peer) = self.peer.as_mut() {
            peer.resume();
        }
        for registry in &mut self.draining {
            registry.resume();
        }
        if held {
            for (id, pending) in &mut self.pending {
                if matches!(pending.state, Waiting::Unsubmitted) {
                    dispatch(
                        self.pause,
                        self.harness.as_ref(),
                        &mut self.registry,
                        self.slots.as_ref(),
                        &mut self.settled,
                        id,
                        pending,
                    );
                }
            }
        }
    }

    pub(crate) fn idle(&self) -> bool {
        self.pending.is_empty()
            && self.requests.is_empty()
            && self.settled.is_empty()
            && self.peer.as_ref().is_none_or(super::peer::PeerPort::idle)
    }

    pub(crate) fn awaiting(&self) -> bool {
        !self.settled.is_empty()
            || self
                .peer
                .as_ref()
                .is_some_and(super::peer::PeerPort::awaiting)
            || self
                .pending
                .values()
                .any(|pending| matches!(pending.state, Waiting::Submitted | Waiting::Slot(_)))
    }

    pub(crate) fn resubmit(&mut self) -> Vec<EffectOutcome<EffectId>> {
        let mut denied = self
            .peer
            .as_mut()
            .map(super::peer::PeerPort::interrupted)
            .unwrap_or_default();
        for (id, pending) in &mut self.pending {
            match &pending.state {
                Waiting::Unsubmitted if self.listener && self.pause.is_some() => {}
                Waiting::Unsubmitted | Waiting::Retry(_) | Waiting::Harness(_) => {
                    dispatch(
                        self.pause,
                        self.harness.as_ref(),
                        &mut self.registry,
                        self.slots.as_ref(),
                        &mut self.settled,
                        id,
                        pending,
                    );
                }
                Waiting::Failed(failure) => {
                    denied.push(EffectOutcome::new(id.clone(), Err(failure.clone())));
                }
                Waiting::Submitted | Waiting::Answered | Waiting::Approval | Waiting::Slot(_) => {}
            }
        }
        denied
    }

    pub(crate) fn poll_outcome(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<EffectOutcome<EffectId>> {
        for (id, pending) in &mut self.pending {
            if self.pause.is_some() {
                break;
            }
            let Waiting::Slot(permit) = &pending.state else {
                continue;
            };
            if !permit.ready() {
                permit.set_waker(context.waker().clone());
                continue;
            }
            let permit = permit.clone();
            if let Err(error) = self
                .registry
                .submit_leased(id.clone(), &pending.effect, permit)
            {
                refused(&mut self.settled, id, &error);
            }
            pending.state = Waiting::Submitted;
        }
        if let Some(outcome) = self.settled.pop_front() {
            return Poll::Ready(outcome);
        }
        self.registry.set_outcome_waker(context.waker().clone());
        if let Some(outcome) = self.registry.next_outcome() {
            return Poll::Ready(outcome);
        }
        for registry in &mut self.draining {
            registry.set_outcome_waker(context.waker().clone());
            if let Some(outcome) = registry.next_outcome() {
                return Poll::Ready(outcome);
            }
        }
        if let Some(peer) = self.peer.as_mut()
            && let Poll::Ready(outcome) = peer.poll(context)
        {
            return Poll::Ready(outcome);
        }
        Poll::Pending
    }
}

fn attach_ticket(effect: &mut Effect, approved: ApprovalTicket<EffectId>) {
    match effect {
        Effect::Http { ticket, .. }
        | Effect::FileRead { ticket, .. }
        | Effect::FileWrite { ticket, .. }
        | Effect::Spawn { ticket, .. }
        | Effect::Notify { ticket, .. }
        | Effect::AgentInvoke { ticket, .. } => *ticket = Some(approved),
        Effect::RequestApproval { .. }
        | Effect::MutateInstance { .. }
        | Effect::Schedule { .. } => {
            unreachable!("only ticket-capable effects are held for approval")
        }
    }
}

#[derive(Default)]
pub(crate) struct Timers {
    armed: BTreeMap<EffectId, Armed>,
    dispatched: std::collections::BTreeSet<EffectId>,
}

pub(crate) struct Armed {
    pub(crate) deadline: Tick,
    pub(crate) correlation: u64,
}

impl Timers {
    pub(crate) fn arm(&mut self, id: EffectId, armed: Armed) {
        self.armed.insert(id, armed);
    }

    pub(crate) fn fired(&mut self, id: &EffectId) {
        self.armed.remove(id);
        self.dispatched.remove(id);
    }

    pub(crate) fn recorded(&mut self, id: &EffectId) {
        self.armed.remove(id);
        self.dispatched.insert(id.clone());
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.armed.is_empty() && self.dispatched.is_empty()
    }

    pub(crate) fn next_deadline(&self) -> Option<Tick> {
        self.armed.values().map(|armed| armed.deadline).min()
    }

    pub(crate) fn due(&mut self, now: Tick) -> Vec<(EffectId, Armed)> {
        let due: Vec<EffectId> = self
            .armed
            .iter()
            .filter(|(_, armed)| armed.deadline <= now)
            .map(|(id, _)| id.clone())
            .collect();
        let mut fired: Vec<(EffectId, Armed)> = due
            .into_iter()
            .filter_map(|id| self.armed.remove(&id).map(|armed| (id, armed)))
            .collect();
        self.dispatched
            .extend(fired.iter().map(|(id, _)| id.clone()));
        fired.sort_by_key(|(_, armed)| armed.deadline);
        fired
    }
}
