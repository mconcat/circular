
use std::collections::BTreeMap;
use std::num::NonZeroUsize;

#[derive(Clone, Debug, Eq, PartialEq)]
enum InjectionEntry<O, R, Q> {
    InFlight { outcome: O },
    Completed { response: R, sequence: Q },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InjectionLookup<M, K, O, R> {
    Started { outcome: O, evicted: Option<(M, K)> },
    Awaiting { outcome: O },
    Replayed { response: R },
    Exhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InjectionTransitionError {
    UnknownKey,
    AlreadyCompleted,
    NonIncreasingCompletionSequence,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InjectionLedger<M, K, O, R, Q> {
    capacity: NonZeroUsize,
    entries: BTreeMap<(M, K), InjectionEntry<O, R, Q>>,
    last_completion: Option<Q>,
}

impl<M, K, O, R, Q> InjectionLedger<M, K, O, R, Q>
where
    M: Clone + Ord,
    K: Clone + Ord,
    O: Clone,
    R: Clone,
    Q: Clone + Ord,
{
    #[must_use]
    pub const fn new(capacity: NonZeroUsize) -> Self {
        Self {
            capacity,
            entries: BTreeMap::new(),
            last_completion: None,
        }
    }

    #[must_use]
    pub const fn capacity(&self) -> NonZeroUsize {
        self.capacity
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn begin(&mut self, mount: M, key: K, outcome: O) -> InjectionLookup<M, K, O, R> {
        let decision_key = (mount, key);
        if let Some(entry) = self.entries.get(&decision_key) {
            return match entry {
                InjectionEntry::InFlight { outcome } => InjectionLookup::Awaiting {
                    outcome: outcome.clone(),
                },
                InjectionEntry::Completed { response, .. } => InjectionLookup::Replayed {
                    response: response.clone(),
                },
            };
        }

        let evicted = if self.entries.len() == self.capacity.get() {
            let oldest = self
                .entries
                .iter()
                .filter_map(|(key, entry)| match entry {
                    InjectionEntry::Completed { sequence, .. } => Some((key, sequence)),
                    InjectionEntry::InFlight { .. } => None,
                })
                .min_by(|(left_key, left_sequence), (right_key, right_sequence)| {
                    left_sequence
                        .cmp(right_sequence)
                        .then_with(|| left_key.cmp(right_key))
                })
                .map(|(key, _)| key.clone());
            let Some(oldest) = oldest else {
                return InjectionLookup::Exhausted;
            };
            self.entries.remove(&oldest);
            Some(oldest)
        } else {
            None
        };

        self.entries.insert(
            decision_key,
            InjectionEntry::InFlight {
                outcome: outcome.clone(),
            },
        );
        InjectionLookup::Started { outcome, evicted }
    }

    pub fn reject(&mut self, mount: &M, key: &K) -> bool {
        let decision_key = (mount.clone(), key.clone());
        if matches!(
            self.entries.get(&decision_key),
            Some(InjectionEntry::InFlight { .. })
        ) {
            self.entries.remove(&decision_key);
            true
        } else {
            false
        }
    }

    pub fn complete(
        &mut self,
        mount: &M,
        key: &K,
        response: R,
        sequence: Q,
    ) -> Result<(), InjectionTransitionError> {
        let decision_key = (mount.clone(), key.clone());
        match self.entries.get(&decision_key) {
            None => return Err(InjectionTransitionError::UnknownKey),
            Some(InjectionEntry::Completed { .. }) => {
                return Err(InjectionTransitionError::AlreadyCompleted);
            }
            Some(InjectionEntry::InFlight { .. }) => {}
        }
        if self
            .last_completion
            .as_ref()
            .is_some_and(|last| &sequence <= last)
        {
            return Err(InjectionTransitionError::NonIncreasingCompletionSequence);
        }

        self.entries.insert(
            decision_key,
            InjectionEntry::Completed {
                response,
                sequence: sequence.clone(),
            },
        );
        self.last_completion = Some(sequence);
        Ok(())
    }

    #[must_use]
    pub fn is_in_flight(&self, mount: &M, key: &K) -> bool {
        self.entries
            .get(&(mount.clone(), key.clone()))
            .is_some_and(|entry| matches!(entry, InjectionEntry::InFlight { .. }))
    }

    pub fn restart(&mut self) {
        self.entries.clear();
        self.last_completion = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        EventInjectionVerb, FeatureSet, KindRegistration, Partition, SessionRole, SessionRoles,
        TransportTrust, accepts,
    };

    #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
    struct Scope;

    impl crate::ScopeCoverage for Scope {
        fn covers(&self, _target: &Self) -> bool {
            true
        }
    }

    fn features() -> FeatureSet<u8> {
        FeatureSet::try_new([
            (Partition::SessionMechanics, 0),
            (Partition::Declaration, 0),
            (Partition::Query, 0),
            (Partition::Subscription, 0),
            (Partition::EventInjection, 0),
            (Partition::LedgerTransition, 0),
            (Partition::ReplayControl, 0),
            (Partition::Experimental, 0),
        ])
        .expect("a complete capability set without duplicates")
    }

    #[test]
    fn role_rejection_happens_before_the_injection_table_changes() {
        let registration =
            KindRegistration::<_, Scope, ()>::event_injection(EventInjectionVerb::Inject, 0_u8)
                .expect("request verb");
        let reader = SessionRoles::<Scope>::reader_only();
        let mut table = InjectionLedger::<&str, &str, &str, &str, u8>::new(
            NonZeroUsize::new(2).expect("positive"),
        );

        if accepts(
            &registration,
            &reader,
            TransportTrust::LocalOwner,
            &features(),
        ) {
            table.begin("mount", "key", "outcome");
        }
        assert!(table.is_empty());

        let operator = reader.with_role(SessionRole::Operator);
        assert!(accepts(
            &registration,
            &operator,
            TransportTrust::Remote,
            &features(),
        ));
        assert!(matches!(
            table.begin("mount", "key", "outcome"),
            InjectionLookup::Started { evicted: None, .. }
        ));
    }

    #[test]
    fn duplicate_injection_waits_then_replays_the_first_response_across_sessions() {
        let mut table = InjectionLedger::new(NonZeroUsize::new(3).expect("positive"));
        assert_eq!(
            table.begin("mount", "key", "first-outcome"),
            InjectionLookup::Started {
                outcome: "first-outcome",
                evicted: None
            }
        );
        assert_eq!(
            table.begin("mount", "key", "other-session-outcome"),
            InjectionLookup::Awaiting {
                outcome: "first-outcome"
            }
        );

        table
            .complete(&"mount", &"key", "stamp-7", 1_u8)
            .expect("first router acceptance");
        assert_eq!(
            table.begin("mount", "key", "third-outcome"),
            InjectionLookup::Replayed {
                response: "stamp-7"
            }
        );
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn rejected_injection_leaves_no_suppression_entry() {
        let mut table =
            InjectionLedger::<_, _, _, &str, u8>::new(NonZeroUsize::new(1).expect("positive"));
        table.begin("mount", "bad", "outcome");
        assert!(table.reject(&"mount", &"bad"));
        assert!(table.is_empty());
        assert!(matches!(
            table.begin("mount", "bad", "retry"),
            InjectionLookup::Started { .. }
        ));
    }

    #[test]
    fn full_table_evicts_oldest_completed_but_never_in_flight() {
        let mut table = InjectionLedger::new(NonZeroUsize::new(2).expect("positive"));
        table.begin("mount", "one", "outcome-1");
        table
            .complete(&"mount", &"one", "response-1", 1_u8)
            .expect("completion");
        table.begin("mount", "two", "outcome-2");

        assert_eq!(
            table.begin("mount", "three", "outcome-3"),
            InjectionLookup::Started {
                outcome: "outcome-3",
                evicted: Some(("mount", "one"))
            }
        );
        assert!(table.is_in_flight(&"mount", &"two"));

        let mut all_busy =
            InjectionLedger::<_, _, _, &str, u8>::new(NonZeroUsize::new(2).expect("positive"));
        all_busy.begin("mount", "one", "outcome-1");
        all_busy.begin("mount", "two", "outcome-2");
        assert_eq!(
            all_busy.begin("mount", "three", "outcome-3"),
            InjectionLookup::Exhausted
        );
        assert_eq!(all_busy.len(), 2);
    }

    #[test]
    fn completion_sequence_is_strict_and_restart_is_empty() {
        let mut table = InjectionLedger::new(NonZeroUsize::new(2).expect("positive"));
        table.begin("mount", "one", "outcome-1");
        table
            .complete(&"mount", &"one", "response-1", 5_u8)
            .expect("first completion");
        table.begin("mount", "two", "outcome-2");
        assert_eq!(
            table.complete(&"mount", &"two", "response-2", 5),
            Err(InjectionTransitionError::NonIncreasingCompletionSequence)
        );
        assert!(table.is_in_flight(&"mount", &"two"));
        table.restart();
        assert!(table.is_empty());
    }
}
