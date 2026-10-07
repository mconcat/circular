use circular_protocol::TransportTrust;
use std::fmt;

/// A connectable local endpoint.  In-process transport is deliberately absent.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum LocalEndpoint<P, A> {
    OwnerLocal { path: P },
    UserLocal { address: A },
}

/// Validated owner-local IPC evidence.  Invalid raw observations cannot inhabit
/// this type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerLocalEvidence<U, P> {
    path: P,
    peer_subject: U,
}

impl<U: Eq, P> OwnerLocalEvidence<U, P> {
    pub fn try_new(
        path: P,
        peer_subject: U,
        path_owner: U,
        owner_only_access: bool,
    ) -> Result<Self, LocalEvidenceError> {
        if peer_subject != path_owner {
            return Err(LocalEvidenceError::PeerDoesNotOwnPath);
        }
        if !owner_only_access {
            return Err(LocalEvidenceError::PathNotOwnerOnly);
        }
        Ok(Self { path, peer_subject })
    }

    #[must_use]
    pub const fn peer_subject(&self) -> &U {
        &self.peer_subject
    }
}

/// Validated user-local IPC evidence.  A non-local raw peer observation cannot
/// inhabit this type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserLocalEvidence<U, A> {
    address: A,
    peer_subject: U,
}

impl<U, A> UserLocalEvidence<U, A> {
    pub fn try_new(
        address: A,
        peer_subject: U,
        same_machine: bool,
    ) -> Result<Self, LocalEvidenceError> {
        if !same_machine {
            return Err(LocalEvidenceError::PeerNotLocal);
        }
        Ok(Self {
            address,
            peer_subject,
        })
    }

    #[must_use]
    pub const fn peer_subject(&self) -> &U {
        &self.peer_subject
    }
}

/// Local IPC evidence checked before a connection may become established.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LocalEndpointEvidence<U, P, A> {
    OwnerLocal(OwnerLocalEvidence<U, P>),
    UserLocal(UserLocalEvidence<U, A>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalEvidenceError {
    EndpointEvidenceMismatch,
    EvidenceForDifferentEndpoint,
    PeerDoesNotOwnPath,
    PathNotOwnerOnly,
    PeerNotLocal,
}

impl fmt::Display for LocalEvidenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EndpointEvidenceMismatch => {
                formatter.write_str("local endpoint and peer evidence kinds do not match")
            }
            Self::EvidenceForDifferentEndpoint => {
                formatter.write_str("local peer evidence was collected for a different endpoint")
            }
            Self::PeerDoesNotOwnPath => {
                formatter.write_str("verified peer does not own the owner-local path")
            }
            Self::PathNotOwnerOnly => {
                formatter.write_str("owner-local path is not restricted to its owner")
            }
            Self::PeerNotLocal => {
                formatter.write_str("user-local peer is not verified on the same machine")
            }
        }
    }
}

impl std::error::Error for LocalEvidenceError {}

/// Derives a local connection's trust solely from endpoint evidence and a
/// caller-supplied deployment ceiling.  A ceiling can lower but never raise the
/// endpoint's evidence-derived trust.
pub fn establish_local_trust<P: Eq, A: Eq, U: Eq>(
    endpoint: &LocalEndpoint<P, A>,
    evidence: &LocalEndpointEvidence<U, P, A>,
    ceiling: TransportTrust,
) -> Result<TransportTrust, LocalEvidenceError> {
    let evidence_trust = match (endpoint, evidence) {
        (LocalEndpoint::OwnerLocal { path }, LocalEndpointEvidence::OwnerLocal(evidence)) => {
            if path != &evidence.path {
                return Err(LocalEvidenceError::EvidenceForDifferentEndpoint);
            }
            TransportTrust::LocalOwner
        }
        (LocalEndpoint::UserLocal { address }, LocalEndpointEvidence::UserLocal(evidence)) => {
            if address != &evidence.address {
                return Err(LocalEvidenceError::EvidenceForDifferentEndpoint);
            }
            TransportTrust::LocalUser
        }
        _ => return Err(LocalEvidenceError::EndpointEvidenceMismatch),
    };

    Ok(std::cmp::min(evidence_trust, ceiling))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_and_ceiling_are_the_only_trust_inputs() {
        let endpoint: LocalEndpoint<&str, &str> = LocalEndpoint::OwnerLocal { path: "sock" };
        let evidence = LocalEndpointEvidence::OwnerLocal(
            OwnerLocalEvidence::try_new("sock", 7_u8, 7_u8, true).expect("valid evidence"),
        );
        for _unrelated_metadata in 0..32 {
            assert_eq!(
                establish_local_trust(&endpoint, &evidence, TransportTrust::LocalUser),
                Ok(TransportTrust::LocalUser)
            );
        }
        assert_eq!(
            establish_local_trust(&endpoint, &evidence, TransportTrust::LocalOwner),
            Ok(TransportTrust::LocalOwner)
        );
        assert_eq!(
            establish_local_trust(&endpoint, &evidence, TransportTrust::Remote),
            Ok(TransportTrust::Remote)
        );
    }

    #[test]
    fn wrong_or_insufficient_local_evidence_is_rejected() {
        let owner: LocalEndpoint<&str, &str> = LocalEndpoint::OwnerLocal { path: "sock" };
        let user: LocalEndpoint<&str, &str> = LocalEndpoint::UserLocal { address: "addr" };
        assert_eq!(
            OwnerLocalEvidence::try_new("sock", 1_u8, 2_u8, true),
            Err(LocalEvidenceError::PeerDoesNotOwnPath)
        );
        assert_eq!(
            UserLocalEvidence::try_new("addr", 1_u8, false),
            Err(LocalEvidenceError::PeerNotLocal)
        );

        let user_evidence = LocalEndpointEvidence::UserLocal(
            UserLocalEvidence::try_new("different", 1_u8, true).expect("local evidence"),
        );
        assert_eq!(
            establish_local_trust(&user, &user_evidence, TransportTrust::LocalOwner),
            Err(LocalEvidenceError::EvidenceForDifferentEndpoint)
        );

        assert_eq!(
            establish_local_trust(&owner, &user_evidence, TransportTrust::LocalOwner),
            Err(LocalEvidenceError::EndpointEvidenceMismatch)
        );
    }
}
