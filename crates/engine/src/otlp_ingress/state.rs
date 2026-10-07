
use std::collections::{BTreeSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value};

const FILE_NAME: &str = "otlp-custody.jsonl";
/// The retired whole-file custody. Its presence refuses the custody.
const RETIRED_FILE_NAME: &str = "otlp-edge-state.json";
const VERSION: u64 = 1;
pub const MAX_QUEUED_SPLITS: usize = 64;
const MAX_TOMBSTONES: usize = 64;
static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

circular_core::closed_table! {
    pub enum OtlpSignal {
        Logs => "logs",
        Metrics => "metrics",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DropReason {
    InvalidRequest,
    Timeout,
    UnsupportedContentType,
    UnsupportedContentEncoding,
    InvalidJson,
    UnsupportedSignalShape,
    UnsplittableItem,
    ScrubFailure,
    QueueCapacity,
}

use circular_actors::otlp as refusal;

impl DropReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => refusal::INVALID_REQUEST,
            Self::Timeout => refusal::TIMEOUT,
            Self::UnsupportedContentType => refusal::UNSUPPORTED_CONTENT_TYPE,
            Self::UnsupportedContentEncoding => refusal::UNSUPPORTED_CONTENT_ENCODING,
            Self::InvalidJson => refusal::INVALID_JSON,
            Self::UnsupportedSignalShape => refusal::UNSUPPORTED_SIGNAL_SHAPE,
            Self::UnsplittableItem => refusal::UNSPLITTABLE_ITEM,
            Self::ScrubFailure => "scrub_failure",
            Self::QueueCapacity => "queue_capacity",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            refusal::INVALID_REQUEST => Some(Self::InvalidRequest),
            refusal::TIMEOUT => Some(Self::Timeout),
            refusal::UNSUPPORTED_CONTENT_TYPE => Some(Self::UnsupportedContentType),
            refusal::UNSUPPORTED_CONTENT_ENCODING => Some(Self::UnsupportedContentEncoding),
            refusal::INVALID_JSON => Some(Self::InvalidJson),
            refusal::UNSUPPORTED_SIGNAL_SHAPE => Some(Self::UnsupportedSignalShape),
            refusal::UNSPLITTABLE_ITEM => Some(Self::UnsplittableItem),
            "scrub_failure" => Some(Self::ScrubFailure),
            "queue_capacity" => Some(Self::QueueCapacity),
            _ => None,
        }
    }
}

