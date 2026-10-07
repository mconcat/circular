
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;

use circular_core::{Boundary, Ceilings, Value, decode, encode};
use circular_store::SqliteJournal;

use crate::daemon::authoring::{AuthoringState, JournalEntryRejection};
use engine::authoring_assembly::ledger::ProjectCreation;
use engine::authoring_assembly::rejection::JournalVocabulary;

#[cfg(test)]
const JOURNAL_FILE_NAME: &str = engine::state_journal::STATE_JOURNAL_FILE_NAME;

struct OpenJournal {
    journal: SqliteJournal,
}

struct JournalEpoch {
    sequence: u64,
    payload: Box<[u8]>,
}

struct AuthoringSegment {
    previous: Option<Arc<AuthoringSegment>>,
    entries: Box<[JournalEpoch]>,
}

impl Drop for AuthoringSegment {
    fn drop(&mut self) {
        let mut previous = self.previous.take();
        while let Some(segment) = previous {
            match Arc::try_unwrap(segment) {
                Ok(mut segment) => previous = segment.previous.take(),
                Err(_) => break,
            }
        }
    }
}

struct AuthoringPrefix {
    creation: ProjectCreation,
    tail: Option<Arc<AuthoringSegment>>,
    projection: AuthoringState,
    through: u64,
}

impl AuthoringPrefix {
    fn epochs(&self) -> impl Iterator<Item = &JournalEpoch> {
        let mut segments = Vec::new();
        let mut next = self.tail.as_deref();
        while let Some(segment) = next {
            segments.push(segment);
            next = segment.previous.as_deref();
        }
        segments
            .into_iter()
            .rev()
            .flat_map(|segment| segment.entries.iter())
    }

    fn load_at(&self, cursor: u64) -> Result<AuthoringState, String> {
        let created = AuthoringState::created(&self.creation);
        if cursor == 0 {
            return Ok(created);
        }
        let mut projection = Some(created);
        for entry in self.epochs() {
            let next = fold_entry(projection, entry.sequence, &entry.payload)?;
            match next.cursor().cmp(&cursor) {
                std::cmp::Ordering::Less => projection = Some(next),
                std::cmp::Ordering::Equal => return Ok(next),
                std::cmp::Ordering::Greater => {
                    return Err(format!(
                        "authoring journal cursor {cursor} is before retained entry {} at cursor {}",
                        entry.sequence,
                        next.cursor()
                    ));
                }
            }
        }
        Err(format!(
            "authoring journal does not contain cursor {cursor}"
        ))
    }
}

pub(crate) struct AuthoringStore {
    path: std::path::PathBuf,
    directory: PathBuf,
    open: Mutex<OpenJournal>,
    prefix: ArcSwap<AuthoringPrefix>,
    #[cfg(test)]
    before_commit: Mutex<Option<Box<dyn FnOnce() -> Result<(), String> + Send>>>,
}

#[derive(Debug)]
pub(crate) enum AuthoringJournalReadError {
    UnsupportedFormat { entry: u64, reason: String },
    Corrupt(String),
}

impl From<circular_store::SqliteJournalError> for AuthoringJournalReadError {
    fn from(error: circular_store::SqliteJournalError) -> Self {
        match error {
            circular_store::SqliteJournalError::UnsupportedJournalFormat { entry, found } => {
                Self::UnsupportedFormat {
                    entry: entry.unwrap_or(0),
                    reason: format!(
                        "journal format version {} is not supported",
                        found.map_or_else(|| "missing".to_owned(), |n| n.to_string())
                    ),
                }
            }
            circular_store::SqliteJournalError::UnsupportedRecordFormat {
                entry,
                vocabulary,
                found,
            } => Self::UnsupportedFormat {
                entry,
                reason: format!("{vocabulary}={found}"),
            },
            other => Self::Corrupt(other.to_string()),
        }
    }
}

