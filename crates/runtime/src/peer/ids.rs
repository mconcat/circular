use std::error::Error;
use std::fmt;

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(Box<[u8]>);

        impl $name {
            pub fn try_new(bytes: impl Into<Box<[u8]>>) -> Result<Self, EmptyPeerIdentifier> {
                let bytes = bytes.into();
                if bytes.is_empty() {
                    Err(EmptyPeerIdentifier)
                } else {
                    Ok(Self(bytes))
                }
            }

            #[must_use]
            pub const fn as_bytes(&self) -> &[u8] {
                &self.0
            }
        }
    };
}

opaque_id!(PeerRealmId);
opaque_id!(PeerId);
opaque_id!(PeerMessageId);
opaque_id!(ProviderMessageId);
opaque_id!(PeerBindingId);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmptyPeerIdentifier;

impl fmt::Display for EmptyPeerIdentifier {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("peer identifiers must not be empty")
    }
}

impl Error for EmptyPeerIdentifier {}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerAdapterName(Box<str>);

impl PeerAdapterName {
    pub fn try_new(value: impl Into<Box<str>>) -> Result<Self, PeerTextError> {
        let value = value.into();
        validate_label(&value)?;
        Ok(Self(value))
    }

    #[must_use]
    pub const fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerDisplayName(Box<str>);

impl PeerDisplayName {
    pub fn try_new(value: impl Into<Box<str>>) -> Result<Self, PeerTextError> {
        let value = value.into();
        validate_label(&value)?;
        Ok(Self(value))
    }

    #[must_use]
    pub const fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PeerBody(Box<str>);

impl PeerBody {
    pub fn try_new(value: impl Into<Box<str>>) -> Result<Self, PeerTextError> {
        let value = value.into();
        if value.is_empty() {
            return Err(PeerTextError::Empty);
        }
        if value.contains('\0') {
            return Err(PeerTextError::ContainsNul);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerTextError {
    Empty,
    SurroundingWhitespace,
    ContainsControl,
    ContainsNul,
}

impl fmt::Display for PeerTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "peer text must not be empty",
            Self::SurroundingWhitespace => "peer labels must not contain surrounding whitespace",
            Self::ContainsControl => "peer labels must not contain control characters",
            Self::ContainsNul => "peer message bodies must not contain NUL",
        })
    }
}

impl Error for PeerTextError {}

pub(super) fn validate_label(value: &str) -> Result<(), PeerTextError> {
    if value.is_empty() {
        return Err(PeerTextError::Empty);
    }
    if value.trim() != value {
        return Err(PeerTextError::SurroundingWhitespace);
    }
    if value.chars().any(char::is_control) {
        return Err(PeerTextError::ContainsControl);
    }
    Ok(())
}
