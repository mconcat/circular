
use crate::ObservationKey;
pub(crate) use crate::port::Store;
use crate::query::{
    Bound, Cursor, CursorRejection, Page, Query, QueryFingerprint, ResolvedBound, ScanStart,
    StoreError, SurfaceSequence,
};
use crate::record::{BoundaryKey, Class, ClassKey, EncodedRow, Record, StoreSchema, StructureKey};
use circular_core::{Stamp, Tick};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;

use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppendBatch<S: StoreSchema> {
    rows: Box<[EncodedRow<S>]>,
    commits: Vec<u64>,
}

impl<S: StoreSchema> AppendBatch<S> {
    #[must_use]
    pub fn at_commit(mut self, commit: u64) -> Self {
        self.commits = vec![commit; self.rows.len()];
        self
    }

    pub fn with_commits(mut self, commits: Vec<u64>) -> Result<Self, AppendBatchError> {
        if commits.len() != self.rows.len() {
            return Err(AppendBatchError::Empty);
        }
        self.commits = commits;
        Ok(self)
    }

    pub fn try_new(records: Vec<Record<S>>) -> Result<Self, AppendBatchError> {
        Self::try_new_rows(records.into_iter().map(EncodedRow::new).collect())
    }

    pub fn try_new_rows(rows: Vec<EncodedRow<S>>) -> Result<Self, AppendBatchError> {
        if rows.is_empty() {
            Err(AppendBatchError::Empty)
        } else {
            Ok(Self {
                rows: rows.into_boxed_slice(),
                commits: Vec::new(),
            })
        }
    }

