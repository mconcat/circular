mod domain;
mod gate;
mod ports;
use crate::daemon::declaration::{
    SessionEpoch, abort_epoch, answer_declaration, begin_epoch, prepare_content_declaration,
    validate_epoch,
};
use circular_protocol::rejection_code::RejectionReason;

use crate::daemon::authoring_store;
use crate::daemon::query::{OpenQueries, QueryRequestContext, answer_query};
use crate::daemon::replay::{
    FeedReset, Lenses, ReplayLens, answer_replay, replay_end, replay_rewind, replay_start,
};
use crate::daemon::replay_clock::SystemTime;
use crate::daemon::run_control::answer_run_control;
use crate::daemon::subscription::{
    Subscription, Subscriptions, answer_subscription, pump_subscription,
};
use circular_core::{Boundary, Ceilings, Value};
use circular_protocol::declaration_payload::{self, CommandResult};
use circular_protocol::{
    DeclarationVerb, EnvelopeHeader, EventInjectionVerb, LedgerTransitionVerb, LifecycleVerb,
    QueryVerb, ReplayControlVerb, SessionMechanicsVerb, StableVerb, SubscriptionVerb,
};
use circular_transport::{
    OwnerLocalChannelId, OwnerLocalStream, SessionIoError, decode_session_envelope, read_envelope,
    write_envelope,
};
use engine::execution_profile::ProductExecutionProfile;

pub(crate) struct ReplayClock<'a> {
    pub(crate) time: &'a SystemTime,
    pub(crate) wake: &'a std::sync::Arc<engine::wake::Wake>,
    pub(crate) correlations: &'a mut circular_protocol::LiveCorrelations,
}

pub(crate) fn serve_session(
    stream: &mut OwnerLocalStream,
    owners: &crate::daemon::platform::SessionOwners,
) {
    let push = match stream.try_clone() {
        Ok(push) => push,
        Err(error) => {
            eprintln!("circular-daemon: could not split the stream: {error}");
            return;
        }
    };
    if let Err(error) = stream.set_read_deadline(Some(SILENT_PEER_DEADLINE)) {
        eprintln!("circular-daemon: could not bound the first read: {error}");
        return;
    }
    let session = match gate::State::stand(
        push,
        &owners.world,
        &owners.authoring_store,
        &owners.execution,
        owners.session_registry.clone(),
        owners.time.clone(),
    ) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("circular-daemon: could not open the session wake: {error}");
            return;
        }
    };
    let wake = session.wake.clone();
    let mut first = true;
    let state = std::rc::Rc::new(std::cell::RefCell::new(session));
    let mut admission = gate::new(state.clone());

    use std::os::fd::AsFd;
    loop {
        let read = if first {
            read_envelope(stream)
        } else {
            loop {
                wake.drain();
                gate::poll(&state);
                let woke = match engine::wake::wait_readable(&[stream.as_fd(), wake.as_fd()]) {
                    Ok(woke) => woke,
                    Err(error) => {
                        eprintln!("circular-daemon: session wait failed: {error}");
                        return;
                    }
                };
                if woke.at(0) {
                    break;
                }
            }
            read_envelope(stream)
        };
        let bytes = match read {
            Ok(bytes) => bytes,
            Err(error) => {
                if !matches!(error, SessionIoError::PeerClosed) {
                    eprintln!("circular-daemon: could not read an envelope: {error}");
                }
                return;
            }
        };

        let decoded = match decode_session_envelope(&bytes) {
            Ok(decoded) => decoded,
            Err(error) => {
                eprintln!("circular-daemon: refused an envelope: {error}");
                return;
            }
        };

        if first {
            first = false;
            if let Err(error) = stream.set_read_deadline(None) {
                eprintln!("circular-daemon: could not release the first-envelope bound: {error}");
                return;
            }
        }

        let header = *decoded.header();
        if !gate::receive(&mut admission, &state, header, decoded.payload()) {
            return;
        }
    }
}

enum CommandAnswer {
    Declaration(CommandResult),
    Injection(CommandResult),
    Transition(CommandResult),
    Lifecycle(circular_protocol::LifecycleResult),
    Query(circular_protocol::declaration_payload::QueryResult),
    Subscription(CommandResult),
    Replay(CommandResult),
    Unanswered,
}

