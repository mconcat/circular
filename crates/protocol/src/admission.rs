
use crate::{
    CorrelationId, EstablishmentPolicy, EstablishmentRejection, Kind, KindRegistration,
    KindRegistrationError, ScopeCoverage, ServerSession, SessionDomain, SessionEnvelope,
    SessionPayload, SessionPhase, SessionTransitionError, TransportTrust,
};
use std::fmt;

pub type ServerSessionOf<D> = ServerSession<
    <D as SessionDomain>::ProtocolVersion,
    <D as SessionDomain>::FeatureMinor,
    <D as SessionDomain>::Scope,
    <D as SessionDomain>::SessionToken,
>;

pub type KindRegistrationOf<D> = KindRegistration<
    <D as SessionDomain>::FeatureMinor,
    <D as SessionDomain>::Scope,
    <D as SessionDomain>::ExperimentalVerb,
>;

pub trait AdmissionRegistry<D: SessionDomain> {
    fn registration(
        &self,
        payload: &SessionPayload<D>,
    ) -> Result<KindRegistrationOf<D>, KindRegistrationError>;
}

pub trait AdmittedHandler<C, D: SessionDomain> {
    fn handle(
        &mut self,
        correlation: &CorrelationId<C>,
        payload: SessionPayload<D>,
    ) -> Option<SessionPayload<D>>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionRejection {
    Transition(SessionTransitionError),
    OutboundOnly,
    Unauthorized,
    KindMismatch,
}

pub enum Outcome<C, D: SessionDomain> {
    Handshake(SessionEnvelope<C, D>),
    Handled(Option<SessionEnvelope<C, D>>),
    Closed,
    Rejected {
        kind: Kind<D::ExperimentalVerb>,
        rejection: AdmissionRejection,
    },
}

impl<C, D: SessionDomain> fmt::Debug for Outcome<C, D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Handshake(envelope) => {
                write!(formatter, "Outcome::Handshake({:?})", envelope.payload())
            }
            Self::Handled(None) => formatter.write_str("Outcome::Handled(None)"),
            Self::Handled(Some(envelope)) => {
                write!(formatter, "Outcome::Handled({:?})", envelope.payload())
            }
            Self::Closed => formatter.write_str("Outcome::Closed"),
            Self::Rejected { rejection, .. } => {
                write!(formatter, "Outcome::Rejected({rejection:?})")
            }
        }
    }
}

pub struct SessionAdmission<D: SessionDomain, R, P, H> {
    session: ServerSessionOf<D>,
    trust: TransportTrust,
    registry: R,
    policy: P,
    handler: H,
}

impl<D: SessionDomain, R, P, H> fmt::Debug for SessionAdmission<D, R, P, H> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionAdmission")
            .field("phase", &self.session.phase())
            .field("trust", &self.trust)
            .finish_non_exhaustive()
    }
}

impl<D: SessionDomain, R, P, H> SessionAdmission<D, R, P, H> {
    #[must_use]
    pub const fn new(
        session: ServerSessionOf<D>,
        trust: TransportTrust,
        registry: R,
        policy: P,
        handler: H,
    ) -> Self {
        Self {
            session,
            trust,
            registry,
            policy,
            handler,
        }
    }

    #[must_use]
    pub const fn trust(&self) -> TransportTrust {
        self.trust
    }

    #[must_use]
    pub const fn phase(&self) -> SessionPhase {
        self.session.phase()
    }

    pub fn transport_closed(&mut self) -> bool
    where
        P: EstablishmentPolicy<D::Scope, D::SessionToken>,
    {
        let changed = self.session.transport_closed();
        self.policy.session_closed();
        changed
    }

