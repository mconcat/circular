use circular_store::SqliteJournal;
use std::path::Path;

/// Capture once per daemon invocation. Wall time is observation data only;
/// neither this value nor physical commit sequence drives actor scheduling.
pub struct RestartBoot {
    boot_id: [u8; 16],
    wall_millis: u64,
}
impl RestartBoot {
    pub fn capture_for(boot_id: [u8; 16]) -> Result<Self, String> {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| format!("restart wall clock precedes Unix epoch: {error}"))?
            .as_millis();
        Ok(Self {
            boot_id,
            wall_millis: u64::try_from(millis).map_err(|_| "restart wall clock overflow")?,
        })
    }

    /// Assembly reads the exact pre-boot prefix. The System actor receives the
    /// resulting body; this path never opens a writer or issues a stamp.
    pub fn read_after(
        &self,
        state_directory: &Path,
        horizon: u64,
    ) -> Result<RestartRecovery, String> {
        let path = crate::state_journal::state_journal_path(state_directory);
        let namespace = crate::state_journal::ARRIVAL_JOURNAL_NAMESPACE;
        let snapshot = SqliteJournal::read_only_namespace_after(&path, namespace, horizon)
            .map_err(|e| e.to_string())?;
        let source =
            SqliteJournal::open_read_only_namespace(&path, namespace).map_err(|e| e.to_string())?;
        let facts = circular_store::restart_facts(&snapshot, &source, horizon > 0)?;
        let checkpoints = facts.checkpoints.map(checkpoint_consumption).transpose()?;
        Ok(RestartRecovery {
            body: circular_store::ProductRestartBody {
                previous_sequence: facts.previous_sequence,
                boot_id: self.boot_id,
                wall_millis: self.wall_millis,
                reason: facts.reason,
                unsettled: facts.unsettled,
            },
            checkpoints,
        })
    }
}

/// Recorded pre-boot facts. None means normal/signal recovery uses each actor's
/// recorded stop boundary; Some contains the crash checkpoint consumption cut.
pub struct RestartRecovery {
    pub body: circular_store::ProductRestartBody,
    pub checkpoints: Option<
        std::collections::BTreeMap<
            circular_plan::NamedActorId,
            Option<(
                circular_core::ArrivalIndex,
                circular_core::Stamp<circular_plan::ActorId>,
            )>,
        >,
    >,
}

/// Checkpoint-only adapter over the existing store-owned body Stamp codec.
/// Approval terms are outside this reader's domain, so no term codec is invented.
struct CheckpointStampCodec;
impl crate::restart_custody_codec::RestoreNestedCodec for CheckpointStampCodec {
    type ApprovalTerm = std::convert::Infallible;
    type Stamp = circular_core::Stamp<circular_plan::ActorId>;
    fn encode_approval_term(&self, term: &Self::ApprovalTerm) -> Result<Vec<u8>, String> {
        match *term {}
    }
    fn decode_approval_term(&self, _: &[u8]) -> Result<Self::ApprovalTerm, String> {
        Err("checkpoint reader does not decode approval terms".into())
    }
    fn encode_stamp(&self, stamp: &Self::Stamp) -> Result<Vec<u8>, String> {
        let mut bytes = Vec::new();
        circular_store::push_stamp(&mut bytes, stamp).map_err(|e| format!("{e:?}"))?;
        Ok(bytes)
    }
    fn decode_stamp(&self, bytes: &[u8]) -> Result<Self::Stamp, String> {
        let mut cursor = 0;
        let stamp =
            circular_store::take_stamp(bytes, &mut cursor).ok_or("invalid checkpoint Stamp")?;
        if cursor != bytes.len() || self.encode_stamp(&stamp)? != bytes {
            return Err("noncanonical checkpoint Stamp".into());
        }
        Ok(stamp)
    }
}

fn checkpoint_consumption(
    rows: impl IntoIterator<
        Item = circular_store::TransactionCheckpoint<circular_store::ProductTransaction>,
    >,
) -> Result<
    std::collections::BTreeMap<
        circular_plan::NamedActorId,
        Option<(
            circular_core::ArrivalIndex,
            circular_core::Stamp<circular_plan::ActorId>,
        )>,
    >,
    String,
> {
    rows.into_iter()
        .map(|checkpoint| {
            let actor = checkpoint.actor().clone();
            let circular_plan::ActorId::Scoped {
                scope,
                local: circular_plan::LocalKey::Named(local),
            } = actor
            else {
                return Err("restart checkpoint requires a named actor".into());
            };
            let actor = circular_plan::NamedActorId::new(scope, local);
            let body = crate::restart_custody_codec::decode_checkpoint_body(
                &CheckpointStampCodec,
                checkpoint.state(),
            )?;
            if body
                .covered_arrival
                .as_ref()
                .is_some_and(|(_, stamp)| stamp.producer() != &actor.as_actor_id())
            {
                return Err("restart checkpoint consumption belongs to another actor".into());
            }
            Ok((actor, body.covered_arrival))
        })
        .collect()
}

