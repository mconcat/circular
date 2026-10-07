//! Owner-local establishment policy over state manifest authority.
use circular_plan::ScopeId;
use circular_protocol::{
    EstablishmentPolicy, SessionRole, SessionRoles, SessionToken, SessionTokenIssueError,
    SessionTokenIssuer, TransportTrust,
};
use std::{
    num::NonZeroUsize,
    sync::{Arc, Mutex},
};

/// Adapter for the protocol's foreign ScopeCoverage trait; identity and
/// structural prefix comparison remain owned by plan::ScopeId.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct DaemonScope(pub(crate) ScopeId);

impl circular_protocol::ScopeCoverage for DaemonScope {
    fn covers(&self, target: &Self) -> bool {
        self.0.is_ancestor_of(&target.0)
    }
}

type Roles = SessionRoles<DaemonScope>;
pub(crate) type OwnerHello = circular_protocol::Hello<u16, u8, DaemonScope>;

impl DaemonScope {
    pub(crate) fn from_wire(
        segments: Vec<circular_protocol::session_payload::ScopeSegment>,
    ) -> Result<Self, circular_protocol::declaration_payload::PayloadRejection> {
        let segments = segments
            .iter()
            .map(circular_runtime::product_identity::scope_segment_from_wire)
            .collect::<Vec<_>>();
        ScopeId::from_segments(segments).map(Self).map_err(|_| {
            circular_protocol::declaration_payload::PayloadRejection::BeyondWidth { key: "scope" }
        })
    }
}

/// Internal decode failures retain role policy separately from malformed bytes.
/// Neither arm is a wire tag or a persisted value.
#[derive(Clone, Copy, Debug)]
pub(crate) enum SessionDecodeRejection {
    Malformed,
    MissingReader,
}

impl From<circular_protocol::declaration_payload::PayloadRejection> for SessionDecodeRejection {
    fn from(_: circular_protocol::declaration_payload::PayloadRejection) -> Self {
        Self::Malformed
    }
}

/// Decode the existing wire carrier into the production FSM's domain.
pub(crate) fn decode_owner_hello(
    header_version: u16,
    bytes: &[u8],
) -> Result<OwnerHello, SessionDecodeRejection> {
    use circular_protocol::declaration_payload::PayloadRejection;
    use circular_protocol::session_payload::{SessionRole as WireRole, decode_hello};
    let decoded = decode_hello(
        bytes,
        circular_core::Ceilings::for_boundary(circular_core::Boundary::Wire),
    )?;
    if decoded.protocol_version != header_version {
        return Err(SessionDecodeRejection::Malformed);
    }
    let roles = decoded
        .requested_roles
        .into_iter()
        .map(|role| {
            Ok(match role {
                WireRole::Reader => SessionRole::Reader,
                WireRole::Writer { scope } => SessionRole::Writer {
                    scope: DaemonScope::from_wire(scope)?,
                },
                WireRole::Operator => SessionRole::Operator,
            })
        })
        .collect::<Result<Vec<_>, PayloadRejection>>()?;
    let roles = Roles::try_from_roles(roles).map_err(|_| SessionDecodeRejection::MissingReader)?;
    Ok(circular_protocol::Hello::new(
        decoded.protocol_version,
        decoded.features,
        roles,
    ))
}

#[derive(Default)]
pub(crate) struct DaemonSessionRegistry {
    issuer: SessionTokenIssuer,
}

impl DaemonSessionRegistry {
    fn issue(&mut self) -> Result<SessionToken, SessionPolicyError> {
        let mut source = engine::OsSessionTokenSource;
        let token = loop {
            match self.issuer.issue(&mut source, NonZeroUsize::MIN) {
                Ok(token) => break token,
                Err(SessionTokenIssueError::Source(_)) => {
                    return Err(SessionPolicyError::EntropyUnavailable);
                }
                Err(SessionTokenIssueError::Exhausted(_)) => continue,
            }
        };
        Ok(token)
    }

    pub(crate) fn retire(&mut self, token: &SessionToken) {
        self.issuer.retire(token);
    }

    #[cfg(test)]
    pub(crate) fn live_count(&self) -> usize {
        self.issuer.live_count()
    }
}

/// These errors contain neither token bytes nor formatted decoder payloads.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionPolicyError {
    OwnerLocalRequired,
    EntropyUnavailable,
}

pub(crate) struct DaemonEstablishmentPolicy {
    registry: Arc<Mutex<DaemonSessionRegistry>>,
    issued: Option<SessionToken>,
}

impl DaemonEstablishmentPolicy {
    pub(crate) fn new(registry: Arc<Mutex<DaemonSessionRegistry>>) -> Self {
        Self {
            registry,
            issued: None,
        }
    }

