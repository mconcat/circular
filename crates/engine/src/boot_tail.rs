use crate::restart_custody_codec::{ProductRestoreNestedCodec, decode_checkpoint_body};
use circular_plan::{ActorId, LocalKey, NamedActorId};
use circular_store::{
    ArrivalProjection, BoundaryFact, BoundaryKey, ClassKey, JournalProjection, Record,
    SqliteJournal, StoreTransactionOp,
};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::ControlFlow;
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootTail {
    pub horizon: u64,
    pub scanned: u64,
    pub reason: BootTailReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BootTailReason {
    Tail,
    Unadopted(circular_core::RevisionEpochId),
    Cells,
    Front(Option<NamedActorId>),
    Empty,
}

impl std::fmt::Display for BootTailReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tail => f.write_str("checkpoint tail"),
            Self::Unadopted(revision) => write!(
                f,
                "revision {} has no adoption record in this stream",
                revision.get()
            ),
            Self::Cells => f.write_str("plan declares cell templates"),
            Self::Front(Some(actor)) => write!(f, "reached the front for {actor:?}"),
            Self::Front(None) => f.write_str("reached the front for open custody"),
            Self::Empty => f.write_str("empty journal"),
        }
    }
}

#[derive(Clone, Copy)]
enum Latest {
    Resume { first: u64, before_row: bool },
    Whole,
}

#[derive(Default)]
struct Column {
    latest: Option<Latest>,
    revision: Option<circular_core::RevisionEpochId>,
    lowest: Option<u64>,
}

impl Column {
    fn satisfied(&self) -> bool {
        match (self.latest, self.lowest) {
            (_, Some(0)) => true,
            (
                Some(Latest::Resume {
                    before_row: false, ..
                }),
                _,
            ) => true,
            (
                Some(Latest::Resume {
                    first,
                    before_row: true,
                }),
                Some(lowest),
            ) => lowest <= first,
            _ => false,
        }
    }
}

