//! Localhost-only OTLP/HTTP JSON receiver and durable forwarding worker.
//!
//! HTTP success means the complete scrubbed batch has entered the local
//! durable queue.  The raw request body is never persisted.  Each split keeps
//! a stable queue id across retention/restart and is removed only after the
//! owning Source actor recorded it at its own coordinate.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use circular_transport::is_socket_timeout;
use serde_json::Value;

use crate::activation_detail::RegistrationFailure as StartFailure;
use crate::activation_detail::source;
use crate::otlp_ingress::state::{
    DropReason, OtlpEdgeCounters, OtlpSignal, OtlpStateStore, PendingSplit,
};
use crate::peer_bridges::scrubber::{PreLedgerScrubber, ScrubError};

const HEADER_LIMIT: usize = 16 * 1024;
const RECEIVER_BODY_LIMIT: usize = circular_core::MAX_REASSEMBLED_BODY_BYTES;
pub const OUTBOUND_BODY_CAP: usize = 900 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(2);
/// Time budget for one complete request. `IO_TIMEOUT` still interrupts a
/// stalled socket; this deadline permits a normally slow request to progress
/// across more than one such interruption.
const REQUEST_DEADLINE: Duration = Duration::from_secs(30);
const DRAIN_LIMIT: usize = 1024 * 1024;

pub(crate) enum OtlpDelivery {
    Held,
    Recorded,
    Failed(StartFailure),
}

struct ForwardWake {
    bell: mpsc::SyncSender<()>,
}

impl ForwardWake {
    fn new() -> (Self, mpsc::Receiver<()>) {
        let (bell, rung) = mpsc::sync_channel(1);
        (Self { bell }, rung)
    }

    fn ring(&self) {
        let _ = self.bell.try_send(());
    }

