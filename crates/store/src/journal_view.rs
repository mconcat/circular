use crate::surface_index::SurfaceIndex;
use crate::{
    ArrivalProjection, MemoryStore, ProductJournalCodec, ProductRecordCodec, ProductRowCodec,
    ProductStore, ProductTransaction, Record, RecordRow, SqliteJournal, StoreTransaction,
    StoredRow, SurfaceSequence,
};
use circular_core::{EncodedPayload, RevisionEpochId};
use std::borrow::Cow;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

type Actor = <ProductStore as crate::StoreSchema>::ActorId;
type Producer = <ProductStore as crate::manifest::ManifestSchema>::Producer;
type DisplayKey = <ProductStore as crate::StoreSchema>::DisplayKey;
type KindKey = <ProductStore as crate::StoreSchema>::ObservationKindKey;

pub const PREFIX_ORIGIN: usize = 1 << 40;

pub struct PrefixHooks {
    pub facts: Box<
        dyn Fn(
                u64,
                &StoreTransaction<ProductTransaction>,
            ) -> Result<Vec<Record<ProductStore>>, String>
            + Send
            + Sync,
    >,
}

pub struct JournalPrefix {
    path: PathBuf,
    namespace: String,
    through: u64,
    hooks: PrefixHooks,
    index: OnceLock<Arc<PrefixIndex>>,
    building: Mutex<()>,
    reader: Mutex<Option<SqliteJournal>>,
}

impl std::fmt::Debug for JournalPrefix {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JournalPrefix")
            .field("path", &self.path)
            .field("namespace", &self.namespace)
            .field("through", &self.through)
            .field("indexed", &self.index.get().is_some())
            .finish()
    }
}

struct PrefixIndex {
    rows: Vec<(u64, u32)>,
    surface: SurfaceIndex<ProductStore>,
    facts: Vec<(u64, Record<ProductStore>)>,
    data_arrivals: usize,
    data_columns: std::collections::BTreeMap<circular_runtime::ActorId, (u64, u64)>,
}

enum PrefixStored {
    Encoded(EncodedPayload),
    Decoded,
}

struct CommitRows {
    commit: u64,
    rows: Vec<(u32, Record<ProductStore>, PrefixStored)>,
}

impl JournalPrefix {
    #[must_use]
    pub fn new(
        path: impl Into<PathBuf>,
        namespace: impl Into<String>,
        through: u64,
        hooks: PrefixHooks,
    ) -> Self {
        Self {
            path: path.into(),
            namespace: namespace.into(),
            through,
            hooks,
            index: OnceLock::new(),
            building: Mutex::new(()),
            reader: Mutex::new(None),
        }
    }

    #[must_use]
    pub const fn through(&self) -> u64 {
        self.through
    }

    #[must_use]
    pub fn indexed(&self) -> bool {
        self.index.get().is_some()
    }

    pub fn row_count(&self) -> Result<usize, String> {
        Ok(self.index()?.rows.len())
    }

    pub fn data_arrivals(&self) -> Result<usize, String> {
        Ok(self.index()?.data_arrivals)
    }

    pub fn data_columns(
        &self,
    ) -> Result<std::collections::BTreeMap<circular_runtime::ActorId, (u64, u64)>, String> {
        Ok(self.index()?.data_columns.clone())
    }

    fn with_reader<T>(
        &self,
        read: impl FnOnce(&SqliteJournal) -> Result<T, String>,
    ) -> Result<T, String> {
        let mut slot = self
            .reader
            .lock()
            .map_err(|_| "journal prefix reader poisoned".to_owned())?;
        if slot.is_none() {
            *slot = Some(
                SqliteJournal::open_read_only_namespace(&self.path, &self.namespace)
                    .map_err(|error| format!("journal prefix open: {error}"))?,
            );
        }
        read(slot.as_ref().expect("reader opened above"))
    }

    fn index(&self) -> Result<Arc<PrefixIndex>, String> {
        if let Some(index) = self.index.get() {
            return Ok(index.clone());
        }
        let _building = self
            .building
            .lock()
            .map_err(|_| "journal prefix index build poisoned".to_owned())?;
        if let Some(index) = self.index.get() {
            return Ok(index.clone());
        }
        let built = Arc::new(self.build_index()?);
        let _ = self.index.set(built.clone());
        Ok(built)
    }

