
use circular_core::{ActorType, Value};
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub struct FoldedConfig {
    actor_type: ActorType,
    value: Value,
}

impl FoldedConfig {
    #[must_use]
    pub const fn minted(actor_type: ActorType, value: Value) -> Self {
        Self { actor_type, value }
    }

    #[must_use]
    pub const fn actor_type(&self) -> ActorType {
        self.actor_type
    }

    #[must_use]
    pub const fn value(&self) -> &Value {
        &self.value
    }

    pub fn for_type(&self, expected: ActorType) -> Result<&Value, FoldedConfigMismatch> {
        if self.actor_type == expected {
            Ok(&self.value)
        } else {
            Err(FoldedConfigMismatch {
                expected,
                found: self.actor_type,
            })
        }
    }

    #[must_use]
    pub fn into_value(self) -> Value {
        self.value
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FoldedConfigMismatch {
    expected: ActorType,
    found: ActorType,
}

impl FoldedConfigMismatch {
    #[must_use]
    pub const fn expected(self) -> ActorType {
        self.expected
    }

    #[must_use]
    pub const fn found(self) -> ActorType {
        self.found
    }
}

impl fmt::Display for FoldedConfigMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "folded config belongs to `{}` but `{}` tried to read it",
            self.found.as_str(),
            self.expected.as_str()
        )
    }
}

impl std::error::Error for FoldedConfigMismatch {}