    fn stop(&self) {
        self.ring();
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OtlpRefusal {
    Request {
        signal: Option<OtlpSignal>,
        kind: DropReason,
    },
    Record {
        signal: OtlpSignal,
        error: ScrubError,
    },
}

impl OtlpRefusal {
    #[must_use]
    pub const fn signal(&self) -> Option<OtlpSignal> {
        match self {
            Self::Request { signal, .. } => *signal,
            Self::Record { signal, .. } => Some(*signal),
        }
    }
}

pub(crate) type RefusalHand = Arc<dyn Fn(OtlpRefusal) + Send + Sync>;

#[derive(Default)]
pub(crate) struct Refusals(Mutex<RefusalSeat>);

#[derive(Default)]
struct RefusalSeat {
    held: Vec<OtlpRefusal>,
    hand: Option<RefusalHand>,
}

impl Refusals {
    fn push(&self, refusals: impl IntoIterator<Item = OtlpRefusal>) -> Result<(), String> {
        let mut seat = self
            .0
            .lock()
            .map_err(|_| "OTLP refusal ledger is poisoned".to_owned())?;
        for refusal in refusals {
            match &seat.hand {
                Some(hand) => hand(refusal),
                None => seat.held.push(refusal),
            }
        }
        Ok(())
    }

    fn arm(&self, hand: RefusalHand) -> Result<(), String> {
        let mut seat = self
            .0
            .lock()
            .map_err(|_| "OTLP refusal ledger is poisoned".to_owned())?;
        for refusal in std::mem::take(&mut seat.held) {
            hand(refusal);
        }
        seat.hand = Some(hand);
        Ok(())
    }
}

pub struct OtlpEdge {
    local_addr: SocketAddr,
    state: Arc<Mutex<OtlpStateStore>>,
    stop: Arc<AtomicBool>,
    recorded_pause: Arc<AtomicBool>,
    accept_wake: Arc<crate::wake::Wake>,
    forward_wake: Arc<ForwardWake>,
    health: Arc<EdgeHealth>,
    refusals: Arc<Refusals>,
    receiver: Option<JoinHandle<()>>,
    forwarder: Option<JoinHandle<()>>,
}

impl OtlpEdge {
    pub(crate) fn start_with_delivery(
        bind: SocketAddr,
        mut state: OtlpStateStore,
        mut delivery: impl FnMut(OtlpSignal, &str, &[u8]) -> OtlpDelivery + Send + 'static,
        fallen: impl Fn(StartFailure) + Send + Sync + 'static,
    ) -> Result<Self, StartFailure> {
        let at = |step, message: String| StartFailure::new(step, message);
        if !bind.ip().is_loopback() {
            return Err(at(
                source::LISTEN_REJECTED,
                "OTLP receiver bind address must be numeric loopback".to_owned(),
            ));
        }
        audit_durable_queue(&state).map_err(|message| at(source::CUSTODY_UNAVAILABLE, message))?;
        let listener = match TcpListener::bind(bind) {
            Ok(listener) => listener,
            Err(error) => {
                let warning = state
                    .record_bind_failure()
                    .map_err(|message| at(source::CUSTODY_UNAVAILABLE, message))?;
                publish_warning(warning);
                return Err(at(
                    source::BIND_REFUSED,
                    format!("cannot bind OTLP receiver at {bind}: {error}"),
                ));
            }
        };
        listener.set_nonblocking(true).map_err(|error| {
            at(
                source::RECEIVER_UNSTARTED,
                format!("cannot make OTLP receiver nonblocking: {error}"),
            )
        })?;
        let local_addr = listener.local_addr().map_err(|error| {
            at(
                source::RECEIVER_UNSTARTED,
                format!("cannot inspect OTLP receiver address: {error}"),
            )
        })?;
        publish_warning(
            state
                .flush()
                .map_err(|message| at(source::CUSTODY_UNAVAILABLE, message))?,
        );

        let state = Arc::new(Mutex::new(state));
        let stop = Arc::new(AtomicBool::new(false));
        let recorded_pause = Arc::new(AtomicBool::new(false));
        let receiver_paused = recorded_pause.clone();
        let accept_wake = Arc::new(crate::wake::Wake::new().map_err(|error| {
            at(
                source::RECEIVER_UNSTARTED,
                format!("cannot open the OTLP accept wake: {error}"),
            )
        })?);
        let receiver_wake = Arc::clone(&accept_wake);
        let (forward_wake, rung) = ForwardWake::new();
        let forward_wake = Arc::new(forward_wake);
        let receiver_queued = Arc::clone(&forward_wake);
        let health = Arc::new(EdgeHealth {
            first: Mutex::new(None),
            fallen: Box::new(fallen),
        });

        let refusals = Arc::new(Refusals::default());
        let receiver_state = Arc::clone(&state);
        let receiver_stop = Arc::clone(&stop);
        let receiver_health = Arc::clone(&health);
        let receiver_refusals = Arc::clone(&refusals);
        let receiver = std::thread::Builder::new()
            .name("otlp-receiver".to_owned())
            .spawn(move || {
                receiver_loop(
                    listener,
                    &receiver_state,
                    &receiver_stop,
                    &receiver_health,
                    &receiver_paused,
                    &receiver_wake,
                    &receiver_refusals,
                    &receiver_queued,
                )
            })
            .map_err(|error| {
                at(
                    source::RECEIVER_UNSTARTED,
                    format!("cannot start OTLP receiver thread: {error}"),
                )
            })?;

        let forwarder_state = Arc::clone(&state);
        let forwarder_stop = Arc::clone(&stop);
        let forwarder_health = Arc::clone(&health);
        let forwarder_wake = Arc::clone(&forward_wake);
        let forwarder = match std::thread::Builder::new()
            .name("otlp-forwarder".to_owned())
            .spawn(move || {
                let _bell = forwarder_wake;
                forwarder_loop(
                    &mut delivery,
                    &forwarder_state,
                    &forwarder_stop,
                    &forwarder_health,
                    &rung,
                )
            }) {
            Ok(forwarder) => forwarder,
            Err(error) => {
                stop.store(true, Ordering::SeqCst);
                accept_wake.notify();
                let _ = receiver.join();
                return Err(at(
                    source::RECEIVER_UNSTARTED,
                    format!("cannot start OTLP forwarder thread: {error}"),
                ));
            }
        };

        Ok(Self {
            local_addr,
            state,
            stop,
            recorded_pause,
            accept_wake,
            forward_wake,
            health,
            refusals,
            receiver: Some(receiver),
            forwarder: Some(forwarder),
        })
    }

    pub fn pause_input(&self, paused: bool) {
        self.recorded_pause.store(paused, Ordering::Release);
        self.accept_wake.notify();
        self.forward_wake.ring();
    }

    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn counters(&self) -> Result<OtlpEdgeCounters, String> {
        self.state
            .lock()
            .map(|state| state.counters())
            .map_err(|_| "OTLP edge state lock is poisoned".to_owned())
    }

    pub(crate) fn arm_refusals(&self, hand: RefusalHand) -> Result<(), String> {
        self.refusals.arm(hand)
    }

    pub fn check_health(&self) -> Result<(), StartFailure> {
        let health = self.health.first.lock().map_err(|_| {
            StartFailure::new(
                crate::activation_detail::source::HEALTH_POISONED,
                "OTLP edge health lock is poisoned",
            )
        })?;
        match health.as_ref() {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    pub fn shutdown(mut self) -> Result<OtlpEdgeCounters, String> {
        self.reap()?;
        self.check_health().map_err(|failure| failure.to_string())?;
        self.counters()
    }

    fn reap(&mut self) -> Result<(), String> {
        self.stop.store(true, Ordering::SeqCst);
        self.accept_wake.notify();
        self.forward_wake.stop();
        let running = self.receiver.is_some() || self.forwarder.is_some();
        let receiver = join_thread(self.receiver.take(), "receiver");
        let forwarder = join_thread(self.forwarder.take(), "forwarder");
        if running {
            match self.state.lock().map(|mut state| state.flush()) {
                Ok(Ok(warning)) => publish_warning(warning),
                Ok(Err(error)) => eprintln!("circular-daemon: otlp: {error}"),
                Err(_) => eprintln!("circular-daemon: otlp: OTLP edge state lock is poisoned"),
            }
        }
        receiver.and(forwarder)
    }
}

struct EdgeHealth {
    first: Mutex<Option<StartFailure>>,
    fallen: Box<dyn Fn(StartFailure) + Send + Sync>,
}

fn audit_durable_queue(state: &OtlpStateStore) -> Result<(), String> {
    for item in state.queued_items() {
        let body: Value = serde_json::from_slice(&item.body)
            .map_err(|_| format!("durable OTLP queue item {:?} is not JSON", item.id))?;
        validate_signal(item.signal, &body).map_err(|_| {
            format!(
                "durable OTLP queue item {:?} has an unsupported signal shape",
                item.id
            )
        })?;
        if item.body.len() > OUTBOUND_BODY_CAP {
            return Err(format!(
                "durable OTLP queue item {:?} exceeds the forwarding cap",
                item.id
            ));
        }
        let scrubbed = PreLedgerScrubber
            .scrub(body.clone())
            .map_err(|_| format!("durable OTLP queue item {:?} is not scrub-safe", item.id))?;
        let (scrubbed, scrubbed_fields) = scrubbed.into_parts();
        if scrubbed_fields != 0 || scrubbed != body {
            return Err(format!(
                "durable OTLP queue item {:?} contains unscrubbed fields",
                item.id
            ));
        }
    }
    Ok(())
}

impl Drop for OtlpEdge {
    fn drop(&mut self) {
        let _ = self.reap();
    }
}

fn join_thread(thread: Option<JoinHandle<()>>, name: &str) -> Result<(), String> {
    if let Some(thread) = thread {
        thread
            .join()
            .map_err(|_| format!("OTLP {name} thread panicked"))?;
    }
    Ok(())
}

fn accept_is_transient(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::Interrupted | std::io::ErrorKind::ConnectionAborted
    ) || matches!(
        error.raw_os_error(),
        Some(nix::libc::EMFILE | nix::libc::ENFILE | nix::libc::ENOBUFS)
    )
}

struct HeldConnection {
    stream: TcpStream,
    peer: SocketAddr,
    pending: Vec<u8>,
}

enum AfterResponse {
    KeepAlive,
    Close,
}

const HELD_CONNECTIONS: usize = 64 - 2;

fn hold(held: &mut Vec<HeldConnection>, connection: HeldConnection) {
    if held.len() == HELD_CONNECTIONS {
        held.remove(0);
    }
    held.push(connection);
}

fn receiver_loop(
    listener: TcpListener,
    state: &Arc<Mutex<OtlpStateStore>>,
    stop: &AtomicBool,
    health: &EdgeHealth,
    recorded_pause: &AtomicBool,
    wake: &crate::wake::Wake,
    refusals: &Refusals,
    queued: &ForwardWake,
) {
    use std::os::fd::AsFd;
    let backoff = crate::effect_retry::ReceiverBackoff::default();
    let mut held: Vec<HeldConnection> = Vec::new();
    while !stop.load(Ordering::SeqCst) {
        if recorded_pause.load(Ordering::Acquire) {
            held.clear();
            wake.drain();
            if stop.load(Ordering::SeqCst) || !recorded_pause.load(Ordering::Acquire) {
                continue;
            }
            if let Err(error) = crate::wake::wait_readable(&[wake.as_fd()]) {
                set_unhealthy(
                    health,
                    source::RECEIVER_WAIT_FAILED,
                    format!("OTLP receiver wait failed: {error}"),
                );
                return;
            }
            continue;
        }
        match listener.accept() {
            Ok((stream, peer)) => {
                if let Err(error) = stream.set_nonblocking(false) {
                    set_unhealthy(
                        health,
                        source::RECEIVER_BLOCKING_UNRESTORED,
                        format!("OTLP receiver cannot restore blocking reads: {error}"),
                    );
                    return;
                }
                let connection = HeldConnection {
                    stream,
                    peer,
                    pending: Vec::new(),
                };
                match serve_connection(connection, true, state, refusals, queued, &backoff) {
                    Ok(Some(connection)) => hold(&mut held, connection),
                    Ok(None) => {}
                    Err(error) => {
                        set_unhealthy(health, source::RECEIVER_CONNECTION_FAILED, error);
                        return;
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                wake.drain();
                if stop.load(Ordering::SeqCst) || recorded_pause.load(Ordering::Acquire) {
                    continue;
                }
                let woke = {
                    let mut fds = vec![listener.as_fd(), wake.as_fd()];
                    fds.extend(held.iter().map(|connection| connection.stream.as_fd()));
                    crate::wake::wait_readable(&fds)
                };
                let woke = match woke {
                    Ok(woke) => woke,
                    Err(error) => {
                        set_unhealthy(
                            health,
                            source::RECEIVER_WAIT_FAILED,
                            format!("OTLP receiver wait failed: {error}"),
                        );
                        return;
                    }
                };
                let mut ready = Vec::new();
                for index in (0..held.len()).rev() {
                    if woke.at(index + 2) {
                        ready.push(held.remove(index));
                    }
                }
                for connection in ready.into_iter().rev() {
                    match serve_connection(connection, false, state, refusals, queued, &backoff) {
                        Ok(Some(connection)) => hold(&mut held, connection),
                        Ok(None) => {}
                        Err(error) => {
                            set_unhealthy(health, source::RECEIVER_CONNECTION_FAILED, error);
                            return;
                        }
                    }
                }
            }
            Err(error) if accept_is_transient(&error) => {
                eprintln!("circular-daemon: OTLP accept waits: {error}");
                wake.drain();
                if stop.load(Ordering::SeqCst) || recorded_pause.load(Ordering::Acquire) {
                    continue;
                }
                if let Err(error) = crate::wake::wait_readable(&[listener.as_fd(), wake.as_fd()]) {
                    set_unhealthy(
                        health,
                        source::RECEIVER_WAIT_FAILED,
                        format!("OTLP receiver wait failed: {error}"),
                    );
                    return;
                }
            }
            Err(error) => {
                set_unhealthy(
                    health,
                    source::RECEIVER_ACCEPT_FAILED,
                    format!("OTLP receiver accept failed: {error}"),
                );
                return;
            }
        }
    }
}

fn serve_connection(
    mut connection: HeldConnection,
    fresh: bool,
    state: &Arc<Mutex<OtlpStateStore>>,
    refusals: &Refusals,
    queued: &ForwardWake,
    backoff: &crate::effect_retry::ReceiverBackoff,
) -> Result<Option<HeldConnection>, String> {
    let mut fresh = fresh;
    loop {
        let after = serve_request(
            &mut connection.stream,
            connection.peer,
            &mut connection.pending,
            fresh,
            state,
            refusals,
            queued,
            IO_TIMEOUT,
            REQUEST_DEADLINE,
            backoff,
        )?;
        fresh = false;
        match after {
            None => return Ok(None),
            Some(AfterResponse::Close) => {
                crate::http_close::close_after_response(
                    &mut connection.stream,
                    DRAIN_LIMIT,
                    IO_TIMEOUT,
                );
                return Ok(None);
            }
            Some(AfterResponse::KeepAlive) if !connection.pending.is_empty() => {}
            Some(AfterResponse::KeepAlive) => return Ok(Some(connection)),
        }
    }
}

#[cfg(test)]
fn handle_connection_with_timeouts(
    stream: &mut TcpStream,
    peer: SocketAddr,
    state: &Arc<Mutex<OtlpStateStore>>,
    refusals: &Refusals,
    queued: &ForwardWake,
    io_timeout: Duration,
    request_deadline: Duration,
) -> Result<(), String> {
    let after = serve_request(
        stream,
        peer,
        &mut Vec::new(),
        true,
        state,
        refusals,
        queued,
        io_timeout,
        request_deadline,
        &crate::effect_retry::ReceiverBackoff::default(),
    )?;
    if matches!(after, Some(AfterResponse::Close)) {
        crate::http_close::close_after_response(stream, DRAIN_LIMIT, io_timeout);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn serve_request(
    stream: &mut TcpStream,
    peer: SocketAddr,
    pending: &mut Vec<u8>,
    fresh: bool,
    state: &Arc<Mutex<OtlpStateStore>>,
    refusals: &Refusals,
    queued: &ForwardWake,
    io_timeout: Duration,
    request_deadline: Duration,
    backoff: &crate::effect_retry::ReceiverBackoff,
) -> Result<Option<AfterResponse>, String> {
    if !peer.ip().is_loopback() {
        refuse_request(state, refusals, None, DropReason::InvalidRequest, 0, true)?;
        write_response(stream, 403, "Forbidden", "loopback clients only", false);
        return Ok(Some(AfterResponse::Close));
    }
    let request = match read_request(stream, pending, fresh, io_timeout, request_deadline) {
        Ok(Some(request)) => request,
        Ok(None) => return Ok(None),
        Err(rejection) => {
            refuse_request(
                state,
                refusals,
                rejection.signal,
                rejection.reason,
                rejection.dropped_items,
                rejection.terminal_drop,
            )?;
            write_response(
                stream,
                rejection.status,
                rejection.status_text,
                rejection.message,
                false,
            );
            return Ok(Some(AfterResponse::Close));
        }
    };
    let keep_alive = request.keep_alive;
    let after = if keep_alive {
        AfterResponse::KeepAlive
    } else {
        AfterResponse::Close
    };

    let original_bytes = u64::try_from(request.body.len())
        .map_err(|_| "OTLP request body length does not fit u64".to_owned())?;
    let value: Value = match serde_json::from_slice(&request.body) {
        Ok(value) => value,
        Err(_) => {
            refuse_request(
                state,
                refusals,
                Some(request.signal),
                DropReason::InvalidJson,
                1,
                true,
            )?;
            write_response(stream, 400, "Bad Request", "invalid OTLP JSON", false);
            return Ok(Some(AfterResponse::Close));
        }
    };
    let item_count = match validate_signal(request.signal, &value) {
        Ok(item_count) => item_count,
        Err(_) => {
            refuse_request(
                state,
                refusals,
                Some(request.signal),
                DropReason::UnsupportedSignalShape,
                1,
                true,
            )?;
            write_response(
                stream,
                400,
                "Bad Request",
                "unsupported OTLP signal shape",
                false,
            );
            return Ok(Some(AfterResponse::Close));
        }
    };
    let (scrubbed, dropped, scrubbed_fields) = scrub_signal(request.signal, value);
    let item_count = if dropped.is_empty() {
        item_count
    } else {
        record_record_drops(state, request.signal, &dropped, refusals)?;
        signal_item_count(request.signal, &scrubbed)?
    };
    let scrubbed_batch_bytes = encoded_len(&scrubbed)?;
    let fragments = match split_to_cap(request.signal, scrubbed, OUTBOUND_BODY_CAP) {
        Ok(fragments) => fragments,
        Err(_) => {
            refuse_request(
                state,
                refusals,
                Some(request.signal),
                DropReason::UnsplittableItem,
                item_count,
                true,
            )?;
            write_response(
                stream,
                413,
                "Content Too Large",
                "OTLP item cannot fit the forwarding cap",
                false,
            );
            return Ok(Some(AfterResponse::Close));
        }
    };
    let mut item_start = 0_u64;
    let mut splits = Vec::with_capacity(fragments.len());
    for fragment in fragments {
        let count = signal_item_count(request.signal, &fragment)?;
        if count == 0 {
            continue;
        }
        let item_end = item_start
            .checked_add(count)
            .ok_or_else(|| "OTLP split item range overflow".to_owned())?;
        let body = serde_json::to_vec(&fragment)
            .map_err(|error| format!("cannot encode OTLP split: {error}"))?;
        splits.push(PendingSplit {
            item_start,
            item_end,
            body,
        });
        item_start = item_end;
    }
    if item_start != item_count {
        return Err("OTLP split mapping did not preserve every signal item".to_owned());
    }

    let mut state = state
        .lock()
        .map_err(|_| "OTLP edge state lock is poisoned".to_owned())?;
    if !state.can_enqueue(splits.len()) {
        publish_warning(state.record_rejection(
            Some(request.signal),
            DropReason::QueueCapacity,
            item_count,
            false,
        )?);
        drop(state);
        refusals.push([OtlpRefusal::Request {
            signal: Some(request.signal),
            kind: DropReason::QueueCapacity,
        }])?;
        write_response_after(
            stream,
            503,
            "Service Unavailable",
            "OTLP durable queue is full",
            keep_alive,
            Some(backoff.refused()),
        );
        return Ok(Some(after));
    }
    let custody = !splits.is_empty();
    if state.queue_len() == 0 {
        backoff.accepted();
    }
    publish_warning(
        state.enqueue(
            request.signal,
            splits,
            original_bytes,
            u64::try_from(scrubbed_batch_bytes)
                .map_err(|_| "scrubbed OTLP size does not fit u64".to_owned())?,
            scrubbed_fields,
        )?,
    );
    drop(state);
    if custody {
        queued.ring();
    }
    write_response(stream, 200, "OK", "", keep_alive);
    Ok(Some(after))
}

fn forwarder_loop(
    delivery: &mut impl FnMut(OtlpSignal, &str, &[u8]) -> OtlpDelivery,
    state: &Arc<Mutex<OtlpStateStore>>,
    stop: &AtomicBool,
    health: &EdgeHealth,
    rung: &mpsc::Receiver<()>,
) {
    let mut current: Option<(OtlpSignal, String, Arc<[u8]>)> = None;
    loop {
        while rung.try_recv().is_ok() {}
        if stop.load(Ordering::SeqCst) {
            return;
        }
        if current.is_none() {
            let item = match state.lock() {
                Ok(state) => state.front(),
                Err(_) => {
                    set_unhealthy(
                        health,
                        source::FORWARDER_STATE_POISONED,
                        "OTLP edge state lock is poisoned".to_owned(),
                    );
                    return;
                }
            };
            let Some(item) = item else {
                if rung.recv().is_err() {
                    return;
                }
                continue;
            };
            if item.body.len() > OUTBOUND_BODY_CAP {
                set_unhealthy(
                    health,
                    source::FORWARDER_SPLIT_OVERSIZED,
                    "durable OTLP split exceeds the forwarding cap".to_owned(),
                );
                return;
            }
            current = Some((item.signal, item.id, item.body));
        }
        let Some((signal, id, body)) = current.as_ref() else {
            continue;
        };
        match delivery(*signal, id, body) {
            OtlpDelivery::Recorded => {
                let result = state
                    .lock()
                    .map_err(|_| "OTLP edge state lock is poisoned".to_owned())
                    .and_then(|mut state| state.acknowledge_forward(id));
                match result {
                    Ok(warning) => publish_warning(warning),
                    Err(error) => {
                        set_unhealthy(health, source::FORWARDER_ACKNOWLEDGE_FAILED, error);
                        return;
                    }
                }
                current = None;
            }
            OtlpDelivery::Failed(failure) => {
                set_unhealthy(health, failure.detail(), failure.to_string());
                return;
            }
            OtlpDelivery::Held => {
                if rung.recv().is_err() {
                    return;
                }
            }
        }
    }
}

fn set_unhealthy(health: &EdgeHealth, step: circular_actors::FailureDetail, error: String) {
    let failure = StartFailure::new(step, error);
    let first = match health.first.lock() {
        Ok(mut slot) if slot.is_none() => {
            *slot = Some(failure.clone());
            true
        }
        Ok(_) => false,
        Err(_) => true,
    };
    if first {
        (health.fallen)(failure);
    }
}

fn record_record_drops(
    state: &Arc<Mutex<OtlpStateStore>>,
    signal: OtlpSignal,
    dropped: &[ScrubError],
    refusals: &Refusals,
) -> Result<(), String> {
    let count = u64::try_from(dropped.len())
        .map_err(|_| "OTLP dropped record count does not fit u64".to_owned())?;
    record_rejection(state, Some(signal), DropReason::ScrubFailure, count, true)?;
    refusals.push(dropped.iter().map(|error| OtlpRefusal::Record {
        signal,
        error: *error,
    }))
}

/// Scrub one OTLP envelope, dropping only the signal items the scrubber cannot
/// classify. `validate_signal` has already accepted the shape, so every layer
/// below is the shape this walk expects.
///
/// A shared part -- a resource block, a scope block, a metric header -- that
/// cannot be scrubbed takes the items underneath it with it, because those
/// records have no classifiable envelope left to travel in. The unscrubbed part
/// itself never leaves this function.
fn scrub_signal(signal: OtlpSignal, mut value: Value) -> (Value, Vec<ScrubError>, u64) {
    let mut drops = Vec::new();
    let mut scrubbed_fields = 0_u64;
    let Some(Value::Array(resources)) = value
        .as_object_mut()
        .and_then(|object| object.get_mut(root_key(signal)))
    else {
        return (value, drops, scrubbed_fields);
    };
    let taken = std::mem::take(resources);
    let mut kept = Vec::with_capacity(taken.len());
    for mut resource in taken {
        let scopes = take_repeated(&mut resource, scope_key(signal));
        if let Err(error) = scrub_shared(&mut resource, &mut scrubbed_fields) {
            let under = scopes.as_deref().unwrap_or_default();
            push_drops(&mut drops, error, items_under_scopes(signal, under));
            continue;
        }
        if let Some(scopes) = scopes {
            let mut kept_scopes = Vec::with_capacity(scopes.len());
            for mut scope in scopes {
                let items = take_repeated(&mut scope, item_container_key(signal));
                if let Err(error) = scrub_shared(&mut scope, &mut scrubbed_fields) {
                    let under = items.as_deref().unwrap_or_default();
                    push_drops(&mut drops, error, items_under_container(signal, under));
                    continue;
                }
                if let Some(items) = items {
                    let kept_items = match signal {
                        OtlpSignal::Logs => scrub_records(items, &mut drops, &mut scrubbed_fields),
                        OtlpSignal::Metrics => {
                            scrub_metrics(items, &mut drops, &mut scrubbed_fields)
                        }
                    };
                    put_repeated(&mut scope, item_container_key(signal), kept_items);
                }
                kept_scopes.push(scope);
            }
            put_repeated(&mut resource, scope_key(signal), kept_scopes);
        }
        kept.push(resource);
    }
    if let Some(Value::Array(resources)) = value
        .as_object_mut()
        .and_then(|object| object.get_mut(root_key(signal)))
    {
        *resources = kept;
    }
    (value, drops, scrubbed_fields)
}

/// Scrub the part of a container that is not its repeated signal items. The
/// items are already out of `value`, so this verdict is never a record's.
fn scrub_shared(value: &mut Value, scrubbed_fields: &mut u64) -> Result<(), ScrubError> {
    let shared = std::mem::replace(value, Value::Null);
    let scrubbed = match PreLedgerScrubber.scrub(shared.clone()) {
        Ok(scrubbed) => scrubbed,
        Err(error) => {
            *value = shared;
            return Err(error);
        }
    };
    let (shared, count) = scrubbed.into_parts();
    *value = shared;
    *scrubbed_fields = scrubbed_fields.saturating_add(count);
    Ok(())
}

fn scrub_records(
    items: Vec<Value>,
    drops: &mut Vec<ScrubError>,
    scrubbed_fields: &mut u64,
) -> Vec<Value> {
    let mut kept = Vec::with_capacity(items.len());
    for item in items {
        match PreLedgerScrubber.scrub(item) {
            Ok(scrubbed) => {
                let (item, count) = scrubbed.into_parts();
                *scrubbed_fields = scrubbed_fields.saturating_add(count);
                kept.push(item);
            }
            Err(error) => drops.push(error),
        }
    }
    kept
}

/// A metric's signal items are its data points. A metric whose header cannot be
/// scrubbed, or whose every point leaves, leaves with them: an emptied metric
/// would otherwise still count as one custody item with nothing in it.
fn scrub_metrics(
    metrics: Vec<Value>,
    drops: &mut Vec<ScrubError>,
    scrubbed_fields: &mut u64,
) -> Vec<Value> {
    let mut kept = Vec::with_capacity(metrics.len());
    for mut metric in metrics {
        let Some(data_key) = metric_data_key(&metric) else {
            match PreLedgerScrubber.scrub(metric) {
                Ok(scrubbed) => {
                    let (metric, count) = scrubbed.into_parts();
                    *scrubbed_fields = scrubbed_fields.saturating_add(count);
                    kept.push(metric);
                }
                Err(error) => drops.push(error),
            }
            continue;
        };
        let points = metric
            .get_mut(data_key)
            .and_then(|data| take_repeated(data, "dataPoints"));
        let declared = points.as_ref().map_or(0, Vec::len).max(1);
        if let Err(error) = scrub_shared(&mut metric, scrubbed_fields) {
            push_drops(drops, error, declared);
            continue;
        }
        let Some(points) = points else {
            kept.push(metric);
            continue;
        };
        let had_points = !points.is_empty();
        let kept_points = scrub_records(points, drops, scrubbed_fields);
        if had_points && kept_points.is_empty() {
            continue;
        }
        if let Some(data) = metric.get_mut(data_key) {
            put_repeated(data, "dataPoints", kept_points);
        }
        kept.push(metric);
    }
    kept
}

fn metric_data_key(metric: &Value) -> Option<&'static str> {
    let metric = metric.as_object()?;
    let fields = metric_data_fields(metric);
    (fields.len() == 1).then(|| fields[0])
}

fn take_repeated(value: &mut Value, key: &str) -> Option<Vec<Value>> {
    match value.as_object_mut().and_then(|fields| fields.remove(key)) {
        Some(Value::Array(values)) => Some(values),
        Some(other) => {
            if let Some(fields) = value.as_object_mut() {
                fields.insert(key.to_owned(), other);
            }
            None
        }
        None => None,
    }
}

fn put_repeated(value: &mut Value, key: &str, items: Vec<Value>) {
    if let Some(fields) = value.as_object_mut() {
        fields.insert(key.to_owned(), Value::Array(items));
    }
}

fn push_drops(drops: &mut Vec<ScrubError>, error: ScrubError, count: usize) {
    for _ in 0..count {
        drops.push(error);
    }
}

fn items_under_scopes(signal: OtlpSignal, scopes: &[Value]) -> usize {
    scopes
        .iter()
        .map(|scope| {
            let items = scope
                .get(item_container_key(signal))
                .and_then(Value::as_array)
                .map_or([].as_slice(), Vec::as_slice);
            items_under_container(signal, items)
        })
        .sum()
}

fn items_under_container(signal: OtlpSignal, items: &[Value]) -> usize {
    match signal {
        OtlpSignal::Logs => items.len(),
        OtlpSignal::Metrics => items
            .iter()
            .map(|metric| {
                metric_data_key(metric)
                    .and_then(|key| metric.get(key))
                    .and_then(|data| data.get("dataPoints"))
                    .and_then(Value::as_array)
                    .map_or(0, Vec::len)
                    .max(1)
            })
            .sum(),
    }
}

fn refuse_request(
    state: &Arc<Mutex<OtlpStateStore>>,
    refusals: &Refusals,
    signal: Option<OtlpSignal>,
    kind: DropReason,
    dropped_items: u64,
    terminal_drop: bool,
) -> Result<(), String> {
    record_rejection(state, signal, kind, dropped_items, terminal_drop)?;
    refusals.push([OtlpRefusal::Request { signal, kind }])
}

fn record_rejection(
    state: &Arc<Mutex<OtlpStateStore>>,
    signal: Option<OtlpSignal>,
    reason: DropReason,
    dropped_items: u64,
    terminal_drop: bool,
) -> Result<(), String> {
    let mut state = state
        .lock()
        .map_err(|_| "OTLP edge state lock is poisoned".to_owned())?;
    publish_warning(state.record_rejection(signal, reason, dropped_items, terminal_drop)?);
    Ok(())
}

struct IncomingRequest {
    signal: OtlpSignal,
    body: Vec<u8>,
    keep_alive: bool,
}

#[derive(Clone, Copy, Debug)]
struct RequestRejection {
    signal: Option<OtlpSignal>,
    reason: DropReason,
    status: u16,
    status_text: &'static str,
    message: &'static str,
    dropped_items: u64,
    terminal_drop: bool,
}

impl RequestRejection {
    const fn bad_request(message: &'static str) -> Self {
        Self {
            signal: None,
            reason: DropReason::InvalidRequest,
            status: 400,
            status_text: "Bad Request",
            message,
            dropped_items: 1,
            terminal_drop: true,
        }
    }

    const fn timeout(signal: Option<OtlpSignal>) -> Self {
        Self {
            signal,
            reason: DropReason::Timeout,
            status: 408,
            status_text: "Request Timeout",
            message: "request body did not arrive within the receiver deadline",
            dropped_items: 1,
            terminal_drop: false,
        }
    }
}

fn read_request(
    stream: &mut TcpStream,
    pending: &mut Vec<u8>,
    fresh: bool,
    io_timeout: Duration,
    request_deadline: Duration,
) -> Result<Option<IncomingRequest>, RequestRejection> {
    stream
        .set_read_timeout(Some(io_timeout))
        .map_err(|_| RequestRejection::bad_request("cannot set request timeout"))?;
    stream
        .set_write_timeout(Some(io_timeout))
        .map_err(|_| RequestRejection::bad_request("cannot set response timeout"))?;
    let started = Instant::now();
    let mut bytes = std::mem::take(pending);
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break position + 4;
        }
        if bytes.len() > HEADER_LIMIT {
            return Err(RequestRejection {
                status: 431,
                status_text: "Request Header Fields Too Large",
                message: "request headers exceed the receiver limit",
                ..RequestRejection::bad_request("")
            });
        }
        if started.elapsed() >= request_deadline {
            return Err(RequestRejection::timeout(None));
        }
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if is_socket_timeout(&error) => {
                if started.elapsed() >= request_deadline {
                    return Err(RequestRejection::timeout(None));
                }
                continue;
            }
            Err(_) if bytes.is_empty() && !fresh => return Ok(None),
            Err(_) => return Err(RequestRejection::bad_request("cannot read request")),
        };
        if started.elapsed() >= request_deadline {
            return Err(RequestRejection::timeout(None));
        }
        if read == 0 {
            if bytes.is_empty() && !fresh {
                return Ok(None);
            }
            return Err(RequestRejection::bad_request(
                "request ended before headers",
            ));
        }
        bytes.extend_from_slice(&buffer[..read]);
    };
    if header_end > HEADER_LIMIT {
        return Err(RequestRejection {
            status: 431,
            status_text: "Request Header Fields Too Large",
            message: "request headers exceed the receiver limit",
            ..RequestRejection::bad_request("")
        });
    }
    let header_text = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| RequestRejection::bad_request("request headers are not UTF-8"))?;
    let mut lines = header_text[..header_text.len() - 4].split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| RequestRejection::bad_request("request line is missing"))?;
    let parts = request_line.split_ascii_whitespace().collect::<Vec<_>>();
    if parts.len() != 3 || !matches!(parts[2], "HTTP/1.0" | "HTTP/1.1") {
        return Err(RequestRejection::bad_request("invalid HTTP request line"));
    }
    let signal = match parts[1] {
        "/v1/logs" => Some(OtlpSignal::Logs),
        "/v1/metrics" => Some(OtlpSignal::Metrics),
        _ => None,
    };
    if signal.is_none() {
        return Err(RequestRejection {
            signal,
            status: 404,
            status_text: "Not Found",
            message: "unknown OTLP path",
            ..RequestRejection::bad_request("")
        });
    }
    if parts[0] != "POST" {
        return Err(RequestRejection {
            signal,
            status: 405,
            status_text: "Method Not Allowed",
            message: "OTLP paths accept POST only",
            ..RequestRejection::bad_request("")
        });
    }
    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| RequestRejection::bad_request("malformed HTTP header"))?;
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty() || headers.insert(name, value.trim()).is_some() {
            return Err(RequestRejection::bad_request(
                "duplicate or empty HTTP header",
            ));
        }
    }
    if headers.get("content-type").copied() != Some("application/json") {
        return Err(RequestRejection {
            signal,
            reason: DropReason::UnsupportedContentType,
            status: 415,
            status_text: "Unsupported Media Type",
            message: "Content-Type must be application/json",
            ..RequestRejection::bad_request("")
        });
    }
    if headers.contains_key("content-encoding") || headers.contains_key("transfer-encoding") {
        return Err(RequestRejection {
            signal,
            reason: DropReason::UnsupportedContentEncoding,
            status: 415,
            status_text: "Unsupported Media Type",
            message: "compressed or transfer-encoded OTLP is unsupported",
            ..RequestRejection::bad_request("")
        });
    }
    let connection = headers
        .get("connection")
        .map(|value| value.to_ascii_lowercase())
        .unwrap_or_default();
    let asks = |token: &str| connection.split(',').any(|part| part.trim() == token);
    let keep_alive = if parts[2] == "HTTP/1.1" {
        !asks("close")
    } else {
        asks("keep-alive")
    };
    let content_length = headers
        .get("content-length")
        .ok_or(RequestRejection {
            signal,
            status: 411,
            status_text: "Length Required",
            message: "Content-Length is required",
            ..RequestRejection::bad_request("")
        })?
        .parse::<usize>()
        .map_err(|_| RequestRejection::bad_request("invalid Content-Length"))?;
    if content_length > RECEIVER_BODY_LIMIT {
        return Err(RequestRejection {
            signal,
            status: 413,
            status_text: "Content Too Large",
            message: "OTLP request body exceeds the receiver limit",
            ..RequestRejection::bad_request("")
        });
    }
    while bytes.len().saturating_sub(header_end) < content_length {
        if started.elapsed() >= request_deadline {
            return Err(RequestRejection::timeout(signal));
        }
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if is_socket_timeout(&error) => {
                if started.elapsed() >= request_deadline {
                    return Err(RequestRejection::timeout(signal));
                }
                continue;
            }
            Err(_) => {
                return Err(RequestRejection {
                    signal,
                    ..RequestRejection::bad_request("cannot read request body")
                });
            }
        };
        if started.elapsed() >= request_deadline {
            return Err(RequestRejection::timeout(signal));
        }
        if read == 0 {
            return Err(RequestRejection::bad_request(
                "request ended before its declared body",
            ));
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    *pending = bytes.split_off(header_end + content_length);
    Ok(Some(IncomingRequest {
        signal: signal.expect("known signal path"),
        body: bytes[header_end..].to_vec(),
        keep_alive,
    }))
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    status_text: &str,
    message: &str,
    keep_alive: bool,
) {
    write_response_after(stream, status, status_text, message, keep_alive, None);
}

