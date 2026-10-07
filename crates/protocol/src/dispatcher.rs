
use crate::{
    CommandResult, CorrelationId, DeclarationCommand, DeclarationDomain, Injection, LedgerDomain,
    LedgerTransition, Lifecycle, LifecycleAccepted, LifecycleDomain, PositiveCredit,
    PositiveFrameCount, QueryRequest, QueryResult, Subscribe,
};

pub trait DeclarationPort<C> {
    type Domain: DeclarationDomain;
    type Accepted;
    type Rejection;

    fn dispatch_declaration(
        &mut self,
        correlation: &CorrelationId<C>,
        command: DeclarationCommand<Self::Domain>,
    ) -> CommandResult<Self::Accepted, Self::Rejection>;
}

pub trait EventInjectionPort<C> {
    type Mount;
    type Payload;
    type IdempotencyKey;
    type Accepted;
    type Rejection;

    fn dispatch_injection(
        &mut self,
        correlation: &CorrelationId<C>,
        injection: Injection<Self::Mount, Self::Payload, Self::IdempotencyKey>,
    ) -> CommandResult<Self::Accepted, Self::Rejection>;
}

pub trait LedgerTransitionPort<C> {
    type Domain: LedgerDomain;
    type Accepted;
    type Rejection;

    fn dispatch_ledger_transition(
        &mut self,
        correlation: &CorrelationId<C>,
        transition: LedgerTransition<Self::Domain>,
    ) -> CommandResult<Self::Accepted, Self::Rejection>;
}

/// Stable standing-run lifecycle port.
pub trait LifecyclePort<C> {
    type Domain: LifecycleDomain;
    type Rejection;

    fn dispatch_lifecycle(
        &mut self,
        correlation: &CorrelationId<C>,
        control: Lifecycle<Self::Domain>,
    ) -> CommandResult<LifecycleAccepted, Self::Rejection>;
}

pub trait QueryPort<C> {
    type Name;
    type Arguments;
    type Limit;
    type Cursor;
    type Since;
    type Page;
    type Rejection;

    fn dispatch_query(
        &mut self,
        correlation: &CorrelationId<C>,
        query: QueryRequest<Self::Name, Self::Arguments, Self::Limit, Self::Cursor, Self::Since>,
    ) -> QueryResult<Self::Page, Self::Rejection>;

    fn close_query(
        &mut self,
        correlation: &CorrelationId<C>,
    ) -> QueryResult<Self::Page, Self::Rejection>;
}

pub trait SubscriptionPort<C> {
    type Target;
    type Credit: PositiveFrameCount;
    type Opened;
    type Credited;
    type Closed;
    type Rejection;

    fn dispatch_subscribe(
        &mut self,
        correlation: &CorrelationId<C>,
        subscribe: Subscribe<Self::Target>,
    ) -> CommandResult<Self::Opened, Self::Rejection>;

    fn dispatch_credit(
        &mut self,
        correlation: &CorrelationId<C>,
        credit: PositiveCredit<Self::Credit>,
    ) -> CommandResult<Self::Credited, Self::Rejection>;

    fn dispatch_unsubscribe(
        &mut self,
        correlation: &CorrelationId<C>,
    ) -> CommandResult<Self::Closed, Self::Rejection>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SemanticDispatcher<D, I, L, R, Q, S> {
    declaration: D,
    injection: I,
    ledger: L,
    lifecycle: R,
    query: Q,
    subscription: S,
}

impl<D, I, L, R, Q, S> SemanticDispatcher<D, I, L, R, Q, S> {
    #[must_use]
    pub const fn new(
        declaration: D,
        injection: I,
        ledger: L,
        lifecycle: R,
        query: Q,
        subscription: S,
    ) -> Self {
        Self {
            declaration,
            injection,
            ledger,
            lifecycle,
            query,
            subscription,
        }
    }

    #[must_use]
    pub const fn declaration(&self) -> &D {
        &self.declaration
    }

    #[must_use]
    pub const fn injection(&self) -> &I {
        &self.injection
    }

    #[must_use]
    pub const fn ledger(&self) -> &L {
        &self.ledger
    }

    #[must_use]
    pub const fn lifecycle(&self) -> &R {
        &self.lifecycle
    }

    #[must_use]
    pub const fn query(&self) -> &Q {
        &self.query
    }

