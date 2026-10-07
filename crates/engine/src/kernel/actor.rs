
use super::inlet::{Inlets, Received, Recorded};
use super::outlet::{Gone, Outlets};
use super::record::Issuer;
use super::share::Share;
use super::turn::{Behavior, Hooked, InputOrigin, preprocess};
use super::{Control, Delivery, Message, ProductEvent};
use crate::actor_time::TimeActor;
use crate::arrival_commit::EventArrivalCommit;
use crate::inlet_preprocess::{Element, InletVerdict};
use crate::product_arrival_journal::ArrivalAnswer;
use circular_actors::ProductPayload;
use circular_core::{RecordedInstant, RevisionEpochId, Stamp, Tick};
use circular_plan::{ActorId, NamedActorId, PortId};
use circular_runtime::{
    ActorEffect, ActorLifecycle, ArrivalOrigin, DeadLetterOrigin, DeadLetterReason,
    DeadLetterRecord, EnvelopeResult, ExternalOrigin,
};
use circular_store::StreamId;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Notify, mpsc, oneshot};

#[derive(Default)]
pub(crate) struct Door {
    paused: AtomicBool,
    opened: Notify,
}

impl Door {
    pub(crate) fn paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }

    pub(crate) fn set(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
        if !paused {
            self.opened.notify_waiters();
        }
    }
}

#[derive(Debug)]
pub(crate) enum Refusal {
    NotAccepting,
    Rejected(String),
}

pub(crate) struct Injection {
    pub(crate) inlet: PortId,
    pub(crate) payload: ProductPayload,
    pub(crate) origin: ExternalOrigin,
    pub(crate) reply: oneshot::Sender<Result<bool, Refusal>>,
    pub(crate) patient: bool,
}

pub(crate) struct Incarnation {
    pub(crate) behavior: Behavior,
    pub(crate) replace: bool,
}

fn lifecycle_inlet() -> PortId {
    PortId::try_derived(circular_actors::LIFECYCLE_PORT_NAME.to_owned())
        .expect("reserved lifecycle inlet is canonical")
}

pub(crate) fn is_lifecycle(inlet: &PortId) -> bool {
    inlet.as_str() == circular_actors::LIFECYCLE_PORT_NAME
}

pub(crate) struct Spawn {
    pub(crate) cache: Option<super::snapshot::Writer>,
    pub(crate) actor: NamedActorId,
    pub(crate) address: super::Address,
    pub(crate) stream: StreamId,
    pub(crate) behavior: Behavior,
    pub(crate) share: Share,
    pub(crate) issuer: Issuer,
    pub(crate) journal: crate::ColumnJournal,
    pub(crate) approvals: crate::runtime_approval::ApprovalReturns,
    pub(crate) time: Arc<TimeActor>,
    pub(crate) door: Arc<Door>,
    pub(crate) pause: Option<bool>,
    pub(crate) mailbox_capacity: usize,
    pub(crate) cells: Option<super::system::CellHand>,
    pub(crate) cell_minted: Option<Stamp<ActorId>>,
}

struct CellPlane {
    restore_after: Option<RevisionEpochId>,
    hand: super::system::CellHand,
    prototype: Arc<super::share::CellPrototype>,
    staged: std::collections::BTreeMap<RevisionEpochId, Arc<super::share::CellPrototype>>,
    prototypes:
        std::collections::BTreeMap<circular_runtime::InstanceKey, Arc<super::share::CellPrototype>>,
    lifecycle: crate::instance_journal::InstanceLifecycle,
    recorded: std::collections::VecDeque<circular_store::Record<circular_store::ProductStore>>,
    declarations: std::collections::BTreeMap<RevisionEpochId, circular_plan::ActorDecl>,
    wires: std::collections::BTreeMap<
        circular_runtime::InstanceKey,
        std::collections::BTreeMap<circular_plan::EdgeId, Vec<super::share::OutWire>>,
    >,
    pending: std::collections::BTreeMap<
        circular_runtime::InstanceKey,
        std::collections::VecDeque<(
            circular_plan::EdgeId,
            PortId,
            ProductEvent,
            circular_core::EncodedPayload,
            circular_runtime::EnvelopeResult,
        )>,
    >,
}

pub(crate) enum Life {
    Activate,
    Become(Option<Box<Incarnation>>),
    Stop(circular_core::ArrivalIndex),
    Pause {
        force: bool,
        consumed: circular_core::ArrivalIndex,
    },
    Resume,
}

#[derive(Default)]
struct Resumed {
    held_from: Option<circular_core::ArrivalIndex>,
    spans: Vec<(
        circular_core::ArrivalIndex,
        circular_core::ArrivalIndex,
        RecordedInstant,
    )>,
}

impl Resumed {
    fn fold(&mut self, index: circular_core::ArrivalIndex, life: &Life, at: RecordedInstant) {
        match life {
            Life::Pause { consumed, .. } => {
                self.held_from.get_or_insert(*consumed);
            }
            Life::Resume => {
                if let Some(from) = self.held_from.take() {
                    self.spans.push((from, index, at));
                }
            }
            _ => {}
        }
    }

    fn consumed(&mut self, index: circular_core::ArrivalIndex) {
        self.spans.retain(|(_, resume, _)| *resume > index);
    }

    fn floor(&mut self, index: circular_core::ArrivalIndex) -> Option<RecordedInstant> {
        self.consumed(index);
        self.spans
            .iter()
            .filter(|(from, _, _)| *from <= index)
            .map(|(_, _, at)| *at)
            .max()
    }
}

impl Life {
    pub(crate) fn declares(&self) -> bool {
        matches!(self, Self::Activate | Self::Become(_))
    }

    pub(crate) fn stop_coordinate(&self) -> Option<circular_core::ArrivalIndex> {
        match self {
            Self::Stop(consumed) => Some(*consumed),
            _ => None,
        }
    }
}

enum Pending {
    Lifecycle(Life),
    Outcome(Box<circular_runtime::EffectOutcome<circular_runtime::EffectId>>),
    Timer {
        timer: circular_runtime::EffectId,
        correlation: u64,
    },
    Wire(Delivery),
    Injection {
        inlet: PortId,
        payload: ProductPayload,
        origin: ExternalOrigin,
        reply: oneshot::Sender<Result<bool, Refusal>>,
    },
}

struct InFlight {
    pending: Pending,
    parents: Box<[Stamp<ActorId>]>,
    revision: RevisionEpochId,
    observed_at: RecordedInstant,
    answer: oneshot::Receiver<ArrivalAnswer>,
}

enum Next {
    Continue,
    Stop,
}

enum Port {
    Replay,
    Live(crate::ColumnJournal),
}

impl Port {
    const fn is_live(&self) -> bool {
        matches!(self, Self::Live(_))
    }

    fn replay(&mut self) -> Option<crate::ColumnJournal> {
        match std::mem::replace(self, Self::Replay) {
            Self::Live(journal) => Some(journal),
            Self::Replay => None,
        }
    }

    fn recorder_alive(&self) -> bool {
        matches!(self, Self::Live(journal) if journal.recorder_stop().is_none())
    }
}

pub(crate) struct Task {
    #[cfg(test)]
    replayed_turns: usize,
    cache: Option<super::snapshot::Writer>,
    last_turn: Option<(circular_core::ArrivalIndex, Stamp<ActorId>)>,
    approvals: crate::runtime_approval::ApprovalReturns,
    actor: NamedActorId,
    stream: StreamId,
    behavior: Behavior,
    revision: RevisionEpochId,
    turn_revision: RevisionEpochId,
    flags: circular_plan::ActorFlags,
    latest_flags: circular_plan::ActorFlags,
    staged_flags: std::collections::BTreeMap<RevisionEpochId, circular_plan::ActorFlags>,
    gate: circular_runtime::EmissionGate,
    staged: std::collections::BTreeMap<
        RevisionEpochId,
        std::collections::BTreeMap<PortId, Vec<super::share::OutWire>>,
    >,
    inlets: Inlets,
    outlets: Outlets,
    issuer: Issuer,
    port: Port,
    time: Arc<TimeActor>,
    door: Arc<Door>,
    /// System supplied this intent at spawn; the member records its own application.
    inherited_pause: Option<bool>,
    /// Fold of this member's Pause/Resume arrivals, including the force bit.
    applied_pause: Option<bool>,
    resumed: Resumed,
    injections: std::collections::VecDeque<Injection>,
    outcomes:
        std::collections::VecDeque<circular_runtime::EffectOutcome<circular_runtime::EffectId>>,
    firing: std::collections::VecDeque<(circular_runtime::EffectId, super::effect::Armed)>,
    lifecycle: std::collections::VecDeque<(RevisionEpochId, Life)>,
    prepared: std::collections::BTreeMap<circular_core::ArrivalIndex, Box<Incarnation>>,
    starts: std::collections::BTreeSet<circular_core::ArrivalIndex>,
    flying: Option<InFlight>,
    writes: std::collections::VecDeque<oneshot::Receiver<Result<(), String>>>,
    poisoned: bool,
    fault: Option<String>,
    health: crate::actor_records::ActorHealth,
    now: Tick,
    display: Option<DisplayWindow>,
    own: super::system::Entrance,
    source: Option<super::source::Worker>,
    restarting: Option<tokio::task::JoinHandle<()>>,
    faulted: bool,
    cells: Option<Box<CellPlane>>,
    cell_minted: Option<Stamp<ActorId>>,
    announce: bool,
    closing: Option<Closing>,
    stopped: Option<Recorded>,
    settled: super::outlet::Settled,
    sent: std::collections::BTreeMap<circular_plan::EdgeId, circular_core::Sequence>,
}

struct Closing {
    effects: Vec<circular_runtime::EffectId>,
    next: Box<super::turn::Behavior>,
    revision: RevisionEpochId,
}

struct DisplayWindow {
    coordinate: crate::display_writer::DisplayCoordinate,
    count: i64,
    last: Stamp<ActorId>,
    end: Tick,
}

const DISPLAY_BUCKET_MILLIS: u64 = 1_000;

pub(crate) struct Ended {
    pub(crate) fault: Option<String>,
    pub(crate) issuer: Option<Issuer>,
}

pub(crate) async fn begin(
    inbox: mpsc::UnboundedReceiver<Message>,
    build: impl FnOnce(Issuer) -> Task + Send + 'static,
    assembled: impl std::future::Future<Output = Result<Issuer, String>> + Send + 'static,
    previous: Option<tokio::task::JoinHandle<Result<Ended, String>>>,
) -> Result<Ended, String> {
    let (handed, failure) = match previous {
        Some(previous) => match previous.await {
            Ok(Ok(ended)) => {
                if let Some(fault) = ended.fault {
                    eprintln!("circular-kernel: the preceding actor ended: {fault}");
                }
                (ended.issuer, None)
            }
            Ok(Err(reason)) => (None, Some(reason)),
            Err(error) => (
                None,
                Some(format!("the preceding actor task failed: {error}")),
            ),
        },
        None => (None, None),
    };
    let issuer = match handed {
        Some(issuer) => issuer,
        None => assembled.await?,
    };
    let mut task = build(issuer);
    if let Some(reason) = failure {
        task.refuse_recovery(&reason).await;
        task.fault = Some(reason);
        return Ok(task.exit(inbox, Exit::Fault).await);
    }
    Ok(task.live(inbox).await)
}

pub(crate) struct Column {
    pub(crate) snapshot: Option<(super::snapshot::Snapshot, Behavior)>,
    pub(crate) arrivals: Vec<Recorded>,
    pub(crate) consumed: circular_core::ArrivalIndex,
    pub(crate) prepared: std::collections::BTreeMap<circular_core::ArrivalIndex, Box<Incarnation>>,
    pub(crate) pending: Option<(RevisionEpochId, Option<Box<Incarnation>>)>,
    pub(crate) activate: bool,
    pub(crate) inlets: Vec<(
        RevisionEpochId,
        std::collections::BTreeMap<circular_plan::EdgeId, super::share::InletWire>,
    )>,
    pub(crate) outlets: Vec<(
        RevisionEpochId,
        std::collections::BTreeMap<PortId, Vec<super::share::OutWire>>,
    )>,
    pub(crate) held: Vec<(Delivery, RecordedInstant)>,
    pub(crate) skipped: Vec<std::ops::Range<circular_core::ArrivalIndex>>,
    pub(crate) starts: std::collections::BTreeSet<circular_core::ArrivalIndex>,
    pub(crate) flags: Vec<(RevisionEpochId, circular_plan::ActorFlags)>,
    pub(crate) prototypes: Vec<(RevisionEpochId, Arc<super::share::CellPrototype>)>,
    pub(crate) cell_facts: Vec<circular_store::Record<circular_store::ProductStore>>,
    pub(crate) cell_declarations: Vec<(RevisionEpochId, circular_plan::ActorDecl)>,
    pub(crate) settled: super::outlet::Settled,
}