fn write_response_after(
    stream: &mut TcpStream,
    status: u16,
    status_text: &str,
    message: &str,
    keep_alive: bool,
    retry_after: Option<u64>,
) {
    let body = if message.is_empty() {
        "{}".to_owned()
    } else {
        serde_json::json!({"error": message}).to_string()
    };
    let connection = if keep_alive { "keep-alive" } else { "close" };
    let retry_after = retry_after
        .map(|seconds| format!("Retry-After: {seconds}\r\n"))
        .unwrap_or_default();
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: application/json\r\n{retry_after}Content-Length: {}\r\nConnection: {connection}\r\n\r\n{body}",
        body.len()
    );
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SignalShapeError;

impl fmt::Display for SignalShapeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("unsupported OTLP signal shape")
    }
}

/// OTLP/HTTP JSON is the protobuf JSON mapping, so a repeated field with no
/// element is written by omitting it: the collector's empty export request is
/// the document `{}`, and an absent `resourceLogs`, `scopeLogs`, `logRecords`,
/// `resourceMetrics`, `scopeMetrics`, `metrics` or `dataPoints` denotes the
/// same message as an empty array. Presence is therefore not part of the
/// accepted shape. A wrong JSON type, a root key belonging to another signal,
/// and an element that is not an object still are.
/// The one envelope name at each depth of a signal. `scrub_signal` walks the
/// same three names this validation does, so the two cannot drift apart.
const fn root_key(signal: OtlpSignal) -> &'static str {
    match signal {
        OtlpSignal::Logs => "resourceLogs",
        OtlpSignal::Metrics => "resourceMetrics",
    }
}

