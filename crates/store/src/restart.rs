use circular_core::{Boundary, Ceilings, EncodedPayload, PayloadVersionTag, Value};
fn validate_unsettled(keys: &[Vec<u8>]) -> Result<(), String> {
    let mut previous = None;
    for bytes in keys {
        let key = circular_runtime::EffectId::decode(bytes)
            .map_err(|e| format!("restart EffectId: {e:?}"))?;
        if previous.as_ref().is_some_and(|previous| previous >= &key) {
            return Err("restart EffectIds are not strictly ordered".into());
        }
        previous = Some(key);
    }
    Ok(())
}

fn read_unsettled(
    snapshot: &crate::SqliteJournalSnapshot,
    source: &crate::SqliteJournal,
    after_horizon: bool,
) -> Result<Vec<Vec<u8>>, crate::SqliteJournalError> {
    let custody =
        crate::ProductCustodySnapshot::read_from_source(snapshot, after_horizon, Some(source))?;
    let invalid = |detail: String| crate::SqliteJournalError::Integrity { detail };
    let mut keys = std::collections::BTreeSet::new();
    for (key, _) in custody.outboxes() {
        keys.insert(key.clone());
    }
    keys.iter()
        .map(|key| {
            circular_runtime::EffectId::encode(key)
                .map_err(|e| invalid(format!("restart EffectId: {e:?}")))
        })
        .collect()
}

circular_core::closed_table! {
    pub enum RestartReason {
        Normal => "normal",
        Signal => "signal",
        Crash => "crash",
    }
}