#[allow(clippy::too_many_arguments)]
fn dispatch_typed(
    stream: &mut impl circular_transport::LocalByteStream<Error = std::io::Error>,
    header: EnvelopeHeader,
    canonical_payload: &[u8],
    payload: domain::Payload,
    world: &crate::daemon::read_world::SharedWorld,
    authoring_store: &authoring_store::AuthoringStore,
    execution: &ProductExecutionProfile,
    candidate: &mut SessionEpoch,
    subscriptions: &mut Subscriptions,
    authoring_queries: &mut OpenQueries,
    lenses: &mut Lenses,
    clock: ReplayClock<'_>,
) -> CommandAnswer {
    use circular_protocol::{AdmittedHandler, SessionPayload as P};
    if let P::ReplayControl(replay) = payload {
        let payload = &replay.bytes;
        let prefix = world.read();
        let server = prefix.server.as_deref();
        let key = header.correlation();
        let mut end_feeds = |subscriptions: &mut Subscriptions, moved| {
            for correlation in
                crate::daemon::subscription::end_lens_feeds(stream, subscriptions, key, moved)
            {
                clock.correlations.finish(correlation);
            }
        };
        return match StableVerb::ReplayControl(replay.verb) {
            StableVerb::ReplayControl(ReplayControlVerb::ReplayStart) => {
                match replay_start(payload, server, clock.time, clock.wake) {
                    Ok(lens) => {
                        lenses.open(key, lens);
                        CommandAnswer::Replay(crate::daemon::replay::accepted())
                    }
                    Err(rejected) => CommandAnswer::Replay(rejected),
                }
            }
            StableVerb::ReplayControl(ReplayControlVerb::ReplayRewind) => {
                match replay_rewind(payload, server, lenses.get_mut(key), clock.time, clock.wake) {
                    Ok(feeds) => {
                        if feeds == FeedReset::Reset
                            && let Some(lens) = lenses.get(key)
                        {
                            end_feeds(
                                subscriptions,
                                crate::daemon::subscription::LensMove::Rewound {
                                    floor: lens.position().end() as u64,
                                },
                            );
                        }
                        CommandAnswer::Replay(crate::daemon::replay::accepted())
                    }
                    Err(rejected) => CommandAnswer::Replay(rejected),
                }
            }
            StableVerb::ReplayControl(ReplayControlVerb::ReplayEnd) => {
                let result = replay_end(payload, lenses.contains(key));
                if matches!(result, CommandResult::Accepted(_)) {
                    lenses.close(key);
                    end_feeds(subscriptions, crate::daemon::subscription::LensMove::Closed);
                }
                CommandAnswer::Replay(result)
            }
            _ => CommandAnswer::Unanswered,
        };
    }
    let mut handler =
        circular_protocol::DispatchHandler::new(circular_protocol::SemanticDispatcher::new(
            ports::DeclarationPort {
                payload: canonical_payload,
                world,
                store: authoring_store,
                execution,
                candidate,
            },
            ports::InjectionPort { world },
            ports::TransitionPort { world, execution },
            ports::LifecyclePort { world, execution },
            ports::QueryPort {
                header,
                world,
                store: authoring_store,
                execution,
                queries: authoring_queries,
                lenses,
            },
            ports::SubscriptionPort {
                stream,
                world,
                store: authoring_store,
                subscriptions,
                lenses,
                wake: clock.wake,
            },
        ));
    let response = handler.handle(
        &circular_protocol::CorrelationId::from_value(header.correlation()),
        payload,
    );
    fn result(
        r: circular_protocol::CommandResult<
            declaration_payload::Accepted,
            declaration_payload::Rejected,
        >,
    ) -> CommandResult {
        match r {
            circular_protocol::CommandResult::Accepted(a) => CommandResult::Accepted(a),
            circular_protocol::CommandResult::Rejected(r) => CommandResult::Rejected(r),
        }
    }
    match response {
        Some(P::CommandResult(r)) => CommandAnswer::Declaration(result(r)),
        Some(P::InjectAck(r)) => CommandAnswer::Injection(result(r)),
        Some(P::TransitionResult(r)) => CommandAnswer::Transition(result(r)),
        Some(P::SubscribeAck(r)) => CommandAnswer::Subscription(result(r)),
        Some(P::LifecycleResult(r)) => CommandAnswer::Lifecycle(match r {
            circular_protocol::CommandResult::Accepted(a) => {
                circular_protocol::LifecycleResult::Accepted(a)
            }
            circular_protocol::CommandResult::Rejected(r) => {
                circular_protocol::LifecycleResult::Rejected(r)
            }
        }),
        Some(P::QueryResult(r)) => CommandAnswer::Query(match r {
            circular_protocol::QueryResult::Rejected(r) => {
                declaration_payload::QueryResult::Rejected(r)
            }
            circular_protocol::QueryResult::Accepted(page) => {
                declaration_payload::QueryResult::Page(declaration_payload::QueryPage {
                    cut: page.cut().cloned(),
                    folded_from: page.folded_from().cloned(),
                    reached: page.reached().cloned(),
                    anchor: page.anchor().clone(),
                    items: page.items().to_vec(),
                    terminal: match page.end() {
                        circular_protocol::PageEnd::Complete => {
                            declaration_payload::Terminal::Complete
                        }
                        circular_protocol::PageEnd::More { next } => {
                            declaration_payload::Terminal::More(next.clone())
                        }
                        circular_protocol::PageEnd::Diagnostic { code } => {
                            declaration_payload::Terminal::Diagnostic(*code)
                        }
                    },
                })
            }
        }),
        _ => CommandAnswer::Unanswered,
    }
}