const fn scope_key(signal: OtlpSignal) -> &'static str {
    match signal {
        OtlpSignal::Logs => "scopeLogs",
        OtlpSignal::Metrics => "scopeMetrics",
    }
}

const fn item_container_key(signal: OtlpSignal) -> &'static str {
    match signal {
        OtlpSignal::Logs => "logRecords",
        OtlpSignal::Metrics => "metrics",
    }
}

fn validate_signal(signal: OtlpSignal, value: &Value) -> Result<u64, SignalShapeError> {
    let object = value.as_object().ok_or(SignalShapeError)?;
    let root_key = root_key(signal);
    if object.keys().any(|key| key != root_key) {
        return Err(SignalShapeError);
    }
    let mut item_count = 0_u64;
    for resource in repeated_field(object, root_key)? {
        let resource = resource.as_object().ok_or(SignalShapeError)?;
        let scope_key = scope_key(signal);
        for scope in repeated_field(resource, scope_key)? {
            let scope = scope.as_object().ok_or(SignalShapeError)?;
            match signal {
                OtlpSignal::Logs => {
                    let records = repeated_field(scope, "logRecords")?;
                    if records.iter().any(|record| !record.is_object()) {
                        return Err(SignalShapeError);
                    }
                    item_count = checked_item_count(item_count, records.len())?;
                }
                OtlpSignal::Metrics => {
                    for metric in repeated_field(scope, "metrics")? {
                        validate_metric(metric)?;
                        item_count = item_count
                            .checked_add(metric_item_count(metric)?)
                            .ok_or(SignalShapeError)?;
                    }
                }
            }
        }
    }
    Ok(item_count)
}