    #[must_use]
    pub const fn subscription(&self) -> &S {
        &self.subscription
    }

    #[must_use]
    pub fn into_ports(self) -> (D, I, L, R, Q, S) {
        (
            self.declaration,
            self.injection,
            self.ledger,
            self.lifecycle,
            self.query,
            self.subscription,
        )
    }

    pub fn dispatch_declaration<C>(
        &mut self,
        correlation: &CorrelationId<C>,
        command: DeclarationCommand<D::Domain>,
    ) -> CommandResult<D::Accepted, D::Rejection>
    where
        D: DeclarationPort<C>,
    {
        self.declaration.dispatch_declaration(correlation, command)
    }

    pub fn dispatch_injection<C>(
        &mut self,
        correlation: &CorrelationId<C>,
        injection: Injection<I::Mount, I::Payload, I::IdempotencyKey>,
    ) -> CommandResult<I::Accepted, I::Rejection>
    where
        I: EventInjectionPort<C>,
    {
        self.injection.dispatch_injection(correlation, injection)
    }

    pub fn dispatch_ledger_transition<C>(
        &mut self,
        correlation: &CorrelationId<C>,
        transition: LedgerTransition<L::Domain>,
    ) -> CommandResult<L::Accepted, L::Rejection>
    where
        L: LedgerTransitionPort<C>,
    {
        self.ledger
            .dispatch_ledger_transition(correlation, transition)
    }

    pub fn dispatch_lifecycle<C>(
        &mut self,
        correlation: &CorrelationId<C>,
        control: Lifecycle<R::Domain>,
    ) -> CommandResult<LifecycleAccepted, R::Rejection>
    where
        R: LifecyclePort<C>,
    {
        self.lifecycle.dispatch_lifecycle(correlation, control)
    }

    pub fn dispatch_query<C>(
        &mut self,
        correlation: &CorrelationId<C>,
        query: QueryRequest<Q::Name, Q::Arguments, Q::Limit, Q::Cursor, Q::Since>,
    ) -> QueryResult<Q::Page, Q::Rejection>
    where
        Q: QueryPort<C>,
    {
        self.query.dispatch_query(correlation, query)
    }

    pub fn dispatch_query_close<C>(
        &mut self,
        correlation: &CorrelationId<C>,
    ) -> QueryResult<Q::Page, Q::Rejection>
    where
        Q: QueryPort<C>,
    {
        self.query.close_query(correlation)
    }

    pub fn dispatch_subscribe<C>(
        &mut self,
        correlation: &CorrelationId<C>,
        subscribe: Subscribe<S::Target>,
    ) -> CommandResult<S::Opened, S::Rejection>
    where
        S: SubscriptionPort<C>,
    {
        self.subscription.dispatch_subscribe(correlation, subscribe)
    }

    pub fn dispatch_credit<C>(
        &mut self,
        correlation: &CorrelationId<C>,
        credit: PositiveCredit<S::Credit>,
    ) -> CommandResult<S::Credited, S::Rejection>
    where
        S: SubscriptionPort<C>,
    {
        self.subscription.dispatch_credit(correlation, credit)
    }