pub(crate) async fn restore_then_live(
    spawn: Spawn,
    column: Column,
    inbox: mpsc::UnboundedReceiver<Message>,
) -> Ended {
    match restore(spawn, column).await {
        Ok(task) => task.live(inbox).await,
        Err((mut task, reason)) => {
            task.refuse_recovery(&reason).await;
            task.fault = Some(reason);
            Ended {
                issuer: None,
                ..task.exit(inbox, Exit::Fault).await
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Exit {
    Stop,
    Retire,
    Fault,
}

async fn restore(spawn: Spawn, column: Column) -> Result<Task, (Box<Task>, String)> {
    let Column {
        snapshot,
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
        flags,
        prototypes,
        cell_facts,
        cell_declarations,
        settled,
    } = column;
    let latest_outlets = (spawn.share.revision, spawn.share.outlets.clone());
    let mut replay_outlets = Outlets::default();
    if spawn.share.cell {
        replay_outlets.cell_revision(spawn.share.revision);
    }
    let mut task = Task::new(spawn);
    task.outlets = replay_outlets;
    let snapshot = snapshot.filter(|(snapshot, _)| {
        let covered = snapshot
            .sent
            .iter()
            .all(|(edge, sequence)| settled.covers(edge, *sequence));
        if !covered {
            super::snapshot::miss(&task.actor, "unsettled_emission");
        }
        covered
    });
    task.settled = settled;
    task.staged.extend(outlets);
    task.staged.insert(latest_outlets.0, latest_outlets.1);
    for (revision, wires) in &inlets {
        task.inlets.install(*revision, wires);
    }
    let ticks_per_second = task.time.ticks_per_second();
    for (delivery, admitted_at) in held {
        let until = task
            .inlets
            .delay_of(&delivery, ticks_per_second)
            .filter(|ticks| *ticks > 0)
            .map(|ticks| Tick::new(admitted_at.millis().saturating_add(ticks)));
        task.inlets.rehold(delivery, until);
    }
    task.prepared = prepared;
    task.starts = starts;
    task.staged_flags.extend(flags);
    if let Some(plane) = task.cells.as_mut() {
        plane.staged.extend(prototypes);
        plane.declarations.extend(cell_declarations);
        for record in cell_facts {
            let relevant = crate::instance_journal::decode_transition(&record)
                .map(|transition| transition.is_some())
                .map_err(|error| error.to_string())
                .and_then(|instance| {
                    crate::incarnation_transition::standing_incarnation(&record)
                        .map(|incarnation| instance || incarnation.is_some())
                });
            match relevant {
                Ok(true) => plane.recorded.push_back(record),
                Ok(false) => {}
                Err(reason) => return Err((Box::new(task), reason)),
            }
        }
    }
    let mut cut = circular_core::ArrivalIndex::FIRST;
    if let Some((snapshot, mut behavior)) = snapshot {
        let restored = match snapshot.state {
            Some(state) => behavior.restore_state(state).is_ok(),
            None => behavior.state().is_ok_and(|state| state.is_none()),
        };
        if restored {
            cut = snapshot.index;
            task.behavior = behavior;
            task.turn_revision = snapshot.revision;
            for arrival in arrivals.iter().take_while(|arrival| arrival.index < cut) {
                if is_lifecycle(&arrival.inlet)
                    && super::turn::lifecycle(&arrival.inlet, arrival.event.payload().value())
                        .is_ok_and(|life| life.is_some_and(|life| life.declares()))
                    && !skipped.iter().any(|range| range.contains(&arrival.index))
                {
                    task.activate_outlets(arrival.at.revision());
                }
            }
            task.flags = task
                .staged_flags
                .range(..=snapshot.revision)
                .next_back()
                .map_or(task.flags, |(_, flags)| *flags);
            task.staged_flags
                .retain(|revision, _| *revision > snapshot.revision);
            task.prepared.retain(|index, _| *index >= cut);
            task.starts.retain(|index| *index >= cut);
            task.issuer
                .resume_emissions(snapshot.emitted, snapshot.emission_clock);
            task.sent = snapshot.sent.into_iter().collect();
            task.health.resumed();
            task.last_turn = Some((cut, snapshot.previous));
            eprintln!(
                "circular-kernel: actor={:?} checkpoint_cache_hit cut={}",
                task.actor,
                cut.get()
            );
        } else {
            super::snapshot::miss(&task.actor, "hook_state_invalid");
        }
    }
    let Some(journal) = task.port.replay() else {
        return Err((Box::new(task), "a restored actor has no journal".to_owned()));
    };
    let mut next = circular_core::ArrivalIndex::FIRST;
    let mut restored_pause = None;
    for arrival in &arrivals {
        if let Ok(Some(life)) =
            super::turn::lifecycle(&arrival.inlet, arrival.event.payload().value())
        {
            task.resumed.fold(arrival.index, &life, arrival.observed_at);
        }
    }
    for arrival in arrivals {
        task.issuer.observe(&arrival.at);
        let Some(following) = arrival.index.next() else {
            task.port = Port::Live(journal);
            return Err((Box::new(task), "arrival index exhausted".to_owned()));
        };
        next = following;
        let life = match super::turn::lifecycle(&arrival.inlet, arrival.event.payload().value()) {
            Ok(life) => life,
            Err(reason) => {
                task.port = Port::Live(journal);
                return Err((Box::new(task), reason));
            }
        };
        match life {
            Some(Life::Pause { force, consumed }) => {
                task.door.set(true);
                restored_pause = Some(force);
                if force && let Some(effects) = task.behavior.effects.as_mut() {
                    effects.replay_force_pause(consumed);
                }
                continue;
            }
            Some(Life::Resume) => {
                task.door.set(false);
                restored_pause = None;
                continue;
            }
            Some(Life::Stop(_)) => continue,
            _ => {}
        }
        if arrival.index < cut || skipped.iter().any(|range| range.contains(&arrival.index)) {
            continue;
        }
        if arrival.index < consumed {
            if let Next::Stop = task.turn(arrival).await {
                let reason = task
                    .fault
                    .take()
                    .unwrap_or_else(|| "replay stopped".to_owned());
                task.port = Port::Live(journal);
                return Err((Box::new(task), reason));
            }
        } else {
            if is_lifecycle(&arrival.inlet) {
                task.turn_revision = arrival.revision;
                if let Next::Stop = task.fold_cell_arrival(arrival.revision).await {
                    let reason = task
                        .fault
                        .take()
                        .unwrap_or_else(|| "cell arrival replay stopped".to_owned());
                    task.port = Port::Live(journal);
                    return Err((Box::new(task), reason));
                }
            }
            if let (Some(outcome), Some(effects)) =
                (&arrival.outcome, task.behavior.effects.as_mut())
            {
                effects.answered(outcome.correlation());
            }
            if let ArrivalOrigin::TimerFire { timer } = &arrival.origin {
                task.behavior.timers.recorded(timer);
            }
            task.inlets.recorded(arrival);
        }
    }
    task.resumed.consumed(consumed);
    task.settled = super::outlet::Settled::default();
    while let Some(record) = task
        .cells
        .as_mut()
        .and_then(|plane| plane.recorded.pop_front())
    {
        if let Err(reason) = task.fold_cell_record(&record) {
            task.port = Port::Live(journal);
            return Err((Box::new(task), reason));
        }
    }
    task.issuer.resume_at(next);
    task.port = Port::Live(journal);
    task.apply_pause(restored_pause);
    if activate {
        task.activate();
    }
    let intent = task.inherited_pause;
    if let Next::Stop = task.record_pause(intent).await {
        let reason = task
            .fault
            .take()
            .unwrap_or_else(|| "restored pause intent could not be recorded".into());
        return Err((Box::new(task), reason));
    }
    if let Some(effects) = task.behavior.effects.as_mut() {
        task.outcomes.extend(effects.resubmit());
    }
    if let Some(now) = task.time.query_tick().await {
        task.now = now;
    }
    task.harness_health(true);
    let registered = task.behavior.effects.as_ref().map(|effects| {
        effects.requests().try_for_each(|request| {
            task.approvals
                .register(request.clone(), task.own.address().clone())
        })
    });
    if let Some(Err(reason)) = registered {
        return Err((Box::new(task), reason));
    }

    if let Some((revision, incarnation)) = pending {
        if let Some(plane) = task.cells.as_mut() {
            plane.restore_after = Some(revision);
        }
        task.lifecycle
            .push_back((revision, Life::Become(incarnation)));
    }
    Ok(task)
}

impl Task {
    pub(crate) fn new(spawn: Spawn) -> Self {
        let mut inlets = Inlets::new(spawn.mailbox_capacity);
        inlets.install(spawn.share.revision, &spawn.share.inlets);
        let mut outlets = Outlets::new(spawn.share.revision, spawn.share.outlets);
        if spawn.share.cell {
            inlets.cell_revision(spawn.share.revision);
            outlets.cell_revision(spawn.share.revision);
        }
        let mut task = Self {
            #[cfg(test)]
            replayed_turns: 0,
            cache: spawn.cache,
            last_turn: None,
            approvals: spawn.approvals,
            actor: spawn.actor,
            stream: spawn.stream,
            behavior: spawn.behavior,
            revision: spawn.share.revision,
            turn_revision: spawn.share.revision,
            flags: spawn.share.declaration.flags(),
            latest_flags: spawn.share.declaration.flags(),
            staged_flags: std::collections::BTreeMap::new(),
            gate: circular_runtime::EmissionGate::All,
            staged: std::collections::BTreeMap::new(),
            inlets,
            outlets,
            issuer: spawn.issuer,
            port: Port::Live(spawn.journal),
            time: spawn.time,
            door: spawn.door.clone(),
            inherited_pause: spawn.pause,
            applied_pause: None,
            resumed: Resumed::default(),
            injections: std::collections::VecDeque::new(),
            outcomes: std::collections::VecDeque::new(),
            firing: std::collections::VecDeque::new(),
            lifecycle: std::collections::VecDeque::new(),
            prepared: std::collections::BTreeMap::new(),
            starts: std::collections::BTreeSet::new(),
            flying: None,
            writes: std::collections::VecDeque::new(),
            poisoned: false,
            fault: None,
            health: crate::actor_records::ActorHealth::default(),
            now: Tick::ZERO,
            display: None,
            own: super::system::Entrance::of(spawn.address),
            source: None,
            restarting: None,
            faulted: false,
            cells: spawn.cells.map(|hand| {
                Box::new(CellPlane {
                    restore_after: None,
                    hand,
                    prototype: spawn
                        .share
                        .prototype
                        .expect("a container holds its prototype"),
                    staged: std::collections::BTreeMap::new(),
                    prototypes: std::collections::BTreeMap::new(),
                    lifecycle: Default::default(),
                    recorded: Default::default(),
                    declarations: std::collections::BTreeMap::from([(
                        spawn.share.revision,
                        spawn.share.declaration.clone(),
                    )]),
                    wires: std::collections::BTreeMap::new(),
                    pending: std::collections::BTreeMap::new(),
                })
            }),
            cell_minted: spawn.cell_minted,
            announce: false,
            closing: None,
            stopped: None,
            settled: super::outlet::Settled::default(),
            sent: std::collections::BTreeMap::new(),
        };
        task.bind_source();
        task
    }

    fn journal(&self) -> Option<crate::ColumnJournal> {
        match &self.port {
            Port::Live(journal) => Some(journal.clone()),
            Port::Replay => None,
        }
    }

    fn activate(&mut self) {
        self.lifecycle.push_back((self.revision, Life::Activate));
        self.announce = true;
    }

    pub(crate) fn activated(mut self) -> Self {
        self.activate();
        self
    }
}

impl Task {
    async fn live(mut self, mut inbox: mpsc::UnboundedReceiver<Message>) -> Ended {
        if std::mem::take(&mut self.announce) {
            if let Some(now) = self.time.query_tick().await {
                self.now = now;
            }
            let entries = self.health.lifecycle(
                circular_protocol::actor_events::ActorHealthState::Running,
                None,
                self.sample(),
            );
            self.write_health(entries);
        }
        let intent = self.inherited_pause;
        if let Next::Stop = self.record_pause(intent).await {
            return self.exit(inbox, Exit::Fault).await;
        }
        self.start_source();
        self.request_restored_cells();
        let exit = loop {
            if let Next::Stop = self.progress().await {
                break Exit::Fault;
            }
            let release = [
                self.inlets.next_release(),
                self.behavior.timers.next_deadline(),
                self.display.as_ref().map(|window| window.end),
                self.behavior
                    .effects
                    .as_ref()
                    .and_then(super::effect::EffectPort::next_retry),
            ]
            .into_iter()
            .flatten()
            .min();
            let time = self.time.clone();
            let door = self.door.clone();
            let blocked = self.outlets.blocked();
            let awaiting = self
                .behavior
                .effects
                .as_ref()
                .is_some_and(super::effect::EffectPort::awaiting);
            tokio::select! {
                message = inbox.recv() => match message {
                    Some(message) => match self.handle(message).await {
                        Ok(Next::Continue) => {}
                        Ok(Next::Stop) => break Exit::Fault,
                        Err(exit) => break exit,
                    },
                    None => break Exit::Fault,
                },
                answer = async { (&mut self.flying.as_mut().expect("guarded").answer).await },
                    if self.flying.is_some() => {
                    if let Next::Stop = self.on_recorded(answer).await {
                        break Exit::Fault;
                    }
                },
                () = async { time.wait_until(release.expect("guarded")).await },
                    if release.is_some() => {},
                () = async { door.opened.notified().await }, if self.door.paused() => {},
                gone = self.outlets.flush(&door), if blocked && !door.paused() => {
                    self.destinations_gone(gone);
                },
                outcome = std::future::poll_fn(|context| {
                    self.behavior.effects.as_mut().expect("guarded").poll_outcome(context)
                }), if awaiting => {
                    self.outcomes.push_back(outcome);
                },
                _ = async { self.restarting.as_mut().expect("guarded").await },
                    if self.restarting.is_some() => {
                    self.restarting = None;
                    self.start_source();
                },
                written = async { self.writes.front_mut().expect("guarded").await },
                    if !self.writes.is_empty() => {
                    self.writes.pop_front();
                    if let Next::Stop = self.on_written(written) {
                        break Exit::Fault;
                    }
                },
            }
        };
        self.exit(inbox, exit).await
    }

    async fn exit(mut self, mut inbox: mpsc::UnboundedReceiver<Message>, exit: Exit) -> Ended {
        inbox.close();
        let mut deliveries = Vec::new();
        while let Ok(message) = inbox.try_recv() {
            match message {
                Message::Deliver(delivery) => deliveries.push(delivery),
                Message::Inject(injection) => self.injections.push_back(injection),
                Message::Refused(refused) => self.refused(*refused),
                Message::Settled(outcome) => self.outcomes.push_back(*outcome),
                Message::Inlets(_)
                | Message::Share(..)
                | Message::Control(_)
                | Message::Depths(_)
                | Message::CellStood(_)
                | Message::Fell(_)
                | Message::ApprovalUnavailable(_) => {}
            }
        }
        if exit == Exit::Fault && self.faulted {
            self.stop_source();
            self.record_fault().await;
        }
        match exit {
            Exit::Stop => self.stop_here().await,
            Exit::Retire => self.retire(deliveries).await,
            Exit::Fault => {
                self.stop_source();
                let unrecorded = self.inlets.drain_unadmitted();
                self.gone_here(unrecorded.into_iter().chain(deliveries));
                self.record_outcomes().await;
            }
        }
        if exit != Exit::Fault && self.faulted {
            self.stop_source();
            self.record_fault().await;
        }
        for injection in self.injections.drain(..) {
            let _ = injection.reply.send(Err(Refusal::NotAccepting));
        }
        if exit != Exit::Stop {
            let waiting = self.outlets.abandon();
            self.destinations_gone(waiting);
        }
        self.settle().await;
        if let Some(cache) = self.cache.as_mut() {
            cache.finish().await;
        }
        self.ended()
    }

    fn request_restored_cells(&mut self) {
        let restored: Vec<_> = self
            .cells
            .as_ref()
            .filter(|plane| plane.restore_after.is_none())
            .into_iter()
            .flat_map(|plane| {
                plane
                    .lifecycle
                    .minted()
                    .filter(|(key, _)| !plane.wires.contains_key(*key))
                    .map(|(key, minted)| (key.clone(), minted.clone()))
            })
            .collect();
        for (key, minted) in restored {
            self.request_cell(&key, minted, true);
        }
    }

    fn ended(&mut self) -> Ended {
        Ended {
            fault: self.fault.take(),
            issuer: Some(std::mem::replace(
                &mut self.issuer,
                Issuer::new(self.actor.clone()),
            )),
        }
    }

    async fn progress(&mut self) -> Next {
        loop {
            let now = self.time.query_tick().await;
            let Some(now) = now else {
                return self.gone("the time actor stopped answering");
            };
            self.now = now;
            let quiet = self.quiet();
            let behavior = &self.behavior;
            self.inlets
                .release(now, |inlet| quiet && behavior.accepts(inlet));
            self.firing.extend(self.behavior.timers.due(now));
            if self
                .display
                .as_ref()
                .is_some_and(|window| window.end <= now)
            {
                self.close_display();
            }
            if let Some(port) = self.behavior.effects.as_mut() {
                port.release_retries(now);
            }
            self.harness_health(false);
            let mut moved = false;
            if self.can_consume() {
                let Some(arrival) = self.inlets.next_arrival() else {
                    break;
                };
                if let Next::Stop = self.turn(arrival).await {
                    return Next::Stop;
                }
                moved = true;
            }
            if self.flying.is_none() {
                moved |= self.fly(now);
            }
            if !moved {
                break;
            }
        }
        Next::Continue
    }

    fn quiet(&self) -> bool {
        !self.latest_flags.pause()
            && !(self.behavior.effects.is_some() && self.inlets.has_arrival())
    }

    fn can_consume(&self) -> bool {
        !self.poisoned
            && !self.door.paused()
            && !self.outlets.blocked()
            && self.inlets.has_arrival()
    }

    fn fly(&mut self, now: Tick) -> bool {
        let Some(journal) = self.journal() else {
            return false;
        };
        self.now = now;
        if let Some((revision, life)) = self.lifecycle.pop_front() {
            let observed_at = self.sample();
            let parents: Box<[_]> = if matches!(life, Life::Activate | Life::Become(_)) {
                self.cell_minted.iter().cloned().collect()
            } else {
                Box::new([])
            };
            if matches!(life, Life::Activate) {
                self.issuer.begin_life();
            }
            let (index, at) = match self.issuer.arrival(now, parents.iter(), revision) {
                Ok(issued) => issued,
                Err(error) => {
                    self.poison(format!("lifecycle stamp: {error:?}"));
                    return false;
                }
            };
            let candidate = EventArrivalCommit::new(
                self.actor.clone(),
                lifecycle_inlet(),
                index,
                at.clone(),
                ArrivalOrigin::EdgeDelivery {
                    edge: circular_plan::EdgeId::outcome(self.actor.clone()),
                    stamp: at,
                },
                parents.clone(),
                super::turn::lifecycle_payload(&life),
                observed_at,
            );
            let answer = journal.submit_event(candidate);
            self.flying = Some(InFlight {
                pending: Pending::Lifecycle(life),
                parents,
                revision,
                observed_at,
                answer,
            });
            return true;
        }
        if let Some(outcome) = self.outcomes.pop_front() {
            return self.fly_outcome(now, outcome);
        }
        if !self.latest_flags.pause()
            && self.applied_pause.is_none()
            && let Some((timer, armed)) = self.firing.pop_front()
        {
            return self.fly_timer(now, timer, armed);
        }
        let backed_up = self.outlets.blocked() || self.inlets.full();
        let closed = self.door.paused() || self.latest_flags.pause();
        if self
            .injections
            .front()
            .is_some_and(|injection| closed || !injection.patient || !backed_up)
            && let Some(injection) = self.injections.pop_front()
        {
            if closed {
                let _ = injection.reply.send(Err(Refusal::NotAccepting));
                return true;
            }
            let observed_at = self.sample();
            let (index, at) = match self.issuer.arrival(now, std::iter::empty(), self.revision) {
                Ok(issued) => issued,
                Err(error) => {
                    let _ = injection
                        .reply
                        .send(Err(Refusal::Rejected(format!("{error:?}"))));
                    return true;
                }
            };
            let candidate = EventArrivalCommit::new(
                self.actor.clone(),
                injection.inlet.clone(),
                index,
                at,
                ArrivalOrigin::ExternalInject {
                    origin: injection.origin.clone(),
                },
                Box::new([]),
                injection.payload.clone(),
                observed_at,
            );
            let answer = journal.submit_event(candidate);
            self.flying = Some(InFlight {
                pending: Pending::Injection {
                    inlet: injection.inlet,
                    payload: injection.payload,
                    origin: injection.origin,
                    reply: injection.reply,
                },
                parents: Box::new([]),
                revision: self.revision,
                observed_at,
                answer,
            });
            return true;
        }
        let quiet = self.quiet();
        let behavior = &self.behavior;
        let next = self
            .inlets
            .next_to_record(|inlet| quiet && behavior.accepts(inlet));
        self.admit_unrecorded(false);
        let Some(delivery) = next else {
            return false;
        };
        self.observe_pressure(&delivery.edge, false);
        let observed_at = self.sample();
        let sender = delivery.event.stamp().clone();
        let revision = delivery.event.stamp().revision();
        let (index, at) = match self.issuer.arrival(now, std::iter::once(&sender), revision) {
            Ok(issued) => issued,
            Err(error) => {
                self.poison(format!("arrival stamp: {error:?}"));
                return false;
            }
        };
        let parents: Box<[Stamp<ActorId>]> = delivery
            .event
            .causality()
            .parents()
            .iter()
            .map(circular_core::EventId::stamp)
            .cloned()
            .collect();
        let candidate = EventArrivalCommit::new_encoded(
            self.actor.clone(),
            delivery.inlet.clone(),
            index,
            at,
            ArrivalOrigin::EdgeDelivery {
                edge: delivery.edge.clone(),
                stamp: sender.clone(),
            },
            parents.clone(),
            delivery.event.payload().clone(),
            observed_at,
            Ok(delivery.encoded.clone()),
        )
        .with_body(circular_store::ArrivalBody::emitted(&sender))
        .with_route_edge(Some(delivery.edge.clone()))
        .with_result(delivery.result.clone());
        let answer = journal.submit_event(candidate);
        self.flying = Some(InFlight {
            revision,
            pending: Pending::Wire(delivery),
            parents,
            observed_at,
            answer,
        });
        true
    }

    fn fly_timer(
        &mut self,
        now: Tick,
        timer: circular_runtime::EffectId,
        armed: super::effect::Armed,
    ) -> bool {
        let Some(journal) = self.journal() else {
            return false;
        };
        let observed_at = self.sample();
        let (index, at) = match self.issuer.arrival(now, std::iter::empty(), self.revision) {
            Ok(issued) => issued,
            Err(error) => {
                self.poison(format!("timer stamp: {error:?}"));
                return false;
            }
        };
        let candidate = EventArrivalCommit::new(
            self.actor.clone(),
            timer_inlet(),
            index,
            at,
            ArrivalOrigin::TimerFire {
                timer: timer.clone(),
            },
            Box::new([]),
            super::turn::correlation_payload(armed.correlation),
            observed_at,
        );
        let answer = journal.submit_event(candidate);
        self.flying = Some(InFlight {
            pending: Pending::Timer {
                timer,
                correlation: armed.correlation,
            },
            parents: Box::new([]),
            revision: self.revision,
            observed_at,
            answer,
        });
        true
    }

    fn fly_outcome(
        &mut self,
        now: Tick,
        outcome: circular_runtime::EffectOutcome<circular_runtime::EffectId>,
    ) -> bool {
        let Some(journal) = self.journal() else {
            return false;
        };
        let ticks_per_second = self.time.ticks_per_second();
        let outcome = match self.behavior.effects.as_mut() {
            Some(port) => match port.settle_or_retry(outcome, now, ticks_per_second) {
                Some(outcome) => outcome,
                None => return true,
            },
            None => outcome,
        };
        let Some((cause, term)) = self
            .behavior
            .effects
            .as_ref()
            .and_then(|effects| effects.recorded(outcome.correlation()))
        else {
            return true;
        };
        let observed_at = self.sample();
        let (revision, parent) = if term
            .as_ref()
            .is_some_and(|term| term.constructor().is_external_entry())
        {
            self.issuer.observe(&cause);
            (cause.revision().max(self.revision), None)
        } else {
            (self.revision, Some(&cause))
        };
        let (index, at) = match self.issuer.arrival(now, parent, revision) {
            Ok(issued) => issued,
            Err(error) => {
                self.poison(format!("outcome stamp: {error:?}"));
                return false;
            }
        };
        let candidate = crate::arrival_commit::EffectArrivalCommit::new(
            self.actor.clone(),
            index,
            at,
            outcome.correlation().clone(),
            cause,
            term,
            outcome.result().clone(),
            observed_at,
        )
        .with_failure_progress(outcome.failure_progress());
        let answer = journal.submit_effect(candidate);
        self.flying = Some(InFlight {
            pending: Pending::Outcome(Box::new(outcome)),
            parents: Box::new([]),
            revision,
            observed_at,
            answer,
        });
        true
    }

    async fn on_recorded(
        &mut self,
        answer: Result<ArrivalAnswer, oneshot::error::RecvError>,
    ) -> Next {
        let flying = self
            .flying
            .take()
            .expect("an answer belongs to the flying record");
        let receipt = match answer {
            Ok(Ok(receipt)) => receipt,
            Ok(Err(error)) if self.port.recorder_alive() => {
                if let Pending::Injection { reply, .. } = flying.pending {
                    let _ = reply.send(Err(Refusal::Rejected(format!(
                        "the injection was not recorded: {error}"
                    ))));
                    return Next::Continue;
                }
                return self.stop(&format!("arrival was not recorded: {error}"));
            }
            Ok(Err(error)) => return self.gone(&format!("arrival was not recorded: {error}")),
            Err(_) => return self.gone("the journal writer is gone"),
        };
        let needs_mailbox = receipt.needs_mailbox();
        if needs_mailbox && let Err(error) = self.issuer.recorded(receipt.index()) {
            return self.stop(&format!("arrival index: {error:?}"));
        }
        match flying.pending {
            Pending::Lifecycle(life) => {
                if !needs_mailbox {
                    return self.stop("a lifecycle arrival folded into an earlier record");
                }
                let edge = circular_plan::EdgeId::outcome(self.actor.clone());
                let payload = super::turn::lifecycle_payload(&life);
                let event = match super::turn::lifecycle_event(
                    self.stream,
                    receipt.at().clone(),
                    payload,
                    &flying.parents,
                ) {
                    Ok(event) => event,
                    Err(error) => return self.stop(&error),
                };
                let stop = matches!(life, Life::Stop(_));
                let control = matches!(life, Life::Pause { .. } | Life::Resume);
                self.resumed
                    .fold(receipt.index(), &life, receipt.observed_at());
                match life {
                    Life::Activate => {
                        self.starts.insert(receipt.index());
                    }
                    Life::Become(Some(incarnation)) => {
                        self.prepared.insert(receipt.index(), incarnation);
                    }
                    Life::Pause { force, consumed } => {
                        self.resumed.consumed(consumed);
                        self.apply_pause(Some(force));
                    }
                    Life::Resume => {
                        self.apply_pause(None);
                        self.start_source();
                    }
                    Life::Become(None) | Life::Stop(_) => {}
                }
                let recorded = Recorded {
                    index: receipt.index(),
                    at: receipt.at().clone(),
                    observed_at: receipt.observed_at(),
                    route: None,
                    origin: ArrivalOrigin::EdgeDelivery {
                        edge,
                        stamp: receipt.at().clone(),
                    },
                    inlet: lifecycle_inlet(),
                    revision: flying.revision,
                    event,
                    emission_body: None,
                    result: EnvelopeResult::Ok,
                    outcome: None,
                };
                if stop {
                    self.stopped = Some(recorded);
                } else if !control {
                    self.inlets.recorded(recorded);
                }
            }
            Pending::Outcome(outcome) => {
                if let Some(port) = self.behavior.effects.as_mut() {
                    port.acknowledge_peer(outcome.correlation(), receipt.index());
                }
                if !needs_mailbox {
                    return Next::Continue;
                }
                let recorded = super::column::outcome(
                    self.stream,
                    self.actor.clone(),
                    super::column::Outcome {
                        index: receipt.index(),
                        at: receipt.at().clone(),
                        observed_at: receipt.observed_at(),
                        outcome: *outcome,
                    },
                );
                match recorded {
                    Ok(recorded) => self.inlets.recorded(recorded),
                    Err(unreadable) => return self.stop(&unreadable.0),
                }
            }
            Pending::Timer { timer, correlation } => {
                if !needs_mailbox {
                    return Next::Continue;
                }
                let recorded = super::column::timer_fire(
                    self.stream,
                    receipt.index(),
                    receipt.at().clone(),
                    receipt.observed_at(),
                    timer,
                    super::turn::correlation_payload(correlation),
                );
                match recorded {
                    Ok(recorded) => self.inlets.recorded(recorded),
                    Err(unreadable) => return self.stop(&unreadable.0),
                }
            }
            Pending::Wire(delivery) => {
                if !needs_mailbox {
                    self.inlets.folded(&delivery.edge);
                    return Next::Continue;
                }
                let origin: InputOrigin = ArrivalOrigin::EdgeDelivery {
                    edge: delivery.edge.clone(),
                    stamp: delivery.event.stamp().clone(),
                };
                self.inlets.recorded(Recorded {
                    index: receipt.index(),
                    at: receipt.at().clone(),
                    observed_at: receipt.observed_at(),
                    route: Some(delivery.edge),
                    origin,
                    inlet: delivery.inlet,
                    revision: flying.revision,
                    event: delivery.event,
                    emission_body: Some(delivery.encoded),
                    result: delivery.result,
                    outcome: None,
                });
            }
            Pending::Injection {
                inlet,
                payload,
                origin,
                reply,
            } => {
                if !needs_mailbox {
                    let _ = reply.send(Ok(false));
                    return Next::Continue;
                }
                let event = match circular_core::admit(
                    self.stream,
                    circular_core::Emission::from_runtime(
                        circular_core::emit(payload),
                        circular_core::Causality::Source,
                        None,
                    ),
                    receipt.at().clone(),
                ) {
                    Ok(event) => event,
                    Err(error) => return self.stop(&format!("injected event: {error:?}")),
                };
                self.inlets.recorded(Recorded {
                    index: receipt.index(),
                    at: receipt.at().clone(),
                    observed_at: receipt.observed_at(),
                    route: None,
                    origin: ArrivalOrigin::ExternalInject { origin },
                    inlet,
                    revision: flying.revision,
                    event,
                    emission_body: None,
                    result: EnvelopeResult::Ok,
                    outcome: None,
                });
                let _ = reply.send(Ok(true));
            }
        }
        let _ = flying.parents;
        let _ = flying.observed_at;
        Next::Continue
    }

    async fn handle(&mut self, message: Message) -> Result<Next, Exit> {
        match message {
            Message::Settled(outcome) => self.outcomes.push_back(*outcome),
            Message::ApprovalUnavailable(reason) => return Ok(self.stop(&reason)),
            Message::Refused(refused) => self.refused(*refused),
            Message::Fell(failure) => self.fell(&failure),
            Message::CellStood(stood) => self.cell_stood(*stood),
            Message::Depths(reply) => reply.answer(self.inlets.depths()),
            Message::Deliver(delivery) => {
                let Some(now) = self.time.query_tick().await else {
                    return Ok(self.gone("the time actor stopped answering"));
                };
                self.now = now;
                let edge = delivery.edge.clone();
                match self
                    .inlets
                    .receive(delivery, now, self.time.ticks_per_second())
                {
                    Received::Admitted => self.observe_pressure(&edge, false),
                    Received::Held => {
                        self.admit_held();
                        self.observe_pressure(&edge, false);
                    }
                    Received::Shed(delivery) => {
                        self.shed(*delivery, DeadLetterReason::Capacity);
                        self.observe_pressure(&edge, true);
                    }
                    Received::Unknown(delivery) => {
                        return Ok(self.stop(&format!(
                            "a delivery on an edge this actor does not hold: {:?}",
                            delivery.edge
                        )));
                    }
                }
            }
            Message::Inject(injection) => self.injections.push_back(injection),
            Message::Inlets(share) => self.inlets.install(share.revision, &share.inlets),
            Message::Share(share, incarnation) => {
                let revision = share.revision;
                self.install(*share);
                self.lifecycle
                    .push_back((revision, Life::Become(incarnation)));
            }
            Message::Control(Control::Retire) => return Err(Exit::Retire),
            Message::Control(Control::Stop) => return Err(Exit::Stop),
            Message::Control(Control::Pause { force }) => {
                return Ok(self.record_pause(Some(force)).await);
            }
            Message::Control(Control::Resume) => {
                return Ok(self.record_pause(None).await);
            }
            Message::Control(Control::Harness { harness, executor }) => {
                self.rebind(&harness, executor.as_ref()).await;
            }
        }
        Ok(Next::Continue)
    }

    async fn rebind(
        &mut self,
        harness: &circular_runtime::AgentHarnessName,
        executor: Option<&crate::execution_profile::AgentExecutorFactory>,
    ) {
        let queued = self
            .lifecycle
            .iter_mut()
            .filter_map(|(_, life)| match life {
                Life::Become(Some(incarnation)) => incarnation.behavior.effects.as_mut(),
                _ => None,
            });
        let flying = self
            .flying
            .iter_mut()
            .filter_map(|flying| match &mut flying.pending {
                Pending::Lifecycle(Life::Become(Some(incarnation))) => {
                    incarnation.behavior.effects.as_mut()
                }
                _ => None,
            });
        let prepared = self
            .prepared
            .values_mut()
            .filter_map(|incarnation| incarnation.behavior.effects.as_mut());
        let closing = self
            .closing
            .iter_mut()
            .filter_map(|closing| closing.next.effects.as_mut());
        for port in self
            .behavior
            .effects
            .iter_mut()
            .chain(queued)
            .chain(flying)
            .chain(prepared)
            .chain(closing)
        {
            port.rebind(harness, executor);
        }
        if let Some(now) = self.time.query_tick().await {
            self.now = now;
        }
        self.harness_health(false);
    }

    fn harness_health(&mut self, restated: bool) {
        let Some(port) = self
            .behavior
            .effects
            .as_ref()
            .filter(|port| port.harness().is_some())
        else {
            return;
        };
        let changed = self
            .health
            .harness_wait(port.harness_wait().map(super::effect::HarnessWait::reason));
        if changed || restated {
            let entries = self.health.waiting(restated);
            self.write_health(entries);
        }
    }

    fn install(&mut self, share: Share) {
        self.revision = share.revision;
        self.latest_flags = share.declaration.flags();
        self.staged_flags
            .insert(share.revision, share.declaration.flags());
        self.inlets.install(share.revision, &share.inlets);
        self.staged.insert(share.revision, share.outlets);
        if let Some(plane) = self.cells.as_mut() {
            if let Some(prototype) = share.prototype {
                plane.staged.insert(share.revision, prototype);
            }
            plane.declarations.insert(share.revision, share.declaration);
        }
    }

    async fn turn(&mut self, arrival: Recorded) -> Next {
        #[cfg(test)]
        if !self.port.is_live() {
            self.replayed_turns += 1;
        }
        let boundary = arrival.index.next().map(|next| (next, arrival.at.clone()));
        let observed_at = arrival.observed_at;
        let life = match super::turn::lifecycle(&arrival.inlet, arrival.event.payload().value()) {
            Ok(life) => life,
            Err(reason) => return self.stop(&reason),
        };
        let stop = life.as_ref().is_some_and(|life| !life.declares());
        if !stop {
            self.health.arrived(observed_at);
        }
        if !is_lifecycle(&arrival.inlet) && arrival.outcome.is_none() {
            self.tally(&arrival);
        }
        let next = self.turn_inner(arrival).await;
        if !stop {
            let harness = self
                .behavior
                .effects
                .as_ref()
                .and_then(super::effect::EffectPort::harness_wait)
                .map(super::effect::HarnessWait::reason);
            self.health.harness_wait(harness);
            let entries = self.health.turn_end();
            self.write_health(entries);
        }
        if matches!(next, Next::Continue) {
            self.last_turn = boundary;
            if !stop
                && self
                    .last_turn
                    .as_ref()
                    .is_some_and(|(index, _)| index.get() % super::snapshot::EVERY == 0)
            {
                self.snapshot(false).await;
            }
        }
        next
    }

    async fn turn_inner(&mut self, mut arrival: Recorded) -> Next {
        if let Some(outcome) = arrival.outcome.take() {
            return self.outcome_turn(arrival, &outcome).await;
        }
        if let ArrivalOrigin::TimerFire { timer } = &arrival.origin {
            self.behavior.timers.fired(timer);
        }
        if is_lifecycle(&arrival.inlet) {
            match super::turn::lifecycle(&arrival.inlet, arrival.event.payload().value()) {
                Ok(Some(Life::Activate | Life::Become(_))) => {}
                Ok(_) => return Next::Continue,
                Err(reason) => return self.stop(&reason),
            }
            let revision = arrival.revision;
            let next = self.lifecycle_turn(arrival).await;
            if matches!(next, Next::Continue)
                && self.port.is_live()
                && let Some(plane) = self.cells.as_mut()
                && plane.restore_after.is_some_and(|after| revision >= after)
            {
                plane.restore_after = None;
                self.request_restored_cells();
            }
            return next;
        }
        let elements = match &arrival.route {
            Some(edge) => match self.inlets.wire(edge, arrival.revision) {
                Some(wire) => match preprocess(wire, &arrival, self.behavior.inlet_type()) {
                    InletVerdict::Elements(elements) => elements,
                    InletVerdict::Suppressed => return Next::Continue,
                },
                None => {
                    return self.stop(&format!(
                        "a recorded arrival on an edge this actor no longer holds: {edge:?}"
                    ));
                }
            },
            None => vec![Element::Input(
                arrival.event.payload().clone(),
                arrival.result.clone(),
            )],
        };
        let mut inputs = Vec::with_capacity(elements.len());
        for element in elements {
            match element {
                Element::Input(payload, result) => inputs.push((payload, result)),
                Element::Failed {
                    subject,
                    cause,
                    point,
                } => {
                    let record = DeadLetterRecord::new(
                        subject,
                        DeadLetterOrigin::new(
                            self.actor.as_actor_id(),
                            Some(arrival.inlet.clone()),
                        ),
                        DeadLetterReason::Processing(cause),
                        arrival.at.clone(),
                    )
                    .with_failure_point(point);
                    self.dead_letter(record);
                }
            }
        }
        let bypass = bypass_ports(self.behavior.actor_type);
        let decision = circular_runtime::decide_input_flags(self.flags, bypass.is_some())
            .unwrap_or(circular_runtime::InputFlagDecision::Invoke {
                emissions: gate_of(self.flags),
            });
        self.gate = match decision {
            circular_runtime::InputFlagDecision::Invoke { emissions }
            | circular_runtime::InputFlagDecision::Bypass {
                passthrough: emissions,
            } => emissions,
            circular_runtime::InputFlagDecision::Paused => gate_of(self.flags),
        };
        if let (circular_runtime::InputFlagDecision::Bypass { .. }, Some((inlet, outlet))) =
            (decision, &bypass)
            && &arrival.inlet == inlet
        {
            for (payload, result) in inputs {
                if let Next::Stop = self.emit(&arrival, outlet, payload, &result) {
                    return Next::Stop;
                }
            }
            return Next::Continue;
        }
        let origin: InputOrigin = arrival.origin.clone();
        let effects = match self.behavior.on_arrival(&arrival, &origin, inputs) {
            Hooked::Effects(effects) => effects,
            Hooked::Panicked => {
                self.poisoned = true;
                self.poisoned_health(arrival.observed_at);
                let record = DeadLetterRecord::new(
                    arrival.event.payload().clone(),
                    DeadLetterOrigin::new(self.actor.as_actor_id(), Some(arrival.inlet.clone())),
                    DeadLetterReason::Poisoned,
                    arrival.at.clone(),
                );
                self.dead_letter(record);
                return Next::Continue;
            }
        };
        let occasion = circular_runtime::EffectOccasion::Delivery(
            arrival.route.clone(),
            arrival.event.stamp().clone(),
        );
        self.apply_effects(&arrival, effects, occasion).await
    }

    async fn outcome_turn(
        &mut self,
        arrival: Recorded,
        outcome: &circular_runtime::EffectOutcome<circular_runtime::EffectId>,
    ) -> Next {
        if !self
            .behavior
            .effects
            .as_ref()
            .is_some_and(|port| port.knows(outcome.correlation()))
        {
            return Next::Continue;
        }
        if self
            .behavior
            .effects
            .as_ref()
            .is_some_and(|port| port.is_request(outcome.correlation()))
        {
            return self.decision_turn(outcome);
        }
        if let Some(effects) = self.behavior.effects.as_mut() {
            effects.settle(outcome.correlation());
        }
        self.health.settled(outcome.correlation());
        self.gate = gate_of(self.flags);
        match self.behavior.on_outcome(outcome) {
            Hooked::Effects(effects) => {
                let occasion = circular_runtime::EffectOccasion::Delivery(
                    Some(circular_plan::EdgeId::outcome(self.actor.clone())),
                    arrival.at.clone(),
                );
                if let Next::Stop = self.apply_effects(&arrival, effects, occasion).await {
                    return Next::Stop;
                }
            }
            Hooked::Panicked => {
                self.poisoned = true;
                self.poisoned_health(arrival.observed_at);
            }
        }
        self.closed(arrival, outcome.correlation()).await
    }

    async fn closed(&mut self, arrival: Recorded, id: &circular_runtime::EffectId) -> Next {
        let Some(closing) = self.closing.as_mut() else {
            return Next::Continue;
        };
        closing.effects.retain(|effect| effect != id);
        if !closing.effects.is_empty() {
            return Next::Continue;
        }
        let Some(Closing { next, revision, .. }) = self.closing.take() else {
            return Next::Continue;
        };
        if self.stand(*next, true, revision).await.is_err() {
            return Next::Stop;
        }
        self.opened(arrival, true).await
    }

    fn decision_turn(
        &mut self,
        outcome: &circular_runtime::EffectOutcome<circular_runtime::EffectId>,
    ) -> Next {
        let live = self.port.is_live();
        let request = outcome.correlation().clone();
        let decision = match outcome.result() {
            Ok(circular_runtime::OutcomePayload::Approval(decision)) => Ok(decision.clone()),
            Ok(_) => Err(circular_runtime::EffectFailure::InterpreterFault(
                circular_runtime::InterpreterFault::InvalidInput,
            )),
            Err(failure) => Err(failure.clone()),
        };
        let Some(port) = self.behavior.effects.as_mut() else {
            return Next::Continue;
        };
        match port.decided(&request, decision, live) {
            Some(super::effect::Decided::Approved { standing }) => {
                if let Some(target) = standing {
                    self.health.settled(&target);
                }
                if let Some(journal) = self.journal() {
                    self.writes.push_back(journal.submit_approval(
                        crate::product_arrival_journal::ApprovalMutation::Settle(
                            request,
                            circular_store::ApprovalTerminal::Consumed,
                        ),
                    ));
                }
                Next::Continue
            }
            Some(super::effect::Decided::Failed(failed)) => {
                if live {
                    self.outcomes.push_back(failed);
                }
                Next::Continue
            }
            None => Next::Continue,
        }
    }

    fn open_approval(
        &mut self,
        arrival: &Recorded,
        request: circular_runtime::EffectId,
        spec: circular_runtime::ApprovalSpec,
    ) -> Result<(), String> {
        let body = crate::restart_custody_codec::encode_approval_body(
            &crate::restart_custody_codec::ProductRestoreNestedCodec,
            &crate::restart_custody_codec::ApprovalRestoreBody {
                actor: self.actor.as_actor_id(),
                request_term: circular_runtime::EffectTerm::RequestApproval(spec),
                cause: (arrival.index, arrival.at.clone()),
                submitted_at: arrival.observed_at,
            },
        )
        .map_err(|error| format!("approval request body: {error}"))?;
        let journal = self
            .journal()
            .ok_or("an approval request is opened only on the live port")?;
        self.writes.push_back(journal.submit_approval(
            crate::product_arrival_journal::ApprovalMutation::Open(request.clone(), body),
        ));
        self.approvals
            .register(request, self.own.address().clone())?;
        Ok(())
    }

    async fn apply_effects(
        &mut self,
        arrival: &Recorded,
        effects: Vec<ActorEffect<ProductPayload>>,
        occasion: circular_runtime::EffectOccasion,
    ) -> Next {
        self.apply_effects_issuing(arrival, effects, occasion)
            .await
            .0
    }

    async fn apply_effects_issuing(
        &mut self,
        arrival: &Recorded,
        effects: Vec<ActorEffect<ProductPayload>>,
        occasion: circular_runtime::EffectOccasion,
    ) -> (Next, Vec<circular_runtime::EffectId>) {
        let mut issued = Vec::new();
        let mut index = 0u64;
        let mut forward = None;
        for effect in effects {
            let at = index;
            index += 1;
            match effect {
                ActorEffect::Emit {
                    port,
                    payload,
                    key: _,
                    result,
                } => {
                    if let Next::Stop = self.emit(arrival, &port, payload, &result) {
                        return (Next::Stop, issued);
                    }
                }
                ActorEffect::Reject { subject, cause } => {
                    let record = DeadLetterRecord::new(
                        subject,
                        DeadLetterOrigin::new(
                            self.actor.as_actor_id(),
                            Some(arrival.inlet.clone()),
                        ),
                        DeadLetterReason::Processing(cause),
                        arrival.at.clone(),
                    );
                    self.dead_letter(record);
                }
                ActorEffect::DeadLetter { subject, reason } => {
                    let record = DeadLetterRecord::new(
                        subject,
                        DeadLetterOrigin::new(self.actor.as_actor_id(), None),
                        DeadLetterReason::ActorDeclared(reason),
                        arrival.at.clone(),
                    );
                    self.dead_letter(record);
                }
                ActorEffect::Suppress { .. } => {}
                ActorEffect::External(effect) => {
                    if let circular_runtime::Effect::MutateInstance { spec } = &effect {
                        let id = circular_runtime::EffectId::at_hook(
                            &self.behavior.incarnation,
                            occasion.clone(),
                            at,
                        );
                        if let Next::Stop =
                            self.mutate_instance(arrival, spec, id, &mut forward).await
                        {
                            return (Next::Stop, issued);
                        }
                        continue;
                    }
                    if matches!(effect, circular_runtime::Effect::RequestApproval { .. }) {
                        return (
                            self.stop("actor-raised approvals are not wired into this kernel"),
                            issued,
                        );
                    }
                    let id = circular_runtime::EffectId::at_hook(
                        &self.behavior.incarnation,
                        occasion.clone(),
                        at,
                    );
                    if let circular_runtime::Effect::Schedule { spec } = effect {
                        let after = super::inlet::delay_ticks(
                            spec.after().get().get(),
                            1_000,
                            self.time.ticks_per_second(),
                        );
                        let from = self
                            .resumed
                            .floor(arrival.index)
                            .map_or(arrival.observed_at, |resumed| {
                                resumed.max(arrival.observed_at)
                            });
                        self.behavior.timers.arm(
                            id,
                            super::effect::Armed {
                                deadline: Tick::new(from.millis().saturating_add(after)),
                                correlation: spec.correlation().get(),
                            },
                        );
                        continue;
                    }
                    let required = match crate::actor_approval::required(
                        &self.behavior.approval,
                        &self.behavior.actor,
                        effect.constructor(),
                    ) {
                        Ok(required) => required && effect.approval_ticket().is_none(),
                        Err(reason) => {
                            return (
                                self.stop(&format!("approval requirement: {reason}")),
                                issued,
                            );
                        }
                    };
                    let live = self.port.is_live();
                    if required {
                        index += 1;
                        let target = circular_runtime::EffectId::at_hook(
                            &self.behavior.incarnation,
                            occasion.clone(),
                            at + 1,
                        );
                        let Some(port) = self.behavior.effects.as_mut() else {
                            return (
                                self.stop("this actor type declares no external effect"),
                                issued,
                            );
                        };
                        let spec =
                            port.hold(id.clone(), target.clone(), effect, arrival.at.clone());
                        self.health.submitted(target.clone(), arrival.observed_at);
                        issued.push(target);
                        if live && let Err(reason) = self.open_approval(arrival, id, spec) {
                            return (self.stop(&reason), issued);
                        }
                        continue;
                    }
                    let Some(port) = self.behavior.effects.as_mut() else {
                        return (
                            self.stop("this actor type declares no external effect"),
                            issued,
                        );
                    };
                    let standing = port.stands(effect.constructor());
                    port.begin(id.clone(), effect, arrival.at.clone(), live);
                    if !standing {
                        self.health.submitted(id.clone(), arrival.observed_at);
                    }
                    issued.push(id);
                }
                ActorEffect::Peer(effect) => {
                    let id = circular_runtime::EffectId::at_hook(
                        &self.behavior.incarnation,
                        occasion.clone(),
                        at,
                    );
                    let live = self.port.is_live();
                    let Some(port) = self.behavior.effects.as_mut() else {
                        return (
                            self.stop("this actor type declares no external effect"),
                            issued,
                        );
                    };
                    let standing = port.stands(effect.constructor());
                    if !port.begin_peer(id.clone(), effect, arrival.at.clone(), live) {
                        return (self.stop("this actor holds no peer binding"), issued);
                    }
                    if !standing {
                        self.health.submitted(id.clone(), arrival.observed_at);
                    }
                    issued.push(id);
                }
            }
        }
        if let Some(key) = forward {
            self.forward_to_cell(&key, arrival);
        }
        (Next::Continue, issued)
    }

    async fn mutate_instance(
        &mut self,
        arrival: &Recorded,
        spec: &circular_runtime::InstanceMutationSpec,
        id: circular_runtime::EffectId,
        forward: &mut Option<circular_runtime::InstanceKey>,
    ) -> Next {
        use circular_runtime::{InstanceDisposition, InstanceIntent};
        let Some(plane) = self.cells.as_mut() else {
            return self.stop("an instance mutation from an actor that holds no cells");
        };
        let (disposition, key) = match spec.intent() {
            InstanceIntent::Instantiate { key } => {
                *forward = Some(key.clone());
                (
                    if plane.lifecycle.get(key).is_some() {
                        InstanceDisposition::AlreadyLive
                    } else {
                        InstanceDisposition::Minted
                    },
                    key.clone(),
                )
            }
            InstanceIntent::Retire { key } => (
                if plane.lifecycle.get(key).is_some() {
                    InstanceDisposition::Retired
                } else {
                    InstanceDisposition::NotLive
                },
                key.clone(),
            ),
        };
        let target = plane.lifecycle.get(&key).cloned();
        let transition = self.write_instance_transition(disposition, &key).await;
        if self.fault.is_some() {
            return Next::Stop;
        }
        if matches!(
            disposition,
            InstanceDisposition::Minted | InstanceDisposition::Retired
        ) && transition.is_none()
        {
            *forward = None;
            return Next::Continue;
        }
        if self.port.is_live() {
            match disposition {
                InstanceDisposition::Minted => {
                    if let Some(minted) = transition {
                        self.request_cell(&key, minted, false);
                    }
                }
                InstanceDisposition::Retired => {
                    if let (Some(plane), Some(target), Some(retired)) =
                        (self.cells.as_ref(), target, transition)
                    {
                        plane.hand.retire(&self.actor, &key, target, retired);
                    }
                }
                _ => {}
            }
        }
        if disposition != InstanceDisposition::Retired
            && let Some(schedule) = self
                .behavior
                .instance_disposition(spec.intent(), disposition)
        {
            let after = super::inlet::delay_ticks(
                schedule.after().get().get(),
                1_000,
                self.time.ticks_per_second(),
            );
            self.behavior.timers.arm(
                id,
                super::effect::Armed {
                    deadline: Tick::new(arrival.observed_at.millis().saturating_add(after)),
                    correlation: schedule.correlation().get(),
                },
            );
        }
        Next::Continue
    }

    fn fold_cell_record(
        &mut self,
        record: &circular_store::Record<circular_store::ProductStore>,
    ) -> Result<Vec<(circular_runtime::InstanceKey, Stamp<ActorId>)>, String> {
        let Some(plane) = self.cells.as_mut() else {
            return Ok(Vec::new());
        };
        let before: Vec<_> = plane
            .lifecycle
            .minted()
            .map(|(key, at)| (key.clone(), at.clone()))
            .collect();
        plane
            .lifecycle
            .read(record)
            .map_err(|error| error.to_string())?;
        if let Some(transition) =
            crate::instance_journal::decode_transition(record).map_err(|error| error.to_string())?
            && transition.disposition == circular_runtime::InstanceDisposition::Minted
            && plane.lifecycle.get(&transition.key) == Some(record.header().at())
        {
            plane
                .prototypes
                .insert(transition.key, plane.prototype.clone());
        }
        let ended: Vec<_> = before
            .into_iter()
            .filter(|(key, at)| plane.lifecycle.get(key) != Some(at))
            .collect();
        self.forget_cells(&ended);
        Ok(ended)
    }

    fn forget_cells(&mut self, ended: &[(circular_runtime::InstanceKey, Stamp<ActorId>)]) {
        let Some(plane) = self.cells.as_mut() else {
            return;
        };
        for (key, _) in ended {
            self.behavior.instance_disposition(
                &circular_runtime::InstanceIntent::Retire { key: key.clone() },
                circular_runtime::InstanceDisposition::Retired,
            );
            plane.prototypes.remove(key);
            plane.wires.remove(key);
            plane.pending.remove(key);
        }
    }

    async fn write_instance_transition(
        &mut self,
        disposition: circular_runtime::InstanceDisposition,
        key: &circular_runtime::InstanceKey,
    ) -> Option<Stamp<ActorId>> {
        use circular_runtime::InstanceDisposition;
        if !matches!(
            disposition,
            InstanceDisposition::Minted | InstanceDisposition::Retired
        ) {
            return None;
        }
        if !self.port.is_live() {
            let plane = self.cells.as_mut()?;
            let record = plane.recorded.front()?;
            let transition = match crate::instance_journal::decode_transition(record) {
                Ok(Some(transition)) => transition,
                Ok(None) => return None,
                Err(error) => {
                    self.poison(error.to_string());
                    return None;
                }
            };
            if transition.disposition != disposition
                || &transition.key != key
                || record.header().at().revision() != self.turn_revision
            {
                return None;
            }
            let record = plane.recorded.pop_front().expect("the recorded transition");
            let at = record.header().at().clone();
            if let Err(reason) = self.fold_cell_record(&record) {
                self.poison(reason);
                return None;
            }
            return Some(at);
        }
        let Ok(scope) = self
            .actor
            .scope()
            .append_segment(circular_plan::ScopeSeg::Child(self.actor.name().clone()))
        else {
            self.poison("an instance set scope does not extend its container".to_owned());
            return None;
        };
        let incarnation = self
            .behavior
            .incarnation
            .generations()
            .last()
            .map_or(0, |generation| generation.get());
        let written = self
            .issuer
            .observation(self.now, self.turn_revision)
            .map_err(|error| format!("instance transition stamp: {error:?}"))
            .and_then(|at| {
                crate::instance_journal::instance_transition_record(
                    disposition,
                    scope,
                    key.clone(),
                    at,
                    incarnation,
                )
                .map_err(|error| error.to_string())
            });
        match written {
            Ok(Some(record)) => {
                let at = record.header().at().clone();
                let record = circular_store::Record::Observation(record);
                self.write(vec![record.clone()]);
                self.settle().await;
                if self.fault.is_some() {
                    return None;
                }
                if let Err(reason) = self.fold_cell_record(&record) {
                    self.poison(reason);
                    return None;
                }
                Some(at)
            }
            Ok(None) => None,
            Err(error) => {
                self.poison(format!("instance transition was not recorded: {error}"));
                None
            }
        }
    }

    async fn retire_cells(
        &mut self,
        cells: Vec<(circular_runtime::InstanceKey, Stamp<ActorId>)>,
    ) -> Next {
        for (key, minted) in cells {
            let retired = self
                .write_instance_transition(circular_runtime::InstanceDisposition::Retired, &key)
                .await;
            if self.fault.is_some() {
                return Next::Stop;
            }
            if let Some(plane) = self.cells.as_mut() {
                if self.port.is_live()
                    && let Some(retired) = retired
                {
                    plane.hand.retire(&self.actor, &key, minted, retired);
                }
            }
        }
        self.settle().await;
        if self.fault.is_some() {
            Next::Stop
        } else {
            Next::Continue
        }
    }

    async fn close_cells(&mut self) -> Next {
        let cells = self
            .cells
            .as_ref()
            .map(|plane| {
                plane
                    .lifecycle
                    .minted()
                    .map(|(key, at)| (key.clone(), at.clone()))
                    .collect()
            })
            .unwrap_or_default();
        self.retire_cells(cells).await
    }

    fn forward_to_cell(&mut self, key: &circular_runtime::InstanceKey, arrival: &Recorded) {
        if !self.port.is_live() {
            return;
        }
        let Some(edge) = arrival.route.clone() else {
            return;
        };
        let encoded = arrival
            .emission_body
            .as_ref()
            .expect("a recorded wire retains its producer body");
        let Some(plane) = self.cells.as_mut() else {
            return;
        };
        let Some(wires) = plane.wires.get(key) else {
            plane.pending.entry(key.clone()).or_default().push_back((
                edge,
                arrival.inlet.clone(),
                arrival.event.clone(),
                encoded.clone(),
                arrival.result.clone(),
            ));
            return;
        };
        let wires = wires.get(&edge).cloned().unwrap_or_default();
        self.send_to_cell(
            edge,
            arrival.inlet.clone(),
            wires,
            &arrival.event,
            encoded,
            &arrival.result,
        );
    }

    fn send_to_cell(
        &mut self,
        edge: circular_plan::EdgeId,
        inlet: PortId,
        wires: Vec<super::share::OutWire>,
        event: &ProductEvent,
        encoded: &circular_core::EncodedPayload,
        result: &EnvelopeResult,
    ) {
        if wires.is_empty() {
            let delivery = Delivery {
                edge,
                inlet,
                event: event.clone(),
                encoded: encoded.clone(),
                result: result.clone(),
                credit: None,
            };
            self.destinations_gone(vec![Gone(Box::new(delivery))]);
            return;
        }
        for wire in wires {
            let closed = self.door.paused();
            if let Some(gone) = self.outlets.forward(wire, event, encoded, result, closed) {
                self.destinations_gone(vec![gone]);
            }
        }
    }

    fn current_cell(
        &mut self,
        key: &circular_runtime::InstanceKey,
        minted: &Stamp<ActorId>,
    ) -> bool {
        let Some(plane) = self.cells.as_ref() else {
            return false;
        };
        if plane.lifecycle.get(key) == Some(minted) {
            return true;
        }
        self.dead_letter(DeadLetterRecord::new(
            super::turn::null_payload(),
            DeadLetterOrigin::new(self.actor.as_actor_id(), None),
            DeadLetterReason::DestinationGone,
            minted.clone(),
        ));
        false
    }

    fn request_cell(
        &mut self,
        key: &circular_runtime::InstanceKey,
        minted: Stamp<ActorId>,
        restore: bool,
    ) {
        if !self.current_cell(key, &minted) {
            return;
        }
        let plane = self.cells.as_ref().expect("the cell owner");
        let prototype = plane
            .prototypes
            .get(key)
            .expect("a recorded mint holds its prototype");
        plane.hand.mint(
            &self.actor,
            key,
            prototype.clone(),
            minted,
            self.own.address().clone(),
            restore,
        );
    }

    fn cell_stood(&mut self, stood: super::system::CellStood) {
        if !self.current_cell(&stood.key, &stood.minted) {
            return;
        }
        let plane = self.cells.as_mut().expect("the cell owner");
        let pending = plane.pending.remove(&stood.key).unwrap_or_default();
        plane.wires.insert(stood.key.clone(), stood.wires.clone());
        for (edge, inlet, event, encoded, result) in pending {
            let wires = stood.wires.get(&edge).cloned().unwrap_or_default();
            self.send_to_cell(edge, inlet, wires, &event, &encoded, &result);
        }
    }

    async fn fold_cell_arrival(&mut self, revision: RevisionEpochId) -> Next {
        if let Some(plane) = self.cells.as_mut() {
            let before: Vec<_> = plane
                .lifecycle
                .minted()
                .map(|(key, at)| (key.clone(), at.clone()))
                .collect();
            if let Some(declaration) = plane.declarations.remove(&revision)
                && let Err(error) = plane.lifecycle.declare(revision, Some(&declaration))
            {
                return self.stop(&error.to_string());
            }
            plane.lifecycle.arrive(revision);
            let ended: Vec<_> = before
                .into_iter()
                .filter(|(key, at)| plane.lifecycle.get(key) != Some(at))
                .collect();
            self.forget_cells(&ended);
            if let Next::Stop = self.retire_cells(ended).await {
                return Next::Stop;
            }
        }
        Next::Continue
    }

    async fn lifecycle_turn(&mut self, arrival: Recorded) -> Next {
        let index = arrival.index;
        let revision = arrival.at.revision();
        self.turn_revision = revision;
        self.inlets.begin_life(revision);
        self.outlets.begin_life(revision);
        if let Some(plane) = self.cells.as_mut() {
            while let Some((&at, _)) = plane.staged.first_key_value() {
                if at > revision {
                    break;
                }
                plane.prototype = plane.staged.remove(&at).expect("a staged prototype");
            }
        }
        self.activate_outlets(revision);
        let flagged: Vec<RevisionEpochId> = self
            .staged_flags
            .range(..=revision)
            .map(|(r, _)| *r)
            .collect();
        if let Some(latest) = flagged.last() {
            self.flags = self.staged_flags[latest];
        }
        for stale in flagged {
            self.staged_flags.remove(&stale);
        }
        if let Next::Stop = self.fold_cell_arrival(revision).await {
            return Next::Stop;
        }
        let started = self.starts.remove(&index);
        let Some(incarnation) = self.prepared.remove(&index) else {
            if !started {
                return Next::Continue;
            }
            self.write_transition(
                |incarnation| circular_runtime::IncarnationTransition::Activated { incarnation },
                revision,
            )
            .await;
            if self.fault.is_some() {
                return Next::Stop;
            }
            return self.opened(arrival, true).await;
        };
        let Incarnation { behavior, replace } = *incarnation;
        if let Some(closing) = self.closing.as_mut() {
            closing.next = Box::new(behavior);
            closing.revision = revision;
            return Next::Continue;
        }
        if !replace
            && !self.poisoned
            && behavior.actor_type == self.behavior.actor_type
            && behavior.config == self.behavior.config
        {
            return if started {
                self.opened(arrival, false).await
            } else {
                Next::Continue
            };
        }
        if !self.poisoned {
            let (next, effects) = self
                .lifecycle_effects(&arrival, ActorLifecycle::Closing)
                .await;
            if let Next::Stop = next {
                return Next::Stop;
            }
            if !effects.is_empty() {
                self.closing = Some(Closing {
                    effects,
                    next: Box::new(behavior),
                    revision,
                });
                return Next::Continue;
            }
        }
        let instance = match self.stand(behavior, replace, revision).await {
            Ok(instance) => instance,
            Err(()) => return Next::Stop,
        };
        self.opened(arrival, instance).await
    }

    fn activate_outlets(&mut self, revision: RevisionEpochId) {
        let staged: Vec<RevisionEpochId> =
            self.staged.range(..=revision).map(|(r, _)| *r).collect();
        for at in staged {
            let wires = self.staged.remove(&at).expect("a staged revision");
            self.outlets.install(at, wires);
        }
    }

    async fn stand(
        &mut self,
        behavior: super::turn::Behavior,
        fresh: bool,
        revision: RevisionEpochId,
    ) -> Result<bool, ()> {
        let mut behavior = behavior;
        let fresh = fresh || self.poisoned;
        let absorbed = !fresh && self.behavior.absorbs(&behavior.config);
        let incarnation = if absorbed {
            &self.behavior.incarnation
        } else {
            &behavior.incarnation
        }
        .generations()
        .last()
        .map_or(0, |generation| generation.get());
        let transition = if fresh {
            circular_runtime::IncarnationTransition::Activated { incarnation }
        } else {
            circular_runtime::IncarnationTransition::ConfigApplied { incarnation }
        };
        self.record_incarnation(transition, incarnation, revision)
            .await;
        if self.fault.is_some() {
            return Err(());
        }
        if fresh {
            self.firing.clear();
        } else {
            if absorbed {
                std::mem::swap(&mut behavior.actor, &mut self.behavior.actor);
                behavior.incarnation = self.behavior.incarnation.clone();
            } else if let Err(reason) = self.behavior.hand_over(&mut behavior) {
                eprintln!(
                    "circular-kernel: {:?} stood its new declaration without its state: {reason}",
                    self.actor
                );
            }
            behavior.timers = std::mem::take(&mut self.behavior.timers);
            if let (Some(next), Some(before)) =
                (behavior.effects.as_mut(), self.behavior.effects.take())
            {
                next.adopt(before, self.port.is_live());
            }
        }
        self.behavior = behavior;
        self.poisoned = false;
        self.bind_source();
        self.restand_source();
        Ok(!absorbed)
    }

    async fn opened(&mut self, arrival: Recorded, instance: bool) -> Next {
        if instance {
            let (next, _) = self
                .lifecycle_effects(&arrival, ActorLifecycle::Opened)
                .await;
            if let Next::Stop = next {
                return Next::Stop;
            }
        }
        self.start_pulse(arrival).await
    }

    async fn lifecycle_effects(
        &mut self,
        arrival: &Recorded,
        life: ActorLifecycle,
    ) -> (Next, Vec<circular_runtime::EffectId>) {
        let effects = match self.behavior.on_lifecycle(life) {
            Hooked::Effects(effects) => effects,
            Hooked::Panicked => {
                self.poisoned = true;
                self.poisoned_health(arrival.observed_at);
                return (Next::Continue, Vec::new());
            }
        };
        self.gate = gate_of(self.flags);
        let occasion = circular_runtime::EffectOccasion::Delivery(
            Some(circular_plan::EdgeId::outcome(self.actor.clone())),
            arrival.at.clone(),
        );
        self.apply_effects_issuing(arrival, effects, occasion).await
    }

    async fn start_pulse(&mut self, lifecycle: Recorded) -> Next {
        let Some(inlet) = circular_actors::start_inlet(self.behavior.actor_type) else {
            return Next::Continue;
        };
        let Ok(inlet) = PortId::try_new(inlet) else {
            return self.stop("a registered start inlet is not a port name");
        };
        let pulse = Recorded {
            inlet,
            route: None,
            ..lifecycle
        };
        Box::pin(self.turn_inner(pulse)).await
    }

    fn emit(
        &mut self,
        arrival: &Recorded,
        port: &PortId,
        payload: ProductPayload,
        result: &EnvelopeResult,
    ) -> Next {
        let class = if matches!(result, EnvelopeResult::Err { .. }) {
            circular_runtime::EmissionClass::Error
        } else {
            circular_runtime::EmissionClass::Ordinary
        };
        if matches!(
            self.gate.decide(class),
            circular_runtime::EmissionDisposition::Discarded(_)
        ) {
            return Next::Continue;
        }
        let revision =
            self.issuer
                .emission_revision(&arrival.at, arrival.event.stamp(), self.turn_revision);
        let Some(wires) = self.outlets.wires(port, revision) else {
            return self.stop(&format!(
                "an emission has no outlet share: port {port:?}, revision {:?}",
                revision
            ));
        };
        let encoded = match crate::product_arrival_journal::encode_event_payload(&payload) {
            Ok(encoded) => encoded,
            Err(error) => return self.stop(&format!("emission encoding: {error}")),
        };
        let wires = wires.to_vec();
        let stamp =
            match self
                .issuer
                .emission(&arrival.at, arrival.event.stamp(), self.turn_revision)
            {
                Ok(stamp) => stamp,
                Err(error) => {
                    self.poison(format!("emission stamp: {error:?}"));
                    return Next::Continue;
                }
            };
        let event: ProductEvent =
            match super::turn::emission_event(&self.stream, &arrival.event, payload, stamp) {
                Ok(event) => event,
                Err(error) => {
                    self.poison(format!("emission event: {error:?}"));
                    return Next::Continue;
                }
            };
        let sequence = event.stamp().sequence();
        for wire in wires.iter().filter(|wire| wire.credit.is_some()) {
            self.sent.insert(wire.edge.clone(), sequence);
        }
        if !self.port.is_live() {
            for wire in wires {
                if wire.credit.is_some() && self.settled.pending(&wire.edge, sequence) {
                    self.outlets.resend(wire, &event, &encoded, result);
                }
            }
            return Next::Continue;
        }
        let journal = self.journal().expect("a live actor owns its writer hand");
        let emission = circular_store::EmissionFact {
            port: port.clone(),
            cause: arrival.index,
            observed_at: arrival.observed_at,
            payload: encoded.clone(),
        };
        if let Err(error) = journal.submit_emission_body(event.stamp().clone(), emission) {
            return self.gone(&format!("emission body was not submitted: {error}"));
        }
        let gone = self
            .outlets
            .emit(wires, &event, &encoded, result, self.door.paused());
        self.destinations_gone(gone);
        Next::Continue
    }

    fn destinations_gone(&mut self, gone: Vec<Gone>) {
        for Gone(delivery) in gone {
            let record = DeadLetterRecord::new(
                delivery.event.payload().clone(),
                DeadLetterOrigin::new(self.actor.as_actor_id(), None),
                DeadLetterReason::DestinationGone,
                delivery.event.stamp().clone(),
            )
            .with_target(Some(circular_runtime::DeadLetterTarget::Delivery(
                delivery.edge.clone(),
            )));
            self.dead_letter(record);
        }
    }

    fn shed(&mut self, delivery: Delivery, reason: DeadLetterReason) {
        let record = DeadLetterRecord::new(
            delivery.event.payload().clone(),
            DeadLetterOrigin::new(self.actor.as_actor_id(), Some(delivery.inlet.clone())),
            reason,
            delivery.event.stamp().clone(),
        )
        .with_target(Some(circular_runtime::DeadLetterTarget::Delivery(
            delivery.edge.clone(),
        )));
        self.dead_letter(record);
    }

    fn dead_letter(&mut self, record: DeadLetterRecord<ActorId, ProductPayload>) {
        if !self.port.is_live() {
            return;
        }
        match self.issuer.observation(self.now, self.revision) {
            Ok(at) => self.write_dead_letter(&record, at),
            Err(error) => self.poison(format!("dead letter stamp: {error:?}")),
        }
    }

    fn write_dead_letter(
        &mut self,
        record: &DeadLetterRecord<ActorId, ProductPayload>,
        at: Stamp<ActorId>,
    ) {
        let incarnation = self
            .behavior
            .incarnation
            .generations()
            .last()
            .map_or(0, |generation| generation.get());
        let written = crate::dead_letter_writer::product_dead_letter_record(
            self.actor.scope(),
            record,
            at,
            circular_store::RecordOrigin::Actor(circular_store::IncarnationId::new(incarnation)),
        );
        match written {
            Ok(record) => self.write(vec![record]),
            Err(error) => {
                self.poison(format!("dead letter was not recorded: {error}"));
            }
        }
    }

    fn tally(&mut self, arrival: &Recorded) {
        if !self.port.is_live() {
            return;
        }
        let bucket = circular_runtime::NonZeroMillis::new(DISPLAY_BUCKET_MILLIS)
            .expect("the display bucket is nonzero");
        let coordinate = crate::display_writer::DisplayCoordinate::at(
            circular_core::BuiltinObservationName::Tally,
            self.actor.clone(),
            bucket,
            arrival.observed_at,
        );
        let Ok(sequence) = circular_core::Sequence::new(arrival.index.get().saturating_add(1))
        else {
            return;
        };
        let last = Stamp::from_event_producer(
            Tick::new(arrival.observed_at.millis()),
            self.actor.clone(),
            sequence,
            arrival.at.revision(),
        );
        if let Some(window) = self.display.as_mut()
            && window.coordinate == coordinate
        {
            window.count += 1;
            window.last = last;
            return;
        }
        self.close_display();
        let end = Tick::new(
            coordinate
                .window()
                .saturating_add(1)
                .saturating_mul(DISPLAY_BUCKET_MILLIS),
        );
        self.display = Some(DisplayWindow {
            coordinate,
            count: 1,
            last,
            end,
        });
    }

    fn close_display(&mut self) {
        let Some(window) = self.display.take() else {
            return;
        };
        let mut places = vec![circular_core::Value::int(window.count)];
        places.extend(self.behavior.observed_slots().iter().cloned());
        let bytes = match circular_core::encode(
            &circular_core::Value::array(places),
            circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
        ) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.poison(format!("display window does not encode: {error:?}"));
                return;
            }
        };
        let key = circular_store::DisplayKey::new(
            window.coordinate.name(),
            self.actor.clone(),
            Some(window.coordinate.bucket()),
        );
        let record = crate::display_writer::project_display::<circular_store::ProductStore, _>(
            window.last,
            key,
            &(),
            |()| bytes,
        );
        self.write(vec![record]);
    }

    fn write_health(&mut self, entries: Vec<crate::actor_records::HealthEntry>) {
        if !self.port.is_live() || entries.is_empty() {
            return;
        }
        let mut records = Vec::with_capacity(entries.len());
        for entry in entries {
            let written = self
                .issuer
                .observation(self.now, self.revision)
                .map_err(|error| format!("health stamp: {error:?}"))
                .and_then(|at| {
                    let fact = crate::actor_records::HealthFact {
                        actor: self.actor.clone(),
                        state: entry.state,
                        reason: entry.reason,
                        since: entry.since,
                        mailbox_depths: entry.mailbox_depths,
                        at: at.clone(),
                    };
                    crate::actor_records::actor_health_record(
                        at,
                        &crate::actor_records::health_transition(&fact)?,
                    )
                });
            match written {
                Ok(record) => records.push(record),
                Err(error) => {
                    self.poison(format!("actor health was not recorded: {error}"));
                    return;
                }
            }
        }
        self.write(records);
    }

    async fn write_transition(
        &mut self,
        transition: impl FnOnce(u64) -> circular_runtime::IncarnationTransition,
        revision: RevisionEpochId,
    ) {
        let incarnation = self
            .behavior
            .incarnation
            .generations()
            .last()
            .map_or(0, |generation| generation.get());
        self.record_incarnation(transition(incarnation), incarnation, revision)
            .await;
    }

    async fn record_incarnation(
        &mut self,
        transition: circular_runtime::IncarnationTransition,
        incarnation: u64,
        revision: RevisionEpochId,
    ) {
        if !self.port.is_live() {
            let Some(plane) = self.cells.as_mut() else {
                return;
            };
            let Some(record) = plane.recorded.front() else {
                return;
            };
            match crate::incarnation_transition::standing_incarnation(record) {
                Ok(Some((_, recorded)))
                    if recorded == incarnation && record.header().at().revision() == revision => {}
                Ok(_) => return,
                Err(reason) => {
                    self.poison(reason);
                    return;
                }
            }
            let record = plane
                .recorded
                .pop_front()
                .expect("the recorded incarnation");
            match self.fold_cell_record(&record) {
                Ok(ended) => {
                    self.retire_cells(ended).await;
                }
                Err(reason) => self.poison(reason),
            }
            return;
        }
        let written = self
            .issuer
            .observation(self.now, revision)
            .map_err(|error| format!("transition stamp: {error:?}"))
            .and_then(|at| {
                crate::incarnation_transition::incarnation_transition_record(
                    &self.actor,
                    incarnation,
                    revision,
                    transition,
                    at,
                    circular_store::OpaqueId::new(0),
                )
            });
        match written {
            Ok(record) => {
                self.write(vec![record.clone()]);
                if self.cells.is_some() {
                    self.settle().await;
                    if self.fault.is_some() {
                        return;
                    }
                    match self.fold_cell_record(&record) {
                        Ok(ended) => {
                            self.retire_cells(ended).await;
                        }
                        Err(reason) => self.poison(reason),
                    }
                }
            }
            Err(error) => {
                self.poison(format!("incarnation transition was not recorded: {error}"));
            }
        }
    }

    fn poisoned_health(&mut self, at: RecordedInstant) {
        if let Some(reason) =
            crate::actor_records::dead_letter_actor_failure(&DeadLetterReason::Poisoned)
        {
            self.health.failed(reason, at);
        }
    }

    fn observe_pressure(&mut self, edge: &circular_plan::EdgeId, shed: bool) {
        let Some((depth, capacity)) = self.inlets.pressure(edge) else {
            return;
        };
        let entries = self
            .health
            .pressure(vec![crate::mailbox_pressure::MailboxPressure {
                actor: self.actor.clone(),
                edge: edge.clone(),
                depth,
                capacity,
                overflow: shed || depth >= capacity,
                observed_at: self.sample(),
            }]);
        self.write_health(entries);
    }

    fn write(&mut self, records: Vec<circular_store::Record<circular_store::ProductStore>>) {
        self.write_bodies(records, Vec::new());
    }

    fn write_bodies(
        &mut self,
        records: Vec<circular_store::Record<circular_store::ProductStore>>,
        live_bodies: Vec<(usize, circular_core::EncodedPayload)>,
    ) {
        let Some(journal) = self.journal() else {
            return;
        };
        self.writes
            .push_back(journal.submit_records(records, live_bodies));
    }

    fn on_written(
        &mut self,
        written: Result<Result<(), String>, oneshot::error::RecvError>,
    ) -> Next {
        match written {
            Ok(Ok(())) => Next::Continue,
            Ok(Err(error)) if self.port.recorder_alive() => {
                self.stop(&format!("a record was not written: {error}"))
            }
            Ok(Err(error)) => self.gone(&format!("a record was not written: {error}")),
            Err(_) => self.gone("the journal writer is gone"),
        }
    }

    fn admit_held(&mut self) {
        let observed_at = self.sample();
        let record = match self.inlets.admit_last() {
            Some(delivery) => admission(self.stream, &self.actor, delivery, observed_at)
                .map(|record| (record, delivery.encoded.clone())),
            None => return,
        };
        match record {
            Ok((record, encoded)) => self.write_bodies(vec![record], vec![(0, encoded)]),
            Err(error) => {
                self.poison(format!("admission was not recorded: {error:?}"));
            }
        }
    }

    async fn settle(&mut self) {
        if let Some(flying) = self.flying.as_mut() {
            let answer = (&mut flying.answer).await;
            if let Next::Stop = self.on_recorded(answer).await {
                return;
            }
        }
        while let Some(written) = self.writes.pop_front() {
            if let Next::Stop = self.on_written(written.await) {
                return;
            }
        }
    }

    fn admit_unrecorded(&mut self, stopping: bool) {
        let mut records = Vec::new();
        let mut live_bodies = Vec::new();
        let mut failed = None;
        let (stream, actor, observed_at) = (self.stream, self.actor.clone(), self.sample());
        self.inlets.admit_unrecorded(stopping, |delivery| {
            match admission(stream, &actor, delivery, observed_at) {
                Ok(record) => {
                    live_bodies.push((records.len(), delivery.encoded.clone()));
                    records.push(record);
                }
                Err(error) => failed = Some(format!("{error:?}")),
            }
        });
        if let Some(error) = failed {
            self.fault = Some(format!("admission was not recorded: {error}"));
        }
        if !records.is_empty() {
            self.write_bodies(records, live_bodies);
        }
    }

    /// Only replayable, actor-owned state crosses this disposable boundary.
    async fn snapshot(&mut self, clean: bool) {
        if self.cache.is_none()
            || !self.port.is_live()
            || self.poisoned
            || self.fault.is_some()
            || self.cells.is_some()
            || self.cell_minted.is_some()
            || self.outlets.blocked()
            || self.closing.is_some()
            || !self.lifecycle.is_empty()
            || !self.prepared.is_empty()
            || !self.staged.is_empty()
            || !self.staged_flags.is_empty()
            || self.revision != self.turn_revision
            || !self.behavior.timers.is_empty()
            || !self.firing.is_empty()
            || !self.outcomes.is_empty()
            || self
                .behavior
                .effects
                .as_ref()
                .is_some_and(|effects| !effects.idle())
        {
            return;
        }
        let Some((index, previous)) = self.last_turn.as_ref() else {
            return;
        };
        let state = match self.behavior.state() {
            Ok(state) => state,
            Err(reason) => {
                super::snapshot::miss(&self.actor, reason);
                return;
            }
        };
        let (emitted, emission_clock) = self.issuer.emissions();
        let snapshot = super::snapshot::Snapshot {
            stream: self.stream,
            index: *index,
            previous: previous.clone(),
            revision: self.turn_revision,
            generation: self
                .behavior
                .incarnation
                .generations()
                .last()
                .map_or(0, |g| g.get()),
            emitted,
            emission_clock,
            state,
            sent: self
                .sent
                .iter()
                .map(|(edge, sequence)| (edge.clone(), *sequence))
                .collect(),
        };
        self.cache
            .as_mut()
            .expect("writer present")
            .write(snapshot, clean)
            .await;
    }

    async fn stop_here(&mut self) {
        self.stop_source();
        self.close_display();
        self.settle().await;
        self.admit_unrecorded(true);
        self.record_stop().await;
        self.snapshot(true).await;
    }

    async fn record_boundary(
        &mut self,
        life: impl FnOnce(circular_core::ArrivalIndex) -> Life,
    ) -> Next {
        loop {
            self.settle().await;
            if self.fault.is_some() {
                return Next::Stop;
            }
            let Some(now) = self.time.query_tick().await else {
                return self.gone("the time actor stopped answering");
            };
            if self.lifecycle.is_empty() {
                let consumed = self.inlets.first_unconsumed(self.issuer.next_index());
                self.lifecycle.push_back((self.revision, life(consumed)));
                self.fly(now);
                self.settle().await;
                return if self.fault.is_some() {
                    Next::Stop
                } else {
                    Next::Continue
                };
            }
            self.fly(now);
        }
    }

    async fn record_pause(&mut self, pause: Option<bool>) -> Next {
        loop {
            self.settle().await;
            if self.fault.is_some() {
                return Next::Stop;
            }
            if self.lifecycle.is_empty() {
                break;
            }
            let Some(now) = self.time.query_tick().await else {
                return self.gone("the time actor stopped answering");
            };
            self.fly(now);
        }
        if self.applied_pause == pause {
            return Next::Continue;
        }
        self.record_boundary(|consumed| match pause {
            Some(force) => Life::Pause { force, consumed },
            None => Life::Resume,
        })
        .await
    }

    fn apply_pause(&mut self, pause: Option<bool>) {
        self.applied_pause = pause;
        self.door.set(pause.is_some());
        if let Some(port) = self.behavior.effects.as_mut() {
            match pause {
                Some(force) => port.pause(force),
                None => port.resume(),
            }
        }
        self.harness_health(false);
        if let Some(worker) = &self.source {
            worker.pause(pause.is_some());
        }
    }

    async fn record_stop(&mut self) {
        for injection in self.injections.drain(..) {
            let _ = injection.reply.send(Err(Refusal::NotAccepting));
        }
        let _ = self.record_boundary(Life::Stop).await;
    }

    async fn retire(&mut self, queued: Vec<Delivery>) {
        self.stop_source();
        self.close_display();
        self.settle().await;
        let unrecorded = self.inlets.drain_unrecorded();
        self.gone_here(unrecorded.into_iter().chain(queued));
        self.record_stop().await;
        if self.fault.is_none() {
            let _ = self.close_cells().await;
        }
        if let Some(stop) = self.stopped.take()
            && self.fault.is_none()
            && !self.poisoned
        {
            let (next, mut effects) = self.lifecycle_effects(&stop, ActorLifecycle::Closing).await;
            if let Some(closing) = self.closing.take() {
                effects.extend(closing.effects);
            }
            if let Next::Continue = next {
                self.record_closing(effects).await;
            }
        }
        self.record_outcomes().await;
        self.write_transition(
            |incarnation| circular_runtime::IncarnationTransition::Terminated { incarnation },
            self.revision,
        )
        .await;
        let entries = self.health.lifecycle(
            circular_protocol::actor_events::ActorHealthState::Stopped,
            None,
            self.sample(),
        );
        self.write_health(entries);
        self.settle().await;
    }

    async fn record_closing(&mut self, mut effects: Vec<circular_runtime::EffectId>) {
        while !effects.is_empty() {
            self.settle().await;
            if self.fault.is_some() {
                return;
            }
            if let Some(position) = self
                .outcomes
                .iter()
                .position(|outcome| effects.contains(outcome.correlation()))
            {
                let outcome = self.outcomes.remove(position).expect("a found position");
                effects.retain(|effect| effect != outcome.correlation());
                let Some(now) = self.time.query_tick().await else {
                    self.fault = Some("the time actor stopped answering".to_owned());
                    return;
                };
                self.fly_outcome(now, outcome);
                continue;
            }
            let Some(port) = self.behavior.effects.as_mut() else {
                return;
            };
            if !port.awaiting() {
                return;
            }
            let outcome = std::future::poll_fn(|context| port.poll_outcome(context)).await;
            self.outcomes.push_back(outcome);
        }
        self.settle().await;
    }

    fn gone_here(&mut self, deliveries: impl IntoIterator<Item = Delivery>) {
        for delivery in deliveries {
            let record = DeadLetterRecord::new(
                delivery.event.payload().clone(),
                DeadLetterOrigin::new(self.actor.as_actor_id(), Some(delivery.inlet.clone())),
                DeadLetterReason::DestinationGone,
                delivery.event.stamp().clone(),
            )
            .with_target(Some(circular_runtime::DeadLetterTarget::Delivery(
                delivery.edge.clone(),
            )));
            self.dead_letter(record);
        }
    }

    async fn record_outcomes(&mut self) {
        self.settle().await;
        while let Some(outcome) = self.outcomes.pop_front() {
            let Some(now) = self.time.query_tick().await else {
                return;
            };
            if self.fly_outcome(now, outcome) {
                self.settle().await;
            }
        }
    }

    fn bind_source(&mut self) {
        if matches!(self.behavior.source, Some(super::source::Plan::Listener(_))) {
            let Some(super::source::Plan::Listener(plan)) = self.behavior.source.take() else {
                unreachable!("the listener plan was just matched")
            };
            self.behavior
                .effects
                .as_mut()
                .expect("listener declares FileRead")
                .bind_listener(plan, self.own.clone(), self.door.paused());
        }
    }

    fn start_source(&mut self) {
        let Some(journal) = self.journal() else {
            return;
        };
        if self.door.paused() || self.source.is_some() || self.restarting.is_some() {
            return;
        }
        let Some(super::source::Plan::Otlp(plan)) = self.behavior.source.as_ref() else {
            return;
        };
        match plan.start(
            &self.actor,
            self.own.clone(),
            journal.source_custody_root(),
            self.stream,
        ) {
            Ok(worker) => {
                if self.door.paused() {
                    worker.pause(true);
                }
                self.source = Some(worker);
            }
            Err(failure) => self.fell(&failure),
        }
    }

    fn restand_source(&mut self) {
        if !self.port.is_live() {
            return;
        }
        match self.source.take() {
            Some(worker) => {
                self.restarting = Some(tokio::task::spawn_blocking(worker.stop()));
            }
            None => self.start_source(),
        }
    }

    fn stop_source(&mut self) {
        if let Some(worker) = self.source.take() {
            let stop = worker.stop();
            std::thread::spawn(stop);
        }
        self.restarting = None;
    }

    fn refused(&mut self, refused: super::source::Refused) {
        if !self.port.is_live() {
            return;
        }
        let at = match self.issuer.observation(self.now, self.revision) {
            Ok(at) => at,
            Err(error) => {
                self.poison(format!("refusal stamp: {error:?}"));
                return;
            }
        };
        let record = DeadLetterRecord::new(
            refused.subject,
            DeadLetterOrigin::new(self.actor.as_actor_id(), refused.inlet),
            refused.reason,
            at.clone(),
        );
        self.write_dead_letter(&record, at);
    }

    fn fell(&mut self, failure: &crate::activation_detail::RegistrationFailure) {
        let reason = circular_protocol::actor_events::ActorHealthReason {
            code: circular_protocol::actor_events::ActorHealthReasonCode::SourceFailure,
            detail: failure.detail().to_value(),
        };
        let entries = self.health.lifecycle(
            circular_protocol::actor_events::ActorHealthState::Failed,
            Some(reason),
            self.sample(),
        );
        self.write_health(entries);
    }

    async fn refuse_recovery(&mut self, reason: &str) {
        eprintln!(
            "circular-kernel: {:?} was not restored: {reason}",
            self.actor
        );
        self.faulted = false;
        if let Some(now) = self.time.query_tick().await {
            self.now = now;
        }
        let entries = self.health.lifecycle(
            circular_protocol::actor_events::ActorHealthState::Failed,
            Some(circular_protocol::actor_events::ActorHealthReason {
                code: circular_protocol::actor_events::ActorHealthReasonCode::RecoveryRefused,
                detail: circular_core::Value::Null,
            }),
            self.sample(),
        );
        self.write_health(entries);
        self.settle().await;
    }

    fn sample(&self) -> RecordedInstant {
        let per_second = u128::from(self.time.ticks_per_second().max(1));
        let millis = u128::from(self.now.get()) * 1_000 / per_second;
        RecordedInstant::from_millis(u64::try_from(millis).unwrap_or(u64::MAX))
    }

    fn stop(&mut self, reason: &str) -> Next {
        eprintln!("circular-kernel: {:?} stopped: {reason}", self.actor);
        self.fault = Some(reason.to_owned());
        self.faulted = true;
        Next::Stop
    }

    fn gone(&mut self, reason: &str) -> Next {
        eprintln!("circular-kernel: {:?} ended: {reason}", self.actor);
        self.fault = Some(reason.to_owned());
        Next::Stop
    }

    async fn record_fault(&mut self) {
        if let Some(now) = self.time.query_tick().await {
            self.now = now;
        }
        let entries = self.health.lifecycle(
            circular_protocol::actor_events::ActorHealthState::Failed,
            Some(circular_protocol::actor_events::ActorHealthReason {
                code: circular_protocol::actor_events::ActorHealthReasonCode::KernelFault,
                detail: circular_core::Value::Null,
            }),
            self.sample(),
        );
        self.write_health(entries);
        while let Some(written) = self.writes.pop_front() {
            let _ = written.await;
        }
    }

    fn poison(&mut self, fault: String) {
        eprintln!(
            "circular-kernel: {:?} stopped consuming: {fault}",
            self.actor
        );
        self.fault = Some(fault);
        self.poisoned = true;
    }
}

fn admission(
    stream: StreamId,
    actor: &NamedActorId,
    delivery: &Delivery,
    observed_at: RecordedInstant,
) -> Result<
    circular_store::Record<circular_store::ProductStore>,
    crate::arrival_commit::ArrivalCommitError,
> {
    let parents: Vec<Stamp<ActorId>> = delivery
        .event
        .causality()
        .parents()
        .iter()
        .map(circular_core::EventId::stamp)
        .cloned()
        .collect();
    crate::product_arrival_journal::admission_record(
        stream,
        actor,
        &delivery.inlet,
        &delivery.edge,
        delivery.event.stamp(),
        &parents,
        circular_store::ArrivalBody::emitted(delivery.event.stamp()),
        &delivery.result,
        observed_at,
    )
}

fn bypass_ports(actor_type: circular_core::ActorType) -> Option<(PortId, PortId)> {
    use circular_actors::{Side, StaticPortRequest, resolve_static_port};
    let spec = circular_actors::get(actor_type);
    let inlet = resolve_static_port(spec, StaticPortRequest::Default { side: Side::Inlet })
        .ok()
        .flatten()?;
    let outlet = resolve_static_port(spec, StaticPortRequest::Default { side: Side::Outlet })
        .ok()
        .flatten()?;
    (inlet.ty() == outlet.ty()).then(|| (inlet.id().clone(), outlet.id().clone()))
}

fn gate_of(flags: circular_plan::ActorFlags) -> circular_runtime::EmissionGate {
    if flags.mute() {
        circular_runtime::EmissionGate::ErrorOnly
    } else {
        circular_runtime::EmissionGate::All
    }
}

fn timer_inlet() -> PortId {
    PortId::try_derived(circular_actors::TIMER_PORT_NAME.to_owned())
        .expect("reserved timer inlet is canonical")
}

pub(crate) fn outcome_inlet() -> PortId {
    PortId::try_derived("_outcome".to_owned()).expect("reserved outcome inlet is canonical")
}

