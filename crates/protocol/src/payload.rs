
use crate::{
    CommandResult, CorrelationId, DeclarationCommand, DeclarationDomain, Envelope,
    EstablishedSession, EventInjectionVerb, Hello, HelloAck, Injection, Kind, LedgerDomain,
    LedgerTransition, LedgerTransitionVerb, Lifecycle, LifecycleAccepted, LifecycleDomain,
    LifecycleVerb, PositiveCredit, PositiveFrameCount, QueryPage, QueryRequest, QueryResult,
    QueryVerb, ReplayControlVerb, SessionMechanicsVerb, Subscribe, SubscriptionEnded,
    SubscriptionFrame, SubscriptionTerminationDomain, SubscriptionVerb,
};
use std::fmt;

pub trait ReplayControlPayload {
    fn verb(&self) -> ReplayControlVerb;
}

pub trait ExperimentalPayload {
    type Verb;

    fn verb(&self) -> Self::Verb;
}

pub trait SessionDomain {
    type ProtocolVersion;
    type FeatureMinor;
    type Scope;
    type SessionToken;
    type EstablishmentRejection;

    type Declaration: DeclarationDomain;
    type DeclarationAccepted;
    type DeclarationRejection;

    type QueryName;
    type QueryArguments;
    type PageLimit;
    type PageCursor;
    type QuerySince;
    type QueryAnchor;
    type QueryItem;
    type PageCut;
    type PageReached;
    type PageDiagnostic;
    type QueryRejection;

    type SubscriptionTarget;
    type Credit: PositiveFrameCount;
    type SubscriptionOpened;
    type SubscriptionRejection;
    type ConflationSlot;
    type FramePayload;
    type SubscriptionAnchor;
    type SubscriptionDiagnostic;
    type SubscriptionTermination: SubscriptionTerminationDomain;

    type InjectionMount;
    type InjectionPayload;
    type IdempotencyKey;
    type InjectionAccepted;
    type InjectionRejection;

    type Ledger: LedgerDomain;
    type TransitionAccepted;
    type TransitionRejection;

    type Lifecycle: LifecycleDomain;
    type LifecycleRejection;

    type ReplayControl: ReplayControlPayload;
    type ExperimentalVerb;
    type Experimental: ExperimentalPayload<Verb = Self::ExperimentalVerb>;
}

pub type HelloOf<D> = Hello<
    <D as SessionDomain>::ProtocolVersion,
    <D as SessionDomain>::FeatureMinor,
    <D as SessionDomain>::Scope,
>;

pub type SubscriptionEndedOf<D> = SubscriptionEnded<
    <D as SessionDomain>::SubscriptionTermination,
    <D as SessionDomain>::SubscriptionAnchor,
    <D as SessionDomain>::SubscriptionDiagnostic,
>;

pub type QueryPageOf<D> = QueryPage<
    <D as SessionDomain>::QueryAnchor,
    <D as SessionDomain>::QueryItem,
    <D as SessionDomain>::PageCursor,
    <D as SessionDomain>::PageDiagnostic,
    <D as SessionDomain>::PageCut,
    <D as SessionDomain>::PageReached,
>;

pub type QueryRequestOf<D> = QueryRequest<
    <D as SessionDomain>::QueryName,
    <D as SessionDomain>::QueryArguments,
    <D as SessionDomain>::PageLimit,
    <D as SessionDomain>::PageCursor,
    <D as SessionDomain>::QuerySince,
>;

/// The closed accepted result carried by a lifecycle response.
pub type LifecycleAcceptedOf = LifecycleAccepted;

pub type EstablishedSessionOf<D> = EstablishedSession<
    <D as SessionDomain>::ProtocolVersion,
    <D as SessionDomain>::FeatureMinor,
    <D as SessionDomain>::Scope,
    <D as SessionDomain>::SessionToken,
>;

