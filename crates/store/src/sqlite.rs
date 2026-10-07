
use crate::{
    AtomicStoreTransactionPort, CheckpointOwners, CrashDurableTransactionPort, GroupCommitFailure,
    GroupTransactionPort, RecoveredTransactionModel, RecoveringTransactionModel, RecoveryTerminal,
    StoreTransaction, StoreTransactionFailure, StoreTransactionFailureReason, StoreTransactionOp,
    StoreTransactionReceipt, StoreTransactionReject, TransactionRecoveryFailure,
    TransactionRecoveryPlan, TransactionSchema,
};
use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use std::fmt::{self, Debug};
use std::fs::{self, OpenOptions};
use std::io;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub const SQLITE_FIXED_SCHEMA_VERSION: u32 = 1;

const APPLICATION_ID: i32 = 0x4349_5243;
const MAX_SQLITE_SEQUENCE: u64 = i64::MAX as u64;

const CREATE_SCHEMA: &str = "
BEGIN EXCLUSIVE;
PRAGMA application_id = 1128878659;
PRAGMA user_version = 1;
CREATE TABLE circular_meta (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL CHECK (schema_version > 0),
    last_sequence INTEGER NOT NULL CHECK (last_sequence >= 0)
);
CREATE TABLE circular_commits (
    sequence INTEGER PRIMARY KEY CHECK (sequence > 0),
    payload BLOB NOT NULL,
    payload_sha256 BLOB NOT NULL CHECK (length(payload_sha256) = 32)
);
INSERT INTO circular_meta(singleton, schema_version, last_sequence) VALUES (1, 1, 0);
COMMIT;
";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SqliteCommitSequence(u64);