    pub fn records(&self) -> impl ExactSizeIterator<Item = &Record<S>> + Clone {
        self.rows.iter().map(EncodedRow::record)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppendBatchError {
    Empty,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordRef<S: StoreSchema> {
    class: Class,
    key: ClassKey<S>,
}

impl<S: StoreSchema> RecordRef<S> {
    fn from_record(record: &Record<S>) -> Self {
        Self {
            class: record.header().class(),
            key: record.header().key().clone(),
        }
    }

    #[must_use]
    pub const fn class(&self) -> Class {
        self.class
    }

    #[must_use]
    pub const fn key(&self) -> &ClassKey<S> {
        &self.key
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Receipt<S: StoreSchema>(Box<[RecordRef<S>]>);

impl<S: StoreSchema> Receipt<S> {
    fn new<'a>(records: impl ExactSizeIterator<Item = &'a Record<S>>) -> Self
    where
        S: 'a,
    {
        debug_assert!(records.len() != 0);
        Self(
            records
                .map(RecordRef::from_record)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
        )
    }

    #[must_use]
    pub fn for_batch(batch: &AppendBatch<S>) -> Self {
        Self::new(batch.records())
    }

    #[must_use]
    pub fn refs(&self) -> &[RecordRef<S>] {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppendRejectReason<S: StoreSchema> {
    ConflictingRecord(ClassKey<S>),
    MissingManifest,
    BoundaryAtOrBeforeSealedTick {
        sealed_through: Tick,
        attempted: circular_core::Stamp<S::Producer>,
    },
    StampDidNotIncrease {
        previous: circular_core::Stamp<S::Producer>,
        attempted: circular_core::Stamp<S::Producer>,
    },
    NotAppendableOrigin(ClassKey<S>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppendFailure<S: StoreSchema> {
    Rejected {
        index: usize,
        reason: AppendRejectReason<S>,
    },
    StorageUnavailable {
        reason: Box<str>,
    },
    IntegrityLost {
        reason: Box<str>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppendResult<S: StoreSchema> {
    Committed(Receipt<S>),
    Failed(AppendFailure<S>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PagePolicy {
    default: NonZeroUsize,
    maximum: NonZeroUsize,
}

impl PagePolicy {
    pub const fn new(
        default: NonZeroUsize,
        maximum: NonZeroUsize,
    ) -> Result<Self, PagePolicyError> {
        if default.get() > maximum.get() {
            Err(PagePolicyError::DefaultExceedsMaximum)
        } else {
            Ok(Self { default, maximum })
        }
    }

    #[must_use]
    pub const fn default(self) -> NonZeroUsize {
        self.default
    }

    #[must_use]
    pub const fn maximum(self) -> NonZeroUsize {
        self.maximum
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PagePolicyError {
    DefaultExceedsMaximum,
}

#[derive(Clone, Debug)]
pub struct MemoryStore<S: StoreSchema> {
    records: crate::RecordSegments<S>,
    checkpoint_facts: crate::RecordSegments<S>,
    applied_through: u64,
    keys: HashMap<u64, KeySlots>,
    indexed: bool,
    columns: HashMap<S::Producer, Stamp<S::Producer>>,
    surface: crate::surface_index::SurfaceIndex<S>,
    store_generation: Arc<()>,
    sealed_through: Option<SurfaceSequence>,
    page_policy: PagePolicy,
}

#[derive(Clone, Debug)]
enum KeySlots {
    One(usize),
    Many(Vec<usize>),
}

impl KeySlots {
    fn push(&mut self, index: usize) {
        match self {
            Self::One(first) => *self = Self::Many(vec![*first, index]),
            Self::Many(all) => all.push(index),
        }
    }

    fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        match self {
            Self::One(first) => std::slice::from_ref(first).iter().copied(),
            Self::Many(all) => all.iter().copied(),
        }
    }
}

fn key_hash<S: StoreSchema>(key: &ClassKey<S>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

impl<S: StoreSchema> QueryFingerprint<S> {
    fn continuation_upto(&self, prior: &Self) -> Option<ResolvedBound> {
        match (self, prior) {
            (Self::Boundary { .. }, Self::Boundary { upto }) => Some(upto.clone()),
            (
                Self::Display { key, .. },
                Self::Display {
                    key: prior_key,
                    upto,
                },
            ) if key == prior_key => Some(upto.clone()),
            (
                Self::Observation { scope, buckets, .. },
                Self::Observation {
                    scope: prior_scope,
                    buckets: prior_buckets,
                    upto,
                },
            ) if scope == prior_scope && buckets == prior_buckets => Some(upto.clone()),
            (Self::Structure { .. }, Self::Structure { upto }) => Some(upto.clone()),
            (
                Self::Arrival { actor, .. },
                Self::Arrival {
                    actor: prior_actor,
                    upto,
                },
            ) if actor == prior_actor => Some(upto.clone()),
            (Self::ExternalArrivalScan { .. }, Self::ExternalArrivalScan { upto }) => {
                Some(upto.clone())
            }
            _ => None,
        }
    }

    fn replace_upto(&mut self, resolved: ResolvedBound) {
        let upto = match self {
            Self::Boundary { upto, .. }
            | Self::Display { upto, .. }
            | Self::Observation { upto, .. }
            | Self::Structure { upto, .. }
            | Self::Arrival { upto, .. }
            | Self::ExternalArrivalScan { upto, .. } => upto,
        };
        *upto = resolved;
    }
}

impl<S: StoreSchema> MemoryStore<S> {
    #[must_use]
    pub fn new(page_policy: PagePolicy) -> Self {
        Self {
            records: crate::RecordSegments::default(),
            checkpoint_facts: crate::RecordSegments::default(),
            applied_through: 0,
            keys: HashMap::new(),
            indexed: true,
            columns: HashMap::new(),
            surface: crate::surface_index::SurfaceIndex::default(),
            store_generation: Arc::new(()),
            sealed_through: None,
            page_policy,
        }
    }

    /// Publish the sealed segment tree, without copying the writer's indices.
    /// A read prefix never mutates them. A caller deliberately making it writable
    /// reconstructs its indices once before the first write.
    pub fn published_prefix(&self) -> Self {
        Self {
            records: self.records.clone(),
            checkpoint_facts: self.checkpoint_facts.clone(),
            applied_through: self.applied_through,
            keys: HashMap::new(),
            indexed: false,
            columns: HashMap::new(),
            surface: self.surface.clone(),
            store_generation: self.store_generation.clone(),
            sealed_through: self.sealed_through,
            page_policy: self.page_policy,
        }
    }

    fn index_record(&mut self, index: usize, record: &Record<S>) {
        match self.keys.entry(key_hash(record.header().key())) {
            std::collections::hash_map::Entry::Occupied(mut slots) => slots.get_mut().push(index),
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(KeySlots::One(index));
            }
        }
        self.surface.observe_appended(index, record);
        if boundary_advances_column(record) {
            let at = record.header().at();
            self.columns.insert(at.producer().clone(), at.clone());
        }
    }

    fn reindex(&mut self) {
        self.indexed = true;
        self.keys.clear();
        self.columns.clear();
        self.surface.clear();
        let records = std::mem::take(&mut self.records);
        for (index, record) in records.iter().enumerate() {
            self.index_record(index, &record);
        }
        self.records = records;
        self.index_query_positions(0..self.sealed_records().len());
    }

    fn position_of_key(&self, key: &ClassKey<S>) -> Option<usize> {
        if !self.indexed {
            return self
                .records
                .iter()
                .position(|record| record.header().key() == key);
        }
        self.keys
            .get(&key_hash(key))?
            .iter()
            .find(|index| self.records.row(*index).header().key() == key)
    }

    #[must_use]
    pub fn records(&self) -> &crate::RecordSegments<S> {
        &self.records
    }

    #[must_use]
    pub fn record_for_key(&self, key: &ClassKey<S>) -> Option<std::borrow::Cow<'_, Record<S>>> {
        self.position_of_key(key)
            .map(|index| self.records.row(index))
    }

    pub fn checkpoint_facts(&self) -> &crate::RecordSegments<S> {
        &self.checkpoint_facts
    }

    pub fn push_checkpoint_fact(&mut self, record: Record<S>, commit: u64) {
        self.checkpoint_facts.push_committed(record, commit);
    }

    #[must_use]
    pub const fn applied_through(&self) -> u64 {
        self.applied_through
    }

    pub fn note_commit(&mut self, commit: u64) {
        self.applied_through = self.applied_through.max(commit);
    }

    /// Borrow exactly the committed surface prefix admitted by EndOfSealed.
    /// The borrow cannot outlive a mutation and adds no lock or second cursor.
    #[must_use]
    pub fn sealed_records(&self) -> crate::RecordSlice<'_, S> {
        let end = self
            .sealed_through
            .map_or(0, |through| {
                usize::try_from(through.get()).unwrap_or(usize::MAX)
            })
            .min(self.records.len());
        self.records.slice(..end)
    }

    #[must_use]
    pub fn surface_mark(&self) -> SurfaceSequence {
        SurfaceSequence::from_index(self.records.len())
    }

    pub fn live_rows_since(
        &self,
        mark: SurfaceSequence,
    ) -> impl Iterator<Item = (SurfaceSequence, std::borrow::Cow<'_, Record<S>>)> {
        let from = (mark.get().saturating_sub(1)) as usize;
        self.records
            .slice(from.min(self.records.len())..)
            .iter()
            .enumerate()
            .filter_map(move |(offset, record)| {
                let surface = SurfaceSequence::from_index(from.checked_add(offset)?);
                crate::live_feed::LiveFrame::of(&record)
                    .is_some()
                    .then_some((surface, record))
            })
    }

    pub fn seal_through(&mut self, through: SurfaceSequence) {
        let start = self.sealed_records().len();
        self.sealed_through = Some(self.sealed_through.map_or(through, |old| old.max(through)));
        let end = self.sealed_records().len();
        self.index_query_positions(start..end);
    }

    fn index_query_positions(&mut self, sealed: std::ops::Range<usize>) {
        let records = self.records.clone();
        self.surface
            .observe_sealed(sealed.map(|position| (position, records.row(position))));
    }

    /// Sealed per-actor arrival bounds. The ordinal is an exclusive prefix bound.
    pub fn arrival_coordinate_bounds(&self) -> impl Iterator<Item = (&S::ActorId, u64)> {
        self.surface.arrival_coordinate_bounds()
    }

    pub fn arrival_positions_since(
        &self,
        actor: &S::ActorId,
        from: u64,
    ) -> impl Iterator<Item = usize> + '_ {
        self.surface.arrival_positions_since(actor, from)
    }

    pub fn emission_positions_between(
        &self,
        producer: &S::Producer,
        from: u64,
        end: u64,
    ) -> impl Iterator<Item = usize> + '_ {
        self.surface.emission_positions_between(producer, from, end)
    }

    #[must_use]
    pub fn emission_first_cause(&self, producer: &S::Producer) -> Option<u64> {
        self.surface.emission_first_cause(producer)
    }

    pub fn note_prior_manifest(&mut self) {
        self.surface.note_manifest();
    }

    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn published_after(&self, through: u64) -> Self {
        let mut tail = Self::new(self.page_policy);
        tail.note_prior_manifest();
        let (mut records, mut commits) = (Vec::new(), Vec::new());
        for row in self.records.rows() {
            if row.commit() > through {
                records.push(row.record().into_owned());
                commits.push(row.commit());
            }
        }
        if !records.is_empty() {
            let batch = AppendBatch::try_new(records)
                .and_then(|batch| batch.with_commits(commits))
                .expect("a nonempty committed tail");
            assert!(matches!(tail.append(batch), AppendResult::Committed(_)));
        }
        for row in self.checkpoint_facts.rows() {
            if row.commit() > through {
                tail.push_checkpoint_fact(row.record().into_owned(), row.commit());
            }
        }
        tail.note_commit(self.applied_through);
        tail.seal_all();
        tail.published_prefix()
    }

    #[must_use]
    pub fn arrival_first_ordinal(&self, actor: &S::ActorId) -> Option<u64> {
        self.surface.arrival_first_ordinal(actor)
    }

    pub fn display_coordinate_bounds(&self) -> impl Iterator<Item = (&S::Producer, u64)> {
        self.surface.display_coordinate_bounds()
    }

    pub fn display_positions_since(
        &self,
        actor: &S::Producer,
        from: u64,
    ) -> impl Iterator<Item = usize> + '_ {
        self.surface.display_positions_since(actor, from)
    }

    pub fn recorded_display_keys(&self) -> impl Iterator<Item = &S::DisplayKey> {
        self.surface.recorded_display_keys()
    }

    /// Metadata needed to resolve a template's recorded cells without scanning arrivals.
    pub fn lifecycle_positions_of(
        &self,
        kind: &S::ObservationKindKey,
    ) -> impl Iterator<Item = usize> + '_ {
        self.surface.lifecycle_positions_of(kind)
    }

    pub fn sealed_structure_positions(&self) -> impl Iterator<Item = usize> + '_ {
        let end = self.sealed_records().len();
        self.surface.structure_positions_before(end)
    }

    pub fn seal_all(&mut self) {
        if !self.records.is_empty() {
            self.seal_through(SurfaceSequence::from_index(self.records.len() - 1));
        }
    }

    pub fn query_page(
        &self,
        query: &Query<S>,
        page_size: NonZeroUsize,
    ) -> Result<Page<S>, StoreError<S>> {
        if page_size > self.page_policy.maximum {
            return Err(StoreError::PageSizeTooLarge {
                requested: page_size.get(),
                maximum: self.page_policy.maximum.get(),
            });
        }
        match query {
            Query::BoundaryScan { start, upto } => {
                if !self.has_manifest() {
                    return Err(StoreError::MissingManifest);
                }
                self.scan(
                    start,
                    upto,
                    |upto| QueryFingerprint::Boundary { upto },
                    page_size,
                )
            }
            Query::DisplayRange { key, start, upto } => {
                if !self
                    .records
                    .iter()
                    .any(|record| {
                        matches!(record.header().key(), ClassKey::Display { key: found, .. } if found == key)
                    })
                {
                    let mut recorded = Vec::new();
                    for record in self.records.iter() {
                        if let ClassKey::Display { key: found, .. } = record.header().key()
                            && !recorded.contains(found)
                        {
                            recorded.push(found.clone());
                        }
                    }
                    return Err(StoreError::UnrecordedDisplayKey {
                        requested: key.clone(),
                        recorded: recorded.into_boxed_slice(),
                    });
                }
                self.scan(
                    start,
                    upto,
                    |upto| QueryFingerprint::Display {
                        key: key.clone(),
                        upto,
                    },
                    page_size,
                )
            }
            Query::ObservationScan {
                scope,
                buckets,
                start,
                upto,
            } => self.scan(
                start,
                upto,
                |upto| QueryFingerprint::Observation {
                    scope: scope.clone(),
                    buckets: *buckets,
                    upto,
                },
                page_size,
            ),
            Query::StructureOf { start, upto } => self.scan(
                start,
                upto,
                |upto| QueryFingerprint::Structure { upto },
                page_size,
            ),
            Query::ArrivalScan { actor, start, upto } => {
                if !self.has_manifest() {
                    return Err(StoreError::MissingManifest);
                }
                self.scan(
                    start,
                    upto,
                    |upto| QueryFingerprint::Arrival {
                        actor: actor.clone(),
                        upto,
                    },
                    page_size,
                )
            }
            Query::ExternalArrivalScan { start, upto } => {
                if !self.has_manifest() {
                    return Err(StoreError::MissingManifest);
                }
                self.scan(
                    start,
                    upto,
                    |upto| QueryFingerprint::ExternalArrivalScan { upto },
                    page_size,
                )
            }
        }
    }

    fn resolve_bound(&self, bound: &Bound) -> ResolvedBound {
        match bound {
            Bound::UpTo(upto) => ResolvedBound::UpTo(*upto),
            Bound::Before(before) => ResolvedBound::Before(*before),
            Bound::EndOfSealed => ResolvedBound::Sealed(self.sealed_through),
        }
    }

    fn scan(
        &self,
        start: &ScanStart<S>,
        upto: &Bound,
        make: impl FnOnce(ResolvedBound) -> QueryFingerprint<S>,
        page_size: NonZeroUsize,
    ) -> Result<Page<S>, StoreError<S>> {
        let mut fingerprint = make(self.resolve_bound(upto));
        if matches!(upto, Bound::EndOfSealed)
            && let ScanStart::After(cursor) = start
            && let Some(resolved) = fingerprint.continuation_upto(&cursor.query_fingerprint)
        {
            fingerprint.replace_upto(resolved);
        }
        let matched = self
            .records
            .iter()
            .enumerate()
            .filter(|(index, record)| record_matches_fingerprint(*index, record, &fingerprint))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        self.page_matched(&matched, start, fingerprint, page_size)
    }

    fn page_matched(
        &self,
        matched: &[usize],
        start: &ScanStart<S>,
        fingerprint: QueryFingerprint<S>,
        page_size: NonZeroUsize,
    ) -> Result<Page<S>, StoreError<S>> {
        let start_index = match start {
            ScanStart::Beginning => 0,
            ScanStart::Tail { count } => matched.len().saturating_sub(*count),
            ScanStart::After(cursor) => {
                self.validate_cursor(cursor, &fingerprint)?;
                cursor.position
            }
        };
        let end = start_index
            .saturating_add(page_size.get())
            .min(matched.len());
        let records = matched[start_index..end]
            .iter()
            .map(|index| self.records.row(*index).into_owned())
            .collect::<Vec<_>>();
        let next = (end < matched.len()).then(|| Cursor {
            store_generation: Arc::clone(&self.store_generation),
            query_fingerprint: fingerprint,
            position: end,
        });
        Ok(Page::new(records, next))
    }

    fn validate_cursor(
        &self,
        cursor: &Cursor<S>,
        fingerprint: &QueryFingerprint<S>,
    ) -> Result<(), StoreError<S>> {
        if !Arc::ptr_eq(&cursor.store_generation, &self.store_generation) {
            return Err(StoreError::Cursor(CursorRejection::DifferentStore));
        }
        if &cursor.query_fingerprint != fingerprint {
            return Err(StoreError::Cursor(CursorRejection::DifferentQuery));
        }
        Ok(())
    }

    fn has_manifest(&self) -> bool {
        self.surface.has_manifest()
    }

    #[must_use]
    pub fn sealed_structure_count(&self) -> usize {
        let end = self
            .sealed_through
            .map_or(0, |through| {
                usize::try_from(through.get()).unwrap_or(usize::MAX)
            })
            .min(self.records.len());
        self.surface.structure_count_before(end)
    }

    fn validate_append(
        &self,
        rows: &[EncodedRow<S>],
    ) -> Result<Vec<usize>, (usize, AppendRejectReason<S>)> {
        let mut additions = Vec::<usize>::new();
        let mut manifest = false;
        for record in rows.iter().map(EncodedRow::record) {
            if let ClassKey::Structure(StructureKey::Manifest) = record.header().key() {
                manifest = true;
            }
        }
        let mut added_keys = HashMap::<u64, KeySlots>::new();
        let mut added_columns = HashMap::<S::Producer, Stamp<S::Producer>>::new();

        for (index, record) in rows.iter().map(EncodedRow::record).enumerate() {
            let key = record.header().key();
            if record.header().position().stamp().is_none() {
                return Err((index, AppendRejectReason::NotAppendableOrigin(key.clone())));
            }
            let existing = self
                .position_of_key(key)
                .map(|found| self.records.row(found))
                .or_else(|| {
                    added_keys
                        .get(&key_hash(key))?
                        .iter()
                        .find(|found| rows[*found].record().header().key() == key)
                        .map(|found| std::borrow::Cow::Borrowed(rows[found].record()))
                });
            if let Some(existing) = existing {
                if &*existing != record {
                    return Err((index, AppendRejectReason::ConflictingRecord(key.clone())));
                }
                continue;
            }

            if matches!(record.header().class(), Class::Boundary | Class::Structure) {
                if !self.surface.has_manifest()
                    && !manifest
                    && !matches!(key, ClassKey::Structure(StructureKey::Manifest))
                {
                    return Err((index, AppendRejectReason::MissingManifest));
                }
            }

            if boundary_advances_column(record) {
                let at = record.header().at();
                let column = at.producer().clone();
                if let Some(previous) = added_columns
                    .get(&column)
                    .or_else(|| self.columns.get(&column))
                    && !at.timeline_cmp(previous).is_gt()
                {
                    return Err((
                        index,
                        AppendRejectReason::StampDidNotIncrease {
                            previous: previous.clone(),
                            attempted: at.clone(),
                        },
                    ));
                }
                added_columns.insert(column, at.clone());
            }
            match added_keys.entry(key_hash(key)) {
                std::collections::hash_map::Entry::Occupied(mut slots) => {
                    slots.get_mut().push(index);
                }
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(KeySlots::One(index));
                }
            }
            additions.push(index);
        }
        Ok(additions)
    }
}

impl<S: StoreSchema> Store<S> for MemoryStore<S> {
    fn append(&mut self, batch: AppendBatch<S>) -> AppendResult<S> {
        if !self.indexed {
            self.reindex();
        }
        let receipt = Receipt::for_batch(&batch);
        let commits = batch.commits;
        match self.validate_append(&batch.rows) {
            Ok(additions) => {
                let mut selected = additions.into_iter().peekable();
                let mut kept_commits = Vec::with_capacity(commits.len());
                let additions = batch
                    .rows
                    .into_vec()
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, row)| {
                        if selected.peek() == Some(&index) {
                            selected.next();
                            if let Some(commit) = commits.get(index) {
                                kept_commits.push(*commit);
                            }
                            Some(row)
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                for (offset, row) in additions.iter().enumerate() {
                    self.index_record(self.records.len() + offset, row.record());
                }
                if let Some(last) = commits.iter().copied().max() {
                    self.note_commit(last);
                }
                self.records.append_committed(additions, kept_commits);
                AppendResult::Committed(receipt)
            }
            Err((index, reason)) => AppendResult::Failed(AppendFailure::Rejected { index, reason }),
        }
    }

    fn query(&self, query: &Query<S>) -> Result<Page<S>, StoreError<S>> {
        self.query_page(query, self.page_policy.default)
    }
}

fn within_bound(index: usize, bound: &ResolvedBound) -> bool {
    let sequence = SurfaceSequence::from_index(index);
    match bound {
        ResolvedBound::UpTo(upto) => sequence <= *upto,
        ResolvedBound::Before(before) => sequence < *before,
        ResolvedBound::Sealed(Some(sealed)) => sequence <= *sealed,
        ResolvedBound::Sealed(None) => false,
    }
}

fn record_matches_fingerprint<S: StoreSchema>(
    index: usize,
    record: &Record<S>,
    fingerprint: &QueryFingerprint<S>,
) -> bool {
    match fingerprint {
        QueryFingerprint::Boundary { upto } => {
            record.header().class() == Class::Boundary && within_bound(index, upto)
        }
        QueryFingerprint::Display { key, upto } => {
            matches!(record.header().key(), ClassKey::Display { key: found, .. } if found == key)
                && within_bound(index, upto)
        }
        QueryFingerprint::Arrival { actor, upto } => {
            matches!(
                record.header().key(),
                ClassKey::Boundary(BoundaryKey::Arrival {
                    actor: found_actor,
                    ..
                }) if found_actor == actor
            ) && within_bound(index, upto)
        }
        QueryFingerprint::ExternalArrivalScan { upto } => {
            matches!(
                record.header().key(),
                ClassKey::Boundary(BoundaryKey::Arrival {
                    origin,
                    ..
                }) if origin.is_external_nondeterministic()
            ) && within_bound(index, upto)
        }
        QueryFingerprint::Observation {
            scope,
            buckets,
            upto,
        } => observation_matches(record, scope, buckets.as_ref()) && within_bound(index, upto),
        QueryFingerprint::Structure { upto } => {
            record.header().class() == Class::Structure && within_bound(index, upto)
        }
    }
}

fn observation_matches<S: StoreSchema>(
    record: &Record<S>,
    scope: &crate::query::ObservationScope<S>,
    buckets: Option<&crate::query::BucketRange>,
) -> bool {
    let ClassKey::Observation(key) = record.header().key() else {
        return false;
    };
    if let Some(range) = buckets
        && !range.contains(key.bucket())
    {
        return false;
    }
    match scope {
        crate::query::ObservationScope::All => true,
        crate::query::ObservationScope::Stream => matches!(
            record.header().key(),
            ClassKey::Observation(
                ObservationKey::StreamItem(..) | ObservationKey::CheckpointItem(..)
            )
        ),
        crate::query::ObservationScope::Origin(origin) => record.header().origin() == origin,
        crate::query::ObservationScope::Kind(kind) => key.item().kind() == kind,
    }
}

fn boundary_advances_column<S: StoreSchema>(record: &Record<S>) -> bool {
    matches!(record, Record::Boundary(boundary) if !matches!(
        boundary.fact(),
        crate::record::BoundaryFact::EmissionBody { .. }
            | crate::record::BoundaryFact::ScheduleReservation { .. }
            | crate::record::BoundaryFact::Admission { .. }
    ))
}

#[cfg(test)]
mod checkpoint_tests {
    use super::*;
    use crate::{
        BoundaryRecord, FailureParams, ManifestGroups, ManifestSchema, ObservationBucket,
        ObservationItemKey, ObservationRecord, PlacementParams, Query, RecordOrigin,
        RevisionContext, RevisionStart, RunInputs, RunManifest, StructureRecord, TimeParams,
    };
    use circular_core::{
        NonZeroTicks, ProducerIdentity, Sequence, TicksPerSecond, TimeSourceKind, TimeSourcePlan,
    };

    use circular_testkit::types::TestRun;

    #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
    struct TestProducer(u8);

    impl ProducerIdentity for TestProducer {
        type EventProducer = Self;

        fn from_event_producer(producer: Self::EventProducer) -> Self {
            producer
        }
    }

    #[derive(Clone, Debug, Eq, Hash, PartialEq)]
    struct TestSchema;

    impl ManifestSchema for TestSchema {
        type Stream = TestRun;
        type Producer = TestProducer;
        type Placement = ();
        type Failure = ();
        type Versions = ();
        type RevisionId = u8;
        type AuthoringCut = u8;
        type ScopeId = u8;
        type GrantSet = ();
        type InputValue = ();

        type CadencePolicy = ();
        type TickOrigin = ();
    }

    impl StoreSchema for TestSchema {
        type Incarnation = u64;
        type ActorId = u64;
        type EdgeId = u64;
        type EffectId = u64;
        type TimerId = u64;
        type DisplayKey = u64;
        type ObservationKindKey = u8;
        type ObservationIdentityKey = u16;
        type ExternalOrigin = ();
        type EffectTerm = ();
        type EffectOutcome = ();
        type GraphRevision = ();
        type DisplayPayload = ();
        type ObservationPayload = u16;
    }

    fn stamp_with_sequence(tick: u64, sequence: u64) -> circular_core::Stamp<TestProducer> {
        circular_core::Stamp::from_event_producer(
            Tick::new(tick),
            TestProducer(1),
            Sequence::new(sequence).expect("small fixture sequence"),
            circular_core::RevisionEpochId::new(1).expect("first revision"),
        )
    }

    fn stamp(tick: u64) -> circular_core::Stamp<TestProducer> {
        stamp_with_sequence(tick, Sequence::FIRST.get())
    }

    fn store() -> MemoryStore<TestSchema> {
        let page = NonZeroUsize::new(16).expect("the constant is not zero");
        MemoryStore::new(
            PagePolicy::new(page, page).expect("the same default and ceiling are valid"),
        )
    }

    fn assert_index_matches_a_linear_scan(store: &MemoryStore<TestSchema>) {
        for record in store.records() {
            let key = record.header().key();
            let scanned = store
                .records()
                .iter()
                .position(|found| found.header().key() == key);
            assert_eq!(
                store.position_of_key(key),
                scanned,
                "the key index differs from a linear scan"
            );
        }
        let mut columns = HashMap::new();
        let mut manifest = false;
        for record in store.records() {
            if let ClassKey::Structure(StructureKey::Manifest) = record.header().key() {
                manifest = true;
            }
            if boundary_advances_column(&record) {
                let at = record.header().at();
                columns.insert(at.producer().clone(), at.clone());
            }
        }
        assert_eq!(
            store.columns, columns,
            "the end of the sequence differs from a linear scan"
        );
        assert_eq!(
            store.surface.has_manifest(),
            manifest,
            "the manifest set differs from a linear scan"
        );
        assert_structure_count_matches_a_linear_scan(store);
    }

    fn assert_structure_count_matches_a_linear_scan(store: &MemoryStore<TestSchema>) {
        let scanned = store
            .sealed_records()
            .iter()
            .filter(|record| record.header().class() == Class::Structure)
            .count();
        assert_eq!(store.sealed_structure_count(), scanned);
    }

    #[test]
    fn the_folded_structure_count_follows_the_seal_and_never_rescans() {
        let mut store = store();
        let run = TestRun(7);
        assert_eq!(store.sealed_structure_count(), 0);
        assert!(matches!(
            store.append(
                AppendBatch::try_new(vec![
                    manifest_record(run),
                    boundary_record(1, 1),
                    Record::Structure(StructureRecord::graph_revision(9, stamp(2), ())),
                ])
                .unwrap()
            ),
            AppendResult::Committed(_)
        ));
        assert_eq!(store.sealed_structure_count(), 0);
        store.seal_through(SurfaceSequence::from_index(0));
        assert_eq!(store.sealed_structure_count(), 1);
        store.seal_all();
        assert_eq!(store.sealed_structure_count(), 2);
        assert_structure_count_matches_a_linear_scan(&store);
    }

    #[test]
    fn the_key_index_answers_exactly_what_a_linear_scan_answers() {
        let mut store = store();
        let run = TestRun(1);
        let manifest = manifest_record(run);
        let first = boundary_record(1, 1);
        let second = boundary_record(2, 2);
        let third = boundary_record(3, 3);
        assert!(matches!(
            store.append(
                AppendBatch::try_new(vec![manifest.clone(), first.clone(), second.clone()])
                    .unwrap()
            ),
            AppendResult::Committed(_)
        ));
        assert!(matches!(
            store.append(AppendBatch::try_new(vec![third.clone()]).unwrap()),
            AppendResult::Committed(_)
        ));
        assert_index_matches_a_linear_scan(&store);

        let before = store.records().len();
        assert!(matches!(
            store.append(AppendBatch::try_new(vec![second.clone()]).unwrap()),
            AppendResult::Committed(_)
        ));
        assert_eq!(store.records().len(), before);
        assert_index_matches_a_linear_scan(&store);

        let Record::Boundary(same_key) = &first else {
            unreachable!("fixture boundary")
        };
        let conflicting = Record::Boundary(BoundaryRecord::arrival(
            1,
            stamp_with_sequence(9, 9),
            RecordOrigin::Stream,
            crate::record::ArrivalOrigin::TimerFire { timer: 1_001 },
            crate::ArrivalBody::Owned(circular_core::EncodedPayload::new(
                circular_core::PayloadVersionTag::FIRST,
                &[9],
            )),
            circular_core::ArrivalIndex::new(9),
            Box::new([]),
            circular_core::RecordedInstant::from_millis(9),
            None,
        ));
        assert_eq!(conflicting.header().key(), same_key.header().key());
        assert!(matches!(
            store.append(AppendBatch::try_new(vec![conflicting]).unwrap()),
            AppendResult::Failed(AppendFailure::Rejected {
                index: 0,
                reason: AppendRejectReason::ConflictingRecord(_)
            })
        ));

        assert!(matches!(
            store.append(AppendBatch::try_new(vec![boundary_record(1, 5)]).unwrap()),
            AppendResult::Failed(AppendFailure::Rejected {
                index: 0,
                reason: AppendRejectReason::StampDidNotIncrease { .. }
            })
        ));
        assert_index_matches_a_linear_scan(&store);

        assert!(matches!(
            MemoryStore::new(store.page_policy)
                .append(AppendBatch::try_new(vec![boundary_record(1, 1)]).unwrap()),
            AppendResult::Failed(AppendFailure::Rejected {
                index: 0,
                reason: AppendRejectReason::MissingManifest
            })
        ));
    }

    #[test]
    fn sealed_records_borrow_excludes_unsealed_suffix_and_matches_query_bound() {
        let mut store = store();
        assert!(store.sealed_records().is_empty());
        let run = TestRun(1);
        let manifest = manifest_record(run);
        let first = boundary_record(1, 1);
        let second = boundary_record(2, 2);
        assert!(matches!(
            store.append(AppendBatch::try_new(vec![manifest.clone(), first.clone()]).unwrap()),
            AppendResult::Committed(_)
        ));
        assert!(store.sealed_records().is_empty());
        store.seal_all();
        assert!(matches!(
            store.append(AppendBatch::try_new(vec![second.clone()]).unwrap()),
            AppendResult::Committed(_)
        ));
        assert_eq!(store.sealed_records(), &[manifest.clone(), first.clone()]);
        let page = store
            .query(&Query::ArrivalScan {
                actor: 1,
                start: ScanStart::Beginning,
                upto: Bound::EndOfSealed,
            })
            .unwrap();
        assert_eq!(page.records(), &[first.clone()]);
        store.seal_all();
        assert_eq!(store.sealed_records(), &[manifest, first, second]);
    }

    fn manifest_record(run: TestRun) -> Record<TestSchema> {
        let revision = RevisionContext::try_new(RevisionStart::Fresh(1), Vec::new())
            .expect("an empty grant sequence is valid");
        let groups = ManifestGroups::new(
            TimeParams::new(
                TicksPerSecond::new(TicksPerSecond::MIN).expect("the minimal resolution is valid"),
                NonZeroTicks::new(1).expect("a one-tick cadence is valid"),
                (),
                TimeSourcePlan::Single(TimeSourceKind::Manual),
                (),
            ),
            PlacementParams::from_validated(()),
            FailureParams::from_validated(()),
            (),
            revision,
            RunInputs::from_primary_data(()),
        );
        Record::Structure(StructureRecord::manifest(
            stamp(0),
            RunManifest::new(run, groups),
        ))
    }

    fn boundary_record(tick: u64, sequence: u64) -> Record<TestSchema> {
        Record::Boundary(BoundaryRecord::arrival(
            1,
            stamp_with_sequence(tick, sequence),
            RecordOrigin::Stream,
            crate::record::ArrivalOrigin::TimerFire {
                timer: tick.saturating_mul(1_000).saturating_add(sequence),
            },
            crate::ArrivalBody::Owned(circular_core::EncodedPayload::new(
                circular_core::PayloadVersionTag::FIRST,
                &sequence.to_be_bytes(),
            )),
            circular_core::ArrivalIndex::new(sequence),
            Box::new([]),
            circular_core::RecordedInstant::from_millis(tick),
            None,
        ))
    }

    fn observation_item(kind: u8, identity: u16) -> ObservationItemKey<TestSchema> {
        ObservationItemKey::new(kind, identity)
    }

    #[test]
    fn an_external_arrival_scan_selects_only_external_injects() {
        let mut store = store();
        let run = TestRun(1);
        let arrival = |actor: u64, tick: u64, origin| {
            Record::Boundary(crate::record::BoundaryRecord::arrival(
                actor,
                stamp_with_sequence(tick, tick),
                RecordOrigin::Actor(actor),
                origin,
                crate::ArrivalBody::Owned(circular_core::EncodedPayload::new(
                    circular_core::PayloadVersionTag::FIRST,
                    b"same-payload",
                )),
                circular_core::ArrivalIndex::new(tick),
                Box::new([]),
                circular_core::RecordedInstant::from_millis(tick),
                None,
            ))
        };
        let records = vec![
            manifest_record(run),
            arrival(
                7,
                1,
                crate::record::ArrivalOrigin::ExternalInject { origin: () },
            ),
            arrival(7, 2, crate::record::ArrivalOrigin::TimerFire { timer: 2 }),
            arrival(
                8,
                3,
                crate::record::ArrivalOrigin::ExternalInject { origin: () },
            ),
        ];
        assert!(matches!(
            store.append(AppendBatch::try_new(records).unwrap()),
            AppendResult::Committed(_)
        ));
        store.seal_all();

        let page = store
            .query(&Query::ExternalArrivalScan {
                start: ScanStart::Beginning,
                upto: Bound::EndOfSealed,
            })
            .unwrap();
        assert_eq!(page.records().len(), 2);
        assert!(page.records().iter().all(|record| matches!(
            record.header().key(),
            ClassKey::Boundary(BoundaryKey::Arrival { origin, .. })
                if origin.is_external_nondeterministic()
        )));
    }

    #[test]
    fn an_observation_cursor_does_not_continue_into_a_different_window() {
        use crate::query::{BucketRange, CursorRejection, ObservationScope};

        let page = NonZeroUsize::new(1).expect("the constant is not zero");
        let mut store =
            MemoryStore::<TestSchema>::new(PagePolicy::new(page, page).expect("same value"));
        let run = TestRun(1);
        let at = |millis: u64, identity: u16| {
            Record::Observation(ObservationRecord::accounting(
                stamp_with_sequence(millis, millis),
                ObservationBucket::from_millis(millis),
                RecordOrigin::Stream,
                observation_item(1, identity),
                1,
            ))
        };
        assert!(matches!(
            store.append(
                AppendBatch::try_new(vec![manifest_record(run), at(100, 1), at(200, 2)])
                    .expect("not empty")
            ),
            AppendResult::Committed(_)
        ));
        store.seal_all();

        let window = |from: u64, through: u64| {
            BucketRange::try_new(
                ObservationBucket::from_millis(from),
                ObservationBucket::from_millis(through),
            )
            .expect("normal window")
        };
        let first = store
            .query(&Query::ObservationScan {
                scope: ObservationScope::Stream,
                buckets: Some(window(100, 200)),
                start: ScanStart::Beginning,
                upto: Bound::EndOfSealed,
            })
            .expect("observation lookup");
        let cursor = first
            .next()
            .cloned()
            .expect("with two entries a cursor remains");

        let refused = store.query(&Query::ObservationScan {
            scope: ObservationScope::Stream,
            buckets: Some(window(100, 100)),
            start: ScanStart::After(cursor.clone()),
            upto: Bound::EndOfSealed,
        });
        assert!(matches!(
            refused,
            Err(StoreError::Cursor(CursorRejection::DifferentQuery))
        ));

        let continued = store
            .query(&Query::ObservationScan {
                scope: ObservationScope::Stream,
                buckets: Some(window(100, 200)),
                start: ScanStart::After(cursor),
                upto: Bound::EndOfSealed,
            })
            .expect("the same window continues");
        assert_eq!(continued.records().len(), 1);
    }

    #[test]
    fn a_cursor_does_not_continue_across_an_inclusive_and_an_exclusive_bound() {
        use crate::query::{CursorRejection, ObservationScope};

        let page = NonZeroUsize::new(1).expect("the constant is not zero");
        let mut store =
            MemoryStore::<TestSchema>::new(PagePolicy::new(page, page).expect("same value"));
        let run = TestRun(1);
        let failure = |tick: u64, identity: u16| {
            Record::Observation(ObservationRecord::dead_letter(
                stamp_with_sequence(tick, tick),
                ObservationBucket::from_millis(tick),
                RecordOrigin::Stream,
                observation_item(9, identity),
                identity,
            ))
        };
        assert!(matches!(
            store.append(
                AppendBatch::try_new(vec![manifest_record(run), failure(1, 1), failure(2, 2)])
                    .expect("not empty")
            ),
            AppendResult::Committed(_)
        ));
        store.seal_all();

        let mark = store.surface_mark();
        let first = store
            .query(&Query::ObservationScan {
                scope: ObservationScope::Kind(9),
                buckets: None,
                start: ScanStart::Beginning,
                upto: Bound::Before(mark),
            })
            .expect("first page with an exclusive ceiling");
        let cursor = first.next().cloned().expect("with two a cursor remains");

        let refused = store.query(&Query::ObservationScan {
            scope: ObservationScope::Kind(9),
            buckets: None,
            start: ScanStart::After(cursor.clone()),
            upto: Bound::UpTo(mark),
        });
        assert!(
            matches!(
                refused,
                Err(StoreError::Cursor(CursorRejection::DifferentQuery))
            ),
            "continuing with an inclusive ceiling passed: {refused:?}"
        );

        let continued = store
            .query(&Query::ObservationScan {
                scope: ObservationScope::Kind(9),
                buckets: None,
                start: ScanStart::After(cursor),
                upto: Bound::Before(mark),
            })
            .expect("the same ceiling continues");
        assert_eq!(continued.records().len(), 1);
    }

    #[test]
    fn nothing_flows_for_a_refused_batch_or_a_folded_retry() {
        let mut store = store();
        let run = TestRun(1);
        let display = Record::Display(crate::record::DisplayRecord::new(
            stamp(1),
            RecordOrigin::Stream,
            42,
            (),
        ));
        assert!(matches!(
            store.append(
                AppendBatch::try_new(vec![manifest_record(run), display.clone()])
                    .expect("not empty")
            ),
            AppendResult::Committed(_)
        ));

        let mark = store.surface_mark();
        let before = store.records().len();
        assert!(matches!(
            store.append(AppendBatch::try_new(vec![display]).expect("re-arrival")),
            AppendResult::Committed(_)
        ));
        assert_eq!(
            store.records().len(),
            before,
            "folding does not grow the surface"
        );
        assert_eq!(
            store.live_rows_since(mark).count(),
            0,
            "a folded re-arrival produced a frame"
        );

        let mark = store.surface_mark();
        let conflicting = manifest_record(TestRun(9));
        assert!(matches!(
            store.append(AppendBatch::try_new(vec![conflicting]).expect("conflicting manifest")),
            AppendResult::Failed(_)
        ));
        assert_eq!(store.live_rows_since(mark).count(), 0);
    }
}