    pub fn dispatch_unsubscribe<C>(
        &mut self,
        correlation: &CorrelationId<C>,
    ) -> CommandResult<S::Closed, S::Rejection>
    where
        S: SubscriptionPort<C>,
    {
        self.subscription.dispatch_unsubscribe(correlation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PageStep, Target};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestDeclarationDomain;

    impl DeclarationDomain for TestDeclarationDomain {
        type CommitId = &'static str;
        type ScopeDeclaration = &'static str;
        type Scope = &'static str;
        type EpochId = &'static str;
        type ExpectedRevision = u8;
        type ActorId = &'static str;
        type ActorDecl = &'static str;
        type EdgeId = &'static str;
        type EdgeAttrs = &'static str;
        type ScopeSeg = &'static str;
        type ExportName = &'static str;
        type ExportMount = &'static str;
        type AnnotationId = &'static str;
        type Annotation = &'static str;
        type PresentationOwner = &'static str;
        type Presentation = &'static str;
        type Flags = &'static str;
        type AuthoringEnvironment = &'static str;
        type TemplateName = String;
        type TemplateCommands = Vec<circular_core::Value>;
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestLedgerDomain;

    impl LedgerDomain for TestLedgerDomain {
        type ApprovalItem = &'static str;
        type ApprovalDecision = &'static str;
        type ObservationControl = &'static str;
        type AgentHarness = &'static str;
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestLifecycleDomain;

    impl LifecycleDomain for TestLifecycleDomain {
        type AuthoringRevision = &'static str;
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Frames(u8);

    impl PositiveFrameCount for Frames {
        fn is_positive(&self) -> bool {
            self.0 > 0
        }
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum Call {
        Declaration(u64, &'static str, &'static str),
        Injection(u64, &'static str),
        Ledger(u64, &'static str),
        Lifecycle(u64, &'static str),
        Query(u64, &'static str),
        QueryClose(u64),
        Subscribe(u64, &'static str),
        Credit(u64, u8),
        Unsubscribe(u64),
    }

    type Transcript = Rc<RefCell<Vec<Call>>>;

    struct DeclarationMock(Transcript);
    struct InjectionMock(Transcript);
    struct LedgerMock(Transcript);
    struct LifecycleMock(Transcript);
    struct QueryMock(Transcript);
    struct SubscriptionMock(Transcript);

    impl DeclarationPort<u64> for DeclarationMock {
        type Domain = TestDeclarationDomain;
        type Accepted = &'static str;
        type Rejection = &'static str;

        fn dispatch_declaration(
            &mut self,
            correlation: &CorrelationId<u64>,
            command: DeclarationCommand<Self::Domain>,
        ) -> CommandResult<Self::Accepted, Self::Rejection> {
            let (epoch, actor) = match command {
                DeclarationCommand::SetFlags { epoch, actor, .. } => (epoch, actor),
                DeclarationCommand::BeginEpoch { .. } => ("not-open", "other"),
                command => (*command.epoch().expect("names an open epoch"), "other"),
            };
            self.0
                .borrow_mut()
                .push(Call::Declaration(*correlation.value(), epoch, actor));
            if actor == "reject" {
                CommandResult::Rejected("declaration-rejected")
            } else {
                CommandResult::Accepted("candidate-applied")
            }
        }
    }

    impl EventInjectionPort<u64> for InjectionMock {
        type Mount = &'static str;
        type Payload = &'static str;
        type IdempotencyKey = &'static str;
        type Accepted = &'static str;
        type Rejection = &'static str;

        fn dispatch_injection(
            &mut self,
            correlation: &CorrelationId<u64>,
            injection: Injection<Self::Mount, Self::Payload, Self::IdempotencyKey>,
        ) -> CommandResult<Self::Accepted, Self::Rejection> {
            self.0
                .borrow_mut()
                .push(Call::Injection(*correlation.value(), injection.mount()));
            if injection.mount() == &"reject" {
                CommandResult::Rejected("injection-rejected")
            } else {
                CommandResult::Accepted("router-accepted")
            }
        }
    }

    impl LedgerTransitionPort<u64> for LedgerMock {
        type Domain = TestLedgerDomain;
        type Accepted = &'static str;
        type Rejection = &'static str;

        fn dispatch_ledger_transition(
            &mut self,
            correlation: &CorrelationId<u64>,
            transition: LedgerTransition<Self::Domain>,
        ) -> CommandResult<Self::Accepted, Self::Rejection> {
            let item = match transition {
                LedgerTransition::ApprovalDecide { item, .. } => item,
                LedgerTransition::SetObservationControl { control } => control,
                LedgerTransition::SetAgentHarness { harness } => harness,
            };
            self.0
                .borrow_mut()
                .push(Call::Ledger(*correlation.value(), item));
            if item == "reject" {
                CommandResult::Rejected("ledger-rejected")
            } else {
                CommandResult::Accepted("ledger-committed")
            }
        }
    }

    impl LifecyclePort<u64> for LifecycleMock {
        type Domain = TestLifecycleDomain;
        type Rejection = &'static str;

        fn dispatch_lifecycle(
            &mut self,
            correlation: &CorrelationId<u64>,
            control: Lifecycle<Self::Domain>,
        ) -> CommandResult<LifecycleAccepted, Self::Rejection> {
            let operation = match control {
                Lifecycle::Resume { .. } => "start",
                Lifecycle::Pause { .. } => "stop",
            };
            self.0
                .borrow_mut()
                .push(Call::Lifecycle(*correlation.value(), operation));
            CommandResult::Accepted(if operation == "start" {
                LifecycleAccepted::Resumed
            } else {
                LifecycleAccepted::Paused
            })
        }
    }

    impl QueryPort<u64> for QueryMock {
        type Name = &'static str;
        type Arguments = &'static str;
        type Limit = u8;
        type Cursor = &'static str;
        type Since = &'static str;
        type Page = &'static str;
        type Rejection = &'static str;

        fn dispatch_query(
            &mut self,
            correlation: &CorrelationId<u64>,
            query: QueryRequest<
                Self::Name,
                Self::Arguments,
                Self::Limit,
                Self::Cursor,
                Self::Since,
            >,
        ) -> QueryResult<Self::Page, Self::Rejection> {
            self.0
                .borrow_mut()
                .push(Call::Query(*correlation.value(), query.name()));
            if query.name() == &"reject" {
                QueryResult::Rejected("query-rejected")
            } else {
                QueryResult::Accepted("page")
            }
        }

        fn close_query(
            &mut self,
            correlation: &CorrelationId<u64>,
        ) -> QueryResult<Self::Page, Self::Rejection> {
            self.0
                .borrow_mut()
                .push(Call::QueryClose(*correlation.value()));
            if correlation.value() == &13 {
                QueryResult::Rejected("close-rejected")
            } else {
                QueryResult::Accepted("closed-page")
            }
        }
    }

    impl SubscriptionPort<u64> for SubscriptionMock {
        type Target = Target<&'static str, &'static str>;
        type Credit = Frames;
        type Opened = &'static str;
        type Credited = &'static str;
        type Closed = &'static str;
        type Rejection = &'static str;

        fn dispatch_subscribe(
            &mut self,
            correlation: &CorrelationId<u64>,
            subscribe: Subscribe<Self::Target>,
        ) -> CommandResult<Self::Opened, Self::Rejection> {
            self.0.borrow_mut().push(Call::Subscribe(
                *correlation.value(),
                subscribe.target().name(),
            ));
            if subscribe.target().name() == &"reject" {
                CommandResult::Rejected("subscribe-rejected")
            } else {
                CommandResult::Accepted("opened")
            }
        }

        fn dispatch_credit(
            &mut self,
            correlation: &CorrelationId<u64>,
            credit: PositiveCredit<Self::Credit>,
        ) -> CommandResult<Self::Credited, Self::Rejection> {
            let frames = credit.into_frames().0;
            self.0
                .borrow_mut()
                .push(Call::Credit(*correlation.value(), frames));
            if frames == 13 {
                CommandResult::Rejected("credit-rejected")
            } else {
                CommandResult::Accepted("credited")
            }
        }

        fn dispatch_unsubscribe(
            &mut self,
            correlation: &CorrelationId<u64>,
        ) -> CommandResult<Self::Closed, Self::Rejection> {
            self.0
                .borrow_mut()
                .push(Call::Unsubscribe(*correlation.value()));
            if correlation.value() == &13 {
                CommandResult::Rejected("unsubscribe-rejected")
            } else {
                CommandResult::Accepted("closed")
            }
        }
    }

    fn dispatcher(
        transcript: &Transcript,
    ) -> SemanticDispatcher<
        DeclarationMock,
        InjectionMock,
        LedgerMock,
        LifecycleMock,
        QueryMock,
        SubscriptionMock,
    > {
        SemanticDispatcher::new(
            DeclarationMock(Rc::clone(transcript)),
            InjectionMock(Rc::clone(transcript)),
            LedgerMock(Rc::clone(transcript)),
            LifecycleMock(Rc::clone(transcript)),
            QueryMock(Rc::clone(transcript)),
            SubscriptionMock(Rc::clone(transcript)),
        )
    }

    fn credit(frames: u8) -> PositiveCredit<Frames> {
        PositiveCredit::try_new(Frames(frames)).expect("positive frame")
    }

    #[test]
    fn mock_transcript_preserves_correlation_order_and_typed_payloads() {
        let transcript = Rc::new(RefCell::new(Vec::new()));
        let mut dispatcher = dispatcher(&transcript);

        assert_eq!(
            dispatcher.dispatch_declaration(
                &CorrelationId::from_value(1),
                DeclarationCommand::SetFlags {
                    epoch: "epoch-a",
                    actor: "actor-a",
                    flags: "enabled",
                },
            ),
            CommandResult::Accepted("candidate-applied")
        );
        assert_eq!(
            dispatcher.dispatch_injection(
                &CorrelationId::from_value(2),
                Injection::new("mount-a", "payload", "key-a"),
            ),
            CommandResult::Accepted("router-accepted")
        );
        assert_eq!(
            dispatcher.dispatch_ledger_transition(
                &CorrelationId::from_value(3),
                LedgerTransition::ApprovalDecide {
                    item: "approval-a",
                    decision: "approve",
                },
            ),
            CommandResult::Accepted("ledger-committed")
        );
        assert_eq!(
            dispatcher.dispatch_query(
                &CorrelationId::from_value(4),
                QueryRequest::new("records", "all", PageStep::First { limit: 5 }, None),
            ),
            QueryResult::Accepted("page")
        );
        assert_eq!(
            dispatcher.dispatch_query_close(&CorrelationId::from_value(4)),
            QueryResult::Accepted("closed-page")
        );
        assert_eq!(
            dispatcher.dispatch_subscribe(
                &CorrelationId::from_value(5),
                Subscribe::new(Target::new("facts", "all")),
            ),
            CommandResult::Accepted("opened")
        );
        assert_eq!(
            dispatcher.dispatch_credit(&CorrelationId::from_value(5), credit(2)),
            CommandResult::Accepted("credited")
        );
        assert_eq!(
            dispatcher.dispatch_unsubscribe(&CorrelationId::from_value(5)),
            CommandResult::Accepted("closed")
        );

        assert_eq!(
            &*transcript.borrow(),
            &[
                Call::Declaration(1, "epoch-a", "actor-a"),
                Call::Injection(2, "mount-a"),
                Call::Ledger(3, "approval-a"),
                Call::Query(4, "records"),
                Call::QueryClose(4),
                Call::Subscribe(5, "facts"),
                Call::Credit(5, 2),
                Call::Unsubscribe(5),
            ]
        );
    }

    #[test]
    fn port_rejections_are_returned_without_cross_port_calls_or_rewriting() {
        let transcript = Rc::new(RefCell::new(Vec::new()));
        let mut dispatcher = dispatcher(&transcript);

        assert_eq!(
            dispatcher.dispatch_declaration(
                &CorrelationId::from_value(10),
                DeclarationCommand::SetFlags {
                    epoch: "epoch-reject",
                    actor: "reject",
                    flags: "unchanged",
                },
            ),
            CommandResult::Rejected("declaration-rejected")
        );
        assert_eq!(
            dispatcher.dispatch_injection(
                &CorrelationId::from_value(11),
                Injection::new("reject", "payload", "key"),
            ),
            CommandResult::Rejected("injection-rejected")
        );
        assert_eq!(
            dispatcher.dispatch_ledger_transition(
                &CorrelationId::from_value(12),
                LedgerTransition::ApprovalDecide {
                    item: "reject",
                    decision: "approve",
                },
            ),
            CommandResult::Rejected("ledger-rejected")
        );
        assert_eq!(
            dispatcher.dispatch_query(
                &CorrelationId::from_value(13),
                QueryRequest::new("reject", "all", PageStep::First { limit: 1 }, None),
            ),
            QueryResult::Rejected("query-rejected")
        );
        assert_eq!(
            dispatcher.dispatch_query_close(&CorrelationId::from_value(13)),
            QueryResult::Rejected("close-rejected")
        );
        assert_eq!(
            dispatcher.dispatch_subscribe(
                &CorrelationId::from_value(14),
                Subscribe::new(Target::new("reject", "all")),
            ),
            CommandResult::Rejected("subscribe-rejected")
        );
        assert_eq!(
            dispatcher.dispatch_credit(&CorrelationId::from_value(15), credit(13)),
            CommandResult::Rejected("credit-rejected")
        );
        assert_eq!(
            dispatcher.dispatch_unsubscribe(&CorrelationId::from_value(13)),
            CommandResult::Rejected("unsubscribe-rejected")
        );

        assert_eq!(
            &*transcript.borrow(),
            &[
                Call::Declaration(10, "epoch-reject", "reject"),
                Call::Injection(11, "reject"),
                Call::Ledger(12, "reject"),
                Call::Query(13, "reject"),
                Call::QueryClose(13),
                Call::Subscribe(14, "reject"),
                Call::Credit(15, 13),
                Call::Unsubscribe(13),
            ]
        );
    }
}
