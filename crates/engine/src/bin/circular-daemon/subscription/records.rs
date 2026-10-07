use super::*;
use circular_protocol::Partition;
use circular_protocol::rejection_code::RejectionReason;
use circular_store::{
    ArrivalProjection, ClassKey, ObservationFact, ObservationKey, OpaqueWitness,
    ProductRecordCodec, ProductStore, Record, SqliteJournal, StoredRow,
};
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
};

mod query;
pub(crate) use query::FrozenRecords;

pub(crate) use crate::daemon::subscription_catalog::RECORDS_TARGET as TARGET;

/// The fact bytes stay intact; observations carry their bucket and System projection when present.
#[derive(Clone, Debug, PartialEq)]
enum ItemBody {
    Record(Vec<u8>, Option<Value>),
}
impl ItemBody {
    fn record(record: &Record<ProductStore>, bytes: Vec<u8>) -> Result<Self, String> {
        use crate::kernel::system::SystemBody;
        use circular_core::BuiltinObservationName as N;
        let system = match record.header().key() {
            ClassKey::Observation(ObservationKey::StreamItem(_, _, item))
                if matches!(
                    item.kind(),
                    N::SystemActivationOutcome
                        | N::SystemRevisionAdoptionOutcome
                        | N::SystemRecoveryOutcome
                        | N::SystemPauseAccepted
                        | N::SystemResumeAccepted
                ) =>
            {
                SystemBody::read(record)?.map(|body| {
                    let mut fields = vec![
                        ("producer", Value::string("pipeline")),
                        (
                            "kind",
                            Value::string(
                                item.kind()
                                    .as_str()
                                    .strip_prefix("system_")
                                    .expect("System catalog prefix"),
                            ),
                        ),
                    ];
                    let revision = match body {
                        SystemBody::Outcome {
                            revision, result, ..
                        } => {
                            fields.push((
                                "code",
                                result.err().map_or(Value::Null, |reason| {
                                    Value::UInt(u64::from(
                                        reason
                                            .code_in(Partition::Query)
                                            .expect("System outcome code"),
                                    ))
                                }),
                            ));
                            revision
                        }
                        SystemBody::PauseAccepted { force } => {
                            fields.push(("force", Value::Bool(force)));
                            record.header().at().revision()
                        }
                        SystemBody::ResumeAccepted => record.header().at().revision(),
                    };
                    fields.push(("revision", Value::UInt(revision.get())));
                    Value::object(fields).expect("distinct System fields")
                })
            }
            _ => None,
        };
        Ok(Self::Record(bytes, system))
    }
    /// Query and subscription publish the same projection of the recorded envelope.
    fn payload(&self, cursor: Value) -> Result<Value, String> {
        let Self::Record(bytes, system) = self;
        let envelope = circular_store::decode_envelope(bytes)
            .map_err(|e| format!("records envelope: {e:?}"))?;
        let mut fields = vec![("cursor", cursor), ("fact", Value::Bytes(bytes.clone()))];
        if let Some(bucket) = envelope.observation_bucket {
            fields.push(("observation_bucket", Value::UInt(bucket.millis())));
        }
        if let Some(system) = system {
            fields.push(("system", system.clone()));
        }
        Ok(Value::object(fields).expect("distinct records fields"))
    }
    fn canonical(&self) -> Result<Vec<u8>, String> {
        let Self::Record(bytes, _) = self;
        Ok(bytes.clone())
    }
}

/// One immutable fact. The record codec owns its classification and complete provenance.
struct Item {
    cursor: Value,
    body: ItemBody,
    retained: bool,
    commit: u64,
}

pub(crate) struct Records {
    path: PathBuf,
    anchor: Value,
    scope: circular_plan::ScopeId,
    cut: u64,
    arrivals: Held,
    checkpoints: Held,
    epochs: Held,
    authoring: Option<engine::authoring_assembly::ledger::AuthoringState>,
    instances: engine::InstanceLifecycleReplay,
    authored_scope: Vec<circular_protocol::declaration_payload::ScopeSegment>,
    environment: Option<circular_protocol::declaration_payload::AuthoringEnvironment>,
    ending: Option<SubscriptionEndReason>,
    pending: VecDeque<Item>,
    seen: BTreeMap<Vec<u8>, Vec<u8>>,
    boundary: bool,
    retained_left: usize,
    retained_delivered: u64,
    last: Value,
    reset: Option<Value>,
    replay: bool,
}

#[derive(Clone, Copy, Default)]
struct Held {
    seen: usize,
    last: u64,
}

