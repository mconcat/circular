
use crate::daemon::declaration::malformed;
use crate::daemon::ledger;
use crate::daemon::ledger::ReadBound;
use crate::daemon::replay_clock::{Answer, SystemTime};
use crate::daemon::session::{REPLAY_RESULT, answer_with};
use circular_core::{Boundary, Ceilings, RecordedInstant, Tick};
use circular_protocol::EnvelopeHeader;
use circular_protocol::Partition;
use circular_protocol::declaration_payload::{Accepted, CommandResult};
use circular_protocol::rejection_code::RejectionReason;
use circular_protocol::replay_payload::{
    Arrangement, Pace, ReplayTarget, decode_replay_rewind, decode_replay_start,
};
use circular_protocol::{ReplayControlVerb, StableVerb};
use std::sync::{Arc, mpsc};

#[derive(Default)]
pub(crate) struct Lenses(std::collections::BTreeMap<u32, ReplayLens>);

impl Lenses {
    pub(crate) fn contains(&self, key: u32) -> bool {
        self.0.contains_key(&key)
    }

    pub(crate) fn get(&self, key: u32) -> Option<&ReplayLens> {
        self.0.get(&key)
    }

    pub(crate) fn get_mut(&mut self, key: u32) -> Option<&mut ReplayLens> {
        self.0.get_mut(&key)
    }

    pub(crate) fn open(&mut self, key: u32, lens: ReplayLens) {
        self.0.insert(key, lens);
    }

    pub(crate) fn close(&mut self, key: u32) -> Option<ReplayLens> {
        self.0.remove(&key)
    }

    pub(crate) fn named(&self, key: Option<u32>) -> Result<Option<&ReplayLens>, String> {
        match key {
            None => Ok(None),
            Some(key) => self.get(key).map(Some).ok_or_else(|| {
                format!("this read names replay lens {key}, and that correlation holds no open replay lens")
            }),
        }
    }

    pub(crate) fn drain(
        &mut self,
        server: Option<&ledger::ServerRead>,
        time: &SystemTime,
        wake: &Arc<engine::wake::Wake>,
    ) {
        for lens in self.0.values_mut() {
            lens.drain(server, time, wake);
        }
    }
}

pub(crate) struct ReplayLens {
    stream: u64,
    position: ReadBound,
    horizon: ReadBound,
    pace: Pace,
    generation: u64,
    pacing: Option<Pacing>,
    replies: mpsc::Sender<Answer>,
    answers: mpsc::Receiver<Answer>,
}

struct Pacing {
    num: u64,
    den: u64,
    wall: Option<Tick>,
    recorded: RecordedInstant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FeedReset {
    Reset,
    Keep,
}

impl ReplayLens {
    pub(crate) const fn stream(&self) -> u64 {
        self.stream
    }

    pub(crate) const fn position(&self) -> &ReadBound {
        &self.position
    }

    #[cfg(test)]
    pub(crate) fn held_for_tests(stream: u64) -> Self {
        let (replies, answers) = mpsc::channel();
        Self {
            stream,
            position: ReadBound::at(0),
            horizon: ReadBound::at(0),
            pace: Pace::Paused,
            generation: 0,
            pacing: None,
            replies,
            answers,
        }
    }

    pub(crate) fn drain(
        &mut self,
        server: Option<&ledger::ServerRead>,
        time: &SystemTime,
        wake: &Arc<engine::wake::Wake>,
    ) {
        while let Ok(answer) = self.answers.try_recv() {
            if answer.generation != self.generation {
                continue;
            }
            let Some(now) = answer.now else {
                eprintln!("circular-daemon: replay clock — the time actor is gone; the lens holds");
                self.pacing = None;
                continue;
            };
            let Some(server) = server.filter(|standing| standing.stream().get() == self.stream)
            else {
                self.pacing = None;
                continue;
            };
            if let Err(reason) = self.step(server, now, time, wake) {
                eprintln!("circular-daemon: replay clock — {reason}");
                self.pacing = None;
            }
        }
    }