pub fn find(
    path: &Path,
    plan: &crate::authoring_assembly::projection::AuthoredProjection,
    authoring_cursor: u64,
) -> Result<BootTail, String> {
    let front = |reason| {
        Ok(BootTail {
            horizon: 0,
            scanned: 0,
            reason,
        })
    };
    let journal = SqliteJournal::open_read_only_namespace(
        path,
        crate::state_journal::ARRIVAL_JOURNAL_NAMESPACE,
    )
    .map_err(|error| format!("boot tail open: {error}"))?;
    let Some(_) = journal
        .namespace_first_commit()
        .map_err(|error| format!("boot tail read: {error}"))?
    else {
        return front(BootTailReason::Empty);
    };
    let graph = crate::run_graph::fold_revision(plan).map_err(|error| error.to_string())?;
    if !graph.templates().is_empty() {
        return front(BootTailReason::Cells);
    }
    let mut columns: BTreeMap<NamedActorId, Column> = graph
        .actors()
        .keys()
        .map(|actor| (actor.clone(), Column::default()))
        .collect();
    let mut outboxes = BTreeSet::new();
    let mut approvals = BTreeSet::new();
    let mut names = circular_core::RevisionEpochId::new(authoring_cursor)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut adopted = BTreeSet::new();
    let mut scanned = 0_u64;
    let mut stopped = false;
    let mut decoder = circular_store::ProductJournalReader::default();
    let visited = journal
        .visit_namespace_back(|sequence, bytes| -> Result<ControlFlow<()>, String> {
            scanned += 1;
            let transaction = decoder
                .read_at(&journal, sequence.get(), bytes)
                .map_err(|error| format!("boot tail commit {}: {error:?}", sequence.get()))?;
            for operation in transaction.operations() {
                match operation {
                    StoreTransactionOp::ReplaceCheckpoint(row) => {
                        let Some(actor) = named(row.actor()) else {
                            continue;
                        };
                        let column = columns.entry(actor).or_default();
                        if column.latest.is_none() {
                            let body =
                                decode_checkpoint_body(&ProductRestoreNestedCodec, row.state())
                                    .map_err(|error| {
                                        format!(
                                            "boot tail checkpoint at commit {}: {error}",
                                            sequence.get()
                                        )
                                    })?;
                            column.revision = Some(body.continuation.revision);
                            names.insert(body.continuation.revision);
                            column.latest =
                                Some(match (&body.covered_arrival, &body.continuation.issuance) {
                                    (covered, Some(issuance)) => {
                                        let first = body.continuation.held.first().map_or(
                                            covered
                                                .as_ref()
                                                .map_or(0, |(covered, _)| covered.get() + 1),
                                            |held| held.get(),
                                        );
                                        Latest::Resume {
                                            first,
                                            before_row: first < issuance.horizon.get(),
                                        }
                                    }
                                    _ => Latest::Whole,
                                });
                        }
                    }
                    StoreTransactionOp::OpenOutbox { effect, .. } => {
                        outboxes.remove(effect);
                    }
                    StoreTransactionOp::SubmitOutbox { effect }
                    | StoreTransactionOp::AcquireOutboxDispatch { effect }
                    | StoreTransactionOp::SettleOutbox { effect, .. }
                    | StoreTransactionOp::CancelCommittedOutbox { effect, .. } => {
                        outboxes.insert(effect.clone());
                    }
                    StoreTransactionOp::OpenApproval { key, .. } => {
                        approvals.remove(key);
                    }
                    StoreTransactionOp::ApproveApproval { key, .. }
                    | StoreTransactionOp::SettleApproval { key, .. } => {
                        approvals.insert(key.clone());
                    }
                    _ => {}
                }
            }
            for record in ArrivalProjection::new()
                .project(&transaction)
                .map_err(|rejection| {
                    format!(
                        "boot tail commit {} operation {} does not project",
                        sequence.get(),
                        rejection.operation
                    )
                })?
            {
                let boundary = match &record {
                    Record::Boundary(boundary) => boundary,
                    Record::Structure(structure)
                        if matches!(
                            structure.fact(),
                            circular_store::StructureFact::GraphRevision(_)
                        ) =>
                    {
                        adopted.insert(structure.header().at().revision());
                        continue;
                    }
                    _ => continue,
                };
                names.insert(boundary.header().at().revision());
                let (
                    ClassKey::Boundary(BoundaryKey::Arrival { actor, .. }),
                    BoundaryFact::Arrival { arrival_index, .. },
                ) = (boundary.header().key(), boundary.fact())
                else {
                    continue;
                };
                let Some(actor) = named(actor) else {
                    continue;
                };
                let column = columns.entry(actor).or_default();
                column.lowest = Some(column.lowest.map_or(arrival_index.get(), |lowest| {
                    lowest.min(arrival_index.get())
                }));
            }
            if outboxes.is_empty()
                && approvals.is_empty()
                && columns.values().all(Column::satisfied)
                && unadopted(&columns, &names, &adopted).is_none()
            {
                stopped = true;
                return Ok(ControlFlow::Break(()));
            }
            Ok(ControlFlow::Continue(()))
        })
        .map_err(|error| format!("boot tail read: {error}"))??;
    let Some(last) = visited else {
        return Ok(BootTail {
            horizon: 0,
            scanned,
            reason: BootTailReason::Empty,
        });
    };
    if !stopped {
        let unsatisfied = columns
            .iter()
            .find(|(_, column)| !column.satisfied())
            .map(|(actor, _)| actor.clone());
        let reason = match unadopted(&columns, &names, &adopted) {
            Some(revision)
                if unsatisfied.is_none() && outboxes.is_empty() && approvals.is_empty() =>
            {
                BootTailReason::Unadopted(revision)
            }
            _ => BootTailReason::Front(unsatisfied),
        };
        return Ok(BootTail {
            horizon: 0,
            scanned,
            reason,
        });
    }
    Ok(BootTail {
        horizon: last - 1,
        scanned,
        reason: BootTailReason::Tail,
    })
}

fn unadopted(
    columns: &BTreeMap<NamedActorId, Column>,
    named: &BTreeSet<circular_core::RevisionEpochId>,
    adopted: &BTreeSet<circular_core::RevisionEpochId>,
) -> Option<circular_core::RevisionEpochId> {
    let floor = columns.values().filter_map(|column| column.revision).min();
    named
        .iter()
        .filter(|revision| floor.is_none_or(|floor| **revision > floor))
        .find(|revision| !adopted.contains(revision))
        .copied()
}

pub fn named_revisions(
    records: &[Record<circular_store::ProductStore>],
    rows: &[circular_store::TransactionCheckpoint<circular_store::ProductTransaction>],
) -> Result<BTreeSet<circular_core::RevisionEpochId>, String> {
    let mut revisions = BTreeSet::new();
    for record in records {
        match record {
            Record::Boundary(boundary) => {
                revisions.insert(boundary.header().at().revision());
            }
            Record::Structure(structure)
                if matches!(
                    structure.fact(),
                    circular_store::StructureFact::GraphRevision(_)
                ) =>
            {
                revisions.insert(structure.header().at().revision());
            }
            _ => {}
        }
    }
    for row in rows {
        let body = decode_checkpoint_body(&ProductRestoreNestedCodec, row.state())?;
        revisions.insert(body.continuation.revision);
        if let Some(born) = body
            .generations
            .last()
            .and_then(|own| circular_core::RevisionEpochId::new(own.get()))
        {
            revisions.insert(born);
        }
    }
    Ok(revisions)
}

fn named(actor: &ActorId) -> Option<NamedActorId> {
    match actor {
        ActorId::Scoped {
            scope,
            local: LocalKey::Named(name),
        } => Some(NamedActorId::new(scope.clone(), name.clone())),
        _ => None,
    }
}
