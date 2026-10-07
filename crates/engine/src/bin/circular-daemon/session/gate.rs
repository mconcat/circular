//! The owner socket's sole command entry is the existing SessionAdmission.
use super::super::session_policy::{
    DaemonEstablishmentPolicy, DaemonScope, DaemonSessionRegistry, SessionDecodeRejection,
    SessionPolicyError,
};
use super::*;
use circular_protocol::{self as p, declaration_payload as w};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Debug)]
pub(super) enum CommandGateFailure {
    MalformedPayload,
    HelloRequired,
    SessionClosed,
    OutboundOnly,
    RoleInsufficient,
    LiveCorrelationUnavailable,
}
impl CommandGateFailure {
    const fn reason(self) -> RejectionReason {
        match self {
            Self::MalformedPayload => RejectionReason::MalformedPayload,
            Self::HelloRequired => RejectionReason::HelloRequired,
            Self::SessionClosed => RejectionReason::SessionClosed,
            Self::OutboundOnly => RejectionReason::OutboundOnly,
            Self::RoleInsufficient => RejectionReason::RoleInsufficient,
            Self::LiveCorrelationUnavailable => RejectionReason::LiveCorrelationUnavailable,
        }
    }
    fn rejected(self, verb: StableVerb) -> w::Rejected {
        let reason = self.reason();
        let message = if matches!(
            (self, verb),
            (
                Self::RoleInsufficient,
                StableVerb::SessionMechanics(SessionMechanicsVerb::Hello)
            )
        ) {
            "Hello requested_roles must include Reader"
        } else {
            reason.message()
        };
        reason.reject(verb.partition(), message)
    }
}
#[derive(Clone, Copy, Debug)]
pub(super) enum HelloGateFailure {
    UnexpectedHello,
    ProtocolVersionMismatch,
    IncompleteFeatures,
    OwnerLocalRequired,
    EntropyUnavailable,
    GrantedUnrequestedRole,
}
impl HelloGateFailure {
    const fn reason(self) -> RejectionReason {
        match self {
            Self::UnexpectedHello => RejectionReason::UnexpectedHello,
            Self::ProtocolVersionMismatch => RejectionReason::ProtocolVersionMismatch,
            Self::IncompleteFeatures => RejectionReason::IncompleteFeatures,
            Self::OwnerLocalRequired => RejectionReason::OwnerLocalRequired,
            Self::EntropyUnavailable => RejectionReason::EntropyUnavailable,
            Self::GrantedUnrequestedRole => RejectionReason::GrantedUnrequestedRole,
        }
    }
}
pub(super) fn answer_rejection(
    stream: &mut impl circular_transport::LocalByteStream<Error = std::io::Error>,
    header: EnvelopeHeader,
    failure: CommandGateFailure,
) {
    let verb = match header.verb() {
        StableVerb::SessionMechanics(_) => HELLO_ACK,
        StableVerb::Declaration(_) => COMMAND_RESULT,
        StableVerb::EventInjection(_) => INJECT_ACK,
        StableVerb::LedgerTransition(_) => TRANSITION_RESULT,
        StableVerb::Lifecycle(_) => LIFECYCLE_RESULT,
        StableVerb::Query(_) => QUERY_RESULT,
        StableVerb::Subscription(_) => StableVerb::Subscription(SubscriptionVerb::SubscribeAck),
        StableVerb::ReplayControl(_) => StableVerb::ReplayControl(ReplayControlVerb::ReplayResult),
    };
    answer_with(
        stream,
        header,
        verb,
        CommandResult::Rejected(failure.rejected(header.verb())),
    );
}
pub(super) fn answer_hello_rejection(
    stream: &mut impl circular_transport::LocalByteStream<Error = std::io::Error>,
    header: EnvelopeHeader,
    failure: HelloGateFailure,
) {
    let reason = failure.reason();
    answer_with(
        stream,
        header,
        HELLO_ACK,
        CommandResult::Rejected(reason.reject(p::Partition::SessionMechanics, reason.message())),
    );
}
fn policy_error(error: SessionPolicyError) -> HelloGateFailure {
    match error {
        SessionPolicyError::OwnerLocalRequired => HelloGateFailure::OwnerLocalRequired,
        SessionPolicyError::EntropyUnavailable => HelloGateFailure::EntropyUnavailable,
    }
}
fn handshake_error(error: p::EstablishmentRejection<SessionPolicyError>) -> HelloGateFailure {
    match error {
        p::EstablishmentRejection::ProtocolVersionMismatch => {
            HelloGateFailure::ProtocolVersionMismatch
        }
        p::EstablishmentRejection::IncompleteClientFeatures(_) => {
            HelloGateFailure::IncompleteFeatures
        }
        p::EstablishmentRejection::RolePolicy(e) | p::EstablishmentRejection::TokenIssue(e) => {
            policy_error(e)
        }
        p::EstablishmentRejection::GrantedUnrequestedRole => {
            HelloGateFailure::GrantedUnrequestedRole
        }
    }
}

