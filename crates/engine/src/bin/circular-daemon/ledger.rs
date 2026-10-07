#[path = "ledger_projection.rs"]
pub(crate) mod projection;
#[path = "ledger_query_cut.rs"]
mod query_cut;
#[path = "ledger_timeline.rs"]
mod timeline;
pub(crate) use query_cut::ReadBound;
pub(crate) use timeline::unrecorded_timeline_bins;

use crate::authoring_assembly::projection::AuthoredProjection;
use crate::authoring_assembly::projection::fold_projection;
use circular_actors::{BaseShape, GroundShape, ProductPayload, Shape};
#[cfg(test)]
use circular_core::Sequence;
use circular_core::{BuiltinObservationName, Value};
use circular_core::{
    EncodedPayload, NonZeroTicks, PayloadVersionTag, RecordedInstant, Stamp, Tick, TickSource,
    TicksPerSecond, TimeSourceKind, TimeSourcePlan, WallAnchored,
};
use circular_plan::{
    ActorId, DeclaredScopeSeg, ExportName, NamedActorId, PortId, Role, ScopeId, ScopeRole,
};
use circular_protocol::actor_events::{ActorHealthReason, ActorHealthReasonCode, ActorHealthState};
use circular_protocol::authoring_snapshot::environment_value;
use circular_protocol::declaration_payload::{
    AuthoringEnvironment, PlanExportKey, decode_environment,
};
use circular_protocol::{InjectionLedger, InjectionLookup};
#[cfg(test)]
use circular_store::{AppendBatch, AppendResult};
use circular_store::{
    BoundaryFact, BoundaryKey, ClassKey, FailureParams, LiveFrame, ManifestGroups, ObservationKey,
    OpaqueId, PlacementParams, ProductStore, Record, RevisionContext, RevisionStart, RunInputs,
    RunManifest, Store, StreamId, StructureFact, TimeParams,
};
use engine::authoring_assembly::ledger::AuthoringSnapshot;
use engine::execution_profile::ProductExecutionProfile;
use engine::{
    RequestExportIngress, RevisionEpochId, RuntimeApprovalQueue, RuntimeApprovalQueueSnapshot,
    observation_mount_actor, request_mount_actors,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub(crate) fn record_witness(
    header: &circular_store::RecordHeader<ProductStore>,
) -> Result<circular_store::OpaqueWitness, String> {
    use circular_store::{ObservationKey, OpaqueWitness, ProductRecordCodec};
    let witness = OpaqueWitness::for_record_ref(header.class(), header.key(), &ProductRecordCodec)
        .map_err(|e| format!("record reference: {e:?}"))?;
    if let ClassKey::Boundary(BoundaryKey::EmissionBody { .. }) = header.key() {
        let mut bytes = witness.as_bytes().to_vec();
        circular_store::push_stamp(&mut bytes, header.at())
            .map_err(|e| format!("record stamp: {e:?}"))?;
        return OpaqueWitness::from_bytes(&bytes).map_err(|e| format!("record reference: {e:?}"));
    }
    let ClassKey::Observation(key) = header.key() else {
        return Ok(witness);
    };
    let mut bytes = witness.as_bytes().to_vec();
    let at = match key {
        ObservationKey::StreamItem(at, _, _) => {
            at
        }
        ObservationKey::GlobalItem(at, _, _) => at,
        ObservationKey::CheckpointItem(coordinate, _, _) => {
            let namespace = coordinate.namespace().as_bytes();
            let length = u32::try_from(namespace.len())
                .map_err(|_| "record coordinate namespace is too long".to_owned())?;
            bytes.extend(length.to_be_bytes());
            bytes.extend(namespace);
            bytes.extend(coordinate.commit().to_be_bytes());
            bytes.extend(coordinate.operation().to_be_bytes());
            bytes.extend(coordinate.at().to_be_bytes());
            return OpaqueWitness::from_bytes(&bytes)
                .map_err(|e| format!("record reference: {e:?}"));
        }
    };
    circular_store::push_stamp(&mut bytes, at).map_err(|e| format!("record stamp: {e:?}"))?;
    OpaqueWitness::from_bytes(&bytes).map_err(|e| format!("record reference: {e:?}"))
}

pub(crate) fn actor_health_producer() -> NamedActorId {
    NamedActorId::new(
        circular_plan::ScopeId::root(),
        circular_plan::Name::from_normalized("actor-health-transitions"),
    )
}

/// The record owner and System outlive user assembly and recovery failures.
/// World retains this handle before asking System to prepare any user actors.
pub(crate) struct SystemRuntime {
    pub(crate) stream: StreamId,
    pub(crate) pipeline: crate::kernel::system::Pipeline,
    pub(crate) journal: engine::ProductDurableArrivalJournal,
    approvals: RuntimeApprovalQueue,
}

impl SystemRuntime {
    fn start(
        stream: StreamId,
        journal: engine::ProductDurableArrivalJournal,
        clock: Arc<dyn TickSource>,
        execution: &ProductExecutionProfile,
        directory: &Path,
    ) -> Result<Arc<Self>, String> {
        journal.set_record_wake(execution.publication_wake());
        let approvals = RuntimeApprovalQueue::open(journal.clone())?;
        let execution = crate::daemon::environment::with_saved_agent_bindings(execution, directory);
        let pipeline = crate::kernel::system::Pipeline::start(
            clock,
            stream,
            journal.clone(),
            Arc::new(execution),
            &approvals,
        )
        .map_err(|error| format!("failed to start the actor kernel: {error}"))?;
        Ok(Arc::new(Self {
            stream,
            pipeline,
            journal,
            approvals,
        }))
    }

    /// Reopen the recorded stream before interpreting any user's declarations.
    pub(crate) fn reopen(
        directory: &Path,
        stream: StreamId,
        execution: &ProductExecutionProfile,
    ) -> Result<(Arc<Self>, Tick), String> {
        let path = engine::state_journal::state_journal_path(directory);
        let journal = engine::ProductDurableArrivalJournal::open_unseeded(path, stream)?;
        let view = journal.read_view();
        let mut offset = 0;
        for row in view.all_rows()? {
            let row = row?;
            let record = row.record();
            offset = offset.max(record.header().at().physical_time().get());
        }
        let clock = Arc::new(
            WallAnchored::new(
                NonZeroTicks::new(1).unwrap(),
                TicksPerSecond::new(1_000).unwrap(),
            )
            .starting_at(Tick::new(offset)),
        );
        Ok((
            Self::start(stream, journal, clock, execution, directory)?,
            Tick::new(offset),
        ))
    }

    pub(crate) fn read_view(&self) -> circular_store::JournalView {
        self.journal.read_view()
    }
}

pub(crate) struct ServerRun {
    run: StreamId,
    pipeline: crate::kernel::system::Pipeline,
    stood_actors: BTreeSet<NamedActorId>,
    store: circular_store::JournalView,
    arrival_journal: Option<engine::ProductDurableArrivalJournal>,
    journal_limits: Option<crate::daemon::runtime_arrival_retention::RuntimeArrivalLimits>,
    recorded: usize,
    /// The revision this run stands at: the last GraphRevision it recorded or
    /// recovered. Record sequences belong to System's issuer.
    revisions: u64,
    journal_ceiling_recorded: Option<Option<String>>,
    unboarded: Vec<String>,
    templates: Vec<String>,
    plan: Arc<AuthoredProjection>,
    ports: Arc<engine::ResolvedRevisionPorts>,
    last_recorded_instant_ms: u64,
    execution: ProductExecutionProfile,
    entrances: MountEntrances,
    /// Canonical approval custody shared with live producers and the run journal.
    approvals: RuntimeApprovalQueue,
}

/// Immutable, consistently sealed journal prefix. It owns no runtime driver.
#[derive(Clone)]
pub(crate) struct ServerRead {
    journal: Option<engine::ProductDurableArrivalJournal>,
    input: Option<ServerIngress>,
    approval_owner: Option<RuntimeApprovalQueue>,
    checkpoint_reader: Option<engine::ProductCheckpointReader>,
    run: StreamId,
    stood_actors: BTreeSet<NamedActorId>,
    store: circular_store::JournalView,
    recorded: usize,
    revisions: u64,
    unboarded: Vec<String>,
    templates: Vec<String>,
    plan: Arc<AuthoredProjection>,
    execution: ProductExecutionProfile,
}
impl ServerRun {
    pub(crate) fn same_read_prefix(&self, old: &ServerRead) -> bool {
        let mark = self
            .arrival_journal
            .as_ref()
            .map_or_else(|| self.mark(), |journal| journal.read_view().surface_mark());
        self.run == old.run
            && mark == old.mark()
            && self.recorded == old.recorded
            && self.revisions == old.revisions
    }

    pub(crate) fn read_prefix(&self) -> ServerRead {
        ServerRead {
            journal: self.arrival_journal.clone(),
            input: self.journal_limits.map(|limits| self.ingress(limits)),
            approval_owner: Some(self.approvals.clone()),
            checkpoint_reader: self
                .arrival_journal
                .as_ref()
                .map(|journal| journal.checkpoint_reader()),
            run: self.run.clone(),
            stood_actors: self.stood_actors.clone(),
            store: self.store.clone(),
            recorded: self.recorded.clone(),
            revisions: self.revisions.clone(),
            unboarded: self.unboarded.clone(),
            templates: self.templates.clone(),
            plan: self.plan.clone(),
            execution: self.execution.clone(),
        }
    }
}
impl ServerRead {
    pub(crate) fn caught_up(&self) -> Option<Self> {
        let view = self.journal.as_ref()?.read_view();
        (view.surface_mark() != self.store.surface_mark()).then(|| Self {
            store: view,
            ..self.clone()
        })
    }
    pub(crate) fn arrival_prefix(&self) -> circular_store::JournalView {
        self.checkpoint_reader
            .as_ref()
            .and_then(engine::ProductCheckpointReader::published_view)
            .unwrap_or_else(|| self.store.clone())
    }
    pub(crate) fn approval_snapshot(&self) -> Result<RuntimeApprovalQueueSnapshot, String> {
        self.approval_owner.as_ref().map_or_else(
            || RuntimeApprovalQueue::new().snapshot(),
            RuntimeApprovalQueue::snapshot,
        )
    }
}
fn arrival_origin_stamp(
    record: &circular_store::BoundaryRecord<ProductStore>,
) -> Option<&Stamp<ActorId>> {
    let BoundaryFact::Arrival {
        origin,
        causal_parents,
        ..
    } = record.fact()
    else {
        return None;
    };
    match origin.as_ref() {
        circular_store::ArrivalOrigin::EdgeDelivery { sender, .. } => Some(sender),
        circular_store::ArrivalOrigin::ExternalInject { .. } => {
            causal_parents.last().map(circular_core::EventId::stamp)
        }
        _ => Some(record.header().at()),
    }
}

#[cfg(test)]
pub(crate) fn arrival_digest(row: &Value) -> Value {
    let fields = row.as_object().expect("arrival row is an object");
    Value::object(["role", "kind", "origin", "body"].map(|key| {
        (
            key,
            fields
                .get(key)
                .unwrap_or_else(|| panic!("arrival row has `{key}`"))
                .clone(),
        )
    }))
    .expect("four distinct keys")
}

#[cfg(test)]
pub(crate) fn external_arrival_digest(role: &str, origin: &[u8], body: Value) -> Value {
    Value::object([
        ("role", Value::string(role)),
        ("kind", Value::Int(4)),
        ("origin", Value::bytes(origin.to_vec())),
        ("body", body),
    ])
    .expect("four distinct keys")
}

include!("ledger_read.rs");

#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use]
pub(crate) enum PlanAdoptionOutcome {
    Adopted,
    Behind { reason: String },
}

pub(crate) enum PreparedAdoption {
    Settled {
        target: RevisionEpochId,
        outcome: PlanAdoptionOutcome,
    },
    Ready(Box<ReadyAdoption>),
}

pub(crate) struct ReadyAdoption {
    target: RevisionEpochId,
    authoring_cut: GraphRevisionCut,
    plan: Arc<AuthoredProjection>,
    ports: Arc<engine::ResolvedRevisionPorts>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GraphRevisionCut {
    cursor: u64,
    environment: AuthoringEnvironment,
    pub(crate) authoring_revision: Vec<u8>,
    topology_revision: Vec<u8>,
}

impl GraphRevisionCut {
    fn from_snapshot(snapshot: &AuthoringSnapshot) -> Result<Self, String> {
        if !snapshot.scope.is_empty() {
            return Err("GraphRevision authoring cut is not the project root".to_owned());
        }
        if snapshot.cursor == 0 {
            return Err("GraphRevision authoring cursor is zero".to_owned());
        }
        let authoring_revision = snapshot
            .authoring_revision
            .clone()
            .ok_or_else(|| "GraphRevision authoring revision is absent".to_owned())?;
        if authoring_revision.len() != 32 {
            return Err("GraphRevision authoring revision is not 32 bytes".to_owned());
        }
        let topology_revision = snapshot
            .topology_revision
            .clone()
            .ok_or_else(|| "GraphRevision topology revision is unavailable".to_owned())?;
        if topology_revision.len() != 32 {
            return Err("GraphRevision topology revision is not 32 bytes".to_owned());
        }
        Ok(Self {
            cursor: snapshot.cursor,
            environment: snapshot.environment.clone(),
            authoring_revision,
            topology_revision,
        })
    }

    fn manifest_cut(
        &self,
        project: [u8; 32],
    ) -> Result<circular_store::ProductAuthoringCut, String> {
        Ok(circular_store::AuthoringCut {
            project,
            cursor: self.cursor,
            environment: self.environment.clone(),
            authoring_revision: circular_protocol::RevisionDigest::try_from_bytes(
                &self.authoring_revision,
            )
            .map_err(|_| "manifest authoring revision is not 32 bytes".to_owned())?,
            topology_revision: circular_protocol::RevisionDigest::try_from_bytes(
                &self.topology_revision,
            )
            .map_err(|_| "manifest topology revision is not 32 bytes".to_owned())?,
        })
    }

    fn to_value(&self) -> Result<Value, String> {
        Value::object([
            (
                "authoring_revision",
                Value::Bytes(self.authoring_revision.clone()),
            ),
            ("cursor", Value::UInt(self.cursor)),
            ("environment", environment_value(&self.environment)?),
            ("project", Value::Null),
            (
                "topology_revision",
                Value::Bytes(self.topology_revision.clone()),
            ),
        ])
        .map_err(|error| format!("GraphRevision authoring cut: {error:?}"))
    }

    pub(crate) fn from_value(value: Value) -> Result<Self, String> {
        let Value::Object(object) = value else {
            return Err("GraphRevision payload is not an object".to_owned());
        };
        let mut fields = object.into_map();
        let authoring_revision = graph_revision_bytes(
            graph_revision_field(&mut fields, "authoring_revision")?,
            "authoring_revision",
        )?;
        let cursor = match graph_revision_field(&mut fields, "cursor")? {
            Value::UInt(cursor) if cursor != 0 => cursor,
            _ => return Err("GraphRevision cursor is not a nonzero UInt".to_owned()),
        };
        let environment = decode_environment(graph_revision_field(&mut fields, "environment")?)
            .map_err(|error| format!("GraphRevision environment: {error:?}"))?;
        if graph_revision_field(&mut fields, "project")? != Value::Null {
            return Err("GraphRevision project identity carrier is unavailable".to_owned());
        }
        let topology_revision = graph_revision_bytes(
            graph_revision_field(&mut fields, "topology_revision")?,
            "topology_revision",
        )?;
        if let Some((unknown, _)) = fields.into_iter().next() {
            return Err(format!("unknown GraphRevision field {unknown:?}"));
        }
        Ok(Self {
            cursor,
            environment,
            authoring_revision,
            topology_revision,
        })
    }

    fn verify_snapshot(&self, snapshot: &AuthoringSnapshot) -> Result<(), String> {
        let found = Self::from_snapshot(snapshot)?;
        if &found == self {
            Ok(())
        } else {
            Err("GraphRevision witnesses disagree with the replayed authoring cut".to_owned())
        }
    }
}

fn graph_revision_field(fields: &mut BTreeMap<String, Value>, name: &str) -> Result<Value, String> {
    fields
        .remove(name)
        .ok_or_else(|| format!("GraphRevision payload has no {name}"))
}

fn graph_revision_bytes(value: Value, name: &str) -> Result<Vec<u8>, String> {
    match value {
        Value::Bytes(bytes) if bytes.len() == 32 => Ok(bytes),
        _ => Err(format!("GraphRevision {name} is not 32 bytes")),
    }
}

#[cfg(test)]
fn test_graph_revision_cut(cursor: u64) -> GraphRevisionCut {
    GraphRevisionCut {
        cursor,
        environment: AuthoringEnvironment {
            declaration_schema: vec![0xa1],
            spec_set: vec![0xa2],
        },
        authoring_revision: vec![0xa3; 32],
        topology_revision: vec![0xa4; 32],
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServerReplayCheckpoint {
    revision: RevisionEpochId,
    at: RecordedInstant,
    cut: circular_runtime::LogCut,
}

impl ServerReplayCheckpoint {
    pub(crate) const fn revision(&self) -> RevisionEpochId {
        self.revision
    }

    pub(crate) const fn at(&self) -> RecordedInstant {
        self.at
    }

    pub(crate) const fn cut(&self) -> &circular_runtime::LogCut {
        &self.cut
    }
}

type RecordedColumns = engine::recorded::RecordedColumns;

pub(crate) const RECOVERY_SHUTDOWN_REQUESTED: &str =
    "shutdown requested during recovery; the retained stream is untouched";

#[derive(Clone, Copy)]
pub(crate) struct RecoveryInterrupt(fn() -> bool);

impl RecoveryInterrupt {
    pub(crate) fn process() -> Self {
        Self(crate::daemon::shutdown::requested)
    }

    pub(crate) const fn new(observe: fn() -> bool) -> Self {
        Self(observe)
    }

    fn observed(self) -> Result<(), String> {
        if (self.0)() {
            return Err(RECOVERY_SHUTDOWN_REQUESTED.to_owned());
        }
        Ok(())
    }
}

pub(crate) struct RecordedPrefix {
    run: StreamId,
    records: Vec<Record<ProductStore>>,
    snapshots: Vec<(RevisionEpochId, Arc<AuthoredProjection>)>,
    recorded: RecordedColumns,
    admitted: BTreeMap<NamedActorId, Vec<crate::kernel::column::Admitted>>,
    outcomes: BTreeMap<NamedActorId, Vec<crate::kernel::column::Outcome>>,
    refused: BTreeMap<NamedActorId, String>,
    lives: crate::kernel::record::LifeStarts,
    interrupt: RecoveryInterrupt,
}

impl RecordedPrefix {
    pub(crate) fn read_from(
        state_directory: &Path,
        journal: &engine::ProductDurableArrivalJournal,
        run: StreamId,
        interrupt: RecoveryInterrupt,
    ) -> Result<Self, String> {
        let authoring_store = crate::daemon::authoring_store::AuthoringStore::open(state_directory)
            .map_err(|e| e.to_string())?;
        Self::read_after(journal, &authoring_store, run, interrupt)
    }

    fn read_after(
        journal: &engine::ProductDurableArrivalJournal,
        authoring_store: &crate::daemon::authoring_store::AuthoringStore,
        run: StreamId,
        interrupt: RecoveryInterrupt,
    ) -> Result<Self, String> {
        interrupt.observed()?;
        let records = journal
            .read_prefix()
            .records()
            .iter()
            .map(std::borrow::Cow::into_owned)
            .collect::<Vec<_>>();
        interrupt.observed()?;
        if records.is_empty() {
            return Err(format!(
                "recorded run {} arrival prefix is not retained; replay unavailable",
                run.get()
            )
            .into());
        }
        let adopted = records
            .iter()
            .filter_map(|record| match record {
                Record::Structure(structure) => match structure.fact() {
                    StructureFact::GraphRevision(payload) => Some(payload),
                    _ => None,
                },
                _ => None,
            })
            .map(|payload| {
                let value = circular_core::decode(
                    payload.body(),
                    circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
                )
                .map_err(|error| format!("GraphRevision payload does not decode: {error:?}"))?;
                GraphRevisionCut::from_value(value)
            })
            .collect::<Result<Vec<_>, String>>()?;
        interrupt.observed()?;
        if adopted.is_empty() {
            return Err(format!(
                "recorded run {} has no GraphRevision authoring cursor; a journal recorded before that cursor existed cannot be replayed",
                run.get()
            )
            .into());
        }
        let mut snapshots = Vec::new();
        let stood = adopted
            .iter()
            .map(|cut| {
                RevisionEpochId::new(cut.cursor)
                    .map(|revision| (revision, cut))
                    .ok_or_else(|| "GraphRevision cursor is zero".to_owned())
            })
            .collect::<Result<Vec<_>, String>>()?;
        for (revision, cut) in stood {
            interrupt.observed()?;
            let replayed = authoring_store.load_at(cut.cursor)?;
            let snapshot = replayed
                .snapshot(Vec::new())
                .map_err(|rejection| rejection.to_string())?;
            cut.verify_snapshot(&snapshot)?;
            if snapshots.iter().any(|(old, _)| *old == revision) {
                return Err("duplicate GraphRevision cursor".into());
            }
            let plan = Arc::new(
                replayed
                    .current_plan()
                    .map_err(|rejection| rejection.to_string())?,
            );
            snapshots.push((revision, plan));
        }
        let lives = crate::kernel::record::LifeStarts::of(&records);
        let RecordedInputs {
            columns: recorded,
            admitted,
            outcomes,
            refused,
        } = stopped_run_arrivals_after(&records, &snapshots, &lives, interrupt)?;
        interrupt.observed()?;
        Ok(Self {
            run,
            records,
            snapshots,
            recorded,
            admitted,
            outcomes,
            refused,
            lives,
            interrupt,
        })
    }

    fn into_recorded(self) -> (RecordedColumns, Vec<Record<ProductStore>>) {
        (self.recorded, self.records)
    }
}

type RestoredPrefix = (
    crate::kernel::system::Restoration,
    Vec<Record<ProductStore>>,
);

pub(crate) struct StoodPrefix {
    prefix: RecordedPrefix,
}

/// User recovery is a System request. Preparation failures are recorded by the
/// same System that remains available to explicit Resume and queries.
pub(crate) fn recover_at_boot(
    state_directory: &Path,
    execution: &ProductExecutionProfile,
    system: &Arc<SystemRuntime>,
    interrupt: RecoveryInterrupt,
) -> Result<Option<ServerRun>, String> {
    let state = system
        .pipeline
        .system_state()
        .map_err(|_| "System stopped before boot recovery")?;
    if state
        .last_recovery
        .as_ref()
        .is_some_and(|(_, result)| result.is_err())
    {
        return Ok(None);
    }
    recover_execution(state_directory, execution, system, interrupt).map(Some)
}

pub(crate) fn recover_execution(
    state_directory: &Path,
    execution: &ProductExecutionProfile,
    system: &Arc<SystemRuntime>,
    interrupt: RecoveryInterrupt,
) -> Result<ServerRun, String> {
    let view = system.read_view();
    let revision = view.all_rows()?.try_fold(None, |last, row| {
        let row = row?;
        let record = row.record();
        Ok::<_, String>(if matches!(&*record, Record::Structure(s) if matches!(s.fact(), StructureFact::GraphRevision(_))) {
            Some(record.header().at().revision())
        } else { last })
    })?.ok_or("restored journal has no GraphRevision")?;
    let directory = state_directory.to_path_buf();
    let execution = execution.clone();
    let run = system.stream;
    let journal = system.journal.clone();
    let (send, receive) = std::sync::mpsc::channel();
    let stood = system.pipeline.recover(revision, move || {
        let prefix = RecordedPrefix::read_from(&directory, &journal, run, interrupt)?;
        let (_, plan) = prefix
            .snapshots
            .last()
            .ok_or("restored journal has no GraphRevision")?;
        let plan = plan.as_ref().clone();
        let authoring = crate::daemon::authoring_store::AuthoringStore::open(&directory)?;
        let snapshot = authoring
            .load_at(revision.get())?
            .snapshot(Vec::new())
            .map_err(|error| error.to_string())?;
        let mut pending = ServerRun::prepare_boundary_at_authoring_cut(
            plan,
            revision,
            run,
            execution,
            &directory,
            &snapshot,
            StoodPrefix { prefix },
        )?;
        pending.ports =
            Some(engine::ResolvedRevisionPorts::resolve(&pending.plan).map_err(|e| e.to_string())?);
        let restoration = pending
            .restoration
            .take()
            .ok_or("recovery has no actor columns")?;
        send.send(pending)
            .map_err(|_| "recovery preparation receiver stopped")?;
        Ok(restoration)
    })?;
    let pending = receive.recv().map_err(|_| "recovery preparation stopped")?;
    pending.finish(system, stood)
}

struct RecordedInputs {
    columns: RecordedColumns,
    admitted: BTreeMap<NamedActorId, Vec<crate::kernel::column::Admitted>>,
    outcomes: BTreeMap<NamedActorId, Vec<crate::kernel::column::Outcome>>,
    refused: BTreeMap<NamedActorId, String>,
}

enum ReadRow {
    Arrival(engine::recorded::RecordedArrival),
    Outcome(crate::kernel::column::Outcome),
    Admitted(crate::kernel::column::Admitted),
}

fn stopped_run_arrivals_after(
    records: &[Record<ProductStore>],
    snapshots: &[(RevisionEpochId, Arc<AuthoredProjection>)],
    lives: &crate::kernel::record::LifeStarts,
    interrupt: RecoveryInterrupt,
) -> Result<RecordedInputs, String> {
    let ports = snapshots
        .iter()
        .map(|(revision, plan)| {
            interrupt.observed()?;
            engine::ResolvedRevisionPorts::resolve(plan)
                .map(|ports| (*revision, ports))
                .map_err(|error| format!("recorded port resolution: {error:?}"))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let borrowed = ports.iter().map(|(r, ports)| (*r, ports)).collect();
    stopped_run_arrivals_from(records, snapshots, &borrowed, lives, interrupt)
}

fn stopped_run_arrivals_from(
    records: &[Record<ProductStore>],
    snapshots: &[(RevisionEpochId, Arc<AuthoredProjection>)],
    ports_by_revision: &BTreeMap<RevisionEpochId, &engine::ResolvedRevisionPorts>,
    lives: &crate::kernel::record::LifeStarts,
    interrupt: RecoveryInterrupt,
) -> Result<RecordedInputs, String> {
    let mut outcomes = BTreeMap::<NamedActorId, Vec<crate::kernel::column::Outcome>>::new();
    let scopes = snapshots
        .iter()
        .map(|(revision, plan)| {
            interrupt.observed()?;
            Ok((
                *revision,
                crate::authoring_assembly::projection::scope_roles(plan),
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let timer = GroundShape::try_new(Shape::Base(BaseShape::UInt)).expect("UInt is ground");
    let lifecycle = GroundShape::try_new(Shape::Any).expect("Any is ground");
    let mut columns: RecordedColumns = BTreeMap::new();
    let mut indices = BTreeMap::<NamedActorId, Vec<circular_core::ArrivalIndex>>::new();
    let mut admitted = BTreeMap::<NamedActorId, Vec<crate::kernel::column::Admitted>>::new();
    let mut delivered =
        std::collections::HashSet::<(NamedActorId, circular_plan::EdgeId, Stamp<ActorId>)>::new();
    let mut refused = BTreeMap::<NamedActorId, String>::new();
    for record in records {
        interrupt.observed()?;
        let Record::Boundary(boundary) = record else {
            continue;
        };
        let ClassKey::Boundary(
            BoundaryKey::Arrival { actor, .. } | BoundaryKey::Admission { actor, .. },
        ) = boundary.header().key()
        else {
            continue;
        };
        let ActorId::Scoped {
            scope,
            local: circular_plan::LocalKey::Named(name),
        } = actor
        else {
            return Err("stopped-run journal contains an unnamed arrival actor".into());
        };
        let life = lives.get(actor);
        let actor = NamedActorId::new(scope.clone(), name.clone());
        let (
            result,
            origin,
            body,
            arrival_index,
            causal_parents,
            observed_at,
            recorded_inlet,
            route_edge,
        ) = match boundary.fact() {
            BoundaryFact::Arrival {
                result,
                origin,
                body,
                arrival_index,
                causal_parents,
                observed_at,
                inlet,
                route_edge,
            } => (
                result,
                origin,
                body,
                Some(arrival_index),
                causal_parents,
                observed_at,
                inlet,
                route_edge,
            ),
            BoundaryFact::Admission {
                result,
                origin,
                body,
                causal_parents,
                observed_at,
                inlet,
                route_edge,
            } => (
                result,
                origin,
                body,
                None,
                causal_parents,
                observed_at,
                inlet,
                route_edge,
            ),
            _ => continue,
        };
        if let Some(arrival_index) = arrival_index {
            if let circular_store::ArrivalOrigin::EdgeDelivery { edge, sender } = origin.as_ref() {
                delivered.insert((actor.clone(), edge.clone(), sender.clone()));
            }
            if life.is_some_and(|life| *arrival_index < life.index()) {
                continue;
            }
            indices
                .entry(actor.clone())
                .or_default()
                .push(*arrival_index);
        }
        let read = || -> Result<ReadRow, String> {
            let revision = boundary.header().at().revision();
            let plan = scopes.get(&revision).ok_or("arrival revision missing")?;
            let ports = ports_by_revision
                .get(&revision)
                .ok_or("arrival references an unknown revision")?;
            let emission_shape =
                |from: &circular_plan::Endpoint, sender: Option<&Stamp<ActorId>>| {
                    let revision =
                        sender.map_or(boundary.header().at().revision(), Stamp::revision);
                    let scopes = scopes
                        .get(&revision)
                        .ok_or("emission scope revision missing")?;
                    let ports = ports_by_revision
                        .get(&revision)
                        .ok_or("emission port revision missing")?;
                    if sender.is_some_and(|sender| sender.producer() == &from.actor().as_actor_id())
                    {
                        return stopped_port_shape(scopes, ports, from.actor(), from.port(), false);
                    }
                    let projection = &snapshots
                        .iter()
                        .find(|(r, _)| *r == revision)
                        .ok_or("emission revision missing")?
                        .1;
                    stopped_emission_shape(
                        projection,
                        scopes,
                        ports,
                        from,
                        sender.map(Stamp::producer),
                    )
                };
            let (runtime_origin, inlet, shape) = match origin.as_ref() {
                circular_store::ArrivalOrigin::EdgeDelivery { edge, sender } => {
                    let inlet = recorded_inlet
                        .clone()
                        .ok_or_else(|| "edge arrival has no recorded inlet".to_owned())?;
                    let shape = match edge {
                        circular_plan::EdgeId::Declared { from, .. } => {
                            emission_shape(from, Some(sender))?
                        }
                        circular_plan::EdgeId::Outcome { target }
                            if target == &actor
                                && inlet.as_str() == circular_actors::LIFECYCLE_PORT_NAME =>
                        {
                            lifecycle.clone()
                        }
                        circular_plan::EdgeId::Outcome { target } if target == &actor => {
                            stopped_port_shape(plan, ports, &actor, &inlet, true)?
                        }
                        circular_plan::EdgeId::Outcome { .. } => {
                            return Err("reserved ingress target differs from arrival actor".into());
                        }
                    };
                    (
                        circular_runtime::ArrivalOrigin::EdgeDelivery {
                            edge: edge.clone(),
                            stamp: sender.clone(),
                        },
                        inlet,
                        shape,
                    )
                }
                circular_store::ArrivalOrigin::TimerFire { timer: timer_id } => (
                    circular_runtime::ArrivalOrigin::TimerFire {
                        timer: timer_id.clone(),
                    },
                    recorded_inlet
                        .clone()
                        .ok_or("timer arrival has no recorded inlet")?,
                    timer.clone(),
                ),
                circular_store::ArrivalOrigin::ExternalInject { origin } => (
                    circular_runtime::ArrivalOrigin::ExternalInject {
                        origin: circular_runtime::ExternalOrigin::try_new(origin.body().to_vec())
                            .map_err(|_| "recorded external origin is empty".to_owned())?,
                    },
                    recorded_inlet
                        .clone()
                        .ok_or_else(|| "external arrival has no recorded inlet".to_owned())?,
                    match route_edge {
                        Some(circular_plan::EdgeId::Declared { from, .. }) => {
                            emission_shape(from, None)?
                        }
                        Some(_) => {
                            return Err("external arrival route is not a declared edge".into());
                        }
                        None => stopped_port_shape(
                            plan,
                            ports,
                            &actor,
                            recorded_inlet.as_ref().ok_or("external inlet missing")?,
                            true,
                        )?,
                    },
                ),
                circular_store::ArrivalOrigin::EffectOutcome { .. } => {
                    let index =
                        arrival_index.ok_or("a recorded effect outcome is not an arrival")?;
                    let recorded = engine::effect_outcome_record::read_effect_outcome(record)?
                        .ok_or("a recorded effect outcome carries no owned summary")?;
                    return Ok(ReadRow::Outcome(crate::kernel::column::Outcome {
                        index: *index,
                        at: boundary.header().at().clone(),
                        observed_at: *observed_at,
                        outcome: circular_runtime::EffectOutcome::new(
                            recorded.effect,
                            recorded.result,
                        )
                        .with_failure_progress(recorded.failure_progress),
                    }));
                }
            };
            let body = match body {
                circular_store::ArrivalBody::Owned(payload) => {
                    engine::recorded::RecordedPayload::new(shape, payload.clone())
                }
                circular_store::ArrivalBody::Emitted { .. } => {
                    return Err("stopped-run journal carries an unresolved arrival body".into());
                }
            };
            let parents = causal_parents
                .iter()
                .map(|parent| parent.stamp().clone())
                .collect::<Vec<_>>()
                .into_boxed_slice();
            let Some(arrival_index) = arrival_index else {
                let circular_runtime::ArrivalOrigin::EdgeDelivery { edge, stamp } = runtime_origin
                else {
                    return Err("a recorded admission is not an owned wire delivery".into());
                };
                return Ok(ReadRow::Admitted(crate::kernel::column::Admitted {
                    edge,
                    inlet,
                    sender: stamp,
                    parents,
                    payload: body,
                    result: result.clone(),
                    observed_at: *observed_at,
                }));
            };
            let arrival = engine::recorded::RecordedArrival::new(
                actor.clone(),
                boundary.header().at().clone(),
                *arrival_index,
                runtime_origin,
                inlet,
                body,
                *observed_at,
            )
            .with_causal_parents(parents)
            .with_route_edge(route_edge.clone())
            .with_result(result.clone());
            Ok(ReadRow::Arrival(arrival))
        };
        match read() {
            Ok(ReadRow::Arrival(arrival)) => columns.entry(actor).or_default().push(arrival),
            Ok(ReadRow::Admitted(admission)) => {
                admitted.entry(actor).or_default().push(admission);
            }
            Ok(ReadRow::Outcome(outcome)) => outcomes.entry(actor).or_default().push(outcome),
            Err(reason) => {
                refused.entry(actor).or_insert(reason);
            }
        }
    }
    for column in columns.values_mut() {
        column.sort_by_key(engine::recorded::RecordedArrival::index);
    }
    for (actor, indices) in &mut indices {
        indices.sort();
        let first = lives
            .get(&actor.as_actor_id())
            .map_or(0, |life| life.index().get());
        if indices
            .iter()
            .enumerate()
            .any(|(offset, index)| index.get() != first + offset as u64)
        {
            refused
                .entry(actor.clone())
                .or_insert_with(|| format!("recorded arrival column for {actor:?} is not dense"));
        }
    }
    for (actor, admissions) in &mut admitted {
        admissions.retain(|admission| {
            !delivered.contains(&(
                actor.clone(),
                admission.edge.clone(),
                admission.sender.clone(),
            ))
        });
    }
    admitted.retain(|_, admissions| !admissions.is_empty());
    Ok(RecordedInputs {
        columns,
        admitted,
        outcomes,
        refused,
    })
}

fn stopped_emission_shape(
    projection: &AuthoredProjection,
    scopes: &circular_plan::ScopeRoleTable,
    ports: &engine::ResolvedRevisionPorts,
    from: &circular_plan::Endpoint,
    producer: Option<&ActorId>,
) -> Result<GroundShape, String> {
    use circular_plan::{Endpoint, LocalKey, ScopeSeg};
    let declared = |actor: &NamedActorId| {
        circular_plan::admit_runtime_scope(scopes, actor.scope())
            .map(|scope| NamedActorId::new(scope.declared().clone(), actor.name().clone()))
            .map_err(|error| format!("recorded runtime scope: {error}"))
    };
    let producer = match producer {
        Some(ActorId::Scoped {
            scope,
            local: LocalKey::Named(name),
        }) => Some(declared(&NamedActorId::new(scope.clone(), name.clone()))?),
        Some(_) => return Err("recorded emission producer is not a named actor".into()),
        None => None,
    };
    let mut pending = vec![Endpoint::new(declared(from.actor())?, from.port().clone())];
    let mut visited = Vec::new();
    let mut shape = None;
    while let Some(endpoint) = pending.pop() {
        if visited.contains(&endpoint) {
            continue;
        }
        visited.push(endpoint.clone());
        let mut layer = projection;
        for segment in endpoint.actor().scope().segments() {
            let ScopeSeg::Child(name) = segment else {
                unreachable!("admitted declaration scope")
            };
            layer = layer
                .graph
                .scopes
                .get(&DeclaredScopeSeg::child(name.clone()))
                .ok_or("recorded emission scope is absent from its revision")?;
        }
        let declaration = layer
            .graph()
            .actors()
            .get(endpoint.actor())
            .ok_or("recorded emission actor is absent from its revision")?;
        if declaration.domain().actor_type().is_container() {
            let child = layer
                .graph
                .scopes
                .get(&DeclaredScopeSeg::child(endpoint.actor().name().clone()))
                .ok_or("recorded emission container has no scope")?;
            let outlets = child.boundary().outlets();
            pending.extend(
                outlets
                    .iter()
                    .filter(|(port, _)| {
                        crate::run_graph::container_boundary_outlet(
                            declaration,
                            outlets.len(),
                            port,
                        ) == *endpoint.port()
                    })
                    .map(|(_, inner)| inner.clone()),
            );
        } else if producer
            .as_ref()
            .is_none_or(|producer| producer == endpoint.actor())
        {
            let next =
                stopped_declared_port_shape(ports, endpoint.actor(), endpoint.port(), false)?;
            if shape.as_ref().is_some_and(|shape| shape != &next) {
                return Err("recorded emission has ambiguous original outlet shapes".into());
            }
            shape = Some(next);
        }
    }
    shape.ok_or_else(|| "recorded emission has no original outlet in its revision".into())
}

fn stopped_port_shape(
    scopes: &circular_plan::ScopeRoleTable,
    ports: &engine::ResolvedRevisionPorts,
    actor: &NamedActorId,
    port: &PortId,
    input: bool,
) -> Result<GroundShape, String> {
    let admitted = circular_plan::admit_runtime_scope(scopes, actor.scope())
        .map_err(|error| format!("recorded runtime scope: {error}"))?;
    let actor = NamedActorId::new(admitted.declared().clone(), actor.name().clone());
    stopped_declared_port_shape(ports, &actor, port, input)
}

fn stopped_declared_port_shape(
    ports: &engine::ResolvedRevisionPorts,
    actor: &NamedActorId,
    port: &PortId,
    input: bool,
) -> Result<GroundShape, String> {
    if !ports.contains_actor(actor) {
        return Err(format!("recorded actor {actor:?} is not in its revision"));
    }
    if !input && let Some(shape) = ports.carried_shape(actor, port, false) {
        return shape.clone();
    }
    let resolved = ports
        .shape(actor, port, input)
        .map_err(|error| format!("recorded port resolution: {error:?}"))?;
    match resolved {
        Some(shape) => Ok(shape),
        None if input => Ok(GroundShape::try_new(Shape::Any).expect("Any ground")),
        None => Err(format!(
            "recorded output port {actor:?}/{port:?} is unresolved"
        )),
    }
}

struct StandingPipelineTree {
    role: ScopeRole,
    stood_actors: usize,
    children: BTreeMap<DeclaredScopeSeg, StandingPipelineTree>,
}

struct AuthoringCutHistory<'a> {
    store: &'a crate::daemon::authoring_store::AuthoringStore,
    pending: Option<&'a AuthoringSnapshot>,
}

#[cfg(test)]
pub(crate) fn committed_authoring_snapshot_for_test(
    store: &crate::daemon::authoring_store::AuthoringStore,
    cursor: u64,
) -> Result<AuthoringSnapshot, String> {
    AuthoringCutHistory::committed(store).snapshot(cursor)
}

impl<'a> AuthoringCutHistory<'a> {
    fn committed(store: &'a crate::daemon::authoring_store::AuthoringStore) -> Self {
        Self {
            store,
            pending: None,
        }
    }

    fn snapshot(&self, cursor: u64) -> Result<AuthoringSnapshot, String> {
        if let Some(pending) = self.pending.filter(|cut| cut.cursor == cursor) {
            return Ok(pending.clone());
        }
        self.store
            .load_at(cursor)?
            .snapshot(Vec::new())
            .map_err(|rejection| rejection.to_string())
    }

    fn verify_manifest(
        &self,
        manifest: &RunManifest<ProductStore>,
        project: [u8; 32],
    ) -> Result<(), String> {
        let cut = match manifest.groups().revision().start() {
            RevisionStart::Fresh(cut) => cut,
        };
        let expected =
            GraphRevisionCut::from_snapshot(&self.snapshot(cut.cursor)?)?.manifest_cut(project)?;
        if cut != &expected {
            return Err(
                "manifest authoring cut disagrees with its committed authoring history".to_owned(),
            );
        }
        Ok(())
    }
}

pub(crate) struct PendingServerRun {
    run: StreamId,
    first_revision: RevisionEpochId,
    execution: ProductExecutionProfile,
    state_directory: Option<std::path::PathBuf>,
    verify_history: bool,
    authoring_cut: GraphRevisionCut,
    plan: Arc<AuthoredProjection>,
    ports: Option<engine::ResolvedRevisionPorts>,
    clock: Arc<dyn TickSource>,
    clock_origin: (Tick, u64),
    restoration: Option<crate::kernel::system::Restoration>,
    restored_count: usize,
    last_recorded_instant_ms: u64,
}

impl PendingServerRun {
    /// Assembly reads immutable identity and prepares unstamped seed bodies.
    /// System starts before any user preparation and commits the entire seed.
    pub(crate) fn start_system(&mut self) -> Result<Arc<SystemRuntime>, String> {
        let directory = self
            .state_directory
            .as_deref()
            .ok_or("the actor kernel needs the state journal")?;
        let path = engine::state_journal::state_journal_path(directory);
        let project = crate::daemon::authoring_store::recorded_creation(directory)?.project;
        let existing = engine::state_manifest::read_state_manifest(&path)?;
        let manifest = existing.clone().unwrap_or(manifest_of(
            self.run,
            self.authoring_cut.manifest_cut(project)?,
        ));
        let RevisionStart::Fresh(cut) = manifest.groups().revision().start();
        if cut.project != project {
            return Err("runtime manifest project disagrees with project creation".into());
        }
        if self.verify_history {
            let store = crate::daemon::authoring_store::AuthoringStore::open(directory)?;
            AuthoringCutHistory::committed(&store).verify_manifest(&manifest, project)?;
        }
        let journal = engine::ProductDurableArrivalJournal::open_unseeded(&path, self.run)?;
        let system = SystemRuntime::start(
            self.run,
            journal,
            self.clock.clone(),
            &self.execution,
            directory,
        )?;
        if existing.is_none() {
            use crate::kernel::system::DaemonFact;
            system.pipeline.record_facts(
                self.first_revision,
                vec![
                    DaemonFact::Manifest(manifest),
                    DaemonFact::Revision(revision_body(&self.authoring_cut)?),
                    DaemonFact::StreamStart {
                        origin: self.clock_origin.0,
                        body: circular_store::ProductStreamStartBody {
                            boot_id: self.execution.boot_id(),
                            wall_millis: self.clock_origin.1,
                        },
                    },
                ],
            )?;
        }
        Ok(system)
    }

    pub(crate) fn activate(mut self) -> Result<ServerRun, String> {
        let system = self.start_system()?;
        self.activate_on(&system)
    }

    pub(crate) fn activate_on(mut self, system: &Arc<SystemRuntime>) -> Result<ServerRun, String> {
        let plan = self.plan.clone();
        let execution = self.execution.clone();
        let revision = self.first_revision;
        let (send, receive) = std::sync::mpsc::channel();
        let stood = system.pipeline.activate(revision, move || {
            let ports = engine::ResolvedRevisionPorts::resolve(&plan).map_err(|e| e.to_string())?;
            let declarations = engine::RevisionDeclarations::published(&plan)
                .map_err(|error| error.to_string())?;
            let grants = execution.capability_grants(declarations.parts().0);
            send.send(ports)
                .map_err(|_| "activation preparation receiver stopped")?;
            Ok(crate::kernel::system::Standing {
                declarations,
                revision,
                grants,
            })
        })?;
        self.ports = Some(receive.recv().map_err(|_| "user preparation stopped")?);
        self.finish(system, stood)
    }

    fn finish(
        self,
        system: &Arc<SystemRuntime>,
        stood: crate::kernel::system::Stood,
    ) -> Result<ServerRun, String> {
        let store = system.read_view();
        let mut server = ServerRun {
            run: self.run,
            pipeline: system.pipeline.clone(),
            stood_actors: stood.actors.iter().cloned().collect(),
            store,
            arrival_journal: Some(system.journal.clone()),
            journal_limits: None,
            recorded: self.restored_count,
            journal_ceiling_recorded: None,
            revisions: self.first_revision.get(),
            unboarded: stood
                .unboarded
                .iter()
                .map(|(actor, kind)| format!("{actor:?} ({kind:?})"))
                .collect(),
            templates: Vec::new(),
            plan: self.plan,
            ports: Arc::new(
                self.ports
                    .expect("successful System preparation resolved ports"),
            ),
            last_recorded_instant_ms: self.last_recorded_instant_ms,
            execution: self.execution,
            entrances: MountEntrances::default(),
            approvals: system.approvals.clone(),
        };
        server.activate_mount_entrances();
        Ok(server)
    }
}
impl ServerRun {
    pub(crate) fn open_authoring_cut(
        plan: AuthoredProjection,
        revision: RevisionEpochId,
        run: StreamId,
        execution: ProductExecutionProfile,
        authoring_cut: &AuthoringSnapshot,
    ) -> Result<Self, String> {
        Self::prepare_authoring_cut(plan, revision, run, execution, authoring_cut)?.activate()
    }

    pub(crate) fn prepare_authoring_cut(
        plan: AuthoredProjection,
        revision: RevisionEpochId,
        run: StreamId,
        execution: ProductExecutionProfile,
        authoring_cut: &AuthoringSnapshot,
    ) -> Result<PendingServerRun, String> {
        Self::prepare_with_mode(
            plan,
            revision,
            run,
            execution,
            None,
            GraphRevisionCut::from_snapshot(authoring_cut)?,
            None,
            None,
        )
    }

    pub(crate) fn open_at_authoring_cut(
        plan: AuthoredProjection,
        revision: RevisionEpochId,
        run: StreamId,
        execution: ProductExecutionProfile,
        state_directory: &Path,
        authoring_cut: &AuthoringSnapshot,
    ) -> Result<Self, String> {
        let history = crate::daemon::authoring_store::AuthoringStore::open(state_directory)
            .map_err(|e| e.to_string())?;
        Self::open_with_mode(
            plan,
            revision,
            run,
            execution,
            Some(state_directory),
            GraphRevisionCut::from_snapshot(authoring_cut)?,
            Some(AuthoringCutHistory::committed(&history)),
        )
    }

    /// Prepare the record basis for an authoring cut. Opening the journal and
    /// retaining System follow the authoring receipt; user assembly is a System
    /// request and its success or failure is recorded in System's own column.
    pub(crate) fn prepare_at_authoring_cut(
        plan: AuthoredProjection,
        revision: RevisionEpochId,
        run: StreamId,
        execution: ProductExecutionProfile,
        state_directory: &Path,
        authoring_cut: &AuthoringSnapshot,
    ) -> Result<PendingServerRun, String> {
        let history = crate::daemon::authoring_store::AuthoringStore::open(state_directory)
            .map_err(|e| e.to_string())?;
        Self::prepare_with_mode(
            plan,
            revision,
            run,
            execution,
            Some(state_directory),
            GraphRevisionCut::from_snapshot(authoring_cut)?,
            Some(AuthoringCutHistory {
                store: &history,
                pending: Some(authoring_cut),
            }),
            None,
        )
    }

    #[cfg(test)]
    pub(crate) fn open(
        plan: AuthoredProjection,
        revision: RevisionEpochId,
        run: StreamId,
        execution: ProductExecutionProfile,
    ) -> Result<Self, String> {
        static ORDINAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "circular-server-run-{}-{}",
            std::process::id(),
            ORDINAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        Self::open_at(plan, revision, run, execution, &directory)
    }

    #[cfg(test)]
    fn prepare_at(
        plan: AuthoredProjection,
        revision: RevisionEpochId,
        run: StreamId,
        execution: ProductExecutionProfile,
        state_directory: &Path,
    ) -> Result<PendingServerRun, String> {
        let cut = test_graph_revision_cut(revision.get());
        crate::daemon::authoring_store::AuthoringStore::open(state_directory)
            .map_err(|e| e.to_string())?;
        Self::prepare_with_mode(
            plan,
            revision,
            run,
            execution,
            Some(state_directory),
            cut,
            None,
            None,
        )
    }

    #[cfg(test)]
    fn with_test_journal_limits(mut run: Self) -> Self {
        run.set_journal_limits(
            crate::daemon::runtime_arrival_retention::RuntimeArrivalLimits::from_mib(
                256, 500_000, 2_048,
            )
            .expect("explicit test journal limits"),
        );
        run
    }

    #[cfg(test)]
    pub(crate) fn open_at(
        plan: AuthoredProjection,
        revision: RevisionEpochId,
        run: StreamId,
        execution: ProductExecutionProfile,
        state_directory: &Path,
    ) -> Result<Self, String> {
        Self::prepare_at(plan, revision, run, execution, state_directory)?
            .activate()
            .map(Self::with_test_journal_limits)
    }

    /// Open the runtime consumer restored from durable authoring state.
    ///
    /// This is a newly issued run, even when authoring was loaded after a restart.
    /// Its first GraphRevision is recorded below and owns its exact plan.
    pub(crate) fn open_restored_at_authoring_cut(
        plan: AuthoredProjection,
        revision: RevisionEpochId,
        run: StreamId,
        execution: ProductExecutionProfile,
        state_directory: &Path,
        authoring_cut: &AuthoringSnapshot,
    ) -> Result<Self, String> {
        let history = crate::daemon::authoring_store::AuthoringStore::open(state_directory)
            .map_err(|e| e.to_string())?;
        Self::open_with_mode(
            plan,
            revision,
            run,
            execution,
            Some(state_directory),
            GraphRevisionCut::from_snapshot(authoring_cut)?,
            Some(AuthoringCutHistory::committed(&history)),
        )
    }

    fn prepare_boundary_at_authoring_cut(
        plan: AuthoredProjection,
        revision: RevisionEpochId,
        run: StreamId,
        execution: ProductExecutionProfile,
        state_directory: &Path,
        authoring_cut: &AuthoringSnapshot,
        stood: StoodPrefix,
    ) -> Result<PendingServerRun, String> {
        let authoring_store = crate::daemon::authoring_store::AuthoringStore::open(state_directory)
            .map_err(|e| e.to_string())?;
        let StoodPrefix { mut prefix } = stood;
        let interrupt = prefix.interrupt;
        let standings = prefix
            .snapshots
            .iter()
            .map(|(recorded, plan)| {
                let declarations = engine::RevisionDeclarations::published(plan)
                    .map_err(|error| error.to_string())?;
                let grants = execution.capability_grants(declarations.parts().0);
                Ok(crate::kernel::system::Standing {
                    declarations,
                    revision: *recorded,
                    grants,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let admitted = std::mem::take(&mut prefix.admitted);
        let outcomes = std::mem::take(&mut prefix.outcomes);
        let refused = std::mem::take(&mut prefix.refused);
        for (actor, reason) in &refused {
            eprintln!(
                "circular-daemon: the recorded rows of {actor:?} cannot be restored: {reason}"
            );
        }
        let cell_facts = crate::kernel::system::cell_facts(&prefix.records, &prefix.lives);
        let emitted = prefix.lives.emitted(&prefix.records);
        let (recorded, records) = prefix.into_recorded();
        interrupt.observed()?;
        let restoration = crate::kernel::system::Restoration {
            columns: recorded,
            history: standings,
            admitted,
            outcomes,
            cell_facts,
            refused,
            emitted,
        };
        Self::prepare_with_mode(
            plan,
            revision,
            run,
            execution,
            Some(state_directory),
            GraphRevisionCut::from_snapshot(authoring_cut)?,
            Some(AuthoringCutHistory::committed(&authoring_store)),
            Some((restoration, records)),
        )
    }

    fn open_with_mode(
        plan: AuthoredProjection,
        first_revision: RevisionEpochId,
        run: StreamId,
        execution: ProductExecutionProfile,
        state_directory: Option<&Path>,
        authoring_cut: GraphRevisionCut,
        authoring_history: Option<AuthoringCutHistory<'_>>,
    ) -> Result<Self, String> {
        Self::prepare_with_mode(
            plan,
            first_revision,
            run,
            execution,
            state_directory,
            authoring_cut,
            authoring_history,
            None,
        )?
        .activate()
    }

    fn prepare_with_mode(
        plan: AuthoredProjection,
        first_revision: RevisionEpochId,
        run: StreamId,
        execution: ProductExecutionProfile,
        state_directory: Option<&Path>,
        authoring_cut: GraphRevisionCut,
        authoring_history: Option<AuthoringCutHistory<'_>>,
        restored: Option<RestoredPrefix>,
    ) -> Result<PendingServerRun, String> {
        if authoring_cut.cursor != first_revision.get() {
            return Err(format!(
                "GraphRevision cursor {} disagrees with runtime revision {}",
                authoring_cut.cursor,
                first_revision.get()
            ));
        }
        if let Some(history) = &authoring_history {
            authoring_cut.verify_snapshot(&history.snapshot(authoring_cut.cursor)?)?;
        }
        let verify_history = authoring_history.is_some();
        let (restoration, restored_records) = match restored {
            Some((restoration, records)) => (Some(restoration), Some(records)),
            None => (None, None),
        };
        let restored_columns = restoration.as_ref().map(|restoration| &restoration.columns);
        let plan = Arc::new(plan);
        let recorded_offset = restored_records
            .as_ref()
            .into_iter()
            .flatten()
            .filter_map(|record| {
                let Record::Boundary(boundary) = record else {
                    return None;
                };
                match boundary.fact() {
                    BoundaryFact::Arrival { observed_at, .. }
                    | BoundaryFact::Admission { observed_at, .. } => Some(observed_at.millis()),
                    BoundaryFact::ScheduleReservation { .. }
                    | BoundaryFact::EmissionBody { .. } => None,
                }
            })
            .chain(
                restored_records
                    .as_ref()
                    .into_iter()
                    .flatten()
                    .filter_map(|record| {
                        let Record::Observation(observation) = record else {
                            return None;
                        };
                        let ClassKey::Observation(ObservationKey::StreamItem(_, bucket, item)) =
                            observation.header().key()
                        else {
                            return None;
                        };
                        (item.kind() == &BuiltinObservationName::DiagnosticOccurrence)
                            .then(|| bucket.millis())
                    }),
            )
            .max()
            .unwrap_or(0);
        let restored_count =
            restored_columns.map_or(0, |columns| columns.values().map(Vec::len).sum::<usize>());
        let last_recorded_instant_ms = restored_columns
            .into_iter()
            .flat_map(|columns| columns.values())
            .flatten()
            .map(|arrival| arrival.observed_at().millis())
            .fold(recorded_offset, u64::max);
        let resolution = TicksPerSecond::new(1_000).expect("millisecond resolution");
        let clock: Arc<dyn TickSource> = Arc::new(
            WallAnchored::new(NonZeroTicks::new(1).expect("1 is nonzero"), resolution)
                .starting_at(Tick::new(recorded_offset)),
        );
        let clock_origin = (
            Tick::new(recorded_offset),
            u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|error| format!("run clock origin precedes Unix epoch: {error}"))?
                    .as_millis(),
            )
            .map_err(|_| "run clock origin wall overflow".to_owned())?,
        );
        Ok(PendingServerRun {
            run,
            first_revision,
            execution,
            state_directory: state_directory.map(Path::to_path_buf),
            verify_history,
            authoring_cut,
            plan,
            ports: None,
            clock,
            clock_origin,
            restoration,
            restored_count,
            last_recorded_instant_ms,
        })
    }

    pub(crate) fn is_paused(&self) -> bool {
        self.pipeline.paused()
    }

    /// System commits shutdown after members stop; that submission closes the writer.
    ///
    /// System stamps DaemonShutdown with the revision this run stands at
    /// (as Resume and Pause do), never with a newer
    /// accepted declaration it did not adopt.
    pub(crate) fn persist_shutdown(
        &mut self,
        body: circular_store::ProductShutdownBody,
    ) -> Result<(), String> {
        self.pipeline.shutdown(
            RevisionEpochId::new(self.revisions).ok_or("runtime revision is zero")?,
            body,
        )
    }

    pub(crate) fn resume(&self) -> crate::kernel::system::Acceptance {
        self.pipeline
            .resume(RevisionEpochId::new(self.revisions).expect("standing revision"))
    }

    pub(crate) fn approval_snapshot(&self) -> Result<RuntimeApprovalQueueSnapshot, String> {
        self.approvals.snapshot()
    }

    pub(crate) fn set_journal_limits(
        &mut self,
        limits: crate::daemon::runtime_arrival_retention::RuntimeArrivalLimits,
    ) {
        self.journal_limits = Some(limits);
    }

    fn required_journal_limits(
        &self,
    ) -> Result<crate::daemon::runtime_arrival_retention::RuntimeArrivalLimits, String> {
        self.journal_limits.ok_or_else(||
            "ConfigRejected: daemon runtime_arrivals.arrivals_max_mib, runtime_arrivals.arrivals_max_records, runtime_arrivals.total_max_mib are required before serving; values are absent".to_owned())
    }

    fn journal_measure(
        &self,
    ) -> Result<Option<crate::daemon::runtime_arrival_retention::JournalMeasure>, String> {
        let Some(journal) = &self.arrival_journal else {
            return Ok(None);
        };
        crate::daemon::runtime_arrival_retention::JournalMeasure::of(
            journal,
            self.required_journal_limits()?,
        )
        .map(Some)
    }

    pub(crate) fn record_journal_ceiling_health(&mut self) -> Result<(), String> {
        let Some(measure) = self.journal_measure()? else {
            return Ok(());
        };
        let exceeded = measure.exceeded();
        let recorded = match &self.journal_ceiling_recorded {
            Some(recorded) => recorded.clone(),
            None => crate::daemon::daemon_health::recorded_journal_ceiling(&self.store)?,
        };
        if recorded.as_deref() == exceeded.map(circular_actors::FailureDetail::code) {
            self.journal_ceiling_recorded = Some(recorded);
            return Ok(());
        }
        let (state, reason) = match exceeded {
            Some(detail) => (
                ActorHealthState::Backpressure,
                Some(ActorHealthReason {
                    code: ActorHealthReasonCode::Capacity,
                    detail: detail.to_value(),
                }),
            ),
            None => (ActorHealthState::Running, None),
        };
        self.pipeline.record_facts(
            RevisionEpochId::new(self.revisions).ok_or("runtime revision is zero")?,
            vec![crate::kernel::system::DaemonFact::Health {
                actor: actor_health_producer(),
                state,
                reason,
                since: RecordedInstant::from_millis(self.last_recorded_instant_ms),
            }],
        )?;
        self.refresh_journal_prefix();
        self.journal_ceiling_recorded = Some(exceeded.map(|detail| detail.code().to_owned()));
        match exceeded {
            Some(detail) => eprintln!(
                "circular-daemon: journal ceiling exceeded ({}): {}",
                detail.code(),
                measure.message()
            ),
            None => eprintln!(
                "circular-daemon: journal is back under its ceilings: {}",
                measure.message()
            ),
        }
        Ok(())
    }

    pub(crate) fn arrival_journal_usage(&self) -> Option<engine::ProductArrivalJournalUsage> {
        self.arrival_journal
            .as_ref()
            .map(engine::ProductDurableArrivalJournal::usage)
    }

    /// Fixture injection of already stamped records through the same writer.
    #[cfg(test)]
    fn append_records(
        &mut self,
        batch: AppendBatch<ProductStore>,
        label: &str,
    ) -> Result<(), String> {
        if let Some(journal) = &self.arrival_journal {
            journal
                .append_records(batch)
                .map_err(|error| format!("{label}: {error}"))?;
            self.store = journal.read_view();
            Ok(())
        } else {
            let store = self
                .store
                .memory_mut()
                .ok_or("a run without a journal holds no prefix to append beside")?;
            match store.append(batch) {
                AppendResult::Committed(_) => {
                    store.seal_all();
                    Ok(())
                }
                other => Err(format!("{label} was not appended: {other:?}")),
            }
        }
    }

    pub(crate) fn refresh_journal_prefix(&mut self) {
        if let Some(journal) = &self.arrival_journal {
            self.store = journal.read_view();
        }
    }

    fn append_adopted_revision(&mut self, cut: &GraphRevisionCut) -> Result<(), String> {
        self.pipeline.record_facts(
            RevisionEpochId::new(cut.cursor).ok_or("GraphRevision cursor is zero")?,
            vec![crate::kernel::system::DaemonFact::Revision(revision_body(
                cut,
            )?)],
        )?;
        self.refresh_journal_prefix();
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn execution(&self) -> &ProductExecutionProfile {
        &self.execution
    }

    #[cfg(test)]
    pub(crate) fn adopt(
        &mut self,
        target: RevisionEpochId,
        candidate: AuthoredProjection,
    ) -> PlanAdoptionOutcome {
        self.adopt_with_cut(
            target,
            Arc::new(candidate),
            test_graph_revision_cut(target.get()),
        )
    }

    pub(crate) fn prepare_authoring_cut_adoption(
        &mut self,
        target: RevisionEpochId,
        candidate: AuthoredProjection,
        snapshot: &AuthoringSnapshot,
    ) -> PreparedAdoption {
        let cut = match GraphRevisionCut::from_snapshot(snapshot) {
            Ok(cut) => cut,
            Err(reason) => {
                return PreparedAdoption::Settled {
                    target,
                    outcome: PlanAdoptionOutcome::Behind { reason },
                };
            }
        };
        self.prepare_with_cut(target, Arc::new(candidate), cut)
    }

    pub(crate) fn commit_prepared_adoption(
        &mut self,
        prepared: PreparedAdoption,
    ) -> PlanAdoptionOutcome {
        match prepared {
            PreparedAdoption::Settled { target, outcome } => {
                if let PlanAdoptionOutcome::Behind { reason } = &outcome {
                    let reason = reason.clone();
                    if let Err(reason) = self.pipeline.adopt(target, move || Err(reason)) {
                        return PlanAdoptionOutcome::Behind { reason };
                    }
                }
                outcome
            }
            PreparedAdoption::Ready(ready) => match self.commit_ready_adoption(*ready) {
                Ok(()) => PlanAdoptionOutcome::Adopted,
                Err(reason) => PlanAdoptionOutcome::Behind { reason },
            },
        }
    }

    fn adopt_with_cut(
        &mut self,
        target: RevisionEpochId,
        candidate: Arc<AuthoredProjection>,
        authoring_cut: GraphRevisionCut,
    ) -> PlanAdoptionOutcome {
        let prepared = self.prepare_with_cut(target, candidate, authoring_cut);
        self.commit_prepared_adoption(prepared)
    }

    fn prepare_with_cut(
        &mut self,
        target: RevisionEpochId,
        candidate: Arc<AuthoredProjection>,
        authoring_cut: GraphRevisionCut,
    ) -> PreparedAdoption {
        match self.prepare_reconcile_plan(candidate, target, authoring_cut) {
            Ok(ready) => PreparedAdoption::Ready(Box::new(ready)),
            Err(reason) => PreparedAdoption::Settled {
                target,
                outcome: PlanAdoptionOutcome::Behind { reason },
            },
        }
    }

    fn revision_reconciliation(&self, plan: &AuthoredProjection) -> engine::RevisionReconciliation {
        engine::RevisionReconciliation::between(&self.plan, plan)
    }

    fn prepare_reconcile_plan(
        &mut self,
        plan: Arc<AuthoredProjection>,
        revision: RevisionEpochId,
        authoring_cut: GraphRevisionCut,
    ) -> Result<ReadyAdoption, String> {
        let next_revision = revision.get();
        if authoring_cut.cursor != next_revision {
            return Err(format!(
                "GraphRevision cursor {} disagrees with runtime revision {next_revision}",
                authoring_cut.cursor
            ));
        }
        if next_revision <= self.revisions {
            return Err(format!(
                "target revision {next_revision} does not advance applied revision {}",
                self.revisions
            ));
        }

        let classification = self.revision_reconciliation(&plan);
        if !classification.is_metadata_only() && classification.is_preprocess_only() {
            engine::validate_published_plan(&plan).map_err(|error| error.to_string())?;
        }

        let ports = Arc::new(
            engine::ResolvedRevisionPorts::resolve(&plan).map_err(|error| error.to_string())?,
        );
        Ok(ReadyAdoption {
            target: revision,
            authoring_cut,
            plan,
            ports,
        })
    }

    fn commit_ready_adoption(&mut self, ready: ReadyAdoption) -> Result<(), String> {
        let ReadyAdoption {
            target: revision,
            authoring_cut,
            plan,
            ports,
        } = ready;
        let execution = self.execution.clone();
        let published = self.append_adopted_revision(&authoring_cut);
        if published.is_ok() {
            self.revisions = revision.get();
        }
        let candidate = plan.clone();
        let stood = self.pipeline.adopt(revision, move || {
            published?;
            let declarations = engine::RevisionDeclarations::published(&candidate)
                .map_err(|error| error.to_string())?;
            let grants = execution.capability_grants(declarations.parts().0);
            Ok(crate::kernel::system::Reconcile {
                standing: crate::kernel::system::Standing {
                    declarations,
                    revision,
                    grants,
                },
            })
        })?;
        self.stood_actors.extend(stood.actors.iter().cloned());
        for actor in stood
            .failures
            .iter()
            .map(|failure| &failure.actor)
            .chain(&stood.retired)
        {
            self.stood_actors.remove(actor);
        }
        self.unboarded = stood
            .unboarded
            .iter()
            .map(|(actor, kind)| format!("{actor:?} ({kind:?})"))
            .collect();
        self.templates.clear();
        self.plan = plan;
        self.ports = ports;
        self.activate_mount_entrances();
        Ok(())
    }

    pub(crate) fn inject(
        &mut self,
        mount: &PlanExportKey,
        payload: ProductPayload,
        origin: circular_runtime::ExternalOrigin,
    ) -> Result<(), String> {
        self.ingress(self.required_journal_limits()?)
            .inject(mount, payload, origin)
            .map_err(|refusal| refusal.to_string())?;
        self.refresh_journal_prefix();
        Ok(())
    }
}

fn standing_pipeline_projection(
    plan: &AuthoredProjection,
    stood: &BTreeSet<NamedActorId>,
    lifecycle: u64,
    plan_revision: u64,
) -> Result<(Value, Vec<Value>), String> {
    let tree = fold_projection(plan, |layer| StandingPipelineTree {
        role: layer.graph().declaration().role(),
        stood_actors: layer
            .actors()
            .keys()
            .filter(|actor| stood.contains(*actor))
            .count(),
        children: layer.into_scopes(),
    });
    let anchor = Value::object([
        ("plan_revision", Value::UInt(plan_revision)),
        (
            "lifecycle",
            Value::Int(
                i64::try_from(lifecycle)
                    .map_err(|_| "lifecycle identity exceeds Int".to_owned())?,
            ),
        ),
    ])
    .map_err(|error| format!("pipelines anchor: {error:?}"))?;
    let mut items = Vec::new();
    append_concrete_pipelines(&ScopeId::root(), tree.children, &mut items)?;
    Ok((anchor, items))
}

fn append_concrete_pipelines(
    parent: &ScopeId,
    children: BTreeMap<DeclaredScopeSeg, StandingPipelineTree>,
    output: &mut Vec<Value>,
) -> Result<(), String> {
    for (segment, child) in children {
        if child.role.is_template() {
            continue;
        }
        let scope = parent
            .append_segment(segment.as_scope_seg())
            .map_err(|error| format!("pipelines scope exceeds plan bounds: {error}"))?;
        let scope_value = circular_store::scope_value(&scope)
            .map_err(|error| format!("pipelines scope identity: {error}"))?;
        output.push(
            Value::object([
                ("scope", scope_value),
                (
                    "stood_actors",
                    Value::UInt(
                        u64::try_from(child.stood_actors)
                            .map_err(|_| "pipelines stood-actor count exceeds UInt".to_owned())?,
                    ),
                ),
            ])
            .map_err(|error| format!("pipelines item: {error:?}"))?,
        );
        append_concrete_pipelines(&scope, child.children, output)?;
    }
    Ok(())
}

/// Canonical live display carrier. The axis comes from `DisplayKey`; the window comes from the
/// record's outer `at` coordinate.
fn display_frame_value(
    key: &circular_store::DisplayKey,
    at: &Stamp<ActorId>,
    actor: Value,
    body: Value,
) -> Option<Value> {
    let bucket = key.bucket()?;
    let bucket_millis = bucket.get().get();
    let bucket_ms = i64::try_from(bucket_millis).ok()?;
    let window = i64::try_from(at.physical_time().get() / bucket_millis).ok()?;
    let fields = vec![
        ("body", body),
        ("bucket_ms", Value::int(bucket_ms)),
        ("name", Value::string(key.name().as_str().to_owned())),
        ("actor", actor),
        ("window", Value::int(window)),
    ];
    Value::object(fields).ok()
}

/// Authoring supplies the existing cut body; System owns its record stamp.
fn revision_body(authoring_cut: &GraphRevisionCut) -> Result<EncodedPayload, String> {
    Ok(EncodedPayload::new(
        PayloadVersionTag::FIRST,
        &circular_core::encode(
            &authoring_cut.to_value()?,
            circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
        )
        .map_err(|error| format!("GraphRevision cut does not encode: {error:?}"))?,
    ))
}

fn presentation_value(plan: &AuthoredProjection) -> Value {
    use circular_protocol::declaration_payload::{
        PlanAnnotationKey, PresentationOwner, presentation_owner_value,
    };
    use circular_protocol::scope_identity::{
        AddressRef, plan_actor_key_value, scope_identity_value,
    };
    use circular_runtime::product_identity::{wire_named_actor, wire_scope};

    let coord = |value: circular_plan::LayoutCoord| Value::int(i64::from(value.get()));
    let actor = |id: &circular_plan::NamedActorId| Value::string(format!("{id:?}"));
    let anchor = |anchor: &circular_plan::Anchor| match anchor {
        circular_plan::Anchor::Flow => Value::array(vec![Value::int(1)]),
        circular_plan::Anchor::Relative { target, relation } => Value::array(vec![
            Value::int(2),
            actor(target),
            Value::int(relation.tag()),
        ]),
        circular_plan::Anchor::Align { target, axis } => {
            Value::array(vec![Value::int(3), actor(target), Value::int(axis.tag())])
        }
    };

    let mut rows: Vec<(PresentationOwner, circular_plan::Presentation)> =
        crate::authoring_assembly::projection::fold_projection(plan, |layer| {
            let mut rows: Vec<_> = layer
                .presentation()
                .iter()
                .map(|(id, presentation)| {
                    let owner = id.clone().map(
                        |id| wire_named_actor(&id),
                        |id| PlanAnnotationKey {
                            scope: wire_scope(layer.scope()),
                            local: id.name().as_str().to_owned(),
                        },
                    );
                    (owner, presentation.clone())
                })
                .collect();
            for mut child in layer.into_scopes().into_values() {
                rows.append(&mut child);
            }
            rows
        });
    rows.sort_by(|(left, _), (right, _)| left.cmp(right));

    let mut entries = Vec::new();
    for (owner, presentation) in &rows {
        entries.push(Value::array(vec![
            presentation_owner_value(
                owner,
                |key| {
                    Ok::<_, std::convert::Infallible>(
                        AddressRef::Absolute(key.clone()).to_value_with(plan_actor_key_value),
                    )
                },
                |key| {
                    Ok(AddressRef::Absolute(key).to_value_with(|key| {
                        Value::object([
                            ("scope", scope_identity_value(&key.scope)),
                            ("local", Value::string(key.local.clone())),
                        ])
                        .expect("annotation identity fields are unique")
                    }))
                },
            )
            .expect("presentation owner encoding is infallible"),
            presentation
                .label()
                .map_or(Value::Null, |label| Value::string(label.to_owned())),
            presentation.anchor().map_or(Value::Null, anchor),
            presentation.fixed().map_or(Value::Null, |point| {
                Value::array(vec![coord(point.x()), coord(point.y())])
            }),
            presentation.size().map_or(Value::Null, |size| {
                Value::array(vec![Value::UInt(size.width()), Value::UInt(size.height())])
            }),
            presentation
                .view()
                .map_or(Value::Null, |view| Value::string(view.kind.clone())),
            Value::bool(presentation.collapsed()),
            presentation.board().map_or(Value::Null, |board| {
                Value::array([
                    Value::UInt(u64::from(board.col())),
                    Value::UInt(u64::from(board.row())),
                    Value::UInt(u64::from(board.w())),
                    Value::UInt(u64::from(board.h())),
                ])
            }),
            presentation
                .group()
                .map_or(Value::Null, |name| Value::string(name.as_str().to_owned())),
        ]));
    }
    Value::array(entries)
}

#[cfg(test)]
pub(crate) fn verify_manifest_history(
    manifest: &RunManifest<ProductStore>,
    project: [u8; 32],
    history: &crate::daemon::authoring_store::AuthoringStore,
) -> Result<(), String> {
    AuthoringCutHistory::committed(history).verify_manifest(manifest, project)
}

pub(crate) fn manifest_of(
    run: StreamId,
    cut: circular_store::ProductAuthoringCut,
) -> RunManifest<ProductStore> {
    let payload = || EncodedPayload::new(PayloadVersionTag::FIRST, b"");
    RunManifest::new(
        run,
        ManifestGroups::new(
            TimeParams::new(
                TicksPerSecond::new(1_000).expect("resolution"),
                NonZeroTicks::new(1).expect("cadence"),
                payload(),
                TimeSourcePlan::Single(TimeSourceKind::Manual),
                OpaqueId::new(0),
            ),
            PlacementParams::from_validated(payload()),
            FailureParams::from_validated(payload()),
            EncodedPayload::new(
                PayloadVersionTag::FIRST,
                &circular_core::encode(
                    &circular_core::compatibility::current().to_value(),
                    circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
                )
                .expect("the fixed compatibility axes fit the value codec ceilings"),
            ),
            RevisionContext::try_new(RevisionStart::Fresh(cut), Vec::new()).expect("empty grant"),
            RunInputs::from_primary_data(payload()),
        ),
    )
}

struct ObservedPrefix {
    next: u64,
    early: BTreeSet<u64>,
}

impl ObservedPrefix {
    fn from(next: u64) -> Self {
        Self {
            next,
            early: BTreeSet::new(),
        }
    }

    fn observe(&mut self, ordinal: u64) -> bool {
        if ordinal < self.next {
            return false;
        }
        self.early.insert(ordinal);
        let before = self.next;
        while self.early.remove(&self.next) {
            self.next += 1;
        }
        self.next > before
    }

    const fn len(&self) -> u64 {
        self.next
    }
}

include!("boundary_ingress.rs");

