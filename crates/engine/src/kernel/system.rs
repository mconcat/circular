
use super::actor::{Column, Door, Incarnation, Injection, Refusal, Spawn, is_lifecycle};
use super::assemble::{Assembly, Unassembled};
use super::record::Issuer;
use super::share::{Credit, InletWire, OutWire, Share, ShareShape, capacity_of, holds_back};
use super::{Address, Control, Message};
use crate::actor_time::TimeActor;
use crate::declarations::RevisionDeclarations;
use crate::tap_pilot::ProductGrantCatalog;
use circular_actors::ProductPayload;
use circular_core::{ActorType, RevisionEpochId, Stamp};
use circular_plan::{ActorId, EdgeId, NamedActorId, PortId};
use circular_protocol::rejection_code::RejectionReason;
use circular_runtime::ExternalOrigin;
use circular_store::StreamId;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tokio::sync::{Semaphore, mpsc, oneshot};

/// The System column's vocabulary. Member consumption boundaries never occur here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OutcomeKind {
    Activation,
    RevisionAdoption,
    Recovery,
}

impl OutcomeKind {
    const ALL: [Self; 3] = [Self::Activation, Self::RevisionAdoption, Self::Recovery];

    fn name(self) -> circular_core::BuiltinObservationName {
        use circular_core::BuiltinObservationName as N;
        match self {
            Self::Activation => N::SystemActivationOutcome,
            Self::RevisionAdoption => N::SystemRevisionAdoptionOutcome,
            Self::Recovery => N::SystemRecoveryOutcome,
        }
    }

    fn failure(self) -> RejectionReason {
        match self {
            Self::Activation => RejectionReason::ActivationFailed,
            Self::RevisionAdoption => RejectionReason::RevisionAdoptionFailed,
            Self::Recovery => RejectionReason::RecoveryFailed,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SystemBody {
    Outcome {
        kind: OutcomeKind,
        revision: RevisionEpochId,
        result: Result<(), RejectionReason>,
    },
    PauseAccepted {
        force: bool,
    },
    ResumeAccepted,
}

impl SystemBody {
    /// The catalog names this column's bodies carry — the same names the encoder writes.
    fn kinds() -> [circular_core::BuiltinObservationName; 5] {
        use circular_core::BuiltinObservationName as N;
        let [activation, adoption, recovery] = OutcomeKind::ALL.map(OutcomeKind::name);
        [
            activation,
            adoption,
            recovery,
            N::SystemPauseAccepted,
            N::SystemResumeAccepted,
        ]
    }

    fn name(&self) -> circular_core::BuiltinObservationName {
        use circular_core::BuiltinObservationName as N;
        match self {
            Self::Outcome { kind, .. } => kind.name(),
            Self::PauseAccepted { .. } => N::SystemPauseAccepted,
            Self::ResumeAccepted => N::SystemResumeAccepted,
        }
    }

    pub(crate) fn to_value(&self) -> circular_core::Value {
        use circular_core::Value as V;
        let kind = V::UInt(u64::from(self.name().tag()));
        V::array(match self {
            Self::Outcome {
                revision, result, ..
            } => vec![
                kind,
                V::UInt(revision.get()),
                result.as_ref().err().map_or(V::Null, |reason| {
                    V::UInt(u64::from(
                        reason
                            .code_in(circular_protocol::Partition::Query)
                            .expect("System outcomes use outside-lifecycle rejection codes"),
                    ))
                }),
                V::Null,
            ],
            Self::PauseAccepted { force } => vec![kind, V::Bool(*force)],
            Self::ResumeAccepted => vec![kind],
        })
    }

    pub(crate) fn from_value(value: &circular_core::Value) -> Result<Self, String> {
        use circular_core::{BuiltinObservationName as N, Value as V};
        let V::Array(fields) = value else {
            return Err("System body is not an array".into());
        };
        let [V::UInt(tag), rest @ ..] = fields.as_slice() else {
            return Err("System body has an unknown shape".into());
        };
        let name = u8::try_from(*tag)
            .ok()
            .and_then(N::from_tag)
            .ok_or("System body has an unknown kind")?;
        match (name, rest) {
            (N::SystemPauseAccepted, [V::Bool(force)]) => Ok(Self::PauseAccepted { force: *force }),
            (N::SystemResumeAccepted, []) => Ok(Self::ResumeAccepted),
            (name, [V::UInt(revision), reason, V::Null]) => {
                let kind = OutcomeKind::ALL
                    .into_iter()
                    .find(|kind| kind.name() == name)
                    .ok_or("System body has an unknown shape")?;
                let result = match reason {
                    V::Null => Ok(()),
                    V::UInt(code)
                        if Some(*code)
                            == kind
                                .failure()
                                .code_in(circular_protocol::Partition::Query)
                                .map(u64::from) =>
                    {
                        Err(kind.failure())
                    }
                    _ => return Err("System outcome has an invalid rejection code".into()),
                };
                Ok(Self::Outcome {
                    kind,
                    revision: RevisionEpochId::new(*revision)
                        .ok_or("System revision is reserved")?,
                    result,
                })
            }
            _ => Err("System body has an unknown shape".into()),
        }
    }

    /// Catalog kind selects the body codec. System also produces other facts.
    pub(crate) fn read(
        record: &circular_store::Record<circular_store::ProductStore>,
    ) -> Result<Option<Self>, String> {
        use circular_store::{ClassKey, ObservationFact, ObservationKey, Record};
        if record.header().at().producer() != &ActorId::System(circular_plan::SystemActor::Pipeline)
        {
            return Ok(None);
        }
        let Record::Observation(observation) = record else {
            return Ok(None);
        };
        let ClassKey::Observation(ObservationKey::StreamItem(_, _, item)) = record.header().key()
        else {
            return Ok(None);
        };
        if !Self::kinds().contains(item.kind()) {
            return Ok(None);
        }
        let (payload, diagnostic) = match observation.fact() {
            ObservationFact::Diagnostic(payload) => (payload, true),
            ObservationFact::Lifecycle(payload) => (payload, false),
            _ => return Err("System observation has an invalid envelope".into()),
        };
        if payload.version_tag() != circular_core::PayloadVersionTag::FIRST {
            return Err("System observation has an unknown payload version".into());
        }
        let body = Self::from_value(
            &circular_core::decode(
                payload.body(),
                circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
            )
            .map_err(|e| format!("System body: {e:?}"))?,
        )?;
        if item.kind() != &body.name() || diagnostic != matches!(body, Self::Outcome { .. }) {
            return Err("System observation envelope and body disagree".into());
        }
        Ok(Some(body))
    }
}

pub(crate) type OutcomeReceipt = (Stamp<ActorId>, Result<(), RejectionReason>);
pub(crate) type Acceptance = Result<Stamp<ActorId>, RejectionReason>;

/// A pure projection of one System column, also used by daemon queries.
#[derive(Clone, Debug, Default)]
pub(crate) struct SystemFold {
    pub(crate) pause: Option<bool>,
    pub(crate) last_activation: Option<OutcomeReceipt>,
    pub(crate) last_revision_adoption: Option<OutcomeReceipt>,
    pub(crate) last_recovery: Option<OutcomeReceipt>,
}

impl SystemFold {
    pub(crate) fn apply(&mut self, at: &Stamp<ActorId>, body: &SystemBody) {
        match body {
            SystemBody::Outcome { kind, result, .. } => {
                let slot = match kind {
                    OutcomeKind::Activation => &mut self.last_activation,
                    OutcomeKind::RevisionAdoption => &mut self.last_revision_adoption,
                    OutcomeKind::Recovery => &mut self.last_recovery,
                };
                *slot = Some((at.clone(), *result));
            }
            SystemBody::PauseAccepted { force } => self.pause = Some(*force),
            SystemBody::ResumeAccepted => self.pause = None,
        }
    }

    pub(crate) fn read<'a>(
        records: impl IntoIterator<Item = &'a circular_store::Record<circular_store::ProductStore>>,
    ) -> Result<Self, String> {
        let mut state = Self::default();
        for record in records {
            if let Some(body) = SystemBody::read(record)? {
                state.apply(record.header().at(), &body);
            }
        }
        Ok(state)
    }
}

/// Assembly's address. Blocking material preparation runs outside the System
/// judgment task and returns a value, including its failure, through a mailbox.
#[derive(Clone)]
struct Preparation(mpsc::UnboundedSender<Box<dyn FnOnce() + Send>>);

impl Preparation {
    fn spawn(runtime: &tokio::runtime::Handle) -> Self {
        let (send, mut receive) = mpsc::unbounded_channel::<Box<dyn FnOnce() + Send>>();
        runtime.spawn(async move {
            while let Some(job) = receive.recv().await {
                let _ = tokio::task::spawn_blocking(job).await;
            }
        });
        Self(send)
    }

    async fn ask<T: Send + 'static>(
        &self,
        prepare: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let (reply, answer) = oneshot::channel();
        self.0
            .send(Box::new(move || {
                let _ = reply.send(prepare());
            }))
            .map_err(|_| "the assembly task stopped".to_owned())?;
        answer
            .await
            .map_err(|_| "the assembly task did not answer".to_owned())?
    }
}

type Prepare<T> = Box<dyn FnOnce() -> Result<T, String> + Send>;

enum Rest {
    Held(Issuer, Settlement),
    Lost,
}

type Settlement = Result<super::outlet::Settled, String>;

fn unreadable_row(error: &str) -> String {
    format!("the settlement fold cannot read a row this actor holds: {error}")
}

fn latest_unread(
    unread: &mut BTreeMap<ActorId, (Stamp<ActorId>, String)>,
    owner: &ActorId,
    at: &Stamp<ActorId>,
    error: &str,
) {
    if unread
        .get(owner)
        .is_none_or(|(latest, _)| latest.hlc() < at.hlc())
    {
        unread.insert(owner.clone(), (at.clone(), unreadable_row(error)));
    }
}

#[derive(Clone)]
pub(crate) struct Standing {
    pub(crate) declarations: RevisionDeclarations,
    pub(crate) revision: RevisionEpochId,
    pub(crate) grants: ProductGrantCatalog,
}

#[derive(Debug, Default)]
pub(crate) struct Stood {
    pub(crate) actors: Vec<NamedActorId>,
    pub(crate) unboarded: Vec<(NamedActorId, ActorType)>,
    pub(crate) failures: Vec<Unassembled>,
    pub(crate) retired: Vec<NamedActorId>,
}

#[derive(PartialEq)]
struct Attempt {
    inlets: circular_actors::ResolvedInletShapes,
    grants: ProductGrantCatalog,
}

impl Attempt {
    fn of(standing: &Standing, actor: &NamedActorId) -> Self {
        let (_, table, _) = standing.declarations.parts();
        Self {
            inlets: table.inlets_for(actor).clone(),
            grants: standing.grants.for_actors(std::iter::once(actor)),
        }
    }
}

#[derive(Clone)]
pub(crate) struct Entrance {
    address: Address,
}

impl Entrance {
    pub(crate) const fn address(&self) -> &Address {
        &self.address
    }

    pub(crate) fn of(address: Address) -> Self {
        Self { address }
    }

    pub(crate) fn inject(
        &self,
        inlet: PortId,
        payload: ProductPayload,
        origin: ExternalOrigin,
    ) -> Result<bool, Refusal> {
        self.send(inlet, payload, origin, false)
    }

    pub(crate) fn inject_patiently(
        &self,
        inlet: PortId,
        payload: ProductPayload,
        origin: ExternalOrigin,
    ) -> Result<bool, Refusal> {
        self.send(inlet, payload, origin, true)
    }

