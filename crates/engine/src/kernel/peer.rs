
use circular_core::{ArrivalIndex, Stamp};
use circular_plan::ActorId;
use circular_runtime::{
    CrashDurablePeerOutcomeArrival, EffectFailure, EffectId, EffectOutcome, EffectTerm,
    InterpreterFault, OutcomePayload, PeerAdapter, PeerEffect, PeerEffectTerm, PeerEventEnvelope,
    PeerFailureKind, PeerOutcomeArrivalReceipt, PeerOutcomeArrivalStore,
};
use std::collections::{BTreeMap, VecDeque};
use std::task::{Context, Poll};

pub(crate) struct PeerPort {
    adapter: Box<dyn PeerAdapter + Send>,
    pending: BTreeMap<EffectId, Pending>,
    settled: VecDeque<EffectOutcome<EffectId>>,
    handed: BTreeMap<EffectId, PeerEventEnvelope>,
    paused: bool,
}

struct Pending {
    cause: Stamp<ActorId>,
    effect: PeerEffect,
    state: State,
}

enum State {
    Counted,
    Receiving,
    Answered,
}

struct RecordedOutcome;

impl PeerOutcomeArrivalStore for RecordedOutcome {
    type Error = std::convert::Infallible;

    fn outcome_arrival_receipt(
        &mut self,
        event: &PeerEventEnvelope,
    ) -> Result<PeerOutcomeArrivalReceipt, Self::Error> {
        Ok(PeerOutcomeArrivalReceipt::new(
            event.binding().clone(),
            event.cursor(),
        ))
    }
}

impl CrashDurablePeerOutcomeArrival for RecordedOutcome {}

impl PeerPort {
    pub(crate) fn idle(&self) -> bool {
        self.pending.is_empty() && self.settled.is_empty() && self.handed.is_empty()
    }

    pub(crate) fn new(adapter: Box<dyn PeerAdapter + Send>) -> Self {
        Self {
            adapter,
            pending: BTreeMap::new(),
            settled: VecDeque::new(),
            handed: BTreeMap::new(),
            paused: false,
        }
    }

    pub(crate) fn begin(
        &mut self,
        id: EffectId,
        effect: PeerEffect,
        cause: Stamp<ActorId>,
        live: bool,
    ) {
        let mut pending = Pending {
            cause,
            effect,
            state: State::Counted,
        };
        if live {
            self.submit(&id, &mut pending);
        }
        self.pending.insert(id, pending);
    }

    fn submit(&mut self, id: &EffectId, pending: &mut Pending) {
        if matches!(pending.effect, PeerEffect::Receive { .. }) {
            pending.state = State::Receiving;
            return;
        }
        pending.state = State::Answered;
        let result = self.execute(&pending.effect);
        self.settled
            .push_back(EffectOutcome::new(id.clone(), result));
    }

    fn execute(&mut self, effect: &PeerEffect) -> Result<OutcomePayload, EffectFailure> {
        let named = match effect {
            PeerEffect::Discover { request, .. } => Some(request.adapter()),
            PeerEffect::Bind { request, .. } => Some(request.adapter()),
            PeerEffect::Send { request, .. } => Some(request.target().adapter()),
            PeerEffect::Unbind { .. } | PeerEffect::Receive { .. } => None,
        };
        if named.is_some_and(|name| name != self.adapter.name()) {
            return Err(EffectFailure::Peer(PeerFailureKind::WrongAdapter));
        }
        match effect {
            PeerEffect::Discover { request, .. } => self
                .adapter
                .discover(request.clone())
                .map(OutcomePayload::PeerSnapshot),
            PeerEffect::Bind { request, .. } => self
                .adapter
                .bind(request.clone())
                .map(OutcomePayload::PeerBinding),
            PeerEffect::Send { request, .. } => self
                .adapter
                .send(request.clone())
                .map(OutcomePayload::SubmissionReceipt),
            PeerEffect::Unbind { binding, .. } => self
                .adapter
                .unbind(binding)
                .map(OutcomePayload::UnbindReceipt),
            PeerEffect::Receive { .. } => unreachable!("a receive waits for its envelope"),
        }
        .map_err(|failure| EffectFailure::Peer(failure.kind()))
    }

