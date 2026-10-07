
use circular_plan::{ActorFlags, Delivery, PositiveCapacity, Shed};
use std::fmt;
use std::num::NonZeroUsize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapacityDecision {
    Queue,
    BlockReliable,
    DurableCommitRequired,
    ShedIncoming(Shed),
    ReplaceOldest { shed: Shed, count: NonZeroUsize },
}

impl CapacityDecision {
    #[must_use]
    pub const fn blocks_input(self) -> bool {
        matches!(self, Self::BlockReliable)
    }
}

#[must_use]
pub fn decide_capacity(
    delivery: Delivery,
    capacity: PositiveCapacity,
    outstanding: usize,
) -> CapacityDecision {
    if outstanding < capacity.get() {
        return match delivery {
            Delivery::Durable => CapacityDecision::DurableCommitRequired,
            Delivery::Lossless | Delivery::BestEffort { .. } => CapacityDecision::Queue,
        };
    }

    match delivery {
        Delivery::Lossless | Delivery::Durable => CapacityDecision::BlockReliable,
        Delivery::BestEffort {
            on_full: Shed::DropNewest,
        } => CapacityDecision::ShedIncoming(Shed::DropNewest),
        Delivery::BestEffort {
            on_full: Shed::DropOldest,
        } => {
            let count = outstanding - capacity.get() + 1;
            CapacityDecision::ReplaceOldest {
                shed: Shed::DropOldest,
                count: NonZeroUsize::new(count)
                    .expect("in the saturation branch the replacement count is positive"),
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmissionClass {
    Ordinary,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscardedFlag {
    Mute,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmissionDisposition {
    Accepted,
    Discarded(DiscardedFlag),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmissionGate {
    All,
    ErrorOnly,
}

impl EmissionGate {
    #[must_use]
    pub const fn decide(self, class: EmissionClass) -> EmissionDisposition {
        match (self, class) {
            (Self::All, _) | (Self::ErrorOnly, EmissionClass::Error) => {
                EmissionDisposition::Accepted
            }
            (Self::ErrorOnly, EmissionClass::Ordinary) => {
                EmissionDisposition::Discarded(DiscardedFlag::Mute)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputFlagDecision {
    Paused,
    Invoke { emissions: EmissionGate },
    Bypass { passthrough: EmissionGate },
}

impl InputFlagDecision {
    #[must_use]
    pub const fn consumes_input(self) -> bool {
        !matches!(self, Self::Paused)
    }

    #[must_use]
    pub const fn invokes_hook(self) -> bool {
        matches!(self, Self::Invoke { .. })
    }

    #[must_use]
    pub const fn gate(self) -> Option<EmissionGate> {
        match self {
            Self::Paused => None,
            Self::Invoke { emissions } => Some(emissions),
            Self::Bypass { passthrough } => Some(passthrough),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputFlagDecisionError {
    BypassUnsupported,
}

impl fmt::Display for InputFlagDecisionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BypassUnsupported => formatter.write_str(
                "actor cannot bypass without a primary inlet/outlet pass-through contract",
            ),
        }
    }
}

impl std::error::Error for InputFlagDecisionError {}

pub fn decide_input_flags(
    flags: ActorFlags,
    bypass_supported: bool,
) -> Result<InputFlagDecision, InputFlagDecisionError> {
    if flags.pause() {
        return Ok(InputFlagDecision::Paused);
    }

    let gate = if flags.mute() {
        EmissionGate::ErrorOnly
    } else {
        EmissionGate::All
    };
    if flags.bypass() {
        if !bypass_supported {
            return Err(InputFlagDecisionError::BypassUnsupported);
        }
        Ok(InputFlagDecision::Bypass { passthrough: gate })
    } else {
        Ok(InputFlagDecision::Invoke { emissions: gate })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_table_separates_reliable_blocking_and_both_best_effort_sheds() {
        let capacity = PositiveCapacity::new(2).unwrap();
        for delivery in [
            Delivery::Lossless,
            Delivery::Durable,
            Delivery::BestEffort {
                on_full: Shed::DropNewest,
            },
            Delivery::BestEffort {
                on_full: Shed::DropOldest,
            },
        ] {
            let below = decide_capacity(delivery, capacity, 1);
            assert_eq!(
                below,
                if delivery == Delivery::Durable {
                    CapacityDecision::DurableCommitRequired
                } else {
                    CapacityDecision::Queue
                }
            );
        }

        assert_eq!(
            decide_capacity(Delivery::Lossless, capacity, 2),
            CapacityDecision::BlockReliable
        );
        assert_eq!(
            decide_capacity(Delivery::Durable, capacity, 7),
            CapacityDecision::BlockReliable
        );
        assert_eq!(
            decide_capacity(
                Delivery::BestEffort {
                    on_full: Shed::DropNewest
                },
                capacity,
                2
            ),
            CapacityDecision::ShedIncoming(Shed::DropNewest)
        );
        assert_eq!(
            decide_capacity(
                Delivery::BestEffort {
                    on_full: Shed::DropOldest
                },
                capacity,
                5
            ),
            CapacityDecision::ReplaceOldest {
                shed: Shed::DropOldest,
                count: NonZeroUsize::new(4).unwrap(),
            }
        );
    }

    #[test]
    fn all_eight_flag_combinations_follow_consume_hook_and_emission_stages() {
        for pause in [false, true] {
            for bypass in [false, true] {
                for mute in [false, true] {
                    let decision =
                        decide_input_flags(ActorFlags::new(bypass, mute, pause), true).unwrap();
                    if pause {
                        assert_eq!(decision, InputFlagDecision::Paused);
                        assert!(!decision.consumes_input());
                        assert!(!decision.invokes_hook());
                        assert_eq!(decision.gate(), None);
                        continue;
                    }

                    assert!(decision.consumes_input());
                    assert_eq!(decision.invokes_hook(), !bypass);
                    let expected_gate = if mute {
                        EmissionGate::ErrorOnly
                    } else {
                        EmissionGate::All
                    };
                    assert_eq!(decision.gate(), Some(expected_gate));
                    assert_eq!(
                        expected_gate.decide(EmissionClass::Ordinary),
                        if mute {
                            EmissionDisposition::Discarded(DiscardedFlag::Mute)
                        } else {
                            EmissionDisposition::Accepted
                        }
                    );
                    assert_eq!(
                        expected_gate.decide(EmissionClass::Error),
                        EmissionDisposition::Accepted
                    );
                }
            }
        }
    }

    #[test]
    fn unsupported_bypass_is_rejected_only_when_pause_does_not_short_circuit_consumption() {
        assert_eq!(
            decide_input_flags(ActorFlags::new(true, false, false), false),
            Err(InputFlagDecisionError::BypassUnsupported)
        );
        assert_eq!(
            decide_input_flags(ActorFlags::new(true, true, true), false),
            Ok(InputFlagDecision::Paused)
        );
    }
}
