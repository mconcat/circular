pub(crate) mod edge_depths;
pub(crate) mod records;
use crate::daemon::declaration::malformed;
use crate::daemon::ledger;
use crate::daemon::session::{SUBSCRIBE_ACK, SUBSCRIPTION_ENDED, SUBSCRIPTION_FRAME, answer_with};
use crate::daemon::subscription_catalog::{
    AUTHORING_COMMITS_TARGET, AnsweredBy, SubscriptionTarget,
};
use circular_core::{Boundary, Ceilings, Value};
use circular_protocol::Partition;
use circular_protocol::declaration_payload::{Accepted, CommandResult, decode_scope_identity};
use circular_protocol::rejection_code::RejectionReason;
use circular_protocol::subscription_payload::{
    FrameOrigin, SubscriptionEndReason, SubscriptionEnded, SubscriptionFrame,
};
#[cfg(test)]
use circular_protocol::subscription_payload::{decode_credit, decode_subscribe};
use circular_protocol::{
    CreditBalance, DeliveryDiscipline, EnvelopeHeader, INITIAL_PROTOCOL_VERSION, PositiveCredit,
    PositiveFrameCount, SubscriptionAction, SubscriptionEndReason as FsmEndReason, SubscriptionFsm,
    SubscriptionTerminationDomain,
};
use circular_transport::{LocalByteStream, OwnerLocalChannelId, write_envelope};
use engine::authoring_assembly::ledger as authoring;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FrameCredits(pub(crate) u32);

impl PositiveFrameCount for FrameCredits {
    fn is_positive(&self) -> bool {
        self.0 > 0
    }
}

impl CreditBalance for FrameCredits {
    fn zero() -> Self {
        Self(0)
    }

    fn is_zero(&self) -> bool {
        self.0 == 0
    }

    fn checked_add(&self, amount: &Self) -> Option<Self> {
        self.0.checked_add(amount.0).map(Self)
    }