/// One split in custody. `body` is the scrubbed split as the JSON bytes the
/// custody log carries — the forwarder hands these bytes on without re-encoding.
#[derive(Clone, Debug, PartialEq)]
pub struct QueuedSplit {
    pub id: String,
    pub signal: OtlpSignal,
    pub split_index: u64,
    pub split_count: u64,
    pub item_start: u64,
    pub item_end: u64,
    pub original_bytes: u64,
    pub scrubbed_batch_bytes: u64,
    pub body: Arc<[u8]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DropTombstone {
    id: String,
    signal: Option<OtlpSignal>,
    reason: DropReason,
    dropped_items: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OtlpEdgeCounters {
    pub bind_failures: u64,
    pub rejected_requests: u64,
    pub dropped_batches: u64,
    pub dropped_items: u64,
    pub accepted_batches: u64,
    pub queued_splits: u64,
    pub forwarded_splits: u64,
    pub scrubbed_fields: u64,
}

/// The fold of the custody log — the only state this store has.
#[derive(Clone, Debug, Default)]
struct Fold {
    next_sequence: u64,
    queue: VecDeque<QueuedSplit>,
    tombstones: VecDeque<DropTombstone>,
    counters: OtlpEdgeCounters,
}

pub struct OtlpStateStore {
    directory: PathBuf,
    file: PathBuf,
    fold: Fold,
    /// The open log, positioned at its end. `None` until the file exists.
    log: Option<File>,
    /// The log's length — every record in it is complete. A failed append is
    /// cut back to this length, so the log never keeps half a record.
    log_len: u64,
    /// Records appended since the file's `snapshot` that describe no custody:
    /// settled splits and counter-only records. Compaction resets it.
    settled_since_snapshot: usize,
    /// A failed append this store could not cut back. The log's end is then
    /// unknown, so the store refuses every further write.
    broken: Option<String>,
}

/// One split of an admitted batch. `body` is the scrubbed split's JSON bytes.
pub struct PendingSplit {
    pub item_start: u64,
    pub item_end: u64,
    pub body: Vec<u8>,
}

impl OtlpStateStore {
    pub fn create_empty(directory: &Path) -> Result<Self, String> {
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(directory).map_err(|error| {
            format!(
                "cannot create fresh OTLP custody {}: {error}",
                directory.display()
            )
        })?;
        let mut store = Self::open(directory)?;
        if let Some(warning) = store.flush()? {
            eprintln!("circular-daemon: otlp: {warning}");
        }
        Ok(store)
    }

    /// Reopen custody belonging to this actor. Never pass a legacy catch path.
    ///
    /// The state is the fold of the log. A log that carries more than its
    /// `snapshot` is compacted to one `snapshot` line before the store is
    /// returned, so a reopened custody starts from its fold alone.
    pub fn open(directory: &Path) -> Result<Self, String> {
        if !directory.is_dir() {
            return Err(format!(
                "OTLP custody path is not a directory: {}",
                directory.display()
            ));
        }
        let retired = directory.join(RETIRED_FILE_NAME);
        if retired.exists() {
            return Err(format!(
                "OTLP custody {} holds the retired whole-file state {RETIRED_FILE_NAME}; \
                 custody is now the append-only log {FILE_NAME} and the old file is not read",
                directory.display()
            ));
        }
        let file = directory.join(FILE_NAME);
        let bytes = match std::fs::read(&file) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => {
                return Err(format!(
                    "cannot read OTLP custody log {}: {error}",
                    file.display()
                ));
            }
        };
        let mut store = Self {
            directory: directory.to_path_buf(),
            file,
            fold: Fold::default(),
            log: None,
            log_len: 0,
            settled_since_snapshot: 0,
            broken: None,
        };
        if let Some(bytes) = bytes {
            let (fold, records, torn) = replay(&bytes)?;
            store.fold = fold;
            store.log_len = u64::try_from(bytes.len() - torn)
                .map_err(|_| "OTLP custody log length does not fit u64".to_owned())?;
            if torn > 0 {
                eprintln!(
                    "circular-daemon: otlp: custody log {} ended in {torn} byte(s) of a record whose write \
                     never completed; nothing was acknowledged on it and it is dropped",
                    store.file.display()
                );
            }
            if records > 1 || torn > 0 {
                if let Some(warning) = store.compact()? {
                    eprintln!("circular-daemon: otlp: {warning}");
                }
            } else {
                store.log = Some(open_log(&store.file)?);
            }
        }
        Ok(store)
    }

    pub const fn counters(&self) -> OtlpEdgeCounters {
        self.fold.counters
    }

    pub fn queue_len(&self) -> usize {
        self.fold.queue.len()
    }

    #[cfg(test)]
    pub fn latest_drop_reason(&self) -> Option<DropReason> {
        self.fold
            .tombstones
            .back()
            .map(|tombstone| tombstone.reason)
    }

    pub fn front(&self) -> Option<QueuedSplit> {
        self.fold.queue.front().cloned()
    }

    pub fn queued_items(&self) -> impl Iterator<Item = &QueuedSplit> {
        self.fold.queue.iter()
    }

    pub fn can_enqueue(&self, split_count: usize) -> bool {
        split_count <= MAX_QUEUED_SPLITS.saturating_sub(self.fold.queue.len())
    }

    /// Admit one complete batch: one `accepted` record, synced before return —
    /// the HTTP 200 that follows promises exactly this record.
    pub fn enqueue(
        &mut self,
        signal: OtlpSignal,
        splits: Vec<PendingSplit>,
        original_bytes: u64,
        scrubbed_batch_bytes: u64,
        scrubbed_fields: u64,
    ) -> Result<Option<String>, String> {
        if !splits.is_empty() && !self.can_enqueue(splits.len()) {
            return Err("OTLP durable queue has no capacity for the complete batch".to_owned());
        }
        let record = if splits.is_empty() {
            AcceptedRecord {
                signal,
                sequence: None,
                original_bytes,
                scrubbed_batch_bytes,
                scrubbed_fields,
                splits: Vec::new(),
            }
        } else {
            let sequence = self.fold.next_sequence;
            let split_count = u64::try_from(splits.len())
                .map_err(|_| "OTLP split count does not fit u64".to_owned())?;
            let mut queued = Vec::with_capacity(splits.len());
            for (index, split) in splits.into_iter().enumerate() {
                let split_index = u64::try_from(index)
                    .map_err(|_| "OTLP split index does not fit u64".to_owned())?;
                queued.push(QueuedSplit {
                    id: format!(
                        "otlp-{}-{sequence:020}-{:04}-of-{split_count:04}",
                        signal.as_str(),
                        split_index + 1
                    ),
                    signal,
                    split_index,
                    split_count,
                    item_start: split.item_start,
                    item_end: split.item_end,
                    original_bytes,
                    scrubbed_batch_bytes,
                    body: Arc::from(split.body),
                });
            }
            AcceptedRecord {
                signal,
                sequence: Some(sequence),
                original_bytes,
                scrubbed_batch_bytes,
                scrubbed_fields,
                splits: queued,
            }
        };
        let empty = record.splits.is_empty();
        let mut next = self.fold.clone_without_queue();
        next.apply_accepted_counters(&record)?;
        let line = encode_accepted(&record);
        let warning = self.append(&line, Durability::Synced)?;
        self.fold.next_sequence = next.next_sequence;
        self.fold.counters = next.counters;
        self.fold.queue.extend(record.splits);
        if empty {
            return Ok(warning.or(self.settle_one()?));
        }
        Ok(warning)
    }

    /// The queue head was accepted by its owning Source. One `forwarded`
    /// record, not synced on its own (see the module header).
    pub fn acknowledge_forward(&mut self, id: &str) -> Result<Option<String>, String> {
        let Some(front) = self.fold.queue.front() else {
            return Err("OTLP forward acknowledgement has no queued item".to_owned());
        };
        if front.id != id {
            return Err(format!(
                "OTLP forward acknowledgement id {id:?} does not match queue head"
            ));
        }
        let forwarded_splits =
            checked_add(self.fold.counters.forwarded_splits, 1, "forwarded_splits")?;
        let line = encode_record("forwarded", &serde_json::json!({ "id": id }));
        let warning = self.append(&line, Durability::Unsynced)?;
        self.fold.queue.pop_front();
        self.fold.counters.forwarded_splits = forwarded_splits;
        Ok(warning.or(self.settle_one()?))
    }

    pub fn record_bind_failure(&mut self) -> Result<Option<String>, String> {
        let bind_failures = checked_add(self.fold.counters.bind_failures, 1, "bind_failures")?;
        let line = encode_record("bind_failure", &Value::Object(Map::new()));
        let warning = self.append(&line, Durability::Unsynced)?;
        self.fold.counters.bind_failures = bind_failures;
        Ok(warning.or(self.settle_one()?))
    }

    pub fn record_rejection(
        &mut self,
        signal: Option<OtlpSignal>,
        reason: DropReason,
        dropped_items: u64,
        terminal_drop: bool,
    ) -> Result<Option<String>, String> {
        let record = RejectedRecord {
            sequence: self.fold.next_sequence,
            signal,
            reason,
            dropped_items,
            terminal: terminal_drop,
        };
        let mut next = self.fold.clone_without_queue();
        next.apply_rejected(&record)?;
        let line = encode_rejected(&record);
        let warning = self.append(&line, Durability::Unsynced)?;
        self.fold.next_sequence = next.next_sequence;
        self.fold.tombstones = next.tombstones;
        self.fold.counters = next.counters;
        Ok(warning.or(self.settle_one()?))
    }

    /// Make everything appended so far durable. Creates the log (one `snapshot`
    /// line) if it does not exist yet.
    pub fn flush(&mut self) -> Result<Option<String>, String> {
        match self.log.as_mut() {
            None => self.compact(),
            Some(log) => {
                log.sync_data()
                    .map_err(|error| format!("cannot sync OTLP custody log: {error}"))?;
                Ok(None)
            }
        }
    }

    /// Append one record. Creates the log first if it does not exist yet.
    ///
    /// A write or sync that fails is cut back to the length before this record,
    /// so the fold and the log stay the same thing; the caller answers the
    /// failure (no HTTP 200 follows it). If even the cut fails, the store
    /// refuses every later write.
    fn append(&mut self, line: &[u8], durability: Durability) -> Result<Option<String>, String> {
        if let Some(broken) = &self.broken {
            return Err(broken.clone());
        }
        let warning = if self.log.is_none() {
            self.compact()?
        } else {
            None
        };
        let log = self
            .log
            .as_mut()
            .ok_or_else(|| "OTLP custody log is not open".to_owned())?;
        let written = log
            .write_all(line)
            .map_err(|error| format!("cannot append to OTLP custody log: {error}"))
            .and_then(|()| match durability {
                Durability::Synced => log
                    .sync_data()
                    .map_err(|error| format!("cannot sync OTLP custody log: {error}")),
                Durability::Unsynced => Ok(()),
            });
        if let Err(error) = written {
            if let Err(cut) = log.set_len(self.log_len) {
                self.broken = Some(format!(
                    "{error}; the OTLP custody log could not be cut back to its last complete record: {cut}"
                ));
            }
            return Err(error);
        }
        self.log_len = self.log_len.saturating_add(line.len() as u64);
        Ok(warning)
    }

    /// Count one record that describes no custody; compact once they reach
    /// the queue's capacity.
    fn settle_one(&mut self) -> Result<Option<String>, String> {
        self.settled_since_snapshot = self.settled_since_snapshot.saturating_add(1);
        if self.settled_since_snapshot < MAX_QUEUED_SPLITS {
            return Ok(None);
        }
        self.compact()
    }

    /// Replace the log by one `snapshot` line: write a temporary file, sync it,
    /// rename it over the log, sync the directory. The fold does not change.
    fn compact(&mut self) -> Result<Option<String>, String> {
        if let Some(broken) = &self.broken {
            return Err(broken.clone());
        }
        let bytes = encode_snapshot(&self.fold);
        let temporary = self.directory.join(format!(
            ".{FILE_NAME}.{}-{}.tmp",
            std::process::id(),
            TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let publication = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut temporary_file = options.open(&temporary).map_err(|error| {
                format!(
                    "cannot create OTLP custody temporary {}: {error}",
                    temporary.display()
                )
            })?;
            temporary_file
                .write_all(&bytes)
                .map_err(|error| format!("cannot write OTLP custody temporary: {error}"))?;
            temporary_file
                .sync_data()
                .map_err(|error| format!("cannot sync OTLP custody temporary: {error}"))?;
            std::fs::rename(&temporary, &self.file)
                .map_err(|error| format!("cannot publish OTLP custody log: {error}"))?;
            Ok(())
        })();
        if let Err(error) = publication {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
        self.log = Some(open_log(&self.file)?);
        self.log_len = bytes.len() as u64;
        self.settled_since_snapshot = 0;
        let warning = File::open(&self.directory)
            .and_then(|directory| directory.sync_all())
            .err()
            .map(|error| {
                format!(
                    "OTLP custody log was compacted, but its directory could not be synced: {error}"
                )
            });
        Ok(warning)
    }
}

#[derive(Clone, Copy)]
enum Durability {
    Synced,
    Unsynced,
}

fn open_log(file: &Path) -> Result<File, String> {
    OpenOptions::new()
        .append(true)
        .open(file)
        .map_err(|error| format!("cannot open OTLP custody log {}: {error}", file.display()))
}

struct AcceptedRecord {
    signal: OtlpSignal,
    sequence: Option<u64>,
    original_bytes: u64,
    scrubbed_batch_bytes: u64,
    scrubbed_fields: u64,
    splits: Vec<QueuedSplit>,
}

struct RejectedRecord {
    sequence: u64,
    signal: Option<OtlpSignal>,
    reason: DropReason,
    dropped_items: u64,
    terminal: bool,
}

impl RejectedRecord {
    fn id(&self) -> String {
        format!("otlp-drop-{:020}", self.sequence)
    }
}

impl Fold {
    /// The fold's scalar part. Applying a record to it first lets a store
    /// check every counter before it writes, then adopt the result.
    fn clone_without_queue(&self) -> Self {
        Self {
            next_sequence: self.next_sequence,
            queue: VecDeque::new(),
            tombstones: self.tombstones.clone(),
            counters: self.counters,
        }
    }

    fn apply_accepted_counters(&mut self, record: &AcceptedRecord) -> Result<(), String> {
        let split_count = u64::try_from(record.splits.len())
            .map_err(|_| "OTLP split count does not fit u64".to_owned())?;
        match record.sequence {
            None if record.splits.is_empty() => {}
            Some(sequence) if !record.splits.is_empty() && sequence == self.next_sequence => {
                self.next_sequence = sequence
                    .checked_add(1)
                    .ok_or_else(|| "OTLP queue sequence overflow".to_owned())?;
            }
            _ => return Err("OTLP accepted record carries the wrong sequence".to_owned()),
        }
        self.counters.accepted_batches =
            checked_add(self.counters.accepted_batches, 1, "accepted_batches")?;
        self.counters.queued_splits =
            checked_add(self.counters.queued_splits, split_count, "queued_splits")?;
        self.counters.scrubbed_fields = checked_add(
            self.counters.scrubbed_fields,
            record.scrubbed_fields,
            "scrubbed_fields",
        )?;
        Ok(())
    }

    fn apply_rejected(&mut self, record: &RejectedRecord) -> Result<(), String> {
        if record.sequence != self.next_sequence {
            return Err("OTLP rejected record carries the wrong sequence".to_owned());
        }
        self.next_sequence = record
            .sequence
            .checked_add(1)
            .ok_or_else(|| "OTLP tombstone sequence overflow".to_owned())?;
        self.tombstones.push_back(DropTombstone {
            id: record.id(),
            signal: record.signal,
            reason: record.reason,
            dropped_items: record.dropped_items,
        });
        while self.tombstones.len() > MAX_TOMBSTONES {
            self.tombstones.pop_front();
        }
        self.counters.rejected_requests =
            checked_add(self.counters.rejected_requests, 1, "rejected_requests")?;
        self.counters.dropped_batches = checked_add(
            self.counters.dropped_batches,
            u64::from(record.terminal),
            "dropped_batches",
        )?;
        self.counters.dropped_items = checked_add(
            self.counters.dropped_items,
            if record.terminal {
                record.dropped_items
            } else {
                0
            },
            "dropped_items",
        )?;
        Ok(())
    }
}

/// Fold a whole log. Returns the fold, the number of complete records, and the
/// bytes of a final record whose write never completed.
fn replay(bytes: &[u8]) -> Result<(Fold, usize, usize), String> {
    let complete = bytes
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |position| position + 1);
    let torn = bytes.len() - complete;
    let mut lines = bytes[..complete]
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty());
    let first = lines
        .next()
        .ok_or_else(|| "OTLP custody log has no snapshot record".to_owned())?;
    let mut fold = match decode_line(first, 1)? {
        (kind, body) if kind == "snapshot" => decode_snapshot(body)?,
        _ => return Err("OTLP custody log does not begin with its snapshot record".to_owned()),
    };
    let mut records = 1_usize;
    for line in lines {
        records += 1;
        let (kind, body) = decode_line(line, records)?;
        let context = |error: String| format!("OTLP custody log record {records}: {error}");
        match kind.as_str() {
            "accepted" => {
                let record = decode_accepted(body).map_err(context)?;
                fold.apply_accepted_counters(&record).map_err(context)?;
                for split in record.splits {
                    if fold.queue.iter().any(|queued| queued.id == split.id) {
                        return Err(context("duplicate queue item id".to_owned()));
                    }
                    fold.queue.push_back(split);
                }
                if fold.queue.len() > MAX_QUEUED_SPLITS {
                    return Err(context("queue exceeds its capacity".to_owned()));
                }
            }
            "forwarded" => {
                let mut body = into_object(body, "forwarded").map_err(context)?;
                let id = take_string(&mut body, "id").map_err(context)?;
                reject_unknown(&body, "forwarded record").map_err(context)?;
                if fold.queue.front().is_none_or(|front| front.id != id) {
                    return Err(context(format!(
                        "forwarded id {id:?} is not the queue head"
                    )));
                }
                fold.queue.pop_front();
                fold.counters.forwarded_splits =
                    checked_add(fold.counters.forwarded_splits, 1, "forwarded_splits")
                        .map_err(context)?;
            }
            "rejected" => {
                let record = decode_rejected(body).map_err(context)?;
                fold.apply_rejected(&record).map_err(context)?;
            }
            "bind_failure" => {
                let body = into_object(body, "bind_failure").map_err(context)?;
                reject_unknown(&body, "bind_failure record").map_err(context)?;
                fold.counters.bind_failures =
                    checked_add(fold.counters.bind_failures, 1, "bind_failures")
                        .map_err(context)?;
            }
            "snapshot" => {
                return Err(context(
                    "a snapshot record appears after the first line".to_owned(),
                ));
            }
            other => return Err(context(format!("unknown record kind {other:?}"))),
        }
    }
    Ok((fold, records, torn))
}