    pub(crate) fn recorded(&self, id: &EffectId) -> Option<(Stamp<ActorId>, Option<EffectTerm>)> {
        self.pending.get(id).map(|pending| {
            (
                pending.cause.clone(),
                Some(EffectTerm::Peer(PeerEffectTerm::from(&pending.effect))),
            )
        })
    }

    pub(crate) fn answered(&mut self, id: &EffectId) {
        if let Some(pending) = self.pending.get_mut(id) {
            pending.state = State::Answered;
        }
    }

    pub(crate) fn settle(&mut self, id: &EffectId) {
        self.pending.remove(id);
    }

    pub(crate) fn acknowledge(&mut self, id: &EffectId, index: ArrivalIndex) {
        let Some(envelope) = self.handed.remove(id) else {
            return;
        };
        let committed = match circular_runtime::commit_peer_event(&mut RecordedOutcome, envelope) {
            Ok(committed) => committed,
            Err(error) => {
                eprintln!(
                    "circular-kernel: peer receive {id:?} at arrival {index:?} has no receipt: {error:?}"
                );
                return;
            }
        };
        if let Err(error) = self.adapter.acknowledge_receive(committed) {
            eprintln!(
                "circular-kernel: peer receive {id:?} at arrival {index:?} was recorded but the adapter refused its acknowledgement: {:?}",
                error.reason()
            );
        }
    }

    pub(crate) fn awaiting(&self) -> bool {
        !self.settled.is_empty()
            || (!self.paused
                && self
                    .pending
                    .values()
                    .any(|pending| matches!(pending.state, State::Receiving)))
    }

    pub(crate) fn poll(&mut self, context: &mut Context<'_>) -> Poll<EffectOutcome<EffectId>> {
        if let Some(outcome) = self.settled.pop_front() {
            return Poll::Ready(outcome);
        }
        if self.paused {
            return Poll::Pending;
        }
        self.adapter.set_receive_waker(context.waker().clone());
        for (id, pending) in &mut self.pending {
            let (State::Receiving, PeerEffect::Receive { binding, after, .. }) =
                (&pending.state, &pending.effect)
            else {
                continue;
            };
            let result = match self.adapter.receive(binding, *after) {
                Ok(stream) => match stream.events().first() {
                    None => continue,
                    Some(envelope)
                        if envelope.binding() != binding
                            || after.is_some_and(|cursor| envelope.cursor() <= cursor) =>
                    {
                        eprintln!(
                            "circular-kernel: peer receive {id:?} got an envelope outside its request"
                        );
                        Err(EffectFailure::InterpreterFault(InterpreterFault::Other))
                    }
                    Some(envelope) => {
                        self.handed.insert(id.clone(), envelope.clone());
                        Ok(OutcomePayload::PeerEnvelope(envelope.clone()))
                    }
                },
                Err(failure) => Err(EffectFailure::Peer(failure.kind())),
            };
            pending.state = State::Answered;
            return Poll::Ready(EffectOutcome::new(id.clone(), result));
        }
        Poll::Pending
    }

    pub(crate) fn pause(&mut self) {
        self.paused = true;
    }

    pub(crate) fn resume(&mut self) {
        self.paused = false;
    }

    pub(crate) fn interrupted(&mut self) -> Vec<EffectOutcome<EffectId>> {
        let mut interrupted = Vec::new();
        for (id, pending) in &mut self.pending {
            if matches!(pending.state, State::Counted) {
                pending.state = State::Answered;
                interrupted.push(EffectOutcome::new(
                    id.clone(),
                    Err(EffectFailure::InterpreterFault(
                        InterpreterFault::Interrupted,
                    )),
                ));
            }
        }
        interrupted
    }
}