    fn build_index(&self) -> Result<PrefixIndex, String> {
        let mut rows = Vec::new();
        let mut surface = SurfaceIndex::default();
        let mut facts = Vec::new();
        let mut data_arrivals = 0_usize;
        let mut data_columns =
            std::collections::BTreeMap::<circular_runtime::ActorId, (u64, u64)>::new();
        let mut decoder = crate::ProductJournalReader::default();
        let visited =
            self.with_reader(|journal| {
                journal
                    .visit_namespace(0, self.through, |sequence, bytes| -> Result<(), String> {
                        let commit = sequence.get();
                        let transaction = decoder
                            .read_at(journal, commit, bytes)
                            .map_err(|error| error.to_string())?;
                        let positioned = ArrivalProjection::new()
                            .project_positions(&transaction)
                            .map_err(|rejection| {
                            ProductJournalCodec
                                .projection_error_at(commit, &transaction, rejection)
                                .to_string()
                        })?;
                        let start = rows.len();
                        for (operation, row) in &positioned {
                            let record = row.record();
                            if let Record::Boundary(boundary) = record
                                && let crate::ClassKey::Boundary(crate::BoundaryKey::Arrival {
                                    actor,
                                    ..
                                }) = boundary.header().key()
                                && let crate::BoundaryFact::Arrival {
                                    origin,
                                    observed_at,
                                    ..
                                } = boundary.fact()
                                && !matches!(
                                    origin.as_ref(),
                                    crate::ArrivalOrigin::EffectOutcome { .. }
                                )
                            {
                                data_arrivals += 1;
                                let column = data_columns.entry(actor.clone()).or_default();
                                column.0 += 1;
                                column.1 = column.1.max(observed_at.millis());
                            }
                            surface.observe_appended(rows.len(), record);
                            rows.push((commit, *operation));
                        }
                        surface.observe_sealed(positioned.iter().enumerate().map(
                            |(offset, (_, row))| (start + offset, Cow::Borrowed(row.record())),
                        ));
                        for fact in (self.hooks.facts)(commit, &transaction)? {
                            facts.push((commit, fact));
                        }
                        Ok(())
                    })
                    .map_err(|error| format!("journal prefix scan: {error}"))
            })??;
        let () = visited;
        if rows.len() > PREFIX_ORIGIN {
            return Err("journal prefix exceeds the position origin".to_owned());
        }
        Ok(PrefixIndex {
            rows,
            surface,
            facts,
            data_arrivals,
            data_columns,
        })
    }

    fn read_commit(&self, commit: u64) -> Result<CommitRows, String> {
        let transaction = self.with_reader(|journal| {
            crate::ProductJournalReader::default()
                .read_commit(journal, commit)
                .map_err(|error| error.to_string())
        })?;
        let mut positioned = ArrivalProjection::new()
            .project_positions(&transaction)
            .map_err(|rejection| {
                ProductJournalCodec
                    .projection_error_at(commit, &transaction, rejection)
                    .to_string()
            })?;
        let mut rows = Vec::with_capacity(positioned.len());
        for (operation, row) in &mut positioned {
            crate::product_journal::resolve_emitted_body(row, |producer, sequence| {
                self.with_reader(|journal| {
                    crate::product_journal::emission_body_before(
                        journal,
                        (commit, *operation),
                        producer,
                        sequence,
                    )
                })
            })?;
        }
        for (operation, row) in positioned {
            let (record, bytes) = row.into_parts();
            let stored = match bytes {
                Some(bytes) => PrefixStored::Encoded(bytes),
                None => {
                    use crate::record::RecordRowCodec as _;
                    match ProductRowCodec.encode(&record) {
                        Some(payload)
                            if ProductRowCodec.decode(&payload).as_ref() == Some(&record) =>
                        {
                            PrefixStored::Encoded(payload)
                        }
                        _ => PrefixStored::Decoded,
                    }
                }
            };
            rows.push((operation, record, stored));
        }
        Ok(CommitRows { commit, rows })
    }
}