impl Held {
    fn holds(self, len: usize, commit_at: impl FnOnce(usize) -> u64) -> bool {
        self.seen == 0 || (self.seen <= len && commit_at(self.seen - 1) == self.last)
    }
    fn holds_in(
        self,
        range: std::ops::Range<usize>,
        commit_at: impl FnOnce(usize) -> Result<u64, String>,
    ) -> Result<bool, String> {
        Ok(self.seen == 0
            || (self.seen <= range.len() && commit_at(range.start + self.seen - 1)? == self.last))
    }
    fn take(&mut self, commit: u64) {
        self.seen += 1;
        self.last = commit;
    }
}

#[derive(Clone)]
pub(crate) struct Sources {
    arrivals: Option<circular_store::JournalView>,
    authoring: crate::daemon::authoring_store::RetainedCommits,
    upto: Option<u64>,
    replay: bool,
}

impl Sources {
    pub(crate) fn of(
        world: &crate::daemon::read_world::WorldRead,
        authoring: &crate::daemon::authoring_store::AuthoringStore,
    ) -> Self {
        Self::from_parts(world.server.as_deref(), world.system.as_deref(), authoring)
    }

    pub(crate) fn from_parts(
        server: Option<&crate::daemon::ledger::ServerRead>,
        system: Option<&crate::daemon::ledger::SystemRuntime>,
        authoring: &crate::daemon::authoring_store::AuthoringStore,
    ) -> Self {
        Self {
            arrivals: server
                .map(crate::daemon::ledger::ServerRead::arrival_prefix)
                .or_else(|| system.map(crate::daemon::ledger::SystemRuntime::read_view)),
            authoring: authoring.retained(),
            upto: None,
            replay: false,
        }
    }

    pub(crate) fn of_read(
        world: &crate::daemon::read_world::WorldRead,
        authoring: &crate::daemon::authoring_store::AuthoringStore,
        lens: Option<&crate::daemon::replay::ReplayLens>,
    ) -> Result<Self, String> {
        crate::daemon::query::record_sources(
            Self::of(world, authoring),
            world.server.as_deref(),
            lens,
            None,
        )
    }

    pub(crate) fn within_lens(
        self,
        standing: &crate::daemon::ledger::ServerRead,
        lens: &crate::daemon::replay::ReplayLens,
    ) -> Result<Self, String> {
        self.bounded(&standing.bounded_cut(lens.position())?)
    }

    pub(crate) fn with_upto(
        self,
        server: Option<&crate::daemon::ledger::ServerRead>,
        upto: &circular_runtime::LogCut,
    ) -> Result<Self, String> {
        let server = server.ok_or("upto names a cut of the standing stream; none stands")?;
        server.read_bound(upto)?;
        let mut sources = self.bounded(upto)?;
        sources.replay = false;
        Ok(sources)
    }

    fn bounded(mut self, cut: &circular_runtime::LogCut) -> Result<Self, String> {
        self.replay = true;
        self.upto = Some(match self.arrivals.as_ref() {
            None => 0,
            Some(prefix) => {
                let mut first_beyond = None::<usize>;
                for (actor, _) in prefix.arrival_coordinate_bounds()? {
                    let from = cut.get(&actor).map_or(0, circular_core::ArrivalIndex::get);
                    if let Some(position) = prefix.first_arrival_position(&actor, from)? {
                        first_beyond =
                            Some(first_beyond.map_or(position, |found| found.min(position)));
                    }
                }
                match first_beyond {
                    Some(position) => prefix.row_commit(position)?.saturating_sub(1),
                    None if prefix.is_empty()? => 0,
                    None => prefix
                        .row_commit(prefix.end() - 1)?
                        .max(prefix.applied_through()),
                }
            }
        });
        Ok(self)
    }

    fn capped(&self, through: u64) -> u64 {
        self.upto.map_or(through, |cap| through.min(cap))
    }