impl SqliteCommitSequence {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SqlitePayloadChecksum([u8; 32]);

impl SqlitePayloadChecksum {
    #[must_use]
    pub const fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteCommitReceipt {
    sequence: SqliteCommitSequence,
    checksum: SqlitePayloadChecksum,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteCommitGroupReceipt {
    receipts: Box<[SqliteCommitReceipt]>,
}

impl SqliteCommitGroupReceipt {
    #[must_use]
    pub const fn receipts(&self) -> &[SqliteCommitReceipt] {
        &self.receipts
    }

    #[must_use]
    pub fn into_receipts(self) -> Box<[SqliteCommitReceipt]> {
        self.receipts
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SqliteGroupCommitCandidate {
    max_batch_size: NonZeroUsize,
    max_wait: Duration,
}

impl SqliteGroupCommitCandidate {
    #[must_use]
    pub const fn new(max_batch_size: NonZeroUsize, max_wait: Duration) -> Self {
        Self {
            max_batch_size,
            max_wait,
        }
    }

    #[must_use]
    pub const fn max_batch_size(self) -> NonZeroUsize {
        self.max_batch_size
    }

    #[must_use]
    pub const fn max_wait(self) -> Duration {
        self.max_wait
    }
}

impl SqliteCommitReceipt {
    #[must_use]
    pub const fn sequence(&self) -> SqliteCommitSequence {
        self.sequence
    }

    #[must_use]
    pub const fn checksum(&self) -> SqlitePayloadChecksum {
        self.checksum
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteJournalEntry {
    sequence: SqliteCommitSequence,
    checksum: SqlitePayloadChecksum,
    payload: Box<[u8]>,
}

impl SqliteJournalEntry {
    #[must_use]
    pub const fn sequence(&self) -> SqliteCommitSequence {
        self.sequence
    }

    #[must_use]
    pub const fn checksum(&self) -> SqlitePayloadChecksum {
        self.checksum
    }

    /// Decode the existing backend namespace envelope without re-encoding its body.
    #[must_use]
    pub fn namespace_payload(&self) -> Result<Option<(&str, &[u8])>, SqliteJournalError> {
        decode_namespace(&self.payload)
    }

    #[must_use]
    pub const fn payload(&self) -> &[u8] {
        &self.payload
    }
}

impl SqliteJournalEntry {
    /// Decode only the backend-owned namespace envelope from a physical snapshot.
    /// Retired entries return None. Product payload codecs stay with their owners.
    pub fn namespaced_payload(&self) -> Result<Option<(&str, &[u8])>, SqliteJournalError> {
        decode_namespace(self.payload())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteJournalSnapshot {
    through: Option<SqliteCommitSequence>,
    entries: Box<[SqliteJournalEntry]>,
}

impl SqliteJournalSnapshot {
    #[must_use]
    pub const fn through(&self) -> Option<SqliteCommitSequence> {
        self.through
    }

    #[must_use]
    pub const fn entries(&self) -> &[SqliteJournalEntry] {
        &self.entries
    }
}

#[derive(Debug)]
pub enum SqliteJournalError {
    Io {
        operation: &'static str,
        source: io::Error,
    },
    Database(rusqlite::Error),
    AlreadyExists(PathBuf),
    Missing(PathBuf),
    NotRegularFile(PathBuf),
    PermissionsTooBroad {
        path: PathBuf,
        mode: u32,
    },
    Poisoned,
    Contended,
    WrongApplicationId {
        found: i32,
    },
    UnsupportedSchema {
        found: u32,
        supported: u32,
    },
    UnsupportedJournalFormat {
        entry: Option<u64>,
        found: Option<u8>,
    },
    UnsupportedRecordFormat {
        entry: u64,
        vocabulary: &'static str,
        found: u64,
    },
    SchemaObjects {
        found: Box<[String]>,
    },
    Integrity {
        detail: String,
    },
    MetadataRows {
        found: usize,
    },
    MetadataSequenceMismatch {
        declared: u64,
        found: u64,
    },
    SequenceExhausted,
    SequenceOutOfRange(i64),
    SequenceGap {
        expected: u64,
        found: u64,
    },
    PayloadChecksum {
        sequence: u64,
    },
    EmptyCommitGroup,
    ReceiptMismatch {
        detail: String,
    },
}

impl fmt::Display for SqliteJournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { operation, source } => write!(formatter, "{operation}: {source}"),
            Self::Database(source) => write!(formatter, "SQLite error: {source}"),
            Self::AlreadyExists(path) => {
                write!(formatter, "store already exists: {}", path.display())
            }
            Self::Missing(path) => write!(formatter, "store does not exist: {}", path.display()),
            Self::NotRegularFile(path) => {
                write!(
                    formatter,
                    "store path is not a regular file: {}",
                    path.display()
                )
            }
            Self::PermissionsTooBroad { path, mode } => write!(
                formatter,
                "store permissions are broader than owner-only: {} ({mode:o})",
                path.display()
            ),
            Self::Poisoned => formatter.write_str("SQLite handle is poisoned; reopen is required"),
            Self::Contended => formatter.write_str(
                "another connection holds the write lock; nothing was written and this attempt can be made again",
            ),
            Self::WrongApplicationId { found } => {
                write!(formatter, "wrong SQLite application id: {found}")
            }
            Self::UnsupportedSchema { found, supported } => write!(
                formatter,
                "unsupported SQLite schema version {found}; this build supports {supported}"
            ),
            Self::UnsupportedJournalFormat { entry, found } => write!(
                formatter,
                "journal format rejected code={}: this daemon does not read this older or unknown journal format (entry {}: journal format version {}; supported {})",
                circular_protocol::rejection_code::RejectionReason::JournalFormatRejected.recorded_code(),
                entry.map_or_else(|| "unknown".to_owned(), |n| n.to_string()),
                found.map_or_else(|| "missing".to_owned(), |n| n.to_string()),
                JOURNAL_FORMAT_VERSION
            ),
            Self::UnsupportedRecordFormat { entry, vocabulary, found } => write!(
                formatter,
                "journal format rejected code={} entry={entry} {vocabulary}={found}",
                circular_protocol::rejection_code::RejectionReason::JournalFormatRejected.recorded_code()
            ),
            Self::SchemaObjects { found } => {
                write!(formatter, "unexpected SQLite schema objects: {found:?}")
            }
            Self::Integrity { detail } => write!(formatter, "SQLite integrity failure: {detail}"),
            Self::MetadataRows { found } => {
                write!(formatter, "expected one metadata row, found {found}")
            }
            Self::MetadataSequenceMismatch { declared, found } => write!(
                formatter,
                "metadata declares sequence {declared}, journal ends at {found}"
            ),
            Self::SequenceExhausted => formatter.write_str("SQLite commit sequence exhausted"),
            Self::SequenceOutOfRange(found) => {
                write!(formatter, "SQLite commit sequence is out of range: {found}")
            }
            Self::SequenceGap { expected, found } => {
                write!(
                    formatter,
                    "SQLite commit sequence gap: expected {expected}, found {found}"
                )
            }
            Self::PayloadChecksum { sequence } => {
                write!(
                    formatter,
                    "SQLite payload checksum mismatch at sequence {sequence}"
                )
            }
            Self::EmptyCommitGroup => formatter.write_str("SQLite commit group is empty"),
            Self::ReceiptMismatch { detail } => {
                write!(formatter, "SQLite commit receipt mismatch: {detail}")
            }
        }
    }
}

impl std::error::Error for SqliteJournalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Database(source) => Some(source),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for SqliteJournalError {
    fn from(source: rusqlite::Error) -> Self {
        Self::Database(source)
    }
}

pub struct SqliteJournal {
    connection: Connection,
    path: PathBuf,
    poisoned: bool,
    namespace: Option<String>,
    retained_bytes: std::cell::Cell<Option<u64>>,
}

impl SqliteJournal {
    /// Independent read connection for decoding an immutable prefix while this
    /// handle owns the write transaction. It grants no encoder authority.
    pub fn read_source(&self) -> Result<Self, SqliteJournalError> {
        match &self.namespace {
            Some(namespace) => Self::open_read_only_namespace(&self.path, namespace),
            None => Err(SqliteJournalError::Integrity {
                detail: "product column read needs a namespace".into(),
            }),
        }
    }
    pub fn create(path: impl AsRef<Path>) -> Result<Self, SqliteJournalError> {
        let path = path.as_ref().to_path_buf();
        create_owner_only_file(&path)?;
        let connection = open_read_write(&path)?;
        let mut journal = Self {
            connection,
            path,
            poisoned: false,
            namespace: None,
            retained_bytes: std::cell::Cell::new(None),
        };
        journal.configure_durable_connection()?;
        journal.connection.execute_batch(CREATE_SCHEMA)?;
        journal.validate_fixed_schema()?;
        let snapshot = journal.snapshot()?;
        debug_assert!(snapshot.entries().is_empty());
        Ok(journal)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, SqliteJournalError> {
        let path = path.as_ref().to_path_buf();
        validate_existing_file(&path)?;
        let connection = open_read_write(&path)?;
        let mut journal = Self {
            connection,
            path,
            poisoned: false,
            namespace: None,
            retained_bytes: std::cell::Cell::new(None),
        };
        journal.validate_fixed_schema()?;
        journal.configure_durable_connection()?;
        journal.validate_fixed_schema()?;
        journal.validate_sequence_head()?;
        Ok(journal)
    }

    /// Read one exact committed prefix without returning a writable journal handle.
    ///
    /// The connection is opened with SQLite's `READ_ONLY` flag and is consumed
    /// inside this function.  Callers receive owned bytes only, so the type
    /// boundary exposes neither `commit` nor a connection that could be
    /// upgraded into a writer.
    pub fn read_only_snapshot(
        path: impl AsRef<Path>,
    ) -> Result<SqliteJournalSnapshot, SqliteJournalError> {
        let path = path.as_ref().to_path_buf();
        validate_existing_file(&path)?;
        let mut journal = Self {
            connection: open_read_only(&path)?,
            path,
            poisoned: false,
            namespace: None,
            retained_bytes: std::cell::Cell::new(None),
        };
        journal.validate_fixed_schema()?;
        journal.snapshot()
    }

    /// Read only newly committed physical entries in one SQLite read transaction.
    /// `after` is a reader-local physical position, never a public record cursor.
    pub fn read_only_since(
        path: impl AsRef<Path>,
        after: u64,
    ) -> Result<SqliteJournalSnapshot, SqliteJournalError> {
        Self::read_only_since_retaining(path, after, &[]).map(|(snapshot, _)| snapshot)
    }

    /// Read the next immutable prefix and validate retained namespace witnesses
    /// in that same SQLite transaction. A witness is an existing physical row
    /// and checksum, never a new durable field or a wire cursor. Whole-namespace
    /// pruning replaces that row, so a live reader cannot silently cross a gap.
    pub fn read_only_since_retaining(
        path: impl AsRef<Path>,
        after: u64,
        retained: &[(SqliteCommitSequence, SqlitePayloadChecksum)],
    ) -> Result<(SqliteJournalSnapshot, bool), SqliteJournalError> {
        let path = path.as_ref();
        validate_existing_file(path)?;
        let mut journal = Self {
            connection: open_read_only(path)?,
            path: path.to_owned(),
            poisoned: false,
            retained_bytes: std::cell::Cell::new(None),
            namespace: None,
        };
        if after == 0 {
            journal.validate_fixed_schema()?;
        } else {
            journal.validate_schema_identity()?;
        }
        let transaction = journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let mut intact = true;
        {
            let mut statement = transaction
                .prepare("SELECT payload_sha256 FROM circular_commits WHERE sequence = ?1")?;
            for (sequence, checksum) in retained {
                let found = statement
                    .query_row([sequence.get()], |row| row.get::<_, Vec<u8>>(0))
                    .optional()?;
                intact &= found.as_deref() == Some(checksum.bytes().as_slice());
            }
        }
        let snapshot = read_snapshot_after(&transaction, after)?;
        transaction.commit()?;
        Ok((snapshot, intact))
    }

    pub fn read_only_namespace_sequences(
        path: impl AsRef<Path>,
        after: u64,
        namespaces: &[&str],
    ) -> Result<(u64, Vec<(u64, usize)>), SqliteJournalError> {
        let path = path.as_ref();
        validate_existing_file(path)?;
        let mut journal = Self {
            connection: open_read_only(path)?,
            path: path.to_owned(),
            poisoned: false,
            retained_bytes: std::cell::Cell::new(None),
            namespace: None,
        };
        if after == 0 {
            journal.validate_fixed_schema()?;
        } else {
            journal.validate_schema_identity()?;
        }
        let prefixes = namespaces
            .iter()
            .map(|namespace| namespace_envelope_prefix(namespace))
            .collect::<Vec<_>>();
        let mut kind = String::from("CASE");
        for (index, prefix) in prefixes.iter().enumerate() {
            kind.push_str(&format!(
                " WHEN substr(payload, 1, {}) = ?{} THEN {index}",
                prefix.len(),
                index + 2
            ));
        }
        kind.push_str(" END");
        let transaction = journal
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)?;
        let through: i64 = transaction.query_row(
            "SELECT last_sequence FROM circular_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        let through = sqlite_sequence(through)?;
        let mut found = Vec::new();
        {
            let mut statement = transaction.prepare(&format!(
                "SELECT sequence, kind FROM (SELECT sequence, {kind} AS kind FROM circular_commits \
                 WHERE sequence > ?1) WHERE kind IS NOT NULL ORDER BY sequence"
            ))?;
            let mut values: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(prefixes.len() + 1);
            let after_sql =
                i64::try_from(after).map_err(|_| SqliteJournalError::SequenceExhausted)?;
            values.push(&after_sql);
            for prefix in &prefixes {
                values.push(prefix);
            }
            let rows = statement.query_map(values.as_slice(), |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })?;
            for row in rows {
                let (sequence, kind) = row?;
                let kind = usize::try_from(kind).map_err(|_| SqliteJournalError::Integrity {
                    detail: "namespace kind out of range".to_owned(),
                })?;
                found.push((sqlite_sequence(sequence)?, kind));
            }
        }
        transaction.commit()?;
        Ok((through, found))
    }

    /// Open a storage namespace in one physical journal. Payload codecs remain
    /// unchanged; the backend routing envelope is not a product record.
    /// All namespaces share the physical commit sequence and SQLite sync point.
    pub fn open_namespace(
        path: impl AsRef<Path>,
        namespace: &str,
    ) -> Result<Self, SqliteJournalError> {
        let mut journal = match Self::open(path.as_ref()) {
            Ok(journal) => journal,
            Err(SqliteJournalError::Missing(_)) => Self::create(path)?,
            Err(error) => return Err(error),
        };
        journal.namespace = Some(namespace.to_owned());
        Ok(journal)
    }

    pub fn open_namespace_after(
        path: impl AsRef<Path>,
        namespace: &str,
        folded_through: u64,
    ) -> Result<(Self, SqliteJournalSnapshot), SqliteJournalError> {
        let mut journal = match Self::open(path.as_ref()) {
            Ok(journal) => journal,
            Err(SqliteJournalError::Missing(_)) if folded_through == 0 => Self::create(path)?,
            Err(error) => return Err(error),
        };
        journal.namespace = Some(namespace.to_owned());
        let tail = journal.snapshot_since(folded_through)?;
        journal
            .retained_bytes
            .set(Some(journal.measure_namespace_bytes(namespace)?));
        Ok((journal, tail))
    }

    fn measure_namespace_bytes(&self, namespace: &str) -> Result<u64, SqliteJournalError> {
        let prefix = namespace_envelope_prefix(namespace);
        let width = i64::try_from(prefix.len()).map_err(|_| SqliteJournalError::Integrity {
            detail: "journal namespace envelope exceeds the query domain".to_owned(),
        })?;
        let total: i64 = self.connection.query_row(
            "SELECT COALESCE(SUM(length(payload) - ?1), 0) FROM circular_commits \
             WHERE substr(payload, 1, ?1) = ?2",
            params![width, prefix],
            |row| row.get(0),
        )?;
        u64::try_from(total).map_err(|_| SqliteJournalError::Integrity {
            detail: "retained namespace bytes exceed the byte domain".to_owned(),
        })
    }

    fn validate_sequence_head(&self) -> Result<(), SqliteJournalError> {
        let declared: i64 = self.connection.query_row(
            "SELECT last_sequence FROM circular_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        let found: Option<i64> =
            self.connection
                .query_row("SELECT MAX(sequence) FROM circular_commits", [], |row| {
                    row.get(0)
                })?;
        let declared = sqlite_sequence(declared)?;
        let found = found.map(sqlite_sequence).transpose()?.unwrap_or(0);
        if declared != found {
            return Err(SqliteJournalError::MetadataSequenceMismatch { declared, found });
        }
        Ok(())
    }

    #[must_use]
    pub fn retained_namespace_bytes(&self) -> Option<u64> {
        let namespace = self.namespace.as_deref()?;
        if let Some(bytes) = self.retained_bytes.get() {
            return Some(bytes);
        }
        let measured = self.measure_namespace_bytes(namespace).ok()?;
        self.retained_bytes.set(Some(measured));
        Some(measured)
    }

    pub fn open_read_only_namespace(
        path: impl AsRef<Path>,
        namespace: &str,
    ) -> Result<Self, SqliteJournalError> {
        let path = path.as_ref().to_path_buf();
        validate_existing_file(&path)?;
        let journal = Self {
            connection: open_read_only(&path)?,
            path,
            poisoned: false,
            namespace: Some(namespace.to_owned()),
            retained_bytes: std::cell::Cell::new(None),
        };
        journal.validate_fixed_schema()?;
        Ok(journal)
    }

    pub fn visit_namespace<E>(
        &self,
        after: u64,
        through: u64,
        mut visit: impl FnMut(SqliteCommitSequence, &[u8]) -> Result<(), E>,
    ) -> Result<Result<(), E>, SqliteJournalError> {
        let namespace = self
            .namespace
            .as_deref()
            .ok_or_else(|| SqliteJournalError::Integrity {
                detail: "namespace visit needs a routed handle".to_owned(),
            })?;
        let prefix = namespace_envelope_prefix(namespace);
        let mut current_format = NAMESPACE_MAGIC.to_vec();
        current_format.push(JOURNAL_FORMAT_VERSION);
        let declared: i64 = self.connection.query_row(
            "SELECT last_sequence FROM circular_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        if through > sqlite_sequence(declared)? {
            return Err(SqliteJournalError::MetadataSequenceMismatch {
                declared: sqlite_sequence(declared)?,
                found: through,
            });
        }
        let mut statement = self.connection.prepare(
            "SELECT sequence, CASE WHEN substr(payload, 1, ?3) = ?4 OR length(payload) < ?3 \
             OR substr(payload, 1, ?6) <> ?5 THEN payload END, payload_sha256 \
             FROM circular_commits WHERE sequence > ?1 AND sequence <= ?2 ORDER BY sequence",
        )?;
        let width = i64::try_from(prefix.len()).map_err(|_| SqliteJournalError::Integrity {
            detail: "journal namespace envelope exceeds the query domain".to_owned(),
        })?;
        let format_width = i64::try_from(current_format.len()).expect("fixed envelope head");
        let after_sql = i64::try_from(after).map_err(|_| SqliteJournalError::SequenceExhausted)?;
        let through_sql =
            i64::try_from(through).map_err(|_| SqliteJournalError::SequenceExhausted)?;
        let mut rows = statement.query(params![
            after_sql,
            through_sql,
            width,
            prefix,
            current_format,
            format_width
        ])?;
        let mut expected = after
            .checked_add(1)
            .ok_or(SqliteJournalError::SequenceExhausted)?;
        while let Some(row) = rows.next()? {
            let sequence = sqlite_sequence(row.get::<_, i64>(0)?)?;
            if sequence != expected {
                return Err(SqliteJournalError::SequenceGap {
                    expected,
                    found: sequence,
                });
            }
            expected = expected
                .checked_add(1)
                .ok_or(SqliteJournalError::SequenceExhausted)?;
            let Some(payload) = row.get::<_, Option<Vec<u8>>>(1)? else {
                continue;
            };
            let stored_checksum = row.get::<_, Vec<u8>>(2)?;
            if stored_checksum.as_slice() != payload_checksum(&payload).bytes() {
                return Err(SqliteJournalError::PayloadChecksum { sequence });
            }
            let Some((found, body)) = decode_namespace(&payload).map_err(|error| match error {
                SqliteJournalError::UnsupportedJournalFormat { found, .. } => {
                    SqliteJournalError::UnsupportedJournalFormat {
                        entry: Some(sequence),
                        found,
                    }
                }
                other => other,
            })?
            else {
                continue;
            };
            if found != namespace {
                continue;
            }
            if let Err(error) = visit(SqliteCommitSequence(sequence), body) {
                return Ok(Err(error));
            }
        }
        if through != 0 && expected - 1 != through {
            return Err(SqliteJournalError::SequenceGap {
                expected,
                found: through,
            });
        }
        Ok(Ok(()))
    }

    /// Read only the selected namespace, keeping physical sequence numbers.
    pub fn read_only_namespace(
        path: impl AsRef<Path>,
        namespace: &str,
    ) -> Result<SqliteJournalSnapshot, SqliteJournalError> {
        let path = path.as_ref().to_path_buf();
        validate_existing_file(&path)?;
        let mut journal = Self {
            connection: open_read_only(&path)?,
            path,
            poisoned: false,
            namespace: Some(namespace.to_owned()),
            retained_bytes: std::cell::Cell::new(None),
        };
        journal.validate_fixed_schema()?;
        journal.snapshot()
    }

    pub fn read_only_namespace_after(
        path: impl AsRef<Path>,
        namespace: &str,
        after: u64,
    ) -> Result<SqliteJournalSnapshot, SqliteJournalError> {
        let path = path.as_ref().to_path_buf();
        validate_existing_file(&path)?;
        let mut journal = Self {
            connection: open_read_only(&path)?,
            path,
            poisoned: false,
            namespace: Some(namespace.to_owned()),
            retained_bytes: std::cell::Cell::new(None),
        };
        journal.validate_fixed_schema()?;
        journal.snapshot_since(after)
    }

    pub fn namespace_first_commit(&self) -> Result<Option<u64>, SqliteJournalError> {
        let namespace = self
            .namespace
            .as_deref()
            .ok_or_else(|| SqliteJournalError::Integrity {
                detail: "namespace read needs a routed handle".to_owned(),
            })?;
        let prefix = namespace_envelope_prefix(namespace);
        let width = i64::try_from(prefix.len()).map_err(|_| SqliteJournalError::Integrity {
            detail: "journal namespace envelope exceeds the query domain".to_owned(),
        })?;
        let first: Option<i64> = self.connection.query_row(
            "SELECT MIN(sequence) FROM circular_commits WHERE substr(payload, 1, ?1) = ?2",
            params![width, prefix],
            |row| row.get(0),
        )?;
        first.map(sqlite_sequence).transpose()
    }

    pub fn visit_namespace_back<E>(
        &self,
        mut visit: impl FnMut(SqliteCommitSequence, &[u8]) -> Result<std::ops::ControlFlow<()>, E>,
    ) -> Result<Result<Option<u64>, E>, SqliteJournalError> {
        let namespace = self
            .namespace
            .as_deref()
            .ok_or_else(|| SqliteJournalError::Integrity {
                detail: "namespace visit needs a routed handle".to_owned(),
            })?;
        let prefix = namespace_envelope_prefix(namespace);
        let mut current_format = NAMESPACE_MAGIC.to_vec();
        current_format.push(JOURNAL_FORMAT_VERSION);
        let declared: i64 = self.connection.query_row(
            "SELECT last_sequence FROM circular_meta WHERE singleton = 1",
            [],
            |row| row.get(0),
        )?;
        let through = sqlite_sequence(declared)?;
        let mut statement = self.connection.prepare(
            "SELECT sequence, CASE WHEN substr(payload, 1, ?2) = ?3 OR length(payload) < ?2 \
             OR substr(payload, 1, ?5) <> ?4 THEN payload END, payload_sha256 \
             FROM circular_commits WHERE sequence <= ?1 ORDER BY sequence DESC",
        )?;
        let width = i64::try_from(prefix.len()).map_err(|_| SqliteJournalError::Integrity {
            detail: "journal namespace envelope exceeds the query domain".to_owned(),
        })?;
        let format_width = i64::try_from(current_format.len()).expect("fixed envelope head");
        let through_sql =
            i64::try_from(through).map_err(|_| SqliteJournalError::SequenceExhausted)?;
        let mut rows = statement.query(params![
            through_sql,
            width,
            prefix,
            current_format,
            format_width
        ])?;
        let mut expected = through;
        let mut visited = None;
        while let Some(row) = rows.next()? {
            let sequence = sqlite_sequence(row.get::<_, i64>(0)?)?;
            if sequence != expected {
                return Err(SqliteJournalError::SequenceGap {
                    expected,
                    found: sequence,
                });
            }
            expected = expected.saturating_sub(1);
            let Some(payload) = row.get::<_, Option<Vec<u8>>>(1)? else {
                continue;
            };
            let stored_checksum = row.get::<_, Vec<u8>>(2)?;
            if stored_checksum.as_slice() != payload_checksum(&payload).bytes() {
                return Err(SqliteJournalError::PayloadChecksum { sequence });
            }
            let Some((found, body)) = decode_namespace(&payload).map_err(|error| match error {
                SqliteJournalError::UnsupportedJournalFormat { found, .. } => {
                    SqliteJournalError::UnsupportedJournalFormat {
                        entry: Some(sequence),
                        found,
                    }
                }
                other => other,
            })?
            else {
                continue;
            };
            if found != namespace {
                continue;
            }
            visited = Some(sequence);
            match visit(SqliteCommitSequence(sequence), body) {
                Ok(std::ops::ControlFlow::Continue(())) => {}
                Ok(std::ops::ControlFlow::Break(())) => return Ok(Ok(visited)),
                Err(error) => return Ok(Err(error)),
            }
        }
        if expected != 0 {
            return Err(SqliteJournalError::SequenceGap { expected, found: 0 });
        }
        Ok(Ok(visited))
    }

    pub fn read_only_commit(
        path: impl AsRef<Path>,
        namespace: &str,
        sequence: u64,
    ) -> Result<Option<Vec<u8>>, SqliteJournalError> {
        let path = path.as_ref().to_path_buf();
        validate_existing_file(&path)?;
        let connection = open_read_only(&path)?;
        let journal = Self {
            connection,
            path,
            poisoned: false,
            namespace: None,
            retained_bytes: std::cell::Cell::new(None),
        };
        journal.validate_fixed_schema()?;
        let sql = i64::try_from(sequence)
            .map_err(|_| SqliteJournalError::SequenceOutOfRange(i64::MAX))?;
        let payload = journal
            .connection
            .query_row(
                "SELECT payload FROM circular_commits WHERE sequence = ?1",
                params![sql],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()?;
        let Some(payload) = payload else {
            return Ok(None);
        };
        Ok(decode_namespace(&payload)?
            .filter(|(found, _)| *found == namespace)
            .map(|(_, body)| body.to_vec()))
    }

    pub fn namespace_payload_bytes(
        path: impl AsRef<Path>,
        namespace: &str,
    ) -> Result<Option<u64>, SqliteJournalError> {
        let journal = Self::open_read_only_namespace(path, namespace)?;
        if journal.namespace_first_commit()?.is_none() {
            return Ok(None);
        }
        journal.measure_namespace_bytes(namespace).map(Some)
    }

    /// Physical allocation attributable to retained payloads in each namespace.
    /// SQLite pages and WAL overhead are measured separately by the file owner.
    pub fn namespace_bytes(
        path: impl AsRef<Path>,
    ) -> Result<std::collections::BTreeMap<String, u64>, SqliteJournalError> {
        let snapshot = Self::read_only_snapshot(path)?;
        let mut bytes = std::collections::BTreeMap::new();
        for entry in snapshot.entries() {
            if let Some((namespace, payload)) = decode_namespace(entry.payload())? {
                *bytes.entry(namespace.to_owned()).or_insert(0_u64) += payload.len() as u64;
            }
        }
        Ok(bytes)
    }

    pub fn namespace_payload_at(
        &self,
        commit: u64,
    ) -> Result<(SqliteCommitSequence, Vec<u8>), SqliteJournalError> {
        let (bytes, checksum): (Vec<u8>, Vec<u8>) = self.connection.query_row(
            "SELECT payload, payload_sha256 FROM circular_commits WHERE sequence = ?1",
            [commit as i64],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if payload_checksum(&bytes).bytes().as_slice() != checksum.as_slice() {
            return Err(SqliteJournalError::PayloadChecksum { sequence: commit });
        }
        let Some((namespace, payload)) = decode_namespace(&bytes)? else {
            return Err(SqliteJournalError::Integrity {
                detail: "journal coordinate has no namespace".into(),
            });
        };
        if self.namespace.as_deref() != Some(namespace) {
            return Err(SqliteJournalError::Integrity {
                detail: "journal coordinate belongs to another namespace".into(),
            });
        }
        Ok((SqliteCommitSequence(commit), payload.to_vec()))
    }

    /// Observe SQLite's physical commit callback, including grouped appends.
    #[cfg(feature = "test-support")]
    pub fn observe_commits(&self, mut observe: impl FnMut() + Send + 'static) {
        self.connection.commit_hook(Some(move || {
            observe();
            false
        }));
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    #[must_use]
    pub const fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    pub(crate) const fn poison(&mut self) {
        self.poisoned = true;
    }

    pub fn commit(&mut self, payload: &[u8]) -> Result<SqliteCommitReceipt, SqliteJournalError> {
        let group = self.commit_group(&[payload])?;
        let mut receipts = group.into_receipts().into_vec();
        Ok(receipts
            .pop()
            .expect("one-payload group returns exactly one receipt"))
    }

    pub fn commit_group<P>(
        &mut self,
        payloads: &[P],
    ) -> Result<SqliteCommitGroupReceipt, SqliteJournalError>
    where
        P: AsRef<[u8]>,
    {
        if self.poisoned {
            return Err(SqliteJournalError::Poisoned);
        }
        if payloads.is_empty() {
            return Err(SqliteJournalError::EmptyCommitGroup);
        }
        let logical_checksums = payloads
            .iter()
            .map(|payload| payload_checksum(payload.as_ref()))
            .collect::<Vec<_>>();
        let retained_after = match self.retained_bytes.get() {
            Some(before) => Some(
                payloads
                    .iter()
                    .try_fold(before, |total, payload| {
                        total.checked_add(payload.as_ref().len() as u64)
                    })
                    .ok_or_else(|| SqliteJournalError::Integrity {
                        detail: "retained namespace bytes exceed the byte domain".to_owned(),
                    })?,
            ),
            None => None,
        };
        let payloads = payloads
            .iter()
            .map(|payload| match &self.namespace {
                Some(namespace) => namespace_payload(Some(namespace), payload.as_ref()),
                None => Ok(payload.as_ref().to_vec()),
            })
            .collect::<Result<Vec<_>, SqliteJournalError>>()?;
        let checksums = payloads
            .iter()
            .map(|payload| payload_checksum(payload))
            .collect::<Vec<_>>();
        let result = (|| {
            let transaction = begin(&mut self.connection, TransactionBehavior::Immediate)?;
            let last: i64 = transaction.query_row(
                "SELECT last_sequence FROM circular_meta WHERE singleton = 1",
                [],
                |row| row.get(0),
            )?;
            let last = sqlite_sequence(last)?;
            let count =
                u64::try_from(payloads.len()).map_err(|_| SqliteJournalError::SequenceExhausted)?;
            let through = last
                .checked_add(count)
                .filter(|value| *value <= MAX_SQLITE_SEQUENCE)
                .ok_or(SqliteJournalError::SequenceExhausted)?;
            let mut receipts = Vec::with_capacity(payloads.len());
            for (index, (payload, checksum)) in payloads.iter().zip(checksums.iter()).enumerate() {
                let offset =
                    u64::try_from(index + 1).map_err(|_| SqliteJournalError::SequenceExhausted)?;
                let sequence = last
                    .checked_add(offset)
                    .ok_or(SqliteJournalError::SequenceExhausted)?;
                let sequence_sql =
                    i64::try_from(sequence).map_err(|_| SqliteJournalError::SequenceExhausted)?;
                transaction.execute(
                    "INSERT INTO circular_commits(sequence, payload, payload_sha256) VALUES (?1, ?2, ?3)",
                    params![sequence_sql, payload, checksum.bytes().as_slice()],
                )?;
                receipts.push(SqliteCommitReceipt {
                    sequence: SqliteCommitSequence(sequence),
                    checksum: *checksum,
                });
            }
            let through_sql =
                i64::try_from(through).map_err(|_| SqliteJournalError::SequenceExhausted)?;
            let changed = transaction.execute(
                "UPDATE circular_meta SET last_sequence = ?1 WHERE singleton = 1 AND last_sequence = ?2",
                params![through_sql, i64::try_from(last).expect("SQLite sequence is in range")],
            )?;
            if changed != 1 {
                return Err(SqliteJournalError::Integrity {
                    detail: "metadata compare-and-set did not update exactly one row".into(),
                });
            }
            transaction.commit()?;
            validate_group_receipts(last, &checksums, &receipts)?;
            Ok(SqliteCommitGroupReceipt {
                receipts: receipts.into_boxed_slice(),
            })
        })();

        if !matches!(result, Ok(_) | Err(SqliteJournalError::Contended)) {
            self.poisoned = true;
        }
        result.map(|mut group| {
            if self.namespace.is_some() {
                self.retained_bytes.set(retained_after);
                for (receipt, checksum) in group.receipts.iter_mut().zip(logical_checksums) {
                    receipt.checksum = checksum;
                }
            }
            group
        })
    }

    /// Build a payload from the exact prefix protected by the same write lock
    /// as its append. The callback sees this namespace's entries and the whole
    /// physical file's through, even when another namespace wrote most recently.
    pub fn commit_from_snapshot(
        &mut self,
        build: impl FnOnce(&SqliteJournalSnapshot) -> Result<Vec<u8>, SqliteJournalError>,
    ) -> Result<SqliteCommitReceipt, SqliteJournalError> {
        self.commit_from_snapshot_after(0, build)
    }

    pub fn commit_from_snapshot_after(
        &mut self,
        after: u64,
        build: impl FnOnce(&SqliteJournalSnapshot) -> Result<Vec<u8>, SqliteJournalError>,
    ) -> Result<SqliteCommitReceipt, SqliteJournalError> {
        if self.poisoned {
            return Err(SqliteJournalError::Poisoned);
        }
        let mut build = Some(build);
        let result = retry_while_contended(|| {
            let transaction = begin(&mut self.connection, TransactionBehavior::Immediate)?;
            let prefix = self.namespace.as_deref().map(namespace_envelope_prefix);
            let full = read_snapshot_after_in(&transaction, after, prefix.as_deref())?;
            let last = full.through().map_or(after, SqliteCommitSequence::get);
            let snapshot = match &self.namespace {
                Some(namespace) => namespace_snapshot(full, namespace)?,
                None => full,
            };
            let logical = build.take().ok_or_else(|| SqliteJournalError::Integrity {
                detail: "derived commit payload was already built for this attempt".to_owned(),
            })?(&snapshot)?;
            let logical_checksum = payload_checksum(&logical);
            let retained_after = match self.retained_bytes.get() {
                Some(before) => {
                    Some(before.checked_add(logical.len() as u64).ok_or_else(|| {
                        SqliteJournalError::Integrity {
                            detail: "retained namespace bytes exceed the byte domain".to_owned(),
                        }
                    })?)
                }
                None => None,
            };
            let payload = match &self.namespace {
                Some(namespace) => namespace_payload(Some(namespace), &logical)?,
                None => logical,
            };
            let sequence = last
                .checked_add(1)
                .filter(|n| *n <= MAX_SQLITE_SEQUENCE)
                .ok_or(SqliteJournalError::SequenceExhausted)?;
            let checksum = payload_checksum(&payload);
            transaction.execute(
                "INSERT INTO circular_commits(sequence, payload, payload_sha256) VALUES (?1, ?2, ?3)",
                params![sequence as i64, payload, checksum.bytes().as_slice()],
            )?;
            let changed = transaction.execute(
                "UPDATE circular_meta SET last_sequence = ?1 WHERE singleton = 1 AND last_sequence = ?2",
                params![sequence as i64, last as i64],
            )?;
            if changed != 1 {
                return Err(SqliteJournalError::Integrity {
                    detail: "derived commit metadata compare-and-set failed".into(),
                });
            }
            transaction.commit()?;
            if self.namespace.is_some() {
                self.retained_bytes.set(retained_after);
            }
            Ok(SqliteCommitReceipt {
                sequence: SqliteCommitSequence(sequence),
                checksum: logical_checksum,
            })
        });
        if !matches!(result, Ok(_) | Err(SqliteJournalError::Contended)) {
            self.poisoned = true;
        }
        result
    }

    pub fn snapshot(&mut self) -> Result<SqliteJournalSnapshot, SqliteJournalError> {
        if self.poisoned {
            return Err(SqliteJournalError::Poisoned);
        }
        let prefix = self.namespace.as_deref().map(namespace_envelope_prefix);
        let result = retry_while_contended(|| {
            let transaction = begin(&mut self.connection, TransactionBehavior::Deferred)?;
            let snapshot = read_snapshot_after_in(&transaction, 0, prefix.as_deref())?;
            transaction.commit()?;
            match &self.namespace {
                Some(namespace) => namespace_snapshot(snapshot, namespace),
                None => Ok(snapshot),
            }
        });
        if !matches!(result, Ok(_) | Err(SqliteJournalError::Contended)) {
            self.poisoned = true;
        }
        result
    }

    pub fn snapshot_since(
        &mut self,
        after: u64,
    ) -> Result<SqliteJournalSnapshot, SqliteJournalError> {
        if self.poisoned {
            return Err(SqliteJournalError::Poisoned);
        }
        let prefix = self.namespace.as_deref().map(namespace_envelope_prefix);
        let result = retry_while_contended(|| {
            let transaction = begin(&mut self.connection, TransactionBehavior::Deferred)?;
            let snapshot = read_snapshot_after_in(&transaction, after, prefix.as_deref())?;
            transaction.commit()?;
            match &self.namespace {
                Some(namespace) => namespace_snapshot(snapshot, namespace),
                None => Ok(snapshot),
            }
        });
        if !matches!(result, Ok(_) | Err(SqliteJournalError::Contended)) {
            self.poisoned = true;
        }
        result
    }

    fn configure_durable_connection(&mut self) -> Result<(), SqliteJournalError> {
        let mode: String = self
            .connection
            .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(SqliteJournalError::Integrity {
                detail: format!("journal_mode is {mode}, expected WAL"),
            });
        }
        self.connection.execute_batch(
            "PRAGMA synchronous = FULL;
             PRAGMA fullfsync = ON;
             PRAGMA checkpoint_fullfsync = ON;
             PRAGMA foreign_keys = ON;
             PRAGMA trusted_schema = OFF;",
        )?;
        let synchronous: i64 = self
            .connection
            .query_row("PRAGMA synchronous", [], |row| row.get(0))?;
        if synchronous != 2 {
            return Err(SqliteJournalError::Integrity {
                detail: format!("synchronous is {synchronous}, expected FULL(2)"),
            });
        }
        Ok(())
    }

    fn validate_fixed_schema(&self) -> Result<(), SqliteJournalError> {
        self.validate_schema_identity()
    }

    fn validate_schema_identity(&self) -> Result<(), SqliteJournalError> {
        let application_id: i32 =
            self.connection
                .query_row("PRAGMA application_id", [], |row| row.get(0))?;
        if application_id != APPLICATION_ID {
            return Err(SqliteJournalError::WrongApplicationId {
                found: application_id,
            });
        }
        let user_version: u32 = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if user_version != SQLITE_FIXED_SCHEMA_VERSION {
            return Err(SqliteJournalError::UnsupportedSchema {
                found: user_version,
                supported: SQLITE_FIXED_SCHEMA_VERSION,
            });
        }

        let mut statement = self.connection.prepare(
            "SELECT type || ':' || name FROM sqlite_schema
             WHERE name NOT LIKE 'sqlite_%'
             ORDER BY type, name",
        )?;
        let objects = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        let expected = ["table:circular_commits", "table:circular_meta"];
        if objects.iter().map(String::as_str).collect::<Vec<_>>() != expected {
            return Err(SqliteJournalError::SchemaObjects {
                found: objects.into_boxed_slice(),
            });
        }

        let mut metadata = self.connection.prepare(
            "SELECT schema_version, last_sequence FROM circular_meta WHERE singleton = 1",
        )?;
        let rows = metadata
            .query_map([], |row| Ok((row.get::<_, u32>(0)?, row.get::<_, i64>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        if rows.len() != 1 {
            return Err(SqliteJournalError::MetadataRows { found: rows.len() });
        }
        let (schema_version, _) = rows[0];
        if schema_version != SQLITE_FIXED_SCHEMA_VERSION {
            return Err(SqliteJournalError::UnsupportedSchema {
                found: schema_version,
                supported: SQLITE_FIXED_SCHEMA_VERSION,
            });
        }
        Ok(())
    }
}

const NAMESPACE_MAGIC: &[u8] = b"CIRC-NS";
/// Journal envelope version. Unpublished, so it stays 1 and a format change
/// edits it in place. Any other value is refused; no rewrite or migration is
/// performed on open.
pub const JOURNAL_FORMAT_VERSION: u8 = 1;

fn namespace_payload(
    namespace: Option<&str>,
    payload: &[u8],
) -> Result<Vec<u8>, SqliteJournalError> {
    let name = namespace.unwrap_or("").as_bytes();
    let length = u32::try_from(name.len()).map_err(|_| SqliteJournalError::Integrity {
        detail: "journal namespace exceeds u32".to_owned(),
    })?;
    if namespace == Some("") {
        return Err(SqliteJournalError::Integrity {
            detail: "empty journal namespace".to_owned(),
        });
    }
    let mut encoded = NAMESPACE_MAGIC.to_vec();
    encoded.push(JOURNAL_FORMAT_VERSION);
    encoded.extend_from_slice(&length.to_be_bytes());
    encoded.extend_from_slice(name);
    encoded.extend_from_slice(payload);
    Ok(encoded)
}

fn namespace_envelope_prefix(namespace: &str) -> Vec<u8> {
    let name = namespace.as_bytes();
    let mut prefix = NAMESPACE_MAGIC.to_vec();
    prefix.push(JOURNAL_FORMAT_VERSION);
    prefix.extend_from_slice(&(name.len() as u32).to_be_bytes());
    prefix.extend_from_slice(name);
    prefix
}

fn decode_namespace(payload: &[u8]) -> Result<Option<(&str, &[u8])>, SqliteJournalError> {
    let invalid = || SqliteJournalError::Integrity {
        detail: "invalid journal namespace envelope".to_owned(),
    };
    let unsupported = |found| SqliteJournalError::UnsupportedJournalFormat { entry: None, found };
    let rest = payload
        .strip_prefix(NAMESPACE_MAGIC)
        .ok_or_else(|| unsupported(None))?;
    let (&version, rest) = rest.split_first().ok_or_else(|| unsupported(None))?;
    if version != JOURNAL_FORMAT_VERSION {
        return Err(unsupported(Some(version)));
    }
    let length = rest.get(..4).ok_or_else(invalid)?;
    let length = u32::from_be_bytes(length.try_into().map_err(|_| invalid())?) as usize;
    let rest = &rest[4..];
    let name = rest.get(..length).ok_or_else(invalid)?;
    let payload = &rest[length..];
    if name.is_empty() {
        return if payload.is_empty() {
            Ok(None)
        } else {
            Err(invalid())
        };
    }
    let namespace = std::str::from_utf8(name).map_err(|_| invalid())?;
    Ok(Some((namespace, payload)))
}

fn namespace_snapshot(
    snapshot: SqliteJournalSnapshot,
    namespace: &str,
) -> Result<SqliteJournalSnapshot, SqliteJournalError> {
    let mut entries = Vec::new();
    for entry in snapshot.entries() {
        if let Some((found, payload)) =
            decode_namespace(entry.payload()).map_err(|error| match error {
                SqliteJournalError::UnsupportedJournalFormat { found, .. } => {
                    SqliteJournalError::UnsupportedJournalFormat {
                        entry: Some(entry.sequence().get()),
                        found,
                    }
                }
                other => other,
            })?
            && found == namespace
        {
            entries.push(SqliteJournalEntry {
                sequence: entry.sequence(),
                checksum: payload_checksum(payload),
                payload: payload.to_vec().into_boxed_slice(),
            });
        }
    }
    Ok(SqliteJournalSnapshot {
        through: snapshot.through(),
        entries: entries.into_boxed_slice(),
    })
}

fn create_owner_only_file(path: &Path) -> Result<(), SqliteJournalError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    match options.open(path) {
        Ok(file) => drop(file),
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
            return Err(SqliteJournalError::AlreadyExists(path.to_path_buf()));
        }
        Err(source) => {
            return Err(SqliteJournalError::Io {
                operation: "create SQLite store",
                source,
            });
        }
    }
    validate_existing_file(path)
}

fn validate_existing_file(path: &Path) -> Result<(), SqliteJournalError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Err(SqliteJournalError::Missing(path.to_path_buf()));
        }
        Err(source) => {
            return Err(SqliteJournalError::Io {
                operation: "inspect SQLite store",
                source,
            });
        }
    };
    if !metadata.file_type().is_file() {
        return Err(SqliteJournalError::NotRegularFile(path.to_path_buf()));
    }
    #[cfg(unix)]
    {
        let mode = metadata.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(SqliteJournalError::PermissionsTooBroad {
                path: path.to_path_buf(),
                mode,
            });
        }
    }
    Ok(())
}

fn open_read_write(path: &Path) -> Result<Connection, SqliteJournalError> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(SqliteJournalError::Database)
}

fn open_read_only(path: &Path) -> Result<Connection, SqliteJournalError> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(SqliteJournalError::Database)
}

fn next_commit_opportunity() -> Duration {
    crate::ACCEPTED_GROUP_COMMIT.candidate().max_wait()
}

fn is_lock_contention(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error {
                code: rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked,
                ..
            },
            _,
        )
    )
}

fn begin<'connection>(
    connection: &'connection mut Connection,
    behavior: TransactionBehavior,
) -> Result<rusqlite::Transaction<'connection>, SqliteJournalError> {
    connection
        .transaction_with_behavior(behavior)
        .map_err(|error| {
            if is_lock_contention(&error) {
                SqliteJournalError::Contended
            } else {
                SqliteJournalError::Database(error)
            }
        })
}