/// One line is one JSON object with exactly one key — the record kind.
fn decode_line(line: &[u8], number: usize) -> Result<(String, Value), String> {
    let value: Value = serde_json::from_slice(line)
        .map_err(|error| format!("OTLP custody log record {number} is not valid JSON: {error}"))?;
    let Value::Object(fields) = value else {
        return Err(format!(
            "OTLP custody log record {number} must be a JSON object"
        ));
    };
    if fields.len() != 1 {
        return Err(format!(
            "OTLP custody log record {number} must name exactly one record kind"
        ));
    }
    Ok(fields.into_iter().next().expect("one field"))
}

fn decode_snapshot(body: Value) -> Result<Fold, String> {
    let mut fields = into_object(body, "snapshot")?;
    if take_u64(&mut fields, "version")? != VERSION {
        return Err("unsupported OTLP custody log version".to_owned());
    }
    let next_sequence = take_u64(&mut fields, "next_sequence")?;
    let queue = take_array(&mut fields, "queue")?
        .into_iter()
        .map(decode_queue_item)
        .collect::<Result<VecDeque<_>, _>>()?;
    if queue.len() > MAX_QUEUED_SPLITS {
        return Err("OTLP custody snapshot queue exceeds its capacity".to_owned());
    }
    let mut queue_ids = BTreeSet::new();
    if queue.iter().any(|item| !queue_ids.insert(&item.id)) {
        return Err("OTLP custody snapshot contains a duplicate queue item id".to_owned());
    }
    let tombstones = take_array(&mut fields, "tombstones")?
        .into_iter()
        .map(decode_tombstone)
        .collect::<Result<VecDeque<_>, _>>()?;
    if tombstones.len() > MAX_TOMBSTONES {
        return Err("OTLP custody snapshot has too many tombstones".to_owned());
    }
    let mut tombstone_ids = BTreeSet::new();
    if tombstones
        .iter()
        .any(|item| !tombstone_ids.insert(&item.id))
    {
        return Err("OTLP custody snapshot contains a duplicate tombstone id".to_owned());
    }
    let counters = decode_counters(
        fields
            .remove("counters")
            .ok_or_else(|| "OTLP custody snapshot has no counters".to_owned())?,
    )?;
    reject_unknown(&fields, "OTLP custody snapshot")?;
    Ok(Fold {
        next_sequence,
        queue,
        tombstones,
        counters,
    })
}