fn write_answer(
    stream: &mut impl circular_transport::LocalByteStream<Error = std::io::Error>,
    header: EnvelopeHeader,
    answer: CommandAnswer,
) {
    match answer {
        CommandAnswer::Declaration(result) => answer_declaration(stream, header, result),
        CommandAnswer::Injection(result) => answer_with(stream, header, INJECT_ACK, result),
        CommandAnswer::Transition(result) => answer_with(stream, header, TRANSITION_RESULT, result),
        CommandAnswer::Lifecycle(result) => answer_run_control(stream, header, result),
        CommandAnswer::Query(result) => answer_query(stream, header, result),
        CommandAnswer::Subscription(result) => answer_subscription(stream, header, result),
        CommandAnswer::Replay(result) => answer_replay(stream, header, result),
        CommandAnswer::Unanswered => unanswered(header),
    }
}

pub(crate) fn poll_subscriptions(
    stream: &mut impl circular_transport::LocalByteStream<Error = std::io::Error>,
    subscriptions: &mut Subscriptions,
    world: &crate::daemon::read_world::SharedWorld,
    store: &authoring_store::AuthoringStore,
    lenses: &Lenses,
) -> Vec<u32> {
    let prefix = world.read();
    let retained = store.retained();
    let live_records = crate::daemon::subscription::records::Sources::of(&prefix, store);
    subscriptions.each(|live| {
        let lens = match live.as_ref().and_then(Subscription::lens) {
            None => None,
            Some(key) => match lenses.get(key) {
                Some(lens) => Some(lens),
                None => return,
            },
        };
        let bounded;
        let records = match lens {
            None => &live_records,
            Some(lens) => {
                if !matches!(
                    live,
                    Some(Subscription {
                        source: crate::daemon::subscription::SubscriptionSource::Records { .. },
                        ..
                    })
                ) {
                    &live_records
                } else {
                    match crate::daemon::subscription::records::Sources::of_read(
                        &prefix,
                        store,
                        Some(lens),
                    ) {
                        Ok(sources) => {
                            bounded = sources;
                            &bounded
                        }
                        Err(_) => return,
                    }
                }
            }
        };
        pump_subscription(
            stream,
            live,
            prefix.server.as_deref(),
            &prefix.authoring,
            &retained,
            records,
            lens,
        );
    })
}