fn retry_while_contended<T>(
    mut attempt: impl FnMut() -> Result<T, SqliteJournalError>,
) -> Result<T, SqliteJournalError> {
    loop {
        match attempt() {
            Err(SqliteJournalError::Contended) => {
                std::thread::sleep(next_commit_opportunity());
            }
            settled => return settled,
        }
    }
}

fn sqlite_sequence(value: i64) -> Result<u64, SqliteJournalError> {
    u64::try_from(value).map_err(|_| SqliteJournalError::SequenceOutOfRange(value))
}

fn payload_checksum(payload: &[u8]) -> SqlitePayloadChecksum {
    SqlitePayloadChecksum(
        ring::digest::digest(&ring::digest::SHA256, payload)
            .as_ref()
            .try_into()
            .expect("SHA-256 has 32 bytes"),
    )
}

fn validate_group_receipts(
    previous_sequence: u64,
    expected_checksums: &[SqlitePayloadChecksum],
    receipts: &[SqliteCommitReceipt],
) -> Result<(), SqliteJournalError> {
    if receipts.len() != expected_checksums.len() {
        return Err(SqliteJournalError::ReceiptMismatch {
            detail: format!(
                "expected {} receipts, found {}",
                expected_checksums.len(),
                receipts.len()
            ),
        });
    }
    for (index, (expected_checksum, receipt)) in
        expected_checksums.iter().zip(receipts.iter()).enumerate()
    {
        let offset = u64::try_from(index + 1).map_err(|_| SqliteJournalError::ReceiptMismatch {
            detail: "receipt index is outside the SQLite sequence domain".to_owned(),
        })?;
        let expected_sequence = previous_sequence.checked_add(offset).ok_or_else(|| {
            SqliteJournalError::ReceiptMismatch {
                detail: "receipt sequence arithmetic overflowed".to_owned(),
            }
        })?;
        if receipt.sequence().get() != expected_sequence {
            return Err(SqliteJournalError::ReceiptMismatch {
                detail: format!(
                    "at index {index}, expected sequence {expected_sequence}, found {}",
                    receipt.sequence().get()
                ),
            });
        }
        if receipt.checksum() != *expected_checksum {
            return Err(SqliteJournalError::ReceiptMismatch {
                detail: format!("at index {index}, checksum differs from the submitted payload"),
            });
        }
    }
    Ok(())
}