fn decode_queue_item(value: Value) -> Result<QueuedSplit, String> {
    let mut fields = into_object(value, "queue item")?;
    let signal = take_signal(&mut fields, "signal")?;
    let original_bytes = take_u64(&mut fields, "original_bytes")?;
    let scrubbed_batch_bytes = take_u64(&mut fields, "scrubbed_batch_bytes")?;
    decode_split(fields, signal, original_bytes, scrubbed_batch_bytes)
}

fn decode_split(
    mut fields: Map<String, Value>,
    signal: OtlpSignal,
    original_bytes: u64,
    scrubbed_batch_bytes: u64,
) -> Result<QueuedSplit, String> {
    let id = take_string(&mut fields, "id")?;
    let split_index = take_u64(&mut fields, "split_index")?;
    let split_count = take_u64(&mut fields, "split_count")?;
    let item_start = take_u64(&mut fields, "item_start")?;
    let item_end = take_u64(&mut fields, "item_end")?;
    let body = fields
        .remove("body")
        .ok_or_else(|| "OTLP custody split has no body".to_owned())?;
    reject_unknown(&fields, "OTLP custody split")?;
    if id.is_empty()
        || id.contains(['\r', '\n'])
        || split_count == 0
        || split_index >= split_count
        || item_end < item_start
    {
        return Err("OTLP custody split metadata is invalid".to_owned());
    }
    let body = serde_json::to_vec(&body)
        .map_err(|error| format!("cannot encode OTLP custody split body: {error}"))?;
    Ok(QueuedSplit {
        id,
        signal,
        split_index,
        split_count,
        item_start,
        item_end,
        original_bytes,
        scrubbed_batch_bytes,
        body: Arc::from(body),
    })
}