pub enum SessionPayload<D: SessionDomain> {
    Hello(HelloOf<D>),
    HelloAck(HelloAck<EstablishedSessionOf<D>, D::EstablishmentRejection>),
    Goodbye,
    Declaration(DeclarationCommand<D::Declaration>),
    CommandResult(CommandResult<D::DeclarationAccepted, D::DeclarationRejection>),
    Query(QueryRequestOf<D>),
    QueryResult(QueryResult<QueryPageOf<D>, D::QueryRejection>),
    QueryClose,
    Subscribe(Subscribe<D::SubscriptionTarget>),
    SubscribeAck(CommandResult<D::SubscriptionOpened, D::SubscriptionRejection>),
    Credit(PositiveCredit<D::Credit>),
    Unsubscribe,
    Frame(SubscriptionFrame<D::ConflationSlot, D::FramePayload>),
    SubscriptionEnded(SubscriptionEndedOf<D>),
    Inject(Injection<D::InjectionMount, D::InjectionPayload, D::IdempotencyKey>),
    InjectAck(CommandResult<D::InjectionAccepted, D::InjectionRejection>),
    LedgerTransition(LedgerTransition<D::Ledger>),
    TransitionResult(CommandResult<D::TransitionAccepted, D::TransitionRejection>),
    ReplayControl(D::ReplayControl),
    Experimental(D::Experimental),
    Lifecycle(Lifecycle<D::Lifecycle>),
    LifecycleResult(CommandResult<LifecycleAcceptedOf, D::LifecycleRejection>),
}

impl<D: SessionDomain> SessionPayload<D> {
    #[must_use]
    pub fn kind(&self) -> Kind<D::ExperimentalVerb> {
        match self {
            Self::Hello(_) => Kind::SessionMechanics(SessionMechanicsVerb::Hello),
            Self::HelloAck(_) => Kind::SessionMechanics(SessionMechanicsVerb::HelloAck),
            Self::Goodbye => Kind::SessionMechanics(SessionMechanicsVerb::Goodbye),
            Self::Declaration(command) => Kind::Declaration(command.verb()),
            Self::CommandResult(_) => Kind::Declaration(crate::DeclarationVerb::CommandResult),
            Self::Query(_) => Kind::Query(QueryVerb::Query),
            Self::QueryResult(_) => Kind::Query(QueryVerb::QueryResult),
            Self::QueryClose => Kind::Query(QueryVerb::QueryClose),
            Self::Subscribe(_) => Kind::Subscription(SubscriptionVerb::Subscribe),
            Self::SubscribeAck(_) => Kind::Subscription(SubscriptionVerb::SubscribeAck),
            Self::Credit(_) => Kind::Subscription(SubscriptionVerb::Credit),
            Self::Unsubscribe => Kind::Subscription(SubscriptionVerb::Unsubscribe),
            Self::Frame(_) => Kind::Subscription(SubscriptionVerb::Frame),
            Self::SubscriptionEnded(_) => Kind::Subscription(SubscriptionVerb::SubscriptionEnded),
            Self::Inject(_) => Kind::EventInjection(EventInjectionVerb::Inject),
            Self::InjectAck(_) => Kind::EventInjection(EventInjectionVerb::InjectAck),
            Self::LedgerTransition(transition) => Kind::LedgerTransition(transition.verb()),
            Self::TransitionResult(_) => {
                Kind::LedgerTransition(LedgerTransitionVerb::TransitionResult)
            }
            Self::ReplayControl(payload) => Kind::ReplayControl(payload.verb()),
            Self::Experimental(payload) => Kind::Experimental(payload.verb()),
            Self::Lifecycle(control) => Kind::Lifecycle(control.verb()),
            Self::LifecycleResult(_) => Kind::Lifecycle(LifecycleVerb::LifecycleResult),
        }
    }

    #[must_use]
    pub fn into_envelope<C>(self, correlation: CorrelationId<C>) -> SessionEnvelope<C, D> {
        let kind = self.kind();
        Envelope::new(kind, correlation, self)
    }
}

