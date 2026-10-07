//! An actor's disposable replay cache, outside the journal. The actor owns
//! the writer; assembly reads and checks it before handing a snapshot to that task.
//! No cache bytes are projected or authoritative. An unreadable/mismatched cache is
//! a miss, followed by full replay.

use circular_core::{
    ArrivalIndex, Boundary, Ceilings, Hlc, LogicalCounter, RevisionEpochId, Sequence, Stamp, Tick,
    Value,
};
use circular_plan::{ActorId, EdgeId, NamedActorId};
use circular_runtime::ActorState;
use circular_store::StreamId;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Target replay tail: at most 1,023 arrivals between eligible successful writes;
/// busy/non-encodable actors or slow disk can require longer/full replay.
/// Internal cadence, not configuration; a clean stop also writes its last turn.
pub(crate) const EVERY: u64 = 1_024;

pub(crate) struct Snapshot {
    pub(crate) stream: StreamId,
    /// Exclusive end of the turns folded into this snapshot, never a consumption fact.
    pub(crate) index: ArrivalIndex,
    pub(crate) previous: Stamp<ActorId>,
    pub(crate) revision: RevisionEpochId,
    pub(crate) generation: u64,
    pub(crate) emitted: Option<Sequence>,
    pub(crate) emission_clock: Option<Hlc>,
    pub(crate) state: Option<ActorState<u16>>,
    pub(crate) sent: Vec<(EdgeId, Sequence)>,
}

impl Snapshot {
    fn encode(&self) -> Option<Vec<u8>> {
        let optional = |value: Option<u64>| value.map_or(Value::Null, Value::UInt);
        let (schema, bytes) = self
            .state
            .as_ref()
            .map_or((Value::Null, Value::Null), |state| {
                (
                    Value::UInt(u64::from(*state.schema())),
                    Value::bytes(state.bytes().to_vec()),
                )
            });
        let mut previous = Vec::new();
        circular_store::push_stamp(&mut previous, &self.previous).ok()?;
        let sent = self
            .sent
            .iter()
            .map(|(edge, sequence)| {
                Some(Value::array([
                    circular_store::edge_value(edge).ok()?,
                    Value::UInt(sequence.get()),
                ]))
            })
            .collect::<Option<Vec<_>>>()?;
        let body = Value::array([
            Value::UInt(self.stream.get()),
            Value::UInt(self.index.get()),
            Value::bytes(previous),
            Value::UInt(self.revision.get()),
            Value::UInt(self.generation),
            optional(self.emitted.map(Sequence::get)),
            optional(self.emission_clock.map(|clock| clock.l().get())),
            optional(self.emission_clock.map(|clock| clock.c().get())),
            schema,
            bytes,
            Value::Array(sent),
        ]);
        let body = circular_core::encode(&body, Ceilings::for_boundary(Boundary::Journal)).ok()?;
        let mut file = Sha256::digest(&body).to_vec();
        file.extend(body);
        Some(file)
    }

    fn decode(file: &[u8]) -> Option<Self> {
        let (digest, body) = file.split_at_checked(32)?;
        if Sha256::digest(body).as_slice() != digest {
            return None;
        }
        let Value::Array(fields) =
            circular_core::decode(body, Ceilings::for_boundary(Boundary::Journal)).ok()?
        else {
            return None;
        };
        let uint = |value: &Value| match value {
            Value::UInt(value) => Some(*value),
            _ => None,
        };
        let optional = |value: &Value| match value {
            Value::Null => Some(None),
            Value::UInt(value) => Some(Some(*value)),
            _ => None,
        };
        let [
            stream,
            index,
            previous,
            revision,
            generation,
            emitted,
            l,
            c,
            schema,
            bytes,
            sent,
        ] = fields.as_slice()
        else {
            return None;
        };
        let Value::Array(sent) = sent else {
            return None;
        };
        let sent = sent
            .iter()
            .map(|pair| {
                let Value::Array(pair) = pair else {
                    return None;
                };
                let [edge, sequence] = pair.as_slice() else {
                    return None;
                };
                Some((
                    circular_runtime::product_identity::edge_from_value(edge).ok()?,
                    Sequence::new(uint(sequence)?).ok()?,
                ))
            })
            .collect::<Option<Vec<_>>>()?;
        let Value::Bytes(previous) = previous else {
            return None;
        };
        let mut cursor = 0;
        let stamp = circular_store::take_stamp(previous, &mut cursor)?;
        if cursor != previous.len() {
            return None;
        }
        let emission_clock = match (optional(l)?, optional(c)?) {
            (Some(l), Some(c)) => Some(Hlc::new(Tick::new(l), LogicalCounter::new(c))),
            (None, None) => None,
            _ => return None,
        };
        let emitted = optional(emitted)?.map(Sequence::new).transpose().ok()?;
        if emitted.is_some() != emission_clock.is_some() {
            return None;
        }
        let state = match (schema, bytes) {
            (Value::Null, Value::Null) => None,
            (Value::UInt(schema), Value::Bytes(bytes)) => {
                Some(ActorState::new(u16::try_from(*schema).ok()?, bytes.clone()))
            }
            _ => return None,
        };
        Some(Self {
            stream: StreamId::new(uint(stream)?),
            index: ArrivalIndex::new(uint(index)?),
            previous: stamp,
            revision: RevisionEpochId::new(uint(revision)?)?,
            generation: uint(generation)?,
            emitted,
            emission_clock,
            state,
            sent,
        })
    }
}