impl AuthoringJournalReadError {
    pub(crate) fn startup_diagnostic(&self) -> String {
        match self {
            Self::UnsupportedFormat { .. } => self.to_string(),
            Self::Corrupt(reason) => format!("refused corrupt authoring journal: {reason}"),
        }
    }

    fn invalid_entry(entry: u64, rejection: JournalEntryRejection) -> Self {
        match rejection {
            JournalEntryRejection::UnknownVocabulary(reason) => Self::UnsupportedFormat {
                entry,
                reason: reason.to_string(),
            },
            JournalEntryRejection::Corrupt(reason) => Self::Corrupt(format!(
                "authoring journal entry {entry} is invalid: {reason}"
            )),
        }
    }
}

impl std::fmt::Display for AuthoringJournalReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedFormat { entry, reason } => write!(
                f,
                "journal format rejected code={}: this daemon does not read this older or unknown journal format (entry {entry}: {reason})",
                circular_protocol::rejection_code::RejectionReason::JournalFormatRejected
                    .recorded_code()
            ),
            Self::Corrupt(reason) => f.write_str(reason),
        }
    }
}

impl From<String> for AuthoringJournalReadError {
    fn from(reason: String) -> Self {
        Self::Corrupt(reason)
    }
}

impl From<AuthoringJournalReadError> for String {
    fn from(error: AuthoringJournalReadError) -> Self {
        error.to_string()
    }
}

impl AuthoringStore {
    #[cfg(test)]
    pub(crate) fn before_next_commit(
        &self,
        hook: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) {
        *self.before_commit.lock().unwrap() = Some(Box::new(hook));
    }

    pub(crate) fn journal_path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn state_directory(&self) -> &Path {
        &self.directory
    }

    pub(crate) fn open(directory: &Path) -> Result<Self, AuthoringJournalReadError> {
        let path = engine::state_journal::state_journal_path(directory);
        let mut journal = SqliteJournal::open_namespace(
            &path,
            engine::state_journal::AUTHORING_JOURNAL_NAMESPACE,
        )
        .map_err(AuthoringJournalReadError::from)?;
        let snapshot =
            journal
                .snapshot()
                .map_err(|error| match AuthoringJournalReadError::from(error) {
                    AuthoringJournalReadError::Corrupt(reason) => {
                        AuthoringJournalReadError::Corrupt(format!(
                            "cannot snapshot authoring journal: {reason}"
                        ))
                    }
                    unsupported => unsupported,
                })?;
        let mut through = snapshot.through().map_or(0, |through| through.get());
        let creation = match snapshot.entries().first() {
            Some(first) => creation_entry(first.sequence().get(), first.payload())?,
            None => {
                let creation = ProjectCreation {
                    project: engine::state_manifest::issue_project_identity()?,
                    environment: engine::authoring_assembly::ledger::genesis_environment(),
                };
                let payload = encode(
                    &creation
                        .journal_entry_value()
                        .map_err(|rejection| rejection.to_string())?,
                    Ceilings::for_boundary(Boundary::Journal),
                )
                .map_err(|error| format!("project creation does not encode: {error:?}"))?;
                let receipt = journal
                    .commit(&payload)
                    .map_err(|error| format!("cannot record project creation: {error}"))?;
                through = receipt.sequence().get();
                creation
            }
        };
        let epochs = snapshot.entries().get(1..).unwrap_or_default();
        let mut projection = AuthoringState::created(&creation);
        for entry in epochs {
            projection = fold_entry(Some(projection), entry.sequence().get(), entry.payload())?;
        }
        let tail = (!epochs.is_empty()).then(|| {
            Arc::new(AuthoringSegment {
                previous: None,
                entries: epochs
                    .iter()
                    .map(|entry| JournalEpoch {
                        sequence: entry.sequence().get(),
                        payload: entry.payload().into(),
                    })
                    .collect(),
            })
        });
        Ok(Self {
            path,
            directory: directory.to_owned(),
            #[cfg(test)]
            before_commit: Mutex::new(None),
            open: Mutex::new(OpenJournal { journal }),
            prefix: ArcSwap::from_pointee(AuthoringPrefix {
                creation,
                tail,
                projection,
                through,
            }),
        })
    }

