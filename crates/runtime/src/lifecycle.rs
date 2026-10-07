
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConfigChangeOutcome {
    Absorbed,
    #[default]
    ReplaceIncarnation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActorState<V> {
    schema: V,
    bytes: Box<[u8]>,
}

impl<V> ActorState<V> {
    #[must_use]
    pub fn new(schema: V, bytes: impl Into<Box<[u8]>>) -> Self {
        Self {
            schema,
            bytes: bytes.into(),
        }
    }

    #[must_use]
    pub const fn schema(&self) -> &V {
        &self.schema
    }

    #[must_use]
    pub const fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn into_parts(self) -> (V, Box<[u8]>) {
        (self.schema, self.bytes)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Checkpoint<V, I> {
    state: Option<ActorState<V>>,
    pending: BTreeSet<I>,
}

impl<V, I: Ord> Checkpoint<V, I> {
    #[must_use]
    pub fn new(state: Option<ActorState<V>>, pending: impl IntoIterator<Item = I>) -> Self {
        Self {
            state,
            pending: pending.into_iter().collect(),
        }
    }

    #[must_use]
    pub const fn state(&self) -> Option<&ActorState<V>> {
        self.state.as_ref()
    }

    #[must_use]
    pub const fn pending(&self) -> &BTreeSet<I> {
        &self.pending
    }

    #[must_use]
    pub fn into_parts(self) -> (Option<ActorState<V>>, BTreeSet<I>) {
        (self.state, self.pending)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[repr(isize)]
pub enum ActorRestoreError<V> {
    SchemaBeyondLadder { schema: V } = 0,
    DecodeFailed { schema: V } = 1,
    StateInvariantViolated { schema: V } = 3,
}

impl<V> ActorRestoreError<V> {
    #[must_use]
    pub const fn schema(&self) -> &V {
        match self {
            Self::SchemaBeyondLadder { schema }
            | Self::DecodeFailed { schema }
            | Self::StateInvariantViolated { schema } => schema,
        }
    }
}

pub trait EditableActor {
    type StateVersion: Clone;
    type EffectId: Clone + Ord;

    fn on_config_change(&mut self, _config: &crate::FoldedConfig) -> ConfigChangeOutcome {
        ConfigChangeOutcome::ReplaceIncarnation
    }

    /// The container folds the actual, recorded disposition before another input.
    fn on_instance_disposition(
        &mut self,
        _intent: &crate::InstanceIntent,
        _disposition: crate::InstanceDisposition,
    ) -> Option<crate::ScheduleSpec> {
        None
    }

    /// The container's absorbed idle TTL and current correlation, for its cell owner.
    fn instance_expiry(&self, _intent: &crate::InstanceIntent) -> Option<crate::ScheduleSpec> {
        None
    }

    /// Owned cells that must retire when this container's key meaning changes.
    fn instance_retirements(&self) -> Vec<crate::InstanceIntent> {
        Vec::new()
    }

    fn checkpoint(&self) -> Option<ActorState<Self::StateVersion>> {
        None
    }

    /// Disposable caches may skip an encoding failure. State hand-over keeps the
    /// existing `checkpoint` contract; `Ok(None)` still means no available codec.
    fn try_checkpoint(
        &self,
    ) -> Result<Option<ActorState<Self::StateVersion>>, circular_core::CodecError> {
        Ok(self.checkpoint())
    }

    fn stateless(&self) -> bool {
        false
    }

    fn restore(
        &mut self,
        _state: ActorState<Self::StateVersion>,
    ) -> Result<(), ActorRestoreError<Self::StateVersion>> {
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CheckpointRestore<V, I> {
    NotAvailable,
    Restored { pending: BTreeSet<I> },
    Rejected { error: ActorRestoreError<V> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_change_requires_restart() {
        assert_eq!(
            ConfigChangeOutcome::default(),
            ConfigChangeOutcome::ReplaceIncarnation
        );
    }
}