fn read_snapshot_after(
    connection: &Connection,
    after: u64,
) -> Result<SqliteJournalSnapshot, SqliteJournalError> {
    read_snapshot_after_in(connection, after, None)
}

fn read_snapshot_after_in(
    connection: &Connection,
    after: u64,
    namespace_prefix: Option<&[u8]>,
) -> Result<SqliteJournalSnapshot, SqliteJournalError> {
    let metadata = connection
        .prepare("SELECT last_sequence FROM circular_meta WHERE singleton = 1")?
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    if metadata.len() != 1 {
        return Err(SqliteJournalError::MetadataRows {
            found: metadata.len(),
        });
    }
    let declared = sqlite_sequence(metadata[0])?;

    let mut selected = match namespace_prefix {
        Some(_) => connection.prepare(
            "SELECT sequence, CASE WHEN substr(payload, 1, ?2) = ?3 OR length(payload) < ?2 \
             OR substr(payload, 1, ?5) <> ?4 THEN payload END, payload_sha256 \
             FROM circular_commits WHERE sequence > ?1 ORDER BY sequence",
        )?,
        None => connection.prepare(
            "SELECT sequence, payload, payload_sha256 FROM circular_commits WHERE sequence > ?1 ORDER BY sequence",
        )?,
    };
    let read = |row: &rusqlite::Row<'_>| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<Vec<u8>>>(1)?,
            row.get::<_, Vec<u8>>(2)?,
        ))
    };
    let mut current_format = NAMESPACE_MAGIC.to_vec();
    current_format.push(JOURNAL_FORMAT_VERSION);
    let rows = match namespace_prefix {
        Some(prefix) => {
            let width = i64::try_from(prefix.len()).map_err(|_| SqliteJournalError::Integrity {
                detail: "journal namespace envelope exceeds the query domain".to_owned(),
            })?;
            let format_width = i64::try_from(current_format.len()).expect("fixed envelope head");
            selected.query_map(
                params![after, width, prefix, current_format, format_width],
                read,
            )?
        }
        None => selected.query_map(params![after], read)?,
    };
    let mut entries = Vec::new();
    let mut expected = after
        .checked_add(1)
        .ok_or(SqliteJournalError::SequenceExhausted)?;
    for row in rows {
        let (sequence, payload, stored_checksum) = row?;
        let sequence = sqlite_sequence(sequence)?;
        if sequence != expected {
            return Err(SqliteJournalError::SequenceGap {
                expected,
                found: sequence,
            });
        }
        expected = expected
            .checked_add(1)
            .ok_or(SqliteJournalError::SequenceExhausted)?;
        let Some(payload) = payload else {
            continue;
        };
        #[cfg(feature = "test-support")]
        let measured = std::time::Instant::now();
        let checksum = payload_checksum(&payload);
        #[cfg(feature = "test-support")]
        crate::read_measurement::sha(measured, payload.len());
        if stored_checksum.as_slice() != checksum.bytes() {
            return Err(SqliteJournalError::PayloadChecksum { sequence });
        }
        entries.push(SqliteJournalEntry {
            sequence: SqliteCommitSequence(sequence),
            checksum,
            payload: payload.into_boxed_slice(),
        });
    }
    let found = expected - 1;
    if declared != found {
        return Err(SqliteJournalError::MetadataSequenceMismatch { declared, found });
    }
    Ok(SqliteJournalSnapshot {
        through: (declared != 0).then_some(SqliteCommitSequence(declared)),
        entries: entries.into_boxed_slice(),
    })
}