    fn send(
        &self,
        inlet: PortId,
        payload: ProductPayload,
        origin: ExternalOrigin,
        patient: bool,
    ) -> Result<bool, Refusal> {
        let (reply, answer) = oneshot::channel();
        self.address
            .send(Message::Inject(Injection {
                inlet,
                payload,
                origin,
                reply,
                patient,
            }))
            .map_err(|_| Refusal::Rejected("this boundary is no longer standing".to_owned()))?;
        answer
            .blocking_recv()
            .map_err(|_| Refusal::Rejected("this boundary stopped before recording".to_owned()))?
    }

    pub(crate) fn refuse(
        &self,
        inlet: Option<PortId>,
        subject: ProductPayload,
        reason: circular_runtime::DeadLetterReason,
    ) -> Result<(), ()> {
        self.address
            .send(Message::Refused(Box::new(super::source::Refused {
                inlet,
                subject,
                reason,
            })))
            .map_err(|_| ())
    }

    pub(crate) fn fall(&self, failure: crate::activation_detail::RegistrationFailure) {
        eprintln!("circular-kernel: a source worker fell: {failure}");
        let _ = self.address.send(Message::Fell(failure));
    }
}

pub(crate) struct Reconcile {
    pub(crate) standing: Standing,
}

pub(crate) struct Restoration {
    pub(crate) columns: crate::recorded::RecordedColumns,
    pub(crate) history: Vec<Standing>,
    pub(crate) admitted: BTreeMap<NamedActorId, Vec<super::column::Admitted>>,
    pub(crate) outcomes: BTreeMap<NamedActorId, Vec<super::column::Outcome>>,
    pub(crate) cell_facts:
        BTreeMap<NamedActorId, Vec<circular_store::Record<circular_store::ProductStore>>>,
    pub(crate) refused: BTreeMap<NamedActorId, String>,
    pub(crate) emitted: BTreeMap<ActorId, super::record::Emitted>,
}

pub(crate) fn cell_facts(
    records: &[circular_store::Record<circular_store::ProductStore>],
    lives: &super::record::LifeStarts,
) -> BTreeMap<NamedActorId, Vec<circular_store::Record<circular_store::ProductStore>>> {
    use circular_store::{ClassKey, ObservationKey};
    let mut columns: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for record in records {
        let ClassKey::Observation(ObservationKey::StreamItem(_, _, item)) = record.header().key()
        else {
            continue;
        };
        if !lives.holds(record.header().at().producer(), record.header().at()) {
            continue;
        }
        if !matches!(
            item.kind(),
            circular_core::BuiltinObservationName::InstanceTransition
                | circular_core::BuiltinObservationName::IncarnationTransition
        ) {
            continue;
        }
        let ActorId::Scoped {
            scope,
            local: circular_plan::LocalKey::Named(name),
        } = record.header().at().producer()
        else {
            continue;
        };
        columns
            .entry(NamedActorId::new(scope.clone(), name.clone()))
            .or_default()
            .push(record.clone());
    }
    columns
}

struct CellRestore {
    columns: crate::recorded::RecordedColumns,
    admitted: BTreeMap<NamedActorId, Vec<super::column::Admitted>>,
    outcomes: BTreeMap<NamedActorId, Vec<super::column::Outcome>>,
    history: Vec<Standing>,
    facts: BTreeMap<NamedActorId, Vec<circular_store::Record<circular_store::ProductStore>>>,
    refused: BTreeMap<NamedActorId, String>,
    emitted: BTreeMap<ActorId, super::record::Emitted>,
}

struct CellMaterial {
    rows: Vec<crate::recorded::RecordedArrival>,
    admitted: Vec<super::column::Admitted>,
    outcomes: Vec<super::column::Outcome>,
    facts: Vec<circular_store::Record<circular_store::ProductStore>>,
    refused: Option<String>,
    emitted: super::record::Emitted,
}

enum CellLife {
    New {
        previous: Option<tokio::task::JoinHandle<Result<super::actor::Ended, String>>>,
        held: Option<Issuer>,
    },
    Restored(Issuer, Column),
}

impl CellRestore {
    fn claim(&mut self, actor: &NamedActorId) -> Option<CellMaterial> {
        let material = CellMaterial {
            rows: self.columns.remove(actor).unwrap_or_default(),
            admitted: self.admitted.remove(actor).unwrap_or_default(),
            outcomes: self.outcomes.remove(actor).unwrap_or_default(),
            facts: self.facts.remove(actor).unwrap_or_default(),
            refused: self.refused.remove(actor),
            emitted: self
                .emitted
                .remove(&actor.as_actor_id())
                .unwrap_or_default(),
        };
        (!material.rows.is_empty()
            || !material.admitted.is_empty()
            || !material.outcomes.is_empty()
            || !material.facts.is_empty()
            || material.refused.is_some())
        .then_some(material)
    }

    fn column(
        &self,
        material: CellMaterial,
        stream: StreamId,
        execution: &Arc<crate::execution_profile::ProductExecutionProfile>,
        actor: &NamedActorId,
        share: &super::share::Share,
        container: &NamedActorId,
        key: &circular_runtime::InstanceKey,
        minted: &Stamp<ActorId>,
        latest: super::turn::Behavior,
        addresses: &BTreeMap<NamedActorId, Address>,
        credits: &BTreeMap<EdgeId, Credit>,
    ) -> Result<(super::turn::Behavior, Column), String> {
        let CellMaterial {
            rows,
            admitted,
            outcomes,
            facts,
            refused,
            emitted: _,
        } = material;
        if let Some(reason) = refused {
            return Err(reason);
        }
        let mut arrivals = rows
            .iter()
            .map(|row| super::column::recorded(stream, row))
            .chain(
                outcomes
                    .into_iter()
                    .map(|outcome| super::column::outcome(stream, actor.clone(), outcome)),
            )
            .collect::<Result<Vec<_>, _>>()
            .map_err(|unreadable| unreadable.0)?;
        arrivals.sort_by_key(|arrival| arrival.index);
        if arrivals
            .first()
            .is_some_and(|first| !is_lifecycle(&first.inlet))
        {
            return Err("the recorded column does not begin with this cell's activation".into());
        }
        let mut starts = BTreeSet::new();
        let mut skipped = Vec::new();
        let mut stopped = None;
        let mut initial = None;
        let mut prepared = BTreeMap::new();
        let mut inlets = BTreeMap::new();
        let mut outlets = BTreeMap::new();
        let mut flags = BTreeMap::new();
        let mut last = None;
        for arrival in arrivals
            .iter()
            .filter(|arrival| is_lifecycle(&arrival.inlet))
        {
            match super::turn::lifecycle(&arrival.inlet, arrival.event.payload().value())? {
                Some(super::actor::Life::Stop(consumed)) => {
                    stopped = Some((consumed, arrival.index));
                    continue;
                }
                Some(super::actor::Life::Activate | super::actor::Life::Become(_)) => {}
                _ => continue,
            }
            starts.insert(arrival.index);
            let revision = arrival.at.revision();
            let standing = self
                .history
                .iter()
                .find(|standing| standing.revision == revision)
                .ok_or("a cell activation names an absent revision")?;
            let prototype = super::share::cell_prototype(standing, container)
                .ok_or("a cell activation names an absent prototype")?;
            let shape = super::share::cell_shapes(&prototype, key)
                .map_err(|error| error.to_string())?
                .actors
                .remove(actor)
                .ok_or("a cell activation names an absent actor")?;
            let mut assembly = Assembly::new(
                stream,
                prototype.registry,
                prototype.grants,
                prototype.declarations,
                revision.get(),
            )
            .executing(execution.clone());
            let behavior = assembly
                .behavior(actor, &shape.declaration)
                .map_err(|failure| failure.reason)?;
            if initial.is_none() {
                initial = Some(behavior);
            } else {
                prepared.insert(
                    arrival.index,
                    Box::new(Incarnation {
                        behavior,
                        replace: true,
                    }),
                );
            }
            flags.insert(revision, shape.declaration.flags());
            let share = bind(shape, addresses, credits);
            inlets.insert(revision, share.inlets);
            outlets.insert(revision, share.outlets);
            last = Some(arrival.index);
            if let Some((consumed, at)) = stopped.take() {
                skipped.push(consumed..at);
            }
        }
        let consumed = consumption_boundary(&arrivals)?;
        flags.insert(share.revision, share.declaration.flags());
        let (behavior, pending) = match initial {
            Some(initial) => (
                initial,
                (!rows
                    .iter()
                    .any(|row| Some(row.index()) == last && row.causal_parents().contains(minted)))
                .then(|| {
                    (
                        share.revision,
                        Some(Box::new(Incarnation {
                            behavior: latest,
                            replace: true,
                        })),
                    )
                }),
            ),
            None => (latest, None),
        };
        let held = admitted
            .iter()
            .map(|admitted| {
                super::column::admitted(stream, admitted, credits)
                    .map(|delivery| (delivery, admitted.observed_at))
                    .map_err(|unreadable| unreadable.0)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((
            behavior,
            Column {
                snapshot: None,
                activate: arrivals.is_empty(),
                arrivals,
                consumed,
                prepared,
                pending,
                inlets: inlets.into_iter().collect(),
                outlets: outlets.into_iter().collect(),
                held,
                skipped,
                starts,
                flags: flags.into_iter().collect(),
                prototypes: Vec::new(),
                cell_facts: facts,
                cell_declarations: vec![(share.revision, share.declaration.clone())],
                settled: super::outlet::Settled::default(),
            },
        ))
    }
}

/// Unstamped daemon facts. Assembly supplies bodies; only the System task
/// gives them coordinates and submits them through its own column.
pub(crate) enum DaemonFact {
    Manifest(circular_store::RunManifest<circular_store::ProductStore>),
    Revision(circular_core::EncodedPayload),
    StreamStart {
        origin: circular_core::Tick,
        body: circular_store::ProductStreamStartBody,
    },
    Restart {
        origin: circular_core::Tick,
        body: circular_store::ProductRestartBody,
    },
    Health {
        actor: NamedActorId,
        state: circular_protocol::actor_events::ActorHealthState,
        reason: Option<circular_protocol::actor_events::ActorHealthReason>,
        since: circular_core::RecordedInstant,
    },
}

enum Command {
    Stand(
        RevisionEpochId,
        Prepare<Standing>,
        oneshot::Sender<Result<Stood, String>>,
    ),
    Restore(
        RevisionEpochId,
        Prepare<Restoration>,
        oneshot::Sender<Result<Stood, String>>,
    ),
    Reconcile(
        RevisionEpochId,
        Prepare<Reconcile>,
        oneshot::Sender<Result<Stood, String>>,
    ),
    ReadState(oneshot::Sender<SystemFold>),
    Entrance(NamedActorId, oneshot::Sender<Option<Entrance>>),
    Pause {
        revision: RevisionEpochId,
        force: bool,
        reply: oneshot::Sender<Acceptance>,
    },
    Harness {
        harness: circular_runtime::AgentHarnessName,
        binding: Option<(
            circular_runtime::NormalizedPath,
            crate::execution_profile::AgentExecutorFactory,
        )>,
        reply: oneshot::Sender<()>,
    },
    Harnesses(
        oneshot::Sender<
            Vec<(
                circular_runtime::AgentHarnessName,
                circular_runtime::NormalizedPath,
            )>,
        >,
    ),
    Depths {
        wake: Arc<crate::wake::Wake>,
        reply: oneshot::Sender<Vec<Asked>>,
    },
    Resume {
        revision: RevisionEpochId,
        reply: oneshot::Sender<Acceptance>,
    },
    RecordFacts {
        revision: RevisionEpochId,
        facts: Vec<DaemonFact>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Shutdown {
        revision: RevisionEpochId,
        body: circular_store::ProductShutdownBody,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Ended {
        actor: NamedActorId,
        address: Address,
    },
    Mint {
        container: NamedActorId,
        key: circular_runtime::InstanceKey,
        prototype: Arc<super::share::CellPrototype>,
        minted: Stamp<ActorId>,
        reply: Address,
        restore: bool,
    },
    RetireCell {
        container: NamedActorId,
        key: circular_runtime::InstanceKey,
        minted: Stamp<ActorId>,
        retired: Stamp<ActorId>,
    },
}

pub(crate) struct Asked {
    pub(crate) actor: NamedActorId,
    pub(crate) answer: oneshot::Receiver<Vec<super::inlet::EdgeDepth>>,
}

#[derive(Clone)]
pub(crate) struct CellHand(mpsc::UnboundedSender<Command>);

impl CellHand {
    #[cfg(test)]
    pub(super) fn for_test() -> Self {
        Self(mpsc::unbounded_channel().0)
    }

    pub(crate) fn mint(
        &self,
        container: &NamedActorId,
        key: &circular_runtime::InstanceKey,
        prototype: Arc<super::share::CellPrototype>,
        minted: Stamp<ActorId>,
        reply: Address,
        restore: bool,
    ) {
        let _ = self.0.send(Command::Mint {
            container: container.clone(),
            key: key.clone(),
            prototype,
            minted,
            reply,
            restore,
        });
    }

    pub(crate) fn retire(
        &self,
        container: &NamedActorId,
        key: &circular_runtime::InstanceKey,
        minted: Stamp<ActorId>,
        retired: Stamp<ActorId>,
    ) {
        let _ = self.0.send(Command::RetireCell {
            container: container.clone(),
            key: key.clone(),
            minted,
            retired,
        });
    }
}

struct Lifetime {
    commands: mpsc::UnboundedSender<Command>,
    actor: NamedActorId,
    address: Address,
}

impl Drop for Lifetime {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Ended {
            actor: self.actor.clone(),
            address: self.address.clone(),
        });
    }
}

pub(crate) struct CellStood {
    pub(crate) key: circular_runtime::InstanceKey,
    pub(crate) minted: Stamp<ActorId>,
    pub(crate) wires: BTreeMap<EdgeId, Vec<super::share::OutWire>>,
}

struct Cell {
    owner: Address,
    minted: Stamp<ActorId>,
    actors: Vec<NamedActorId>,
    credits: BTreeMap<EdgeId, Credit>,
    forwards: BTreeMap<EdgeId, Vec<super::share::OutletShape>>,
}

#[derive(Clone)]
pub(crate) struct Pipeline {
    commands: mpsc::UnboundedSender<Command>,
    _runtime: Arc<tokio::runtime::Runtime>,
    paused: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Debug)]
pub(crate) struct Stopped;

impl Pipeline {
    pub(crate) fn start(
        clock: Arc<dyn circular_core::TickSource>,
        stream: StreamId,
        journal: crate::ProductDurableArrivalJournal,
        execution: Arc<crate::execution_profile::ProductExecutionProfile>,
        approvals: &crate::RuntimeApprovalQueue,
    ) -> std::io::Result<Self> {
        let actor = ActorId::System(circular_plan::SystemActor::Pipeline);
        let column = journal.open_column(&actor).map_err(std::io::Error::other)?;
        let view = journal.read_view();
        let mut state = SystemFold::default();
        let mut issuers = BTreeMap::<ActorId, Issuer>::new();
        let mut settled = BTreeMap::<ActorId, super::outlet::Settled>::new();
        let mut unread = BTreeMap::<ActorId, (Stamp<ActorId>, String)>::new();
        let mut lives = super::record::LifeStarts::default();
        for row in view.all_rows().map_err(std::io::Error::other)? {
            let row = row.map_err(std::io::Error::other)?;
            let record = row.record();
            if let Some(body) = SystemBody::read(&record).map_err(std::io::Error::other)? {
                state.apply(record.header().at(), &body);
            }
            let owner = super::record::owner(&record);
            lives.retain(&record);
            if let Err(error) = super::outlet::settle(&mut settled, &record) {
                latest_unread(&mut unread, owner, record.header().at(), &error);
            }
            if !issuers.contains_key(owner) {
                issuers.insert(owner.clone(), Issuer::new(owner.clone()));
            }
            issuers
                .get_mut(owner)
                .expect("an issuer per owner")
                .retain(&record)
                .map_err(std::io::Error::other)?;
        }
        let issuer = issuers.remove(&actor).unwrap_or_else(|| Issuer::new(actor));
        let paused = Arc::new(std::sync::atomic::AtomicBool::new(state.pause.is_some()));
        let runtime = Arc::new(
            tokio::runtime::Builder::new_multi_thread()
                .enable_time()
                .thread_name("circular-actor")
                .build()?,
        );
        let approvals = approvals.start(runtime.handle()).ok_or_else(|| {
            std::io::Error::other("the approval ledger is already standing or unavailable")
        })?;
        let time = Arc::new(TimeActor::spawn(runtime.handle(), clock));
        let (commands, inbox) = mpsc::unbounded_channel();
        let own = commands.clone();
        let system = System {
            runtime: runtime.handle().clone(),
            stream,
            journal,
            column,
            time,
            execution,
            approvals,
            members: BTreeMap::new(),
            retiring: BTreeMap::new(),
            resting: issuers
                .into_iter()
                .map(|(actor, issuer)| {
                    let settled = match unread.remove(&actor) {
                        Some((at, reason)) if lives.holds(&actor, &at) => Err(reason),
                        _ => Ok(settled.remove(&actor).unwrap_or_default()),
                    };
                    (actor, Rest::Held(issuer, settled))
                })
                .collect(),
            state,
            paused: paused.clone(),
            issuer,
            preparation: Preparation::spawn(runtime.handle()),
            shapes: BTreeMap::new(),
            credits: BTreeMap::new(),
            commands: own,
            cells: BTreeMap::new(),
            restoration: None,
            refused: BTreeMap::new(),
            standing: None,
        };
        runtime.spawn(system.live(inbox));
        Ok(Self {
            commands,
            _runtime: runtime,
            paused,
        })
    }