    fn step(
        &mut self,
        server: &ledger::ServerRead,
        now: Tick,
        time: &SystemTime,
        wake: &Arc<engine::wake::Wake>,
    ) -> Result<(), String> {
        let Some(pacing) = self.pacing.as_mut() else {
            return Ok(());
        };
        let wall = *pacing.wall.get_or_insert(now);
        let elapsed = u128::from(now.get().saturating_sub(wall.get()));
        let recorded = elapsed * u128::from(pacing.num) / u128::from(pacing.den);
        let until = RecordedInstant::from_millis(
            pacing
                .recorded
                .millis()
                .saturating_add(u64::try_from(recorded).unwrap_or(u64::MAX)),
        );
        self.position = server.advance_bound(&self.position, &self.horizon, until)?;
        let Some((_, next)) = server.next_arrival_after(&self.position, &self.horizon)? else {
            self.position = self.horizon.clone();
            self.pacing = None;
            self.pace = Pace::Paused;
            return Ok(());
        };
        let gap = u128::from(next.millis().saturating_sub(pacing.recorded.millis()));
        let wait = (gap * u128::from(pacing.den)).div_ceil(u128::from(pacing.num));
        let at = Tick::new(
            wall.get()
                .saturating_add(u64::try_from(wait).unwrap_or(u64::MAX)),
        );
        if !time.register(at, self.generation, self.replies.clone(), wake.clone()) {
            return Err("the time actor refused the deadline".to_owned());
        }
        Ok(())
    }

