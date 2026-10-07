#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalDecision {
    Approve,
    Deny,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalSpec<I = crate::EffectId> {
    target_effect: I,
}

impl<I> ApprovalSpec<I> {
    #[must_use]
    pub const fn new(target_effect: I) -> Self {
        Self { target_effect }
    }

    #[must_use]
    pub const fn target_effect(&self) -> &I {
        &self.target_effect
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ApprovalTicket<I = crate::EffectId> {
    ledger_item: I,
    target_effect: I,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalRequestOutcome<I = crate::EffectId> {
    Approved(ApprovalTicket<I>),
    Denied,
}

impl<I> ApprovalTicket<I> {
    #[must_use]
    pub(crate) const fn restore(ledger_item: I, target_effect: I) -> Self {
        Self {
            ledger_item,
            target_effect,
        }
    }

    #[must_use]
    pub const fn ledger_item(&self) -> &I {
        &self.ledger_item
    }

    #[must_use]
    pub const fn target_effect(&self) -> &I {
        &self.target_effect
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalState<I> {
    Requested,
    Approved(ApprovalTicket<I>),
    Denied,
    Consumed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OpenApprovalState<I> {
    Requested,
    Approved(ApprovalTicket<I>),
}

impl<I: Clone> OpenApprovalState<I> {
    #[must_use]
    pub fn as_runtime_state(&self) -> ApprovalState<I> {
        match self {
            Self::Requested => ApprovalState::Requested,
            Self::Approved(ticket) => ApprovalState::Approved(ticket.clone()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalDecisionDisposition<I> {
    Applied(ApprovalState<I>),
    Ignored(ApprovalState<I>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalConsumeRejection {
    NotApproved,
    WrongLedgerItem,
    WrongTargetEffect,
    AlreadyConsumed,
    Terminal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalConsumeDisposition {
    Consumed,
    Rejected(ApprovalConsumeRejection),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalMachine<I> {
    ledger_item: I,
    target_effect: I,
    state: ApprovalState<I>,
}

impl<I: Clone + Eq> ApprovalMachine<I> {
    #[must_use]
    pub const fn requested(ledger_item: I, target_effect: I) -> Self {
        Self {
            ledger_item,
            target_effect,
            state: ApprovalState::Requested,
        }
    }

    #[must_use]
    pub fn restore(ledger_item: I, target_effect: I, state: OpenApprovalState<I>) -> Self {
        Self {
            ledger_item,
            target_effect,
            state: state.as_runtime_state(),
        }
    }

    #[must_use]
    pub const fn ledger_item(&self) -> &I {
        &self.ledger_item
    }

    #[must_use]
    pub const fn target_effect(&self) -> &I {
        &self.target_effect
    }

    #[must_use]
    pub const fn state(&self) -> &ApprovalState<I> {
        &self.state
    }

    #[must_use]
    pub fn open_state(&self) -> Option<OpenApprovalState<I>> {
        match &self.state {
            ApprovalState::Requested => Some(OpenApprovalState::Requested),
            ApprovalState::Approved(ticket) => Some(OpenApprovalState::Approved(ticket.clone())),
            ApprovalState::Denied | ApprovalState::Consumed => None,
        }
    }

    pub fn decide(&mut self, decision: ApprovalDecision) -> ApprovalDecisionDisposition<I> {
        if self.state != ApprovalState::Requested {
            return ApprovalDecisionDisposition::Ignored(self.state.clone());
        }
        self.state = match decision {
            ApprovalDecision::Approve => ApprovalState::Approved(ApprovalTicket {
                ledger_item: self.ledger_item.clone(),
                target_effect: self.target_effect.clone(),
            }),
            ApprovalDecision::Deny => ApprovalState::Denied,
        };
        ApprovalDecisionDisposition::Applied(self.state.clone())
    }

    pub fn consume(
        &mut self,
        target_effect: &I,
        ticket: &ApprovalTicket<I>,
    ) -> ApprovalConsumeDisposition {
        match &self.state {
            ApprovalState::Requested => {
                ApprovalConsumeDisposition::Rejected(ApprovalConsumeRejection::NotApproved)
            }
            ApprovalState::Approved(expected) => {
                if ticket.ledger_item() != &self.ledger_item {
                    return ApprovalConsumeDisposition::Rejected(
                        ApprovalConsumeRejection::WrongLedgerItem,
                    );
                }
                if ticket.target_effect() != target_effect
                    || target_effect != &self.target_effect
                    || ticket != expected
                {
                    return ApprovalConsumeDisposition::Rejected(
                        ApprovalConsumeRejection::WrongTargetEffect,
                    );
                }
                self.state = ApprovalState::Consumed;
                ApprovalConsumeDisposition::Consumed
            }
            ApprovalState::Consumed => {
                ApprovalConsumeDisposition::Rejected(ApprovalConsumeRejection::AlreadyConsumed)
            }
            ApprovalState::Denied => {
                ApprovalConsumeDisposition::Rejected(ApprovalConsumeRejection::Terminal)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_is_bound_to_one_item_and_target_and_consumed_once() {
        let mut machine = ApprovalMachine::requested(10_u64, 20_u64);
        let ApprovalDecisionDisposition::Applied(ApprovalState::Approved(ticket)) =
            machine.decide(ApprovalDecision::Approve)
        else {
            panic!("the first approval must issue a ticket");
        };
        let before = machine.clone();
        assert_eq!(
            machine.consume(&21, &ticket),
            ApprovalConsumeDisposition::Rejected(ApprovalConsumeRejection::WrongTargetEffect)
        );
        assert_eq!(machine, before);
        assert_eq!(
            machine.consume(&20, &ticket),
            ApprovalConsumeDisposition::Consumed
        );
        assert_eq!(
            machine.consume(&20, &ticket),
            ApprovalConsumeDisposition::Rejected(ApprovalConsumeRejection::AlreadyConsumed)
        );
    }

    #[test]
    fn first_decision_wins_and_terminal_states_absorb_later_decisions() {
        let mut machine = ApprovalMachine::requested(1_u8, 2_u8);
        assert!(matches!(
            machine.decide(ApprovalDecision::Deny),
            ApprovalDecisionDisposition::Applied(ApprovalState::Denied)
        ));
        assert_eq!(
            machine.decide(ApprovalDecision::Approve),
            ApprovalDecisionDisposition::Ignored(ApprovalState::Denied)
        );
        assert_eq!(machine.state(), &ApprovalState::Denied);
    }
}