    fn ask<T>(&self, command: impl FnOnce(oneshot::Sender<T>) -> Command) -> Result<T, Stopped> {
        let (reply, answer) = oneshot::channel();
        self.commands.send(command(reply)).map_err(|_| Stopped)?;
        answer.blocking_recv().map_err(|_| Stopped)
    }

    /// Read System's own durable column fold through its mailbox. Call after
    /// `start`, before recovery, to distinguish no recovery from success/failure.
    /// The same handle reads subsequent outcomes even when user preparation fails.
    /// Restored state is handed in at spawn; this request never reads the journal.
    pub(crate) fn system_state(&self) -> Result<SystemFold, Stopped> {
        self.ask(Command::ReadState)
    }

    pub(crate) fn stand(&self, standing: Standing) -> Result<Stood, String> {
        self.activate(standing.revision, move || Ok(standing))
    }

    /// System stands before this preparation runs. Keep this handle even when
    /// user preparation fails; its column and control mailbox remain available.
    pub(crate) fn activate(
        &self,
        revision: RevisionEpochId,
        prepare: impl FnOnce() -> Result<Standing, String> + Send + 'static,
    ) -> Result<Stood, String> {
        self.ask(|reply| Command::Stand(revision, Box::new(prepare), reply))
            .map_err(|_| "the actor kernel stopped before activation".to_owned())?
    }

    pub(crate) fn restore(&self, restoration: Restoration) -> Result<Stood, String> {
        let revision = restoration
            .history
            .last()
            .ok_or("restoration names no revision")?
            .revision;
        self.recover(revision, move || Ok(restoration))
    }

    pub(crate) fn recover(
        &self,
        revision: RevisionEpochId,
        prepare: impl FnOnce() -> Result<Restoration, String> + Send + 'static,
    ) -> Result<Stood, String> {
        self.ask(|reply| Command::Restore(revision, Box::new(prepare), reply))
            .map_err(|_| "the actor kernel stopped before recovery".to_owned())?
    }

    pub(crate) fn reconcile(&self, reconcile: Reconcile) -> Result<Stood, String> {
        self.adopt(reconcile.standing.revision, move || Ok(reconcile))
    }

    pub(crate) fn adopt(
        &self,
        revision: RevisionEpochId,
        prepare: impl FnOnce() -> Result<Reconcile, String> + Send + 'static,
    ) -> Result<Stood, String> {
        self.ask(|reply| Command::Reconcile(revision, Box::new(prepare), reply))
            .map_err(|_| "the actor kernel stopped before revision adoption".to_owned())?
    }

    pub(crate) fn entrance(&self, actor: &NamedActorId) -> Result<Option<Entrance>, Stopped> {
        self.ask(|reply| Command::Entrance(actor.clone(), reply))
    }

    pub(crate) fn pause(&self, revision: RevisionEpochId, force: bool) -> Acceptance {
        self.ask(|reply| Command::Pause {
            revision,
            force,
            reply,
        })
        .map_err(|_| RejectionReason::ArrivalRecorderStopped)?
    }

    pub(crate) fn bind_harness(
        &self,
        harness: circular_runtime::AgentHarnessName,
        binding: Option<(
            circular_runtime::NormalizedPath,
            crate::execution_profile::AgentExecutorFactory,
        )>,
    ) -> Result<(), Stopped> {
        self.ask(|reply| Command::Harness {
            harness,
            binding,
            reply,
        })
    }

    pub(crate) fn harnesses(
        &self,
    ) -> Result<
        Vec<(
            circular_runtime::AgentHarnessName,
            circular_runtime::NormalizedPath,
        )>,
        Stopped,
    > {
        self.ask(Command::Harnesses)
    }

    pub(crate) fn edge_depths(&self, wake: Arc<crate::wake::Wake>) -> Result<Vec<Asked>, Stopped> {
        self.ask(|reply| Command::Depths { wake, reply })
    }

    pub(crate) fn resume(&self, revision: RevisionEpochId) -> Acceptance {
        self.ask(|reply| Command::Resume { revision, reply })
            .map_err(|_| RejectionReason::ArrivalRecorderStopped)?
    }