pub trait SqliteTransactionCodec<S: TransactionSchema> {
    type Error: Debug;
    type ReadContext: Default;

    fn encode(&self, transaction: &StoreTransaction<S>) -> Result<Vec<u8>, Self::Error>;
    fn decode(&self, bytes: &[u8]) -> Result<StoreTransaction<S>, Self::Error>;

    fn decode_in(
        &self,
        _context: &mut Self::ReadContext,
        _sequence: u64,
        bytes: &[u8],
    ) -> Result<StoreTransaction<S>, Self::Error> {
        self.decode(bytes)
    }
    fn checks_canonical_on_decode(&self) -> bool {
        false
    }

    fn prepare(
        &self,
        transactions: &[StoreTransaction<S>],
    ) -> Result<PreparedSqliteBatch, (usize, Self::Error)> {
        let mut payloads = Vec::with_capacity(transactions.len());
        for (i, transaction) in transactions.iter().enumerate() {
            payloads.push(self.encode(transaction).map_err(|error| (i, error))?);
        }
        Ok(PreparedSqliteBatch {
            payloads,
            committed: None,
        })
    }
}

/// Candidate encoder material is installed only after SQLite and receipt
/// validation succeed. Dropping this value after any failure changes nothing.
pub struct PreparedSqliteBatch {
    pub(crate) payloads: Vec<Vec<u8>>,
    pub(crate) committed: Option<Box<dyn FnOnce(&[SqliteCommitReceipt])>>,
}

#[derive(Debug)]
pub enum SqliteTransactionOpenError<S: TransactionSchema, E> {
    Journal(SqliteJournalError),
    Decode {
        sequence: SqliteCommitSequence,
        source: E,
    },
    NonCanonical {
        sequence: SqliteCommitSequence,
    },
    ReplayRejected {
        sequence: SqliteCommitSequence,
        operation: usize,
        reason: StoreTransactionReject<S>,
    },
}

impl<S: TransactionSchema, E> From<SqliteJournalError> for SqliteTransactionOpenError<S, E> {
    fn from(source: SqliteJournalError) -> Self {
        Self::Journal(source)
    }
}