    #[cfg(test)]
    pub(crate) fn from_journal(path: &Path) -> Self {
        let directory = path.parent().expect("state directory");
        let manifest = engine::state_manifest::read_state_manifest(path).unwrap();
        let arrivals = manifest.map(|manifest| {
            let snapshot = SqliteJournal::read_only_namespace(
                path,
                engine::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
            )
            .unwrap();
            let mut store = engine::published_arrivals(&snapshot).unwrap();
            store.seal_all();
            circular_store::JournalView::memory(std::sync::Arc::new(store.published_prefix()))
        });
        Self {
            upto: None,
            replay: false,
            arrivals,
            authoring: crate::daemon::authoring_store::AuthoringStore::open(directory)
                .unwrap()
                .retained(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_arrivals_of(mut self, held: &Self) -> Self {
        self.arrivals = held.arrivals.clone();
        self
    }

    fn marks(&self) -> [u64; 2] {
        [
            self.arrivals
                .as_ref()
                .map_or(0, circular_store::JournalView::applied_through),
            self.authoring.through(),
        ]
    }
}

fn column_bytes(
    prefix: &circular_store::JournalView,
    index: usize,
    row: bool,
) -> Result<Vec<u8>, String> {
    if !row {
        return prefix.fact_bytes(prefix.fact_range()?.start + index);
    }
    let stored = prefix.record_row(prefix.start()? + index)?;
    let encoded;
    let bytes: &[u8] = match stored.stored() {
        StoredRow::Encoded(payload) => payload.body(),
        StoredRow::Decoded(record) => {
            encoded = circular_store::encode_record(record, &ProductRecordCodec)
                .map_err(|e| format!("records prefix row: {e:?}"))?;
            &encoded
        }
    };
    Ok(bytes.to_vec())
}

fn next_arrival(
    prefix: &circular_store::JournalView,
    rows: usize,
    checkpoints: usize,
    cut: u64,
) -> Result<Option<(u64, Next)>, String> {
    let start = prefix.start()?;
    let row = if start + rows < prefix.end() {
        Some(prefix.row_commit(start + rows)?)
    } else {
        None
    }
    .filter(|commit| *commit <= cut)
    .map(|commit| (commit, Next::Row));
    let facts = prefix.fact_range()?;
    let checkpoint = if checkpoints < facts.len() {
        Some(prefix.fact_commit(facts.start + checkpoints)?)
    } else {
        None
    }
    .filter(|commit| *commit <= cut)
    .map(|commit| (commit, Next::Checkpoint));
    Ok([row, checkpoint].into_iter().flatten().min())
}

const SOURCE_NAMESPACES: [&str; 2] = [
    engine::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
    engine::state_journal::AUTHORING_JOURNAL_NAMESPACE,
];

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Next {
    Row,
    Checkpoint,
    Authoring,
}

#[cfg(test)]
thread_local! {
    static BUILT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn thread_built() -> u64 {
    BUILT.with(std::cell::Cell::get)
}

fn cursor(anchor: &Value, position: Vec<u8>) -> Value {
    Value::object([
        ("anchor", anchor.clone()),
        ("domain", Value::String(TARGET.into())),
        ("position", Value::Bytes(position)),
    ])
    .expect("distinct cursor fields")
}

/// Record-class witnesses are the only positions. Retired history tag 6 is refused.
fn validate_position(bytes: &[u8]) -> Result<(), String> {
    OpaqueWitness::from_bytes(bytes)
        .map(|_| ())
        .map_err(|e| format!("invalid record reference: {e:?}"))
}

fn position(value: &Value, anchor: &Value) -> Result<Vec<u8>, String> {
    let Value::Object(fields) = value else {
        return Err("records cursor is not an object".into());
    };
    if fields.len() != 3
        || fields.get("anchor") != Some(anchor)
        || fields.get("domain") != Some(&Value::String(TARGET.into()))
    {
        return Err("records cursor belongs to another state, target or scope".into());
    }
    let Some(Value::Bytes(bytes)) = fields.get("position") else {
        return Err("records cursor position is not Bytes".into());
    };
    validate_position(bytes)?;
    Ok(bytes.clone())
}

impl Item {
    fn payload(&self) -> Result<Value, String> {
        self.body.payload(self.cursor.clone())
    }
}

impl Records {
    pub(crate) fn open(path: &Path, args: Value, sources: &Sources) -> Result<Self, String> {
        let (mut result, since) = Self::prepare(path, args, sources.authoring.creation())?;
        result.replay = sources.replay;
        result.advance(sources, true)?;
        result.finish_open(since, sources)?;
        Ok(result)
    }

    fn prepare(
        path: &Path,
        args: Value,
        creation: &engine::authoring_assembly::ledger::ProjectCreation,
    ) -> Result<(Self, Option<Value>), String> {
        let Value::Object(fields) = args else {
            return Err("records args are not an object".into());
        };
        let mut fields = fields.into_map();
        let scope_value = fields.remove("scope").ok_or("records.scope is absent")?;
        let scope = circular_store::scope_from_value(&scope_value)
            .map_err(|e| format!("records.scope: {e:?}"))?;
        if circular_store::scope_value(&scope).map_err(|e| format!("records.scope: {e:?}"))?
            != scope_value
        {
            return Err("records.scope is not canonical".into());
        }
        let authored_scope = scope
            .segments()
            .iter()
            .map(|segment| {
                let name = match segment {
                    circular_plan::ScopeSeg::Child(name) => name,
                    circular_plan::ScopeSeg::Instance { of, .. } => of,
                };
                circular_protocol::declaration_payload::ScopeSegment::Child(
                    name.as_str().to_owned(),
                )
            })
            .collect();
        let since = fields.remove("since");
        if !fields.is_empty() {
            return Err("unknown records argument".into());
        }
        let anchor = Value::Array(vec![Value::Bytes(creation.project.to_vec()), scope_value]);
        if let Some(value) = &since {
            position(value, &anchor)?;
        }
        let result = Self {
            path: path.into(),
            anchor: anchor.clone(),
            scope,
            cut: 0,
            arrivals: Held::default(),
            checkpoints: Held::default(),
            epochs: Held::default(),
            authoring: None,
            instances: engine::InstanceLifecycleReplay::new(),
            authored_scope,
            environment: Some(creation.environment.clone()),
            ending: None,
            pending: VecDeque::new(),
            seen: BTreeMap::new(),
            boundary: false,
            retained_left: 0,
            retained_delivered: 0,
            last: anchor,
            reset: None,
            replay: false,
        };
        Ok((result, since))
    }

    fn finish_open(&mut self, since: Option<Value>, sources: &Sources) -> Result<(), String> {
        let after = since
            .as_ref()
            .map(|c| position(c, &self.anchor))
            .transpose()?;
        let result = self;
        if let Some(state) = result.authoring.as_ref() {
            result.environment = Some(state.environment().clone());
        }
        let scope_exists = result.scope_exists(sources)?;
        if !scope_exists && after.is_none() {
            return Err("records.scope does not exist in the retained authoring prefix".into());
        }
        if let Some(after) = after {
            let mut found = false;
            let mut kept = VecDeque::new();
            for item in result.pending.drain(..) {
                if !found {
                    if position(&item.cursor, &result.anchor)? == after {
                        found = true;
                        let environment = Some(sources.authoring.environment_before(item.commit)?);
                        if environment != result.environment {
                            result.ending =
                                result.environment.clone().map(|required_environment| {
                                    SubscriptionEndReason::IncompatibleClient {
                                        required_environment,
                                    }
                                });
                            result.boundary = true;
                        }
                    }
                    continue;
                }
                kept.push_back(item);
            }
            if !found {
                result.reset = Some(result.anchor.clone());
            }
            if result.ending.is_none() {
                result.pending = kept;
            }
            result.last = since.expect("since supplied");
        }
        if !scope_exists && result.reset.is_none() {
            return Err("records.scope does not exist in the retained authoring prefix".into());
        }
        result.retained_left = result.pending.len();
        Ok(())
    }

    fn concrete(&self) -> bool {
        self.scope
            .segments()
            .iter()
            .any(|segment| matches!(segment, circular_plan::ScopeSeg::Instance { .. }))
    }

    fn scope_exists(&self, sources: &Sources) -> Result<bool, String> {
        if !self.concrete() {
            return Ok(self.authored_scope.is_empty()
                || self
                    .authoring
                    .as_ref()
                    .is_some_and(|state| state.scope_exists(&self.authored_scope)));
        }
        if !self.instances_live() {
            return Ok(false);
        }
        let (index, of, key) = self
            .scope
            .segments()
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, segment)| match segment {
                circular_plan::ScopeSeg::Instance { of, key } => Some((index, of, key)),
                _ => None,
            })
            .expect("a concrete scope");
        if index + 1 == self.scope.segments().len() {
            return Ok(true);
        }
        let mut scope = self.scope.segments()[..index].to_vec();
        scope.push(circular_plan::ScopeSeg::Child(of.clone()));
        let scope = circular_plan::ScopeId::from_segments(scope).expect("a validated scope prefix");
        let Some((_, minted)) = self.instances.minted(&scope).find(|(live, _)| *live == key) else {
            return Ok(false);
        };
        Ok(sources
            .authoring
            .at_cursor(minted.revision().get())?
            .scope_exists(&self.authored_scope))
    }

    fn instances_live(&self) -> bool {
        self.scope
            .segments()
            .iter()
            .enumerate()
            .all(|(index, segment)| {
                let circular_plan::ScopeSeg::Instance { of, key } = segment else {
                    return true;
                };
                let mut template = self.scope.segments()[..index].to_vec();
                template.push(circular_plan::ScopeSeg::Child(of.clone()));
                let template = circular_plan::ScopeId::from_segments(template)
                    .expect("prefix of validated scope");
                self.instances.live(&template).any(|live| live == key)
            })
    }

    fn fold_instances(
        &mut self,
        record: &Record<ProductStore>,
        sources: &Sources,
    ) -> Result<bool, String> {
        if let Record::Structure(structure) = record
            && matches!(
                structure.fact(),
                circular_store::StructureFact::GraphRevision(_)
            )
        {
            let revision = record.header().at().revision();
            let state = sources.authoring.at_cursor(revision.get())?;
            let plan = state.current_plan().map_err(|e| e.to_string())?;
            let declarations =
                engine::RevisionDeclarations::published(&plan).map_err(|e| e.to_string())?;
            self.instances
                .declarations(revision, declarations.graph())
                .map_err(|error| error.to_string())?;
            return Ok(true);
        }
        self.instances
            .read(record)
            .map_err(|e| format!("records instance lifecycle: {e}"))
    }

    fn refresh(&mut self, sources: &Sources) -> Result<(), String> {
        if self.reset.is_some() {
            return Ok(());
        }
        self.advance(sources, false)
    }

    fn complete_through(&self, marks: [u64; 2]) -> Result<u64, String> {
        let floor = marks.iter().copied().min().unwrap_or(0).max(self.cut);
        let (through, tail) =
            SqliteJournal::read_only_namespace_sequences(&self.path, floor, &SOURCE_NAMESPACES)
                .map_err(|e| format!("records prefix read failed: {e}"))?;
        for (sequence, namespace) in tail {
            if sequence > marks[namespace] {
                return Ok((sequence - 1).max(self.cut));
            }
        }
        Ok(through.max(self.cut))
    }

    fn advance(&mut self, sources: &Sources, retained: bool) -> Result<(), String> {
        let arrivals = sources.arrivals.as_ref();
        let epochs = sources.authoring.epochs();
        let arrivals_held = match arrivals {
            None => true,
            Some(prefix) => {
                self.arrivals
                    .holds_in(prefix.start()?..prefix.end(), |i| prefix.row_commit(i))?
                    && self
                        .checkpoints
                        .holds_in(prefix.fact_range()?, |i| prefix.fact_commit(i))?
            }
        };
        let held = arrivals_held && self.epochs.holds(epochs.len(), |i| epochs[i].0);
        if !held {
            self.pending.clear();
            self.reset = Some(self.anchor.clone());
            return Ok(());
        }
        let cut = sources
            .capped(self.complete_through(sources.marks())?)
            .max(self.cut);
        if self.ending.is_some() {
            self.cut = cut;
            return Ok(());
        }
        if retained {
            let (count, last, state) = sources.authoring.state_through(cut)?;
            self.epochs = Held { seen: count, last };
            self.authoring = Some(state);
        }
        let mut fresh = Vec::new();
        loop {
            let arrival = match arrivals {
                Some(prefix) => {
                    next_arrival(prefix, self.arrivals.seen, self.checkpoints.seen, cut)?
                }
                None => None,
            };
            let epoch = epochs
                .get(self.epochs.seen)
                .filter(|(sequence, _)| *sequence <= cut)
                .map(|(sequence, _)| (*sequence, Next::Authoring));
            let Some((commit, next)) = [arrival, epoch].into_iter().flatten().min() else {
                break;
            };
            match next {
                Next::Authoring => {
                    let (_, payload) = epochs[self.epochs.seen];
                    self.epochs.take(commit);
                    let value =
                        circular_core::decode(payload, Ceilings::for_boundary(Boundary::Journal))
                            .map_err(|e| format!("records authoring prefix: {e:?}"))?;
                    let state =
                        engine::authoring_assembly::ledger::AuthoringState::fold_journal_entry(
                            self.authoring.take(),
                            value,
                        )
                        .map_err(|rejection| rejection.to_string())?
                        .0;
                    if !retained {
                        if Some(state.environment()) != self.environment.as_ref() {
                            self.ending = Some(SubscriptionEndReason::IncompatibleClient {
                                required_environment: state.environment().clone(),
                            });
                        } else if !self.concrete() && !state.scope_exists(&self.authored_scope) {
                            self.ending =
                                Some(SubscriptionEndReason::ScopeGone { cursor: Vec::new() });
                        }
                    }
                    self.authoring = Some(state);
                    if self.ending.is_some() {
                        break;
                    }
                }
                Next::Row | Next::Checkpoint => {
                    let prefix = arrivals.expect("arrival candidate has a prefix");
                    let held = if next == Next::Row {
                        &mut self.arrivals
                    } else {
                        &mut self.checkpoints
                    };
                    let bytes = column_bytes(prefix, held.seen, next == Next::Row)?;
                    held.take(commit);
                    if self.fact(&bytes, commit, retained, &mut fresh, sources)? {
                        break;
                    }
                }
            }
        }
        let mut unique = BTreeMap::new();
        for (key, body, _) in &fresh {
            if let Some(prior) = unique.insert(key, body) {
                if prior != body {
                    return Err("immutable record reference changed within prefix".into());
                }
            }
        }
        for (key, body, commit) in fresh {
            if self.seen.contains_key(&key) {
                continue;
            }
            let canonical = body.canonical()?;
            let cursor = cursor(&self.anchor, key.clone());
            self.pending.push_back(Item {
                cursor,
                body,
                retained,
                commit,
            });
            self.seen.insert(key, canonical);
        }
        self.cut = cut;
        Ok(())
    }

    fn fact(
        &mut self,
        bytes: &[u8],
        commit: u64,
        retained: bool,
        fresh: &mut Vec<(Vec<u8>, ItemBody, u64)>,
        sources: &Sources,
    ) -> Result<bool, String> {
        let concrete = self
            .scope
            .segments()
            .iter()
            .any(|segment| matches!(segment, circular_plan::ScopeSeg::Instance { .. }));
        let lifecycle_needed = concrete && query::is_cell_lifecycle(bytes)?;
        if !lifecycle_needed && !query::selected_envelope(bytes, &self.scope)? {
            return Ok(false);
        }
        #[cfg(test)]
        BUILT.with(|count| count.set(count.get() + 1));
        let record = ArrivalProjection::new()
            .record(bytes)
            .map_err(|e| format!("records projection: {e:?}"))?;
        if concrete && self.fold_instances(&record, sources)? && !retained && !self.instances_live()
        {
            self.ending = Some(SubscriptionEndReason::ScopeGone { cursor: Vec::new() });
            return Ok(true);
        }
        if !selected(&record, &self.scope)? {
            return Ok(false);
        }
        let key = crate::daemon::ledger::record_witness(record.header())?
            .as_bytes()
            .to_vec();
        let body = circular_store::encode_record(&record, &ProductRecordCodec)
            .map_err(|e| format!("record body: {e:?}"))?;
        if let Some(prior) = self.seen.get(&key) {
            if prior != &body {
                return Err("immutable record reference changed".into());
            }
            return Ok(false);
        }
        fresh.push((key, ItemBody::record(&record, body)?, commit));
        Ok(false)
    }

    fn pending_after(&self) -> u64 {
        self.pending.len() as u64 + u64::from(!self.boundary)
    }

    #[cfg(test)]
    fn pump(
        &mut self,
        stream: &mut impl LocalByteStream<Error = std::io::Error>,
        flow: &mut FlowFsm,
        correlation: u32,
    ) -> Result<u64, String> {
        let sources = Sources::from_journal(&self.path.clone());
        self.refresh(&sources)?;
        self.pump_prepared(stream, flow, correlation)
    }

    fn pump_prepared(
        &mut self,
        stream: &mut impl LocalByteStream<Error = std::io::Error>,
        flow: &mut FlowFsm,
        correlation: u32,
    ) -> Result<u64, String> {
        if self.reset.is_some() {
            return Ok(0);
        }
        loop {
            if flow.credit().0 == 0 {
                return Ok(self.pending_after());
            }
            let (frame, is_boundary) = if !self.boundary && self.retained_left == 0 {
                (
                    SubscriptionFrame::RetentionComplete {
                        anchor: self.last.clone(),
                        delivered: self.retained_delivered,
                    },
                    true,
                )
            } else if let Some(item) = self.pending.front() {
                let payload = item.payload()?;
                (
                    SubscriptionFrame::Credit {
                        origin: if item.retained || self.replay {
                            FrameOrigin::Retained
                        } else {
                            FrameOrigin::Live
                        },
                        payload,
                        pending_after: self.pending_after() - 1,
                    },
                    false,
                )
            } else {
                return Ok(0);
            };
            frame
                .encode(Ceilings::for_boundary(Boundary::Wire))
                .map_err(|e| format!("records frame encoding failed: {e:?}"))?;
            if !write_subscription_frame_at(stream, correlation, &frame, flow, self.cut) {
                return Ok(self.pending_after());
            }
            if is_boundary {
                self.boundary = true;
            } else {
                let item = self.pending.pop_front().expect("prepared item");
                self.last = item.cursor;
                if item.retained {
                    self.retained_left -= 1;
                    self.retained_delivered += 1;
                }
            }
        }
    }
}

pub(crate) const PUBLISHED_OBSERVATION_FACTS: &[(circular_core::BuiltinObservationName, u8)] = &[
    (
        circular_core::BuiltinObservationName::InstanceTransition,
        circular_store::LIFECYCLE_FACT_TAG,
    ),
    (
        circular_core::BuiltinObservationName::Restart,
        circular_store::RESTART_FACT_TAG,
    ),
    (
        circular_core::BuiltinObservationName::DiagnosticOccurrence,
        circular_store::DIAGNOSTIC_FACT_TAG,
    ),
    (
        circular_core::BuiltinObservationName::DeadLetterEntry,
        circular_store::DEAD_LETTER_FACT_TAG,
    ),
    (
        circular_core::BuiltinObservationName::CustodyClaim,
        circular_store::CHECKPOINT_FACT_TAG,
    ),
    (
        circular_core::BuiltinObservationName::SystemActivationOutcome,
        circular_store::DIAGNOSTIC_FACT_TAG,
    ),
    (
        circular_core::BuiltinObservationName::SystemRevisionAdoptionOutcome,
        circular_store::DIAGNOSTIC_FACT_TAG,
    ),
    (
        circular_core::BuiltinObservationName::SystemRecoveryOutcome,
        circular_store::DIAGNOSTIC_FACT_TAG,
    ),
    (
        circular_core::BuiltinObservationName::SystemPauseAccepted,
        circular_store::LIFECYCLE_FACT_TAG,
    ),
    (
        circular_core::BuiltinObservationName::SystemResumeAccepted,
        circular_store::LIFECYCLE_FACT_TAG,
    ),
];

pub(crate) fn published_fact(kind: circular_core::BuiltinObservationName, fact_tag: u8) -> bool {
    PUBLISHED_OBSERVATION_FACTS
        .iter()
        .any(|(published, tag)| *published == kind && *tag == fact_tag)
}

fn selected(record: &Record<ProductStore>, scope: &circular_plan::ScopeId) -> Result<bool, String> {
    let Record::Observation(observation) = record else {
        return Ok(false);
    };
    let (item, checkpoint) = match observation.header().key() {
        ClassKey::Observation(ObservationKey::CheckpointItem(_, _, item)) => (item, true),
        ClassKey::Observation(ObservationKey::StreamItem(_, _, item)) => (item, false),
        _ => return Ok(false),
    };
    if !published_fact(
        *item.kind(),
        circular_store::observation_fact_tag(observation.fact()),
    ) {
        return Ok(false);
    }
    if checkpoint {
        let Some(body) = engine::checkpoint_value(record).map_err(|e| e.to_string())? else {
            return Ok(false);
        };
        if scope == &circular_plan::ScopeId::root() {
            return Ok(true);
        }
        let actor = body
            .as_object()
            .and_then(|fields| fields.get("actor"))
            .ok_or("checkpoint record has no actor attribution")?;
        return circular_store::actor_parts_from_value(actor)
            .map(|(found, _)| &found == scope)
            .map_err(|e| format!("checkpoint actor: {e:?}"));
    }
    let body = match item.kind() {
        circular_core::BuiltinObservationName::InstanceTransition => {
            let Some(body) = engine::instance_transition_value(record)
                .map_err(|e| format!("instance transition: {e}"))?
            else {
                return Ok(false);
            };
            let fields = body.as_array().expect("validated lifecycle carrier");
            return circular_store::scope_from_value(&fields[1])
                .map(|found| scope == &circular_plan::ScopeId::root() || &found == scope)
                .map_err(|e| format!("instance transition scope: {e:?}"));
        }
        circular_core::BuiltinObservationName::DeadLetterEntry => {
            engine::dead_letter_writer::dead_letter_value(record).map_err(|e| e.to_string())?
        }
        circular_core::BuiltinObservationName::Restart => {
            let ObservationFact::Restart(payload) = observation.fact() else {
                return Ok(false);
            };
            circular_store::ProductRestartBody::decode(payload)
                .map_err(|e| format!("restart body: {e}"))?;
            return Ok(true);
        }
        circular_core::BuiltinObservationName::DiagnosticOccurrence => {
            let ObservationFact::Diagnostic(payload) = observation.fact() else {
                return Ok(false);
            };
            let body =
                circular_core::decode(payload.body(), Ceilings::for_boundary(Boundary::Journal))
                    .map_err(|e| format!("observation body: {e:?}"))?;
            if !matches!(body.as_object().and_then(|o| o.get("kind")), Some(Value::String(k)) if k == engine::ACTOR_HEALTH_TRANSITION_KIND)
            {
                return Ok(false);
            }
            circular_protocol::actor_events::decode_actor_health_transition(body.clone())
                .map_err(|e| format!("health transition: {e:?}"))?;
            Some(body)
        }
        circular_core::BuiltinObservationName::SystemActivationOutcome
        | circular_core::BuiltinObservationName::SystemRevisionAdoptionOutcome
        | circular_core::BuiltinObservationName::SystemRecoveryOutcome
        | circular_core::BuiltinObservationName::SystemPauseAccepted
        | circular_core::BuiltinObservationName::SystemResumeAccepted => {
            return crate::kernel::system::SystemBody::read(record).map(|body| body.is_some());
        }
        _ => return Ok(false),
    };
    let selected = body.is_some();
    let body = body.unwrap_or(Value::Null);
    if !selected {
        return Ok(false);
    }
    if scope == &circular_plan::ScopeId::root() {
        return Ok(true);
    }
    if let Some(value) = body.as_object().and_then(|o| o.get("scope")) {
        return circular_store::scope_from_value(value)
            .map(|found| &found == scope)
            .map_err(|e| format!("observation scope: {e:?}"));
    }
    if let Some(value) = body.as_object().and_then(|o| o.get("actor")) {
        return circular_store::actor_parts_from_value(value)
            .map(|(found, _)| &found == scope)
            .map_err(|e| format!("observation actor: {e:?}"));
    }
    Err("record has no scope attribution for this filter".into())
}

pub(crate) fn open(
    path: &Path,
    args: Value,
    sources: &Sources,
    correlation: u32,
    lens: Option<u32>,
    live: &mut Option<Subscription>,
) -> CommandResult {
    match Records::open(path, args, sources) {
        Ok(reader) => {
            *live = Some(Subscription {
                source: crate::daemon::subscription::SubscriptionSource::Records { reader },
                flow: credit_flow(0),
                correlation,
                lens,
            });
            CommandResult::Accepted(Accepted::Nothing)
        }
        Err(message) => malformed(message),
    }
}

pub(crate) fn end_for_lens(
    stream: &mut impl LocalByteStream<Error = std::io::Error>,
    live: &mut Option<Subscription>,
    moved: crate::daemon::subscription::LensMove,
) -> bool {
    let Some(Subscription {
        source: crate::daemon::subscription::SubscriptionSource::Records { reader },
        correlation,
        ..
    }) = live
    else {
        return false;
    };
    let reason = match moved {
        crate::daemon::subscription::LensMove::Rewound { .. } => {
            SubscriptionEndReason::ResetRequired {
                floor_or_cursor: circular_core::encode(
                    &reader.anchor,
                    Ceilings::for_boundary(Boundary::Wire),
                )
                .expect("issued cursor encodes"),
            }
        }
        crate::daemon::subscription::LensMove::Closed => SubscriptionEndReason::TargetGone,
    };
    write_subscription_end_at(
        stream,
        *correlation,
        &SubscriptionEnded {
            reason,
            code: RejectionReason::Unresolved.number_in(Partition::Subscription),
            anchor: circular_core::encode(&reader.last, Ceilings::for_boundary(Boundary::Wire))
                .expect("issued cursor encodes"),
        },
    );
    *live = None;
    true
}

#[cfg(test)]
pub(crate) fn pump(
    stream: &mut impl LocalByteStream<Error = std::io::Error>,
    live: &mut Option<Subscription>,
    sources: &Sources,
) -> Option<u64> {
    let Some(Subscription {
        source: crate::daemon::subscription::SubscriptionSource::Records { reader },
        flow,
        correlation,
        ..
    }) = live
    else {
        return None;
    };
    let result = pump_reader(stream, reader, flow, *correlation, sources);
    Some(result.finish(live))
}

pub(super) fn pump_reader(
    stream: &mut impl LocalByteStream<Error = std::io::Error>,
    reader: &mut Records,
    flow: &mut FlowFsm,
    correlation: u32,
    sources: &Sources,
) -> super::PumpResult {
    let refreshed = reader.refresh(sources);
    if let Some(floor) = &reader.reset {
        write_subscription_end_at(
            stream,
            correlation,
            &SubscriptionEnded {
                reason: SubscriptionEndReason::ResetRequired {
                    floor_or_cursor: circular_core::encode(
                        floor,
                        Ceilings::for_boundary(Boundary::Wire),
                    )
                    .expect("issued cursor encodes"),
                },
                code: RejectionReason::Unresolved.number_in(Partition::Subscription),
                anchor: circular_core::encode(&reader.last, Ceilings::for_boundary(Boundary::Wire))
                    .expect("issued cursor encodes"),
            },
        );
        return super::PumpResult::Ended;
    }
    match refreshed.and_then(|()| reader.pump_prepared(stream, flow, correlation)) {
        Ok(pending) => {
            if pending == 0
                && let Some(mut reason) = reader.ending.take()
            {
                let anchor =
                    circular_core::encode(&reader.last, Ceilings::for_boundary(Boundary::Wire))
                        .expect("issued cursor encodes");
                if let SubscriptionEndReason::ScopeGone { cursor } = &mut reason {
                    *cursor = anchor.clone();
                }
                write_subscription_end_at(
                    stream,
                    correlation,
                    &SubscriptionEnded {
                        reason,
                        code: RejectionReason::Unresolved.number_in(Partition::Subscription),
                        anchor,
                    },
                );
                return super::PumpResult::Ended;
            }
            super::PumpResult::Open { pending }
        }
        Err(message) => {
            eprintln!("circular-daemon: {message}");
            write_subscription_end_at(
                stream,
                correlation,
                &SubscriptionEnded {
                    reason: SubscriptionEndReason::Withdrawn,
                    code: RejectionReason::QueryResultEncodingFailed
                        .number_in(Partition::Subscription),
                    anchor: circular_core::encode(
                        &reader.last,
                        Ceilings::for_boundary(Boundary::Wire),
                    )
                    .unwrap_or_default(),
                },
            );
            super::PumpResult::Ended
        }
    }
}