    pub(crate) fn paused(&self) -> bool {
        self.paused.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(crate) fn record_facts(
        &self,
        revision: RevisionEpochId,
        facts: Vec<DaemonFact>,
    ) -> Result<(), String> {
        self.ask(|reply| Command::RecordFacts {
            revision,
            facts,
            reply,
        })
        .map_err(|_| "System stopped before recording facts".to_owned())?
    }

    /// Stop members, commit System's final fact, then answer. That submission
    /// also closes the writer, including interrupted recovery.
    pub(crate) fn shutdown(
        &self,
        revision: RevisionEpochId,
        body: circular_store::ProductShutdownBody,
    ) -> Result<(), String> {
        self.ask(|reply| Command::Shutdown {
            revision,
            body,
            reply,
        })
        .map_err(|_| "System stopped before shutdown".to_owned())?
    }
}

struct Member {
    address: Address,
    door: Arc<Door>,
    task: tokio::task::JoinHandle<Result<super::actor::Ended, String>>,
    actor_type: ActorType,
}

struct System {
    runtime: tokio::runtime::Handle,
    shapes: BTreeMap<NamedActorId, ShareShape>,
    credits: BTreeMap<EdgeId, Credit>,
    stream: StreamId,
    journal: crate::ProductDurableArrivalJournal,
    column: crate::ColumnJournal,
    time: Arc<TimeActor>,
    execution: Arc<crate::execution_profile::ProductExecutionProfile>,
    approvals: crate::runtime_approval::ApprovalReturns,
    members: BTreeMap<NamedActorId, Member>,
    retiring: BTreeMap<NamedActorId, tokio::task::JoinHandle<Result<super::actor::Ended, String>>>,
    resting: BTreeMap<ActorId, Rest>,
    state: SystemFold,
    paused: Arc<std::sync::atomic::AtomicBool>,
    issuer: Issuer,
    preparation: Preparation,
    commands: mpsc::UnboundedSender<Command>,
    cells: BTreeMap<NamedActorId, BTreeMap<circular_runtime::InstanceKey, Cell>>,
    restoration: Option<CellRestore>,
    refused: BTreeMap<NamedActorId, Attempt>,
    standing: Option<Standing>,
}

fn ended_fault(
    ended: Result<Result<super::actor::Ended, String>, tokio::task::JoinError>,
) -> Option<String> {
    match ended {
        Ok(Ok(ended)) => ended.fault,
        Ok(Err(reason)) => Some(reason),
        Err(error) => Some(error.to_string()),
    }
}

impl System {
    async fn live(mut self, mut inbox: mpsc::UnboundedReceiver<Command>) {
        while let Some(command) = inbox.recv().await {
            while let Some(actor) = self
                .retiring
                .iter()
                .find(|(_, task)| task.is_finished())
                .map(|(actor, _)| actor.clone())
            {
                let ended = self
                    .retiring
                    .remove(&actor)
                    .expect("a retiring actor")
                    .await;
                self.rest(&actor, ended);
            }
            match command {
                Command::Stand(revision, prepare, reply) => {
                    let stood = match self.preparation.ask(prepare).await {
                        Ok(standing) => Ok(self.stand(standing).await),
                        Err(reason) => Err(reason),
                    };
                    let result = self.finish(OutcomeKind::Activation, revision, stood).await;
                    let _ = reply.send(result);
                }
                Command::Restore(revision, prepare, reply) => {
                    let stood = match self.preparation.ask(prepare).await {
                        Ok(restoration) => self.restore(restoration).await,
                        Err(reason) => Err(reason),
                    };
                    let result = self.finish(OutcomeKind::Recovery, revision, stood).await;
                    let _ = reply.send(result);
                }
                Command::Reconcile(revision, prepare, reply) => {
                    let stood = match self.preparation.ask(prepare).await {
                        Ok(reconcile) => Ok(self.reconcile(reconcile).await),
                        Err(reason) => Err(reason),
                    };
                    let result = self
                        .finish(OutcomeKind::RevisionAdoption, revision, stood)
                        .await;
                    let _ = reply.send(result);
                }
                Command::ReadState(reply) => {
                    let _ = reply.send(self.state.clone());
                }
                Command::Entrance(actor, reply) => {
                    let entrance = self.members.get(&actor).map(|member| Entrance {
                        address: member.address.clone(),
                    });
                    let _ = reply.send(entrance);
                }
                Command::Pause {
                    revision,
                    force,
                    reply,
                } => {
                    let accepted = self
                        .record(revision, SystemBody::PauseAccepted { force })
                        .await;
                    if accepted.is_ok() {
                        for member in self.members.values() {
                            let _ = member
                                .address
                                .send(Message::Control(Control::Pause { force }));
                        }
                    }
                    let _ = reply.send(accepted);
                }
                Command::Harness {
                    harness,
                    binding,
                    reply,
                } => {
                    self.bind_harness(harness, binding);
                    let _ = reply.send(());
                }
                Command::Harnesses(reply) => {
                    let _ = reply.send(
                        self.execution
                            .bound_programs()
                            .map(|(harness, program)| (harness.clone(), program.clone()))
                            .collect(),
                    );
                }
                Command::Depths { wake, reply } => {
                    let asked = self
                        .members
                        .iter()
                        .map(|(actor, member)| {
                            let (ask, answer) = oneshot::channel();
                            let _ = member
                                .address
                                .send(Message::Depths(super::DepthsReply::new(ask, wake.clone())));
                            Asked {
                                actor: actor.clone(),
                                answer,
                            }
                        })
                        .collect();
                    let _ = reply.send(asked);
                }
                Command::Resume { revision, reply } => {
                    let accepted = self.record(revision, SystemBody::ResumeAccepted).await;
                    if accepted.is_ok() {
                        for member in self.members.values() {
                            let _ = member.address.send(Message::Control(Control::Resume));
                        }
                    }
                    let _ = reply.send(accepted);
                }
                Command::Mint {
                    container,
                    key,
                    prototype,
                    minted,
                    reply,
                    restore,
                } => {
                    self.mint(&container, &key, &prototype, &minted, reply, restore)
                        .await
                }
                Command::RetireCell {
                    container,
                    key,
                    minted,
                    retired,
                } => {
                    if !self.retire_cell(&container, &key, &minted)
                        && self
                            .cells
                            .get(&container)
                            .is_some_and(|cells| cells.contains_key(&key))
                    {
                        self.reject_cell_request(&container, &retired).await;
                    }
                }
                Command::Ended { actor, address } => self.ended(&actor, &address),
                Command::RecordFacts {
                    revision,
                    facts,
                    reply,
                } => {
                    let result = self.record_facts(revision, facts).await;
                    let _ = reply.send(result);
                }
                Command::Shutdown {
                    revision,
                    body,
                    reply,
                } => {
                    for member in self.members.values() {
                        member.door.set(true);
                    }
                    for (actor, member) in std::mem::take(&mut self.members) {
                        let _ = member.address.send(Message::Control(Control::Stop));
                        if let Some(fault) = ended_fault(member.task.await) {
                            eprintln!("circular-kernel: {actor:?} ended: {fault}");
                        }
                    }
                    for (_, task) in std::mem::take(&mut self.retiring) {
                        if let Some(fault) = ended_fault(task.await) {
                            eprintln!("circular-kernel: a retiring actor ended: {fault}");
                        }
                    }
                    let result = self.record_shutdown(revision, body).await;
                    let _ = reply.send(result);
                    return;
                }
            }
        }
    }

    fn bind_harness(
        &mut self,
        harness: circular_runtime::AgentHarnessName,
        binding: Option<(
            circular_runtime::NormalizedPath,
            crate::execution_profile::AgentExecutorFactory,
        )>,
    ) {
        let executor = binding.as_ref().map(|(_, factory)| factory.clone());
        self.execution = Arc::new(
            (*self.execution)
                .clone()
                .with_agent_binding(harness.clone(), binding),
        );
        for member in self
            .members
            .values()
            .filter(|member| member.actor_type == ActorType::Agent)
        {
            let _ = member.address.send(Message::Control(Control::Harness {
                harness: harness.clone(),
                executor: executor.clone(),
            }));
        }
    }

    async fn stamp(
        &mut self,
        revision: RevisionEpochId,
        origin: Option<circular_core::Tick>,
    ) -> Result<Stamp<ActorId>, String> {
        let sample = match origin {
            Some(sample) => sample,
            None => self.time.query_tick().await.ok_or("time actor stopped")?,
        };
        self.issuer
            .observation(sample, revision)
            .map_err(|e| format!("System stamp: {e:?}"))
    }

    async fn submit(
        &self,
        records: Vec<circular_store::Record<circular_store::ProductStore>>,
    ) -> Result<(), String> {
        self.column
            .submit_records(records, Vec::new())
            .await
            .map_err(|_| "System column writer stopped".to_owned())?
    }

    async fn record_facts(
        &mut self,
        revision: RevisionEpochId,
        facts: Vec<DaemonFact>,
    ) -> Result<(), String> {
        use circular_store::{Record, StructureRecord};
        let origin = facts.iter().find_map(|fact| match fact {
            DaemonFact::StreamStart { origin, .. } | DaemonFact::Restart { origin, .. } => {
                Some(*origin)
            }
            _ => None,
        });
        let mut records = Vec::with_capacity(facts.len());
        for fact in facts {
            let at = self.stamp(revision, origin).await?;
            records.push(match fact {
                DaemonFact::Manifest(body) => {
                    Record::Structure(StructureRecord::manifest(at, body))
                }
                DaemonFact::Revision(body) => Record::Structure(StructureRecord::graph_revision(
                    circular_plan::ScopeId::root(),
                    at,
                    body,
                )),
                DaemonFact::StreamStart { body, .. } => {
                    circular_store::stream_start_record(at, &body)?
                }
                DaemonFact::Restart { body, .. } => circular_store::restart_record(at, &body)?,
                DaemonFact::Health {
                    actor,
                    state,
                    reason,
                    since,
                } => {
                    let fact = crate::actor_records::HealthFact {
                        actor,
                        state,
                        reason,
                        since,
                        mailbox_depths: None,
                        at: at.clone(),
                    };
                    crate::actor_records::actor_health_record(
                        at,
                        &crate::actor_records::health_transition(&fact)?,
                    )?
                }
            });
        }
        if records.is_empty() {
            return Ok(());
        }
        self.submit(records).await
    }

    async fn record_shutdown(
        &mut self,
        revision: RevisionEpochId,
        body: circular_store::ProductShutdownBody,
    ) -> Result<(), String> {
        let at = self.stamp(revision, None).await?;
        self.submit(vec![circular_store::shutdown_record(at, &body)?])
            .await
    }

    async fn record(&mut self, revision: RevisionEpochId, body: SystemBody) -> Acceptance {
        use circular_core::{Boundary, Ceilings, EncodedPayload, PayloadVersionTag};
        use circular_store::{
            ObservationBucket, ObservationItemKey, ObservationRecord, OpaqueId, Record,
            RecordOrigin,
        };
        let failed = RejectionReason::ArrivalRecorderStopped;
        let sample = self.time.query_tick().await.ok_or(failed)?;
        let at = self.issuer.observation(sample, revision).map_err(|error| {
            eprintln!("circular-kernel: System stamp: {error:?}");
            failed
        })?;
        let bytes =
            circular_core::encode(&body.to_value(), Ceilings::for_boundary(Boundary::Journal))
                .map_err(|_| failed)?;
        let millis =
            u128::from(sample.get()) * 1_000 / u128::from(self.time.ticks_per_second().max(1));
        let bucket = ObservationBucket::from_millis(u64::try_from(millis).unwrap_or(u64::MAX));
        let origin = RecordOrigin::Stream;
        let item = ObservationItemKey::new(body.name(), OpaqueId::new(0));
        let payload = EncodedPayload::new(PayloadVersionTag::FIRST, &bytes);
        let row = match body {
            SystemBody::Outcome { .. } => {
                ObservationRecord::diagnostic(at.clone(), bucket, origin, item, payload)
            }
            _ => ObservationRecord::lifecycle(at.clone(), bucket, origin, item, payload),
        };
        self.column
            .submit_records(vec![Record::Observation(row)], Vec::new())
            .await
            .map_err(|_| failed)?
            .map_err(|error| {
                eprintln!("circular-kernel: System record: {error}");
                failed
            })?;
        self.state.apply(&at, &body);
        self.paused.store(
            self.state.pause.is_some(),
            std::sync::atomic::Ordering::SeqCst,
        );
        Ok(at)
    }

    async fn finish(
        &mut self,
        kind: OutcomeKind,
        revision: RevisionEpochId,
        stood: Result<Stood, String>,
    ) -> Result<Stood, String> {
        if let Ok(stood) = &stood {
            self.record_unstood(kind, revision, &stood.failures)
                .await
                .map_err(|reason| {
                    format!("System could not record an actor that did not stand: {reason}")
                })?;
        }
        let result = match &stood {
            Ok(_) => Ok(()),
            Err(_) => Err(kind.failure()),
        };
        self.record(
            revision,
            SystemBody::Outcome {
                kind,
                revision,
                result,
            },
        )
        .await
        .map_err(|reason| format!("System outcome could not be recorded: {reason:?}"))?;
        stood
    }

