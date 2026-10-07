#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Partition {
    SessionMechanics,
    Declaration,
    Query,
    Subscription,
    EventInjection,
    LedgerTransition,
    ReplayControl,
    Experimental,
    Lifecycle,
}

impl Partition {
    pub const ALL: [Self; 9] = [
        Self::SessionMechanics,
        Self::Declaration,
        Self::Query,
        Self::Subscription,
        Self::EventInjection,
        Self::LedgerTransition,
        Self::ReplayControl,
        Self::Experimental,
        Self::Lifecycle,
    ];

    pub const COUNT: usize = Self::ALL.len();
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SessionMechanicsVerb {
    Hello,
    HelloAck,
    Goodbye,
}

impl SessionMechanicsVerb {
    pub const ALL: [Self; 3] = [Self::Hello, Self::HelloAck, Self::Goodbye];
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DeclarationVerb {
    BeginEpoch,
    ValidateEpoch,
    CommitEpoch,
    AbortEpoch,
    UpsertActor,
    RetireActor,
    UpsertEdge,
    RetireEdge,
    UpsertScope,
    RetireScope,
    MoveToScope,
    UpsertExportMount,
    RetireExportMount,
    UpsertAnnotation,
    RetireAnnotation,
    SetPresentation,
    SetFlags,
    UpsertTemplate,
    RetireTemplate,
    CommandResult,
}

impl DeclarationVerb {
    pub const ALL: [Self; 20] = [
        Self::BeginEpoch,
        Self::ValidateEpoch,
        Self::CommitEpoch,
        Self::AbortEpoch,
        Self::UpsertActor,
        Self::RetireActor,
        Self::UpsertEdge,
        Self::RetireEdge,
        Self::UpsertScope,
        Self::RetireScope,
        Self::MoveToScope,
        Self::UpsertExportMount,
        Self::RetireExportMount,
        Self::UpsertAnnotation,
        Self::RetireAnnotation,
        Self::SetPresentation,
        Self::SetFlags,
        Self::UpsertTemplate,
        Self::RetireTemplate,
        Self::CommandResult,
    ];
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum QueryVerb {
    Query,
    QueryResult,
    QueryClose,
}

impl QueryVerb {
    pub const ALL: [Self; 3] = [Self::Query, Self::QueryResult, Self::QueryClose];
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SubscriptionVerb {
    Subscribe,
    SubscribeAck,
    Credit,
    Unsubscribe,
    Frame,
    SubscriptionEnded,
}

impl SubscriptionVerb {
    pub const ALL: [Self; 6] = [
        Self::Subscribe,
        Self::SubscribeAck,
        Self::Credit,
        Self::Unsubscribe,
        Self::Frame,
        Self::SubscriptionEnded,
    ];
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum EventInjectionVerb {
    Inject,
    InjectAck,
}

impl EventInjectionVerb {
    pub const ALL: [Self; 2] = [Self::Inject, Self::InjectAck];
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum LedgerTransitionVerb {
    ApprovalDecide,
    SetObservationControl,
    TransitionResult,
    SetAgentHarness,
}

impl LedgerTransitionVerb {
    pub const ALL: [Self; 4] = [
        Self::ApprovalDecide,
        Self::SetObservationControl,
        Self::TransitionResult,
        Self::SetAgentHarness,
    ];
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ReplayControlVerb {
    ReplayStart,
    ReplayRewind,
    ReplayEnd,
    ReplayResult,
}

impl ReplayControlVerb {
    pub const ALL: [Self; 4] = [
        Self::ReplayStart,
        Self::ReplayRewind,
        Self::ReplayEnd,
        Self::ReplayResult,
    ];
}

/// A stable, owner-local lifecycle partition for the standing product run.
///
/// These verbs do not mutate authoring and are deliberately distinct from
/// observation control and replay lenses.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum LifecycleVerb {
    Resume,
    Pause,
    LifecycleResult,
}

impl LifecycleVerb {
    /// Canonical append-only verb order.
    pub const ALL: [Self; 3] = [Self::Resume, Self::Pause, Self::LifecycleResult];
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Kind<X> {
    SessionMechanics(SessionMechanicsVerb),
    Declaration(DeclarationVerb),
    Query(QueryVerb),
    Subscription(SubscriptionVerb),
    EventInjection(EventInjectionVerb),
    LedgerTransition(LedgerTransitionVerb),
    ReplayControl(ReplayControlVerb),
    Experimental(X),
    Lifecycle(LifecycleVerb),
}

pub const STABLE_VERB_COUNT: usize = SessionMechanicsVerb::ALL.len()
    + DeclarationVerb::ALL.len()
    + QueryVerb::ALL.len()
    + SubscriptionVerb::ALL.len()
    + EventInjectionVerb::ALL.len()
    + LedgerTransitionVerb::ALL.len()
    + ReplayControlVerb::ALL.len()
    + LifecycleVerb::ALL.len();

impl<X> Kind<X> {
    #[must_use]
    pub const fn partition(&self) -> Partition {
        match self {
            Self::SessionMechanics(_) => Partition::SessionMechanics,
            Self::Declaration(_) => Partition::Declaration,
            Self::Query(_) => Partition::Query,
            Self::Subscription(_) => Partition::Subscription,
            Self::EventInjection(_) => Partition::EventInjection,
            Self::LedgerTransition(_) => Partition::LedgerTransition,
            Self::ReplayControl(_) => Partition::ReplayControl,
            Self::Experimental(_) => Partition::Experimental,
            Self::Lifecycle(_) => Partition::Lifecycle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_partitions_are_counted_once_in_declaration_order() {
        assert_eq!(Partition::ALL.len(), Partition::COUNT);
        assert!(
            Partition::ALL.windows(2).all(|pair| pair[0] < pair[1]),
            "Partition::ALL must contain each partition once in declaration order"
        );
    }

    #[test]
    fn kind_owns_its_partition() {
        let cases = [
            (
                Kind::<()>::SessionMechanics(SessionMechanicsVerb::Hello),
                Partition::SessionMechanics,
            ),
            (
                Kind::Declaration(DeclarationVerb::CommitEpoch),
                Partition::Declaration,
            ),
            (Kind::Query(QueryVerb::Query), Partition::Query),
            (
                Kind::Subscription(SubscriptionVerb::Frame),
                Partition::Subscription,
            ),
            (
                Kind::EventInjection(EventInjectionVerb::Inject),
                Partition::EventInjection,
            ),
            (
                Kind::LedgerTransition(LedgerTransitionVerb::ApprovalDecide),
                Partition::LedgerTransition,
            ),
            (
                Kind::ReplayControl(ReplayControlVerb::ReplayStart),
                Partition::ReplayControl,
            ),
            (Kind::Experimental(()), Partition::Experimental),
            (Kind::Lifecycle(LifecycleVerb::Resume), Partition::Lifecycle),
        ];

        for (kind, partition) in &cases {
            assert_eq!(kind.partition(), *partition);
        }
        assert_eq!(
            cases.map(|(_, partition)| partition),
            Partition::ALL,
            "the Kind witness table must cover the canonical partition exact set"
        );
    }

    #[test]
    fn each_verb_list_holds_its_partition_once_in_declaration_order() {
        fn ordered<V: Copy + Ord + std::fmt::Debug>(verbs: &[V]) {
            assert!(
                verbs.windows(2).all(|pair| pair[0] < pair[1]),
                "verb list must contain each verb once in declaration order: {verbs:?}"
            );
        }

        ordered(&SessionMechanicsVerb::ALL);
        ordered(&DeclarationVerb::ALL);
        ordered(&QueryVerb::ALL);
        ordered(&SubscriptionVerb::ALL);
        ordered(&EventInjectionVerb::ALL);
        ordered(&LedgerTransitionVerb::ALL);
        ordered(&ReplayControlVerb::ALL);
        ordered(&LifecycleVerb::ALL);
    }
}
