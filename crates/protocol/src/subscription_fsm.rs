
use crate::{
    DeliveryDiscipline, PositiveCredit, PositiveFrameCount, SubscriptionEndReason,
    SubscriptionTerminationDomain,
};

pub trait CreditBalance: PositiveFrameCount + Clone + Eq {
    fn zero() -> Self;
    fn is_zero(&self) -> bool;
    fn checked_add(&self, amount: &Self) -> Option<Self>;
    fn checked_consume_frame(&self) -> Option<Self>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionTermination<T: SubscriptionTerminationDomain, A, D> {
    reason: SubscriptionEndReason<T>,
    diagnostic: D,
    last_anchor: A,
}

impl<T: SubscriptionTerminationDomain, A, D> SubscriptionTermination<T, A, D> {
    #[must_use]
    pub const fn reason(&self) -> &SubscriptionEndReason<T> {
        &self.reason
    }

    #[must_use]
    pub const fn diagnostic(&self) -> &D {
        &self.diagnostic
    }

    #[must_use]
    pub const fn last_anchor(&self) -> &A {
        &self.last_anchor
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubscriptionPhase<T: SubscriptionTerminationDomain, A, D> {
    Opening,
    Live,
    Starved,
    Closed(SubscriptionTermination<T, A, D>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubscriptionAction {
    Emit,
    Backpressured,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubscriptionTransitionError {
    NotOpen,
    AlreadyOpened,
    Closed,
    CreditForNonCreditDiscipline,
    CreditExhausted,
    ConsumerBehindForNonLossless,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionFsm<T: SubscriptionTerminationDomain, N, A, D> {
    discipline: DeliveryDiscipline,
    credit: N,
    last_anchor: A,
    phase: SubscriptionPhase<T, A, D>,
}

impl<T: SubscriptionTerminationDomain, N, A, D> SubscriptionFsm<T, N, A, D>
where
    N: CreditBalance,
    A: Clone,
{
    #[must_use]
    pub fn new(discipline: DeliveryDiscipline, initial_anchor: A) -> Self {
        Self {
            discipline,
            credit: N::zero(),
            last_anchor: initial_anchor,
            phase: SubscriptionPhase::Opening,
        }
    }

    #[must_use]
    pub const fn discipline(&self) -> DeliveryDiscipline {
        self.discipline
    }

    #[must_use]
    pub const fn phase(&self) -> &SubscriptionPhase<T, A, D> {
        &self.phase
    }

    #[must_use]
    pub const fn credit(&self) -> &N {
        &self.credit
    }

    pub fn opened(&mut self) -> Result<(), SubscriptionTransitionError> {
        match self.phase {
            SubscriptionPhase::Opening => {
                self.phase = if self.discipline == DeliveryDiscipline::Credit {
                    SubscriptionPhase::Starved
                } else {
                    SubscriptionPhase::Live
                };
                Ok(())
            }
            SubscriptionPhase::Live | SubscriptionPhase::Starved => {
                Err(SubscriptionTransitionError::AlreadyOpened)
            }
            SubscriptionPhase::Closed(_) => Err(SubscriptionTransitionError::Closed),
        }
    }

    pub fn grant_credit(
        &mut self,
        credit: &PositiveCredit<N>,
    ) -> Result<(), SubscriptionTransitionError> {
        if self.discipline != DeliveryDiscipline::Credit {
            return Err(SubscriptionTransitionError::CreditForNonCreditDiscipline);
        }
        match self.phase {
            SubscriptionPhase::Opening => return Err(SubscriptionTransitionError::NotOpen),
            SubscriptionPhase::Closed(_) => return Err(SubscriptionTransitionError::Closed),
            SubscriptionPhase::Live | SubscriptionPhase::Starved => {}
        }
        let next = self
            .credit
            .checked_add(credit.frames())
            .ok_or(SubscriptionTransitionError::CreditExhausted)?;
        self.credit = next;
        self.phase = SubscriptionPhase::Live;
        Ok(())
    }

    pub fn observe(
        &mut self,
        anchor: A,
    ) -> Result<SubscriptionAction, SubscriptionTransitionError> {
        match self.phase {
            SubscriptionPhase::Opening => return Err(SubscriptionTransitionError::NotOpen),
            SubscriptionPhase::Closed(_) => return Err(SubscriptionTransitionError::Closed),
            SubscriptionPhase::Live | SubscriptionPhase::Starved => {}
        }
        if self.discipline != DeliveryDiscipline::Credit {
            self.last_anchor = anchor;
            return Ok(SubscriptionAction::Emit);
        }
        let Some(remaining) = self.credit.checked_consume_frame() else {
            self.phase = SubscriptionPhase::Starved;
            return Ok(SubscriptionAction::Backpressured);
        };
        self.credit = remaining;
        self.last_anchor = anchor;
        self.phase = if self.credit.is_zero() {
            SubscriptionPhase::Starved
        } else {
            SubscriptionPhase::Live
        };
        Ok(SubscriptionAction::Emit)
    }

    pub fn end(
        &mut self,
        reason: SubscriptionEndReason<T>,
        diagnostic: D,
    ) -> Result<(), SubscriptionTransitionError> {
        match self.phase {
            SubscriptionPhase::Opening => return Err(SubscriptionTransitionError::NotOpen),
            SubscriptionPhase::Closed(_) => return Err(SubscriptionTransitionError::Closed),
            SubscriptionPhase::Live | SubscriptionPhase::Starved => {}
        }
        if matches!(reason, SubscriptionEndReason::ConsumerBehind)
            && self.discipline != DeliveryDiscipline::Lossless
        {
            return Err(SubscriptionTransitionError::ConsumerBehindForNonLossless);
        }
        self.phase = SubscriptionPhase::Closed(SubscriptionTermination {
            reason,
            diagnostic,
            last_anchor: self.last_anchor.clone(),
        });
        Ok(())
    }

    pub fn unsubscribe(&mut self, diagnostic: D) -> Result<(), SubscriptionTransitionError> {
        self.end(SubscriptionEndReason::ByClient, diagnostic)
    }

    pub fn session_closed(&mut self, diagnostic: D) -> Result<(), SubscriptionTransitionError> {
        self.end(SubscriptionEndReason::SessionClosed, diagnostic)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestTermination;

    impl SubscriptionTerminationDomain for TestTermination {
        type ResetFloorOrCursor = &'static str;
        type StructureCursor = &'static str;
        type AuthoringEnvironment = &'static str;
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Frames(u8);

    impl PositiveFrameCount for Frames {
        fn is_positive(&self) -> bool {
            self.0 > 0
        }
    }

    impl CreditBalance for Frames {
        fn zero() -> Self {
            Self(0)
        }

        fn is_zero(&self) -> bool {
            self.0 == 0
        }

        fn checked_add(&self, amount: &Self) -> Option<Self> {
            self.0.checked_add(amount.0).map(Self)
        }

        fn checked_consume_frame(&self) -> Option<Self> {
            self.0.checked_sub(1).map(Self)
        }
    }

    fn credit(frames: u8) -> PositiveCredit<Frames> {
        PositiveCredit::try_new(Frames(frames)).expect("positive frame count")
    }

    #[test]
    fn credit_transcript_moves_live_starved_live_without_ending() {
        let mut subscription = SubscriptionFsm::<TestTermination, Frames, _, &str>::new(
            DeliveryDiscipline::Credit,
            "anchor-0",
        );
        subscription.opened().expect("opened successfully");
        assert_eq!(subscription.phase(), &SubscriptionPhase::Starved);
        assert_eq!(
            subscription.observe("anchor-1"),
            Ok(SubscriptionAction::Backpressured)
        );
        assert_eq!(subscription.phase(), &SubscriptionPhase::Starved);

        subscription
            .grant_credit(&credit(2))
            .expect("credit accepted");
        assert_eq!(subscription.phase(), &SubscriptionPhase::Live);
        assert_eq!(
            subscription.observe("anchor-1"),
            Ok(SubscriptionAction::Emit)
        );
        assert_eq!(subscription.credit(), &Frames(1));
        assert_eq!(
            subscription.observe("anchor-2"),
            Ok(SubscriptionAction::Emit)
        );
        assert_eq!(subscription.credit(), &Frames(0));
        assert_eq!(subscription.phase(), &SubscriptionPhase::Starved);
        subscription.grant_credit(&credit(1)).expect("live again");
        assert_eq!(subscription.phase(), &SubscriptionPhase::Live);
    }

    #[test]
    fn emitted_frames_never_exceed_accepted_credit() {
        for granted in 1_u8..=8 {
            let mut subscription = SubscriptionFsm::<TestTermination, Frames, _, ()>::new(
                DeliveryDiscipline::Credit,
                0_u8,
            );
            subscription.opened().expect("opened");
            subscription
                .grant_credit(&credit(granted))
                .expect("credit accepted");
            let mut emitted = 0_u8;
            for anchor in 1_u8..=granted.saturating_add(2) {
                if subscription.observe(anchor).expect("open subscription")
                    == SubscriptionAction::Emit
                {
                    emitted += 1;
                }
            }
            assert_eq!(emitted, granted);
            assert_eq!(subscription.credit(), &Frames(0));
        }
    }

    #[test]
    fn termination_is_single_and_blocks_frames_credit_and_second_end() {
        let mut subscription = SubscriptionFsm::<TestTermination, Frames, _, &str>::new(
            DeliveryDiscipline::Credit,
            "anchor-0",
        );
        subscription.opened().expect("opened");
        subscription.grant_credit(&credit(1)).expect("credit");
        subscription.observe("anchor-1").expect("frame");
        subscription
            .unsubscribe("by-client-code")
            .expect("first end");

        let termination = match subscription.phase() {
            SubscriptionPhase::Closed(termination) => termination,
            phase => panic!("unexpected state: {phase:?}"),
        };
        assert_eq!(termination.reason(), &SubscriptionEndReason::ByClient);
        assert_eq!(termination.diagnostic(), &"by-client-code");
        assert_eq!(termination.last_anchor(), &"anchor-1");
        assert_eq!(
            subscription.observe("anchor-2"),
            Err(SubscriptionTransitionError::Closed)
        );
        assert_eq!(
            subscription.grant_credit(&credit(1)),
            Err(SubscriptionTransitionError::Closed)
        );
        assert_eq!(
            subscription.session_closed("second-end"),
            Err(SubscriptionTransitionError::Closed)
        );
    }
}