    async fn record_unstood(
        &mut self,
        kind: OutcomeKind,
        revision: RevisionEpochId,
        failures: &[Unassembled],
    ) -> Result<(), String> {
        use circular_protocol::actor_events::{
            ActorHealthReason, ActorHealthReasonCode, ActorHealthState,
        };
        if failures.is_empty() {
            return Ok(());
        }
        let sample = self.time.query_tick().await.ok_or("time actor stopped")?;
        let millis =
            u128::from(sample.get()) * 1_000 / u128::from(self.time.ticks_per_second().max(1));
        let since =
            circular_core::RecordedInstant::from_millis(u64::try_from(millis).unwrap_or(u64::MAX));
        let facts = failures
            .iter()
            .map(|failure| {
                eprintln!(
                    "circular-kernel: {:?} did not stand: {}",
                    failure.actor, failure.reason
                );
                let reason = match kind {
                    OutcomeKind::Recovery => ActorHealthReason {
                        code: ActorHealthReasonCode::RecoveryRefused,
                        detail: circular_core::Value::Null,
                    },
                    OutcomeKind::Activation | OutcomeKind::RevisionAdoption => ActorHealthReason {
                        code: ActorHealthReasonCode::ActivationFailed,
                        detail: failure
                            .detail
                            .map_or(circular_core::Value::Null, |detail| detail.to_value()),
                    },
                };
                DaemonFact::Health {
                    actor: failure.actor.clone(),
                    state: ActorHealthState::Failed,
                    reason: Some(reason),
                    since,
                }
            })
            .collect();
        self.record_facts(revision, facts).await
    }

    fn remember_unstood(&mut self, standing: &Standing, stood: &Stood) {
        for actor in &stood.actors {
            self.refused.remove(actor);
        }
        for failure in &stood.failures {
            self.refused
                .insert(failure.actor.clone(), Attempt::of(standing, &failure.actor));
        }
    }

    async fn stand(&mut self, standing: Standing) -> Stood {
        let (graph, _, _) = standing.declarations.parts();
        let shapes = super::share::shapes(&standing);
        let actors: Vec<NamedActorId> = graph.actors().keys().cloned().collect();
        let mut stood = Stood::default();
        self.credits = credits(&shapes, &BTreeMap::new(), &BTreeMap::new());
        let behaviors = self.assemble(&standing, &shapes, actors.iter(), &mut stood);
        self.spawn_all(behaviors, &shapes, &mut stood).await;
        self.shapes = shapes;
        self.remember_unstood(&standing, &stood);
        self.standing = Some(standing);
        stood
    }

    async fn reconcile(&mut self, reconcile: Reconcile) -> Stood {
        let Reconcile { standing } = reconcile;
        let shapes = super::share::shapes(&standing);
        self.credits = credits(&shapes, &self.shapes, &self.credits);
        let mut stood = Stood::default();
        let standing_cells: BTreeSet<NamedActorId> = self
            .cells
            .values()
            .flat_map(|cells| cells.values().flat_map(|cell| cell.actors.iter().cloned()))
            .collect();
        let removed: Vec<NamedActorId> = self
            .members
            .keys()
            .filter(|actor| !shapes.contains_key(*actor) && !standing_cells.contains(*actor))
            .cloned()
            .collect();
        self.refused.retain(|actor, _| shapes.contains_key(actor));
        let raised: BTreeSet<NamedActorId> = shapes
            .iter()
            .filter(|(actor, shape)| {
                !self.members.contains_key(*actor)
                    && self.refused.get(*actor).is_none_or(|attempt| {
                        self.shapes
                            .get(*actor)
                            .is_none_or(|before| !before.same_as(shape))
                            || attempt != &Attempt::of(&standing, actor)
                    })
            })
            .map(|(actor, _)| actor.clone())
            .collect();
        let changed: Vec<NamedActorId> = self
            .members
            .keys()
            .filter(|actor| {
                shapes.get(*actor).is_some_and(|shape| {
                    self.shapes
                        .get(*actor)
                        .is_none_or(|before| before.declaration != shape.declaration)
                })
            })
            .cloned()
            .collect();
        let touched: Vec<NamedActorId> = self
            .members
            .keys()
            .filter(|actor| {
                shapes.get(*actor).is_some_and(|shape| {
                    shape.is_entry()
                        || self
                            .shapes
                            .get(*actor)
                            .is_none_or(|before| !before.same_as(shape))
                })
            })
            .cloned()
            .collect();
        let addresses = self.addresses();
        for actor in &touched {
            if let (Some(member), Some(shape)) = (self.members.get(actor), shapes.get(actor)) {
                let share = bind(shape.clone(), &addresses, &self.credits);
                let _ = member.address.send(Message::Inlets(Box::new(share)));
            }
        }
        let behaviors = self.assemble(&standing, &shapes, raised.iter(), &mut stood);
        self.spawn_all(behaviors, &shapes, &mut stood).await;
        let rebound: BTreeSet<NamedActorId> = self
            .members
            .keys()
            .filter(|actor| {
                !touched.contains(*actor)
                    && !stood.actors.contains(*actor)
                    && shapes.get(*actor).is_some_and(|shape| {
                        shape
                            .outlets
                            .values()
                            .flatten()
                            .any(|outlet| stood.actors.contains(outlet.to.actor()))
                    })
            })
            .cloned()
            .collect();
        let mut incarnations = self.assemble(&standing, &shapes, changed.iter(), &mut stood);
        let mut unstood: Vec<NamedActorId> = changed
            .iter()
            .filter(|actor| !incarnations.contains_key(*actor))
            .cloned()
            .collect();
        let addresses = self.addresses();
        for (actor, member) in &mut self.members {
            if stood.actors.contains(actor)
                || unstood.contains(actor)
                || !(touched.contains(actor) || rebound.contains(actor))
            {
                continue;
            }
            let Some(shape) = shapes.get(actor) else {
                continue;
            };
            member.actor_type = *shape.declaration.domain().actor_type();
            let incarnation = incarnations.remove(actor).map(|behavior| {
                let replace = self.shapes.get(actor).is_none_or(|before| {
                    before.declaration.authored_generation()
                        != shape.declaration.authored_generation()
                        || before.declaration.domain().actor_type()
                            != shape.declaration.domain().actor_type()
                });
                Box::new(Incarnation { behavior, replace })
            });
            let share = bind(shape.clone(), &addresses, &self.credits);
            let _ = member
                .address
                .send(Message::Share(Box::new(share), incarnation));
        }
        unstood.extend(removed.iter().cloned());
        for actor in &unstood {
            if let Some(member) = self.members.remove(actor) {
                member.door.set(true);
                let _ = member.address.send(Message::Control(Control::Retire));
                self.retiring.insert(actor.clone(), member.task);
            }
        }
        stood.retired = removed;
        self.shapes = shapes;
        self.remember_unstood(&standing, &stood);
        self.standing = Some(standing);
        stood
    }

    async fn mint(
        &mut self,
        container: &NamedActorId,
        key: &circular_runtime::InstanceKey,
        prototype: &super::share::CellPrototype,
        minted: &Stamp<ActorId>,
        reply: Address,
        restore: bool,
    ) {
        if let Some(cell) = self.cells.get(container).and_then(|cells| cells.get(key)) {
            if &cell.minted != minted {
                let previous = cell.minted.clone();
                self.retire_cell(container, key, &previous);
            }
        }
        if !self
            .cells
            .get(container)
            .is_some_and(|cells| cells.contains_key(key))
        {
            let mut restoration = self.restoration.take();
            let stood = self
                .stand_cell(
                    container,
                    key,
                    prototype,
                    minted,
                    reply.clone(),
                    restoration.as_mut(),
                    restore,
                )
                .await;
            self.restoration = restoration;
            match stood {
                Ok(failures) => {
                    let kind = if restore {
                        OutcomeKind::Recovery
                    } else {
                        OutcomeKind::Activation
                    };
                    if let Err(reason) = self
                        .record_unstood(kind, minted.revision(), &failures)
                        .await
                    {
                        eprintln!(
                            "circular-kernel: a cell actor that did not stand was not recorded: {reason}"
                        );
                    }
                }
                Err(reason) => eprintln!(
                    "circular-kernel: {container:?} could not mint the cell {key:?}: {reason}"
                ),
            }
        }
        let wires = self
            .cells
            .get(container)
            .and_then(|cells| cells.get(key))
            .map(|cell| self.forward_wires(cell))
            .unwrap_or_default();
        let _ = reply.send(Message::CellStood(Box::new(CellStood {
            key: key.clone(),
            minted: minted.clone(),
            wires,
        })));
    }

    fn ended(&mut self, actor: &NamedActorId, address: &Address) {
        self.owner_ended(actor, address);
        if !self
            .members
            .get(actor)
            .is_some_and(|member| member.address.same_channel(address))
        {
            return;
        }
        let member = self.members.remove(actor).expect("the ended member");
        self.retiring.insert(actor.clone(), member.task);
        if let Some(standing) = &self.standing
            && self.shapes.contains_key(actor)
        {
            self.refused
                .insert(actor.clone(), Attempt::of(standing, actor));
        }
    }

    fn launch(
        &self,
        actor: &NamedActorId,
        address: &Address,
        life: impl std::future::Future<Output = Result<super::actor::Ended, String>> + Send + 'static,
    ) -> tokio::task::JoinHandle<Result<super::actor::Ended, String>> {
        let lifetime = Lifetime {
            commands: self.commands.clone(),
            actor: actor.clone(),
            address: address.clone(),
        };
        self.runtime.spawn(async move {
            let _lifetime = lifetime;
            life.await
        })
    }

    fn owner_ended(&mut self, container: &NamedActorId, owner: &Address) {
        let ended: Vec<_> = self
            .cells
            .get(container)
            .into_iter()
            .flat_map(|cells| cells.iter())
            .filter(|(_, cell)| cell.owner.same_channel(owner))
            .map(|(key, cell)| (key.clone(), cell.minted.clone()))
            .collect();
        for (key, minted) in ended {
            self.retire_cell(container, &key, &minted);
        }
    }

    fn retire_cell(
        &mut self,
        container: &NamedActorId,
        key: &circular_runtime::InstanceKey,
        minted: &Stamp<ActorId>,
    ) -> bool {
        let Some(cells) = self.cells.get_mut(container) else {
            return false;
        };
        if !cells.get(key).is_some_and(|cell| &cell.minted == minted) {
            return false;
        }
        let cell = cells.remove(key).expect("the requested cell life");
        for actor in cell.actors {
            if let Some(member) = self.members.remove(&actor) {
                member.door.set(true);
                let _ = member.address.send(Message::Control(Control::Retire));
                self.retiring.insert(actor.clone(), member.task);
            }
        }
        true
    }

    async fn reject_cell_request(&mut self, container: &NamedActorId, dropped: &Stamp<ActorId>) {
        let record = circular_runtime::DeadLetterRecord::new(
            super::turn::null_payload(),
            circular_runtime::DeadLetterOrigin::new(container.as_actor_id(), None),
            circular_runtime::DeadLetterReason::DestinationGone,
            dropped.clone(),
        );
        let result = match self.stamp(dropped.revision(), None).await.and_then(|at| {
            crate::dead_letter_writer::product_dead_letter_record(
                container.scope(),
                &record,
                at,
                circular_store::RecordOrigin::Stream,
            )
        }) {
            Ok(record) => self
                .column
                .submit_records(vec![record], Vec::new())
                .await
                .unwrap_or_else(|_| Err("the recorder stopped before the cell refusal".to_owned())),
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            eprintln!("circular-kernel: cell request refusal was not recorded: {error}");
        }
    }

