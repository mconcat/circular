
use circular_runtime::{
    ApprovalConsumeDisposition, ApprovalConsumeRejection, ApprovalDecision,
    ApprovalDecisionDisposition, ApprovalGateDisposition, ApprovalMachine, ApprovalRequestOutcome,
    ApprovalState, ApprovalTicket, OpenApprovalState,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalRequest<I, S> {
    key: I,
    summary: S,
    target: I,
}

impl<I, S> ApprovalRequest<I, S> {
    #[must_use]
    pub const fn new(key: I, summary: S, target: I) -> Self {
        Self {
            key,
            summary,
            target,
        }
    }

    #[must_use]
    pub const fn key(&self) -> &I {
        &self.key
    }

    #[must_use]
    pub const fn summary(&self) -> &S {
        &self.summary
    }

    #[must_use]
    pub const fn target(&self) -> &I {
        &self.target
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalRow<I, S> {
    request: ApprovalRequest<I, S>,
    state: OpenApprovalState<I>,
}

impl<I, S> ApprovalRow<I, S> {
    #[must_use]
    pub const fn request(&self) -> &ApprovalRequest<I, S> {
        &self.request
    }

    #[must_use]
    pub const fn state(&self) -> &OpenApprovalState<I> {
        &self.state
    }
}

circular_core::closed_table! {
    pub enum ApprovalTerminal: u64 {
        Denied = 1,
        Consumed = 3,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalTerminalObservation<I> {
    key: I,
    target: I,
    terminal: ApprovalTerminal,
}

impl<I> ApprovalTerminalObservation<I> {
    #[must_use]
    pub const fn key(&self) -> &I {
        &self.key
    }

    #[must_use]
    pub const fn target(&self) -> &I {
        &self.target
    }

    #[must_use]
    pub const fn terminal(&self) -> ApprovalTerminal {
        self.terminal
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalLedgerDiagnostic<I> {
    ConflictingRequest {
        key: I,
    },
    ActiveTargetConflict {
        target: I,
    },
    DecisionRejected {
        key: I,
    },
    TicketRejected {
        key: I,
        reason: ApprovalConsumeRejection,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalRequestDisposition {
    Inserted,
    ExactDuplicate,
    Conflict,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalLedgerDecision<I> {
    Applied(ApprovalRequestOutcome<I>),
    Rejected,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalLedgerSnapshot<I, S> {
    rows: BTreeMap<I, ApprovalRow<I, S>>,
}

impl<I, S> ApprovalLedgerSnapshot<I, S> {
    #[must_use]
    pub const fn rows(&self) -> &BTreeMap<I, ApprovalRow<I, S>> {
        &self.rows
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalLedger<I, S> {
    rows: BTreeMap<I, ApprovalRow<I, S>>,
    decision_arrivals: Vec<(I, ApprovalDecision)>,
    terminal_observations: Vec<ApprovalTerminalObservation<I>>,
    diagnostics: Vec<ApprovalLedgerDiagnostic<I>>,
}

impl<I, S> Default for ApprovalLedger<I, S> {
    fn default() -> Self {
        Self {
            rows: BTreeMap::new(),
            decision_arrivals: Vec::new(),
            terminal_observations: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

impl<I: Clone + Ord, S: Clone + Eq> ApprovalLedger<I, S> {
    #[must_use]
    pub const fn rows(&self) -> &BTreeMap<I, ApprovalRow<I, S>> {
        &self.rows
    }

    #[must_use]
    pub fn row(&self, key: &I) -> Option<&ApprovalRow<I, S>> {
        self.rows.get(key)
    }

    #[must_use]
    pub fn decision_arrivals(&self) -> &[(I, ApprovalDecision)] {
        &self.decision_arrivals
    }

    #[must_use]
    pub fn terminal_observations(&self) -> &[ApprovalTerminalObservation<I>] {
        &self.terminal_observations
    }

    #[must_use]
    pub fn diagnostics(&self) -> &[ApprovalLedgerDiagnostic<I>] {
        &self.diagnostics
    }

    pub fn request(&mut self, request: ApprovalRequest<I, S>) -> ApprovalRequestDisposition {
        if let Some(existing) = self.rows.get(request.key()) {
            if existing.request == request {
                return ApprovalRequestDisposition::ExactDuplicate;
            }
            self.diagnostics
                .push(ApprovalLedgerDiagnostic::ConflictingRequest {
                    key: request.key.clone(),
                });
            return ApprovalRequestDisposition::Conflict;
        }
        if self
            .rows
            .values()
            .any(|row| row.request.target == request.target)
        {
            self.diagnostics
                .push(ApprovalLedgerDiagnostic::ActiveTargetConflict {
                    target: request.target.clone(),
                });
            return ApprovalRequestDisposition::Conflict;
        }
        self.rows.insert(
            request.key.clone(),
            ApprovalRow {
                request,
                state: OpenApprovalState::Requested,
            },
        );
        ApprovalRequestDisposition::Inserted
    }

    pub fn decide(&mut self, key: &I, decision: ApprovalDecision) -> ApprovalLedgerDecision<I> {
        self.decision_arrivals.push((key.clone(), decision));
        let Some(row) = self.rows.get_mut(key) else {
            self.diagnostics
                .push(ApprovalLedgerDiagnostic::DecisionRejected { key: key.clone() });
            return ApprovalLedgerDecision::Unknown;
        };
        let mut machine = ApprovalMachine::restore(
            row.request.key.clone(),
            row.request.target.clone(),
            row.state.clone(),
        );
        match machine.decide(decision) {
            ApprovalDecisionDisposition::Applied(ApprovalState::Approved(ticket)) => {
                row.state = OpenApprovalState::Approved(ticket.clone());
                ApprovalLedgerDecision::Applied(ApprovalRequestOutcome::Approved(ticket))
            }
            ApprovalDecisionDisposition::Applied(ApprovalState::Denied) => {
                let row = self
                    .rows
                    .remove(key)
                    .expect("the approval row just looked up");
                self.terminal_observations
                    .push(ApprovalTerminalObservation {
                        key: row.request.key,
                        target: row.request.target,
                        terminal: ApprovalTerminal::Denied,
                    });
                ApprovalLedgerDecision::Applied(ApprovalRequestOutcome::Denied)
            }
            ApprovalDecisionDisposition::Ignored(_) => {
                self.diagnostics
                    .push(ApprovalLedgerDiagnostic::DecisionRejected { key: key.clone() });
                ApprovalLedgerDecision::Rejected
            }
            ApprovalDecisionDisposition::Applied(_) => {
                unreachable!("closed transition of the Requested decision")
            }
        }
    }

    pub fn consume(
        &mut self,
        target: &I,
        ticket: &ApprovalTicket<I>,
    ) -> ApprovalConsumeDisposition {
        let key = ticket.ledger_item().clone();
        let Some(row) = self.rows.get_mut(&key) else {
            self.diagnostics
                .push(ApprovalLedgerDiagnostic::TicketRejected {
                    key,
                    reason: ApprovalConsumeRejection::AlreadyConsumed,
                });
            return ApprovalConsumeDisposition::Rejected(ApprovalConsumeRejection::AlreadyConsumed);
        };
        let mut machine = ApprovalMachine::restore(
            row.request.key.clone(),
            row.request.target.clone(),
            row.state.clone(),
        );
        match machine.consume(target, ticket) {
            ApprovalConsumeDisposition::Consumed => {
                let row = self
                    .rows
                    .remove(&key)
                    .expect("the approval row just looked up");
                self.terminal_observations
                    .push(ApprovalTerminalObservation {
                        key: row.request.key,
                        target: row.request.target,
                        terminal: ApprovalTerminal::Consumed,
                    });
                ApprovalConsumeDisposition::Consumed
            }
            ApprovalConsumeDisposition::Rejected(reason) => {
                self.diagnostics
                    .push(ApprovalLedgerDiagnostic::TicketRejected { key, reason });
                ApprovalConsumeDisposition::Rejected(reason)
            }
        }
    }

    #[must_use]
    pub fn snapshot(&self) -> ApprovalLedgerSnapshot<I, S> {
        ApprovalLedgerSnapshot {
            rows: self.rows.clone(),
        }
    }

    #[must_use]
    pub fn recover(snapshot: ApprovalLedgerSnapshot<I, S>) -> Self {
        Self {
            rows: snapshot.rows,
            decision_arrivals: Vec::new(),
            terminal_observations: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    #[must_use]
    pub fn authorize(
        &mut self,
        target: &I,
        ticket: Option<&ApprovalTicket<I>>,
    ) -> ApprovalGateDisposition {
        let Some(ticket) = ticket else {
            return ApprovalGateDisposition::Required;
        };
        match self.consume(target, ticket) {
            ApprovalConsumeDisposition::Consumed => ApprovalGateDisposition::Authorized,
            ApprovalConsumeDisposition::Rejected(reason) => {
                ApprovalGateDisposition::Rejected(reason)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_runtime::ApprovalState;

    fn request(key: u64, target: u64) -> ApprovalRequest<u64, &'static str> {
        ApprovalRequest::new(key, "fixture", target)
    }

    #[test]
    fn duplicate_requests_and_first_decision_are_state_preserving() {
        let mut ledger = ApprovalLedger::default();
        assert_eq!(
            ledger.request(request(1, 10)),
            ApprovalRequestDisposition::Inserted
        );
        assert_eq!(
            ledger.request(request(1, 10)),
            ApprovalRequestDisposition::ExactDuplicate
        );
        assert_eq!(
            ledger.request(request(2, 10)),
            ApprovalRequestDisposition::Conflict
        );
        let ApprovalLedgerDecision::Applied(ApprovalRequestOutcome::Approved(ticket)) =
            ledger.decide(&1, ApprovalDecision::Approve)
        else {
            panic!("the first decision must issue a ticket");
        };
        let before = ledger.rows.clone();
        assert_eq!(
            ledger.decide(&1, ApprovalDecision::Deny),
            ApprovalLedgerDecision::Rejected
        );
        assert_eq!(ledger.rows, before);
        assert_eq!(
            ledger.consume(&10, &ticket),
            ApprovalConsumeDisposition::Consumed
        );
        assert!(ledger.rows.is_empty());
        assert_eq!(
            ledger.terminal_observations[0].terminal,
            ApprovalTerminal::Consumed
        );
    }

    #[test]
    fn recovery_preserves_requested_and_approved_without_a_new_decision() {
        let mut ledger = ApprovalLedger::default();
        ledger.request(request(1, 10));
        ledger.request(request(2, 20));
        ledger.decide(&2, ApprovalDecision::Approve);
        let snapshot = ledger.snapshot();
        let recovered = ApprovalLedger::recover(snapshot.clone());
        assert_eq!(recovered.rows(), snapshot.rows());
        assert!(recovered.decision_arrivals().is_empty());
        assert!(matches!(
            recovered.row(&1).unwrap().state(),
            OpenApprovalState::Requested
        ));
        assert!(matches!(
            recovered.row(&2).unwrap().state(),
            OpenApprovalState::Approved(_)
        ));
        let _ = ApprovalState::<u64>::Requested;
    }

    #[test]
    fn terminal_settlement_removes_the_row_and_adds_one_observation_together() {
        let mut ledger = ApprovalLedger::default();
        ledger.request(request(1, 10));
        let before_rows = ledger.rows().len();
        let before_observations = ledger.terminal_observations().len();
        assert!(matches!(
            ledger.decide(&1, ApprovalDecision::Deny),
            ApprovalLedgerDecision::Applied(ApprovalRequestOutcome::Denied)
        ));
        assert_eq!(ledger.rows().len(), before_rows - 1);
        assert_eq!(
            ledger.terminal_observations().len(),
            before_observations + 1
        );
        assert_eq!(
            ledger.terminal_observations()[0].terminal(),
            ApprovalTerminal::Denied
        );
    }
}