    fn registry(&self) -> std::sync::MutexGuard<'_, DaemonSessionRegistry> {
        self.registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl EstablishmentPolicy<DaemonScope, SessionToken> for DaemonEstablishmentPolicy {
    type Error = SessionPolicyError;

    fn establish_roles(
        &mut self,
        requested: &Roles,
        trust: TransportTrust,
    ) -> Result<Roles, Self::Error> {
        if trust != TransportTrust::LocalOwner {
            return Err(SessionPolicyError::OwnerLocalRequired);
        }
        Ok(requested.clone())
    }

    fn issue_token(&mut self) -> Result<SessionToken, Self::Error> {
        let token = self.registry().issue()?;
        self.issued = Some(token);
        Ok(token)
    }

    fn session_closed(&mut self) {
        if let Some(token) = self.issued.take() {
            self.registry().retire(&token);
        }
    }
}

impl Drop for DaemonEstablishmentPolicy {
    fn drop(&mut self) {
        if let Some(token) = self.issued.take() {
            self.registry().retire(&token);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_protocol::{
        EstablishmentRejection, FeatureSet, Hello, HelloAck, Partition, ServerSession, SessionPhase,
    };

    fn hello(roles: Roles) -> Hello<u16, u8, DaemonScope> {
        Hello::new(
            1,
            FeatureSet::try_new(Partition::ALL.map(|p| (p, 0_u8))).unwrap(),
            roles,
        )
    }

    fn establish(
        policy: &mut DaemonEstablishmentPolicy,
        roles: Roles,
    ) -> circular_protocol::EstablishedSession<u16, u8, DaemonScope, SessionToken> {
        let mut session = ServerSession::try_new(
            1,
            FeatureSet::try_new(Partition::ALL.map(|p| (p, 0_u8))).unwrap(),
        )
        .unwrap();
        let HelloAck::Established(established) = session
            .receive_hello(hello(roles), TransportTrust::LocalOwner, policy)
            .unwrap()
        else {
            panic!("establishment failed")
        };
        established
    }

    /// The wire carrier still opens into the three published roles, and the
    /// header version must agree with the body.
    #[test]
    fn the_three_published_roles_decode_and_the_version_must_agree() {
        let roles = Roles::reader_only()
            .with_role(SessionRole::Writer {
                scope: DaemonScope(ScopeId::root()),
            })
            .with_role(SessionRole::Operator);
        let wire = circular_protocol::session_payload::hello(
            1,
            0,
            &[
                circular_protocol::session_payload::SessionRole::Reader,
                circular_protocol::session_payload::SessionRole::Writer { scope: vec![] },
                circular_protocol::session_payload::SessionRole::Operator,
            ],
            circular_core::Ceilings::for_boundary(circular_core::Boundary::Wire),
        )
        .unwrap();
        let decoded = decode_owner_hello(1, &wire).unwrap();
        assert_eq!(decoded.requested_roles(), &roles);
        assert!(decode_owner_hello(2, &wire).is_err());
    }

    /// Owner-local trust is the whole admission predicate, and each session gets
    /// its own opaque 32-byte OS token.
    #[test]
    fn owner_local_trust_admits_and_os_tokens_are_real_and_distinct() {
        let registry = Arc::new(Mutex::new(DaemonSessionRegistry::default()));
        let mut first = DaemonEstablishmentPolicy::new(registry.clone());
        let roles = Roles::reader_only().with_role(SessionRole::Operator);
        let accepted = establish(&mut first, roles.clone());
        assert_eq!(accepted.roles(), &roles);
        assert_eq!(accepted.token().as_bytes().len(), 32);
        assert_eq!(format!("{:?}", accepted.token()), "SessionToken(..)");

        let mut other = DaemonEstablishmentPolicy::new(registry.clone());
        let second = establish(&mut other, Roles::reader_only());
        assert_ne!(accepted.token(), second.token());
        assert_eq!(registry.lock().unwrap().issuer.live_count(), 2);

        let declaration = circular_protocol::KindRegistration::<u8, DaemonScope, ()>::declaration(
            circular_protocol::DeclarationVerb::UpsertActor,
            DaemonScope(ScopeId::root()),
            0,
        )
        .unwrap();
        assert!(!circular_protocol::accepts(
            &declaration,
            &Roles::reader_only(),
            accepted.trust(),
            accepted.features()
        ));

        drop(first);
        other.session_closed();
        assert_eq!(registry.lock().unwrap().issuer.live_count(), 0);
    }

    /// A session that is not owner-local never establishes.
    #[test]
    fn a_remote_session_is_refused_before_a_token_is_issued() {
        let registry = Arc::new(Mutex::new(DaemonSessionRegistry::default()));
        let mut policy = DaemonEstablishmentPolicy::new(registry.clone());
        let mut session = ServerSession::try_new(
            1_u16,
            FeatureSet::try_new(Partition::ALL.map(|p| (p, 0_u8))).unwrap(),
        )
        .unwrap();
        assert_eq!(
            session
                .receive_hello(
                    hello(Roles::reader_only()),
                    TransportTrust::Remote,
                    &mut policy
                )
                .unwrap(),
            HelloAck::Rejected(EstablishmentRejection::RolePolicy(
                SessionPolicyError::OwnerLocalRequired
            ))
        );
        assert_eq!(session.phase(), SessionPhase::Closed);
        assert_eq!(registry.lock().unwrap().issuer.live_count(), 0);
    }
}