    fn checked_consume_frame(&self) -> Option<Self> {
        self.0.checked_sub(1).map(Self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DaemonSubscriptionTermination;

impl SubscriptionTerminationDomain for DaemonSubscriptionTermination {
    type ResetFloorOrCursor = u64;
    type StructureCursor = u64;
    type AuthoringEnvironment = Vec<u8>;
}

type FlowFsm = SubscriptionFsm<DaemonSubscriptionTermination, FrameCredits, u64, u16>;

fn credit_flow(anchor: u64) -> FlowFsm {
    let mut flow = FlowFsm::new(DeliveryDiscipline::Credit, anchor);
    flow.opened().expect("a newly constructed flow opens once");
    flow
}

pub(crate) struct Subscription {
    pub(crate) source: SubscriptionSource,
    flow: FlowFsm,
    correlation: u32,
    lens: Option<u32>,
}

impl Subscription {
    pub(crate) const fn lens(&self) -> Option<u32> {
        self.lens
    }
}

pub(crate) enum SubscriptionSource {
    Records {
        reader: records::Records,
    },
    ActorEvents {
        next: circular_store::SurfaceSequence,
        opened: Option<ledger::ReadBound>,
    },
    DisplayFrames {
        next: circular_store::SurfaceSequence,
    },
    EdgeDepths {
        feed: edge_depths::Feed,
    },
    AuthoringCommits {
        scope: Vec<circular_protocol::declaration_payload::ScopeSegment>,
        position: AuthoringPosition,
        live_cut: u64,
        reset_floor: Option<u64>,
    },
}

#[derive(Clone)]
pub(crate) struct AuthoringPosition {
    next_index: usize,
    fold: authoring::AuthoringState,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let _ = self.flow.session_closed(0);
    }
}

#[derive(Default)]
pub(crate) struct Subscriptions(std::collections::BTreeMap<u32, Subscription>);

impl Subscriptions {
    pub(crate) fn contains(&self, correlation: u32) -> bool {
        self.0.contains_key(&correlation)
    }

    pub(crate) fn slot<R>(
        &mut self,
        correlation: u32,
        use_slot: impl FnOnce(&mut Option<Subscription>) -> R,
    ) -> R {
        let mut slot = self.0.remove(&correlation);
        let result = use_slot(&mut slot);
        if let Some(subscription) = slot {
            self.0.insert(correlation, subscription);
        }
        result
    }

    pub(crate) fn each(&mut self, mut use_slot: impl FnMut(&mut Option<Subscription>)) -> Vec<u32> {
        let correlations: Vec<u32> = self.0.keys().copied().collect();
        correlations
            .into_iter()
            .filter(|correlation| {
                self.slot(*correlation, &mut use_slot);
                !self.contains(*correlation)
            })
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn get(&self, correlation: u32) -> Option<&Subscription> {
        self.0.get(&correlation)
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }
}

pub(crate) fn answer_subscription(
    stream: &mut impl circular_transport::LocalByteStream<Error = std::io::Error>,
    header: EnvelopeHeader,
    result: CommandResult,
) {
    answer_with(stream, header, SUBSCRIBE_ACK, result);
}

#[cfg(test)]
pub(crate) fn subscribe(
    payload: &[u8],
    correlation: u32,
    server: Option<&ledger::ServerRead>,
    authoring: &authoring::AuthoringState,
    retained: &crate::daemon::authoring_store::RetainedCommits,
    live: &mut Option<Subscription>,
    lens: Option<&crate::daemon::replay::ReplayLens>,
) -> CommandResult {
    match decode_subscribe(payload, Ceilings::for_boundary(Boundary::Wire)) {
        Ok(open) => subscribe_decoded(open, correlation, server, authoring, retained, live, lens),
        Err(rejection) => malformed(format!("{rejection:?}")),
    }
}

pub(crate) fn subscribe_decoded(
    open: circular_protocol::subscription_payload::Subscribe,
    correlation: u32,
    server: Option<&ledger::ServerRead>,
    authoring: &authoring::AuthoringState,
    retained: &crate::daemon::authoring_store::RetainedCommits,
    live: &mut Option<Subscription>,
    lens: Option<&crate::daemon::replay::ReplayLens>,
) -> CommandResult {
    let Some(target) = crate::daemon::subscription_catalog::resolve(&open.target) else {
        return CommandResult::Rejected(RejectionReason::Unresolved.reject(
            Partition::Subscription,
            format!("no target named {:?} is registered", open.target),
        ));
    };
    if target.answered_by() == AnsweredBy::StateJournal {
        return malformed(format!(
            "{} requires the state journal reader",
            target.name()
        ));
    }
    if target.answered_by() == AnsweredBy::Actors {
        return malformed(format!(
            "{} requires the standing members and the session wake",
            target.name()
        ));
    }
    let unrecorded = || match lens {
        None => Ok(circular_store::SurfaceSequence::from_index(0)),
        Some(_) => Err(CommandResult::Rejected(
            RejectionReason::Unresolved
                .reject(
                    Partition::Subscription,
                    "the stream this replay reads is no longer held".to_owned(),
                )
                .hint("end the replay and start one from a held checkpoint".to_owned()),
        )),
    };
    match (target, server) {
        (SubscriptionTarget::Records, _) => {
            unreachable!("the state journal target is answered by the session port")
        }
        (SubscriptionTarget::EdgeDepths, _) => {
            unreachable!("the members' target is answered by the session port")
        }
        (SubscriptionTarget::AuthoringCommits, _) if open.lens.is_some() => {
            CommandResult::Rejected(RejectionReason::Malformed.reject(
                Partition::Subscription,
                format!("{AUTHORING_COMMITS_TARGET} does not read under a replay lens"),
            ))
        }
        (SubscriptionTarget::ActorEvents, server) => {
            let mark = match server.map_or_else(unrecorded, |standing| feed_mark(standing, lens)) {
                Ok(mark) => mark,
                Err(rejected) => return rejected,
            };
            let opened = lens
                .filter(|lens| {
                    server.is_some_and(|standing| lens.stream() == standing.stream().get())
                })
                .map(|lens| lens.position().clone());
            *live = Some(Subscription {
                source: crate::daemon::subscription::SubscriptionSource::ActorEvents {
                    next: mark,
                    opened,
                },
                flow: credit_flow(mark.get()),
                correlation,
                lens: open.lens,
            });
            CommandResult::Accepted(Accepted::Nothing)
        }
        (SubscriptionTarget::DisplayFrames, server) => {
            let mark = match server.map_or_else(unrecorded, |standing| feed_mark(standing, lens)) {
                Ok(mark) => mark,
                Err(rejected) => return rejected,
            };
            *live = Some(Subscription {
                source: crate::daemon::subscription::SubscriptionSource::DisplayFrames {
                    next: mark,
                },
                flow: credit_flow(mark.get()),
                correlation,
                lens: open.lens,
            });
            CommandResult::Accepted(Accepted::Nothing)
        }
        (SubscriptionTarget::AuthoringCommits, _) => {
            let (scope, after) = match authoring_commit_args(open.args) {
                Ok(args) => args,
                Err(message) => return malformed(message),
            };
            if after > authoring.cursor() {
                return CommandResult::Rejected(RejectionReason::Malformed.reject(
                    Partition::Subscription,
                    format!(
                        "{AUTHORING_COMMITS_TARGET}.after {after} is beyond current cursor {}",
                        authoring.cursor()
                    ),
                ));
            }
            if !authoring.scope_exists(&scope) {
                return CommandResult::Rejected(RejectionReason::Unresolved.reject(
                    Partition::Subscription,
                    format!("{AUTHORING_COMMITS_TARGET} target scope does not exist"),
                ));
            }
            let published = authoring.commit_count();
            let first_retained = if published == 0 {
                authoring.cursor().saturating_add(1)
            } else {
                retained
                    .first_cursor()
                    .unwrap_or_else(|| authoring.cursor().saturating_add(1))
            };
            let reset_floor = (after.saturating_add(1) < first_retained).then_some(first_retained);
            let next_index = retained.index_after(after).min(published);
            let fold = match retained.state_before(next_index) {
                Ok(fold) => fold,
                Err(message) => return malformed(message),
            };
            *live = Some(Subscription {
                source: crate::daemon::subscription::SubscriptionSource::AuthoringCommits {
                    scope,
                    position: AuthoringPosition { next_index, fold },
                    live_cut: authoring.cursor(),
                    reset_floor,
                },
                flow: credit_flow(authoring.cursor()),
                correlation,
                lens: None,
            });
            CommandResult::Accepted(Accepted::Nothing)
        }
    }
}

fn feed_mark(
    standing: &ledger::ServerRead,
    lens: Option<&crate::daemon::replay::ReplayLens>,
) -> Result<circular_store::SurfaceSequence, CommandResult> {
    match lens {
        None => Ok(standing.mark()),
        Some(lens) if lens.stream() == standing.stream().get() => Ok(
            circular_store::SurfaceSequence::from_index(lens.position().end()),
        ),
        Some(_) => Err(CommandResult::Rejected(
            RejectionReason::Unresolved
                .reject(
                    Partition::Subscription,
                    "the stream this replay reads is no longer held".to_owned(),
                )
                .hint("end the replay and start one from a held checkpoint".to_owned()),
        )),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LensMove {
    Rewound { floor: u64 },
    Closed,
}

pub(crate) fn end_lens_feeds(
    stream: &mut impl LocalByteStream<Error = std::io::Error>,
    subscriptions: &mut Subscriptions,
    lens: u32,
    moved: LensMove,
) -> Vec<u32> {
    subscriptions.each(|live| {
        if live.as_ref().and_then(Subscription::lens) != Some(lens) {
            return;
        }
        if records::end_for_lens(stream, live, moved) {
            return;
        }
        let Some(Subscription {
            source:
                SubscriptionSource::ActorEvents { next, .. }
                | SubscriptionSource::DisplayFrames { next },
            flow,
            correlation,
            ..
        }) = live.as_mut()
        else {
            return;
        };
        let correlation = *correlation;
        let (fsm, reason, anchor) = match moved {
            LensMove::Rewound { floor } => (
                FsmEndReason::ResetRequired {
                    floor_or_cursor: floor,
                },
                SubscriptionEndReason::ResetRequired {
                    floor_or_cursor: floor.to_be_bytes().to_vec(),
                },
                floor,
            ),
            LensMove::Closed => (
                FsmEndReason::TargetGone,
                SubscriptionEndReason::TargetGone,
                next.get(),
            ),
        };
        let _ = flow.end(fsm, 0);
        write_subscription_end_at(
            stream,
            correlation,
            &SubscriptionEnded {
                reason,
                code: RejectionReason::Unresolved.number_in(Partition::Subscription),
                anchor: anchor.to_be_bytes().to_vec(),
            },
        );
        *live = None;
    })
}

fn authoring_commit_args(
    args: Value,
) -> Result<
    (
        Vec<circular_protocol::declaration_payload::ScopeSegment>,
        u64,
    ),
    String,
> {
    let Value::Object(object) = args else {
        return Err(format!("{AUTHORING_COMMITS_TARGET} args are not an object"));
    };
    let mut fields = object.into_map();
    let after = match fields.remove("after") {
        Some(Value::Int(value)) if value >= 0 => u64::try_from(value)
            .map_err(|_| format!("{AUTHORING_COMMITS_TARGET}.after exceeds u64"))?,
        Some(_) => {
            return Err(format!(
                "{AUTHORING_COMMITS_TARGET}.after is not a non-negative Int"
            ));
        }
        None => return Err(format!("{AUTHORING_COMMITS_TARGET}.after is absent")),
    };
    let scope = decode_scope_identity(
        fields
            .remove("scope")
            .ok_or_else(|| format!("{AUTHORING_COMMITS_TARGET}.scope is absent"))?,
    )
    .map_err(|error| format!("{AUTHORING_COMMITS_TARGET}.scope: {error}"))?;
    if let Some((unknown, _)) = fields.into_iter().next() {
        return Err(format!(
            "{AUTHORING_COMMITS_TARGET} has an unknown argument {unknown:?}"
        ));
    }
    Ok((scope, after))
}

fn write_subscription_frame_at(
    stream: &mut impl LocalByteStream<Error = std::io::Error>,
    correlation: u32,
    frame: &SubscriptionFrame,
    flow: &mut FlowFsm,
    anchor: u64,
) -> bool {
    let Ok(encoded) = frame.encode(Ceilings::for_boundary(Boundary::Wire)) else {
        return false;
    };
    let mut next_flow = flow.clone();
    match next_flow.observe(anchor) {
        Ok(SubscriptionAction::Emit) => {}
        Ok(SubscriptionAction::Backpressured) => return false,
        Err(reason) => {
            eprintln!("circular-daemon: subscription frame transition rejected: {reason:?}");
            return false;
        }
    }
    if write_envelope(
        stream,
        OwnerLocalChannelId::new(1),
        INITIAL_PROTOCOL_VERSION,
        SUBSCRIPTION_FRAME,
        correlation,
        &encoded,
    )
    .is_err()
    {
        return false;
    }
    *flow = next_flow;
    true
}

fn write_subscription_end_at(
    stream: &mut impl LocalByteStream<Error = std::io::Error>,
    correlation: u32,
    ended: &SubscriptionEnded,
) {
    if let Ok(encoded) = ended.encode(Ceilings::for_boundary(Boundary::Wire)) {
        let _ = write_envelope(
            stream,
            OwnerLocalChannelId::new(1),
            INITIAL_PROTOCOL_VERSION,
            SUBSCRIPTION_ENDED,
            correlation,
            &encoded,
        );
    }
}

#[cfg(test)]
pub(crate) fn credit(payload: &[u8], live: &mut Option<Subscription>) -> CommandResult {
    match decode_credit(payload, Ceilings::for_boundary(Boundary::Wire)) {
        Ok(grant) => grant_credit(
            PositiveCredit::try_new(FrameCredits(grant.frames))
                .expect("wire decoder admits only positive frame credit"),
            live,
        ),
        Err(rejection) => malformed(format!("{rejection:?}")),
    }
}

pub(crate) fn grant_credit(
    credit: PositiveCredit<FrameCredits>,
    live: &mut Option<Subscription>,
) -> CommandResult {
    let Some(subscription) = live.as_mut() else {
        return CommandResult::Rejected(RejectionReason::Unresolved.reject(
            Partition::Subscription,
            "this correlation names no live subscription".to_owned(),
        ));
    };
    let flow = &mut subscription.flow;
    if let Err(reason) = flow.grant_credit(&credit) {
        return CommandResult::Rejected(RejectionReason::Malformed.reject(
            Partition::Subscription,
            format!("credit transition rejected: {reason:?}"),
        ));
    }
    CommandResult::Accepted(Accepted::Nothing)
}

/// Owned frames projected from one published immutable prefix.
/// No world lock, cursor transition, or flow transition occurs here.
struct PreparedFrames<C> {
    frames: Vec<PreparedFrame<C>>,
    pending: u64,
    scanned_to: Option<C>,
    reset: Option<(u64, u64)>,
}

impl<C> Default for PreparedFrames<C> {
    fn default() -> Self {
        Self {
            frames: Vec::new(),
            pending: 0,
            scanned_to: None,
            reset: None,
        }
    }
}

struct PreparedFrame<C> {
    frame: SubscriptionFrame,
    anchor: u64,
    next_cursor: C,
    retires_scope: bool,
}

pub(crate) enum PumpResult {
    Open { pending: u64 },
    Ended,
}

impl PumpResult {
    fn finish(self, live: &mut Option<Subscription>) -> u64 {
        match self {
            Self::Open { pending } => pending,
            Self::Ended => {
                *live = None;
                0
            }
        }
    }
}

pub(crate) fn after(surface: circular_store::SurfaceSequence) -> circular_store::SurfaceSequence {
    circular_store::SurfaceSequence::from_index(
        usize::try_from(surface.get()).unwrap_or(usize::MAX),
    )
}

fn prepare_credit_feed(
    server: Option<&ledger::ServerRead>,
    next: circular_store::SurfaceSequence,
    limit: usize,
    lens: Option<&crate::daemon::replay::ReplayLens>,
    opened: Option<&ledger::ReadBound>,
    feed: fn(
        &ledger::ServerRead,
        circular_store::SurfaceSequence,
        usize,
        Option<(Option<&ledger::ReadBound>, &ledger::ReadBound)>,
    ) -> Result<ledger::LiveFeedPage, String>,
) -> PreparedFrames<circular_store::SurfaceSequence> {
    let Some(standing) = server else {
        return PreparedFrames::default();
    };
    if limit == 0 {
        return PreparedFrames::default();
    }
    let (bound, origin) = match lens {
        Some(lens) if lens.stream() == standing.stream().get() => {
            (Some((opened, lens.position())), FrameOrigin::Retained)
        }
        Some(_) => return PreparedFrames::default(),
        None => (None, FrameOrigin::Live),
    };
    let page = match feed(standing, next, limit, bound) {
        Ok(page) => page,
        Err(error) => {
            eprintln!("circular-daemon: subscription feed read failed: {error}");
            return PreparedFrames::default();
        }
    };
    let pending = page.pending;
    let frames = page
        .frames
        .into_iter()
        .enumerate()
        .map(|(offset, (surface, body))| PreparedFrame {
            frame: SubscriptionFrame::Credit {
                pending_after: pending - offset as u64 - 1,
                origin,
                payload: body,
            },
            anchor: surface.get(),
            next_cursor: after(surface),
            retires_scope: false,
        })
        .collect();
    PreparedFrames {
        frames,
        pending,
        ..PreparedFrames::default()
    }
}

fn prepare_authoring(
    scope: &[circular_protocol::declaration_payload::ScopeSegment],
    position: &AuthoringPosition,
    live_cut: u64,
    reset_floor: Option<u64>,
    limit: usize,
    authoring: &authoring::AuthoringState,
    retained: &crate::daemon::authoring_store::RetainedCommits,
) -> PreparedFrames<AuthoringPosition> {
    if let Some(floor) = reset_floor {
        return PreparedFrames {
            reset: Some((floor, authoring.cursor())),
            ..PreparedFrames::default()
        };
    }
    let published = authoring.commit_count();
    let column = retained.values(position.next_index, published);
    let read = |value: Result<circular_core::Value, String>| {
        value.and_then(|value| {
            authoring::DurableCommit::from_journal_entry(value.clone())
                .map(|commit| (value, commit))
                .map_err(|rejection| rejection.to_string())
        })
    };
    let mut count = 0;
    for value in &column {
        let Ok((_, commit)) = read(value.clone()) else {
            break;
        };
        if commit.affects_scope(scope) {
            count += 1;
            if commit.retires_scope(scope) {
                break;
            }
        }
    }
    let mut prepared = PreparedFrames {
        pending: count,
        ..Default::default()
    };
    let mut index = position.next_index;
    let mut scanned: Option<authoring::AuthoringState> = None;
    for value in column {
        if prepared.frames.len() >= limit {
            break;
        }
        let (value, commit) = match read(value) {
            Ok(read) => read,
            Err(reason) => {
                eprintln!("circular-daemon: retained authoring epoch is unreadable: {reason}");
                scanned = None;
                break;
            }
        };
        let base = scanned.take().unwrap_or_else(|| position.fold.clone());
        let (next, delta) = match authoring::AuthoringState::fold_journal_entry(Some(base), value) {
            Ok(folded) => folded,
            Err(reason) => {
                eprintln!("circular-daemon: retained authoring epoch does not fold: {reason}");
                break;
            }
        };
        index += 1;
        if !commit.affects_scope(scope) {
            scanned = Some(next);
            continue;
        }
        let payload = match next
            .delta_rows(&delta, scope)
            .and_then(|delta| commit.frame_value(delta))
        {
            Ok(payload) => payload,
            Err(reason) => {
                eprintln!("circular-daemon: authoring commit frame did not encode: {reason}");
                break;
            }
        };
        let retires_scope = commit.retires_scope(scope);
        prepared.frames.push(PreparedFrame {
            frame: SubscriptionFrame::Credit {
                pending_after: count - prepared.frames.len() as u64 - 1,
                origin: if commit.cursor <= live_cut {
                    FrameOrigin::Retained
                } else {
                    FrameOrigin::Live
                },
                payload,
            },
            anchor: commit.cursor,
            next_cursor: AuthoringPosition {
                next_index: index,
                fold: next.clone(),
            },
            retires_scope,
        });
        scanned = Some(next);
        if retires_scope {
            break;
        }
    }
    prepared.scanned_to = scanned.map(|fold| AuthoringPosition {
        next_index: index,
        fold,
    });
    prepared
}

/// Session-local phase two: gate → write → commit credit and cursor.
/// Both polling and Credit requests use this path, preserving wire order.
fn pump_prepared<C>(
    stream: &mut impl LocalByteStream<Error = std::io::Error>,
    cursor: &mut C,
    flow: &mut FlowFsm,
    correlation: u32,
    prepared: PreparedFrames<C>,
) -> PumpResult {
    let mut remaining = prepared.pending;
    if let Some((floor, anchor)) = prepared.reset {
        let _ = flow.end(
            FsmEndReason::ResetRequired {
                floor_or_cursor: floor,
            },
            0,
        );
        write_subscription_end_at(
            stream,
            correlation,
            &SubscriptionEnded {
                reason: SubscriptionEndReason::ResetRequired {
                    floor_or_cursor: floor.to_be_bytes().to_vec(),
                },
                code: RejectionReason::Unresolved.number_in(Partition::Subscription),
                anchor: anchor.to_be_bytes().to_vec(),
            },
        );
        return PumpResult::Ended;
    }
    for pending in prepared.frames {
        if !write_subscription_frame_at(stream, correlation, &pending.frame, flow, pending.anchor) {
            return PumpResult::Open { pending: remaining };
        }
        remaining -= 1;
        *cursor = pending.next_cursor;
        if pending.retires_scope {
            let _ = flow.end(
                FsmEndReason::ScopeGone {
                    cursor: pending.anchor,
                },
                0,
            );
            write_subscription_end_at(
                stream,
                correlation,
                &SubscriptionEnded {
                    reason: SubscriptionEndReason::ScopeGone {
                        cursor: pending.anchor.to_be_bytes().to_vec(),
                    },
                    code: RejectionReason::Unresolved.number_in(Partition::Subscription),
                    anchor: pending.anchor.to_be_bytes().to_vec(),
                },
            );
            return PumpResult::Ended;
        }
    }
    if let Some(scanned_to) = prepared.scanned_to {
        *cursor = scanned_to;
    }
    PumpResult::Open { pending: remaining }
}

pub(crate) fn pump_subscription(
    stream: &mut impl LocalByteStream<Error = std::io::Error>,
    live: &mut Option<Subscription>,
    server: Option<&ledger::ServerRead>,
    authoring: &authoring::AuthoringState,
    retained: &crate::daemon::authoring_store::RetainedCommits,
    records: &records::Sources,
    lens: Option<&crate::daemon::replay::ReplayLens>,
) -> u64 {
    let Some(Subscription {
        source,
        flow,
        correlation,
        ..
    }) = live.as_mut()
    else {
        return 0;
    };
    let result = match source {
        SubscriptionSource::Records { reader } => {
            records::pump_reader(stream, reader, flow, *correlation, records)
        }
        SubscriptionSource::ActorEvents { next, opened } => {
            let prepared = prepare_credit_feed(
                server,
                *next,
                flow.credit().0 as usize,
                lens,
                opened.as_ref(),
                ledger::ServerRead::actor_events_feed_within,
            );
            pump_prepared(stream, next, flow, *correlation, prepared)
        }
        SubscriptionSource::DisplayFrames { next } => {
            let prepared = prepare_credit_feed(
                server,
                *next,
                flow.credit().0 as usize,
                lens,
                None,
                ledger::ServerRead::display_frames_feed_within,
            );
            pump_prepared(stream, next, flow, *correlation, prepared)
        }
        SubscriptionSource::EdgeDepths { feed } => {
            edge_depths::pump(stream, feed, flow, *correlation)
        }
        SubscriptionSource::AuthoringCommits {
            scope,
            position,
            live_cut,
            reset_floor,
        } => {
            let prepared = prepare_authoring(
                scope,
                position,
                *live_cut,
                *reset_floor,
                flow.credit().0 as usize,
                authoring,
                retained,
            );
            pump_prepared(stream, position, flow, *correlation, prepared)
        }
    };
    result.finish(live)
}

pub(crate) fn unsubscribe_live(live: &mut Option<Subscription>) -> CommandResult {
    let Some(subscription) = live.as_mut() else {
        return CommandResult::Rejected(RejectionReason::Unresolved.reject(
            Partition::Subscription,
            "this correlation names no live subscription".to_owned(),
        ));
    };
    let flow = &mut subscription.flow;
    match flow.unsubscribe(0) {
        Ok(()) => {
            *live = None;
            CommandResult::Accepted(Accepted::Nothing)
        }
        Err(reason) => CommandResult::Rejected(RejectionReason::Unresolved.reject(
            Partition::Subscription,
            format!("unsubscribe transition rejected: {reason:?}"),
        )),
    }
}