pub(super) struct Request {
    header: EnvelopeHeader,
    canonical_payload: Vec<u8>,
}

pub(super) struct State<'a, S> {
    pub push: S,
    pub request: Option<Request>,
    pub world: &'a crate::daemon::read_world::SharedWorld,
    pub store: &'a authoring_store::AuthoringStore,
    pub execution: &'a ProductExecutionProfile,
    pub candidate: SessionEpoch,
    pub subscriptions: Subscriptions,
    pub queries: OpenQueries,
    pub correlations: p::LiveCorrelations,
    pub lenses: Lenses,
    pub time: Arc<crate::daemon::replay_clock::SystemTime>,
    pub wake: Arc<engine::wake::Wake>,
    pub registry: Arc<Mutex<DaemonSessionRegistry>>,
}
impl<'a, S> State<'a, S> {
    pub(super) fn stand(
        push: S,
        world: &'a crate::daemon::read_world::SharedWorld,
        store: &'a authoring_store::AuthoringStore,
        execution: &'a ProductExecutionProfile,
        registry: Arc<Mutex<DaemonSessionRegistry>>,
        time: Arc<crate::daemon::replay_clock::SystemTime>,
    ) -> std::io::Result<Self> {
        let wake = Arc::new(engine::wake::Wake::new()?);
        world.register_session_wake(&wake);
        Ok(Self {
            push,
            request: None,
            world,
            store,
            execution,
            candidate: SessionEpoch::Idle,
            subscriptions: Subscriptions::default(),
            queries: OpenQueries::default(),
            correlations: p::LiveCorrelations::default(),
            lenses: Lenses::default(),
            time,
            wake,
            registry,
        })
    }
}
pub(super) struct Registry<'a, S>(Rc<RefCell<State<'a, S>>>);
pub(super) struct Handler<'a, S>(Rc<RefCell<State<'a, S>>>);
pub(super) type Gate<'a, S> =
    p::SessionAdmission<domain::Domain, Registry<'a, S>, DaemonEstablishmentPolicy, Handler<'a, S>>;