/// An omitted repeated field reads as the empty sequence. Any other JSON type
/// under that name remains an unsupported shape.
fn repeated_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a [Value], SignalShapeError> {
    match object.get(key) {
        None => Ok(&[]),
        Some(Value::Array(values)) => Ok(values),
        Some(_) => Err(SignalShapeError),
    }
}

fn validate_metric(metric: &Value) -> Result<(), SignalShapeError> {
    let metric = metric.as_object().ok_or(SignalShapeError)?;
    if !metric.get("name").is_some_and(Value::is_string) {
        return Err(SignalShapeError);
    }
    let data_fields = metric_data_fields(metric);
    if data_fields.len() != 1 {
        return Err(SignalShapeError);
    }
    let data = metric
        .get(data_fields[0])
        .and_then(Value::as_object)
        .ok_or(SignalShapeError)?;
    let points = repeated_field(data, "dataPoints")?;
    if points.iter().any(|point| !point.is_object()) {
        return Err(SignalShapeError);
    }
    Ok(())
}

fn metric_data_fields(metric: &serde_json::Map<String, Value>) -> Vec<&'static str> {
    [
        "gauge",
        "sum",
        "histogram",
        "exponentialHistogram",
        "summary",
    ]
    .into_iter()
    .filter(|name| metric.contains_key(*name))
    .collect()
}