#[derive(Debug)]
pub enum SqliteCommitError<E> {
    Codec(E),
    Journal(SqliteJournalError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteTransactionGroupReceipt<S: TransactionSchema> {
    logical: Box<[StoreTransactionReceipt<S>]>,
    physical: SqliteCommitGroupReceipt,
}

impl<S: TransactionSchema> SqliteTransactionGroupReceipt<S> {
    #[must_use]
    pub const fn logical(&self) -> &[StoreTransactionReceipt<S>] {
        &self.logical
    }

    #[must_use]
    pub const fn physical(&self) -> &SqliteCommitGroupReceipt {
        &self.physical
    }
}

#[derive(Debug)]
pub enum SqliteTransactionGroupFailureReason<S: TransactionSchema, E> {
    Empty,
    Rejected {
        transaction: usize,
        operation: usize,
        reason: StoreTransactionReject<S>,
    },
    Codec {
        transaction: usize,
        source: E,
    },
    Journal(SqliteJournalError),
    Contended,
}

impl<S: TransactionSchema, E> From<GroupCommitFailure<S, E, SqliteJournalError>>
    for SqliteTransactionGroupFailureReason<S, E>
{
    fn from(failure: GroupCommitFailure<S, E, SqliteJournalError>) -> Self {
        match failure {
            GroupCommitFailure::Rejected {
                transaction,
                operation,
                reason,
            } => Self::Rejected {
                transaction,
                operation,
                reason,
            },
            GroupCommitFailure::Codec {
                transaction,
                source,
            } => Self::Codec {
                transaction,
                source,
            },
            GroupCommitFailure::Backend(source) => Self::Journal(source),
            GroupCommitFailure::Contended => Self::Contended,
        }
    }
}

#[derive(Debug)]
pub struct SqliteTransactionGroupFailure<S: TransactionSchema, E> {
    transactions: Box<[StoreTransaction<S>]>,
    reason: SqliteTransactionGroupFailureReason<S, E>,
}

impl<S: TransactionSchema, E> SqliteTransactionGroupFailure<S, E> {
    #[must_use]
    pub const fn transactions(&self) -> &[StoreTransaction<S>] {
        &self.transactions
    }

    #[must_use]
    pub const fn reason(&self) -> &SqliteTransactionGroupFailureReason<S, E> {
        &self.reason
    }

    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        Box<[StoreTransaction<S>]>,
        SqliteTransactionGroupFailureReason<S, E>,
    ) {
        (self.transactions, self.reason)
    }
}

pub struct RecoveringSqliteTransactionStore<S, C>
where
    S: TransactionSchema,
    C: SqliteTransactionCodec<S>,
{
    journal: SqliteJournal,
    model: RecoveringTransactionModel<S>,
    codec: C,
}

pub struct RecoveredSqliteTransactionStore<S, C>
where
    S: TransactionSchema,
    C: SqliteTransactionCodec<S>,
{
    journal: SqliteJournal,
    model: RecoveredTransactionModel<S>,
    codec: C,
}

impl<S, C> RecoveringSqliteTransactionStore<S, C>
where
    S: TransactionSchema,
    C: SqliteTransactionCodec<S>,
{
    pub fn create(
        path: impl AsRef<Path>,
        codec: C,
    ) -> Result<Self, SqliteTransactionOpenError<S, C::Error>> {
        let journal = SqliteJournal::create(path)?;
        Ok(Self {
            journal,
            model: RecoveredTransactionModel::empty_for_journal_replay().reopen(),
            codec,
        })
    }

    pub fn open(
        path: impl AsRef<Path>,
        codec: C,
    ) -> Result<Self, SqliteTransactionOpenError<S, C::Error>> {
        Self::from_journal(SqliteJournal::open(path)?, codec)
    }

    pub fn from_journal(
        mut journal: SqliteJournal,
        codec: C,
    ) -> Result<Self, SqliteTransactionOpenError<S, C::Error>> {
        let snapshot = journal.snapshot()?;
        let mut model = RecoveredTransactionModel::empty_for_journal_replay();
        let mut read_context = C::ReadContext::default();
        for entry in snapshot.entries() {
            let transaction = codec
                .decode_in(&mut read_context, entry.sequence().get(), entry.payload())
                .map_err(|source| SqliteTransactionOpenError::Decode {
                    sequence: entry.sequence(),
                    source,
                })?;
            if !codec.checks_canonical_on_decode() {
                let canonical = codec.encode(&transaction).map_err(|source| {
                    SqliteTransactionOpenError::Decode {
                        sequence: entry.sequence(),
                        source,
                    }
                })?;
                if canonical.as_slice() != entry.payload() {
                    return Err(SqliteTransactionOpenError::NonCanonical {
                        sequence: entry.sequence(),
                    });
                }
            }
            model = match model.fold_committed(transaction) {
                Ok((model, _)) => model,
                Err(failure) => {
                    let (_, reason) = failure.into_parts();
                    match reason {
                        StoreTransactionFailureReason::Rejected { operation, reason } => {
                            return Err(SqliteTransactionOpenError::ReplayRejected {
                                sequence: entry.sequence(),
                                operation,
                                reason,
                            });
                        }
                        StoreTransactionFailureReason::Backend(never) => match never {},
                    }
                }
            };
        }
        Ok(Self {
            journal,
            model: model.reopen(),
            codec,
        })
    }

    #[must_use]
    pub fn from_recovered(
        journal: SqliteJournal,
        codec: C,
        model: crate::RecoveredTransactionModel<S>,
    ) -> Self {
        Self {
            journal,
            model: model.reopen(),
            codec,
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.journal.path()
    }

    #[must_use]
    pub const fn is_poisoned(&self) -> bool {
        self.journal.is_poisoned()
    }

    pub fn journal_snapshot(&mut self) -> Result<SqliteJournalSnapshot, SqliteJournalError> {
        self.journal.snapshot()
    }

    pub fn journal_snapshot_since(
        &mut self,
        after: u64,
    ) -> Result<SqliteJournalSnapshot, SqliteJournalError> {
        self.journal.snapshot_since(after)
    }

    pub fn recover<E, F>(
        self,
        owners: &CheckpointOwners<S>,
        mut terminal: F,
    ) -> Result<SqliteRecoveryResult<S, C>, SqliteRecoveryError<S, C, E>>
    where
        F: FnMut(&S::EffectId, &S::Outbox) -> Result<RecoveryTerminal<S>, E>,
    {
        self.recover_outboxes(owners, |key, request| terminal(key, request).map(Some))
    }

    /// Restore custody while letting its owner retry unresolved Submitted rows.
    /// A retained row stays durable and unchanged until a dispatch receipt is acquired.
    pub fn recover_outboxes<E, F>(
        self,
        owners: &CheckpointOwners<S>,
        mut terminal: F,
    ) -> Result<SqliteRecoveryResult<S, C>, SqliteRecoveryError<S, C, E>>
    where
        F: FnMut(&S::EffectId, &S::Outbox) -> Result<Option<RecoveryTerminal<S>>, E>,
    {
        let Self {
            journal,
            model,
            codec,
        } = self;
        let original = model.clone();
        let mut settlements = Vec::new();
        let recovered = match model.recover_outboxes(owners, |effect, outbox| {
            let settlement = terminal(effect, outbox)?;
            if let Some(settlement) = &settlement {
                settlements.push((effect.clone(), settlement.clone()));
            }
            Ok(settlement)
        }) {
            Ok(recovered) => recovered,
            Err(error) => {
                let (model, failure) = error.into_parts();
                return Err(SqliteRecoveryError {
                    store: Box::new(Self {
                        journal,
                        model,
                        codec,
                    }),
                    failure: SqliteRecoveryFailure::Model(failure),
                });
            }
        };
        let (model, plan) = recovered.into_parts();
        let mut store = RecoveredSqliteTransactionStore {
            journal,
            model,
            codec,
        };

        if !settlements.is_empty() {
            let operations = settlements
                .into_iter()
                .map(|(effect, settlement)| {
                    let (outcome, observation) = settlement.into_parts();
                    StoreTransactionOp::SettleOutbox {
                        effect,
                        outcome,
                        observation,
                    }
                })
                .collect();
            let transaction = StoreTransaction::try_new(operations)
                .expect("nonempty submitted-outbox settlement set");
            if let Err(failure) = store.commit(transaction) {
                let (_, failure) = failure.into_parts();
                let RecoveredSqliteTransactionStore { journal, codec, .. } = store;
                return Err(SqliteRecoveryError {
                    store: Box::new(Self {
                        journal,
                        model: original,
                        codec,
                    }),
                    failure: SqliteRecoveryFailure::Settlement(failure),
                });
            }
        }

        Ok(SqliteRecoveryResult { store, plan })
    }
}

impl<S, C> RecoveredSqliteTransactionStore<S, C>
where
    S: TransactionSchema,
    C: SqliteTransactionCodec<S>,
{
    #[must_use]
    pub fn model(&self) -> &RecoveredTransactionModel<S> {
        &self.model
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        self.journal.path()
    }

    #[must_use]
    pub const fn is_poisoned(&self) -> bool {
        self.journal.is_poisoned()
    }

    pub fn journal_snapshot(&mut self) -> Result<SqliteJournalSnapshot, SqliteJournalError> {
        self.journal.snapshot()
    }

    pub fn journal_snapshot_since(
        &mut self,
        after: u64,
    ) -> Result<SqliteJournalSnapshot, SqliteJournalError> {
        self.journal.snapshot_since(after)
    }

    pub fn journal(&self) -> &SqliteJournal {
        &self.journal
    }

    /// Retained payload bytes of this store's namespace, carried at append.
    #[must_use]
    pub fn retained_namespace_bytes(&self) -> Option<u64> {
        self.journal.retained_namespace_bytes()
    }

    pub fn commit_group(
        &mut self,
        transactions: Vec<StoreTransaction<S>>,
    ) -> Result<SqliteTransactionGroupReceipt<S>, SqliteTransactionGroupFailure<S, C::Error>> {
        self.commit_group_via(transactions, |journal, payloads| {
            journal.commit_group(payloads)
        })
    }

    fn commit_group_via<F>(
        &mut self,
        transactions: Vec<StoreTransaction<S>>,
        persist: F,
    ) -> Result<SqliteTransactionGroupReceipt<S>, SqliteTransactionGroupFailure<S, C::Error>>
    where
        F: FnOnce(
            &mut SqliteJournal,
            &[Vec<u8>],
        ) -> Result<SqliteCommitGroupReceipt, SqliteJournalError>,
    {
        if transactions.is_empty() {
            return Err(SqliteTransactionGroupFailure {
                transactions: Box::new([]),
                reason: SqliteTransactionGroupFailureReason::Empty,
            });
        }

        match self.commit_group_pipeline(&transactions, persist) {
            Ok(receipt) => Ok(receipt),
            Err(reason) => Err(SqliteTransactionGroupFailure {
                transactions: transactions.into_boxed_slice(),
                reason: reason.into(),
            }),
        }
    }

    fn commit_group_pipeline<F>(
        &mut self,
        transactions: &[StoreTransaction<S>],
        persist: F,
    ) -> Result<SqliteTransactionGroupReceipt<S>, GroupCommitFailure<S, C::Error, SqliteJournalError>>
    where
        F: FnOnce(
            &mut SqliteJournal,
            &[Vec<u8>],
        ) -> Result<SqliteCommitGroupReceipt, SqliteJournalError>,
    {
        let mut undo = crate::transaction::CommitUndo::default();
        let mut logical = Vec::with_capacity(transactions.len());
        for (index, transaction) in transactions.iter().enumerate() {
            match self.model.fold_undoable(transaction, &mut undo) {
                Ok(receipt) => logical.push(receipt),
                Err((operation, reason)) => {
                    self.model.rewind(undo);
                    return Err(GroupCommitFailure::Rejected {
                        transaction: index,
                        operation,
                        reason,
                    });
                }
            }
        }

        let prepared = match self.codec.prepare(transactions) {
            Ok(prepared) => prepared,
            Err((index, source)) => {
                self.model.rewind(undo);
                return Err(GroupCommitFailure::Codec {
                    transaction: index,
                    source,
                });
            }
        };
        let encoded = &prepared.payloads;

        let physical = match persist(&mut self.journal, encoded) {
            Ok(receipt) => receipt,
            Err(SqliteJournalError::Contended) => {
                self.model.rewind(undo);
                return Err(GroupCommitFailure::Contended);
            }
            Err(source) => {
                self.model.rewind(undo);
                return Err(GroupCommitFailure::Backend(source));
            }
        };
        if let Err(source) = validate_encoded_group_receipts(encoded, &physical) {
            self.model.rewind(undo);
            self.journal.poison();
            return Err(GroupCommitFailure::Backend(source));
        }

        if let Some(committed) = prepared.committed {
            committed(physical.receipts());
        }

        Ok(SqliteTransactionGroupReceipt {
            logical: logical.into_boxed_slice(),
            physical,
        })
    }
}

fn validate_encoded_group_receipts(
    payloads: &[Vec<u8>],
    group: &SqliteCommitGroupReceipt,
) -> Result<(), SqliteJournalError> {
    let receipts = group.receipts();
    let Some(first) = receipts.first() else {
        return Err(SqliteJournalError::ReceiptMismatch {
            detail: "nonempty transaction group returned no physical receipts".to_owned(),
        });
    };
    let previous = first.sequence().get().checked_sub(1).ok_or_else(|| {
        SqliteJournalError::ReceiptMismatch {
            detail: "first physical receipt has invalid zero sequence".to_owned(),
        }
    })?;
    let checksums = payloads
        .iter()
        .map(|payload| payload_checksum(payload))
        .collect::<Vec<_>>();
    validate_group_receipts(previous, &checksums, receipts)
}

impl<S, C> GroupTransactionPort<S> for RecoveredSqliteTransactionStore<S, C>
where
    S: TransactionSchema,
    C: SqliteTransactionCodec<S>,
{
    type CodecError = C::Error;

    fn commit_group(
        &mut self,
        transactions: Vec<StoreTransaction<S>>,
    ) -> Result<
        Box<[crate::transaction::CommittedTransaction<S>]>,
        GroupCommitFailure<S, Self::CodecError, Self::BackendError>,
    > {
        match self.commit_group_pipeline(&transactions, |journal, payloads| {
            journal.commit_group(payloads)
        }) {
            Ok(receipt) => Ok(receipt
                .logical
                .into_vec()
                .into_iter()
                .zip(receipt.physical.receipts().iter())
                .map(|(logical, physical)| {
                    crate::transaction::CommittedTransaction::new(
                        logical,
                        physical.sequence().get(),
                    )
                })
                .collect()),
            Err(GroupCommitFailure::Rejected {
                transaction,
                operation,
                reason,
            }) => Err(GroupCommitFailure::Rejected {
                transaction,
                operation,
                reason,
            }),
            Err(GroupCommitFailure::Codec {
                transaction,
                source,
            }) => Err(GroupCommitFailure::Codec {
                transaction,
                source,
            }),
            Err(GroupCommitFailure::Backend(source)) => Err(GroupCommitFailure::Backend(
                SqliteCommitError::Journal(source),
            )),
            Err(GroupCommitFailure::Contended) => Err(GroupCommitFailure::Contended),
        }
    }
}

impl<S, C> AtomicStoreTransactionPort<S> for RecoveredSqliteTransactionStore<S, C>
where
    S: TransactionSchema,
    C: SqliteTransactionCodec<S>,
{
    type BackendError = SqliteCommitError<C::Error>;

    fn commit(
        &mut self,
        transaction: StoreTransaction<S>,
    ) -> Result<StoreTransactionReceipt<S>, StoreTransactionFailure<S, Self::BackendError>> {
        let original = transaction.clone();
        match GroupTransactionPort::commit_group(self, vec![transaction]) {
            Ok(receipts) => {
                let mut receipts = receipts.into_vec();
                let receipt = receipts.pop().expect("group receipt with one element");
                debug_assert!(receipts.is_empty());
                Ok(receipt.into_parts().0)
            }
            Err(GroupCommitFailure::Rejected {
                transaction,
                operation,
                reason,
            }) => {
                debug_assert_eq!(transaction, 0);
                Err(StoreTransactionFailure::rejected(
                    original, operation, reason,
                ))
            }
            Err(GroupCommitFailure::Codec {
                transaction,
                source,
            }) => {
                debug_assert_eq!(transaction, 0);
                Err(StoreTransactionFailure::backend(
                    original,
                    SqliteCommitError::Codec(source),
                ))
            }
            Err(GroupCommitFailure::Backend(source)) => {
                Err(StoreTransactionFailure::backend(original, source))
            }
            Err(GroupCommitFailure::Contended) => {
                std::thread::sleep(next_commit_opportunity());
                self.commit(original)
            }
        }
    }
}

impl<S, C> CrashDurableTransactionPort<S> for RecoveredSqliteTransactionStore<S, C>
where
    S: TransactionSchema,
    C: SqliteTransactionCodec<S>,
{
}

#[derive(Debug)]
pub enum SqliteRecoveryFailure<S: TransactionSchema, E, C> {
    Model(TransactionRecoveryFailure<S, E>),
    Settlement(StoreTransactionFailureReason<S, SqliteCommitError<C>>),
}

pub struct SqliteRecoveryError<S, C, E>
where
    S: TransactionSchema,
    C: SqliteTransactionCodec<S>,
{
    store: Box<RecoveringSqliteTransactionStore<S, C>>,
    failure: SqliteRecoveryFailure<S, E, C::Error>,
}

impl<S, C, E> SqliteRecoveryError<S, C, E>
where
    S: TransactionSchema,
    C: SqliteTransactionCodec<S>,
{
    #[must_use]
    pub const fn failure(&self) -> &SqliteRecoveryFailure<S, E, C::Error> {
        &self.failure
    }

    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        RecoveringSqliteTransactionStore<S, C>,
        SqliteRecoveryFailure<S, E, C::Error>,
    ) {
        (*self.store, self.failure)
    }
}

pub struct SqliteRecoveryResult<S, C>
where
    S: TransactionSchema,
    C: SqliteTransactionCodec<S>,
{
    store: RecoveredSqliteTransactionStore<S, C>,
    plan: TransactionRecoveryPlan<S>,
}

impl<S, C> SqliteRecoveryResult<S, C>
where
    S: TransactionSchema,
    C: SqliteTransactionCodec<S>,
{
    #[must_use]
    pub const fn store(&self) -> &RecoveredSqliteTransactionStore<S, C> {
        &self.store
    }

    #[must_use]
    pub const fn plan(&self) -> &TransactionRecoveryPlan<S> {
        &self.plan
    }

    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        RecoveredSqliteTransactionStore<S, C>,
        TransactionRecoveryPlan<S>,
    ) {
        (self.store, self.plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static CONTENTION_DIRECTORY_SERIAL: AtomicU64 = AtomicU64::new(1);

    use crate::{
        TransactionAppend, TransactionCheckpoint, TransactionObservation, TransactionOutbox,
        TransactionOutboxPhase,
    };
    use std::convert::Infallible;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[cfg(unix)]
    use std::os::unix::fs::DirBuilderExt;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct TestSchema;

    impl TransactionSchema for TestSchema {
        type WriteContext = ();
        fn record_digest(record: &Self::Record) -> crate::transaction::RecordDigest {
            crate::transaction::RecordDigest::of_bytes(&record.to_be_bytes())
        }

        fn observation_digest(observation: &Self::Observation) -> crate::transaction::RecordDigest {
            crate::transaction::RecordDigest::of_bytes(&observation.to_be_bytes())
        }

        type RecordKey = u64;
        type Record = u64;
        type EffectId = u64;
        type Outbox = u64;
        type ApprovalKey = u64;
        type Approval = u64;
        type ApprovalTicket = u64;
        type ActorId = u64;
        type Incarnation = u64;
        type CheckpointStamp = u64;
        type CheckpointState = u64;
        type Outcome = u64;
        type ObservationKey = u64;
        type Observation = u64;
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TestCodecError {
        Empty,
        Truncated,
        UnknownTag(u8),
        Trailing,
        Count,
        Forced,
        UnsupportedOperation,
    }

    #[derive(Clone, Copy, Debug)]
    struct TestCodec;

    impl SqliteTransactionCodec<TestSchema> for TestCodec {
        type ReadContext = ();
        type Error = TestCodecError;

        fn encode(
            &self,
            transaction: &StoreTransaction<TestSchema>,
        ) -> Result<Vec<u8>, Self::Error> {
            encode_test_transaction(transaction)
        }

        fn decode(&self, bytes: &[u8]) -> Result<StoreTransaction<TestSchema>, Self::Error> {
            decode_test_transaction(bytes)
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct FailingCodec;

    impl SqliteTransactionCodec<TestSchema> for FailingCodec {
        type ReadContext = ();
        type Error = TestCodecError;

        fn encode(
            &self,
            _transaction: &StoreTransaction<TestSchema>,
        ) -> Result<Vec<u8>, Self::Error> {
            Err(TestCodecError::Forced)
        }

        fn decode(&self, bytes: &[u8]) -> Result<StoreTransaction<TestSchema>, Self::Error> {
            decode_test_transaction(bytes)
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct FailingAtRecordCodec(u64);

    impl SqliteTransactionCodec<TestSchema> for FailingAtRecordCodec {
        type ReadContext = ();
        type Error = TestCodecError;

        fn encode(
            &self,
            transaction: &StoreTransaction<TestSchema>,
        ) -> Result<Vec<u8>, Self::Error> {
            if transaction.operations().iter().any(|operation| {
                matches!(operation, StoreTransactionOp::Append(append) if *append.key() == self.0)
            }) {
                Err(TestCodecError::Forced)
            } else {
                encode_test_transaction(transaction)
            }
        }

        fn decode(&self, bytes: &[u8]) -> Result<StoreTransaction<TestSchema>, Self::Error> {
            decode_test_transaction(bytes)
        }
    }

    #[derive(Clone, Copy, Debug)]
    struct RedundantPrefixCodec;

    impl SqliteTransactionCodec<TestSchema> for RedundantPrefixCodec {
        type ReadContext = ();
        type Error = TestCodecError;

        fn encode(
            &self,
            transaction: &StoreTransaction<TestSchema>,
        ) -> Result<Vec<u8>, Self::Error> {
            encode_test_transaction(transaction)
        }

        fn decode(&self, bytes: &[u8]) -> Result<StoreTransaction<TestSchema>, Self::Error> {
            let bytes = bytes.strip_prefix(&[0]).unwrap_or(bytes);
            decode_test_transaction(bytes)
        }
    }

    struct DecodeCursor<'a> {
        bytes: &'a [u8],
        at: usize,
    }

    impl<'a> DecodeCursor<'a> {
        fn new(bytes: &'a [u8]) -> Self {
            Self { bytes, at: 0 }
        }

        fn byte(&mut self) -> Result<u8, TestCodecError> {
            let byte = self
                .bytes
                .get(self.at)
                .copied()
                .ok_or(TestCodecError::Truncated)?;
            self.at += 1;
            Ok(byte)
        }

        fn u64(&mut self) -> Result<u64, TestCodecError> {
            let end = self.at.checked_add(8).ok_or(TestCodecError::Truncated)?;
            let bytes: [u8; 8] = self
                .bytes
                .get(self.at..end)
                .ok_or(TestCodecError::Truncated)?
                .try_into()
                .expect("the selected slice has exactly eight bytes");
            self.at = end;
            Ok(u64::from_be_bytes(bytes))
        }

        fn finished(&self) -> bool {
            self.at == self.bytes.len()
        }
    }

    fn push_u64(output: &mut Vec<u8>, value: u64) {
        output.extend_from_slice(&value.to_be_bytes());
    }

    fn push_observation(output: &mut Vec<u8>, observation: &TransactionObservation<TestSchema>) {
        push_u64(output, *observation.key());
        push_u64(output, *observation.value());
    }

    fn read_observation(
        cursor: &mut DecodeCursor<'_>,
    ) -> Result<TransactionObservation<TestSchema>, TestCodecError> {
        Ok(TransactionObservation::new(cursor.u64()?, cursor.u64()?))
    }

    fn encode_test_transaction(
        transaction: &StoreTransaction<TestSchema>,
    ) -> Result<Vec<u8>, TestCodecError> {
        let count =
            u64::try_from(transaction.operations().len()).map_err(|_| TestCodecError::Count)?;
        let mut output = Vec::new();
        push_u64(&mut output, count);
        for operation in transaction.operations() {
            match operation {
                StoreTransactionOp::Append(append) => {
                    output.push(1);
                    push_u64(&mut output, *append.key());
                    push_u64(&mut output, *append.record());
                }
                StoreTransactionOp::OpenOutbox { effect, request } => {
                    output.push(4);
                    push_u64(&mut output, *effect);
                    push_u64(&mut output, *request);
                }
                StoreTransactionOp::SubmitOutbox { effect } => {
                    output.push(5);
                    push_u64(&mut output, *effect);
                }
                StoreTransactionOp::AcquireOutboxDispatch { effect } => {
                    output.push(6);
                    push_u64(&mut output, *effect);
                }
                StoreTransactionOp::SettleOutbox {
                    effect,
                    outcome,
                    observation,
                } => {
                    output.push(7);
                    push_u64(&mut output, *effect);
                    push_u64(&mut output, *outcome);
                    push_observation(&mut output, observation);
                }
                StoreTransactionOp::OpenApproval { key, approval } => {
                    output.push(8);
                    push_u64(&mut output, *key);
                    push_u64(&mut output, *approval);
                }
                StoreTransactionOp::ApproveApproval { key, ticket } => {
                    output.push(9);
                    push_u64(&mut output, *key);
                    push_u64(&mut output, *ticket);
                }
                StoreTransactionOp::SettleApproval { key, observation } => {
                    output.push(10);
                    push_u64(&mut output, *key);
                    push_observation(&mut output, observation);
                }
                StoreTransactionOp::ReplaceCheckpoint(checkpoint) => {
                    output.push(11);
                    push_u64(&mut output, *checkpoint.actor());
                    push_u64(&mut output, *checkpoint.owner());
                    push_u64(&mut output, *checkpoint.at());
                    push_u64(&mut output, *checkpoint.state());
                    let pending = u64::try_from(checkpoint.pending().len())
                        .map_err(|_| TestCodecError::Count)?;
                    push_u64(&mut output, pending);
                    for effect in checkpoint.pending() {
                        push_u64(&mut output, *effect);
                    }
                }
                StoreTransactionOp::SettleCheckpoint {
                    actor,
                    at,
                    observation,
                } => {
                    output.push(12);
                    push_u64(&mut output, *actor);
                    push_u64(&mut output, *at);
                    push_observation(&mut output, observation);
                }
                StoreTransactionOp::CancelCommittedOutbox {
                    effect,
                    outcome,
                    observation,
                } => {
                    output.push(13);
                    push_u64(&mut output, *effect);
                    push_u64(&mut output, *outcome);
                    push_observation(&mut output, observation);
                }
                StoreTransactionOp::AppendObservation(_) => {
                    return Err(TestCodecError::UnsupportedOperation);
                }
            }
        }
        Ok(output)
    }

    fn decode_test_transaction(
        bytes: &[u8],
    ) -> Result<StoreTransaction<TestSchema>, TestCodecError> {
        if bytes.is_empty() {
            return Err(TestCodecError::Empty);
        }
        let mut cursor = DecodeCursor::new(bytes);
        let count = usize::try_from(cursor.u64()?).map_err(|_| TestCodecError::Count)?;
        if count == 0 {
            return Err(TestCodecError::Empty);
        }
        let mut operations = Vec::with_capacity(count);
        for _ in 0..count {
            operations.push(match cursor.byte()? {
                1 => {
                    StoreTransactionOp::Append(TransactionAppend::new(cursor.u64()?, cursor.u64()?))
                }
                4 => StoreTransactionOp::OpenOutbox {
                    effect: cursor.u64()?,
                    request: cursor.u64()?,
                },
                5 => StoreTransactionOp::SubmitOutbox {
                    effect: cursor.u64()?,
                },
                6 => StoreTransactionOp::AcquireOutboxDispatch {
                    effect: cursor.u64()?,
                },
                7 => StoreTransactionOp::SettleOutbox {
                    effect: cursor.u64()?,
                    outcome: cursor.u64()?,
                    observation: read_observation(&mut cursor)?,
                },
                8 => StoreTransactionOp::OpenApproval {
                    key: cursor.u64()?,
                    approval: cursor.u64()?,
                },
                9 => StoreTransactionOp::ApproveApproval {
                    key: cursor.u64()?,
                    ticket: cursor.u64()?,
                },
                10 => StoreTransactionOp::SettleApproval {
                    key: cursor.u64()?,
                    observation: read_observation(&mut cursor)?,
                },
                11 => {
                    let actor = cursor.u64()?;
                    let owner = cursor.u64()?;
                    let at = cursor.u64()?;
                    let state = cursor.u64()?;
                    let pending_count =
                        usize::try_from(cursor.u64()?).map_err(|_| TestCodecError::Count)?;
                    let mut pending = Vec::with_capacity(pending_count);
                    for _ in 0..pending_count {
                        pending.push(cursor.u64()?);
                    }
                    StoreTransactionOp::ReplaceCheckpoint(TransactionCheckpoint::new(
                        actor, owner, at, state, pending,
                    ))
                }
                12 => StoreTransactionOp::SettleCheckpoint {
                    actor: cursor.u64()?,
                    at: cursor.u64()?,
                    observation: read_observation(&mut cursor)?,
                },
                13 => StoreTransactionOp::CancelCommittedOutbox {
                    effect: cursor.u64()?,
                    outcome: cursor.u64()?,
                    observation: read_observation(&mut cursor)?,
                },
                tag => return Err(TestCodecError::UnknownTag(tag)),
            });
        }
        if !cursor.finished() {
            return Err(TestCodecError::Trailing);
        }
        StoreTransaction::try_new(operations).map_err(|_| TestCodecError::Empty)
    }

    struct TestDirectory(circular_testkit::temp::StateDir);

    impl TestDirectory {
        fn new() -> Self {
            Self(circular_testkit::temp::StateDir::new(
                "circular-store-sqlite",
            ))
        }

        fn database(&self) -> PathBuf {
            self.0.path().join("store.sqlite3")
        }
    }

    fn transaction(
        operations: Vec<StoreTransactionOp<TestSchema>>,
    ) -> StoreTransaction<TestSchema> {
        StoreTransaction::try_new(operations).expect("test transactions are nonempty")
    }

    fn recover_empty<C>(
        store: RecoveringSqliteTransactionStore<TestSchema, C>,
    ) -> RecoveredSqliteTransactionStore<TestSchema, C>
    where
        C: SqliteTransactionCodec<TestSchema>,
    {
        match store.recover(
            &CheckpointOwners::default(),
            |_, _| -> Result<RecoveryTerminal<TestSchema>, Infallible> {
                unreachable!("empty/replayable fixture has no submitted outbox")
            },
        ) {
            Ok(result) => result.into_parts().0,
            Err(_) => panic!("empty/replayable fixture recovery must succeed"),
        }
    }

    #[test]
    fn raw_journal_reopens_exact_owner_only_committed_prefix_and_freezes_snapshots() {
        let directory = TestDirectory::new();
        let path = directory.database();
        let mut journal = SqliteJournal::create(&path).expect("create fixed schema");
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&path)
                .expect("database metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        let first_receipt = journal.commit(b"first").expect("first durable commit");
        assert_eq!(first_receipt.sequence().get(), 1);
        let frozen = journal.snapshot().expect("first snapshot");
        assert_eq!(frozen.through().map(SqliteCommitSequence::get), Some(1));
        assert_eq!(frozen.entries()[0].payload(), b"first");

        journal.commit(b"second").expect("second durable commit");
        assert_eq!(frozen.entries().len(), 1);
        drop(journal);

        let mut reopened = SqliteJournal::open(&path).expect("reopen committed WAL prefix");
        let snapshot = reopened.snapshot().expect("reopened snapshot");
        assert_eq!(snapshot.through().map(SqliteCommitSequence::get), Some(2));
        assert_eq!(
            snapshot
                .entries()
                .iter()
                .map(SqliteJournalEntry::payload)
                .collect::<Vec<_>>(),
            [b"first".as_slice(), b"second".as_slice()]
        );
    }

    #[test]
    fn journal_format_tag_round_trips_and_refuses_older_versions_without_rewriting() {
        let directory = TestDirectory::new();
        let path = directory.database();
        let mut journal = SqliteJournal::open_namespace(&path, "arrivals/1").unwrap();
        journal.commit(b"fact").unwrap();
        let raw = SqliteJournal::read_only_snapshot(&path).unwrap();
        assert_eq!(
            raw.entries()[0].payload(),
            b"CIRC-NS\x01\x00\x00\x00\x0aarrivals/1fact"
        );
        assert_eq!(journal.snapshot().unwrap().entries()[0].payload(), b"fact");
        drop(journal);
        for version in [0, 2, 99] {
            let directory = TestDirectory::new();
            let path = directory.database();
            let mut raw = SqliteJournal::create(&path).unwrap();
            let mut entry = b"CIRC-NS\x01\x00\x00\x00\x0aarrivals/1fact".to_vec();
            entry[7] = version;
            raw.commit(&entry).unwrap();
            let error = SqliteJournal::read_only_namespace(&path, "arrivals/1").unwrap_err();
            assert!(
                matches!(error, SqliteJournalError::UnsupportedJournalFormat { entry: Some(1), found: Some(found) } if found == version)
            );
            assert!(error.to_string().starts_with(
                "journal format rejected code=28: this daemon does not read this older or unknown journal format (entry 1:"
            ));
            assert_eq!(raw.snapshot().unwrap().entries()[0].payload(), entry);
        }
        assert!(
            matches!(
                decode_namespace(b"CIRC-NS\x01\x00"),
                Err(SqliteJournalError::Integrity { .. })
            ),
            "current format truncation is corruption"
        );
    }

    #[test]
    fn raw_group_commit_preserves_one_row_sequence_checksum_and_receipt_per_payload() {
        let directory = TestDirectory::new();
        let path = directory.database();
        let mut journal = SqliteJournal::create(&path).expect("create group journal");

        let empty = journal
            .commit_group::<&[u8]>(&[])
            .expect_err("empty group is rejected before writing");
        assert!(matches!(empty, SqliteJournalError::EmptyCommitGroup));
        assert!(!journal.is_poisoned());

        let payloads = [b"alpha".as_slice(), b"beta".as_slice(), b"gamma".as_slice()];
        let receipt = journal
            .commit_group(&payloads)
            .expect("one durable group commit");
        assert_eq!(receipt.receipts().len(), payloads.len());
        for (index, (payload, item)) in payloads.iter().zip(receipt.receipts().iter()).enumerate() {
            assert_eq!(item.sequence().get(), (index + 1) as u64);
            assert_eq!(item.checksum(), payload_checksum(payload));
        }

        let snapshot = journal.snapshot().expect("group snapshot");
        assert_eq!(snapshot.entries().len(), payloads.len());
        for (index, (payload, entry)) in payloads.iter().zip(snapshot.entries().iter()).enumerate()
        {
            assert_eq!(entry.sequence().get(), (index + 1) as u64);
            assert_eq!(entry.payload(), *payload);
            assert_eq!(entry.checksum(), receipt.receipts()[index].checksum());
        }
    }

    #[test]
    fn typed_group_keeps_distinct_logical_and_physical_receipts_and_replays_in_order() {
        let directory = TestDirectory::new();
        let path = directory.database();
        let recovering =
            RecoveringSqliteTransactionStore::<TestSchema, _>::create(&path, TestCodec)
                .expect("create typed group store");
        let mut store = recover_empty(recovering);
        let receipt = store
            .commit_group(vec![
                transaction(vec![StoreTransactionOp::Append(TransactionAppend::new(
                    1, 10,
                ))]),
                transaction(vec![StoreTransactionOp::Append(TransactionAppend::new(
                    2, 20,
                ))]),
                transaction(vec![StoreTransactionOp::Append(TransactionAppend::new(
                    1, 10,
                ))]),
            ])
            .expect("three logical commits share one physical sync");
        assert_eq!(receipt.logical().len(), 3);
        assert_eq!(receipt.physical().receipts().len(), 3);
        assert_eq!(
            receipt
                .physical()
                .receipts()
                .iter()
                .map(|item| item.sequence().get())
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(
            store.model().record_digest(&1),
            Some(TestSchema::record_digest(&10))
        );
        assert_eq!(
            store.model().record_digest(&2),
            Some(TestSchema::record_digest(&20))
        );
        drop(store);

        let reopened = RecoveringSqliteTransactionStore::<TestSchema, _>::open(&path, TestCodec)
            .expect("reopen typed group journal");
        let mut reopened = recover_empty(reopened);
        assert_eq!(
            reopened.model().record_digest(&1),
            Some(TestSchema::record_digest(&10))
        );
        assert_eq!(
            reopened.model().record_digest(&2),
            Some(TestSchema::record_digest(&20))
        );
        assert_eq!(
            reopened
                .journal_snapshot()
                .expect("three independent journal rows")
                .entries()
                .len(),
            3
        );
    }

    #[test]
    fn typed_group_prevalidation_and_mid_group_encoding_failure_are_all_or_none() {
        let conflict_directory = TestDirectory::new();
        let conflict_path = conflict_directory.database();
        let recovering =
            RecoveringSqliteTransactionStore::<TestSchema, _>::create(&conflict_path, TestCodec)
                .expect("create conflict fixture");
        let mut store = recover_empty(recovering);
        let failure = store
            .commit_group(vec![
                transaction(vec![StoreTransactionOp::Append(TransactionAppend::new(
                    1, 10,
                ))]),
                transaction(vec![StoreTransactionOp::Append(TransactionAppend::new(
                    1, 11,
                ))]),
            ])
            .expect_err("later conflicting transaction rejects the whole group");
        assert!(matches!(
            failure.reason(),
            SqliteTransactionGroupFailureReason::Rejected {
                transaction: 1,
                operation: 0,
                reason: StoreTransactionReject::AppendConflict { key: 1 },
            }
        ));
        assert!(store.model().record_digest(&1).is_none());
        assert!(
            store
                .journal_snapshot()
                .expect("prevalidation failure writes no row")
                .entries()
                .is_empty()
        );

        let codec_directory = TestDirectory::new();
        let codec_path = codec_directory.database();
        let recovering = RecoveringSqliteTransactionStore::<TestSchema, _>::create(
            &codec_path,
            FailingAtRecordCodec(2),
        )
        .expect("create encoding fixture");
        let mut store = recover_empty(recovering);
        let failure = store
            .commit_group(vec![
                transaction(vec![StoreTransactionOp::Append(TransactionAppend::new(
                    1, 10,
                ))]),
                transaction(vec![StoreTransactionOp::Append(TransactionAppend::new(
                    2, 20,
                ))]),
                transaction(vec![StoreTransactionOp::Append(TransactionAppend::new(
                    3, 30,
                ))]),
            ])
            .expect_err("middle encoding failure rejects the whole group");
        assert!(matches!(
            failure.reason(),
            SqliteTransactionGroupFailureReason::Codec {
                transaction: 1,
                source: TestCodecError::Forced,
            }
        ));
        assert!(store.model().record_digest(&1).is_none());
        assert!(store.model().record_digest(&2).is_none());
        assert!(store.model().record_digest(&3).is_none());
        assert!(
            store
                .journal_snapshot()
                .expect("encoding failure writes no row")
                .entries()
                .is_empty()
        );
    }

    #[test]
    fn group_backend_failure_rolls_back_every_row_poisons_and_keeps_model_old() {
        let directory = TestDirectory::new();
        let path = directory.database();
        let recovering =
            RecoveringSqliteTransactionStore::<TestSchema, _>::create(&path, TestCodec)
                .expect("create backend failure fixture");
        let mut store = recover_empty(recovering);
        store
            .journal
            .connection
            .execute_batch(
                "CREATE TRIGGER reject_second_group_row BEFORE INSERT ON circular_commits
                 WHEN NEW.sequence = 2
                 BEGIN SELECT RAISE(ABORT, 'injected second-row failure'); END;",
            )
            .expect("install middle-row failure trigger");
        let failure = store
            .commit_group(vec![
                transaction(vec![StoreTransactionOp::Append(TransactionAppend::new(
                    1, 10,
                ))]),
                transaction(vec![StoreTransactionOp::Append(TransactionAppend::new(
                    2, 20,
                ))]),
                transaction(vec![StoreTransactionOp::Append(TransactionAppend::new(
                    3, 30,
                ))]),
            ])
            .expect_err("middle insert aborts the SQLite transaction");
        assert!(matches!(
            failure.reason(),
            SqliteTransactionGroupFailureReason::Journal(SqliteJournalError::Database(_))
        ));
        assert!(store.is_poisoned());
        assert!(store.model().record_digest(&1).is_none());
        assert!(store.model().record_digest(&2).is_none());
        assert!(store.model().record_digest(&3).is_none());
        let row_count: u64 = store
            .journal
            .connection
            .query_row("SELECT count(*) FROM circular_commits", [], |row| {
                row.get(0)
            })
            .expect("inspect rolled-back rows");
        let last: u64 = store
            .journal
            .connection
            .query_row(
                "SELECT last_sequence FROM circular_meta WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .expect("inspect rolled-back metadata");
        assert_eq!((row_count, last), (0, 0));
    }

    #[test]
    fn group_receipt_mismatch_poisons_and_never_publishes_candidate_model() {
        let directory = TestDirectory::new();
        let path = directory.database();
        let recovering =
            RecoveringSqliteTransactionStore::<TestSchema, _>::create(&path, TestCodec)
                .expect("create receipt mismatch fixture");
        let mut store = recover_empty(recovering);
        let failure = store
            .commit_group_via(
                vec![
                    transaction(vec![StoreTransactionOp::Append(TransactionAppend::new(
                        1, 10,
                    ))]),
                    transaction(vec![StoreTransactionOp::Append(TransactionAppend::new(
                        2, 20,
                    ))]),
                ],
                |_journal, payloads| {
                    Ok(SqliteCommitGroupReceipt {
                        receipts: vec![
                            SqliteCommitReceipt {
                                sequence: SqliteCommitSequence(1),
                                checksum: payload_checksum(&payloads[0]),
                            },
                            SqliteCommitReceipt {
                                sequence: SqliteCommitSequence(3),
                                checksum: payload_checksum(&payloads[1]),
                            },
                        ]
                        .into_boxed_slice(),
                    })
                },
            )
            .expect_err("non-contiguous physical receipts fail closed");
        assert!(matches!(
            failure.reason(),
            SqliteTransactionGroupFailureReason::Journal(
                SqliteJournalError::ReceiptMismatch { .. }
            )
        ));
        assert!(store.is_poisoned());
        assert!(store.model().record_digest(&1).is_none());
        assert!(store.model().record_digest(&2).is_none());
        let rows: u64 = store
            .journal
            .connection
            .query_row("SELECT count(*) FROM circular_commits", [], |row| {
                row.get(0)
            })
            .expect("fake persistence wrote nothing");
        assert_eq!(rows, 0);
    }

    #[test]
    fn raw_group_sequence_exhaustion_writes_no_partial_rows_or_metadata() {
        let directory = TestDirectory::new();
        let path = directory.database();
        let mut journal = SqliteJournal::create(&path).expect("create exhaustion fixture");
        let last = MAX_SQLITE_SEQUENCE - 1;
        journal
            .connection
            .execute(
                "UPDATE circular_meta SET last_sequence = ?1 WHERE singleton = 1",
                params![i64::try_from(last).expect("maximum SQLite sequence is i64")],
            )
            .expect("inject near-exhausted metadata");
        let failure = journal
            .commit_group(&[b"one".as_slice(), b"two".as_slice()])
            .expect_err("two-row reservation exceeds the sequence domain");
        assert!(matches!(failure, SqliteJournalError::SequenceExhausted));
        assert!(journal.is_poisoned());
        let rows: u64 = journal
            .connection
            .query_row("SELECT count(*) FROM circular_commits", [], |row| {
                row.get(0)
            })
            .expect("inspect exhausted journal rows");
        let preserved: u64 = journal
            .connection
            .query_row(
                "SELECT last_sequence FROM circular_meta WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .expect("inspect exhausted metadata");
        assert_eq!(rows, 0);
        assert_eq!(preserved, last);
    }

    #[test]
    fn typed_atomic_state_replays_and_logical_rejection_does_not_append() {
        let directory = TestDirectory::new();
        let path = directory.database();
        let recovering =
            RecoveringSqliteTransactionStore::<TestSchema, _>::create(&path, TestCodec)
                .expect("create typed store");
        let mut store = recover_empty(recovering);
        store
            .commit(transaction(vec![
                StoreTransactionOp::Append(TransactionAppend::new(1, 10)),
                StoreTransactionOp::OpenOutbox {
                    effect: 2,
                    request: 20,
                },
            ]))
            .expect("atomic record and custody commit");

        let rejected = store
            .commit(transaction(vec![StoreTransactionOp::Append(
                TransactionAppend::new(1, 99),
            )]))
            .expect_err("same key with different bytes is rejected");
        assert!(matches!(
            rejected.reason(),
            StoreTransactionFailureReason::Rejected {
                operation: 0,
                reason: StoreTransactionReject::AppendConflict { key: 1 },
            }
        ));
        assert_eq!(
            store
                .journal_snapshot()
                .expect("journal after rejection")
                .entries()
                .len(),
            1
        );
        drop(store);

        let recovering = RecoveringSqliteTransactionStore::<TestSchema, _>::open(&path, TestCodec)
            .expect("reopen typed store");
        let result = recovering
            .recover(
                &CheckpointOwners::default(),
                |_, _| -> Result<RecoveryTerminal<TestSchema>, Infallible> {
                    unreachable!("fixture has no submitted outbox")
                },
            )
            .unwrap_or_else(|_| panic!("recovery succeeds"));
        assert_eq!(result.plan().resubmit_outbox(), &[2]);
        assert_eq!(
            result.store().model().record_digest(&1),
            Some(TestSchema::record_digest(&10))
        );
        assert_eq!(
            result
                .store()
                .model()
                .outbox(&2)
                .map(TransactionOutbox::request),
            Some(&20)
        );
    }

    #[test]
    fn submitted_outbox_is_terminally_settled_durably_before_writable_reopen() {
        let directory = TestDirectory::new();
        let path = directory.database();
        let recovering =
            RecoveringSqliteTransactionStore::<TestSchema, _>::create(&path, TestCodec)
                .expect("create typed store");
        let mut store = recover_empty(recovering);
        store
            .commit(transaction(vec![StoreTransactionOp::OpenOutbox {
                effect: 7,
                request: 70,
            }]))
            .expect("durable committed outbox");
        store
            .commit(transaction(vec![
                StoreTransactionOp::AcquireOutboxDispatch { effect: 7 },
            ]))
            .expect("durable one-shot submitted mark");
        assert_eq!(
            store.model().outbox(&7).map(|row| row.phase()),
            Some(TransactionOutboxPhase::Submitted)
        );
        drop(store);

        let recovering = RecoveringSqliteTransactionStore::<TestSchema, _>::open(&path, TestCodec)
            .expect("reopen submitted outbox");
        let result = recovering
            .recover(&CheckpointOwners::default(), |effect, request| {
                assert_eq!((*effect, *request), (7, 70));
                Ok::<_, Infallible>(RecoveryTerminal::new(
                    700,
                    TransactionObservation::new(71, 701),
                ))
            })
            .unwrap_or_else(|_| panic!("terminal recovery is durable"));
        assert_eq!(result.plan().terminal_outbox(), &[7]);
        assert!(result.store().model().outbox(&7).is_none());
        assert_eq!(result.store().model().outcome(&7), Some(&700));
        assert_eq!(
            result.store().model().observation_digest(&71),
            Some(TestSchema::observation_digest(&701))
        );
        drop(result);

        let recovering = RecoveringSqliteTransactionStore::<TestSchema, _>::open(&path, TestCodec)
            .expect("second reopen sees recovery settlement");
        let result = recovering
            .recover(
                &CheckpointOwners::default(),
                |_, _| -> Result<RecoveryTerminal<TestSchema>, Infallible> {
                    panic!("durably settled effect must not be terminalized twice")
                },
            )
            .unwrap_or_else(|_| panic!("second recovery succeeds without terminal callback"));
        assert!(result.plan().terminal_outbox().is_empty());
        assert_eq!(result.store().model().outcome(&7), Some(&700));
    }

    #[test]
    fn encode_or_database_failure_never_publishes_logical_state_and_database_failure_poisoned() {
        let codec_directory = TestDirectory::new();
        let codec_path = codec_directory.database();
        let recovering =
            RecoveringSqliteTransactionStore::<TestSchema, _>::create(&codec_path, FailingCodec)
                .expect("create store before codec use");
        let mut store = recover_empty(recovering);
        let failure = store
            .commit(transaction(vec![StoreTransactionOp::Append(
                TransactionAppend::new(1, 10),
            )]))
            .expect_err("codec failure prevents journal commit");
        assert!(matches!(
            failure.reason(),
            StoreTransactionFailureReason::Backend(SqliteCommitError::Codec(
                TestCodecError::Forced
            ))
        ));
        assert!(store.model().record_digest(&1).is_none());
        assert!(
            store
                .journal_snapshot()
                .expect("codec failure does not poison journal")
                .entries()
                .is_empty()
        );

        let database_directory = TestDirectory::new();
        let database_path = database_directory.database();
        let recovering =
            RecoveringSqliteTransactionStore::<TestSchema, _>::create(&database_path, TestCodec)
                .expect("create database failure fixture");
        let mut store = recover_empty(recovering);
        store
            .journal
            .connection
            .execute_batch(
                "CREATE TRIGGER reject_commit BEFORE INSERT ON circular_commits
                 BEGIN SELECT RAISE(ABORT, 'injected commit failure'); END;",
            )
            .expect("install test-only failure trigger");
        let failure = store
            .commit(transaction(vec![StoreTransactionOp::Append(
                TransactionAppend::new(2, 20),
            )]))
            .expect_err("SQLite failure returns no receipt");
        assert!(matches!(
            failure.reason(),
            StoreTransactionFailureReason::Backend(SqliteCommitError::Journal(
                SqliteJournalError::Database(_)
            ))
        ));
        assert!(store.is_poisoned());
        assert!(store.model().record_digest(&2).is_none());
        let second = store
            .commit(transaction(vec![StoreTransactionOp::Append(
                TransactionAppend::new(3, 30),
            )]))
            .expect_err("poisoned handle cannot continue");
        assert!(matches!(
            second.reason(),
            StoreTransactionFailureReason::Backend(SqliteCommitError::Journal(
                SqliteJournalError::Poisoned
            ))
        ));
    }
}