#[derive(Clone, Debug)]
pub struct JournalView {
    tail: Arc<MemoryStore<ProductStore>>,
    prefix: Option<Arc<JournalPrefix>>,
}

pub struct ViewRow<'a> {
    position: usize,
    inner: ViewRowInner<'a>,
}

enum ViewRowInner<'a> {
    Tail(RecordRow<'a, ProductStore>),
    Prefix {
        commit: u64,
        record: Record<ProductStore>,
        stored: PrefixStored,
    },
}

impl<'a> ViewRow<'a> {
    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }

    #[must_use]
    pub fn commit(&self) -> u64 {
        match &self.inner {
            ViewRowInner::Tail(row) => row.commit(),
            ViewRowInner::Prefix { commit, .. } => *commit,
        }
    }

    #[must_use]
    pub fn stored(&self) -> StoredRow<'_, ProductStore> {
        match &self.inner {
            ViewRowInner::Tail(row) => row.stored(),
            ViewRowInner::Prefix {
                stored: PrefixStored::Encoded(payload),
                ..
            } => StoredRow::Encoded(payload),
            ViewRowInner::Prefix {
                record,
                stored: PrefixStored::Decoded,
                ..
            } => StoredRow::Decoded(record),
        }
    }

    #[must_use]
    pub fn record(&self) -> Cow<'_, Record<ProductStore>> {
        match &self.inner {
            ViewRowInner::Tail(row) => row.record(),
            ViewRowInner::Prefix { record, .. } => Cow::Borrowed(record),
        }
    }

    #[must_use]
    pub fn into_record(self) -> Cow<'a, Record<ProductStore>> {
        match self.inner {
            ViewRowInner::Tail(row) => row.record(),
            ViewRowInner::Prefix { record, .. } => Cow::Owned(record),
        }
    }
}

pub struct ViewRows<'a> {
    view: &'a JournalView,
    next: usize,
    end: usize,
    commit: Option<CommitRows>,
    tail: Option<crate::RowIter<'a, ProductStore>>,
}

impl<'a> Iterator for ViewRows<'a> {
    type Item = Result<ViewRow<'a>, String>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.end {
            return None;
        }
        let position = self.next;
        self.next += 1;
        let origin = self.view.origin();
        if position < origin {
            return Some(self.view.row_at(position, &mut self.commit));
        }
        if self.tail.is_none() {
            let records = self.view.tail.records();
            self.tail = Some(
                records
                    .slice(position - origin..(self.end - origin).min(records.len()))
                    .rows(),
            );
        }
        let row = self.tail.as_mut().and_then(Iterator::next)?;
        Some(Ok(ViewRow {
            position,
            inner: ViewRowInner::Tail(row),
        }))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.end.saturating_sub(self.next);
        (n, Some(n))
    }
}

impl JournalView {
    #[must_use]
    pub const fn memory(tail: Arc<MemoryStore<ProductStore>>) -> Self {
        Self { tail, prefix: None }
    }

    #[must_use]
    pub const fn with_prefix(
        tail: Arc<MemoryStore<ProductStore>>,
        prefix: Arc<JournalPrefix>,
    ) -> Self {
        Self {
            tail,
            prefix: Some(prefix),
        }
    }

    #[must_use]
    pub const fn tail(&self) -> &Arc<MemoryStore<ProductStore>> {
        &self.tail
    }

    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn records(&self) -> &crate::RecordSegments<ProductStore> {
        self.tail.records()
    }

    #[cfg(feature = "test-support")]
    pub fn query(
        &self,
        query: &crate::Query<ProductStore>,
    ) -> Result<crate::Page<ProductStore>, crate::StoreError<ProductStore>> {
        use crate::Store as _;
        self.tail.query(query)
    }

    pub fn memory_mut(&mut self) -> Option<&mut MemoryStore<ProductStore>> {
        if self.prefix.is_some() {
            return None;
        }
        Some(Arc::make_mut(&mut self.tail))
    }

    #[must_use]
    pub fn prefix(&self) -> Option<&Arc<JournalPrefix>> {
        self.prefix.as_ref()
    }