impl<S> p::AdmissionRegistry<domain::Domain> for Registry<'_, S> {
    fn registration(
        &self,
        payload: &domain::Payload,
    ) -> Result<p::KindRegistrationOf<domain::Domain>, p::KindRegistrationError> {
        use p::KindRegistration as K;
        use p::SessionPayload as P;
        match payload {
            P::Goodbye => Ok(K::goodbye(0)),
            P::Declaration(command) => {
                let target = match command {
                    domain::Command::BeginEpoch { scope, .. } => match scope {
                        w::AddressRef::Absolute(s)
                        | w::AddressRef::EpochLocal(s)
                        | w::AddressRef::Relative(s) => s.clone(),
                    },
                    _ => self.0.borrow().candidate.active_scope().unwrap_or_default(),
                };
                let target = DaemonScope::from_wire(target)
                    .map_err(|_| p::KindRegistrationError::OutboundOnly)?;
                K::declaration(command.verb(), target, 0)
            }
            P::Query(_) => K::query(QueryVerb::Query, 0),
            P::QueryClose => K::query(QueryVerb::QueryClose, 0),
            P::Subscribe(_) => K::subscription(SubscriptionVerb::Subscribe, 0),
            P::Credit(_) => K::subscription(SubscriptionVerb::Credit, 0),
            P::Unsubscribe => K::subscription(SubscriptionVerb::Unsubscribe, 0),
            P::Inject(_) => K::event_injection(EventInjectionVerb::Inject, 0),
            P::LedgerTransition(p::LedgerTransition::ApprovalDecide { .. }) => {
                Ok(K::approval_decide(0))
            }
            P::LedgerTransition(p::LedgerTransition::SetObservationControl { .. }) => {
                Ok(K::observation_control(0))
            }
            P::LedgerTransition(p::LedgerTransition::SetAgentHarness { .. }) => {
                Ok(K::agent_harness(0))
            }
            P::Lifecycle(control) => K::lifecycle(control.verb(), 0),
            P::ReplayControl(replay) => K::replay_control(replay.verb, 0),
            _ => Err(p::KindRegistrationError::OutboundOnly),
        }
    }
}
impl<S: circular_transport::LocalByteStream<Error = std::io::Error>>
    p::AdmittedHandler<u32, domain::Domain> for Handler<'_, S>
{
    fn handle(
        &mut self,
        correlation: &p::CorrelationId<u32>,
        payload: domain::Payload,
    ) -> Option<domain::Payload> {
        let mut state = self.0.borrow_mut();
        let State {
            push,
            request,
            world,
            store,
            execution,
            candidate,
            subscriptions,
            queries,
            correlations,
            lenses,
            time,
            wake,
            ..
        } = &mut *state;
        let request = request
            .take()
            .expect("the byte boundary supplied this request");
        let header = request.header;
        debug_assert_eq!(header.correlation(), *correlation.value());
        let answer = dispatch_typed(
            push,
            header,
            &request.canonical_payload,
            payload,
            world,
            store,
            execution,
            candidate,
            subscriptions,
            queries,
            lenses,
            super::ReplayClock {
                time,
                wake,
                correlations,
            },
        );
        let answered = !matches!(answer, CommandAnswer::Unanswered);
        if matches!(&answer, CommandAnswer::Query(result)
            if result.encode(Ceilings::for_boundary(Boundary::Wire)).is_err()
                || !matches!(result, w::QueryResult::Page(w::QueryPage { terminal: w::Terminal::More(_), .. })))
        {
            queries.finish(header.correlation());
        }
        write_answer(push, header, answer);
        if answered
            && !queries.contains(header.correlation())
            && !subscriptions.contains(header.correlation())
            && !lenses.contains(header.correlation())
        {
            correlations.finish(header.correlation());
        }
        None
    }
}

pub(super) fn poll<S: circular_transport::LocalByteStream<Error = std::io::Error>>(
    state: &Rc<RefCell<State<'_, S>>>,
) {
    let mut s = state.borrow_mut();
    let State {
        push,
        subscriptions,
        world,
        store,
        correlations,
        lenses,
        time,
        wake,
        ..
    } = &mut *s;
    lenses.drain(world.read().server.as_deref(), time, wake);
    for correlation in poll_subscriptions(push, subscriptions, world, store, lenses) {
        correlations.finish(correlation);
    }
}