impl RestartReason {
    pub fn parse(value: &str) -> Result<Self, String> {
        Self::from_str(value).ok_or_else(|| "unknown restart reason".into())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProductRestartBody {
    pub previous_sequence: u64,
    pub boot_id: [u8; 16],
    pub wall_millis: u64,
    pub reason: RestartReason,
    pub unsettled: Vec<Vec<u8>>,
}
impl ProductRestartBody {
    pub fn encode(&self) -> Result<EncodedPayload, String> {
        validate_unsettled(&self.unsettled)?;
        let keys = self.unsettled.iter().cloned().map(Value::bytes);
        let value = Value::array([
            Value::UInt(self.previous_sequence),
            Value::bytes(self.boot_id),
            Value::UInt(self.wall_millis),
            Value::string(self.reason.as_str()),
            Value::array(keys),
        ]);
        Ok(EncodedPayload::new(
            PayloadVersionTag::FIRST,
            &circular_core::encode(&value, Ceilings::for_boundary(Boundary::Journal))
                .map_err(|error| error.to_string())?,
        ))
    }
    pub fn decode(payload: &EncodedPayload) -> Result<Self, String> {
        if payload.version_tag() != PayloadVersionTag::FIRST {
            return Err("unsupported restart body version".into());
        }
        let value =
            circular_core::decode(payload.body(), Ceilings::for_boundary(Boundary::Journal))
                .map_err(|error| error.to_string())?;
        let Some(
            [
                Value::UInt(previous_sequence),
                Value::Bytes(boot),
                Value::UInt(wall_millis),
                Value::String(reason),
                Value::Array(keys),
            ],
        ) = value.as_array()
        else {
            return Err(
                "restart requires [previous UInt, boot Bytes16, wall UInt, reason String, effects Array]"
                    .into(),
            );
        };
        let body = Self {
            previous_sequence: *previous_sequence,
            boot_id: boot
                .as_slice()
                .try_into()
                .map_err(|_| "restart boot id must be 16 bytes")?,
            wall_millis: *wall_millis,
            reason: RestartReason::parse(reason)?,
            unsettled: keys
                .iter()
                .map(|value| {
                    let Value::Bytes(bytes) = value else {
                        return Err("restart effect key must be Bytes".into());
                    };
                    Ok(bytes.clone())
                })
                .collect::<Result<Vec<_>, String>>()?,
        };
        validate_unsettled(&body.unsettled)?;
        Ok(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn body() -> ProductRestartBody {
        ProductRestartBody {
            previous_sequence: 7,
            boot_id: [0x42; 16],
            wall_millis: 9,
            reason: RestartReason::Crash,
            unsettled: vec![],
        }
    }
    #[test]
    fn restart_body_rejects_wrong_shape_reason_and_key() {
        let fields = vec![
            Value::UInt(7),
            Value::bytes([0x42; 16]),
            Value::UInt(9),
            Value::string("crash"),
            Value::array([]),
        ];
        let reject = |fields: Vec<Value>| {
            let bytes = circular_core::encode(
                &Value::array(fields),
                Ceilings::for_boundary(Boundary::Journal),
            )
            .unwrap();
            assert!(
                ProductRestartBody::decode(&EncodedPayload::new(PayloadVersionTag::FIRST, &bytes))
                    .is_err()
            );
        };
        reject(fields[..4].to_vec());
        let mut extra = fields.clone();
        extra.push(Value::array([]));
        reject(extra);
        for (index, wrong) in [
            (0, Value::Int(7)),
            (1, Value::bytes([0; 15])),
            (2, Value::Int(9)),
            (3, Value::string("unknown")),
            (4, Value::array([Value::bytes([0])])),
        ] {
            let mut bad = fields.clone();
            bad[index] = wrong;
            reject(bad);
        }
        let encoded = body().encode().unwrap();
        assert!(
            ProductRestartBody::decode(&EncodedPayload::new(
                PayloadVersionTag::new(2).unwrap(),
                encoded.body()
            ))
            .is_err()
        );
    }
}

/// Read-only assembly material from the immutable pre-boot prefix.
pub struct RestartFacts {
    /// Last physical commit in the prefix, including other namespaces. This is
    /// ProductRestartBody.previous_sequence, never a producer's sequence.
    pub previous_sequence: u64,
    /// Final shutdown evidence, or Crash when that evidence is absent.
    pub reason: RestartReason,
    /// Canonical, ordered unsettled custody keys; reading never settles them.
    pub unsettled: Vec<Vec<u8>>,
    /// On crash, the last recorded checkpoint per actor at previous_sequence,
    /// including checkpoints whose custody was subsequently settled. Assembly
    /// decodes their existing bodies into the actor consumption boundaries.
    pub checkpoints: Option<Vec<crate::TransactionCheckpoint<crate::ProductTransaction>>>,
}

/// `after_horizon` identifies a prefix read after a settled checkpoint horizon.
/// `source` is the same namespace; a segment decoded mid-journal restores its
/// context from its own last keyframe there, before the horizon if need be.
/// Reads facts only: no append, stamp allocation or custody recovery occurs here.
pub fn restart_facts(
    snapshot: &crate::SqliteJournalSnapshot,
    source: &crate::SqliteJournal,
    after_horizon: bool,
) -> Result<RestartFacts, String> {
    let unsettled = read_unsettled(snapshot, source, after_horizon).map_err(|e| e.to_string())?;
    validate_unsettled(&unsettled)?;
    let reason = crate::shutdown::last_shutdown(snapshot, source)?
        .map_or(RestartReason::Crash, |exit| exit.reason);
    let checkpoints = if reason == RestartReason::Crash {
        let mut latest = std::collections::BTreeMap::<
            _,
            crate::TransactionCheckpoint<crate::ProductTransaction>,
        >::new();
        let mut reader = crate::ProductJournalReader::default();
        for entry in snapshot.entries() {
            let transaction = reader
                .read_at(source, entry.sequence().get(), entry.payload())
                .map_err(|e| format!("restart checkpoint transaction: {e}"))?;
            for operation in transaction.operations() {
                if let crate::StoreTransactionOp::ReplaceCheckpoint(checkpoint) = operation {
                    if let Some(prior) = latest.get(checkpoint.actor()) {
                        if checkpoint.at() < prior.at()
                            || (checkpoint.at() == prior.at() && checkpoint != prior)
                        {
                            return Err("restart checkpoint history is not monotone".into());
                        }
                    }
                    latest.insert(checkpoint.actor().clone(), checkpoint.clone());
                }
            }
        }
        Some(latest.into_values().collect())
    } else {
        None
    };
    Ok(RestartFacts {
        previous_sequence: snapshot.through().map_or(0, |through| through.get()),
        reason,
        unsettled,
        checkpoints,
    })
}

/// Construct the ordinary restart fact using the System actor's finished stamp.
pub fn restart_record(
    at: circular_core::Stamp<circular_runtime::ActorId>,
    body: &ProductRestartBody,
) -> Result<crate::Record<crate::ProductStore>, String> {
    if at.producer() != &circular_runtime::ActorId::System(circular_runtime::SystemActor::Pipeline)
    {
        return Err("restart is a System(Pipeline) record".into());
    }
    let identity = crate::OpaqueId::new(at.sequence().get());
    Ok(crate::Record::Observation(
        crate::ObservationRecord::restart(
            at,
            crate::ObservationBucket::from_millis(body.wall_millis),
            crate::RecordOrigin::Stream,
            crate::ObservationItemKey::new(
                circular_core::BuiltinObservationName::Restart,
                identity,
            ),
            body.encode()?,
        ),
    ))
}

#[cfg(test)]
mod atomic_tests {
    #[test]
    fn derived_append_rejection_keeps_the_committed_prefix() {
        let directory = circular_testkit::temp::StateDir::new("circular-restart-atomic");
        let path = directory.path().join("journal.sqlite3");
        let mut journal = crate::SqliteJournal::open_namespace(&path, "fixture").unwrap();
        let first = journal.commit(b"before").unwrap();
        let result = journal.commit_from_snapshot(|snapshot| {
            assert_eq!(snapshot.through(), Some(first.sequence()));
            Err(crate::SqliteJournalError::Integrity {
                detail: "fixture refuses body".into(),
            })
        });
        assert!(result.is_err());
        drop(journal);
        let snapshot = crate::SqliteJournal::read_only_namespace(&path, "fixture").unwrap();
        assert_eq!(snapshot.through(), Some(first.sequence()));
        assert_eq!(snapshot.entries().len(), 1);
        assert_eq!(snapshot.entries()[0].payload(), b"before");
    }
}
