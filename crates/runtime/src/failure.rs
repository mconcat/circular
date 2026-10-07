
use crate::EffectFailure;
use circular_core::{NonZeroTicks, ProducerIdentity, Stamp, Tick, Ticks};
use circular_plan::{ActorId, EdgeId, PortId, ScopeId};
use std::collections::BTreeMap;
use std::num::NonZeroUsize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProcessingCause {
    EffectFailed(EffectFailure),
    OutcomeUnclaimed,
    InputOutOfDomain,
    DomainRejected,
    Poisoned,
    ApprovalRequired,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreprocessFailurePoint {
    pub edge: EdgeId,
    pub index: usize,
    pub kind: circular_plan::PreprocessKind,
    pub code: Option<String>,
}

#[must_use]
pub const fn classify_effect_failure(failure: EffectFailure) -> ProcessingCause {
    match failure {
        EffectFailure::ApprovalRequired => ProcessingCause::ApprovalRequired,
        other => ProcessingCause::EffectFailed(other),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeadLetterReason {
    Processing(ProcessingCause),
    OutcomeUnclaimed,
    DestinationGone,
    Poisoned,
    ActorDeclared(crate::reason::DeclaredReason),
    /// The destination mailbox already holds its declared wire capacity.
    Capacity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeadLetterOrigin {
    actor: ActorId,
    port: Option<PortId>,
}

impl DeadLetterOrigin {
    #[must_use]
    pub const fn new(actor: ActorId, port: Option<PortId>) -> Self {
        Self { actor, port }
    }

    #[must_use]
    pub const fn actor(&self) -> &ActorId {
        &self.actor
    }

    #[must_use]
    pub const fn port(&self) -> Option<&PortId> {
        self.port.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeadLetterTarget {
    Delivery(EdgeId),
    Outlet { actor: ActorId, port: PortId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeadLetterRecord<P: ProducerIdentity, D> {
    subject: D,
    origin: DeadLetterOrigin,
    reason: DeadLetterReason,
    admitted: Stamp<P>,
    target: Option<DeadLetterTarget>,
    failure_point: Option<PreprocessFailurePoint>,
}

impl<P: ProducerIdentity, D> DeadLetterRecord<P, D> {
    #[must_use]
    pub const fn new(
        subject: D,
        origin: DeadLetterOrigin,
        reason: DeadLetterReason,
        admitted: Stamp<P>,
    ) -> Self {
        Self {
            subject,
            origin,
            reason,
            admitted,
            target: None,
            failure_point: None,
        }
    }

    #[must_use]
    pub fn with_target(mut self, target: Option<DeadLetterTarget>) -> Self {
        self.target = target;
        self
    }

    #[must_use]
    pub fn with_failure_point(mut self, point: Option<PreprocessFailurePoint>) -> Self {
        self.failure_point = point;
        self
    }

    #[must_use]
    pub const fn failure_point(&self) -> Option<&PreprocessFailurePoint> {
        self.failure_point.as_ref()
    }

    #[must_use]
    pub const fn target(&self) -> Option<&DeadLetterTarget> {
        self.target.as_ref()
    }

    #[must_use]
    pub const fn subject(&self) -> &D {
        &self.subject
    }

    #[must_use]
    pub const fn origin(&self) -> &DeadLetterOrigin {
        &self.origin
    }

    #[must_use]
    pub const fn reason(&self) -> &DeadLetterReason {
        &self.reason
    }

    #[must_use]
    pub const fn admitted(&self) -> &Stamp<P> {
        &self.admitted
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeadLetterLane<P: ProducerIdentity, D> {
    scope: ScopeId,
    records: Vec<DeadLetterRecord<P, D>>,
}

impl<P: ProducerIdentity, D> DeadLetterLane<P, D> {
    #[must_use]
    pub const fn new(scope: ScopeId) -> Self {
        Self {
            scope,
            records: Vec::new(),
        }
    }

    #[must_use]
    pub const fn scope(&self) -> &ScopeId {
        &self.scope
    }

    pub fn write(&mut self, record: DeadLetterRecord<P, D>) {
        self.records.push(record);
    }

    #[must_use]
    pub fn records(&self) -> &[DeadLetterRecord<P, D>] {
        &self.records
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeadLetterLaneActivationError {
    AlreadyActive(ScopeId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeadLetterLanes<P: ProducerIdentity, D> {
    lanes: BTreeMap<ScopeId, DeadLetterLane<P, D>>,
}

impl<P: ProducerIdentity, D> DeadLetterLanes<P, D> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            lanes: BTreeMap::new(),
        }
    }

    pub fn activate(&mut self, scope: ScopeId) -> Result<(), DeadLetterLaneActivationError> {
        if self.lanes.contains_key(&scope) {
            return Err(DeadLetterLaneActivationError::AlreadyActive(scope));
        }
        self.lanes.insert(scope.clone(), DeadLetterLane::new(scope));
        Ok(())
    }

    #[must_use]
    pub fn lane(&self, scope: &ScopeId) -> Option<&DeadLetterLane<P, D>> {
        self.lanes.get(scope)
    }

    #[must_use]
    pub fn lane_mut(&mut self, scope: &ScopeId) -> Option<&mut DeadLetterLane<P, D>> {
        self.lanes.get_mut(scope)
    }

    /// Recorded failures outlive the actor that owned their scope.
    pub fn lanes(&self) -> impl Iterator<Item = &DeadLetterLane<P, D>> {
        self.lanes.values()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.lanes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lanes.is_empty()
    }
}

impl<P: ProducerIdentity, D> Default for DeadLetterLanes<P, D> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessingFailureEmission<P: ProducerIdentity, D> {
    subject: D,
    origin: DeadLetterOrigin,
    cause: ProcessingCause,
    admitted: Stamp<P>,
}

impl<P: ProducerIdentity, D> ProcessingFailureEmission<P, D> {
    #[must_use]
    pub const fn subject(&self) -> &D {
        &self.subject
    }

    #[must_use]
    pub const fn origin(&self) -> &DeadLetterOrigin {
        &self.origin
    }

    #[must_use]
    pub const fn cause(&self) -> &ProcessingCause {
        &self.cause
    }

    #[must_use]
    pub const fn admitted(&self) -> &Stamp<P> {
        &self.admitted
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProcessingFailureRoute<P: ProducerIdentity, D> {
    ErrorEdges {
        destinations: NonZeroUsize,
        emission: ProcessingFailureEmission<P, D>,
    },
    DeadLetter,
}

pub fn route_processing_failure<P: ProducerIdentity, D>(
    lane: &mut DeadLetterLane<P, D>,
    subject: D,
    origin_actor: ActorId,
    error_port: Option<PortId>,
    wired_error_edges: usize,
    cause: ProcessingCause,
    admitted: Stamp<P>,
) -> ProcessingFailureRoute<P, D> {
    if let Some(destinations) = NonZeroUsize::new(wired_error_edges) {
        ProcessingFailureRoute::ErrorEdges {
            destinations,
            emission: ProcessingFailureEmission {
                subject,
                origin: DeadLetterOrigin::new(origin_actor, error_port),
                cause,
                admitted,
            },
        }
    } else {
        lane.write(processing_letter(
            subject,
            origin_actor,
            error_port,
            cause,
            admitted,
        ));
        ProcessingFailureRoute::DeadLetter
    }
}

#[must_use]
pub fn processing_letter<P: ProducerIdentity, D>(
    subject: D,
    origin_actor: ActorId,
    error_port: Option<PortId>,
    cause: ProcessingCause,
    admitted: Stamp<P>,
) -> DeadLetterRecord<P, D> {
    let target = error_port.as_ref().map(|port| DeadLetterTarget::Outlet {
        actor: origin_actor.clone(),
        port: port.clone(),
    });
    DeadLetterRecord::new(
        subject,
        DeadLetterOrigin::new(origin_actor, error_port),
        DeadLetterReason::Processing(cause),
        admitted,
    )
    .with_target(target)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestartDisposition {
    Restart { remaining: usize },
    Abandon { next_recovery: Tick },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestartBudgetError {
    TickRegressed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestartParameters {
    capacity: NonZeroUsize,
    refill_interval: NonZeroTicks,
}

impl RestartParameters {
    #[must_use]
    pub const fn new(capacity: NonZeroUsize, refill_interval: NonZeroTicks) -> Self {
        Self {
            capacity,
            refill_interval,
        }
    }

    #[must_use]
    pub const fn capacity(self) -> NonZeroUsize {
        self.capacity
    }

    #[must_use]
    pub const fn refill_interval(self) -> NonZeroTicks {
        self.refill_interval
    }

    #[must_use]
    pub const fn start(self, at: Tick) -> RestartBudget {
        RestartBudget::new(self.capacity, self.refill_interval, at)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestartBudget {
    capacity: NonZeroUsize,
    refill_interval: NonZeroTicks,
    balance: usize,
    last_tick: Tick,
}

impl RestartBudget {
    #[must_use]
    pub const fn new(capacity: NonZeroUsize, refill_interval: NonZeroTicks, at: Tick) -> Self {
        Self {
            capacity,
            refill_interval,
            balance: capacity.get(),
            last_tick: at,
        }
    }

    #[must_use]
    pub const fn parameters(&self) -> RestartParameters {
        RestartParameters::new(self.capacity, self.refill_interval)
    }

    #[must_use]
    pub const fn capacity(&self) -> NonZeroUsize {
        self.capacity
    }

    #[must_use]
    pub const fn refill_interval(&self) -> NonZeroTicks {
        self.refill_interval
    }

    #[must_use]
    pub const fn balance(&self) -> usize {
        self.balance
    }

    #[must_use]
    pub const fn last_tick(&self) -> Tick {
        self.last_tick
    }

    pub fn advance_to(&mut self, at: Tick) -> Result<(), RestartBudgetError> {
        let elapsed = at
            .duration_since(self.last_tick)
            .ok_or(RestartBudgetError::TickRegressed)?;
        let interval = self.refill_interval.get().get();
        let refills = elapsed.get() / interval;
        if refills == 0 {
            return Ok(());
        }
        let credited = usize::try_from(refills).unwrap_or(usize::MAX);
        self.balance = self
            .capacity
            .get()
            .min(self.balance.saturating_add(credited));
        let consumed_ticks = refills.saturating_mul(interval);
        self.last_tick = self
            .last_tick
            .checked_add(Ticks::new(consumed_ticks))
            .unwrap_or(Tick::MAX);
        Ok(())
    }

    pub fn try_consume(&mut self, at: Tick) -> Result<RestartDisposition, RestartBudgetError> {
        self.advance_to(at)?;
        if self.balance > 0 {
            self.balance -= 1;
            Ok(RestartDisposition::Restart {
                remaining: self.balance,
            })
        } else {
            let next_recovery = self
                .last_tick
                .checked_add(self.refill_interval.get())
                .unwrap_or(Tick::MAX);
            Ok(RestartDisposition::Abandon { next_recovery })
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IncarnationState {
    Prepared,
    Active,
    Draining,
    Terminated,
    Abandoned,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestartableActorFailure {
    Panicked,
    ConstructionFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IncarnationTransition {
    Activated {
        incarnation: u64,
    },
    Restarted {
        retired: u64,
        prepared: u64,
        remaining: usize,
    },
    ConfigRestarted {
        retired: u64,
        prepared: u64,
    },
    Abandoned {
        incarnation: u64,
        next_recovery: Tick,
    },
    ResumeDenied {
        incarnation: u64,
        next_recovery: Tick,
    },
    Draining {
        incarnation: u64,
    },
    Terminated {
        incarnation: u64,
    },
    ConfigApplied {
        incarnation: u64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IncarnationTransitionError {
    InvalidState { state: IncarnationState },
    MissingPreparedConfig,
    TickRegressed,
}

impl From<RestartBudgetError> for IncarnationTransitionError {
    fn from(_: RestartBudgetError) -> Self {
        Self::TickRegressed
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestartController {
    incarnation: u64,
    state: IncarnationState,
    budget: RestartBudget,
}

impl RestartController {
    #[must_use]
    pub const fn new(budget: RestartBudget) -> Self {
        Self::at_incarnation(budget, 0)
    }

    #[must_use]
    pub const fn at_incarnation(budget: RestartBudget, incarnation: u64) -> Self {
        Self {
            incarnation,
            state: IncarnationState::Prepared,
            budget,
        }
    }

    #[must_use]
    pub const fn incarnation(&self) -> u64 {
        self.incarnation
    }

    #[must_use]
    pub const fn state(&self) -> IncarnationState {
        self.state
    }

    #[must_use]
    pub const fn budget(&self) -> &RestartBudget {
        &self.budget
    }

    pub fn advance_to(&mut self, at: Tick) -> Result<(), IncarnationTransitionError> {
        self.budget.advance_to(at).map_err(Into::into)
    }

    pub fn activate(&mut self) -> Result<IncarnationTransition, IncarnationTransitionError> {
        if self.state != IncarnationState::Prepared {
            return Err(IncarnationTransitionError::InvalidState { state: self.state });
        }
        self.state = IncarnationState::Active;
        Ok(IncarnationTransition::Activated {
            incarnation: self.incarnation,
        })
    }

    pub fn actor_failed(
        &mut self,
        at: Tick,
        failure: RestartableActorFailure,
    ) -> Result<IncarnationTransition, IncarnationTransitionError> {
        let valid = match failure {
            RestartableActorFailure::Panicked => self.state == IncarnationState::Active,
            RestartableActorFailure::ConstructionFailed => {
                matches!(
                    self.state,
                    IncarnationState::Prepared | IncarnationState::Active
                )
            }
        };
        if !valid {
            return Err(IncarnationTransitionError::InvalidState { state: self.state });
        }
        self.restart_with_budget(at, false)
    }

    pub fn resume(
        &mut self,
        at: Tick,
    ) -> Result<IncarnationTransition, IncarnationTransitionError> {
        if self.state != IncarnationState::Abandoned {
            return Err(IncarnationTransitionError::InvalidState { state: self.state });
        }
        self.restart_with_budget(at, true)
    }

    pub fn restart_for_config(
        &mut self,
    ) -> Result<IncarnationTransition, IncarnationTransitionError> {
        if self.state != IncarnationState::Active {
            return Err(IncarnationTransitionError::InvalidState { state: self.state });
        }
        let retired = self.incarnation;
        self.incarnation = next_incarnation(self.incarnation);
        self.state = IncarnationState::Prepared;
        Ok(IncarnationTransition::ConfigRestarted {
            retired,
            prepared: self.incarnation,
        })
    }

    pub fn cancel(&mut self) -> Result<IncarnationTransition, IncarnationTransitionError> {
        if self.state != IncarnationState::Active {
            return Err(IncarnationTransitionError::InvalidState { state: self.state });
        }
        self.state = IncarnationState::Draining;
        Ok(IncarnationTransition::Draining {
            incarnation: self.incarnation,
        })
    }

    /// Explicitly discard an incarnation that cannot enter `Draining` because
    /// it is not active. This is the stop half of v1 stop-and-redeploy for
    /// never-activated `Prepared` and terminally failed `Abandoned` instances.
    pub fn discard_inactive(
        &mut self,
    ) -> Result<IncarnationTransition, IncarnationTransitionError> {
        if !matches!(
            self.state,
            IncarnationState::Prepared | IncarnationState::Abandoned
        ) {
            return Err(IncarnationTransitionError::InvalidState { state: self.state });
        }
        self.state = IncarnationState::Terminated;
        Ok(IncarnationTransition::Terminated {
            incarnation: self.incarnation,
        })
    }

    pub fn finish_draining(&mut self) -> Result<IncarnationTransition, IncarnationTransitionError> {
        if self.state != IncarnationState::Draining {
            return Err(IncarnationTransitionError::InvalidState { state: self.state });
        }
        self.state = IncarnationState::Terminated;
        Ok(IncarnationTransition::Terminated {
            incarnation: self.incarnation,
        })
    }

    fn restart_with_budget(
        &mut self,
        at: Tick,
        explicit_resume: bool,
    ) -> Result<IncarnationTransition, IncarnationTransitionError> {
        match self.budget.try_consume(at)? {
            RestartDisposition::Restart { remaining } => {
                let retired = self.incarnation;
                self.incarnation = next_incarnation(self.incarnation);
                self.state = IncarnationState::Prepared;
                Ok(IncarnationTransition::Restarted {
                    retired,
                    prepared: self.incarnation,
                    remaining,
                })
            }
            RestartDisposition::Abandon { next_recovery } if explicit_resume => {
                Ok(IncarnationTransition::ResumeDenied {
                    incarnation: self.incarnation,
                    next_recovery,
                })
            }
            RestartDisposition::Abandon { next_recovery } => {
                self.state = IncarnationState::Abandoned;
                Ok(IncarnationTransition::Abandoned {
                    incarnation: self.incarnation,
                    next_recovery,
                })
            }
        }
    }
}

fn next_incarnation(current: u64) -> u64 {
    current.checked_add(1).expect(
        "under a pipeline's event-rate bound, the incarnation generation does not exhaust u64",
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn two_declared_reasons_stay_distinct_in_the_record() {
        let vocabulary = crate::ReasonDecl::<crate::DeadLettering>::try_from_names([
            "transform_failed",
            "output_shape_unresolved",
        ])
        .expect("two names");

        let one = super::DeadLetterReason::ActorDeclared(
            vocabulary.resolve("transform_failed").expect("declared"),
        );
        let other = super::DeadLetterReason::ActorDeclared(
            vocabulary
                .resolve("output_shape_unresolved")
                .expect("declared"),
        );

        assert_ne!(one, other, "two declared reasons folded into one value");
        let super::DeadLetterReason::ActorDeclared(carried) = &one else {
            panic!("it is an actor-declared reason");
        };
        assert_eq!(
            carried.name(),
            "transform_failed",
            "the name did not reach the record"
        );
    }

    use super::*;

    #[test]
    fn approval_required_keeps_its_processing_cause_instead_of_becoming_effect_failed() {
        assert_eq!(
            classify_effect_failure(EffectFailure::ApprovalRequired),
            ProcessingCause::ApprovalRequired
        );
        assert_eq!(
            classify_effect_failure(EffectFailure::EndpointGone),
            ProcessingCause::EffectFailed(EffectFailure::EndpointGone)
        );
    }

    #[test]
    fn prepared_and_abandoned_can_be_explicitly_discarded_but_active_must_drain() {
        let parameters = RestartParameters::new(
            std::num::NonZeroUsize::new(1).expect("test capacity is nonzero"),
            circular_core::NonZeroTicks::new(17).expect("test interval is nonzero"),
        );

        let mut prepared = RestartController::new(parameters.start(Tick::ZERO));
        assert_eq!(
            prepared.discard_inactive(),
            Ok(IncarnationTransition::Terminated { incarnation: 0 })
        );
        assert_eq!(prepared.state(), IncarnationState::Terminated);

        let mut abandoned = RestartController::new(parameters.start(Tick::ZERO));
        assert!(matches!(
            abandoned.actor_failed(Tick::ZERO, RestartableActorFailure::ConstructionFailed),
            Ok(IncarnationTransition::Restarted { prepared: 1, .. })
        ));
        assert!(matches!(
            abandoned.actor_failed(Tick::ZERO, RestartableActorFailure::ConstructionFailed),
            Ok(IncarnationTransition::Abandoned { incarnation: 1, .. })
        ));
        assert_eq!(
            abandoned.discard_inactive(),
            Ok(IncarnationTransition::Terminated { incarnation: 1 })
        );

        let mut active = RestartController::new(parameters.start(Tick::ZERO));
        active.activate().expect("control activates");
        assert_eq!(
            active.discard_inactive(),
            Err(IncarnationTransitionError::InvalidState {
                state: IncarnationState::Active,
            })
        );
    }
}
