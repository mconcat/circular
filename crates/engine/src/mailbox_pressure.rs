use circular_core::RecordedInstant;
use circular_plan::{EdgeId, NamedActorId};

#[derive(Clone, Debug)]
pub struct MailboxPressure {
    pub actor: NamedActorId,
    pub edge: EdgeId,
    pub depth: usize,
    pub capacity: usize,
    pub overflow: bool,
    pub observed_at: RecordedInstant,
}