impl<D: SessionDomain> fmt::Debug for SessionPayload<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hello(_) => formatter.write_str("SessionPayload::Hello"),
            Self::HelloAck(_) => formatter.write_str("SessionPayload::HelloAck"),
            Self::Goodbye => formatter.write_str("SessionPayload::Goodbye"),
            Self::Declaration(command) => {
                write!(
                    formatter,
                    "SessionPayload::Declaration({:?})",
                    command.verb()
                )
            }
            Self::CommandResult(_) => formatter.write_str("SessionPayload::CommandResult"),
            Self::Query(_) => formatter.write_str("SessionPayload::Query"),
            Self::QueryResult(_) => formatter.write_str("SessionPayload::QueryResult"),
            Self::QueryClose => formatter.write_str("SessionPayload::QueryClose"),
            Self::Subscribe(_) => formatter.write_str("SessionPayload::Subscribe"),
            Self::SubscribeAck(_) => formatter.write_str("SessionPayload::SubscribeAck"),
            Self::Credit(_) => formatter.write_str("SessionPayload::Credit"),
            Self::Unsubscribe => formatter.write_str("SessionPayload::Unsubscribe"),
            Self::Frame(_) => formatter.write_str("SessionPayload::Frame"),
            Self::SubscriptionEnded(_) => formatter.write_str("SessionPayload::SubscriptionEnded"),
            Self::Inject(_) => formatter.write_str("SessionPayload::Inject"),
            Self::InjectAck(_) => formatter.write_str("SessionPayload::InjectAck"),
            Self::LedgerTransition(transition) => write!(
                formatter,
                "SessionPayload::LedgerTransition({:?})",
                transition.verb()
            ),
            Self::TransitionResult(_) => formatter.write_str("SessionPayload::TransitionResult"),
            Self::ReplayControl(payload) => write!(
                formatter,
                "SessionPayload::ReplayControl({:?})",
                payload.verb()
            ),
            Self::Experimental(_) => formatter.write_str("SessionPayload::Experimental"),
            Self::Lifecycle(control) => {
                write!(formatter, "SessionPayload::Lifecycle({:?})", control.verb())
            }
            Self::LifecycleResult(_) => formatter.write_str("SessionPayload::LifecycleResult"),
        }
    }
}