    fn apply_pace(
        &mut self,
        server: &ledger::ServerRead,
        pace: Pace,
        time: &SystemTime,
        wake: &Arc<engine::wake::Wake>,
    ) -> Result<(), CommandResult> {
        self.generation = self.generation.wrapping_add(1);
        self.pacing = None;
        match &pace {
            Pace::Free => self.position = self.horizon.clone(),
            Pace::Paused => {}
            Pace::Step { upto } => {
                self.position = resolve_target(server, self.stream, upto)?.within(&self.horizon);
            }
            Pace::Realtime { num, den } => {
                if *num == 0 || *den == 0 {
                    return Err(malformed(
                        "a realtime multiplier has a nonzero numerator and denominator".to_owned(),
                    ));
                }
                let recorded = match server
                    .next_arrival_after(&self.position, &self.horizon)
                    .map_err(unresolved)?
                {
                    Some((_, at)) => at,
                    None => {
                        self.pace = Pace::Paused;
                        return Ok(());
                    }
                };
                self.pacing = Some(Pacing {
                    num: *num,
                    den: *den,
                    wall: None,
                    recorded,
                });
                if !time.register(
                    Tick::new(0),
                    self.generation,
                    self.replies.clone(),
                    wake.clone(),
                ) {
                    return Err(unresolved("the time actor refused the deadline".to_owned()));
                }
            }
        }
        self.pace = pace;
        Ok(())
    }
}

fn unresolved(message: String) -> CommandResult {
    CommandResult::Rejected(RejectionReason::Unresolved.reject(Partition::ReplayControl, message))
}

fn resolve_target(
    server: &ledger::ServerRead,
    stream: u64,
    target: &ReplayTarget,
) -> Result<ReadBound, CommandResult> {
    if target.stream != stream {
        return Err(unresolved(format!(
            "the target names stream {} but this replay reads stream {stream}",
            target.stream
        )));
    }
    let Some(cut) = named_cut(&target.cut) else {
        return Err(malformed(
            "the target cut exceeds the scope depth limit".to_owned(),
        ));
    };
    let cut = server
        .replay_checkpoint_at(target.revision_epoch, cut)
        .map_err(|reason| {
            CommandResult::Rejected(
                RejectionReason::Unresolved
                    .reject(Partition::ReplayControl, reason)
                    .hint("choose a coordinate of this stream — a timeline.at answer or a timeline checkpoint".to_owned()),
            )
        })?;
    server.read_bound(&cut).map_err(unresolved)
}

pub(crate) fn answer_replay(
    stream: &mut impl circular_transport::LocalByteStream<Error = std::io::Error>,
    header: EnvelopeHeader,
    result: CommandResult,
) {
    answer_with(stream, header, REPLAY_RESULT, result);
}

pub(crate) fn replay_start(
    payload: &[u8],
    server: Option<&ledger::ServerRead>,
    time: &SystemTime,
    wake: &Arc<engine::wake::Wake>,
) -> Result<ReplayLens, CommandResult> {
    let start = decode_replay_start(payload, Ceilings::for_boundary(Boundary::Wire))
        .map_err(|rejection| malformed(format!("ReplayStart payload is malformed: {rejection}")))?;
    let from = match start.arrangement {
        Arrangement::Observational { from } => from,
        other => {
            return Err(unresolved(format!(
                "only observational replay is supported; this arrangement is outside that contract: {other:?}"
            )));
        }
    };
    let Some(server) = server.filter(|standing| standing.stream().get() == from.stream) else {
        return Err(CommandResult::Rejected(RejectionReason::Unresolved.reject(Partition::ReplayControl, format!(
                "stream {} is not the stream this daemon holds; its records are not open for reading",
                from.stream
            )).hint("choose a coordinate of the held stream — a timeline.at answer or a timeline checkpoint".to_owned())));
    };
    let origin = resolve_target(server, from.stream, &from)?;
    let (replies, answers) = mpsc::channel();
    let mut lens = ReplayLens {
        stream: from.stream,
        position: origin,
        horizon: server.record_horizon().map_err(unresolved)?,
        pace: Pace::Paused,
        generation: 0,
        pacing: None,
        replies,
        answers,
    };
    lens.apply_pace(server, start.pace, time, wake)?;
    Ok(lens)
}

pub(crate) fn replay_rewind(
    payload: &[u8],
    server: Option<&ledger::ServerRead>,
    lens: Option<&mut ReplayLens>,
    time: &SystemTime,
    wake: &Arc<engine::wake::Wake>,
) -> Result<FeedReset, CommandResult> {
    let rewind = decode_replay_rewind(payload, Ceilings::for_boundary(Boundary::Wire)).map_err(
        |rejection| malformed(format!("ReplayRewind payload is malformed: {rejection}")),
    )?;
    let Some(lens) = lens else {
        return Err(unresolved(
            "this correlation names no live replay session".to_owned(),
        ));
    };
    let Some(server) = server.filter(|standing| standing.stream().get() == lens.stream) else {
        return Err(CommandResult::Rejected(
            RejectionReason::Unresolved
                .reject(
                    Partition::ReplayControl,
                    "the stream this replay reads is no longer held".to_owned(),
                )
                .hint("end the replay and start one from a held checkpoint".to_owned()),
        ));
    };
    let before = lens.position.clone();
    if let Some(to) = &rewind.to {
        lens.position = resolve_target(server, lens.stream, to)?.within(&lens.horizon);
    }
    lens.apply_pace(server, rewind.pace, time, wake)?;
    Ok(if lens.position.precedes_somewhere(&before) {
        FeedReset::Reset
    } else {
        FeedReset::Keep
    })
}

pub(crate) fn replay_end(payload: &[u8], replay_open: bool) -> CommandResult {
    match StableVerb::ReplayControl(ReplayControlVerb::ReplayEnd).open_absent_body(payload) {
        Ok(()) => {
            if replay_open {
                CommandResult::Accepted(Accepted::Nothing)
            } else {
                unresolved("this correlation names no live replay session".to_owned())
            }
        }
        Err(rejection) => malformed(format!("{rejection:?}")),
    }
}

pub(crate) const fn accepted() -> CommandResult {
    CommandResult::Accepted(Accepted::Nothing)
}

pub(crate) fn replay_wire_cut(
    cut: &circular_runtime::LogCut,
) -> Result<circular_protocol::replay_payload::LogCut, String> {
    use circular_plan::{ActorId, LocalKey};
    use circular_protocol::replay_payload::LogCutComponent;

    let mut components = Vec::with_capacity(cut.len());
    for (actor, index) in cut.components() {
        let ActorId::Scoped { scope, local } = actor else {
            return Err("timeline cut contains a system actor".to_owned());
        };
        let LocalKey::Named(local) = local else {
            return Err("timeline cut contains an ephemeral actor".to_owned());
        };
        components.push(LogCutComponent {
            actor: circular_runtime::product_identity::wire_named_actor(
                &circular_plan::NamedActorId::new(scope.clone(), local.clone()),
            ),
            index: index.get(),
        });
    }
    Ok(circular_protocol::replay_payload::LogCut { components })
}

fn plan_cut_actor(
    actor: &circular_protocol::declaration_payload::PlanActorKey,
) -> Option<circular_plan::NamedActorId> {
    circular_runtime::product_identity::named_actor_from_wire(actor).ok()
}

fn named_cut(cut: &circular_protocol::replay_payload::LogCut) -> Option<circular_runtime::LogCut> {
    let mut components = Vec::new();
    for component in &cut.components {
        components.push((
            plan_cut_actor(&component.actor)?.as_actor_id(),
            circular_core::ArrivalIndex::new(component.index),
        ));
    }
    Some(circular_runtime::LogCut::new(components))
}