fn checked_item_count(current: u64, count: usize) -> Result<u64, SignalShapeError> {
    current
        .checked_add(u64::try_from(count).map_err(|_| SignalShapeError)?)
        .ok_or(SignalShapeError)
}

fn metric_item_count(metric: &Value) -> Result<u64, SignalShapeError> {
    let metric = metric.as_object().ok_or(SignalShapeError)?;
    let fields = metric_data_fields(metric);
    if fields.len() != 1 {
        return Err(SignalShapeError);
    }
    let data = metric
        .get(fields[0])
        .and_then(Value::as_object)
        .ok_or(SignalShapeError)?;
    let points = repeated_field(data, "dataPoints")?;
    u64::try_from(points.len().max(1)).map_err(|_| SignalShapeError)
}

fn signal_item_count(signal: OtlpSignal, value: &Value) -> Result<u64, String> {
    validate_signal(signal, value).map_err(|error| error.to_string())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SplitError;

fn split_to_cap(signal: OtlpSignal, value: Value, cap: usize) -> Result<Vec<Value>, SplitError> {
    let mut parts = Vec::new();
    split_recursive(signal, value, cap, &mut parts)?;
    Ok(parts)
}

fn split_recursive(
    signal: OtlpSignal,
    value: Value,
    cap: usize,
    parts: &mut Vec<Value>,
) -> Result<(), SplitError> {
    if encoded_len(&value).map_err(|_| SplitError)? <= cap {
        parts.push(value);
        return Ok(());
    }
    let (left, right) = match signal {
        OtlpSignal::Logs => split_logs_once(&value),
        OtlpSignal::Metrics => split_metrics_once(&value),
    }
    .ok_or(SplitError)?;
    split_recursive(signal, left, cap, parts)?;
    split_recursive(signal, right, cap, parts)
}

fn split_logs_once(value: &Value) -> Option<(Value, Value)> {
    let resources = value.get("resourceLogs")?.as_array()?;
    if resources.len() > 1 {
        return split_root_array(value, "resourceLogs", resources);
    }
    let scopes = resources.first()?.get("scopeLogs")?.as_array()?;
    if scopes.len() > 1 {
        let (left_values, right_values) = split_values(scopes)?;
        let mut left = value.clone();
        let mut right = value.clone();
        left["resourceLogs"][0]["scopeLogs"] = Value::Array(left_values);
        right["resourceLogs"][0]["scopeLogs"] = Value::Array(right_values);
        return Some((left, right));
    }
    let records = scopes.first()?.get("logRecords")?.as_array()?;
    let (left_values, right_values) = split_values(records)?;
    let mut left = value.clone();
    let mut right = value.clone();
    left["resourceLogs"][0]["scopeLogs"][0]["logRecords"] = Value::Array(left_values);
    right["resourceLogs"][0]["scopeLogs"][0]["logRecords"] = Value::Array(right_values);
    Some((left, right))
}

fn split_metrics_once(value: &Value) -> Option<(Value, Value)> {
    let resources = value.get("resourceMetrics")?.as_array()?;
    if resources.len() > 1 {
        return split_root_array(value, "resourceMetrics", resources);
    }
    let scopes = resources.first()?.get("scopeMetrics")?.as_array()?;
    if scopes.len() > 1 {
        let (left_values, right_values) = split_values(scopes)?;
        let mut left = value.clone();
        let mut right = value.clone();
        left["resourceMetrics"][0]["scopeMetrics"] = Value::Array(left_values);
        right["resourceMetrics"][0]["scopeMetrics"] = Value::Array(right_values);
        return Some((left, right));
    }
    let metrics = scopes.first()?.get("metrics")?.as_array()?;
    if metrics.len() > 1 {
        let (left_values, right_values) = split_values(metrics)?;
        let mut left = value.clone();
        let mut right = value.clone();
        left["resourceMetrics"][0]["scopeMetrics"][0]["metrics"] = Value::Array(left_values);
        right["resourceMetrics"][0]["scopeMetrics"][0]["metrics"] = Value::Array(right_values);
        return Some((left, right));
    }
    let metric = metrics.first()?.as_object()?;
    let data_fields = metric_data_fields(metric);
    if data_fields.len() != 1 {
        return None;
    }
    let field = data_fields[0];
    let points = metric.get(field)?.get("dataPoints")?.as_array()?;
    let (left_values, right_values) = split_values(points)?;
    let mut left = value.clone();
    let mut right = value.clone();
    left["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0][field]["dataPoints"] =
        Value::Array(left_values);
    right["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0][field]["dataPoints"] =
        Value::Array(right_values);
    Some((left, right))
}

fn split_root_array(value: &Value, key: &str, values: &[Value]) -> Option<(Value, Value)> {
    let (left_values, right_values) = split_values(values)?;
    let mut left = value.clone();
    let mut right = value.clone();
    left[key] = Value::Array(left_values);
    right[key] = Value::Array(right_values);
    Some((left, right))
}

fn split_values(values: &[Value]) -> Option<(Vec<Value>, Vec<Value>)> {
    if values.len() < 2 {
        return None;
    }
    let middle = values.len() / 2;
    Some((values[..middle].to_vec(), values[middle..].to_vec()))
}

fn encoded_len(value: &Value) -> Result<usize, String> {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|error| format!("cannot encode OTLP JSON: {error}"))
}

fn publish_warning(warning: Option<String>) {
    if let Some(warning) = warning {
        eprintln!("circular-daemon: otlp: {warning}");
    }
}

