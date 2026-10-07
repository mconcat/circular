
use crate::{
    DeclarationVerb, EstablishedSession, EventInjectionVerb, FeatureSet, Hello, HelloAck, Kind,
    LedgerTransitionVerb, LifecycleVerb, Partition, QueryVerb, ReplayControlVerb,
    SessionMechanicsVerb, SessionRole, SessionRoles, SessionState, SubscriptionVerb,
    TransportTrust,
};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncompleteFeatureSet {
    missing: Box<[Partition]>,
}

impl IncompleteFeatureSet {
    #[must_use]
    pub const fn missing(&self) -> &[Partition] {
        &self.missing
    }
}

impl fmt::Display for IncompleteFeatureSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "session feature set is missing {} partitions: {:?}",
            self.missing.len(),
            self.missing
        )
    }
}

impl std::error::Error for IncompleteFeatureSet {}

pub fn require_complete_features<M>(features: &FeatureSet<M>) -> Result<(), IncompleteFeatureSet> {
    let missing = Partition::ALL
        .into_iter()
        .filter(|partition| features.get(*partition).is_none())
        .collect::<Box<[_]>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(IncompleteFeatureSet { missing })
    }
}

pub trait ScopeCoverage {
    fn covers(&self, target: &Self) -> bool;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RoleRequirement<S> {
    EstablishedSession,
    Reader,
    Writer {
        target: S,
    },
    Operator,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KindRegistrationError {
    OutboundOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KindRegistration<M, S, X> {
    kind: Kind<X>,
    role: RoleRequirement<S>,
    minimum_trust: TransportTrust,
    introduced_minor: M,
}

impl<M, S, X> KindRegistration<M, S, X> {
    const fn fixed(
        kind: Kind<X>,
        role: RoleRequirement<S>,
        minimum_trust: TransportTrust,
        introduced_minor: M,
    ) -> Self {
        Self {
            kind,
            role,
            minimum_trust,
            introduced_minor,
        }
    }

    #[must_use]
    pub const fn experimental(verb: X, introduced_minor: M) -> Self {
        Self::fixed(
            Kind::Experimental(verb),
            RoleRequirement::EstablishedSession,
            TransportTrust::LocalOwner,
            introduced_minor,
        )
    }

    #[must_use]
    pub const fn kind(&self) -> &Kind<X> {
        &self.kind
    }

    #[must_use]
    pub const fn role(&self) -> &RoleRequirement<S> {
        &self.role
    }

    #[must_use]
    pub const fn minimum_trust(&self) -> TransportTrust {
        self.minimum_trust
    }

    #[must_use]
    pub const fn introduced_minor(&self) -> &M {
        &self.introduced_minor
    }
}

impl<M, S, X> KindRegistration<M, S, X> {
    #[must_use]
    pub const fn goodbye(introduced_minor: M) -> Self {
        Self::fixed(
            Kind::SessionMechanics(SessionMechanicsVerb::Goodbye),
            RoleRequirement::EstablishedSession,
            TransportTrust::Remote,
            introduced_minor,
        )
    }

    pub fn declaration(
        verb: DeclarationVerb,
        target: S,
        introduced_minor: M,
    ) -> Result<Self, KindRegistrationError> {
        match verb {
            DeclarationVerb::BeginEpoch
            | DeclarationVerb::ValidateEpoch
            | DeclarationVerb::CommitEpoch
            | DeclarationVerb::AbortEpoch
            | DeclarationVerb::UpsertActor
            | DeclarationVerb::RetireActor
            | DeclarationVerb::UpsertEdge
            | DeclarationVerb::RetireEdge
            | DeclarationVerb::UpsertScope
            | DeclarationVerb::RetireScope
            | DeclarationVerb::UpsertTemplate
            | DeclarationVerb::RetireTemplate
            | DeclarationVerb::MoveToScope
            | DeclarationVerb::UpsertExportMount
            | DeclarationVerb::RetireExportMount
            | DeclarationVerb::UpsertAnnotation
            | DeclarationVerb::RetireAnnotation
            | DeclarationVerb::SetPresentation
            | DeclarationVerb::SetFlags => Ok(Self::fixed(
                Kind::Declaration(verb),
                RoleRequirement::Writer { target },
                TransportTrust::Remote,
                introduced_minor,
            )),
            DeclarationVerb::CommandResult => Err(KindRegistrationError::OutboundOnly),
        }
    }

    pub fn query(verb: QueryVerb, introduced_minor: M) -> Result<Self, KindRegistrationError> {
        match verb {
            QueryVerb::Query | QueryVerb::QueryClose => Ok(Self::fixed(
                Kind::Query(verb),
                RoleRequirement::Reader,
                TransportTrust::Remote,
                introduced_minor,
            )),
            QueryVerb::QueryResult => Err(KindRegistrationError::OutboundOnly),
        }
    }

    pub fn subscription(
        verb: SubscriptionVerb,
        introduced_minor: M,
    ) -> Result<Self, KindRegistrationError> {
        match verb {
            SubscriptionVerb::Subscribe
            | SubscriptionVerb::Credit
            | SubscriptionVerb::Unsubscribe => Ok(Self::fixed(
                Kind::Subscription(verb),
                RoleRequirement::Reader,
                TransportTrust::Remote,
                introduced_minor,
            )),
            SubscriptionVerb::SubscribeAck
            | SubscriptionVerb::Frame
            | SubscriptionVerb::SubscriptionEnded => Err(KindRegistrationError::OutboundOnly),
        }
    }

    pub fn event_injection(
        verb: EventInjectionVerb,
        introduced_minor: M,
    ) -> Result<Self, KindRegistrationError> {
        match verb {
            EventInjectionVerb::Inject => Ok(Self::fixed(
                Kind::EventInjection(verb),
                RoleRequirement::Operator,
                TransportTrust::Remote,
                introduced_minor,
            )),
            EventInjectionVerb::InjectAck => Err(KindRegistrationError::OutboundOnly),
        }
    }

    #[must_use]
    pub const fn approval_decide(introduced_minor: M) -> Self {
        Self::fixed(
            Kind::LedgerTransition(LedgerTransitionVerb::ApprovalDecide),
            RoleRequirement::Operator,
            TransportTrust::LocalOwner,
            introduced_minor,
        )
    }

    #[must_use]
    pub const fn observation_control(introduced_minor: M) -> Self {
        Self::fixed(
            Kind::LedgerTransition(LedgerTransitionVerb::SetObservationControl),
            RoleRequirement::Operator,
            TransportTrust::LocalOwner,
            introduced_minor,
        )
    }

    #[must_use]
    pub const fn agent_harness(introduced_minor: M) -> Self {
        Self::fixed(
            Kind::LedgerTransition(LedgerTransitionVerb::SetAgentHarness),
            RoleRequirement::Operator,
            TransportTrust::LocalOwner,
            introduced_minor,
        )
    }

    pub fn replay_control(
        verb: ReplayControlVerb,
        introduced_minor: M,
    ) -> Result<Self, KindRegistrationError> {
        match verb {
            ReplayControlVerb::ReplayStart
            | ReplayControlVerb::ReplayRewind
            | ReplayControlVerb::ReplayEnd => Ok(Self::fixed(
                Kind::ReplayControl(verb),
                RoleRequirement::Reader,
                TransportTrust::Remote,
                introduced_minor,
            )),
            ReplayControlVerb::ReplayResult => Err(KindRegistrationError::OutboundOnly),
        }
    }

    /// Standing-run lifecycle requests require `Operator` and `LocalOwner`.
    pub fn lifecycle(
        verb: LifecycleVerb,
        introduced_minor: M,
    ) -> Result<Self, KindRegistrationError> {
        match verb {
            LifecycleVerb::Resume | LifecycleVerb::Pause => Ok(Self::fixed(
                Kind::Lifecycle(verb),
                RoleRequirement::Operator,
                TransportTrust::LocalOwner,
                introduced_minor,
            )),
            LifecycleVerb::LifecycleResult => Err(KindRegistrationError::OutboundOnly),
        }
    }
}

#[must_use]
pub fn accepts<M, S, X>(
    registration: &KindRegistration<M, S, X>,
    roles: &SessionRoles<S>,
    trust: TransportTrust,
    features: &FeatureSet<M>,
) -> bool
where
    M: Ord,
    S: Ord + ScopeCoverage,
{
    let role_allows = match registration.role() {
        RoleRequirement::EstablishedSession => true,
        RoleRequirement::Reader => roles.contains(&SessionRole::Reader),
        RoleRequirement::Writer { target } => roles.iter().any(|role| match role {
            SessionRole::Writer { scope } => scope.covers(target),
            _ => false,
        }),
        RoleRequirement::Operator => roles.contains(&SessionRole::Operator),
    };
    let trust_allows = trust.allows(registration.minimum_trust());
    let negotiated = features
        .get(registration.kind().partition())
        .is_some_and(|minor| minor >= registration.introduced_minor());

    role_allows && trust_allows && negotiated
}

pub trait EstablishmentPolicy<S, T> {
    type Error;

    fn establish_roles(
        &mut self,
        requested: &SessionRoles<S>,
        trust: TransportTrust,
    ) -> Result<SessionRoles<S>, Self::Error>;

    fn issue_token(&mut self) -> Result<T, Self::Error>;

    /// Release the established token when its session closes.
    fn session_closed(&mut self) {}
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EstablishmentRejection<E> {
    ProtocolVersionMismatch,
    IncompleteClientFeatures(IncompleteFeatureSet),
    RolePolicy(E),
    GrantedUnrequestedRole,
    TokenIssue(E),
}

pub type ServerHelloAck<V, M, S, T, E> =
    HelloAck<EstablishedSession<V, M, S, T>, EstablishmentRejection<E>>;

pub type ReceiveHelloResult<V, M, S, T, E> =
    Result<ServerHelloAck<V, M, S, T, E>, SessionTransitionError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionPhase {
    AwaitingHello,
    Established,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionTransitionError {
    UnexpectedHello { phase: SessionPhase },
    NotEstablished { phase: SessionPhase },
    UnexpectedGoodbye { phase: SessionPhase },
}

impl fmt::Display for SessionTransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedHello { phase } => {
                write!(formatter, "{phase:?} session does not accept Hello")
            }
            Self::NotEstablished { phase } => {
                write!(
                    formatter,
                    "{phase:?} session does not accept ordinary kinds"
                )
            }
            Self::UnexpectedGoodbye { phase } => {
                write!(formatter, "{phase:?} session does not accept Goodbye")
            }
        }
    }
}

impl std::error::Error for SessionTransitionError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerSession<V, M, S, T> {
    protocol_version: V,
    supported_features: FeatureSet<M>,
    state: SessionState<EstablishedSession<V, M, S, T>>,
}

impl<V, M, S, T> ServerSession<V, M, S, T> {
    pub fn try_new(
        protocol_version: V,
        supported_features: FeatureSet<M>,
    ) -> Result<Self, IncompleteFeatureSet> {
        require_complete_features(&supported_features)?;
        Ok(Self {
            protocol_version,
            supported_features,
            state: SessionState::AwaitingHello,
        })
    }

    #[must_use]
    pub const fn phase(&self) -> SessionPhase {
        match self.state {
            SessionState::AwaitingHello => SessionPhase::AwaitingHello,
            SessionState::Established(_) => SessionPhase::Established,
            SessionState::Closed => SessionPhase::Closed,
        }
    }

    #[must_use]
    pub const fn state(&self) -> &SessionState<EstablishedSession<V, M, S, T>> {
        &self.state
    }

    #[must_use]
    pub const fn established(&self) -> Option<&EstablishedSession<V, M, S, T>> {
        match &self.state {
            SessionState::Established(established) => Some(established),
            SessionState::AwaitingHello | SessionState::Closed => None,
        }
    }

    fn reject<E>(
        &mut self,
        rejection: EstablishmentRejection<E>,
    ) -> HelloAck<EstablishedSession<V, M, S, T>, EstablishmentRejection<E>> {
        self.state = SessionState::Closed;
        HelloAck::Rejected(rejection)
    }

    pub fn receive_hello<Policy>(
        &mut self,
        hello: Hello<V, M, S>,
        trust: TransportTrust,
        policy: &mut Policy,
    ) -> ReceiveHelloResult<V, M, S, T, Policy::Error>
    where
        V: Clone + Eq,
        M: Clone + Ord,
        S: Clone + Ord,
        T: Clone,
        Policy: EstablishmentPolicy<S, T>,
    {
        if self.phase() != SessionPhase::AwaitingHello {
            return Err(SessionTransitionError::UnexpectedHello {
                phase: self.phase(),
            });
        }

        if hello.protocol_version() != &self.protocol_version {
            return Ok(self.reject(EstablishmentRejection::ProtocolVersionMismatch));
        }
        if let Err(error) = require_complete_features(hello.features()) {
            return Ok(self.reject(EstablishmentRejection::IncompleteClientFeatures(error)));
        }

        let negotiated = hello.features().negotiate(&self.supported_features);
        let roles = match policy.establish_roles(hello.requested_roles(), trust) {
            Ok(roles) => roles,
            Err(error) => {
                return Ok(self.reject(EstablishmentRejection::RolePolicy(error)));
            }
        };
        if !roles
            .iter()
            .all(|role| hello.requested_roles().contains(role))
        {
            return Ok(self.reject(EstablishmentRejection::GrantedUnrequestedRole));
        }

        let token = match policy.issue_token() {
            Ok(token) => token,
            Err(error) => {
                return Ok(self.reject(EstablishmentRejection::TokenIssue(error)));
            }
        };

        let established = EstablishedSession::new(
            self.protocol_version.clone(),
            negotiated,
            roles,
            trust,
            token,
        );
        self.state = SessionState::Established(established.clone());
        Ok(HelloAck::Established(established))
    }

    pub fn accepts<X>(
        &self,
        registration: &KindRegistration<M, S, X>,
    ) -> Result<bool, SessionTransitionError>
    where
        M: Ord,
        S: Ord + ScopeCoverage,
    {
        let established = self
            .established()
            .ok_or(SessionTransitionError::NotEstablished {
                phase: self.phase(),
            })?;
        Ok(accepts(
            registration,
            established.roles(),
            established.trust(),
            established.features(),
        ))
    }

    pub fn receive_goodbye(&mut self) -> Result<(), SessionTransitionError> {
        if self.phase() != SessionPhase::Established {
            return Err(SessionTransitionError::UnexpectedGoodbye {
                phase: self.phase(),
            });
        }
        self.state = SessionState::Closed;
        Ok(())
    }

    pub fn transport_closed(&mut self) -> bool {
        if self.phase() == SessionPhase::Closed {
            false
        } else {
            self.state = SessionState::Closed;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EventInjectionVerb, QueryVerb};

    #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
    struct Scope(&'static [u8]);

    impl ScopeCoverage for Scope {
        fn covers(&self, target: &Self) -> bool {
            target.0.starts_with(self.0)
        }
    }

    fn complete_features(default: u8, overrides: &[(Partition, u8)]) -> FeatureSet<u8> {
        FeatureSet::try_new(Partition::ALL.map(|partition| {
            let minor = overrides
                .iter()
                .find_map(|(candidate, minor)| (*candidate == partition).then_some(*minor))
                .unwrap_or(default);
            (partition, minor)
        }))
        .expect("each partition appears once")
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum PolicyError {
        Roles,
    }

    struct TestPolicy {
        grant: SessionRoles<Scope>,
        fail: Option<PolicyError>,
        calls: Vec<&'static str>,
    }

    impl EstablishmentPolicy<Scope, &'static str> for TestPolicy {
        type Error = PolicyError;

        fn establish_roles(
            &mut self,
            _requested: &SessionRoles<Scope>,
            _trust: TransportTrust,
        ) -> Result<SessionRoles<Scope>, Self::Error> {
            self.calls.push("roles");
            if self.fail == Some(PolicyError::Roles) {
                Err(PolicyError::Roles)
            } else {
                Ok(self.grant.clone())
            }
        }

        fn issue_token(&mut self) -> Result<&'static str, Self::Error> {
            self.calls.push("token");
            Ok("new-token")
        }
    }

    fn reader_operator() -> SessionRoles<Scope> {
        SessionRoles::reader_only().with_role(SessionRole::Operator)
    }

    fn hello(
        version: u8,
        features: FeatureSet<u8>,
        roles: SessionRoles<Scope>,
    ) -> Hello<u8, u8, Scope> {
        Hello::new(version, features, roles)
    }

    fn established_session(
        trust: TransportTrust,
        roles: SessionRoles<Scope>,
    ) -> ServerSession<u8, u8, Scope, &'static str> {
        let features = complete_features(0, &[]);
        let mut session =
            ServerSession::try_new(3_u8, features.clone()).expect("complete engine capabilities");
        let mut policy = TestPolicy {
            grant: roles.clone(),
            fail: None,
            calls: Vec::new(),
        };
        let ack = session
            .receive_hello(hello(3, features, roles), trust, &mut policy)
            .expect("first Hello");
        assert!(matches!(ack, HelloAck::Established(_)));
        session
    }

    #[test]
    fn token_source_failure_rejects_without_establishing() {
        struct FailingTokens(Vec<&'static str>);
        impl EstablishmentPolicy<Scope, &'static str> for FailingTokens {
            type Error = &'static str;
            fn establish_roles(
                &mut self,
                roles: &SessionRoles<Scope>,
                _: TransportTrust,
            ) -> Result<SessionRoles<Scope>, Self::Error> {
                self.0.push("roles");
                Ok(roles.clone())
            }
            fn issue_token(&mut self) -> Result<&'static str, Self::Error> {
                self.0.push("token");
                Err("OS entropy unavailable")
            }
        }
        let features = complete_features(0, &[]);
        let mut session = ServerSession::try_new(3_u8, features.clone()).unwrap();
        let mut policy = FailingTokens(Vec::new());
        let answer = session
            .receive_hello(
                hello(3, features, SessionRoles::reader_only()),
                TransportTrust::LocalOwner,
                &mut policy,
            )
            .unwrap();
        assert!(matches!(
            answer,
            HelloAck::Rejected(EstablishmentRejection::TokenIssue("OS entropy unavailable"))
        ));
        assert_eq!(session.phase(), SessionPhase::Closed);
        assert!(session.established().is_none());
        assert_eq!(policy.0, ["roles", "token"]);
    }

    #[test]
    fn negotiation_is_commutative_idempotent_and_partitionwise() {
        for query_left in 0..=3 {
            for query_right in 0..=3 {
                let left = complete_features(2, &[(Partition::Query, query_left)]);
                let right = complete_features(1, &[(Partition::Query, query_right)]);
                assert_eq!(left.negotiate(&right), right.negotiate(&left));
                assert_eq!(left.negotiate(&left), left);
                assert_eq!(
                    left.negotiate(&right).get(Partition::Query),
                    Some(&query_left.min(query_right))
                );
                assert_eq!(left.negotiate(&right).get(Partition::Declaration), Some(&1));
            }
        }
    }

    #[test]
    fn accepts_is_the_product_of_role_trust_and_negotiated_minor() {
        let child = Scope(&[1]);
        let sibling = Scope(&[2]);
        let roles = SessionRoles::reader_only()
            .with_role(SessionRole::Writer {
                scope: child.clone(),
            })
            .with_role(SessionRole::Operator);
        let features = complete_features(0, &[(Partition::Query, 1)]);

        type Registration = KindRegistration<u8, Scope, ()>;

        let child_write = Registration::declaration(DeclarationVerb::UpsertActor, child, 0)
            .expect("request verb");
        let sibling_write = Registration::declaration(DeclarationVerb::UpsertActor, sibling, 0)
            .expect("request verb");
        let approval = Registration::approval_decide(0);
        let future_query = Registration::query(QueryVerb::Query, 2).expect("request verb");

        assert!(accepts(
            &child_write,
            &roles,
            TransportTrust::Remote,
            &features
        ));
        assert!(!accepts(
            &sibling_write,
            &roles,
            TransportTrust::LocalOwner,
            &features
        ));
        assert!(!accepts(
            &approval,
            &roles,
            TransportTrust::LocalUser,
            &features
        ));
        assert!(accepts(
            &approval,
            &roles,
            TransportTrust::LocalOwner,
            &features
        ));
        assert!(!accepts(
            &future_query,
            &roles,
            TransportTrust::LocalOwner,
            &features
        ));
    }

    #[test]
    fn trust_and_role_growth_never_remove_an_acceptance() {
        let registration =
            KindRegistration::<_, Scope, ()>::event_injection(EventInjectionVerb::Inject, 0_u8)
                .expect("request verb");
        let features = complete_features(0, &[]);
        let reader = SessionRoles::<Scope>::reader_only();
        let operator = reader.clone().with_role(SessionRole::Operator);
        let trusts = [
            TransportTrust::Remote,
            TransportTrust::LocalUser,
            TransportTrust::LocalOwner,
        ];

        for trust in trusts {
            assert!(!accepts(&registration, &reader, trust, &features));
            assert!(accepts(&registration, &operator, trust, &features));
        }
    }

    #[test]
    fn fixed_local_owner_kinds_reject_remote_and_local_user_sessions() {
        let roles = SessionRoles::reader_only().with_role(SessionRole::Operator);
        let registrations: [KindRegistration<u8, Scope, ()>; 4] = [
            KindRegistration::approval_decide(0),
            KindRegistration::observation_control(0),
            KindRegistration::agent_harness(0),
            KindRegistration::experimental((), 0),
        ];

        for trust in [TransportTrust::Remote, TransportTrust::LocalUser] {
            let session = established_session(trust, roles.clone());
            for registration in &registrations {
                assert_eq!(registration.minimum_trust(), TransportTrust::LocalOwner);
                assert!(!session.accepts(registration).expect("established session"));
            }
        }

        let owner = established_session(TransportTrust::LocalOwner, roles);
        for registration in &registrations {
            assert!(owner.accepts(registration).expect("established session"));
        }
    }

    #[test]
    fn fixed_registration_constructors_reject_outbound_and_privileged_verbs() {
        assert_eq!(
            KindRegistration::<u8, Scope, ()>::query(QueryVerb::QueryResult, 0),
            Err(KindRegistrationError::OutboundOnly)
        );
        assert_eq!(
            KindRegistration::<u8, Scope, ()>::event_injection(EventInjectionVerb::InjectAck, 0,),
            Err(KindRegistrationError::OutboundOnly)
        );
        assert_eq!(
            KindRegistration::<u8, Scope, ()>::declaration(
                DeclarationVerb::CommandResult,
                Scope(&[]),
                0,
            ),
            Err(KindRegistrationError::OutboundOnly)
        );
    }

    #[test]
    fn local_owner_still_has_to_complete_the_handshake() {
        let session: ServerSession<u8, u8, Scope, &'static str> =
            ServerSession::try_new(3_u8, complete_features(2, &[]))
                .expect("complete engine capabilities");
        let query =
            KindRegistration::<_, Scope, ()>::query(QueryVerb::Query, 0_u8).expect("request verb");

        assert_eq!(
            session.accepts(&query),
            Err(SessionTransitionError::NotEstablished {
                phase: SessionPhase::AwaitingHello
            })
        );
        assert_eq!(session.phase(), SessionPhase::AwaitingHello);
    }

    #[test]
    fn hello_ack_transcript_negotiates_then_establishes_and_goodbye_closes() {
        let mut session =
            ServerSession::try_new(3_u8, complete_features(2, &[(Partition::Query, 4)]))
                .expect("complete engine capabilities");
        let requested = reader_operator();
        let mut policy = TestPolicy {
            grant: reader_operator(),
            fail: None,
            calls: Vec::new(),
        };

        let ack = session
            .receive_hello(
                hello(3, complete_features(1, &[(Partition::Query, 3)]), requested),
                TransportTrust::LocalOwner,
                &mut policy,
            )
            .expect("first Hello");
        let established = match ack {
            HelloAck::Established(established) => established,
            HelloAck::Rejected(error) => panic!("unexpected refusal: {error:?}"),
        };

        assert_eq!(policy.calls, ["roles", "token"]);
        assert_eq!(established.protocol_version(), &3);
        assert_eq!(established.features().get(Partition::Query), Some(&3));
        assert_eq!(established.features().get(Partition::Declaration), Some(&1));
        assert_eq!(established.roles(), &reader_operator());
        assert_eq!(established.trust(), TransportTrust::LocalOwner);
        assert_eq!(established.token(), &"new-token");
        assert_eq!(session.phase(), SessionPhase::Established);

        let duplicate = session.receive_hello(
            hello(3, complete_features(0, &[]), reader_operator()),
            TransportTrust::LocalOwner,
            &mut policy,
        );
        assert_eq!(
            duplicate,
            Err(SessionTransitionError::UnexpectedHello {
                phase: SessionPhase::Established
            })
        );

        session.receive_goodbye().expect("established session");
        assert_eq!(session.phase(), SessionPhase::Closed);
        assert!(!session.transport_closed());
    }

    #[test]
    fn rejection_stops_at_the_first_failed_establishment_step() {
        let mut version_session =
            ServerSession::try_new(3_u8, complete_features(0, &[])).expect("complete capabilities");
        let mut version_policy = TestPolicy {
            grant: reader_operator(),
            fail: None,
            calls: Vec::new(),
        };
        let version_ack = version_session
            .receive_hello(
                hello(2, complete_features(0, &[]), reader_operator()),
                TransportTrust::Remote,
                &mut version_policy,
            )
            .expect("the first Hello produces an Ack");
        assert_eq!(
            version_ack,
            HelloAck::Rejected(EstablishmentRejection::ProtocolVersionMismatch)
        );
        assert!(version_policy.calls.is_empty());
        assert_eq!(version_session.phase(), SessionPhase::Closed);

        let mut role_session =
            ServerSession::try_new(3_u8, complete_features(0, &[])).expect("complete capabilities");
        let mut role_policy = TestPolicy {
            grant: reader_operator(),
            fail: Some(PolicyError::Roles),
            calls: Vec::new(),
        };
        let role_ack = role_session
            .receive_hello(
                hello(3, complete_features(0, &[]), reader_operator()),
                TransportTrust::Remote,
                &mut role_policy,
            )
            .expect("the first Hello produces an Ack");
        assert_eq!(
            role_ack,
            HelloAck::Rejected(EstablishmentRejection::RolePolicy(PolicyError::Roles))
        );
        assert_eq!(role_policy.calls, ["roles"]);
    }

    #[test]
    fn policy_cannot_grant_an_unrequested_role_or_issue_a_token_afterward() {
        let mut session =
            ServerSession::try_new(3_u8, complete_features(0, &[])).expect("complete capabilities");
        let mut policy = TestPolicy {
            grant: reader_operator(),
            fail: None,
            calls: Vec::new(),
        };
        let ack = session
            .receive_hello(
                hello(3, complete_features(0, &[]), SessionRoles::reader_only()),
                TransportTrust::LocalOwner,
                &mut policy,
            )
            .expect("the first Hello produces an Ack");

        assert_eq!(
            ack,
            HelloAck::Rejected(EstablishmentRejection::GrantedUnrequestedRole)
        );
        assert_eq!(policy.calls, ["roles"]);
        assert_eq!(session.phase(), SessionPhase::Closed);
    }
}
