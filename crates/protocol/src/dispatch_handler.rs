
use crate::{
    AdmittedHandler, CorrelationId, DeclarationPort, EventInjectionPort, LedgerTransitionPort,
    LifecyclePort, QueryPageOf, QueryPort, SemanticDispatcher, SessionDomain, SessionPayload,
    SubscriptionPort,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchHandler<Dc, I, L, R, Q, S> {
    dispatcher: SemanticDispatcher<Dc, I, L, R, Q, S>,
}

impl<Dc, I, L, R, Q, S> DispatchHandler<Dc, I, L, R, Q, S> {
    #[must_use]
    pub const fn new(dispatcher: SemanticDispatcher<Dc, I, L, R, Q, S>) -> Self {
        Self { dispatcher }
    }
}

impl<C, D, Dc, I, L, R, Q, S> AdmittedHandler<C, D> for DispatchHandler<Dc, I, L, R, Q, S>
where
    D: SessionDomain,
    Dc: DeclarationPort<
            C,
            Domain = D::Declaration,
            Accepted = D::DeclarationAccepted,
            Rejection = D::DeclarationRejection,
        >,
    I: EventInjectionPort<
            C,
            Mount = D::InjectionMount,
            Payload = D::InjectionPayload,
            IdempotencyKey = D::IdempotencyKey,
            Accepted = D::InjectionAccepted,
            Rejection = D::InjectionRejection,
        >,
    L: LedgerTransitionPort<
            C,
            Domain = D::Ledger,
            Accepted = D::TransitionAccepted,
            Rejection = D::TransitionRejection,
        >,
    R: LifecyclePort<C, Domain = D::Lifecycle, Rejection = D::LifecycleRejection>,
    Q: QueryPort<
            C,
            Name = D::QueryName,
            Arguments = D::QueryArguments,
            Limit = D::PageLimit,
            Cursor = D::PageCursor,
            Since = D::QuerySince,
            Page = QueryPageOf<D>,
            Rejection = D::QueryRejection,
        >,
    S: SubscriptionPort<
            C,
            Target = D::SubscriptionTarget,
            Credit = D::Credit,
            Opened = D::SubscriptionOpened,
            Credited = D::SubscriptionOpened,
            Closed = D::SubscriptionOpened,
            Rejection = D::SubscriptionRejection,
        >,
{
    fn handle(
        &mut self,
        correlation: &CorrelationId<C>,
        payload: SessionPayload<D>,
    ) -> Option<SessionPayload<D>> {
        match payload {
            SessionPayload::Declaration(command) => Some(SessionPayload::CommandResult(
                self.dispatcher.dispatch_declaration(correlation, command),
            )),
            SessionPayload::Query(request) => Some(SessionPayload::QueryResult(
                self.dispatcher.dispatch_query(correlation, request),
            )),
            SessionPayload::QueryClose => Some(SessionPayload::QueryResult(
                self.dispatcher.dispatch_query_close(correlation),
            )),
            SessionPayload::Inject(injection) => Some(SessionPayload::InjectAck(
                self.dispatcher.dispatch_injection(correlation, injection),
            )),
            SessionPayload::LedgerTransition(transition) => Some(SessionPayload::TransitionResult(
                self.dispatcher
                    .dispatch_ledger_transition(correlation, transition),
            )),
            SessionPayload::Lifecycle(control) => Some(SessionPayload::LifecycleResult(
                self.dispatcher.dispatch_lifecycle(correlation, control),
            )),
            SessionPayload::Subscribe(subscribe) => Some(SessionPayload::SubscribeAck(
                self.dispatcher.dispatch_subscribe(correlation, subscribe),
            )),
            SessionPayload::Credit(credit) => Some(SessionPayload::SubscribeAck(
                self.dispatcher.dispatch_credit(correlation, credit),
            )),
            SessionPayload::Unsubscribe => Some(SessionPayload::SubscribeAck(
                self.dispatcher.dispatch_unsubscribe(correlation),
            )),

            SessionPayload::ReplayControl(_) | SessionPayload::Experimental(_) => None,

            SessionPayload::Hello(_)
            | SessionPayload::HelloAck(_)
            | SessionPayload::Goodbye
            | SessionPayload::CommandResult(_)
            | SessionPayload::QueryResult(_)
            | SessionPayload::SubscribeAck(_)
            | SessionPayload::Frame(_)
            | SessionPayload::SubscriptionEnded(_)
            | SessionPayload::InjectAck(_)
            | SessionPayload::TransitionResult(_)
            | SessionPayload::LifecycleResult(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Anchor, CommandResult, DeclarationCommand, DeclarationDomain, DeclarationVerb,
        EventInjectionVerb, ExperimentalPayload, Injection, Kind, LedgerDomain, LedgerTransition,
        LedgerTransitionVerb, Lifecycle, LifecycleAccepted, LifecycleDomain, LifecycleVerb,
        PageEnd, PageStep, PositiveCredit, PositiveFrameCount, QueryPage, QueryRequest,
        QueryResult, QueryVerb, ReplayControlPayload, ReplayControlVerb, Subscribe,
        SubscriptionTerminationDomain, SubscriptionVerb, Target,
    };
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Domain;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Termination;

    impl SubscriptionTerminationDomain for Termination {
        type ResetFloorOrCursor = &'static str;
        type StructureCursor = &'static str;
        type AuthoringEnvironment = &'static str;
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Frames(u16);

    impl PositiveFrameCount for Frames {
        fn is_positive(&self) -> bool {
            self.0 > 0
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Replay(ReplayControlVerb);

    impl ReplayControlPayload for Replay {
        fn verb(&self) -> ReplayControlVerb {
            self.0
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct ExperimentalVerb(u8);

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Experimental(ExperimentalVerb);

    impl ExperimentalPayload for Experimental {
        type Verb = ExperimentalVerb;

        fn verb(&self) -> Self::Verb {
            self.0
        }
    }

    impl DeclarationDomain for Domain {
        type CommitId = &'static str;
        type ScopeDeclaration = &'static str;
        type Scope = &'static str;
        type EpochId = &'static str;
        type ExpectedRevision = &'static str;
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

    impl LedgerDomain for Domain {
        type ApprovalItem = &'static str;
        type ApprovalDecision = &'static str;
        type ObservationControl = &'static str;
        type AgentHarness = &'static str;
    }

    impl LifecycleDomain for Domain {
        type AuthoringRevision = &'static str;
    }

    impl SessionDomain for Domain {
        type ProtocolVersion = u8;
        type FeatureMinor = u8;
        type Scope = &'static str;
        type SessionToken = u32;
        type EstablishmentRejection = &'static str;

        type Declaration = Self;
        type DeclarationAccepted = &'static str;
        type DeclarationRejection = &'static str;

        type QueryName = &'static str;
        type QueryArguments = &'static str;
        type PageLimit = u16;
        type PageCursor = &'static str;
        type QuerySince = &'static str;
        type QueryAnchor = Anchor<&'static str, u64>;
        type QueryItem = &'static str;
        type PageCut = &'static str;
        type PageReached = &'static str;
        type PageDiagnostic = &'static str;
        type QueryRejection = &'static str;

        type SubscriptionTarget = Target<&'static str, &'static str>;
        type Credit = Frames;
        type SubscriptionOpened = &'static str;
        type SubscriptionRejection = &'static str;
        type ConflationSlot = &'static str;
        type FramePayload = &'static str;
        type SubscriptionAnchor = u64;
        type SubscriptionDiagnostic = &'static str;
        type SubscriptionTermination = Termination;

        type InjectionMount = &'static str;
        type InjectionPayload = &'static str;
        type IdempotencyKey = &'static str;
        type InjectionAccepted = &'static str;
        type InjectionRejection = &'static str;

        type Ledger = Self;
        type TransitionAccepted = &'static str;
        type TransitionRejection = &'static str;

        type Lifecycle = Self;
        type LifecycleRejection = &'static str;

        type ReplayControl = Replay;
        type ExperimentalVerb = ExperimentalVerb;
        type Experimental = Experimental;
    }

    type Payload = SessionPayload<Domain>;

    #[derive(Clone, Default)]
    struct Calls(Rc<RefCell<Vec<&'static str>>>);

    impl Calls {
        fn seen(&self) -> Vec<&'static str> {
            self.0.borrow().clone()
        }

        fn note(&self, port: &'static str) {
            self.0.borrow_mut().push(port);
        }
    }

    struct Ports(Calls);

    impl DeclarationPort<u32> for Ports {
        type Domain = Domain;
        type Accepted = &'static str;
        type Rejection = &'static str;

        fn dispatch_declaration(
            &mut self,
            _correlation: &CorrelationId<u32>,
            _command: DeclarationCommand<Domain>,
        ) -> CommandResult<Self::Accepted, Self::Rejection> {
            self.0.note("declaration");
            CommandResult::Accepted("declaration accepted")
        }
    }

    impl EventInjectionPort<u32> for Ports {
        type Mount = &'static str;
        type Payload = &'static str;
        type IdempotencyKey = &'static str;
        type Accepted = &'static str;
        type Rejection = &'static str;

        fn dispatch_injection(
            &mut self,
            _correlation: &CorrelationId<u32>,
            _injection: Injection<Self::Mount, Self::Payload, Self::IdempotencyKey>,
        ) -> CommandResult<Self::Accepted, Self::Rejection> {
            self.0.note("injection");
            CommandResult::Rejected("injection refused")
        }
    }

    impl LedgerTransitionPort<u32> for Ports {
        type Domain = Domain;
        type Accepted = &'static str;
        type Rejection = &'static str;

        fn dispatch_ledger_transition(
            &mut self,
            _correlation: &CorrelationId<u32>,
            _transition: LedgerTransition<Domain>,
        ) -> CommandResult<Self::Accepted, Self::Rejection> {
            self.0.note("ledger");
            CommandResult::Accepted("transition accepted")
        }
    }

    impl LifecyclePort<u32> for Ports {
        type Domain = Domain;
        type Rejection = &'static str;

        fn dispatch_lifecycle(
            &mut self,
            _correlation: &CorrelationId<u32>,
            control: Lifecycle<Domain>,
        ) -> CommandResult<LifecycleAccepted, Self::Rejection> {
            self.0.note("lifecycle");
            match control {
                Lifecycle::Resume { .. } => CommandResult::Accepted(LifecycleAccepted::Resumed),
                Lifecycle::Pause { .. } => CommandResult::Accepted(LifecycleAccepted::Paused),
            }
        }
    }

    impl QueryPort<u32> for Ports {
        type Name = &'static str;
        type Arguments = &'static str;
        type Limit = u16;
        type Cursor = &'static str;
        type Since = &'static str;
        type Page = QueryPageOf<Domain>;
        type Rejection = &'static str;

        fn dispatch_query(
            &mut self,
            _correlation: &CorrelationId<u32>,
            _query: QueryRequest<
                Self::Name,
                Self::Arguments,
                Self::Limit,
                Self::Cursor,
                Self::Since,
            >,
        ) -> QueryResult<Self::Page, Self::Rejection> {
            self.0.note("query");
            QueryResult::Accepted(QueryPage::new(
                Anchor::LogUpperBound(3),
                ["item"],
                PageEnd::Complete,
                None,
                None,
                None,
            ))
        }

        fn close_query(
            &mut self,
            _correlation: &CorrelationId<u32>,
        ) -> QueryResult<Self::Page, Self::Rejection> {
            self.0.note("query-close");
            QueryResult::Accepted(QueryPage::new(
                Anchor::LogUpperBound(3),
                [],
                PageEnd::Diagnostic { code: "closed" },
                None,
                None,
                None,
            ))
        }
    }

    impl SubscriptionPort<u32> for Ports {
        type Target = Target<&'static str, &'static str>;
        type Credit = Frames;
        type Opened = &'static str;
        type Credited = &'static str;
        type Closed = &'static str;
        type Rejection = &'static str;

        fn dispatch_subscribe(
            &mut self,
            _correlation: &CorrelationId<u32>,
            _subscribe: Subscribe<Self::Target>,
        ) -> CommandResult<Self::Opened, Self::Rejection> {
            self.0.note("subscribe");
            CommandResult::Accepted("opened")
        }

        fn dispatch_credit(
            &mut self,
            _correlation: &CorrelationId<u32>,
            _credit: PositiveCredit<Self::Credit>,
        ) -> CommandResult<Self::Credited, Self::Rejection> {
            self.0.note("credit");
            CommandResult::Rejected("not credit discipline")
        }

        fn dispatch_unsubscribe(
            &mut self,
            _correlation: &CorrelationId<u32>,
        ) -> CommandResult<Self::Closed, Self::Rejection> {
            self.0.note("unsubscribe");
            CommandResult::Accepted("unsubscribed")
        }
    }

    fn handler(calls: &Calls) -> DispatchHandler<Ports, Ports, Ports, Ports, Ports, Ports> {
        DispatchHandler::new(SemanticDispatcher::new(
            Ports(calls.clone()),
            Ports(calls.clone()),
            Ports(calls.clone()),
            Ports(calls.clone()),
            Ports(calls.clone()),
            Ports(calls.clone()),
        ))
    }

    fn kind_of(payload: Option<Payload>) -> Option<Kind<ExperimentalVerb>> {
        payload.map(|payload| payload.kind())
    }

    #[test]
    fn each_wired_request_comes_back_as_its_partitions_result_verb() {
        let calls = Calls::default();
        let mut handler = handler(&calls);
        let correlation = CorrelationId::from_value(1_u32);

        let cases: [(Payload, Kind<ExperimentalVerb>, &'static str); 7] = [
            (
                Payload::Declaration(DeclarationCommand::RetireActor {
                    epoch: "e",
                    id: "n",
                }),
                Kind::Declaration(DeclarationVerb::CommandResult),
                "declaration",
            ),
            (
                Payload::Query(QueryRequest::new(
                    "actors",
                    "args",
                    PageStep::First { limit: 8 },
                    None,
                )),
                Kind::Query(QueryVerb::QueryResult),
                "query",
            ),
            (
                Payload::QueryClose,
                Kind::Query(QueryVerb::QueryResult),
                "query-close",
            ),
            (
                Payload::Inject(Injection::new("mount", "value", "idem")),
                Kind::EventInjection(EventInjectionVerb::InjectAck),
                "injection",
            ),
            (
                Payload::LedgerTransition(LedgerTransition::SetObservationControl { control: "c" }),
                Kind::LedgerTransition(LedgerTransitionVerb::TransitionResult),
                "ledger",
            ),
            (
                Payload::Subscribe(Subscribe::new(Target::new("facts", "args"))),
                Kind::Subscription(SubscriptionVerb::SubscribeAck),
                "subscribe",
            ),
            (
                Payload::Lifecycle(Lifecycle::Resume {
                    expected_authoring_revision: "rev",
                }),
                Kind::Lifecycle(LifecycleVerb::LifecycleResult),
                "lifecycle",
            ),
        ];

        for (request, expected, port) in cases {
            let response = kind_of(handler.handle(&correlation, request));
            assert_eq!(response, Some(expected), "result verb of the {port} port");
        }
        assert_eq!(
            calls.seen(),
            vec![
                "declaration",
                "query",
                "query-close",
                "injection",
                "ledger",
                "subscribe",
                "lifecycle"
            ]
        );
    }

    #[test]
    fn a_rejected_port_result_keeps_the_same_result_verb() {
        let calls = Calls::default();
        let mut handler = handler(&calls);

        let response = handler.handle(
            &CorrelationId::from_value(2_u32),
            Payload::Inject(Injection::new("mount", "value", "idem")),
        );
        assert!(matches!(
            response,
            Some(Payload::InjectAck(CommandResult::Rejected(
                "injection refused"
            )))
        ));
    }

    #[test]
    fn all_three_subscription_requests_come_back_as_the_one_result_verb() {
        let calls = Calls::default();
        let mut handler = handler(&calls);
        let correlation = CorrelationId::from_value(3_u32);

        let requests = [
            Payload::Subscribe(Subscribe::new(Target::new("facts", "args"))),
            Payload::Credit(PositiveCredit::try_new(Frames(2)).expect("positive")),
            Payload::Unsubscribe,
        ];
        for request in requests {
            assert_eq!(
                kind_of(handler.handle(&correlation, request)),
                Some(Kind::Subscription(SubscriptionVerb::SubscribeAck))
            );
        }
        assert_eq!(calls.seen(), vec!["subscribe", "credit", "unsubscribe"]);
    }
}