fn decode_accepted(body: Value) -> Result<AcceptedRecord, String> {
    let mut fields = into_object(body, "accepted")?;
    let signal = take_signal(&mut fields, "signal")?;
    let sequence = match fields.remove("sequence") {
        Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .ok_or_else(|| "OTLP accepted sequence must be u64 or null".to_owned())?,
        ),
        None => return Err("OTLP accepted record has no sequence".to_owned()),
    };
    let original_bytes = take_u64(&mut fields, "original_bytes")?;
    let scrubbed_batch_bytes = take_u64(&mut fields, "scrubbed_batch_bytes")?;
    let scrubbed_fields = take_u64(&mut fields, "scrubbed_fields")?;
    let splits = take_array(&mut fields, "splits")?
        .into_iter()
        .map(|split| {
            decode_split(
                into_object(split, "accepted split")?,
                signal,
                original_bytes,
                scrubbed_batch_bytes,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    reject_unknown(&fields, "accepted record")?;
    Ok(AcceptedRecord {
        signal,
        sequence,
        original_bytes,
        scrubbed_batch_bytes,
        scrubbed_fields,
        splits,
    })
}

fn decode_rejected(body: Value) -> Result<RejectedRecord, String> {
    let mut fields = into_object(body, "rejected")?;
    let tombstone = decode_tombstone_fields(&mut fields)?;
    let sequence = take_u64(&mut fields, "sequence")?;
    let terminal = match fields.remove("terminal") {
        Some(Value::Bool(value)) => value,
        _ => return Err("OTLP rejected record field \"terminal\" must be a boolean".to_owned()),
    };
    reject_unknown(&fields, "rejected record")?;
    let record = RejectedRecord {
        sequence,
        signal: tombstone.signal,
        reason: tombstone.reason,
        dropped_items: tombstone.dropped_items,
        terminal,
    };
    if record.id() != tombstone.id {
        return Err("OTLP rejected record id does not match its sequence".to_owned());
    }
    Ok(record)
}

fn decode_tombstone(value: Value) -> Result<DropTombstone, String> {
    let mut fields = into_object(value, "drop tombstone")?;
    let tombstone = decode_tombstone_fields(&mut fields)?;
    reject_unknown(&fields, "OTLP drop tombstone")?;
    Ok(tombstone)
}

fn decode_tombstone_fields(fields: &mut Map<String, Value>) -> Result<DropTombstone, String> {
    let id = take_string(fields, "id")?;
    let signal = match fields.remove("signal") {
        Some(Value::Null) => None,
        Some(Value::String(value)) => Some(
            OtlpSignal::from_str(&value)
                .ok_or_else(|| format!("unknown OTLP tombstone signal {value:?}"))?,
        ),
        _ => return Err("OTLP tombstone signal must be a string or null".to_owned()),
    };
    let reason = take_string(fields, "reason")?;
    let reason = DropReason::parse(&reason)
        .ok_or_else(|| format!("unknown OTLP tombstone reason {reason:?}"))?;
    let tombstone = DropTombstone {
        id,
        signal,
        reason,
        dropped_items: take_u64(fields, "dropped_items")?,
    };
    if tombstone.id.is_empty() {
        return Err("OTLP drop tombstone id must not be empty".to_owned());
    }
    Ok(tombstone)
}

fn decode_counters(value: Value) -> Result<OtlpEdgeCounters, String> {
    let mut fields = into_object(value, "counters")?;
    let counters = OtlpEdgeCounters {
        bind_failures: take_u64(&mut fields, "bind_failures")?,
        rejected_requests: take_u64(&mut fields, "rejected_requests")?,
        dropped_batches: take_u64(&mut fields, "dropped_batches")?,
        dropped_items: take_u64(&mut fields, "dropped_items")?,
        accepted_batches: take_u64(&mut fields, "accepted_batches")?,
        queued_splits: take_u64(&mut fields, "queued_splits")?,
        forwarded_splits: take_u64(&mut fields, "forwarded_splits")?,
        scrubbed_fields: take_u64(&mut fields, "scrubbed_fields")?,
    };
    reject_unknown(&fields, "OTLP edge counters")?;
    Ok(counters)
}

/// `{"<kind>":<body>}\n` — a body that is a JSON value.
fn encode_record(kind: &str, body: &Value) -> Vec<u8> {
    let mut line = Vec::new();
    line.extend_from_slice(b"{");
    line.extend_from_slice(&serde_json::to_vec(kind).expect("a string encodes"));
    line.extend_from_slice(b":");
    line.extend_from_slice(&serde_json::to_vec(body).expect("a JSON value encodes"));
    line.extend_from_slice(b"}\n");
    line
}

/// A split's fields, with its already-encoded body spliced in unchanged.
fn encode_split(line: &mut Vec<u8>, split: &QueuedSplit, envelope: bool) {
    let mut fields = serde_json::json!({
        "id": split.id,
        "split_index": split.split_index,
        "split_count": split.split_count,
        "item_start": split.item_start,
        "item_end": split.item_end,
    });
    if envelope {
        let object = fields.as_object_mut().expect("split fields are an object");
        object.insert("signal".to_owned(), Value::from(split.signal.as_str()));
        object.insert(
            "original_bytes".to_owned(),
            Value::from(split.original_bytes),
        );
        object.insert(
            "scrubbed_batch_bytes".to_owned(),
            Value::from(split.scrubbed_batch_bytes),
        );
    }
    let head = serde_json::to_vec(&fields).expect("split fields encode");
    line.extend_from_slice(&head[..head.len() - 1]);
    line.extend_from_slice(b",\"body\":");
    line.extend_from_slice(&split.body);
    line.extend_from_slice(b"}");
}

fn encode_accepted(record: &AcceptedRecord) -> Vec<u8> {
    let head = serde_json::to_vec(&serde_json::json!({
        "signal": record.signal.as_str(),
        "sequence": record.sequence,
        "original_bytes": record.original_bytes,
        "scrubbed_batch_bytes": record.scrubbed_batch_bytes,
        "scrubbed_fields": record.scrubbed_fields,
    }))
    .expect("accepted fields encode");
    let mut line = Vec::with_capacity(
        head.len()
            + 32
            + record
                .splits
                .iter()
                .map(|split| split.body.len() + 128)
                .sum::<usize>(),
    );
    line.extend_from_slice(b"{\"accepted\":");
    line.extend_from_slice(&head[..head.len() - 1]);
    line.extend_from_slice(b",\"splits\":[");
    for (index, split) in record.splits.iter().enumerate() {
        if index > 0 {
            line.extend_from_slice(b",");
        }
        encode_split(&mut line, split, false);
    }
    line.extend_from_slice(b"]}}\n");
    line
}

fn encode_rejected(record: &RejectedRecord) -> Vec<u8> {
    encode_record(
        "rejected",
        &serde_json::json!({
            "id": record.id(),
            "sequence": record.sequence,
            "signal": record.signal.map(OtlpSignal::as_str),
            "reason": record.reason.as_str(),
            "dropped_items": record.dropped_items,
            "terminal": record.terminal,
        }),
    )
}

fn encode_snapshot(fold: &Fold) -> Vec<u8> {
    let tombstones = fold
        .tombstones
        .iter()
        .map(|item| {
            serde_json::json!({
                "id": item.id,
                "signal": item.signal.map(OtlpSignal::as_str),
                "reason": item.reason.as_str(),
                "dropped_items": item.dropped_items,
            })
        })
        .collect::<Vec<_>>();
    let head = serde_json::to_vec(&serde_json::json!({
        "version": VERSION,
        "next_sequence": fold.next_sequence,
        "tombstones": tombstones,
        "counters": encode_counters(fold.counters),
    }))
    .expect("snapshot fields encode");
    let mut line = Vec::new();
    line.extend_from_slice(b"{\"snapshot\":");
    line.extend_from_slice(&head[..head.len() - 1]);
    line.extend_from_slice(b",\"queue\":[");
    for (index, split) in fold.queue.iter().enumerate() {
        if index > 0 {
            line.extend_from_slice(b",");
        }
        encode_split(&mut line, split, true);
    }
    line.extend_from_slice(b"]}}\n");
    line
}

fn encode_counters(counters: OtlpEdgeCounters) -> Value {
    serde_json::json!({
        "bind_failures": counters.bind_failures,
        "rejected_requests": counters.rejected_requests,
        "dropped_batches": counters.dropped_batches,
        "dropped_items": counters.dropped_items,
        "accepted_batches": counters.accepted_batches,
        "queued_splits": counters.queued_splits,
        "forwarded_splits": counters.forwarded_splits,
        "scrubbed_fields": counters.scrubbed_fields,
    })
}

fn into_object(value: Value, context: &str) -> Result<Map<String, Value>, String> {
    match value {
        Value::Object(fields) => Ok(fields),
        _ => Err(format!("OTLP custody {context} must be an object")),
    }
}

fn take_array(fields: &mut Map<String, Value>, name: &str) -> Result<Vec<Value>, String> {
    match fields.remove(name) {
        Some(Value::Array(value)) => Ok(value),
        _ => Err(format!("OTLP custody field {name:?} must be an array")),
    }
}

fn take_string(fields: &mut Map<String, Value>, name: &str) -> Result<String, String> {
    match fields.remove(name) {
        Some(Value::String(value)) => Ok(value),
        _ => Err(format!("OTLP custody field {name:?} must be a string")),
    }
}

fn take_signal(fields: &mut Map<String, Value>, name: &str) -> Result<OtlpSignal, String> {
    let value = take_string(fields, name)?;
    OtlpSignal::from_str(&value).ok_or_else(|| format!("unknown OTLP signal {value:?}"))
}

fn take_u64(fields: &mut Map<String, Value>, name: &str) -> Result<u64, String> {
    fields
        .remove(name)
        .and_then(|value| value.as_u64())
        .ok_or_else(|| format!("OTLP custody field {name:?} must be u64"))
}

fn checked_add(current: u64, amount: u64, name: &str) -> Result<u64, String> {
    current
        .checked_add(amount)
        .ok_or_else(|| format!("OTLP edge counter {name:?} overflow"))
}

fn reject_unknown(fields: &Map<String, Value>, context: &str) -> Result<(), String> {
    if let Some(unknown) = fields.keys().next() {
        return Err(format!("{context} has unknown field {unknown:?}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "circular-catch-otlp-state-{}-{label}",
            std::process::id()
        ))
    }

    fn fresh(label: &str) -> PathBuf {
        let directory = directory(label);
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("create state directory");
        directory
    }

    fn split(body: &str) -> PendingSplit {
        PendingSplit {
            item_start: 0,
            item_end: 1,
            body: body.as_bytes().to_vec(),
        }
    }

    fn log_lines(directory: &Path) -> Vec<String> {
        std::fs::read_to_string(directory.join(FILE_NAME))
            .expect("read custody log")
            .lines()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn fresh_actor_ignores_legacy_custody_and_resume_preserves_its_own_pending() {
        let root = directory("fresh-actor-no-migration");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("old-catch")).unwrap();
        let legacy = root.join("old-catch").join(RETIRED_FILE_NAME);
        let legacy_bytes = b"retired custody: never read or rewritten";
        std::fs::write(&legacy, legacy_bytes).unwrap();
        assert!(OtlpStateStore::create_empty(&root.join("old-catch")).is_err());
        let actor = root.join("new-actor");
        let mut store = OtlpStateStore::create_empty(&actor).unwrap();
        assert_eq!(store.queue_len(), 0);
        assert_eq!(store.counters(), OtlpEdgeCounters::default());
        store
            .enqueue(
                OtlpSignal::Metrics,
                vec![split(r#"{"resourceMetrics":[]}"#)],
                22,
                22,
                0,
            )
            .unwrap();
        drop(store);
        assert!(OtlpStateStore::create_empty(&actor).is_err());
        let resumed = OtlpStateStore::open(&actor).unwrap();
        assert_eq!(resumed.queue_len(), 1);
        assert_eq!(
            resumed.front().unwrap().id,
            "otlp-metrics-00000000000000000000-0001-of-0001"
        );
        assert_eq!(resumed.counters().accepted_batches, 1);
        assert_eq!(std::fs::read(legacy).unwrap(), legacy_bytes);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn each_change_appends_one_record_and_never_rewrites_what_is_written() {
        let directory = fresh("append-only");
        let custody = directory.join("custody");
        let mut store = OtlpStateStore::create_empty(&custody).unwrap();
        let mut before = std::fs::read(custody.join(FILE_NAME)).unwrap();
        assert_eq!(
            log_lines(&custody).len(),
            1,
            "a fresh custody is one snapshot"
        );
        let body = r#"{"resourceLogs":[{"scopeLogs":[{"logRecords":[{"severityNumber":9}]}]}]}"#;
        for step in 0..5 {
            store
                .enqueue(OtlpSignal::Logs, vec![split(body)], 70, 70, 0)
                .unwrap();
            let after = std::fs::read(custody.join(FILE_NAME)).unwrap();
            assert!(after.starts_with(&before), "step {step} rewrote the log");
            let added = &after[before.len()..];
            assert_eq!(added.iter().filter(|byte| **byte == b'\n').count(), 1);
            assert!(added.ends_with(b"\n"));
            assert!(
                String::from_utf8_lossy(added).contains(body),
                "step {step} did not carry the body verbatim"
            );
            before = after;
        }
        let id = store.front().unwrap().id;
        assert_eq!(id, "otlp-logs-00000000000000000000-0001-of-0001");
        store.acknowledge_forward(&id).unwrap();
        store
            .record_rejection(Some(OtlpSignal::Logs), DropReason::QueueCapacity, 2, false)
            .unwrap();
        let after = std::fs::read(custody.join(FILE_NAME)).unwrap();
        assert!(after.starts_with(&before));
        let lines = log_lines(&custody);
        assert_eq!(lines.len(), 1 + 5 + 2);
        assert_eq!(
            lines[6],
            r#"{"forwarded":{"id":"otlp-logs-00000000000000000000-0001-of-0001"}}"#
        );
        let reopened = OtlpStateStore::open(&custody).unwrap();
        assert_eq!(reopened.queue_len(), 4);
        assert_eq!(
            reopened.front().unwrap().id,
            "otlp-logs-00000000000000000001-0001-of-0001"
        );
        assert_eq!(&*reopened.front().unwrap().body, body.as_bytes());
        assert_eq!(
            reopened.counters(),
            OtlpEdgeCounters {
                accepted_batches: 5,
                queued_splits: 5,
                forwarded_splits: 1,
                rejected_requests: 1,
                ..OtlpEdgeCounters::default()
            }
        );
        assert_eq!(
            reopened.latest_drop_reason(),
            Some(DropReason::QueueCapacity)
        );
        assert_eq!(log_lines(&custody).len(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn settled_records_are_compacted_at_the_queue_capacity_and_the_fold_survives() {
        let directory = fresh("compaction");
        let mut store = OtlpStateStore::open(&directory).unwrap();
        let body = r#"{"resourceLogs":[]}"#;
        for _ in 0..2 {
            store
                .enqueue(OtlpSignal::Logs, vec![split(body)], 19, 19, 1)
                .unwrap();
        }
        let first_kept = store.front().unwrap().id;
        let mut settled = Vec::new();
        for round in 0..64_usize {
            store
                .enqueue(OtlpSignal::Logs, vec![split(body)], 19, 19, 1)
                .unwrap();
            let head = store.front().unwrap().id;
            store.acknowledge_forward(&head).unwrap();
            settled.push(head);
            if round < 63 {
                assert_eq!(
                    log_lines(&directory).len(),
                    1 + 2 + 2 * (round + 1),
                    "round {round}"
                );
            }
        }
        assert_eq!(settled[0], first_kept);
        let lines = log_lines(&directory);
        assert_eq!(lines.len(), 1, "64 settled records compact the log");
        assert!(lines[0].starts_with(r#"{"snapshot":"#));
        let reopened = OtlpStateStore::open(&directory).unwrap();
        assert_eq!(reopened.queue_len(), 2);
        assert_eq!(
            reopened.front().unwrap().id,
            "otlp-logs-00000000000000000064-0001-of-0001"
        );
        assert_eq!(
            reopened.counters(),
            OtlpEdgeCounters {
                accepted_batches: 66,
                queued_splits: 66,
                forwarded_splits: 64,
                scrubbed_fields: 66,
                ..OtlpEdgeCounters::default()
            }
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_torn_final_record_is_dropped_on_reopen_and_the_log_stays_appendable() {
        let directory = fresh("torn-tail");
        let mut store = OtlpStateStore::open(&directory).unwrap();
        store
            .enqueue(OtlpSignal::Logs, vec![split(r#"{"a":1}"#)], 7, 7, 0)
            .unwrap();
        drop(store);
        let mut log = OpenOptions::new()
            .append(true)
            .open(directory.join(FILE_NAME))
            .unwrap();
        log.write_all(br#"{"accepted":{"signal":"logs","sequ"#)
            .unwrap();
        drop(log);
        let mut reopened = OtlpStateStore::open(&directory).unwrap();
        assert_eq!(reopened.queue_len(), 1);
        assert_eq!(reopened.counters().accepted_batches, 1);
        assert_eq!(log_lines(&directory).len(), 1);
        reopened
            .enqueue(OtlpSignal::Logs, vec![split(r#"{"b":2}"#)], 7, 7, 0)
            .unwrap();
        drop(reopened);
        let again = OtlpStateStore::open(&directory).unwrap();
        assert_eq!(again.queue_len(), 2);
        assert_eq!(again.counters().accepted_batches, 2);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_complete_record_that_does_not_fold_refuses_the_custody() {
        for (label, line) in [
            ("not-json", "this is not a record\n"),
            ("not-head", "{\"forwarded\":{\"id\":\"otlp-logs-9\"}}\n"),
            ("unknown-kind", "{\"rewound\":{}}\n"),
            (
                "second-snapshot",
                "{\"snapshot\":{\"version\":1,\"next_sequence\":0,\"tombstones\":[],\"counters\":{},\"queue\":[]}}\n",
            ),
        ] {
            let directory = fresh(&format!("refuse-{label}"));
            let mut store = OtlpStateStore::open(&directory).unwrap();
            store
                .enqueue(OtlpSignal::Logs, vec![split(r#"{"a":1}"#)], 7, 7, 0)
                .unwrap();
            drop(store);
            let mut log = OpenOptions::new()
                .append(true)
                .open(directory.join(FILE_NAME))
                .unwrap();
            log.write_all(line.as_bytes()).unwrap();
            drop(log);
            let before = std::fs::read(directory.join(FILE_NAME)).unwrap();
            let refused = OtlpStateStore::open(&directory).err();
            assert!(refused.is_some(), "{label} was folded");
            assert_eq!(std::fs::read(directory.join(FILE_NAME)).unwrap(), before);
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn drop_tombstone_and_counters_are_durable_without_raw_payload() {
        let directory = fresh("drop-round-trip");
        let mut store = OtlpStateStore::open(&directory).expect("open state");
        store
            .record_rejection(Some(OtlpSignal::Metrics), DropReason::InvalidJson, 1, true)
            .expect("record terminal drop");
        let encoded = std::fs::read_to_string(directory.join(FILE_NAME)).expect("read state");
        assert!(encoded.contains("invalid_json"));
        assert!(!encoded.contains("private-payload-sentinel"));
        let reopened = OtlpStateStore::open(&directory).expect("reopen state");
        assert_eq!(reopened.counters().rejected_requests, 1);
        assert_eq!(reopened.counters().dropped_batches, 1);
        assert_eq!(reopened.counters().dropped_items, 1);
        std::fs::remove_dir_all(directory).expect("cleanup");
    }

    #[test]
    fn drop_reason_spellings_are_stable_and_timeout_round_trips() {
        let established = [
            (DropReason::InvalidRequest, "invalid_request"),
            (
                DropReason::UnsupportedContentType,
                "unsupported_content_type",
            ),
            (
                DropReason::UnsupportedContentEncoding,
                "unsupported_content_encoding",
            ),
            (DropReason::InvalidJson, "invalid_json"),
            (
                DropReason::UnsupportedSignalShape,
                "unsupported_signal_shape",
            ),
            (DropReason::UnsplittableItem, "unsplittable_item"),
            (DropReason::ScrubFailure, "scrub_failure"),
            (DropReason::QueueCapacity, "queue_capacity"),
        ];
        for (reason, spelling) in established {
            assert_eq!(reason.as_str().as_bytes(), spelling.as_bytes());
            assert_eq!(DropReason::parse(spelling), Some(reason));
        }
        assert_eq!(DropReason::Timeout.as_str(), "timeout");
        assert_eq!(DropReason::parse("timeout"), Some(DropReason::Timeout));
        assert_eq!(DropReason::parse("future_reason"), None);

        let directory = fresh("timeout-drop-round-trip");
        let mut store = OtlpStateStore::open(&directory).expect("open state");
        store
            .record_rejection(Some(OtlpSignal::Logs), DropReason::Timeout, 1, false)
            .expect("record retryable timeout");
        let reopened = OtlpStateStore::open(&directory).expect("reopen state");
        assert_eq!(reopened.latest_drop_reason(), Some(DropReason::Timeout));
        assert_eq!(reopened.counters().rejected_requests, 1);
        assert_eq!(reopened.counters().dropped_batches, 0);
        assert_eq!(reopened.counters().dropped_items, 0);
        std::fs::remove_dir_all(directory).expect("cleanup");
    }
}
