use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use super::ids::*;

/// Provider-owned fields retained for round trips and diagnostics but not interpreted by Core.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OpaqueProviderFields(BTreeMap<Box<str>, Box<[u8]>>);

impl OpaqueProviderFields {
    #[must_use]
    pub const fn empty() -> Self {
        Self(BTreeMap::new())
    }

    pub fn try_new<I, K, V>(entries: I) -> Result<Self, ProviderFieldsError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<Box<str>>,
        V: Into<Box<[u8]>>,
    {
        let mut fields = BTreeMap::new();
        for (key, value) in entries {
            let key = key.into();
            validate_label(&key).map_err(ProviderFieldsError::InvalidKey)?;
            if fields.insert(key.clone(), value.into()).is_some() {
                return Err(ProviderFieldsError::DuplicateKey(key));
            }
        }
        Ok(Self(fields))
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &[u8])> {
        self.0
            .iter()
            .map(|(key, value)| (key.as_ref(), value.as_ref()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderFieldsError {
    InvalidKey(PeerTextError),
    DuplicateKey(Box<str>),
}

impl fmt::Display for ProviderFieldsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKey(error) => write!(formatter, "invalid provider field key: {error}"),
            Self::DuplicateKey(key) => write!(formatter, "duplicate provider field key {key:?}"),
        }
    }
}

impl Error for ProviderFieldsError {}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerAddress {
    adapter: PeerAdapterName,
    realm: PeerRealmId,
    peer: PeerId,
}

impl PeerAddress {
    #[must_use]
    pub const fn new(adapter: PeerAdapterName, realm: PeerRealmId, peer: PeerId) -> Self {
        Self {
            adapter,
            realm,
            peer,
        }
    }

    #[must_use]
    pub const fn adapter(&self) -> &PeerAdapterName {
        &self.adapter
    }

    #[must_use]
    pub const fn realm(&self) -> &PeerRealmId {
        &self.realm
    }

    #[must_use]
    pub const fn peer(&self) -> &PeerId {
        &self.peer
    }
}

circular_core::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum PeerKind: u8 {
        Agent = 1 => "agent",
        Session = 2 => "session",
        Thread = 3 => "thread",
        Other = 4 => "other",
    }
}

circular_core::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum PeerAvailability: u8 {
        Reachable = 1 => "reachable",
        Idle = 2 => "idle",
        Busy = 3 => "busy",
        Offline = 4 => "offline",
        Unknown = 5 => "unknown",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PeerCapabilities {
    receive_text: bool,
    idle_wakeup: bool,
    active_turn_inject: bool,
}

impl PeerCapabilities {
    #[must_use]
    pub const fn new(receive_text: bool, idle_wakeup: bool, active_turn_inject: bool) -> Self {
        Self {
            receive_text,
            idle_wakeup,
            active_turn_inject,
        }
    }

    #[must_use]
    pub const fn receive_text(self) -> bool {
        self.receive_text
    }

    #[must_use]
    pub const fn idle_wakeup(self) -> bool {
        self.idle_wakeup
    }

    #[must_use]
    pub const fn active_turn_inject(self) -> bool {
        self.active_turn_inject
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Peer {
    address: PeerAddress,
    display_name: PeerDisplayName,
    kind: PeerKind,
    availability: PeerAvailability,
    capabilities: PeerCapabilities,
}

impl Peer {
    #[must_use]
    pub const fn new(
        address: PeerAddress,
        display_name: PeerDisplayName,
        kind: PeerKind,
        availability: PeerAvailability,
        capabilities: PeerCapabilities,
    ) -> Self {
        Self {
            address,
            display_name,
            kind,
            availability,
            capabilities,
        }
    }

    #[must_use]
    pub const fn address(&self) -> &PeerAddress {
        &self.address
    }

    #[must_use]
    pub const fn display_name(&self) -> &PeerDisplayName {
        &self.display_name
    }

    #[must_use]
    pub const fn kind(&self) -> PeerKind {
        self.kind
    }

    #[must_use]
    pub const fn availability(&self) -> PeerAvailability {
        self.availability
    }

    #[must_use]
    pub const fn capabilities(&self) -> PeerCapabilities {
        self.capabilities
    }
}