    pub fn receive<C, E>(&mut self, envelope: SessionEnvelope<C, D>) -> Outcome<C, D>
    where
        C: Clone,
        D: SessionDomain<EstablishmentRejection = EstablishmentRejection<E>>,
        D::ProtocolVersion: Clone + Eq,
        D::FeatureMinor: Clone + Ord,
        D::Scope: Clone + Ord + ScopeCoverage,
        D::SessionToken: Clone,
        D::ExperimentalVerb: PartialEq,
        R: AdmissionRegistry<D>,
        P: EstablishmentPolicy<D::Scope, D::SessionToken, Error = E>,
        H: AdmittedHandler<C, D>,
    {
        let (declared, correlation, payload) = envelope.into_parts();
        if payload.kind() != declared {
            return Outcome::Rejected {
                kind: declared,
                rejection: AdmissionRejection::KindMismatch,
            };
        }

        if let SessionPayload::Hello(hello) = payload {
            return match self
                .session
                .receive_hello(hello, self.trust, &mut self.policy)
            {
                Ok(ack) => Outcome::Handshake(
                    SessionPayload::HelloAck(ack).into_envelope(correlation.clone()),
                ),
                Err(error) => Outcome::Rejected {
                    kind: declared,
                    rejection: AdmissionRejection::Transition(error),
                },
            };
        }

        if self.session.phase() != SessionPhase::Established {
            return Outcome::Rejected {
                kind: declared,
                rejection: AdmissionRejection::Transition(SessionTransitionError::NotEstablished {
                    phase: self.session.phase(),
                }),
            };
        }

        let registration = match self.registry.registration(&payload) {
            Ok(registration) => registration,
            Err(KindRegistrationError::OutboundOnly) => {
                return Outcome::Rejected {
                    kind: declared,
                    rejection: AdmissionRejection::OutboundOnly,
                };
            }
        };
        match self.session.accepts(&registration) {
            Ok(true) => {}
            Ok(false) => {
                return Outcome::Rejected {
                    kind: declared,
                    rejection: AdmissionRejection::Unauthorized,
                };
            }
            Err(error) => {
                return Outcome::Rejected {
                    kind: declared,
                    rejection: AdmissionRejection::Transition(error),
                };
            }
        }

        if matches!(payload, SessionPayload::Goodbye) {
            return match self.session.receive_goodbye() {
                Ok(()) => {
                    self.policy.session_closed();
                    Outcome::Closed
                }
                Err(error) => Outcome::Rejected {
                    kind: declared,
                    rejection: AdmissionRejection::Transition(error),
                },
            };
        }

        Outcome::Handled(
            self.handler
                .handle(&correlation, payload)
                .map(|response| response.into_envelope(correlation)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Anchor, CommandResult, DeclarationCommand, DeclarationDomain, DeclarationVerb,
        EventInjectionVerb, ExperimentalPayload, FeatureSet, Hello, HelloAck, Injection, Kind,
        LedgerDomain, LifecycleDomain, Partition, PositiveFrameCount, QueryRequest, QueryVerb,
        ReplayControlPayload, ReplayControlVerb, SessionRole, SessionRoles,
        SubscriptionTerminationDomain, Target,
    };
    use std::cell::RefCell;
    use std::convert::Infallible;
    use std::rc::Rc;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Domain;

    #[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
    struct Scope(&'static str);

    impl ScopeCoverage for Scope {
        fn covers(&self, target: &Self) -> bool {
            self.0 == "root" || self.0 == target.0
        }
    }

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
        type Scope = Scope;
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
        type Scope = Scope;
        type SessionToken = u32;
        type EstablishmentRejection = EstablishmentRejection<Infallible>;

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

    struct Policy;

    impl EstablishmentPolicy<Scope, u32> for Policy {
        type Error = Infallible;

        fn establish_roles(
            &mut self,
            requested: &SessionRoles<Scope>,
            _trust: TransportTrust,
        ) -> Result<SessionRoles<Scope>, Self::Error> {
            Ok(requested.clone())
        }

        fn issue_token(&mut self) -> Result<u32, Self::Error> {
            Ok(7)
        }
    }

    struct Registry;

    impl AdmissionRegistry<Domain> for Registry {
        fn registration(
            &self,
            payload: &Payload,
        ) -> Result<KindRegistrationOf<Domain>, KindRegistrationError> {
            match payload {
                Payload::Goodbye => Ok(KindRegistration::goodbye(0)),
                Payload::Declaration(command) => {
                    let target = match command {
                        DeclarationCommand::BeginEpoch { scope, .. } => *scope,
                        _ => Scope("root"),
                    };
                    KindRegistration::declaration(command.verb(), target, 0)
                }
                Payload::Query(_) => KindRegistration::query(QueryVerb::Query, 0),
                Payload::Inject(_) => {
                    KindRegistration::event_injection(EventInjectionVerb::Inject, 0)
                }
                Payload::CommandResult(_) | Payload::QueryResult(_) | Payload::InjectAck(_) => {
                    Err(KindRegistrationError::OutboundOnly)
                }
                _ => Err(KindRegistrationError::OutboundOnly),
            }
        }
    }

    #[derive(Clone, Default)]
    struct Log(Rc<RefCell<Vec<Kind<ExperimentalVerb>>>>);

    impl Log {
        fn seen(&self) -> Vec<Kind<ExperimentalVerb>> {
            self.0.borrow().clone()
        }
    }

    impl AdmittedHandler<u32, Domain> for Log {
        fn handle(
            &mut self,
            _correlation: &CorrelationId<u32>,
            payload: Payload,
        ) -> Option<Payload> {
            self.0.borrow_mut().push(payload.kind());
            Some(Payload::CommandResult(CommandResult::Accepted("accepted")))
        }
    }

    fn complete_features() -> FeatureSet<u8> {
        FeatureSet::try_new(Partition::ALL.map(|partition| (partition, 0)))
            .expect("unique partition")
    }

    fn gate(log: Log) -> SessionAdmission<Domain, Registry, Policy, Log> {
        let session =
            ServerSession::try_new(1_u8, complete_features()).expect("complete capability set");
        SessionAdmission::new(session, TransportTrust::LocalOwner, Registry, Policy, log)
    }

    fn hello(roles: SessionRoles<Scope>) -> Payload {
        Payload::Hello(Hello::new(1, complete_features(), roles))
    }

    fn writer_roles() -> SessionRoles<Scope> {
        SessionRoles::reader_only().with_role(SessionRole::Writer {
            scope: Scope("root"),
        })
    }

    fn envelope(payload: Payload, correlation: u32) -> SessionEnvelope<u32, Domain> {
        payload.into_envelope(CorrelationId::from_value(correlation))
    }

    fn establish(
        admission: &mut SessionAdmission<Domain, Registry, Policy, Log>,
        roles: SessionRoles<Scope>,
    ) {
        let outcome = admission.receive(envelope(hello(roles), 1));
        match outcome {
            Outcome::Handshake(reply) => assert!(matches!(
                reply.payload(),
                Payload::HelloAck(HelloAck::Established(_))
            )),
            other => panic!("establishment must succeed: {other:?}"),
        }
    }

    fn upsert_actor() -> Payload {
        Payload::Declaration(DeclarationCommand::UpsertActor {
            epoch: "e",
            id: "n",
            declaration: "d",
        })
    }

    #[test]
    fn nothing_but_hello_reaches_the_handler_before_establishment() {
        let log = Log::default();
        let mut admission = gate(log.clone());

        for payload in [
            upsert_actor(),
            Payload::Query(QueryRequest::new(
                "actors",
                "args",
                crate::PageStep::First { limit: 4 },
                None,
            )),
            Payload::Inject(Injection::new("mount", "value", "idem")),
            Payload::Goodbye,
        ] {
            let outcome = admission.receive(envelope(payload, 2));
            assert!(
                matches!(
                    outcome,
                    Outcome::Rejected {
                        rejection: AdmissionRejection::Transition(
                            SessionTransitionError::NotEstablished { .. }
                        ),
                        ..
                    }
                ),
                "no ordinary kind is accepted before establishment: {outcome:?}"
            );
        }
        assert!(log.seen().is_empty(), "the handler must not be called");
    }

    #[test]
    fn a_role_that_the_session_does_not_hold_stops_the_request_at_the_gate() {
        let log = Log::default();
        let mut admission = gate(log.clone());
        establish(&mut admission, SessionRoles::reader_only());

        let outcome = admission.receive(envelope(upsert_actor(), 3));
        assert_eq!(
            outcome_rejection(outcome),
            Some(AdmissionRejection::Unauthorized)
        );
        assert!(log.seen().is_empty(), "the handler must not be called");
    }

    #[test]
    fn a_server_only_verb_arriving_inbound_stops_at_the_gate() {
        let log = Log::default();
        let mut admission = gate(log.clone());
        establish(&mut admission, writer_roles());

        let outcome = admission.receive(envelope(
            Payload::CommandResult(CommandResult::Accepted("forged")),
            4,
        ));
        assert_eq!(
            outcome_rejection(outcome),
            Some(AdmissionRejection::OutboundOnly)
        );
        assert!(log.seen().is_empty());
    }

    #[test]
    fn a_head_that_disagrees_with_its_body_is_rejected_before_anything_else() {
        let log = Log::default();
        let mut admission = gate(log.clone());
        establish(&mut admission, writer_roles());

        let forged = crate::Envelope::new(
            Kind::Query(QueryVerb::Query),
            CorrelationId::from_value(5_u32),
            upsert_actor(),
        );
        let outcome = admission.receive(forged);
        assert_eq!(
            outcome_kind(outcome),
            Some(Kind::Query(QueryVerb::Query)),
            "a mismatched envelope carries the kind its header claims; the side the body decides is the value that envelope itself denies"
        );

        let again = crate::Envelope::new(
            Kind::Query(QueryVerb::Query),
            CorrelationId::from_value(5_u32),
            upsert_actor(),
        );
        assert_eq!(
            outcome_rejection(admission.receive(again)),
            Some(AdmissionRejection::KindMismatch)
        );
        assert!(log.seen().is_empty());
    }

    #[test]
    fn a_second_hello_is_refused_and_the_first_session_is_unaffected() {
        let log = Log::default();
        let mut admission = gate(log.clone());
        establish(&mut admission, writer_roles());

        let outcome = admission.receive(envelope(hello(writer_roles()), 6));
        assert_eq!(
            outcome_rejection(outcome),
            Some(AdmissionRejection::Transition(
                SessionTransitionError::UnexpectedHello {
                    phase: SessionPhase::Established,
                }
            ))
        );
        assert_eq!(admission.phase(), SessionPhase::Established);
    }

    #[test]
    fn goodbye_closes_without_a_reply_and_closes_the_gate_for_good() {
        let log = Log::default();
        let mut admission = gate(log.clone());
        establish(&mut admission, writer_roles());

        assert!(matches!(
            admission.receive(envelope(Payload::Goodbye, 7)),
            Outcome::Closed
        ));
        assert_eq!(admission.phase(), SessionPhase::Closed);

        let outcome = admission.receive(envelope(upsert_actor(), 8));
        assert_eq!(
            outcome_rejection(outcome),
            Some(AdmissionRejection::Transition(
                SessionTransitionError::NotEstablished {
                    phase: SessionPhase::Closed,
                }
            ))
        );
        assert!(log.seen().is_empty());
    }

    #[test]
    fn a_rejected_hello_leaves_a_closed_session_and_a_reply_on_the_same_key() {
        let log = Log::default();
        let mut admission = gate(log.clone());

        let mismatched = Payload::Hello(Hello::new(2, complete_features(), writer_roles()));
        match admission.receive(envelope(mismatched, 9)) {
            Outcome::Handshake(reply) => {
                assert_eq!(reply.correlation().value(), &9);
                assert!(matches!(
                    reply.payload(),
                    Payload::HelloAck(HelloAck::Rejected(
                        EstablishmentRejection::ProtocolVersionMismatch
                    ))
                ));
            }
            other => panic!("a refusal is a HelloAck envelope too: {other:?}"),
        }
        assert_eq!(admission.phase(), SessionPhase::Closed);
        assert!(log.seen().is_empty());
    }

    fn outcome_rejection<C>(outcome: Outcome<C, Domain>) -> Option<AdmissionRejection> {
        match outcome {
            Outcome::Rejected { rejection, .. } => Some(rejection),
            _ => None,
        }
    }

    fn outcome_kind<C>(outcome: Outcome<C, Domain>) -> Option<Kind<ExperimentalVerb>> {
        match outcome {
            Outcome::Rejected { kind, .. } => Some(kind),
            _ => None,
        }
    }
}