fn dispatch_candidate_declaration(
    payload: &[u8],
    command: domain::Command,
    candidate: &mut SessionEpoch,
    authoring: &mut engine::authoring_assembly::ledger::AuthoringState,
    execution: &ProductExecutionProfile,
) -> CommandResult {
    use domain::Command as D;
    match command {
        D::BeginEpoch {
            scope,
            commit_id,
            expected_revision,
            expected_environment,
        } => begin_epoch(
            declaration_payload::BeginEpoch {
                scope,
                commit_id,
                expected_revision,
                expected_environment,
            },
            payload,
            candidate,
            authoring,
        ),
        D::ValidateEpoch { epoch } => validate_epoch(
            epoch
                .as_deref()
                .expect("session admission supplied an epoch reference"),
            candidate,
            authoring,
            execution,
        ),
        D::AbortEpoch { epoch } => abort_epoch(
            epoch
                .as_deref()
                .expect("session admission supplied an epoch reference"),
            candidate,
        ),
        D::CommitEpoch { .. } => {
            unreachable!("CommitEpoch is routed outside World by the declaration port")
        }
        content @ (D::UpsertTemplate { .. }
        | D::RetireTemplate { .. }
        | D::UpsertActor { .. }
        | D::RetireActor { .. }
        | D::UpsertEdge { .. }
        | D::RetireEdge { .. }
        | D::UpsertScope { .. }
        | D::RetireScope { .. }
        | D::MoveToScope { .. }
        | D::UpsertExportMount { .. }
        | D::RetireExportMount { .. }
        | D::UpsertAnnotation { .. }
        | D::RetireAnnotation { .. }
        | D::SetPresentation { .. }
        | D::SetFlags { .. }) => prepare_content_declaration(content.verb(), payload, candidate),
    }
}

fn unanswered(header: EnvelopeHeader) {
    eprintln!(
        "circular-daemon: {:?} has no adapter yet — correlation {} goes unanswered",
        header.verb(),
        header.correlation()
    );
}

pub(crate) fn answer_with(
    stream: &mut impl circular_transport::LocalByteStream<Error = std::io::Error>,
    header: EnvelopeHeader,
    verb: StableVerb,
    result: CommandResult,
) {
    let body = match result.encode(Ceilings::for_boundary(Boundary::Wire)) {
        Ok(body) => body,
        Err(error) => {
            eprintln!("circular-daemon: could not build a command result: {error:?}");
            return;
        }
    };
    if let Err(error) = write_envelope(
        stream,
        OwnerLocalChannelId::new(1),
        header.protocol_version(),
        verb,
        header.correlation(),
        &body,
    ) {
        eprintln!("circular-daemon: could not answer: {error}");
    }
}

pub(crate) const BEGIN_EPOCH: StableVerb = StableVerb::Declaration(DeclarationVerb::BeginEpoch);
pub(crate) const COMMAND_RESULT: StableVerb =
    StableVerb::Declaration(DeclarationVerb::CommandResult);
pub(crate) const INJECT_ACK: StableVerb = StableVerb::EventInjection(EventInjectionVerb::InjectAck);
pub(crate) const SUBSCRIBE_ACK: StableVerb =
    StableVerb::Subscription(SubscriptionVerb::SubscribeAck);
pub(crate) const SUBSCRIPTION_FRAME: StableVerb = StableVerb::Subscription(SubscriptionVerb::Frame);
pub(crate) const SUBSCRIPTION_ENDED: StableVerb =
    StableVerb::Subscription(SubscriptionVerb::SubscriptionEnded);
pub(crate) const REPLAY_RESULT: StableVerb =
    StableVerb::ReplayControl(ReplayControlVerb::ReplayResult);
pub(crate) const LIFECYCLE_RESULT: StableVerb =
    StableVerb::Lifecycle(LifecycleVerb::LifecycleResult);
const TRANSITION_RESULT: StableVerb =
    StableVerb::LedgerTransition(LedgerTransitionVerb::TransitionResult);
pub(crate) const QUERY_RESULT: StableVerb = StableVerb::Query(QueryVerb::QueryResult);
const HELLO_ACK: StableVerb = StableVerb::SessionMechanics(SessionMechanicsVerb::HelloAck);

const SILENT_PEER_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

#[cfg(test)]
use crate::daemon::ledger;
#[cfg(test)]
use crate::daemon::platform::World;