    #[must_use]
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.tail, &other.tail)
            && match (&self.prefix, &other.prefix) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }

    #[must_use]
    pub const fn origin(&self) -> usize {
        if self.prefix.is_some() {
            PREFIX_ORIGIN
        } else {
            0
        }
    }

    fn prefix_index(&self) -> Result<Option<Arc<PrefixIndex>>, String> {
        self.prefix
            .as_ref()
            .map(|prefix| prefix.index())
            .transpose()
    }

    pub fn start(&self) -> Result<usize, String> {
        Ok(match self.prefix_index()? {
            Some(index) => PREFIX_ORIGIN - index.rows.len(),
            None => 0,
        })
    }

    #[must_use]
    pub fn end(&self) -> usize {
        self.origin() + self.tail.records().len()
    }

    #[must_use]
    pub fn sealed_end(&self) -> usize {
        self.origin() + self.tail.sealed_records().len()
    }

    pub fn is_empty(&self) -> Result<bool, String> {
        Ok(self.start()? == self.end())
    }

    pub fn ordinal(&self, position: usize) -> Result<SurfaceSequence, String> {
        let start = self.start()?;
        let index = position
            .checked_sub(start)
            .ok_or("position precedes the journal")?;
        Ok(SurfaceSequence::from_index(index))
    }

    #[must_use]
    pub fn surface_mark(&self) -> SurfaceSequence {
        SurfaceSequence::from_index(self.end())
    }

    fn row_at<'s>(
        &'s self,
        position: usize,
        commit: &mut Option<CommitRows>,
    ) -> Result<ViewRow<'s>, String> {
        let origin = self.origin();
        if position >= origin {
            let local = position - origin;
            if local >= self.tail.records().len() {
                return Err(format!("journal position {position} is beyond the view"));
            }
            return Ok(ViewRow {
                position,
                inner: ViewRowInner::Tail(self.tail.records().record_row(local)),
            });
        }
        let prefix = self
            .prefix
            .as_ref()
            .ok_or("journal position precedes the view")?;
        let index = prefix.index()?;
        let local = position
            .checked_sub(PREFIX_ORIGIN - index.rows.len())
            .ok_or("journal position precedes the prefix")?;
        let (at, operation) = index.rows[local];
        if commit.as_ref().is_none_or(|rows| rows.commit != at) {
            *commit = Some(prefix.read_commit(at)?);
        }
        let rows = commit.as_ref().expect("commit read above");
        let (_, record, stored) = rows
            .rows
            .iter()
            .find(|(found, _, _)| *found == operation)
            .ok_or_else(|| format!("journal prefix commit {at} lost operation {operation}"))?;
        Ok(ViewRow {
            position,
            inner: ViewRowInner::Prefix {
                commit: at,
                record: record.clone(),
                stored: match stored {
                    PrefixStored::Encoded(payload) => PrefixStored::Encoded(payload.clone()),
                    PrefixStored::Decoded => PrefixStored::Decoded,
                },
            },
        })
    }

    pub fn record_row(&self, position: usize) -> Result<ViewRow<'_>, String> {
        self.row_at(position, &mut None)
    }

    pub fn row(&self, position: usize) -> Result<Cow<'_, Record<ProductStore>>, String> {
        self.record_row(position).map(ViewRow::into_record)
    }

    pub fn row_commit(&self, position: usize) -> Result<u64, String> {
        let origin = self.origin();
        if position >= origin {
            return Ok(self.tail.records().row_commit(position - origin));
        }
        let index = self
            .prefix_index()?
            .ok_or("journal position precedes the view")?;
        let local = position
            .checked_sub(PREFIX_ORIGIN - index.rows.len())
            .ok_or("journal position precedes the prefix")?;
        Ok(index.rows[local].0)
    }

    pub fn row_revision(&self, position: usize) -> Result<Option<RevisionEpochId>, String> {
        let origin = self.origin();
        if position >= origin {
            return Ok(self.tail.records().row_revision(position - origin));
        }
        Ok(self
            .row(position)?
            .header()
            .position()
            .stamp()
            .map(circular_core::Stamp::revision))
    }

    #[must_use]
    pub fn rows(&self, range: std::ops::Range<usize>) -> ViewRows<'_> {
        ViewRows {
            view: self,
            next: range.start,
            end: range.end.min(self.end()),
            commit: None,
            tail: None,
        }
    }

    pub fn all_rows(&self) -> Result<ViewRows<'_>, String> {
        Ok(self.rows(self.start()?..self.end()))
    }

    pub fn sealed_rows(&self) -> Result<ViewRows<'_>, String> {
        Ok(self.rows(self.start()?..self.sealed_end()))
    }

    fn translate_tail(&self, local: usize) -> usize {
        self.origin() + local
    }

    fn prefix_for(&self, needed: bool) -> Result<Option<(usize, Arc<PrefixIndex>)>, String> {
        if !needed {
            return Ok(None);
        }
        Ok(self
            .prefix_index()?
            .map(|index| (PREFIX_ORIGIN - index.rows.len(), index)))
    }

    pub fn arrival_coordinate_bounds(&self) -> Result<Vec<(Actor, u64)>, String> {
        let mut ends = std::collections::BTreeMap::<Actor, u64>::new();
        if let Some((_, index)) = self.prefix_for(self.prefix.is_some())? {
            for (actor, end) in index.surface.arrival_coordinate_bounds() {
                ends.insert(actor.clone(), end);
            }
        }
        for (actor, end) in self.tail.arrival_coordinate_bounds() {
            let slot = ends.entry(actor.clone()).or_default();
            *slot = (*slot).max(end);
        }
        Ok(ends.into_iter().collect())
    }

    pub fn arrival_positions_since(&self, actor: &Actor, from: u64) -> Result<Vec<usize>, String> {
        let needed = self.prefix.is_some()
            && self
                .tail
                .arrival_first_ordinal(actor)
                .is_none_or(|first| first > from);
        let mut positions = Vec::new();
        if let Some((start, index)) = self.prefix_for(needed)? {
            positions.extend(
                index
                    .surface
                    .arrival_positions_since(actor, from)
                    .map(|local| start + local),
            );
        }
        positions.extend(
            self.tail
                .arrival_positions_since(actor, from)
                .map(|local| self.translate_tail(local)),
        );
        Ok(positions)
    }

    pub fn emission_positions_between(
        &self,
        producer: &Producer,
        from: u64,
        end: u64,
    ) -> Result<Vec<usize>, String> {
        let needed = self.prefix.is_some()
            && self
                .tail
                .emission_first_cause(producer)
                .is_none_or(|first| first > from);
        let mut positions = Vec::new();
        if let Some((start, index)) = self.prefix_for(needed)? {
            positions.extend(
                index
                    .surface
                    .emission_positions_between(producer, from, end)
                    .map(|local| start + local),
            );
        }
        positions.extend(
            self.tail
                .emission_positions_between(producer, from, end)
                .map(|local| self.translate_tail(local)),
        );
        Ok(positions)
    }

    pub fn first_arrival_position(
        &self,
        actor: &Actor,
        from: u64,
    ) -> Result<Option<usize>, String> {
        let needed = self.prefix.is_some()
            && self
                .tail
                .arrival_first_ordinal(actor)
                .is_none_or(|first| first > from);
        if let Some((start, index)) = self.prefix_for(needed)?
            && let Some(local) = index.surface.arrival_positions_since(actor, from).next()
        {
            return Ok(Some(start + local));
        }
        Ok(self
            .tail
            .arrival_positions_since(actor, from)
            .next()
            .map(|local| self.translate_tail(local)))
    }

    pub fn display_coordinate_bounds(&self) -> Result<Vec<(Producer, u64)>, String> {
        let mut ends = Vec::new();
        if let Some((_, index)) = self.prefix_for(self.prefix.is_some())? {
            ends.extend(
                index
                    .surface
                    .display_coordinate_bounds()
                    .map(|(actor, end)| (actor.clone(), end)),
            );
        }
        ends.extend(
            self.tail
                .display_coordinate_bounds()
                .map(|(actor, end)| (actor.clone(), end)),
        );
        Ok(ends)
    }

    pub fn display_positions_since(
        &self,
        actor: &Producer,
        from: u64,
    ) -> Result<Vec<usize>, String> {
        let mut positions = Vec::new();
        if let Some((start, index)) = self.prefix_for(self.prefix.is_some())? {
            positions.extend(
                index
                    .surface
                    .display_positions_since(actor, from)
                    .map(|local| start + local),
            );
        }
        positions.extend(
            self.tail
                .display_positions_since(actor, from)
                .map(|local| self.translate_tail(local)),
        );
        Ok(positions)
    }

    pub fn recorded_display_keys(&self) -> Result<std::collections::BTreeSet<DisplayKey>, String> {
        let mut keys = std::collections::BTreeSet::new();
        if let Some((_, index)) = self.prefix_for(self.prefix.is_some())? {
            keys.extend(index.surface.recorded_display_keys().cloned());
        }
        keys.extend(self.tail.recorded_display_keys().cloned());
        Ok(keys)
    }

    pub fn lifecycle_positions_of(&self, kind: &KindKey) -> Result<Vec<usize>, String> {
        let mut positions = Vec::new();
        if let Some((start, index)) = self.prefix_for(self.prefix.is_some())? {
            positions.extend(
                index
                    .surface
                    .lifecycle_positions_of(kind)
                    .map(|local| start + local),
            );
        }
        positions.extend(
            self.tail
                .lifecycle_positions_of(kind)
                .map(|local| self.translate_tail(local)),
        );
        Ok(positions)
    }

    pub fn sealed_structure_positions(&self) -> Result<Vec<usize>, String> {
        let mut positions = Vec::new();
        if let Some((start, index)) = self.prefix_for(self.prefix.is_some())? {
            positions.extend(
                index
                    .surface
                    .structure_positions_before(usize::MAX)
                    .map(|local| start + local),
            );
        }
        positions.extend(
            self.tail
                .sealed_structure_positions()
                .map(|local| self.translate_tail(local)),
        );
        Ok(positions)
    }

    pub fn sealed_structure_count(&self) -> Result<usize, String> {
        let before = match self.prefix_for(self.prefix.is_some())? {
            Some((_, index)) => index.surface.structure_count_before(usize::MAX),
            None => 0,
        };
        Ok(before + self.tail.sealed_structure_count())
    }

    #[must_use]
    pub fn applied_through(&self) -> u64 {
        let tail = self.tail.applied_through();
        self.prefix
            .as_ref()
            .map_or(tail, |prefix| tail.max(prefix.through))
    }

    pub fn fact_range(&self) -> Result<std::ops::Range<usize>, String> {
        let origin = self.origin();
        let before = self.prefix_index()?.map_or(0, |index| index.facts.len());
        Ok(origin - before..origin + self.tail.checkpoint_facts().len())
    }

    pub fn fact_commit(&self, position: usize) -> Result<u64, String> {
        let origin = self.origin();
        if position >= origin {
            return Ok(self.tail.checkpoint_facts().row_commit(position - origin));
        }
        let index = self
            .prefix_index()?
            .ok_or("fact position precedes the view")?;
        let local = position
            .checked_sub(origin - index.facts.len())
            .ok_or("fact position precedes the prefix")?;
        Ok(index.facts[local].0)
    }

    pub fn fact_bytes(&self, position: usize) -> Result<Vec<u8>, String> {
        let origin = self.origin();
        if position >= origin {
            let row = self.tail.checkpoint_facts().record_row(position - origin);
            return match row.stored() {
                StoredRow::Encoded(payload) => Ok(payload.body().to_vec()),
                StoredRow::Decoded(record) => crate::encode_record(record, &ProductRecordCodec)
                    .map_err(|error| format!("checkpoint fact: {error:?}")),
            };
        }
        let index = self
            .prefix_index()?
            .ok_or("fact position precedes the view")?;
        let local = position
            .checked_sub(origin - index.facts.len())
            .ok_or("fact position precedes the prefix")?;
        crate::encode_record(&index.facts[local].1, &ProductRecordCodec)
            .map_err(|error| format!("checkpoint fact: {error:?}"))
    }
}