pub type SessionEnvelope<C, D> =
    Envelope<C, SessionPayload<D>, <D as SessionDomain>::ExperimentalVerb>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Anchor, DeclarationVerb, FeatureSet, FrameOrigin, PageEnd, PageStep, Partition,
        STABLE_VERB_COUNT, SessionRoles, SubscriptionEndReason, SubscriptionTerminationDomain,
        Target,
    };
    use std::collections::BTreeSet;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestDomain;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Termination;

    impl SubscriptionTerminationDomain for Termination {
        type ResetFloorOrCursor = &'static str;
        type StructureCursor = &'static str;
        type AuthoringEnvironment = &'static str;
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestFrames(u16);

    impl PositiveFrameCount for TestFrames {
        fn is_positive(&self) -> bool {
            self.0 > 0
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestReplay(ReplayControlVerb);

    impl ReplayControlPayload for TestReplay {
        fn verb(&self) -> ReplayControlVerb {
            self.0
        }
    }

    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    struct TestExperimentalVerb(u8);

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestExperimental(TestExperimentalVerb);

    impl ExperimentalPayload for TestExperimental {
        type Verb = TestExperimentalVerb;

        fn verb(&self) -> Self::Verb {
            self.0
        }
    }

    impl DeclarationDomain for TestDomain {
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

    impl LedgerDomain for TestDomain {
        type ApprovalItem = &'static str;
        type ApprovalDecision = &'static str;
        type ObservationControl = &'static str;
        type AgentHarness = &'static str;
    }

    impl LifecycleDomain for TestDomain {
        type AuthoringRevision = &'static str;
    }

    impl SessionDomain for TestDomain {
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
        type Credit = TestFrames;
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

        type ReplayControl = TestReplay;
        type ExperimentalVerb = TestExperimentalVerb;
        type Experimental = TestExperimental;
    }

    type Payload = SessionPayload<TestDomain>;

    fn declaration_witnesses() -> Vec<DeclarationCommand<TestDomain>> {
        vec![
            DeclarationCommand::BeginEpoch {
                scope: "root",
                commit_id: "commit",
                expected_revision: "revision",
                expected_environment: "environment",
            },
            DeclarationCommand::ValidateEpoch { epoch: "e" },
            DeclarationCommand::CommitEpoch { epoch: "e" },
            DeclarationCommand::AbortEpoch { epoch: "e" },
            DeclarationCommand::UpsertActor {
                epoch: "e",
                id: "n",
                declaration: "d",
            },
            DeclarationCommand::RetireActor {
                epoch: "e",
                id: "n",
            },
            DeclarationCommand::UpsertEdge {
                epoch: "e",
                id: "g",
                attributes: "a",
            },
            DeclarationCommand::RetireEdge {
                epoch: "e",
                id: "g",
            },
            DeclarationCommand::UpsertScope {
                epoch: "e",
                segment: "s",
                declaration: "scope-declaration",
            },
            DeclarationCommand::RetireScope {
                epoch: "e",
                segment: "s",
            },
            DeclarationCommand::MoveToScope {
                epoch: "e",
                actors: vec!["n"],
                target: "s",
            },
            DeclarationCommand::UpsertExportMount {
                epoch: "e",
                name: "x",
                mount: "m",
            },
            DeclarationCommand::RetireExportMount {
                epoch: "e",
                name: "x",
            },
            DeclarationCommand::UpsertAnnotation {
                epoch: "e",
                id: "a",
                annotation: "v",
            },
            DeclarationCommand::RetireAnnotation {
                epoch: "e",
                id: "a",
            },
            DeclarationCommand::SetPresentation {
                epoch: "e",
                owner: "n",
                presentation: "p",
            },
            DeclarationCommand::SetFlags {
                epoch: "e",
                actor: "n",
                flags: "f",
            },
            DeclarationCommand::UpsertTemplate {
                epoch: "e",
                name: "worker".to_owned(),
                commands: vec![],
            },
            DeclarationCommand::RetireTemplate {
                epoch: "e",
                name: "worker".to_owned(),
            },
        ]
    }

    fn stable_witnesses() -> Vec<Payload> {
        let mut witnesses = vec![
            Payload::Hello(Hello::new(
                1,
                FeatureSet::try_new([]).expect("unique partition"),
                SessionRoles::reader_only(),
            )),
            Payload::HelloAck(HelloAck::Rejected("refusal")),
            Payload::Goodbye,
        ];
        witnesses.extend(
            declaration_witnesses()
                .into_iter()
                .map(Payload::Declaration),
        );
        witnesses.extend([
            Payload::CommandResult(CommandResult::Accepted("accepted")),
            Payload::Query(QueryRequest::new(
                "actors",
                "args",
                PageStep::First { limit: 16 },
                Some("cut"),
            )),
            Payload::QueryResult(QueryResult::Accepted(QueryPage::new(
                Anchor::LogUpperBound(7),
                ["item"],
                PageEnd::Complete,
                Some("cut"),
                None,
                Some("reached"),
            ))),
            Payload::QueryClose,
            Payload::Subscribe(Subscribe::new(Target::new("facts", "args"))),
            Payload::SubscribeAck(CommandResult::Accepted("opened")),
            Payload::Credit(PositiveCredit::try_new(TestFrames(4)).expect("positive")),
            Payload::Unsubscribe,
            Payload::Frame(SubscriptionFrame::Lossless {
                origin: FrameOrigin::Live,
                payload: "frame",
            }),
            Payload::SubscriptionEnded(SubscriptionEnded::new(
                SubscriptionEndReason::ByClient,
                "diag",
                5,
            )),
            Payload::Inject(Injection::new("mount", "value", "idem")),
            Payload::InjectAck(CommandResult::Accepted("accepted")),
            Payload::LedgerTransition(LedgerTransition::ApprovalDecide {
                item: "i",
                decision: "d",
            }),
            Payload::LedgerTransition(LedgerTransition::SetObservationControl { control: "c" }),
            Payload::LedgerTransition(LedgerTransition::SetAgentHarness { harness: "h" }),
            Payload::TransitionResult(CommandResult::Accepted("accepted")),
            Payload::Lifecycle(Lifecycle::Resume {
                expected_authoring_revision: "rev",
            }),
            Payload::Lifecycle(Lifecycle::Pause { mode: None }),
            Payload::LifecycleResult(CommandResult::Accepted(LifecycleAccepted::Resumed)),
        ]);
        witnesses
            .extend(ReplayControlVerb::ALL.map(|verb| Payload::ReplayControl(TestReplay(verb))));
        witnesses
    }

    fn expected_stable_kinds() -> BTreeSet<Kind<TestExperimentalVerb>> {
        let mut kinds = BTreeSet::new();
        kinds.extend(SessionMechanicsVerb::ALL.map(Kind::SessionMechanics));
        kinds.extend(DeclarationVerb::ALL.map(Kind::Declaration));
        kinds.extend(QueryVerb::ALL.map(Kind::Query));
        kinds.extend(SubscriptionVerb::ALL.map(Kind::Subscription));
        kinds.extend(EventInjectionVerb::ALL.map(Kind::EventInjection));
        kinds.extend(LedgerTransitionVerb::ALL.map(Kind::LedgerTransition));
        kinds.extend(ReplayControlVerb::ALL.map(Kind::ReplayControl));
        kinds.extend(LifecycleVerb::ALL.map(Kind::Lifecycle));
        kinds
    }

    #[test]
    fn the_union_covers_the_stable_verb_exact_set_once_each() {
        let witnesses = stable_witnesses();
        assert_eq!(
            witnesses.len(),
            STABLE_VERB_COUNT,
            "one witness must cover one verb"
        );

        let produced = witnesses
            .iter()
            .map(SessionPayload::kind)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            produced.len(),
            STABLE_VERB_COUNT,
            "if two witnesses yield the same Kind, the mapping is not injective"
        );
        assert_eq!(
            produced,
            expected_stable_kinds(),
            "the Kind range of the union must be the exact published set"
        );
    }

    #[test]
    fn every_stable_partition_is_reachable_from_the_union() {
        let reached = stable_witnesses()
            .iter()
            .map(|payload| payload.kind().partition())
            .collect::<BTreeSet<_>>();
        let stable = Partition::ALL
            .into_iter()
            .filter(|partition| *partition != Partition::Experimental)
            .collect::<BTreeSet<_>>();

        assert_eq!(reached, stable);
        assert_eq!(reached.len(), Partition::COUNT - 1);
    }

    #[test]
    fn the_experimental_hole_carries_its_own_verb() {
        let payload = Payload::Experimental(TestExperimental(TestExperimentalVerb(3)));

        assert_eq!(payload.kind(), Kind::Experimental(TestExperimentalVerb(3)));
        assert_eq!(payload.kind().partition(), Partition::Experimental);
    }

    #[test]
    fn sealing_derives_the_kind_from_the_payload() {
        let payload = Payload::Declaration(DeclarationCommand::RetireActor {
            epoch: "e",
            id: "n",
        });
        let expected = payload.kind();
        let envelope = payload.into_envelope(CorrelationId::from_value(9_u32));

        assert_eq!(envelope.kind(), &expected);
        assert_eq!(
            envelope.kind(),
            &Kind::Declaration(DeclarationVerb::RetireActor)
        );
        assert_eq!(envelope.correlation().value(), &9);
    }

    #[test]
    fn a_multi_verb_variant_reports_the_verb_its_body_names() {
        for command in declaration_witnesses() {
            let verb = command.verb();
            assert_eq!(
                Payload::Declaration(command).kind(),
                Kind::Declaration(verb),
                "the body decides the Kind of a declaration variant"
            );
        }
    }
}