    fn forward_wires(&self, cell: &Cell) -> BTreeMap<EdgeId, Vec<super::share::OutWire>> {
        cell.forwards
            .iter()
            .map(|(edge, outlets)| {
                let wires = outlets
                    .iter()
                    .filter_map(|outlet| {
                        Some(super::share::OutWire {
                            edge: outlet.edge.clone(),
                            to: outlet.to.clone(),
                            address: self.members.get(outlet.to.actor())?.address.clone(),
                            credit: cell.credits.get(&outlet.edge).cloned(),
                        })
                    })
                    .collect();
                (edge.clone(), wires)
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    async fn stand_cell(
        &mut self,
        container: &NamedActorId,
        key: &circular_runtime::InstanceKey,
        prototype: &super::share::CellPrototype,
        minted: &Stamp<ActorId>,
        owner: Address,
        mut restoration: Option<&mut CellRestore>,
        restore: bool,
    ) -> Result<Vec<Unassembled>, String> {
        let shapes =
            super::share::cell_shapes(prototype, key).map_err(|error| error.to_string())?;
        let cell_credits = credits(&shapes.actors, &BTreeMap::new(), &BTreeMap::new());
        let mut bound = self.credits.clone();
        bound.extend(cell_credits.clone());
        let mut assembly = Assembly::new(
            self.stream,
            prototype.registry,
            prototype.grants.clone(),
            prototype.declarations.clone(),
            prototype.revision.get(),
        )
        .executing(self.execution.clone());
        let mut failures = Vec::new();
        let mut behaviors = BTreeMap::new();
        for (actor, shape) in &shapes.actors {
            let material = restoration
                .as_deref_mut()
                .and_then(|restoration| restoration.claim(actor))
                .filter(|_| restore);
            match assembly.behavior(actor, &shape.declaration) {
                Ok(behavior) => {
                    behaviors.insert(actor.clone(), (behavior, material));
                }
                Err(failure) => failures.push(failure),
            }
        }
        let mut addresses = self.addresses();
        let mut receivers = BTreeMap::new();
        for actor in behaviors.keys() {
            let (address, receiver) = Address::channel();
            addresses.insert(actor.clone(), address);
            receivers.insert(actor.clone(), receiver);
        }
        let mut ready = Vec::with_capacity(behaviors.len());
        for (actor, (behavior, material)) in behaviors {
            let inbox = receivers.remove(&actor).expect("a receiver per actor");
            let shape = shapes.actors[&actor].clone();
            let mailbox_capacity = mailbox_capacity(&shape);
            let share = bind(shape, &addresses, &bound);
            let unstood = |reason: String, detail| Unassembled {
                actor: actor.clone(),
                reason,
                detail,
            };
            let unavailable = Some(crate::activation_detail::activation::JOURNAL_UNAVAILABLE);
            let (behavior, column) = match material {
                None => (behavior, None),
                Some(mut material) => {
                    let emitted = std::mem::take(&mut material.emitted);
                    match restoration
                        .as_deref()
                        .expect("claimed material comes from the restoration")
                        .column(
                            material,
                            self.stream,
                            &self.execution,
                            &actor,
                            &share,
                            container,
                            key,
                            minted,
                            behavior,
                            &addresses,
                            &bound,
                        ) {
                        Ok((behavior, column)) => (behavior, Some((column, emitted))),
                        Err(reason) => {
                            failures.push(unstood(reason, None));
                            continue;
                        }
                    }
                }
            };
            let journal = match self.hand(&actor) {
                Ok(journal) => journal,
                Err(reason) => {
                    failures.push(unstood(reason, unavailable));
                    continue;
                }
            };
            let life = match column {
                Some((mut column, emitted)) => match self.replay_for(&actor).await {
                    Ok((issuer, settled)) => {
                        column.settled = settled;
                        CellLife::Restored(issuer.replaying(&emitted), column)
                    }
                    Err(reason) => {
                        failures.push(unstood(reason, None));
                        continue;
                    }
                },
                None => {
                    let previous = self.retiring.remove(&actor);
                    let held = match previous {
                        Some(_) => None,
                        None => match self.issuer_for(&actor).await {
                            Ok((issuer, _)) => Some(issuer),
                            Err(reason) => {
                                failures.push(unstood(reason, unavailable));
                                continue;
                            }
                        },
                    };
                    CellLife::New { previous, held }
                }
            };
            ready.push((
                actor,
                behavior,
                share,
                mailbox_capacity,
                journal,
                inbox,
                life,
            ));
        }
        for (actor, behavior, share, mailbox_capacity, journal, inbox, life) in ready {
            let address = addresses.get(&actor).expect("an address per actor").clone();
            let door = Arc::new(Door::default());
            door.set(self.state.pause.is_some());
            let actor_type = behavior.actor_type;
            let cells = (behavior.actor_type == ActorType::Replicator)
                .then(|| CellHand(self.commands.clone()));
            let task = match life {
                CellLife::New { previous, held } => {
                    let lost = self.reread(&actor);
                    let stream = self.stream;
                    let approvals = self.approvals.clone();
                    let time = self.time.clone();
                    let pause = self.state.pause;
                    let cell_actor = actor.clone();
                    let cell_address = address.clone();
                    let cell_door = door.clone();
                    let cell_minted = minted.clone();
                    let life = super::actor::begin(
                        inbox,
                        move |issuer| {
                            super::actor::Task::new(Spawn {
                                cache: None,
                                address: cell_address,
                                actor: cell_actor,
                                stream,
                                issuer,
                                behavior,
                                share,
                                journal,
                                approvals,
                                time,
                                door: cell_door,
                                pause,
                                mailbox_capacity,
                                cells,
                                cell_minted: Some(cell_minted),
                            })
                            .activated()
                        },
                        async move {
                            match held {
                                Some(issuer) => Ok(issuer),
                                None => lost.await.map(|(issuer, _)| issuer),
                            }
                        },
                        previous,
                    );
                    self.launch(&actor, &address, life)
                }
                CellLife::Restored(issuer, column) => {
                    let spawn = Spawn {
                        cache: None,
                        address: address.clone(),
                        actor: actor.clone(),
                        stream: self.stream,
                        issuer,
                        behavior,
                        share,
                        journal,
                        approvals: self.approvals.clone(),
                        time: self.time.clone(),
                        door: door.clone(),
                        pause: self.state.pause,
                        mailbox_capacity,
                        cells,
                        cell_minted: Some(minted.clone()),
                    };
                    self.launch(&actor, &address, async move {
                        Ok(super::actor::restore_then_live(spawn, column, inbox).await)
                    })
                }
            };
            self.members.insert(
                actor,
                Member {
                    address,
                    door,
                    task,
                    actor_type,
                },
            );
        }
        self.cells.entry(container.clone()).or_default().insert(
            key.clone(),
            Cell {
                owner,
                minted: minted.clone(),
                actors: shapes.actors.keys().cloned().collect(),
                credits: cell_credits,
                forwards: shapes.forwards,
            },
        );
        Ok(failures)
    }

    fn addresses(&self) -> BTreeMap<NamedActorId, Address> {
        self.members
            .iter()
            .map(|(actor, member)| (actor.clone(), member.address.clone()))
            .collect()
    }

    fn assemble<'a>(
        &self,
        standing: &Standing,
        shapes: &BTreeMap<NamedActorId, ShareShape>,
        actors: impl Iterator<Item = &'a NamedActorId>,
        stood: &mut Stood,
    ) -> BTreeMap<NamedActorId, super::turn::Behavior> {
        let (graph, standing_table, registry) = standing.declarations.parts();
        let mut assembly = Assembly::new(
            self.stream,
            registry,
            standing.grants.clone(),
            standing_table.clone(),
            standing.revision.get(),
        )
        .executing(self.execution.clone());
        let mut behaviors = BTreeMap::new();
        for actor in actors {
            let Some(declaration) = graph.actors().get(actor) else {
                continue;
            };
            let actor_type = *declaration.domain().actor_type();
            if actor_type == ActorType::PipelineActor {
                continue;
            }
            if !assembly.boarded(actor_type) {
                if shapes
                    .get(actor)
                    .is_some_and(|shape| !shape.inlets.is_empty())
                {
                    stood.unboarded.push((actor.clone(), actor_type));
                }
                continue;
            }
            match assembly.behavior(actor, declaration) {
                Ok(behavior) => {
                    behaviors.insert(actor.clone(), behavior);
                }
                Err(failure) => stood.failures.push(failure),
            }
        }
        behaviors
    }

    async fn restore(&mut self, restoration: Restoration) -> Result<Stood, String> {
        let Restoration {
            columns,
            history,
            admitted,
            mut outcomes,
            mut cell_facts,
            refused,
            mut emitted,
        } = restoration;
        let latest = history.last().ok_or("restoration names no revision")?;
        let revisions: Vec<(RevisionEpochId, BTreeMap<NamedActorId, ShareShape>)> = history
            .iter()
            .map(|standing| (standing.revision, super::share::shapes(standing)))
            .collect();
        let shapes = revisions.last().expect("the latest revision").1.clone();
        let mut stood = Stood::default();
        let actors: Vec<NamedActorId> = shapes
            .keys()
            .filter(|actor| match refused.get(*actor) {
                Some(reason) => {
                    stood.failures.push(Unassembled {
                        actor: (*actor).clone(),
                        reason: reason.clone(),
                        detail: None,
                    });
                    false
                }
                None => true,
            })
            .cloned()
            .collect();
        self.credits = credits(&shapes, &BTreeMap::new(), &BTreeMap::new());
        let behaviors = self.assemble(latest, &shapes, actors.iter(), &mut stood);
        let mut receivers = BTreeMap::new();
        let mut addresses = self.addresses();
        for actor in behaviors.keys() {
            let (address, receiver) = Address::channel();
            addresses.insert(actor.clone(), address);
            receivers.insert(actor.clone(), receiver);
        }
        let execution = self.execution.clone();
        let mut lives = Lives {
            stream: self.stream,
            history: &history,
            revisions: &revisions,
            execution: &execution,
            assemblies: BTreeMap::new(),
        };
        let mut ready = Vec::with_capacity(behaviors.len());
        for (actor, behavior) in behaviors {
            let inbox = receivers.remove(&actor).expect("a receiver per actor");
            let shape = shapes
                .get(&actor)
                .expect("every standing actor has a shape")
                .clone();
            let mailbox_capacity = mailbox_capacity(&shape);
            let share = bind(shape, &addresses, &self.credits);
            let rows = columns.get(&actor).map_or(&[][..], Vec::as_slice);
            let admissions = admitted.get(&actor).map_or(&[][..], Vec::as_slice);
            let column = match lives.column(
                &actor,
                behavior,
                rows,
                outcomes.remove(&actor).unwrap_or_default(),
                admissions,
                &addresses,
                &self.credits,
            ) {
                Ok(column) => column,
                Err(reason) => {
                    stood.failures.push(Unassembled {
                        actor,
                        reason,
                        detail: None,
                    });
                    continue;
                }
            };
            let door = Arc::new(Door::default());
            door.set(self.state.pause.is_some());
            let (behavior, mut column) = column;
            column.cell_facts = cell_facts.remove(&actor).unwrap_or_default();
            column.cell_declarations = history
                .iter()
                .filter_map(|standing| {
                    Some((
                        standing.revision,
                        standing.declarations.graph().actors().get(&actor)?.clone(),
                    ))
                })
                .collect();
            let cache_path =
                super::snapshot::path(self.journal.source_custody_root(), self.stream, &actor);
            lives.cache(&actor, cache_path.clone(), &mut column).await;
            let actor_type = behavior.actor_type;
            let cells = (behavior.actor_type == ActorType::Replicator)
                .then(|| CellHand(self.commands.clone()));
            let journal = match self.hand(&actor) {
                Ok(journal) => journal,
                Err(reason) => {
                    stood.failures.push(Unassembled {
                        actor,
                        reason,
                        detail: None,
                    });
                    continue;
                }
            };
            let issuer = match self.replay_for(&actor).await {
                Ok((issuer, settled)) => {
                    column.settled = settled;
                    issuer.replaying(&emitted.remove(&actor.as_actor_id()).unwrap_or_default())
                }
                Err(reason) => {
                    stood.failures.push(Unassembled {
                        actor,
                        reason,
                        detail: None,
                    });
                    continue;
                }
            };
            let spawn = Spawn {
                cache: Some(super::snapshot::Writer::new(cache_path)),
                address: addresses.get(&actor).expect("an address per actor").clone(),
                actor: actor.clone(),
                stream: self.stream,
                behavior,
                share,
                issuer,
                journal,
                approvals: self.approvals.clone(),
                time: self.time.clone(),
                door: door.clone(),
                pause: self.state.pause,
                mailbox_capacity,
                cells,
                cell_minted: None,
            };
            let address = addresses.get(&actor).expect("an address per actor").clone();
            ready.push((actor, spawn, column, inbox, address, door, actor_type));
        }
        for (actor, spawn, column, inbox, address, door, actor_type) in ready {
            let handle = self.launch(&actor, &address, async move {
                Ok(super::actor::restore_then_live(spawn, column, inbox).await)
            });
            stood.actors.push(actor.clone());
            self.members.insert(
                actor,
                Member {
                    address,
                    door,
                    task: handle,
                    actor_type,
                },
            );
        }
        drop(lives);
        self.shapes = shapes;
        self.remember_unstood(latest, &stood);
        self.standing = Some(latest.clone());
        self.restoration = Some(CellRestore {
            columns,
            admitted,
            outcomes,
            history,
            facts: cell_facts,
            refused,
            emitted,
        });
        Ok(stood)
    }

    fn rest(
        &mut self,
        actor: &NamedActorId,
        ended: Result<Result<super::actor::Ended, String>, tokio::task::JoinError>,
    ) {
        let (rest, fault) = match ended {
            Ok(Ok(ended)) => (
                ended.issuer.map_or(Rest::Lost, |issuer| {
                    Rest::Held(issuer, Ok(super::outlet::Settled::default()))
                }),
                ended.fault,
            ),
            Ok(Err(reason)) => (Rest::Lost, Some(reason)),
            Err(error) => (Rest::Lost, Some(error.to_string())),
        };
        self.resting.insert(actor.as_actor_id(), rest);
        if let Some(fault) = fault {
            eprintln!("circular-kernel: a retiring actor ended: {fault}");
        }
    }

    async fn issuer_for(&mut self, actor: &NamedActorId) -> Result<(Issuer, Settlement), String> {
        let id = actor.as_actor_id();
        match self.resting.remove(&id) {
            Some(Rest::Held(issuer, settled)) => Ok((issuer, settled)),
            Some(Rest::Lost) => {
                let read = self.reread(actor).await;
                if read.is_err() {
                    self.resting.insert(id, Rest::Lost);
                }
                read
            }
            None => Ok((Issuer::new(id), Ok(super::outlet::Settled::default()))),
        }
    }

    async fn replay_for(
        &mut self,
        actor: &NamedActorId,
    ) -> Result<(Issuer, super::outlet::Settled), String> {
        match self.issuer_for(actor).await? {
            (issuer, Ok(settled)) => Ok((issuer, settled)),
            (issuer, Err(reason)) => {
                self.resting
                    .insert(actor.as_actor_id(), Rest::Held(issuer, Err(reason.clone())));
                Err(reason)
            }
        }
    }

    fn reread(
        &self,
        actor: &NamedActorId,
    ) -> impl std::future::Future<Output = Result<(Issuer, Settlement), String>> + Send + 'static
    {
        let journal = self.journal.clone();
        let preparation = self.preparation.clone();
        let actor = actor.as_actor_id();
        async move { preparation.ask(move || reread(&journal, actor)).await }
    }

    fn hand(&self, actor: &NamedActorId) -> Result<crate::ColumnJournal, String> {
        self.journal.open_column(&actor.as_actor_id())
    }

    async fn spawn_all(
        &mut self,
        behaviors: BTreeMap<NamedActorId, super::turn::Behavior>,
        shapes: &BTreeMap<NamedActorId, ShareShape>,
        stood: &mut Stood,
    ) {
        let mut receivers = BTreeMap::new();
        let mut addresses = self.addresses();
        for actor in behaviors.keys() {
            let (address, receiver) = Address::channel();
            addresses.insert(actor.clone(), address);
            receivers.insert(actor.clone(), receiver);
        }
        for (actor, behavior) in behaviors {
            let shape = shapes
                .get(&actor)
                .expect("every standing actor has a shape")
                .clone();
            let mailbox_capacity = mailbox_capacity(&shape);
            let share = bind(shape, &addresses, &self.credits);
            let address = addresses.get(&actor).expect("an address per actor").clone();
            let inbox = receivers.remove(&actor).expect("a receiver per actor");
            let door = Arc::new(Door::default());
            door.set(self.state.pause.is_some());
            let actor_type = behavior.actor_type;
            let cells = (behavior.actor_type == ActorType::Replicator)
                .then(|| CellHand(self.commands.clone()));
            let unopened = |actor: NamedActorId, reason: String| Unassembled {
                actor,
                reason,
                detail: Some(crate::activation_detail::activation::JOURNAL_UNAVAILABLE),
            };
            let journal = match self.hand(&actor) {
                Ok(journal) => journal,
                Err(reason) => {
                    stood.failures.push(unopened(actor, reason));
                    continue;
                }
            };
            let previous = self.retiring.remove(&actor);
            let held = match if previous.is_none() {
                self.issuer_for(&actor)
                    .await
                    .map(|(issuer, _)| Some(issuer))
            } else {
                Ok(None)
            } {
                Ok(held) => held,
                Err(reason) => {
                    stood.failures.push(unopened(actor, reason));
                    continue;
                }
            };
            let lost = self.reread(&actor);
            let cache = super::snapshot::Writer::new(super::snapshot::path(
                self.journal.source_custody_root(),
                self.stream,
                &actor,
            ));
            let task_actor = actor.clone();
            let task_address = address.clone();
            let task_door = door.clone();
            let stream = self.stream;
            let approvals = self.approvals.clone();
            let time = self.time.clone();
            let pause = self.state.pause;
            let life = super::actor::begin(
                inbox,
                move |issuer| {
                    super::actor::Task::new(Spawn {
                        cache: Some(cache),
                        address: task_address,
                        actor: task_actor,
                        stream,
                        behavior,
                        share,
                        issuer,
                        journal,
                        approvals,
                        time,
                        door: task_door,
                        pause,
                        mailbox_capacity,
                        cells,
                        cell_minted: None,
                    })
                    .activated()
                },
                async move {
                    match held {
                        Some(issuer) => Ok(issuer),
                        None => lost.await.map(|(issuer, _)| issuer),
                    }
                },
                previous,
            );
            let task = self.launch(&actor, &address, life);
            stood.actors.push(actor.clone());
            self.members.insert(
                actor,
                Member {
                    address,
                    door,
                    task,
                    actor_type,
                },
            );
        }
    }
}

fn consumption_boundary(
    arrivals: &[super::inlet::Recorded],
) -> Result<circular_core::ArrivalIndex, String> {
    let mut paused = None;
    for arrival in arrivals {
        match super::turn::lifecycle(&arrival.inlet, arrival.event.payload().value())? {
            Some(super::actor::Life::Pause { consumed, .. }) => paused = Some(consumed),
            Some(super::actor::Life::Resume) => paused = None,
            _ => {}
        }
    }
    if let Some(consumed) = paused {
        return Ok(consumed);
    }
    match arrivals.last() {
        Some(last) => Ok(
            super::turn::lifecycle(&last.inlet, last.event.payload().value())?
                .and_then(|life| life.stop_coordinate())
                .unwrap_or_else(|| last.index.next().unwrap_or(last.index)),
        ),
        None => Ok(circular_core::ArrivalIndex::FIRST),
    }
}

struct Lives<'a> {
    stream: StreamId,
    history: &'a [Standing],
    revisions: &'a [(RevisionEpochId, BTreeMap<NamedActorId, ShareShape>)],
    execution: &'a Arc<crate::execution_profile::ProductExecutionProfile>,
    assemblies: BTreeMap<RevisionEpochId, Assembly>,
}

impl Lives<'_> {
    /// Restoration assembly alone reads the disposable file. The journal still
    /// owns consumption and lifetime boundaries; a cache can only shorten replay.
    async fn cache(&mut self, actor: &NamedActorId, path: std::path::PathBuf, column: &mut Column) {
        let Some(snapshot) = super::snapshot::read(path, actor).await else {
            return;
        };
        let previous = column
            .arrivals
            .iter()
            .find(|arrival| arrival.index.next() == Some(snapshot.index));
        let revision = column
            .arrivals
            .iter()
            .take_while(|arrival| arrival.index < snapshot.index)
            .filter(|arrival| {
                super::turn::lifecycle(&arrival.inlet, arrival.event.payload().value())
                    .is_ok_and(|life| life.is_some_and(|life| life.declares()))
            })
            .last()
            .map(|arrival| arrival.at.revision());
        let valid = snapshot.stream == self.stream
            && snapshot.previous.producer() == &actor.as_actor_id()
            && snapshot.index <= column.consumed
            && previous.is_some_and(|arrival| arrival.at == snapshot.previous)
            && revision == Some(snapshot.revision)
            && !column
                .skipped
                .iter()
                .any(|range| range.contains(&snapshot.index));
        if !valid {
            super::snapshot::miss(actor, "column_mismatch");
            return;
        }
        if column.starts.len() > 1
            || column
                .prepared
                .values()
                .any(|incarnation| incarnation.replace)
            || column.pending.as_ref().is_some_and(|(_, incarnation)| {
                incarnation
                    .as_ref()
                    .is_some_and(|incarnation| incarnation.replace)
            })
        {
            super::snapshot::miss(actor, "lifetime_history_changed");
            return;
        }
        for record in &column.cell_facts {
            match crate::incarnation_transition::standing_incarnation(record) {
                Ok(Some((_, generation))) if generation != snapshot.generation => {
                    super::snapshot::miss(actor, "incarnation_history_changed");
                    return;
                }
                Err(_) => {
                    super::snapshot::miss(actor, "incarnation_history_unreadable");
                    return;
                }
                _ => {}
            }
        }
        if column.cell_facts.iter().any(|record| {
            matches!(record.header().key(), circular_store::ClassKey::Observation(
                circular_store::ObservationKey::StreamItem(_, _, item)
            ) if item.kind() == &circular_core::BuiltinObservationName::InstanceTransition)
        }) {
            super::snapshot::miss(actor, "instance_history");
            return;
        }
        let wired = |shape: &ShareShape| -> BTreeSet<_> {
            shape
                .outlets
                .iter()
                .filter(|(_, wires)| !wires.is_empty())
                .map(|(port, _)| port.clone())
                .collect()
        };
        let latest = self
            .revisions
            .last()
            .and_then(|(_, shapes)| shapes.get(actor));
        if latest.is_some_and(|shape| shape.prototype.is_some() || shape.cell) {
            super::snapshot::miss(actor, "cell_owner");
            return;
        }
        if latest.is_some_and(|latest| {
            self.revisions
                .iter()
                .filter(|(revision, _)| *revision <= snapshot.revision)
                .filter_map(|(_, shapes)| shapes.get(actor))
                .any(|before| wired(before) != wired(latest))
        }) {
            super::snapshot::miss(actor, "outlet_history_changed");
            return;
        }
        let Ok(mut behavior) = self.behavior(actor, snapshot.revision) else {
            super::snapshot::miss(actor, "declaration_unavailable");
            return;
        };
        if behavior.actor_type == ActorType::Replicator {
            super::snapshot::miss(actor, "cell_owner");
            return;
        }
        behavior.incarnation =
            crate::declarations::fresh_incarnation(self.stream, actor, snapshot.generation);
        column.snapshot = Some((snapshot, behavior));
    }