pub(super) fn new<S>(state: Rc<RefCell<State<'_, S>>>) -> Gate<'_, S> {
    let policy = DaemonEstablishmentPolicy::new(state.borrow().registry.clone());
    let features = p::FeatureSet::try_new(
        p::Partition::ALL
            .into_iter()
            .map(|partition| (partition, 0_u8)),
    )
    .unwrap();
    p::SessionAdmission::new(
        p::ServerSession::try_new(p::INITIAL_PROTOCOL_VERSION, features).unwrap(),
        p::TransportTrust::LocalOwner,
        Registry(state.clone()),
        policy,
        Handler(state),
    )
}
pub(super) fn receive<S: circular_transport::LocalByteStream<Error = std::io::Error>>(
    gate: &mut Gate<'_, S>,
    state: &Rc<RefCell<State<'_, S>>>,
    header: EnvelopeHeader,
    bytes: &[u8],
) -> bool {
    use p::SessionPayload as P;
    let epoch = state.borrow().candidate.active_id();
    let payload = match domain::decode(header, bytes, epoch) {
        Ok(payload) => payload,
        Err(error) => {
            let failure = match error {
                SessionDecodeRejection::Malformed => CommandGateFailure::MalformedPayload,
                SessionDecodeRejection::MissingReader => CommandGateFailure::RoleInsufficient,
            };
            let mut s = state.borrow_mut();
            answer_rejection(&mut s.push, header, failure);
            s.queries.finish(header.correlation());
            if !s.subscriptions.contains(header.correlation())
                && !s.lenses.contains(header.correlation())
            {
                s.correlations.finish(header.correlation());
            }
            if header.verb() == StableVerb::SessionMechanics(SessionMechanicsVerb::Hello) {
                drop(s);
                gate.transport_closed();
                return false;
            }
            return gate.phase() == p::SessionPhase::Established;
        }
    };
    {
        let mut s = state.borrow_mut();
        let correlation = header.correlation();
        let continuation = match &payload {
            P::Query(query) => {
                matches!(query.page(), p::PageStep::Continue { .. })
                    && s.queries.contains(correlation)
            }
            P::QueryClose => s.queries.contains(correlation),
            P::Credit(_) | P::Unsubscribe => s.subscriptions.contains(correlation),
            P::ReplayControl(replay) => {
                matches!(
                    replay.verb,
                    p::ReplayControlVerb::ReplayRewind | p::ReplayControlVerb::ReplayEnd
                ) && s.lenses.contains(correlation)
            }
            _ => false,
        };
        if !continuation && !s.correlations.try_open(correlation) {
            answer_rejection(
                &mut s.push,
                header,
                CommandGateFailure::LiveCorrelationUnavailable,
            );
            return gate.phase() == p::SessionPhase::Established;
        }
    }
    state.borrow_mut().request = Some(Request {
        header,
        canonical_payload: if matches!(&payload, P::Declaration(_)) {
            bytes.to_vec()
        } else {
            Vec::new()
        },
    });
    let outcome =
        gate.receive(payload.into_envelope(p::CorrelationId::from_value(header.correlation())));
    state.borrow_mut().request = None;
    let keep = match outcome {
        p::Outcome::Handshake(envelope) => {
            let (_, _, payload) = envelope.into_parts();
            let mut s = state.borrow_mut();
            match payload {
                P::HelloAck(p::HelloAck::Established(established)) => {
                    let roles = wire_roles(established.roles());
                    let body = p::session_payload::established(
                        *established.protocol_version(),
                        established.features(),
                        &roles,
                        p::session_payload::TransportTrust::LocalOwner,
                        established.token(),
                        Ceilings::for_boundary(Boundary::Wire),
                    )
                    .expect("established values fit the decoded Hello ceilings");
                    write_envelope(
                        &mut s.push,
                        OwnerLocalChannelId::new(1),
                        header.protocol_version(),
                        HELLO_ACK,
                        header.correlation(),
                        &body,
                    )
                    .is_ok()
                }
                P::HelloAck(p::HelloAck::Rejected(error)) => {
                    answer_hello_rejection(&mut s.push, header, handshake_error(error));
                    false
                }
                _ => unreachable!("the gate emits only HelloAck for a handshake"),
            }
        }
        p::Outcome::Handled(_) => return true,
        p::Outcome::Closed => false,
        p::Outcome::Rejected { rejection, .. } => {
            let failure = match rejection {
                p::AdmissionRejection::Transition(p::SessionTransitionError::UnexpectedHello {
                    ..
                }) => Err(HelloGateFailure::UnexpectedHello),
                p::AdmissionRejection::Transition(p::SessionTransitionError::NotEstablished {
                    phase: p::SessionPhase::AwaitingHello,
                }) => Ok(CommandGateFailure::HelloRequired),
                p::AdmissionRejection::Transition(_) => Ok(CommandGateFailure::SessionClosed),
                p::AdmissionRejection::OutboundOnly => Ok(CommandGateFailure::OutboundOnly),
                p::AdmissionRejection::Unauthorized => Ok(CommandGateFailure::RoleInsufficient),
                p::AdmissionRejection::KindMismatch => Ok(CommandGateFailure::MalformedPayload),
            };
            let mut s = state.borrow_mut();
            match failure {
                Ok(failure) => answer_rejection(&mut s.push, header, failure),
                Err(failure) => answer_hello_rejection(&mut s.push, header, failure),
            }
            s.queries.finish(header.correlation());
            gate.phase() == p::SessionPhase::Established
        }
    };
    let mut s = state.borrow_mut();
    if !s.queries.contains(header.correlation())
        && !s.subscriptions.contains(header.correlation())
        && !s.lenses.contains(header.correlation())
    {
        s.correlations.finish(header.correlation());
    }
    keep
}
fn wire_roles(roles: &p::SessionRoles<DaemonScope>) -> Vec<p::session_payload::SessionRole> {
    use p::session_payload::SessionRole as W;
    roles
        .iter()
        .map(|role| match role {
            p::SessionRole::Reader => W::Reader,
            p::SessionRole::Writer { scope } => W::Writer {
                scope: circular_runtime::product_identity::wire_scope(&scope.0),
            },
            p::SessionRole::Operator => W::Operator,
        })
        .collect()
}