    pub(crate) fn load(&self) -> Result<AuthoringState, String> {
        Ok(self.prefix.load_full().projection.clone())
    }

    pub(crate) fn creation(&self) -> ProjectCreation {
        self.prefix.load_full().creation.clone()
    }

    pub(crate) fn retained(&self) -> RetainedCommits {
        RetainedCommits {
            prefix: self.prefix.load_full(),
        }
    }

    /// Replays one immutable committed journal prefix through an exact cursor.
    /// Historical answers come from journal bytes, never the latest projection.
    /// The reader pins that prefix once and never acquires the writer mutex.
    pub(crate) fn load_at(&self, cursor: u64) -> Result<AuthoringState, String> {
        self.prefix.load_full().load_at(cursor)
    }

    pub(crate) fn save(&self, state: &AuthoringState) -> Result<(), String> {
        let entry = state
            .journal_epoch_value()
            .map_err(|rejection| rejection.to_string())?;
        let payload = encode(&entry, Ceilings::for_boundary(Boundary::Journal))
            .map_err(|error| format!("authoring journal epoch does not encode: {error:?}"))?;
        let mut open = self
            .open
            .lock()
            .map_err(|_| "authoring journal lock is poisoned".to_owned())?;
        let previous = self.prefix.load_full();
        let follows = previous.projection.cursor().checked_add(1);
        if follows != Some(state.cursor()) {
            return Err(format!(
                "authoring epoch at cursor {} does not follow the journal prefix at cursor {}",
                state.cursor(),
                previous.projection.cursor()
            ));
        }
        #[cfg(test)]
        if let Some(hook) = self.before_commit.lock().unwrap().take() {
            hook().map_err(|reason| format!("cannot publish authoring journal epoch: {reason}"))?;
        }
        let receipt = open
            .journal
            .commit(&payload)
            .map_err(|error| format!("cannot publish authoring journal epoch: {error}"))?;
        self.prefix.store(Arc::new(AuthoringPrefix {
            creation: previous.creation.clone(),
            tail: Some(Arc::new(AuthoringSegment {
                previous: previous.tail.clone(),
                entries: Box::new([JournalEpoch {
                    sequence: receipt.sequence().get(),
                    payload: payload.into_boxed_slice(),
                }]),
            })),
            projection: state.clone(),
            through: receipt.sequence().get(),
        }));
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct RetainedCommits {
    prefix: Arc<AuthoringPrefix>,
}

impl RetainedCommits {
    pub(crate) fn at_cursor(&self, cursor: u64) -> Result<AuthoringState, String> {
        self.prefix.load_at(cursor)
    }

    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        let creation = ProjectCreation::for_test();
        Self {
            prefix: Arc::new(AuthoringPrefix {
                projection: AuthoringState::created(&creation),
                creation,
                tail: None,
                through: 0,
            }),
        }
    }

    pub(crate) fn creation(&self) -> &ProjectCreation {
        &self.prefix.creation
    }

    fn entries(&self) -> Vec<&JournalEpoch> {
        self.prefix.epochs().collect()
    }

    pub(crate) fn epochs(&self) -> Vec<(u64, &[u8])> {
        self.prefix
            .epochs()
            .map(|epoch| (epoch.sequence, &*epoch.payload))
            .collect()
    }

    pub(crate) fn through(&self) -> u64 {
        self.prefix.through
    }

    pub(crate) fn state_through(
        &self,
        through: u64,
    ) -> Result<(usize, u64, AuthoringState), String> {
        let entries = self.entries();
        let count = entries.partition_point(|epoch| epoch.sequence <= through);
        let last = count
            .checked_sub(1)
            .map_or(0, |index| entries[index].sequence);
        Ok((count, last, self.state_before(count)?))
    }

    pub(crate) fn state_before(&self, index: usize) -> Result<AuthoringState, String> {
        let entries = self.entries();
        if index >= entries.len() {
            return Ok(self.prefix.projection.clone());
        }
        let mut state = AuthoringState::created(&self.prefix.creation);
        for epoch in &entries[..index] {
            state = fold_entry(Some(state), epoch.sequence, &epoch.payload)?;
        }
        Ok(state)
    }

    pub(crate) fn environment_before(
        &self,
        commit: u64,
    ) -> Result<circular_protocol::declaration_payload::AuthoringEnvironment, String> {
        let entries = self.entries();
        let count = entries.partition_point(|epoch| epoch.sequence < commit);
        let Some(index) = count.checked_sub(1) else {
            return Ok(self.prefix.creation.environment.clone());
        };
        decode_epoch(entries[index])?
            .environment_after()
            .map_err(|reason| {
                format!(
                    "retained authoring epoch {} has no environment: {reason}",
                    entries[index].sequence
                )
            })
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.prefix.epochs().count()
    }

    pub(crate) fn index_after(&self, after: u64) -> usize {
        let entries = self.entries();
        let mut low = 0;
        let mut high = entries.len();
        while low < high {
            let middle = low + (high - low) / 2;
            match decode_epoch(entries[middle]) {
                Ok(commit) if commit.cursor <= after => low = middle + 1,
                Ok(_) | Err(_) => high = middle,
            }
        }
        low
    }

    pub(crate) fn first_cursor(&self) -> Option<u64> {
        let entries = self.entries();
        entries
            .first()
            .and_then(|entry| decode_epoch(entry).ok())
            .map(|commit| commit.cursor)
    }

    pub(crate) fn values(&self, from: usize, through: usize) -> Vec<Result<Value, String>> {
        let entries = self.entries();
        let through = through.min(entries.len());
        if from >= through {
            return Vec::new();
        }
        entries[from..through]
            .iter()
            .map(|entry| decode_value(entry))
            .collect()
    }
}

fn decode_value(entry: &JournalEpoch) -> Result<Value, String> {
    decode(&entry.payload, Ceilings::for_boundary(Boundary::Journal)).map_err(|error| {
        format!(
            "retained authoring epoch {} does not decode: {error:?}",
            entry.sequence
        )
    })
}

fn decode_epoch(entry: &JournalEpoch) -> Result<crate::daemon::authoring::DurableCommit, String> {
    crate::daemon::authoring::DurableCommit::from_journal_entry(decode_value(entry)?)
        .map_err(|rejection| rejection.to_string())
}

fn creation_entry(
    sequence: u64,
    payload: &[u8],
) -> Result<ProjectCreation, AuthoringJournalReadError> {
    let value = decode(payload, Ceilings::for_boundary(Boundary::Journal)).map_err(|error| {
        format!("authoring journal entry {sequence} does not decode: {error:?}")
    })?;
    ProjectCreation::from_journal_entry(&value)
        .map_err(|reason| AuthoringJournalReadError::invalid_entry(sequence, reason))?
        .ok_or_else(|| {
            AuthoringJournalReadError::invalid_entry(
                sequence,
                JournalEntryRejection::UnknownVocabulary(JournalVocabulary::NoProjectCreation),
            )
        })
}

#[cfg(test)]
pub(crate) fn create_project_for_test(journal_path: &Path) {
    AuthoringStore::open(journal_path.parent().expect("state directory"))
        .expect("project creation");
}

pub(crate) fn recorded_creation(directory: &Path) -> Result<ProjectCreation, String> {
    let snapshot = SqliteJournal::read_only_namespace(
        engine::state_journal::state_journal_path(directory),
        engine::state_journal::AUTHORING_JOURNAL_NAMESPACE,
    )
    .map_err(|error| format!("project creation read failed: {error}"))?;
    let first = snapshot
        .entries()
        .first()
        .ok_or("state has no project creation record")?;
    Ok(creation_entry(first.sequence().get(), first.payload())?)
}

fn fold_entry(
    previous: Option<AuthoringState>,
    sequence: u64,
    payload: &[u8],
) -> Result<AuthoringState, AuthoringJournalReadError> {
    let value = decode(payload, Ceilings::for_boundary(Boundary::Journal)).map_err(|error| {
        format!(
            "authoring journal entry {} does not decode: {error:?}",
            sequence
        )
    })?;
    AuthoringState::fold_journal_entry(previous, value)
        .map(|(state, _)| state)
        .map_err(|reason| AuthoringJournalReadError::invalid_entry(sequence, reason))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use circular_core::{Ceiling, CodecError, Value};
    use circular_protocol::authoring_snapshot::AuthoringSnapshotEncoder;
    use circular_protocol::declaration_payload::{
        AddressRef, AnnotationDeclaration, AnnotationKind, AuthoringEnvironment, BeginEpoch,
        ExpectedRevision, PlanAnnotationKey,
    };

    use super::*;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    fn test_directory(label: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "circular-authoring-store-{label}-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).expect("test directory");
        directory
    }

    fn environment() -> AuthoringEnvironment {
        AuthoringEnvironment {
            declaration_schema: vec![1],
            spec_set: vec![3],
        }
    }

    fn accepted_annotation(key: &PlanAnnotationKey, declaration: &AnnotationDeclaration) -> Value {
        let Value::Object(object) = AuthoringSnapshotEncoder::new(&[])
            .upsert_annotation(key, declaration)
            .expect("annotation command encodes")
        else {
            panic!("annotation command is an object");
        };
        let mut fields = object.into_map();
        let Value::Array(address) = fields.get_mut("annotation").expect("annotation address")
        else {
            panic!("annotation address is an array");
        };
        address[0] = Value::Int(1);
        Value::object(fields).expect("accepted annotation object")
    }

    fn commit_annotation(state: &mut AuthoringState, ordinal: u64, body_bytes: usize) {
        let expected_revision = state
            .revision()
            .map_or(ExpectedRevision::Absent, |revision| {
                ExpectedRevision::At(revision.to_vec())
            });
        let begin = BeginEpoch {
            scope: AddressRef::Absolute(Vec::new()),
            commit_id: ordinal.to_be_bytes().to_vec(),
            expected_revision,
            expected_environment: environment(),
        };
        let mut candidate = state
            .begin_candidate(&begin, &ordinal.to_be_bytes(), 4)
            .expect("epoch opens");
        candidate.issued_epoch = Some(ordinal.to_be_bytes().to_vec());
        let key = PlanAnnotationKey {
            scope: Vec::new(),
            local: format!("note-{ordinal}"),
        };
        let declaration = AnnotationDeclaration {
            kind: AnnotationKind::Note,
            refs: Vec::new(),
            body: "x".repeat(body_bytes),
        };
        candidate.add_annotation(key.clone(), declaration.clone());
        candidate
            .accepted_commands
            .push(accepted_annotation(&key, &declaration));
        candidate
            .request_parts
            .push((8, ordinal.to_be_bytes().to_vec()));
        let digest = AuthoringState::request_digest(&candidate);
        state
            .promote_commit(candidate, digest)
            .expect("epoch commits");
    }

    fn assert_same_state(expected: &AuthoringState, actual: &AuthoringState) {
        assert_eq!(expected.cursor(), actual.cursor());
        assert_eq!(expected.revision(), actual.revision());
        assert_eq!(expected.environment(), actual.environment());
        assert_eq!(
            expected
                .snapshot(Vec::new())
                .expect("expected snapshot")
                .items,
            actual.snapshot(Vec::new()).expect("actual snapshot").items
        );
        assert_eq!(expected.commit_count(), actual.commit_count());
        match (expected.last_commit(), actual.last_commit()) {
            (Some(expected), Some(actual)) => {
                assert_eq!(expected.cursor, actual.cursor);
                assert_eq!(expected.terminal, actual.terminal);
            }
            (None, None) => {}
            (expected, actual) => panic!("tail epoch differs: {expected:?} vs {actual:?}"),
        }
    }

    #[test]
    fn commits_share_immutable_segments_and_reopen_the_same_prefix() {
        let directory = test_directory("m7-segments");
        let store = AuthoringStore::open(&directory).unwrap();
        let empty = store.prefix.load_full();
        let mut state = AuthoringState::default();
        let mut previous = empty.clone();
        for ordinal in 1..=4 {
            commit_annotation(&mut state, ordinal, 32);
            store.save(&state).unwrap();
            let current = store.prefix.load_full();
            let segment = current.tail.as_ref().unwrap();
            assert_eq!(segment.entries.len(), 1, "only the new epoch is appended");
            if let Some(old) = &previous.tail {
                assert!(
                    Arc::ptr_eq(segment.previous.as_ref().unwrap(), old),
                    "previous epoch bodies must be shared, not copied"
                );
                assert!(
                    previous.load_at(ordinal).is_err(),
                    "a pinned prefix never advances"
                );
            }
            assert_eq!(
                current.load_at(ordinal).unwrap().commit_count(),
                ordinal as usize
            );
            previous = current;
        }
        assert_eq!(empty.projection.cursor(), 0);
        assert!(empty.load_at(1).is_err());
        let frozen = store.prefix.load_full();
        assert!(
            store.save(&state).is_err(),
            "the same epoch cannot be committed twice"
        );
        assert!(
            Arc::ptr_eq(&frozen, &store.prefix.load_full()),
            "a refused save publishes nothing"
        );
        drop(store);
        let reopened = AuthoringStore::open(&directory).unwrap();
        for cursor in 1..=4 {
            let before = frozen.load_at(cursor).unwrap();
            let after = reopened.load_at(cursor).unwrap();
            assert_eq!(after.cursor(), cursor);
            assert_eq!(after.commit_count(), cursor as usize);
            assert_same_state(&before, &after);
        }
        let disk = reopened.prefix.load_full();
        assert!(
            frozen
                .epochs()
                .zip(disk.epochs())
                .all(|(a, b)| a.sequence == b.sequence && a.payload == b.payload)
        );
        assert_eq!(disk.epochs().count(), 4);
        drop(reopened);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn committed_epochs_replay_to_the_same_state_after_reopen() {
        let directory = test_directory("replay");
        let store = AuthoringStore::open(&directory).expect("opens empty journal");
        let mut state = AuthoringState::default();
        for ordinal in 1..=5 {
            commit_annotation(&mut state, ordinal, 32);
            store.save(&state).expect("appends epoch");
        }
        drop(store);

        let reopened = AuthoringStore::open(&directory).expect("reopens journal");
        let restored = reopened.load().expect("loads");
        assert_same_state(&state, &restored);
        std::fs::remove_dir_all(&directory).expect("remove test directory");
    }

    #[test]
    fn cursor_replay_reads_committed_segments_independently_of_latest_projection() {
        let directory = test_directory("cursor-replay");
        let store = AuthoringStore::open(&directory).expect("opens empty journal");
        let mut state = AuthoringState::default();
        let mut first_live = None;
        for ordinal in 1..=2 {
            commit_annotation(&mut state, ordinal, 32);
            store.save(&state).expect("appends epoch");
            if ordinal == 1 {
                first_live = Some(state.clone());
            }
        }

        let prefix = store.prefix.load_full();
        store.prefix.store(Arc::new(AuthoringPrefix {
            creation: prefix.creation.clone(),
            tail: prefix.tail.clone(),
            projection: AuthoringState::created(&prefix.creation),
            through: prefix.through,
        }));
        let first = store
            .load_at(1)
            .expect("first cursor replays committed bytes");
        let second = store
            .load_at(2)
            .expect("second cursor replays committed bytes");
        assert_same_state(&first_live.expect("first live cut"), &first);
        assert_same_state(&state, &second);
        assert_eq!(first.cursor(), 1);
        assert_eq!(first.commit_count(), 1);
        assert_eq!(second.cursor(), 2);
        assert_eq!(second.commit_count(), 2);
        std::fs::remove_dir_all(&directory).expect("remove test directory");
    }

    #[test]
    fn journal_reopen_retains_more_than_256_authoring_epochs() {
        let directory = test_directory("complete-300-epochs");
        let store = AuthoringStore::open(&directory).unwrap();
        let mut state = AuthoringState::default();
        for ordinal in 1..=300 {
            commit_annotation(&mut state, ordinal, 8);
            store.save(&state).unwrap();
        }
        drop(store);
        let reopened = AuthoringStore::open(&directory).unwrap();
        let replay = reopened.load().unwrap();
        assert_same_state(&state, &replay);
        assert_eq!(replay.commit_count(), 300);
        assert_eq!(
            reopened.retained().first_cursor(),
            Some(1),
            "the journal carries the first epoch unchanged"
        );
        assert_eq!(reopened.retained().len(), 300);
        assert_eq!(reopened.load_at(1).unwrap().commit_count(), 1);
        drop(reopened);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn journal_format_refusal_is_not_reported_as_corruption() {
        let directory = test_directory("journal-version");
        let path = directory.join(JOURNAL_FILE_NAME);
        let mut raw = SqliteJournal::create(&path).unwrap();
        raw.commit(b"CIRC-NS\x02\x00\x00\x00\x09authoringold")
            .unwrap();
        let error = match AuthoringStore::open(&directory) {
            Ok(_) => panic!("old journal version must not open"),
            Err(error) => error.startup_diagnostic(),
        };
        assert_eq!(
            error,
            "journal format rejected code=28: this daemon does not read this older or unknown journal format (entry 1: journal format version 2 is not supported)"
        );
        assert!(!error.contains("corrupt"));
        assert_eq!(
            raw.snapshot().unwrap().entries()[0].payload(),
            b"CIRC-NS\x02\x00\x00\x00\x09authoringold"
        );
        drop(raw);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn cumulative_history_can_exceed_the_one_mib_value_ceiling() {
        const COMMITS: u64 = 48;
        const BODY_BYTES: usize = 24 * 1_024;

        let directory = test_directory("ceiling");
        let store = AuthoringStore::open(&directory).expect("opens empty journal");
        let mut state = AuthoringState::default();
        for ordinal in 1..=COMMITS {
            commit_annotation(&mut state, ordinal, BODY_BYTES);
            store
                .save(&state)
                .expect("each epoch remains below the ceiling");
        }
        drop(store);

        let path = directory.join(JOURNAL_FILE_NAME);
        let mut journal = SqliteJournal::open_namespace(
            &path,
            engine::state_journal::AUTHORING_JOURNAL_NAMESPACE,
        )
        .expect("opens raw journal");
        let snapshot = journal.snapshot().expect("snapshots raw journal");
        assert_eq!(snapshot.entries().len(), COMMITS as usize + 1);
        assert!(
            snapshot
                .entries()
                .iter()
                .all(|entry| entry.payload().len() < 1 << 20)
        );
        let journal_payload_bytes = snapshot
            .entries()
            .iter()
            .map(|entry| entry.payload().len())
            .sum::<usize>();
        assert!(journal_payload_bytes > 1 << 20);
        eprintln!("accumulated epoch payloads: {journal_payload_bytes} bytes");
        drop(journal);

        let reopened = AuthoringStore::open(&directory).expect("reopens over-ceiling history");
        let restored = reopened.load().expect("loads");
        assert_same_state(&state, &restored);
        std::fs::remove_dir_all(&directory).expect("remove test directory");
    }

    fn refused_entry(label: &str, payload: &[u8]) -> String {
        let directory = test_directory(label);
        let path = directory.join(JOURNAL_FILE_NAME);
        create_project_for_test(&path);
        let mut journal = SqliteJournal::open_namespace(
            &path,
            engine::state_journal::AUTHORING_JOURNAL_NAMESPACE,
        )
        .unwrap();
        journal.commit(payload).unwrap();
        drop(journal);
        let before = std::fs::read(&path).unwrap();
        let error = AuthoringStore::open(&directory)
            .err()
            .expect("refused entry");
        let diagnostic = error.startup_diagnostic();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "refusal must not repair or truncate"
        );
        std::fs::remove_dir_all(directory).unwrap();
        diagnostic
    }

    #[test]
    fn state_written_before_the_actor_rename_simply_does_not_open() {
        let mut state = AuthoringState::default();
        commit_annotation(&mut state, 1, 1);
        let Value::Object(root) = state.journal_epoch_value().unwrap() else {
            panic!()
        };
        let mut root = root.into_map();
        let Value::Object(epoch) = root.remove("epoch").unwrap() else {
            panic!()
        };
        let mut epoch = epoch.into_map();
        let Value::Array(mut commands) = epoch.remove("commands").unwrap() else {
            panic!()
        };
        let Value::Object(command) = commands.remove(0) else {
            panic!()
        };
        let mut command = command.into_map();
        command.insert("kind".into(), Value::string("UpsertNode"));
        commands.insert(0, Value::object(command).unwrap());
        epoch.insert("commands".into(), Value::Array(commands));
        root.insert("epoch".into(), Value::object(epoch).unwrap());
        let diagnostic = refused_entry(
            "pre-rename-state",
            &encode(
                &Value::object(root).unwrap(),
                Ceilings::for_boundary(Boundary::Journal),
            )
            .unwrap(),
        );
        assert_eq!(
            diagnostic,
            "journal format rejected code=28: this daemon does not read this older or unknown journal format (entry 2: unknown persisted accepted command \"UpsertNode\")"
        );
    }

    #[test]
    fn corrupt_journal_entry_is_refused_without_truncation() {
        let directory = test_directory("corrupt");
        let path = directory.join(JOURNAL_FILE_NAME);
        let mut journal = SqliteJournal::open_namespace(
            &path,
            engine::state_journal::AUTHORING_JOURNAL_NAMESPACE,
        )
        .expect("creates raw journal");
        journal
            .commit(b"not a canonical Value")
            .expect("commits checksum-valid corrupt payload");
        drop(journal);

        let error = AuthoringStore::open(&directory)
            .err()
            .expect("corrupt entry is refused");
        assert!(
            error.to_string().contains("entry 1 does not decode"),
            "{error}"
        );
        let mut journal = SqliteJournal::open_namespace(
            &path,
            engine::state_journal::AUTHORING_JOURNAL_NAMESPACE,
        )
        .expect("prefix remains present");
        assert_eq!(
            journal
                .snapshot()
                .expect("prefix snapshots")
                .entries()
                .len(),
            1
        );
        std::fs::remove_dir_all(&directory).expect("remove test directory");
    }

    #[test]
    fn legacy_development_state_is_ignored_even_when_the_journal_is_empty() {
        let directory = test_directory("legacy-ignored");
        for name in ["authoring-state.value", "run-lifecycle.value"] {
            std::fs::write(directory.join(name), b"obsolete development state").unwrap();
        }
        for _ in 0..2 {
            let store = AuthoringStore::open(&directory).expect("fresh journal opens");
            assert_eq!(store.load().unwrap().cursor(), 0);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