    fn column(
        &mut self,
        actor: &NamedActorId,
        latest: super::turn::Behavior,
        rows: &[crate::recorded::RecordedArrival],
        outcomes: Vec<super::column::Outcome>,
        admissions: &[super::column::Admitted],
        addresses: &BTreeMap<NamedActorId, Address>,
        credits: &BTreeMap<EdgeId, Credit>,
    ) -> Result<(super::turn::Behavior, Column), String> {
        let mut arrivals = rows
            .iter()
            .map(|row| super::column::recorded(self.stream, row))
            .chain(
                outcomes
                    .into_iter()
                    .map(|outcome| super::column::outcome(self.stream, actor.clone(), outcome)),
            )
            .collect::<Result<Vec<_>, _>>()
            .map_err(|unreadable| unreadable.0)?;
        arrivals.sort_by_key(|arrival| arrival.index);
        if arrivals
            .first()
            .is_some_and(|first| !is_lifecycle(&first.inlet))
        {
            return Err("the recorded column does not begin with this actor's activation".into());
        }
        let (latest_revision, _) = self.revisions.last().expect("the latest revision");
        let latest_revision = *latest_revision;
        let mut initial = None;
        let mut prepared = BTreeMap::new();
        let mut last: Option<RevisionEpochId> = None;
        let mut life = None;
        let mut stopped: Option<(circular_core::ArrivalIndex, circular_core::ArrivalIndex)> = None;
        let mut skipped = Vec::new();
        let mut starts = std::collections::BTreeSet::new();
        for arrival in arrivals
            .iter()
            .filter(|arrival| is_lifecycle(&arrival.inlet))
        {
            match super::turn::lifecycle(&arrival.inlet, arrival.event.payload().value())? {
                Some(super::actor::Life::Stop(consumed)) => {
                    stopped = Some((consumed, arrival.index));
                    continue;
                }
                Some(super::actor::Life::Activate | super::actor::Life::Become(_)) => {}
                _ => continue,
            }
            let revision = arrival.at.revision();
            let behavior = self.behavior(actor, revision)?;
            if last.is_none_or(|from| self.leaves(actor, from, revision)) {
                life = Some(revision);
                starts.insert(arrival.index);
                if let Some((consumed, at)) = stopped {
                    skipped.push(consumed..at);
                }
            }
            stopped = None;
            match last {
                None => initial = Some(behavior),
                Some(from)
                    if self.crosses(actor, from, revision)
                        || self.declaration(actor, from) != self.declaration(actor, revision) =>
                {
                    prepared.insert(
                        arrival.index,
                        Box::new(Incarnation {
                            replace: self.crosses(actor, from, revision),
                            behavior,
                        }),
                    );
                }
                Some(_) => {}
            }
            last = Some(revision);
        }
        let (behavior, pending, activate) = match (initial, last) {
            (Some(initial), Some(from)) => {
                let replace = self.crosses(actor, from, latest_revision);
                let declared =
                    self.declaration(actor, from) != self.declaration(actor, latest_revision);
                let reshaped = !self.shape(actor, from).is_some_and(|before| {
                    self.shape(actor, latest_revision)
                        .is_some_and(|after| before.same_as(after))
                });
                let entry_revision = from < latest_revision
                    && self
                        .shape(actor, latest_revision)
                        .is_some_and(ShareShape::is_entry);
                let pending = (replace || declared || reshaped || entry_revision).then(|| {
                    (
                        latest_revision,
                        (replace || declared).then(|| {
                            Box::new(Incarnation {
                                behavior: latest,
                                replace,
                            })
                        }),
                    )
                });
                (initial, pending, false)
            }
            _ => (latest, None, true),
        };
        let (inlets, outlets) = self
            .revisions
            .iter()
            .filter(|(revision, _)| *revision != latest_revision)
            .filter_map(|(revision, shapes)| {
                let shape = shapes.get(actor)?.clone();
                let share = bind(shape, addresses, credits);
                Some(((*revision, share.inlets), (*revision, share.outlets)))
            })
            .unzip();
        let consumed = consumption_boundary(&arrivals)?;
        let held = admissions
            .iter()
            .filter(|admitted| life.is_none_or(|life| admitted.sender.revision() >= life))
            .map(|admitted| {
                super::column::admitted(self.stream, admitted, credits)
                    .map(|delivery| (delivery, admitted.observed_at))
                    .map_err(|unreadable| unreadable.0)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((
            behavior,
            Column {
                snapshot: None,
                arrivals,
                consumed,
                prepared,
                pending,
                activate,
                inlets,
                outlets,
                held,
                skipped,
                starts,
                flags: self
                    .revisions
                    .iter()
                    .filter_map(|(revision, shapes)| {
                        Some((*revision, shapes.get(actor)?.declaration.flags()))
                    })
                    .collect(),
                cell_facts: Vec::new(),
                cell_declarations: Vec::new(),
                settled: super::outlet::Settled::default(),
                prototypes: self
                    .revisions
                    .iter()
                    .filter_map(|(revision, shapes)| {
                        Some((*revision, shapes.get(actor)?.prototype.clone()?))
                    })
                    .collect(),
            },
        ))
    }

    fn leaves(&self, actor: &NamedActorId, from: RevisionEpochId, to: RevisionEpochId) -> bool {
        self.revisions
            .iter()
            .filter(|(revision, _)| *revision > from && *revision <= to)
            .any(|(_, shapes)| !shapes.contains_key(actor))
    }

    fn declaration(
        &self,
        actor: &NamedActorId,
        revision: RevisionEpochId,
    ) -> Option<&circular_plan::ActorDecl> {
        self.shape(actor, revision).map(|shape| &shape.declaration)
    }

    fn shape(&self, actor: &NamedActorId, revision: RevisionEpochId) -> Option<&ShareShape> {
        self.revisions
            .iter()
            .find(|(recorded, _)| *recorded == revision)
            .and_then(|(_, shapes)| shapes.get(actor))
    }

    fn crosses(&self, actor: &NamedActorId, from: RevisionEpochId, to: RevisionEpochId) -> bool {
        let kind = |revision| {
            self.declaration(actor, revision).map(|declaration| {
                (
                    *declaration.domain().actor_type(),
                    declaration.authored_generation(),
                )
            })
        };
        kind(from) != kind(to)
            || self
                .revisions
                .iter()
                .filter(|(revision, _)| *revision > from && *revision <= to)
                .any(|(_, shapes)| !shapes.contains_key(actor))
    }

    fn behavior(
        &mut self,
        actor: &NamedActorId,
        revision: RevisionEpochId,
    ) -> Result<super::turn::Behavior, String> {
        let standing = self
            .history
            .iter()
            .find(|standing| standing.revision == revision)
            .ok_or_else(|| {
                format!(
                    "a lifecycle arrival names revision {} which has no recorded declaration",
                    revision.get()
                )
            })?;
        let (graph, table, registry) = standing.declarations.parts();
        let declaration = graph
            .actors()
            .get(actor)
            .ok_or_else(|| format!("revision {} does not declare this actor", revision.get()))?;
        let stream = self.stream;
        let execution = self.execution;
        let assembly = self.assemblies.entry(revision).or_insert_with(|| {
            Assembly::new(
                stream,
                registry,
                standing.grants.clone(),
                table.clone(),
                revision.get(),
            )
            .executing(execution.clone())
        });
        assembly
            .behavior(actor, declaration)
            .map_err(|failure| failure.reason)
    }
}

fn credits(
    shapes: &BTreeMap<NamedActorId, ShareShape>,
    before: &BTreeMap<NamedActorId, ShareShape>,
    previous: &BTreeMap<EdgeId, Credit>,
) -> BTreeMap<EdgeId, Credit> {
    let mut credits = BTreeMap::new();
    for (actor, shape) in shapes {
        let mailbox = mailbox_capacity(shape);
        for (edge, inlet) in &shape.inlets {
            if !holds_back(&inlet.policy) {
                continue;
            }
            let capacity = capacity_of(&inlet.policy, mailbox).max(1);
            let unchanged = before
                .get(actor)
                .and_then(|old| old.inlets.get(edge))
                .is_some_and(|old| {
                    old.policy == inlet.policy
                        && capacity_of(&old.policy, mailbox_capacity(&before[actor])).max(1)
                            == capacity
                });
            let credit = previous
                .get(edge)
                .filter(|_| unchanged)
                .cloned()
                .unwrap_or_else(|| Arc::new(Semaphore::new(capacity)));
            credits.insert(edge.clone(), credit);
        }
    }
    credits
}

fn bind(
    shape: ShareShape,
    addresses: &BTreeMap<NamedActorId, Address>,
    credits: &BTreeMap<EdgeId, Credit>,
) -> Share {
    let inlets = shape
        .inlets
        .into_iter()
        .map(|(edge, inlet)| (edge, InletWire { shape: inlet }))
        .collect();
    let outlets = shape
        .outlets
        .into_iter()
        .map(|(port, wires)| {
            let wires = wires
                .into_iter()
                .map(|wire| {
                    let address = addresses
                        .get(wire.to.actor())
                        .cloned()
                        .unwrap_or_else(|| Address::channel().0);
                    OutWire {
                        credit: credits.get(&wire.edge).cloned(),
                        edge: wire.edge,
                        to: wire.to,
                        address,
                    }
                })
                .collect();
            (port, wires)
        })
        .collect();
    Share {
        revision: shape.revision,
        declaration: shape.declaration,
        inlets,
        outlets,
        prototype: shape.prototype,
        cell: shape.cell,
    }
}

fn reread(
    journal: &crate::ProductDurableArrivalJournal,
    actor: ActorId,
) -> Result<(Issuer, Settlement), String> {
    let mut issuer = Issuer::new(actor.clone());
    let mut settled = BTreeMap::new();
    let mut unread = BTreeMap::new();
    let mut lives = super::record::LifeStarts::default();
    for row in journal.read_view().all_rows()? {
        let row = row?;
        let record = row.record();
        let own = super::record::owner(&record) == &actor;
        if own {
            issuer.retain(&record)?;
            lives.retain(&record);
        }
        if let Err(error) = super::outlet::settle(&mut settled, &record)
            && own
        {
            latest_unread(&mut unread, &actor, record.header().at(), &error);
        }
    }
    let settlement = match unread.remove(&actor) {
        Some((at, reason)) if lives.holds(&actor, &at) => Err(reason),
        _ => Ok(settled.remove(&actor).unwrap_or_default()),
    };
    Ok((issuer, settlement))
}

fn mailbox_capacity(shape: &ShareShape) -> usize {
    let default = circular_plan::WirePolicy::DEFAULT_EDGE
        .capacity
        .map_or(1, |capacity| capacity.get());
    shape
        .inlets
        .values()
        .map(|inlet| capacity_of(&inlet.policy, default))
        .sum::<usize>()
        .max(1)
}

/// Model witness assembly uses the same column and cache admission as boot.
#[cfg(test)]
pub(super) async fn snapshot_test_column(
    stream: StreamId,
    actor: &NamedActorId,
    history: &[Standing],
    rows: &[crate::recorded::RecordedArrival],
    records: &[circular_store::Record<circular_store::ProductStore>],
    path: std::path::PathBuf,
    share: &Share,
) -> (super::turn::Behavior, Column) {
    let revisions: Vec<_> = history
        .iter()
        .map(|s| (s.revision, super::share::shapes(s)))
        .collect();
    let execution = Arc::new(crate::execution_profile::ProductExecutionProfile::default());
    let mut lives = Lives {
        stream,
        history,
        revisions: &revisions,
        execution: &execution,
        assemblies: BTreeMap::new(),
    };
    let latest = lives
        .behavior(actor, history.last().unwrap().revision)
        .unwrap();
    let addresses = share
        .outlets
        .values()
        .flatten()
        .map(|wire| (wire.to.actor().clone(), wire.address.clone()))
        .collect();
    let (behavior, mut column) = lives
        .column(
            actor,
            latest,
            rows,
            Vec::new(),
            &[],
            &addresses,
            &BTreeMap::new(),
        )
        .unwrap();
    column.cell_facts = cell_facts(records, &super::record::LifeStarts::of(records))
        .remove(actor)
        .unwrap_or_default();
    lives.cache(actor, path, &mut column).await;
    (behavior, column)
}