/// One actor's file, handed out at spawn; no directory scan or shared cache table.
pub(crate) fn path(root: &Path, stream: StreamId, actor: &NamedActorId) -> PathBuf {
    let identity =
        circular_store::record_actor_value(&actor.as_actor_id()).expect("named actor identity");
    let bytes = circular_core::encode(&identity, Ceilings::for_boundary(Boundary::Journal))
        .expect("actor identity encodes");
    root.join("cache")
        .join("actors")
        .join(stream.get().to_string())
        .join(format!("{:x}.ckpt", Sha256::digest(bytes)))
}

pub(crate) fn miss(actor: &NamedActorId, reason: &str) {
    eprintln!("circular-kernel: actor={actor:?} checkpoint_cache_miss reason={reason}");
}

/// Called only by restoration assembly. Absence and damage are cache misses.
pub(crate) async fn read(path: PathBuf, actor: &NamedActorId) -> Option<Snapshot> {
    let result = tokio::task::spawn_blocking(move || {
        let bytes = std::fs::read(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "absent"
            } else {
                "read_failed"
            }
        })?;
        Snapshot::decode(&bytes).ok_or("invalid_bytes")
    })
    .await;
    match result {
        Ok(Ok(snapshot)) => Some(snapshot),
        Ok(Err(reason)) => {
            miss(actor, reason);
            None
        }
        Err(_) => {
            miss(actor, "reader_failed");
            None
        }
    }
}

#[derive(Default)]
struct Candidates {
    latest: Option<Snapshot>,
    running: bool,
}

pub(crate) struct Writer {
    path: PathBuf,
    candidates: std::sync::Arc<std::sync::Mutex<Candidates>>,
    pending: Option<tokio::task::JoinHandle<()>>,
}

impl Writer {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            candidates: Default::default(),
            pending: None,
        }
    }

    /// A live turn replaces only the waiting candidate. The single worker drains
    /// the latest candidate after its current write; no task/copy chain accumulates.
    pub(crate) async fn write(&mut self, snapshot: Snapshot, clean: bool) {
        let (start, skipped) = {
            let mut candidates = self.candidates.lock().expect("candidate transfer");
            let skipped = candidates.latest.replace(snapshot);
            let start = !candidates.running;
            candidates.running = true;
            (start, skipped)
        };
        if let Some(skipped) = skipped {
            eprintln!(
                "circular-kernel: actor={:?} checkpoint_cache_write reason=superseded index={}",
                skipped.previous.producer(),
                skipped.index.get()
            );
        }
        if start {
            let candidates = self.candidates.clone();
            let path = self.path.clone();
            self.pending = Some(tokio::task::spawn_blocking(move || {
                loop {
                    let snapshot = {
                        let mut candidates = candidates.lock().expect("candidate transfer");
                        match candidates.latest.take() {
                            Some(snapshot) => snapshot,
                            None => {
                                candidates.running = false;
                                return;
                            }
                        }
                    };
                    write_file(&path, snapshot);
                }
            }));
        }
        if clean {
            self.finish().await;
        }
    }

    pub(crate) async fn finish(&mut self) {
        if let Some(write) = self.pending.take()
            && write.await.is_err()
        {
            eprintln!("circular-kernel: checkpoint_cache_write reason=writer_failed");
        }
    }
}

fn write_file(path: &Path, snapshot: Snapshot) {
    let Some(bytes) = snapshot.encode() else {
        eprintln!("circular-kernel: checkpoint_cache_write reason=encode_failed");
        return;
    };
    let Ok(nonce) = getrandom::u64() else {
        eprintln!("circular-kernel: checkpoint_cache_write reason=temporary_unavailable");
        return;
    };
    let temporary = path.with_extension(format!("{nonce:016x}.tmp"));
    let result = (|| {
        std::fs::create_dir_all(path.parent().expect("cache parent"))?;
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(&temporary, path)
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        eprintln!(
            "circular-kernel: checkpoint_cache_write reason=io_failed kind={:?}",
            error.kind()
        );
    }
}
