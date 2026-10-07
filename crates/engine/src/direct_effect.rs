//! External-effect dispatch with an optional durable request owner.
//!
//! Product runs bind the SQLite outbox barrier before first submission. Approved
//! targets record their ticketed term before concrete executor contact. Recovery
//! uses the same EffectId and request; terminal outcomes return to the actor only
//! through the ordinary durable outcome path. Isolated interpreter fixtures may
//! still use the dispatcher without a persistent run.

use circular_runtime::{
    AgentHarnessName, ApprovalGate, Effect, EffectCtor, EffectFailure, EffectOutcome, FsReadGrant,
    FsWriteGrant, HttpFetchGrant, HttpHeaderValue, HttpHosts, HttpMethod, HttpResponse,
    Interpreter, InterpreterFault, LiveInterpreter, NormalizedPath, NotificationChannel,
    NotificationReceipt, NotificationSpec, OutcomePayload, PeerEffect, ProcessResult,
    ProcessTargets, SubmitError, UserNotifyGrant, WorkspaceProcessGrant,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::error::Error;
use std::fmt;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(crate) struct SyncSettlement<I> {
    settled: VecDeque<(EffectOutcome<I>, Option<String>)>,
}

impl<I: Clone + Ord> SyncSettlement<I> {
    pub(crate) fn new() -> Self {
        Self {
            settled: VecDeque::new(),
        }
    }

    pub(crate) fn record(&mut self, correlation: I, result: Result<OutcomePayload, EffectFailure>) {
        self.record_with_detail(correlation, result, None);
    }

    pub(crate) fn record_with_detail(
        &mut self,
        correlation: I,
        result: Result<OutcomePayload, EffectFailure>,
        detail: Option<String>,
    ) {
        self.settled
            .push_back((EffectOutcome::new(correlation, result), detail));
    }

    pub(crate) fn next_outcome(&mut self) -> Option<EffectOutcome<I>> {
        self.next_settled().map(|(outcome, _)| outcome)
    }

    pub(crate) fn next_settled(&mut self) -> Option<(EffectOutcome<I>, Option<String>)> {
        self.settled.pop_front()
    }
}

pub(crate) fn caught_worker<T>(body: impl FnOnce() -> T) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(body))
        .map_err(|payload| worker_panic_detail(&*payload))
}

pub(crate) fn caught_boundary(
    boundary: impl FnOnce() -> (Result<OutcomePayload, EffectFailure>, Option<String>),
) -> (Result<OutcomePayload, EffectFailure>, Option<String>) {
    match caught_worker(boundary) {
        Ok(settled) => settled,
        Err(detail) => (
            Err(EffectFailure::InterpreterFault(InterpreterFault::Other)),
            Some(detail),
        ),
    }
}

pub(crate) const EFFECT_WORKER_PANIC: &str = "effect worker panicked";

pub(crate) const EFFECT_WORKER_LOST: &str = "effect worker vanished without settling";

pub(crate) fn worker_panic_detail(payload: &(dyn std::any::Any + Send)) -> String {
    let message = payload
        .downcast_ref::<&'static str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned());
    let spelled = match message {
        Some(message) => format!("{EFFECT_WORKER_PANIC}: {message}"),
        None => EFFECT_WORKER_PANIC.to_owned(),
    };
    crate::cli_agent::bounded_provider_detail(&spelled)
}

pub(crate) struct EffectWorker<I> {
    handle: std::thread::JoinHandle<()>,
    correlation: I,
    settled: Arc<std::sync::atomic::AtomicBool>,
}

impl<I> EffectWorker<I> {
    pub(crate) fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }

    pub(crate) fn reap(self) -> Option<(I, String)> {
        let joined = self.handle.join();
        if self.settled.load(std::sync::atomic::Ordering::SeqCst) {
            return None;
        }
        match joined {
            Ok(()) => Some((self.correlation, EFFECT_WORKER_LOST.to_owned())),
            Err(payload) => Some((self.correlation, worker_panic_detail(&*payload))),
        }
    }
}

pub(crate) fn spawn_effect_worker<I>(
    name: Option<&str>,
    correlation: I,
    sender: std::sync::mpsc::Sender<(EffectOutcome<I>, Option<String>)>,
    wake: OutcomeWake,
    boundary: impl FnOnce() -> (
        Result<OutcomePayload, EffectFailure>,
        Vec<circular_runtime::AgentProgressRecord>,
        Option<String>,
    ) + Send
    + 'static,
) -> std::io::Result<EffectWorker<I>>
where
    I: Clone + Send + 'static,
{
    let settled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = settled.clone();
    let key = correlation.clone();
    let builder = match name {
        Some(name) => std::thread::Builder::new().name(name.to_owned()),
        None => std::thread::Builder::new(),
    };
    let handle = builder.spawn(move || {
        let (result, failure_progress, detail) = caught_worker(boundary).unwrap_or_else(|detail| {
            (
                Err(EffectFailure::InterpreterFault(InterpreterFault::Other)),
                Vec::new(),
                Some(detail),
            )
        });
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = sender.send((
            EffectOutcome::new(key, result).with_failure_progress(failure_progress),
            detail,
        ));
        wake.notify();
    })?;
    Ok(EffectWorker {
        handle,
        correlation,
        settled,
    })
}

#[cfg(test)]
pub(crate) fn lost_effect_worker<I>(correlation: I, message: &'static str) -> EffectWorker<I> {
    let handle = std::thread::Builder::new()
        .spawn(move || panic!("{message}"))
        .expect("fixture worker starts");
    while !handle.is_finished() {
        std::thread::yield_now();
    }
    EffectWorker {
        handle,
        correlation,
        settled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    }
}

pub(crate) fn reap_effect_workers<I>(workers: &mut Vec<EffectWorker<I>>) -> Vec<(I, String)> {
    let mut lost = Vec::new();
    let mut index = 0;
    while index < workers.len() {
        if workers[index].is_finished() {
            if let Some(fact) = workers.swap_remove(index).reap() {
                lost.push(fact);
            }
        } else {
            index += 1;
        }
    }
    lost
}

/// Completion custody for one-shot effect workers. Effect policy, cancellation,
/// and process leases stay with the interpreter that opens the boundary.
struct EffectCompletions<I> {
    wake: OutcomeWake,
    settlement: SyncSettlement<I>,
    sender: std::sync::mpsc::Sender<(EffectOutcome<I>, Option<String>)>,
    results: std::sync::mpsc::Receiver<(EffectOutcome<I>, Option<String>)>,
    workers: Vec<EffectWorker<I>>,
}

impl<I: Clone + Ord> EffectCompletions<I> {
    fn new() -> Self {
        let (sender, results) = std::sync::mpsc::channel();
        Self {
            wake: OutcomeWake::default(),
            settlement: SyncSettlement::new(),
            sender,
            results,
            workers: Vec::new(),
        }
    }

    fn reap_finished(&mut self) {
        for (correlation, detail) in reap_effect_workers(&mut self.workers) {
            self.settlement.record_with_detail(
                correlation,
                Err(EffectFailure::InterpreterFault(InterpreterFault::Other)),
                Some(detail),
            );
            self.wake.notify();
        }
    }

    fn next(&mut self) -> Option<(EffectOutcome<I>, Option<String>)> {
        self.reap_finished();
        self.settlement
            .next_settled()
            .or_else(|| self.results.try_recv().ok())
    }
}

impl<I: Clone + Ord + Send + 'static> EffectCompletions<I> {
    /// The dispatcher's custody admitted this correlation before any boundary opens.
    fn spawn_claimed(
        &mut self,
        name: Option<&str>,
        correlation: I,
        boundary: impl FnOnce() -> (Result<OutcomePayload, EffectFailure>, Option<String>)
        + Send
        + 'static,
    ) {
        let failed = correlation.clone();
        match spawn_effect_worker(
            name,
            correlation,
            self.sender.clone(),
            self.wake.clone(),
            move || {
                let (result, detail) = boundary();
                (result, Vec::new(), detail)
            },
        ) {
            Ok(worker) => self.workers.push(worker),
            Err(_) => self.settlement.record(
                failed,
                Err(EffectFailure::InterpreterFault(
                    InterpreterFault::ResourceExhausted,
                )),
            ),
        }
    }
}

#[derive(Clone)]
pub enum NotificationEndpoint {
    SlackIncomingWebhook {
        vault: Arc<crate::SecretVault>,
        resource: Box<str>,
        timeout: Duration,
    },
    NotifierProgram {
        program: Box<str>,
        deadline: Duration,
    },
}

impl std::fmt::Debug for NotificationEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SlackIncomingWebhook {
                resource, timeout, ..
            } => f
                .debug_struct("SlackIncomingWebhook")
                .field("resource", resource)
                .field("timeout", timeout)
                .finish(),
            Self::NotifierProgram { program, deadline } => f
                .debug_struct("NotifierProgram")
                .field("program", program)
                .field("deadline", deadline)
                .finish(),
        }
    }
}

/// The non-credential settings every child process inherits.
const CHILD_ENVIRONMENT: [&str; 3] = ["HOME", "PATH", "TMPDIR"];

/// Child processes receive only these explicitly selected non-credential settings.
/// Command-local overrides remain explicit; the daemon's ambient credentials do not.
pub(crate) fn explicit_child_environment(command: &mut Command) {
    inherit_child_environment(command, &CHILD_ENVIRONMENT);
}

/// The same discipline with a caller-owned list of inherited names.
///
/// Every name in `inherited` must be a non-credential setting. The caller owns
/// the list; this function owns the order: clear, inherit, then re-apply the
/// command-local overrides.
pub(crate) fn inherit_child_environment(command: &mut Command, inherited: &[&str]) {
    let explicit = command
        .get_envs()
        .map(|(k, v)| (k.to_os_string(), v.map(|v| v.to_os_string())))
        .collect::<Vec<_>>();
    command.env_clear();
    for &name in inherited {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    for (key, value) in explicit {
        if let Some(value) = value {
            command.env(key, value);
        } else {
            command.env_remove(key);
        }
    }
}

fn permits_http_transport(url: &circular_runtime::HttpUrl) -> bool {
    if url.as_str().starts_with("https://") {
        return true;
    }
    let authority = url.host();
    let host = if let Some(rest) = authority.strip_prefix('[') {
        rest.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

impl NotificationEndpoint {
    fn deliver(
        &self,
        spec: &NotificationSpec,
        cancellation: &SubprocessCancellation,
    ) -> Result<(), EffectFailure> {
        let run = |command: &mut Command, deadline| {
            explicit_child_environment(command);
            command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt as _;
                command.process_group(0);
            }
            let started = Instant::now();
            let mut attempted = false;
            let Some((mut child, _registration)) = cancellation
                .spawn(command, &mut attempted)
                .map_err(|_| EffectFailure::TransportUnreached)?
            else {
                return Err(EffectFailure::InterpreterFault(
                    InterpreterFault::Interrupted,
                ));
            };
            let status =
                match wait_for_subprocess(&mut child, started, Some(deadline), cancellation) {
                    Err(EffectFailure::InterpreterFault(InterpreterFault::Interrupted)) => {
                        return Err(EffectFailure::InterpreterFault(
                            InterpreterFault::Interrupted,
                        ));
                    }
                    Err(_) => return Err(EffectFailure::TransportTerminal),
                    Ok(status) => status,
                };
            if status.success() {
                Ok(())
            } else {
                Err(EffectFailure::TransportTerminal)
            }
        };
        if cancellation.is_cancelled() {
            return Err(EffectFailure::InterpreterFault(
                InterpreterFault::Interrupted,
            ));
        }
        match self {
            Self::SlackIncomingWebhook {
                vault,
                resource,
                timeout,
            } => {
                let denied = || EffectFailure::ParameterDenied {
                    capability: circular_runtime::Capability::UserNotify,
                };
                let material =
                    engine_secrets::engine_integration::vault_action_material(vault, resource)
                        .ok_or_else(denied)?;
                let endpoint = std::str::from_utf8(material).map_err(|_| denied())?;
                let url = circular_runtime::HttpUrl::try_new(endpoint).map_err(|_| denied())?;
                if !permits_http_transport(&url) {
                    return Err(denied());
                }
                let payload = slack_payload(spec);
                let agent = ureq::AgentBuilder::new()
                    .try_proxy_from_env(false)
                    .redirects(0)
                    .timeout(*timeout)
                    .build();
                agent
                    .post(endpoint)
                    .timeout(*timeout)
                    .set("content-type", "application/json; charset=utf-8")
                    .send_string(&payload)
                    .map_err(|error| match error {
                        ureq::Error::Status(429 | 503, _) => EffectFailure::RemoteDeferred,
                        ureq::Error::Status(400..=499, _) => denied(),
                        ureq::Error::Status(..) => EffectFailure::TransportTerminal,
                        ureq::Error::Transport(transport) => http_transport_failure(&transport),
                    })?;
                Ok(())
            }
            Self::NotifierProgram { program, deadline } => run(
                Command::new(program.as_ref())
                    .arg(spec.title())
                    .arg(spec.body()),
                *deadline,
            ),
        }
    }
}

/// Wake the registered completion owner after an asynchronous result is queued.
#[derive(Clone, Default)]
pub(crate) struct OutcomeWake(Arc<Mutex<Option<std::task::Waker>>>);
impl OutcomeWake {
    pub(crate) fn bind(&self, waker: std::task::Waker) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(waker);
    }
    pub(crate) fn notify(&self) {
        let waker = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

type NotificationSettled = (Result<OutcomePayload, EffectFailure>, Option<String>, bool);

type NotificationDelivery = Arc<
    dyn Fn(&Effect, &SubprocessCancellation) -> Result<OutcomePayload, EffectFailure> + Send + Sync,
>;

pub struct NotificationInterpreter<I> {
    outcome_wake: OutcomeWake,
    settlement: SyncSettlement<I>,
    pending: VecDeque<I>,
    deliver: NotificationDelivery,
    jobs: std::sync::mpsc::Sender<Effect>,
    results: std::sync::mpsc::Receiver<NotificationSettled>,
    cancellation: Arc<SubprocessCancellation>,
}

pub(crate) const NOTIFICATION_WORKER_RETIRED: &str = "notification delivery worker is retired";

pub(crate) const NOTIFICATION_WORKER_RESTOOD: &str = "a successor delivery worker stands";

impl<I: Clone + Ord> NotificationInterpreter<I> {
    #[must_use]
    pub fn new(
        grant: &UserNotifyGrant,
        endpoints: BTreeMap<NotificationChannel, NotificationEndpoint>,
    ) -> Self {
        let allowed = grant.parameters().clone();
        Self::with_delivery(move |effect, cancellation| {
            perform_notification(&allowed, &endpoints, effect, cancellation)
        })
    }

    fn with_delivery(
        deliver: impl Fn(&Effect, &SubprocessCancellation) -> Result<OutcomePayload, EffectFailure>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        let outcome_wake = OutcomeWake::default();
        let deliver: NotificationDelivery = Arc::new(deliver);
        let cancellation = Arc::new(SubprocessCancellation::default());
        let (jobs, results) = stand_notification_worker(&deliver, &cancellation, &outcome_wake);
        Self {
            outcome_wake,
            settlement: SyncSettlement::new(),
            pending: VecDeque::new(),
            deliver,
            jobs,
            results,
            cancellation,
        }
    }

    fn restand_delivery_worker(&mut self) {
        for correlation in std::mem::take(&mut self.pending) {
            self.settlement.record_with_detail(
                correlation,
                Err(EffectFailure::InterpreterFault(InterpreterFault::Other)),
                Some(NOTIFICATION_WORKER_RETIRED.to_owned()),
            );
        }
        let (jobs, results) =
            stand_notification_worker(&self.deliver, &self.cancellation, &self.outcome_wake);
        self.jobs = jobs;
        self.results = results;
    }
}

fn stand_notification_worker(
    deliver: &NotificationDelivery,
    cancellation: &Arc<SubprocessCancellation>,
    wake: &OutcomeWake,
) -> (
    std::sync::mpsc::Sender<Effect>,
    std::sync::mpsc::Receiver<NotificationSettled>,
) {
    let (jobs, requests) = std::sync::mpsc::channel::<Effect>();
    let (completed, results) = std::sync::mpsc::channel();
    let deliver = deliver.clone();
    let worker_cancellation = cancellation.clone();
    let completed_wake = wake.clone();
    std::thread::spawn(move || {
        for effect in requests {
            let (result, detail) =
                caught_boundary(|| (deliver(&effect, &worker_cancellation), None));
            let retiring = detail.is_some();
            let detail = detail.map(|detail| format!("{detail} ({NOTIFICATION_WORKER_RETIRED})"));
            if completed.send((result, detail, retiring)).is_err() {
                break;
            }
            completed_wake.notify();
            if retiring {
                break;
            }
        }
    });
    (jobs, results)
}

fn perform_notification(
    allowed: &circular_runtime::NotificationChannels,
    endpoints: &BTreeMap<NotificationChannel, NotificationEndpoint>,
    effect: &Effect,
    cancellation: &SubprocessCancellation,
) -> Result<OutcomePayload, EffectFailure> {
    let Effect::Notify { spec, .. } = effect else {
        return Err(EffectFailure::EndpointGone);
    };
    if !allowed.allows(spec.channel()) {
        return Err(EffectFailure::ParameterDenied {
            capability: circular_runtime::Capability::UserNotify,
        });
    }
    let Some(endpoint) = endpoints.get(spec.channel()) else {
        return Err(EffectFailure::EndpointGone);
    };
    endpoint.deliver(spec, cancellation)?;
    Ok(OutcomePayload::NotificationDelivered(
        NotificationReceipt::delivered(spec.channel().clone()),
    ))
}

impl<I: Clone + Ord> Interpreter<I> for NotificationInterpreter<I> {
    fn set_outcome_waker(&mut self, waker: std::task::Waker) {
        self.outcome_wake.bind(waker);
    }
    fn submit(&mut self, correlation: I, effect: &Effect) -> Result<(), SubmitError<I>> {
        if self.jobs.send(effect.clone()).is_err() {
            self.settlement.record_with_detail(
                correlation,
                Err(EffectFailure::InterpreterFault(InterpreterFault::Other)),
                Some(NOTIFICATION_WORKER_RETIRED.to_owned()),
            );
        } else {
            self.pending.push_back(correlation);
        }
        Ok(())
    }

    fn submit_peer(&mut self, correlation: I, _effect: &PeerEffect) -> Result<(), SubmitError<I>> {
        self.settlement
            .record(correlation, Err(EffectFailure::EndpointGone));
        Ok(())
    }

    fn next_outcome(&mut self) -> Option<EffectOutcome<I>> {
        self.next_outcome_with_detail().map(|(outcome, _)| outcome)
    }

    fn next_outcome_with_detail(&mut self) -> Option<(EffectOutcome<I>, Option<String>)> {
        if let Some(settled) = self.settlement.next_settled() {
            return Some(settled);
        }
        let (result, detail, retired) = match self.results.try_recv() {
            Ok(settled) => settled,
            Err(std::sync::mpsc::TryRecvError::Empty) => return None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => (
                Err(EffectFailure::InterpreterFault(InterpreterFault::Other)),
                Some(NOTIFICATION_WORKER_RETIRED.to_owned()),
                false,
            ),
        };
        let detail = if retired {
            Some(match detail {
                Some(detail) => format!("{detail} ({NOTIFICATION_WORKER_RESTOOD})"),
                None => NOTIFICATION_WORKER_RESTOOD.to_owned(),
            })
        } else {
            detail
        };
        let settled = self.pending.pop_front();
        if retired {
            self.restand_delivery_worker();
        }
        settled.map(|correlation| (EffectOutcome::new(correlation, result), detail))
    }

    fn pending_process_outcomes(&self) -> usize {
        self.cancellation.live_process_groups()
    }

    fn pause(&mut self, force: bool) {
        if force {
            self.cancellation.interrupt();
        }
    }

    fn resume(&mut self) {
        self.cancellation.resume();
    }

    fn begin_cancel(&mut self) {
        self.cancellation.cancel();
    }
}

impl<I> Drop for NotificationInterpreter<I> {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

fn slack_payload(spec: &NotificationSpec) -> String {
    serde_json::json!({ "text": format!("*{}*\n{}", spec.title(), spec.body()) }).to_string()
}

pub struct HttpFetchInterpreter<I> {
    hosts: HttpHosts,
    max_body_bytes: usize,
    timeout: Duration,
    vault: Option<Arc<crate::SecretVault>>,
    completions: EffectCompletions<I>,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

impl<I: Clone + Ord> HttpFetchInterpreter<I> {
    #[must_use]
    pub fn new(
        grant: &HttpFetchGrant,
        max_body_bytes: usize,
        timeout: Duration,
        vault: Option<Arc<crate::SecretVault>>,
    ) -> Self {
        Self {
            hosts: grant.parameters().clone(),
            max_body_bytes,
            timeout,
            vault,
            completions: EffectCompletions::new(),
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}

fn http_transport_failure(transport: &ureq::Transport) -> EffectFailure {
    match transport.kind() {
        ureq::ErrorKind::Dns | ureq::ErrorKind::ConnectionFailed => {
            EffectFailure::TransportUnreached
        }
        _ => EffectFailure::TransportTerminal,
    }
}

struct HttpFetchBoundary {
    hosts: HttpHosts,
    max_body_bytes: usize,
    timeout: Duration,
    vault: Option<Arc<crate::SecretVault>>,
}

impl HttpFetchBoundary {
    fn perform(&self, effect: &Effect) -> Result<OutcomePayload, EffectFailure> {
        let Effect::Http { spec, .. } = effect else {
            return Err(EffectFailure::EndpointGone);
        };
        if !self.hosts.allows(spec.url().host()) || !permits_http_transport(spec.url()) {
            return Err(EffectFailure::ParameterDenied {
                capability: circular_runtime::Capability::HttpFetch,
            });
        }

        let agent = ureq::AgentBuilder::new()
            .try_proxy_from_env(false)
            .redirects(0)
            .timeout(self.timeout)
            .build();
        let mut request = match spec.method() {
            HttpMethod::Get => agent.get(spec.url().as_str()),
            HttpMethod::Post => agent.post(spec.url().as_str()),
        }
        .timeout(self.timeout);
        for header in spec.headers() {
            let value = match header.value() {
                HttpHeaderValue::Plain(value) => value.as_ref(),
                HttpHeaderValue::Secret(name) => {
                    let Some(vault) = self.vault.as_ref() else {
                        return Err(EffectFailure::ParameterDenied {
                            capability: circular_runtime::Capability::HttpFetch,
                        });
                    };
                    let Some(material) =
                        engine_secrets::engine_integration::vault_action_material(vault, name)
                    else {
                        return Err(EffectFailure::ParameterDenied {
                            capability: circular_runtime::Capability::HttpFetch,
                        });
                    };
                    let value = std::str::from_utf8(material).map_err(|_| {
                        EffectFailure::ParameterDenied {
                            capability: circular_runtime::Capability::HttpFetch,
                        }
                    })?;
                    if value.bytes().any(|b| b.is_ascii_control() && b != b'\t') {
                        return Err(EffectFailure::ParameterDenied {
                            capability: circular_runtime::Capability::HttpFetch,
                        });
                    }
                    value
                }
            };
            request = request.set(header.name(), value);
        }
        let response = match match spec.method() {
            HttpMethod::Get => request.call(),
            HttpMethod::Post => request.send_bytes(spec.body()),
        } {
            Ok(response) | Err(ureq::Error::Status(_, response)) => response,
            Err(ureq::Error::Transport(transport)) => {
                return Err(http_transport_failure(&transport));
            }
        };
        let status = response.status();
        let retry_after_seconds = response
            .header("retry-after")
            .and_then(|value| value.trim().parse::<u64>().ok());
        let (body, truncated) = read_bounded_http(response.into_reader(), self.max_body_bytes)
            .map_err(|_| EffectFailure::TransportTerminal)?;
        reject_secret_http_output(self.vault.as_deref(), &body)?;
        Ok(OutcomePayload::HttpResponse(HttpResponse::new(
            status,
            body,
            truncated,
            retry_after_seconds,
        )))
    }
}

impl<I: Clone + Ord + Send + 'static> Interpreter<I> for HttpFetchInterpreter<I> {
    fn set_outcome_waker(&mut self, waker: std::task::Waker) {
        self.completions.wake.bind(waker);
    }

    fn submit(&mut self, correlation: I, effect: &Effect) -> Result<(), SubmitError<I>> {
        self.completions.reap_finished();
        if self.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
            self.completions.settlement.record(
                correlation,
                Err(EffectFailure::InterpreterFault(
                    InterpreterFault::Interrupted,
                )),
            );
            return Ok(());
        }
        let boundary = HttpFetchBoundary {
            hosts: self.hosts.clone(),
            max_body_bytes: self.max_body_bytes,
            timeout: self.timeout,
            vault: self.vault.clone(),
        };
        let effect = effect.clone();
        let cancelled = self.cancelled.clone();
        self.completions
            .spawn_claimed(Some("http-fetch-effect"), correlation, move || {
                let result = if cancelled.load(std::sync::atomic::Ordering::SeqCst) {
                    Err(EffectFailure::InterpreterFault(
                        InterpreterFault::Interrupted,
                    ))
                } else {
                    boundary.perform(&effect)
                };
                (result, None)
            });
        Ok(())
    }

    fn submit_peer(&mut self, correlation: I, _effect: &PeerEffect) -> Result<(), SubmitError<I>> {
        self.completions
            .settlement
            .record(correlation, Err(EffectFailure::EndpointGone));
        Ok(())
    }

    fn next_outcome(&mut self) -> Option<EffectOutcome<I>> {
        self.next_outcome_with_detail().map(|(outcome, _)| outcome)
    }

    fn next_outcome_with_detail(&mut self) -> Option<(EffectOutcome<I>, Option<String>)> {
        self.completions.next()
    }

    fn begin_cancel(&mut self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl<I> Drop for HttpFetchInterpreter<I> {
    fn drop(&mut self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

fn reject_secret_http_output(
    vault: Option<&crate::SecretVault>,
    body: &[u8],
) -> Result<(), EffectFailure> {
    if vault.is_some_and(|vault| vault.contains_material(body)) {
        return Err(EffectFailure::ParameterDenied {
            capability: circular_runtime::Capability::HttpFetch,
        });
    }
    Ok(())
}

fn read_bounded_http(mut reader: impl Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut captured = Vec::with_capacity(limit.min(8 * 1024));
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        if captured.len() == limit {
            let mut probe = [0_u8; 1];
            return Ok((captured, reader.read(&mut probe)? != 0));
        }
        let remaining = limit - captured.len();
        let chunk_limit = remaining.min(chunk.len());
        let read = reader.read(&mut chunk[..chunk_limit])?;
        if read == 0 {
            return Ok((captured, false));
        }
        captured.extend_from_slice(&chunk[..read]);
    }
}

type ProcessLease = Arc<crate::process_slots::ProcessPermit>;
type ProcessLeaseReader<I> = Arc<dyn Fn(&I) -> Option<ProcessLease> + Send + Sync>;
type ProcessWorkerLeases = Arc<Mutex<BTreeMap<circular_runtime::EffectId, ProcessLease>>>;

pub struct AllowlistedProcessInterpreter<I> {
    completions: EffectCompletions<I>,
    pending: BTreeSet<I>,
    lease_reader: Option<ProcessLeaseReader<I>>,
    targets: ProcessTargets,
    workspace: NormalizedPath,
    max_output_bytes: usize,
    deadline: Duration,
    cancellation: Arc<SubprocessCancellation>,
}

impl<I: Clone + Ord> AllowlistedProcessInterpreter<I> {
    #[must_use]
    pub fn new(
        grant: &WorkspaceProcessGrant,
        workspace: NormalizedPath,
        max_output_bytes: usize,
        deadline: Duration,
    ) -> Self {
        Self {
            completions: EffectCompletions::new(),
            pending: BTreeSet::new(),
            lease_reader: None,
            targets: grant.parameters().targets().clone(),
            workspace,
            max_output_bytes,
            deadline,
            cancellation: Arc::new(SubprocessCancellation::default()),
        }
    }
}

fn perform_process(
    targets: &ProcessTargets,
    workspace: &NormalizedPath,
    max_output_bytes: usize,
    deadline: Duration,
    effect: &Effect,
    cancellation: &SubprocessCancellation,
) -> Result<OutcomePayload, EffectFailure> {
    if cancellation.is_cancelled() {
        return Err(EffectFailure::InterpreterFault(
            InterpreterFault::Interrupted,
        ));
    }
    let Effect::Spawn { spec, .. } = effect else {
        return Err(EffectFailure::EndpointGone);
    };
    if !targets.allows(spec.program()) || !Path::new(spec.program().as_str()).is_absolute() {
        return Err(EffectFailure::ParameterDenied {
            capability: circular_runtime::Capability::ProcessSpawn,
        });
    }
    let mut command = Command::new(spec.program().as_str());
    command
        .args(spec.arguments())
        .current_dir(workspace.as_path())
        .env_clear();
    let mut attempted = false;
    let driven = drive_cancellable_subprocess(
        &mut command,
        Some(spec.stdin()),
        max_output_bytes,
        Some(deadline),
        false,
        cancellation,
        &mut attempted,
    )?;
    Ok(OutcomePayload::ProcessResult(ProcessResult::direct(
        driven.status.code().unwrap_or(-1),
        driven.stdout,
        driven.stderr,
    )))
}

impl<I: Clone + Ord + Send + 'static> Interpreter<I> for AllowlistedProcessInterpreter<I> {
    fn set_outcome_waker(&mut self, waker: std::task::Waker) {
        self.completions.wake.bind(waker);
    }
    fn submit(&mut self, correlation: I, effect: &Effect) -> Result<(), SubmitError<I>> {
        self.completions.reap_finished();
        self.pending.insert(correlation.clone());
        let lease = self
            .lease_reader
            .as_ref()
            .and_then(|read| read(&correlation));
        if self.cancellation.is_cancelled() {
            self.completions.settlement.record(
                correlation,
                Err(EffectFailure::InterpreterFault(
                    InterpreterFault::Interrupted,
                )),
            );
        } else {
            let targets = self.targets.clone();
            let workspace = self.workspace.clone();
            let effect = effect.clone();
            let output = self.max_output_bytes;
            let deadline = self.deadline;
            let cancellation = self.cancellation.clone();
            self.completions.spawn_claimed(None, correlation, move || {
                let _lease = lease;
                let result = perform_process(
                    &targets,
                    &workspace,
                    output,
                    deadline,
                    &effect,
                    &cancellation,
                );
                (result, None)
            });
        }
        Ok(())
    }

    fn submit_peer(&mut self, correlation: I, _effect: &PeerEffect) -> Result<(), SubmitError<I>> {
        self.pending.insert(correlation.clone());
        self.completions
            .settlement
            .record(correlation, Err(EffectFailure::EndpointGone));
        Ok(())
    }

    fn next_outcome(&mut self) -> Option<EffectOutcome<I>> {
        self.next_outcome_with_detail().map(|(outcome, _)| outcome)
    }

    fn next_outcome_with_detail(&mut self) -> Option<(EffectOutcome<I>, Option<String>)> {
        let (outcome, detail) = self.completions.next()?;
        self.pending.remove(outcome.correlation());
        Some((outcome, detail))
    }

    fn pending_process_outcomes(&self) -> usize {
        self.pending.len()
    }

    fn pause(&mut self, force: bool) {
        if force {
            self.cancellation.interrupt();
        }
    }

    fn resume(&mut self) {
        self.cancellation.resume();
    }

    fn begin_cancel(&mut self) {
        self.cancellation.cancel();
    }
}

impl<I> Drop for AllowlistedProcessInterpreter<I> {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

pub(crate) struct DrivenSubprocess {
    pub(crate) status: std::process::ExitStatus,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) stderr_truncated: bool,
}

/// One cancellation domain for the subprocesses owned by an asynchronous
/// interpreter.
///
/// The cancelled bit and process-group registration share one lock. Therefore
/// cancellation either wins before `spawn` (and no process is created), or it
/// observes the freshly registered group and terminates it. There is no
/// unregistered post-spawn window between those cases.
#[derive(Default)]
pub(crate) struct SubprocessCancellation {
    state: Arc<Mutex<SubprocessCancellationState>>,
    settled: Arc<std::sync::Condvar>,
}

#[derive(Default)]
struct SubprocessCancellationState {
    cancelled: bool,
    terminal: bool,
    process_groups: BTreeMap<u32, ProcessGroupTermination>,
}

impl SubprocessCancellation {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancelled
    }

    pub(crate) fn termination_requested(&self, process_group: u32) -> bool {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .process_groups
            .get(&process_group)
            .is_some_and(|group| group.until.is_some())
    }

    pub(crate) fn live_process_groups(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .process_groups
            .len()
    }

    fn spawn<'cancel>(
        &'cancel self,
        command: &mut Command,
        attempted: &mut bool,
    ) -> Result<Option<(std::process::Child, ProcessGroupRegistration<'cancel>)>, EffectFailure>
    {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.cancelled {
            return Ok(None);
        }
        *attempted = true;
        let child = command.spawn().map_err(subprocess_io_failure)?;
        let process_group = child.id();
        state
            .process_groups
            .insert(process_group, ProcessGroupTermination::default());
        Ok(Some((
            child,
            ProcessGroupRegistration {
                cancellation: self,
                process_group,
            },
        )))
    }

    pub(crate) fn resume(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state.terminal {
            state.cancelled = false;
        }
    }

    pub(crate) fn interrupt(&self) {
        self.stop(false);
    }

    pub(crate) fn cancel(&self) {
        self.stop(true);
    }

    fn deadline(&self, process_group: u32, at: Instant) -> SubprocessDeadline<'_> {
        let done = Arc::new(AtomicBool::new(false));
        let fired = Arc::new(AtomicBool::new(false));
        let state = Arc::clone(&self.state);
        let settled = Arc::clone(&self.settled);
        let worker_done = Arc::clone(&done);
        let worker_fired = Arc::clone(&fired);
        let worker = std::thread::Builder::new()
            .name("subprocess-deadline".to_owned())
            .spawn(move || {
                let mut guard = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                loop {
                    if worker_done.load(Ordering::SeqCst)
                        || !guard.process_groups.contains_key(&process_group)
                    {
                        return;
                    }
                    let now = Instant::now();
                    if now < at {
                        guard = settled
                            .wait_timeout(guard, at - now)
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .0;
                        continue;
                    }
                    worker_fired.store(true, Ordering::SeqCst);
                    let Some(Ok(until)) = guard
                        .process_groups
                        .get_mut(&process_group)
                        .map(|group| group.request(process_group))
                    else {
                        return;
                    };
                    drop(guard);
                    std::thread::sleep(until.saturating_duration_since(Instant::now()));
                    if let Some(group) = state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .process_groups
                        .get_mut(&process_group)
                    {
                        let _ = group.escalate(process_group);
                    }
                    return;
                }
            })
            .ok();
        SubprocessDeadline {
            cancellation: self,
            done,
            fired,
            worker,
        }
    }

    fn terminate(&self, process_group: u32) -> std::io::Result<()> {
        let until = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let group = state
                .process_groups
                .get_mut(&process_group)
                .expect("worker retains its process-group registration until reap");
            group.request(process_group)?
        };
        std::thread::sleep(until.saturating_duration_since(Instant::now()));
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .process_groups
            .get_mut(&process_group)
            .expect("worker retains unreaped group leader through escalation")
            .escalate(process_group)
    }

    fn stop(&self, terminal: bool) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.cancelled = true;
        state.terminal |= terminal;
        let mut requested = false;
        for (process_group, group) in &mut state.process_groups {
            requested |= group.until.is_none();
            let _ = group.request(*process_group);
        }
        if requested {
            let shared = Arc::clone(&self.state);
            std::thread::spawn(move || {
                std::thread::sleep(SUBPROCESS_TERMINATION_GRACE);
                let mut state = shared
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                for (id, group) in &mut state.process_groups {
                    let _ = group.escalate(*id);
                }
            });
        }
    }
}

struct SubprocessDeadline<'cancel> {
    cancellation: &'cancel SubprocessCancellation,
    done: Arc<AtomicBool>,
    fired: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl SubprocessDeadline<'_> {
    fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.fired)
    }
}

impl Drop for SubprocessDeadline<'_> {
    fn drop(&mut self) {
        {
            let _state = self
                .cancellation
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.done.store(true, Ordering::SeqCst);
        }
        self.cancellation.settled.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

const SUBPROCESS_TERMINATION_GRACE: Duration = Duration::from_millis(100);

#[derive(Default)]
struct ProcessGroupTermination {
    until: Option<Instant>,
    term: Option<std::io::Result<()>>,
    kill: Option<std::io::Result<()>>,
}

impl ProcessGroupTermination {
    fn request(&mut self, id: u32) -> std::io::Result<Instant> {
        let until = *self
            .until
            .get_or_insert_with(|| Instant::now() + SUBPROCESS_TERMINATION_GRACE);
        signal_process_group_once(&mut self.term, || signal_process_group(id, false))?;
        Ok(until)
    }

    fn escalate(&mut self, id: u32) -> std::io::Result<()> {
        signal_process_group_once(&mut self.kill, || signal_process_group(id, true))
    }
}

fn signal_process_group_once(
    signal: &mut Option<std::io::Result<()>>,
    send: impl FnOnce() -> std::io::Result<()>,
) -> std::io::Result<()> {
    match signal.get_or_insert_with(send) {
        Ok(()) => Ok(()),
        Err(error) => Err(match error.raw_os_error() {
            Some(code) => std::io::Error::from_raw_os_error(code),
            None => std::io::Error::new(error.kind(), error.to_string()),
        }),
    }
}

struct ProcessGroupRegistration<'cancel> {
    cancellation: &'cancel SubprocessCancellation,
    process_group: u32,
}

impl Drop for ProcessGroupRegistration<'_> {
    fn drop(&mut self) {
        self.cancellation
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .process_groups
            .remove(&self.process_group);
        self.cancellation.settled.notify_all();
    }
}

#[cfg(unix)]
fn signal_process_group(process_group: u32, force: bool) -> std::io::Result<()> {
    use nix::errno::Errno;
    use nix::sys::signal::{Signal, killpg};
    use nix::unistd::Pid;

    let process_group = i32::try_from(process_group)
        .map_err(|_| std::io::Error::other("process group id exceeds i32"))?;
    match killpg(
        Pid::from_raw(process_group),
        if force {
            Signal::SIGKILL
        } else {
            Signal::SIGTERM
        },
    ) {
        Ok(()) | Err(Errno::ESRCH) => Ok(()),
        Err(error) => Err(std::io::Error::from_raw_os_error(error as i32)),
    }
}

#[cfg(not(unix))]
fn signal_process_group(_process_group: u32, _force: bool) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "process-group termination is unavailable",
    ))
}

pub(crate) fn drive_cancellable_subprocess(
    command: &mut Command,
    stdin: Option<&[u8]>,
    output_limit: usize,
    deadline: Option<Duration>,
    capture_stderr_tail: bool,
    cancellation: &SubprocessCancellation,
    attempted: &mut bool,
) -> Result<DrivenSubprocess, EffectFailure> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    drive_subprocess_inner(
        command,
        stdin,
        output_limit,
        deadline,
        capture_stderr_tail,
        cancellation,
        attempted,
    )
}

fn drive_subprocess_inner(
    command: &mut Command,
    stdin: Option<&[u8]>,
    output_limit: usize,
    deadline: Option<Duration>,
    capture_stderr_tail: bool,
    cancellation: &SubprocessCancellation,
    attempted: &mut bool,
) -> Result<DrivenSubprocess, EffectFailure> {
    command
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let started = Instant::now();
    let Some((mut child, _registration)) = cancellation.spawn(command, attempted)? else {
        return Err(EffectFailure::InterpreterFault(
            InterpreterFault::Interrupted,
        ));
    };
    let stdout = child.stdout.take().ok_or(EffectFailure::InterpreterFault(
        InterpreterFault::BrokenPipe,
    ))?;
    let stderr = child.stderr.take().ok_or(EffectFailure::InterpreterFault(
        InterpreterFault::BrokenPipe,
    ))?;
    let stdout = std::thread::spawn(move || read_bounded(stdout, output_limit));
    let stderr = std::thread::spawn(move || {
        if capture_stderr_tail {
            read_bounded_tail(stderr, output_limit)
        } else {
            read_bounded(stderr, output_limit).map(|captured| (captured, false))
        }
    });
    if let Some(bytes) = stdin {
        let sink = child.stdin.as_mut().ok_or(EffectFailure::InterpreterFault(
            InterpreterFault::BrokenPipe,
        ))?;
        match sink.write_all(bytes) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
            Err(error) => return Err(subprocess_io_failure(error)),
        }
    }
    drop(child.stdin.take());
    let status = wait_for_subprocess(&mut child, started, deadline, cancellation)?;
    let stdout = stdout
        .join()
        .map_err(|_| EffectFailure::InterpreterFault(InterpreterFault::Other))?
        .map_err(subprocess_io_failure)?;
    let (stderr, stderr_truncated) = stderr
        .join()
        .map_err(|_| EffectFailure::InterpreterFault(InterpreterFault::Other))?
        .map_err(subprocess_io_failure)?;
    Ok(DrivenSubprocess {
        status,
        stdout,
        stderr,
        stderr_truncated,
    })
}

fn wait_for_subprocess(
    child: &mut std::process::Child,
    started: Instant,
    deadline: Option<Duration>,
    cancellation: &SubprocessCancellation,
) -> Result<std::process::ExitStatus, EffectFailure> {
    if cancellation.is_cancelled() || cancellation.termination_requested(child.id()) {
        interrupt_subprocess(child, Some(cancellation))?;
        return Err(EffectFailure::InterpreterFault(
            InterpreterFault::Interrupted,
        ));
    }
    let group = child.id();
    let armed = deadline.map(|deadline| cancellation.deadline(group, started + deadline));
    let fired = armed.as_ref().map(SubprocessDeadline::flag);
    let status = child.wait().map_err(subprocess_io_failure);
    drop(armed);
    let status = status?;
    if fired.is_some_and(|fired| fired.load(Ordering::SeqCst))
        || cancellation.termination_requested(group)
    {
        return Err(EffectFailure::InterpreterFault(
            InterpreterFault::Interrupted,
        ));
    }
    Ok(status)
}

fn interrupt_subprocess(
    child: &mut std::process::Child,
    cancellation: Option<&SubprocessCancellation>,
) -> Result<(), EffectFailure> {
    #[cfg(unix)]
    let killed = if let Some(cancellation) = cancellation {
        cancellation.terminate(child.id())
    } else {
        child.kill()
    };
    #[cfg(not(unix))]
    let killed = {
        let _ = cancellation;
        child.kill()
    };
    match killed {
        Ok(()) => {
            child.wait().map_err(subprocess_io_failure)?;
        }
        Err(error) => {
            if child.try_wait().map_err(subprocess_io_failure)?.is_none() {
                return Err(subprocess_io_failure(error));
            }
        }
    }
    Ok(())
}

pub(crate) fn read_bounded(mut reader: impl Read, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut captured = Vec::with_capacity(limit.min(8 * 1024));
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            return Ok(captured);
        }
        let remaining = limit.saturating_sub(captured.len());
        captured.extend_from_slice(&chunk[..read.min(remaining)]);
    }
}

fn read_bounded_tail(mut reader: impl Read, limit: usize) -> std::io::Result<(Vec<u8>, bool)> {
    let mut captured = VecDeque::with_capacity(limit.min(8 * 1024));
    let mut truncated = false;
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            return Ok((captured.into_iter().collect(), truncated));
        }
        if limit == 0 {
            truncated = true;
            continue;
        }
        if read >= limit {
            let discarded = !captured.is_empty() || read > limit;
            captured.clear();
            captured.extend(&chunk[read - limit..read]);
            truncated |= discarded;
            continue;
        }
        let overflow = captured.len().saturating_add(read).saturating_sub(limit);
        if overflow != 0 {
            captured.drain(..overflow);
            truncated = true;
        }
        captured.extend(&chunk[..read]);
    }
}

fn subprocess_io_failure(error: std::io::Error) -> EffectFailure {
    EffectFailure::InterpreterFault(InterpreterFault::from(error.kind()))
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DirectExecutorSelector {
    Constructor(EffectCtor),
    AgentHarness(AgentHarnessName),
}

impl DirectExecutorSelector {
    fn for_effect(effect: &Effect) -> Result<Self, EffectCtor> {
        match effect {
            Effect::Http { .. } => Ok(Self::Constructor(EffectCtor::Http)),
            Effect::FileRead { .. } => Ok(Self::Constructor(EffectCtor::FileRead)),
            Effect::FileWrite { .. } => Ok(Self::Constructor(EffectCtor::FileWrite)),
            Effect::Spawn { .. } => Ok(Self::Constructor(EffectCtor::Spawn)),
            Effect::Notify { .. } => Ok(Self::Constructor(EffectCtor::Notify)),
            Effect::AgentInvoke { invoke, .. } => Ok(Self::AgentHarness(invoke.harness().clone())),
            Effect::RequestApproval { .. } => Err(EffectCtor::RequestApproval),
            Effect::MutateInstance { .. } => Err(EffectCtor::MutateInstance),
            Effect::Schedule { .. } => Ok(Self::Constructor(EffectCtor::Schedule)),
        }
    }

    fn for_peer(effect: &PeerEffect) -> Self {
        Self::Constructor(effect.constructor())
    }

    fn is_valid_registration(&self) -> bool {
        matches!(
            self,
            Self::Constructor(
                EffectCtor::FileRead
                    | EffectCtor::Http
                    | EffectCtor::FileWrite
                    | EffectCtor::Spawn
                    | EffectCtor::Notify
                    | EffectCtor::Schedule
                    | EffectCtor::PeerDiscover
                    | EffectCtor::PeerBind
                    | EffectCtor::PeerSend
                    | EffectCtor::PeerUnbind
                    | EffectCtor::PeerReceive,
            ) | Self::AgentHarness(_)
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DirectExecutorRegistrationError {
    EmptySelectors,
    EmptyProcessTargets,
    UnsupportedSelector(DirectExecutorSelector),
    DuplicateSelector(DirectExecutorSelector),
    SelectorAlreadyRegistered(DirectExecutorSelector),
    MissingNotificationEndpoint(NotificationChannel),
}

impl fmt::Display for DirectExecutorRegistrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySelectors => formatter.write_str("direct executor has no selectors"),
            Self::EmptyProcessTargets => {
                formatter.write_str("process executor allowlist names no programs")
            }
            Self::UnsupportedSelector(selector) => {
                write!(
                    formatter,
                    "unsupported direct executor selector: {selector:?}"
                )
            }
            Self::DuplicateSelector(selector) => {
                write!(formatter, "direct executor repeats selector: {selector:?}")
            }
            Self::SelectorAlreadyRegistered(selector) => {
                write!(
                    formatter,
                    "direct executor selector is already registered: {selector:?}"
                )
            }
            Self::MissingNotificationEndpoint(channel) => write!(
                formatter,
                "notification channel has no configured endpoint: {}",
                channel.as_str()
            ),
        }
    }
}

impl Error for DirectExecutorRegistrationError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DirectExecutorSubmitError {
    EngineOwnedEffect(EffectCtor),
    MissingExecutor(DirectExecutorSelector),
    MissingEmissionActor(EffectCtor),
    ExecutorRejected(SubmitError<circular_runtime::EffectId>),
}

impl fmt::Display for DirectExecutorSubmitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EngineOwnedEffect(effect) => {
                write!(formatter, "effect {effect:?} must be handled by the engine")
            }
            Self::MissingExecutor(selector) => {
                write!(
                    formatter,
                    "no direct executor is registered for {selector:?}"
                )
            }
            Self::MissingEmissionActor(effect) => {
                write!(
                    formatter,
                    "effect {effect:?} submission has no emitting actor"
                )
            }
            Self::ExecutorRejected(SubmitError::DuplicateEffectId(effect)) => {
                write!(
                    formatter,
                    "direct executor rejected duplicate effect id {effect}"
                )
            }
            Self::ExecutorRejected(SubmitError::ProgramNotExecutable) => {
                write!(
                    formatter,
                    "the executor's program does not exist or is not an executable file"
                )
            }
        }
    }
}

impl Error for DirectExecutorSubmitError {}

pub struct DirectEffectExecutorRegistry {
    outcome_waker: Option<std::task::Waker>,
    process_worker_leases: Option<ProcessWorkerLeases>,
    secret_vault: Option<Arc<crate::SecretVault>>,
    selectors: BTreeMap<DirectExecutorSelector, usize>,
    executors: Vec<Box<dyn Interpreter<circular_runtime::EffectId> + Send>>,
    next_poll: usize,
}

impl Default for DirectEffectExecutorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl DirectEffectExecutorRegistry {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            outcome_waker: None,
            process_worker_leases: None,
            secret_vault: None,
            selectors: BTreeMap::new(),
            executors: Vec::new(),
            next_poll: 0,
        }
    }

    pub(crate) fn set_outcome_waker(&mut self, waker: std::task::Waker) {
        for interpreter in &mut self.executors {
            interpreter.set_outcome_waker(waker.clone());
        }
        self.outcome_waker = Some(waker);
    }

    pub(crate) fn bind_secret_vault(&mut self, vault: Option<Arc<crate::SecretVault>>) {
        self.secret_vault = vault;
    }

    pub(crate) fn empty_on_same_vault(&self) -> Self {
        let mut registry = Self::new();
        registry.bind_secret_vault(self.secret_vault.clone());
        registry
    }

    fn guard_outcome(
        &self,
        (outcome, detail): (EffectOutcome<circular_runtime::EffectId>, Option<String>),
    ) -> (EffectOutcome<circular_runtime::EffectId>, Option<String>) {
        let Some(vault) = self.secret_vault.as_deref() else {
            return (outcome, detail);
        };
        let leaks = circular_runtime::encode_outcome(outcome.result(), outcome.failure_progress())
            .is_ok_and(|bytes| vault.contains_material(&bytes))
            || detail
                .as_ref()
                .is_some_and(|text| vault.contains_material(text.as_bytes()));
        if leaks {
            (
                EffectOutcome::new(
                    outcome.correlation().clone(),
                    Err(EffectFailure::InterpreterFault(
                        InterpreterFault::InvalidInput,
                    )),
                ),
                None,
            )
        } else {
            (outcome, detail)
        }
    }

    pub fn live_filesystem(
        read_grant: &FsReadGrant,
        write_grant: &FsWriteGrant,
    ) -> Result<Self, DirectExecutorRegistrationError> {
        let mut registry = Self::new();
        registry.register(
            [
                DirectExecutorSelector::Constructor(EffectCtor::FileRead),
                DirectExecutorSelector::Constructor(EffectCtor::FileWrite),
            ],
            LiveInterpreter::<circular_runtime::EffectId>::new(read_grant, write_grant),
        )?;
        Ok(registry)
    }

    pub fn register_process(
        &mut self,
        grant: &WorkspaceProcessGrant,
        workspace: NormalizedPath,
        max_output_bytes: usize,
        deadline: Duration,
    ) -> Result<(), DirectExecutorRegistrationError> {
        if grant.parameters().targets().is_empty() {
            return Err(DirectExecutorRegistrationError::EmptyProcessTargets);
        }
        let mut interpreter = AllowlistedProcessInterpreter::<circular_runtime::EffectId>::new(
            grant,
            workspace,
            max_output_bytes,
            deadline,
        );
        if let Some(leases) = &self.process_worker_leases {
            let leases = leases.clone();
            interpreter.lease_reader = Some(Arc::new(move |key| {
                leases.lock().expect("process worker leases").remove(key)
            }));
        }
        self.register(
            [DirectExecutorSelector::Constructor(EffectCtor::Spawn)],
            interpreter,
        )
    }

    pub fn register_http(
        &mut self,
        grant: &HttpFetchGrant,
        max_body_bytes: usize,
        timeout: Duration,
        vault: Option<Arc<crate::SecretVault>>,
    ) -> Result<(), DirectExecutorRegistrationError> {
        if vault.is_some() {
            self.bind_secret_vault(vault.clone());
        }
        self.register(
            [DirectExecutorSelector::Constructor(EffectCtor::Http)],
            HttpFetchInterpreter::<circular_runtime::EffectId>::new(
                grant,
                max_body_bytes,
                timeout,
                vault,
            ),
        )
    }

    pub fn register_notifications(
        &mut self,
        grant: &UserNotifyGrant,
        endpoints: BTreeMap<NotificationChannel, NotificationEndpoint>,
    ) -> Result<(), DirectExecutorRegistrationError> {
        if grant.parameters().is_empty() {
            return Ok(());
        }
        for channel in grant.parameters().iter() {
            if !endpoints.contains_key(channel) {
                return Err(
                    DirectExecutorRegistrationError::MissingNotificationEndpoint(channel.clone()),
                );
            }
        }
        self.register(
            [DirectExecutorSelector::Constructor(EffectCtor::Notify)],
            NotificationInterpreter::<circular_runtime::EffectId>::new(grant, endpoints),
        )
    }

    pub fn register<X>(
        &mut self,
        selectors: impl IntoIterator<Item = DirectExecutorSelector>,
        interpreter: X,
    ) -> Result<(), DirectExecutorRegistrationError>
    where
        X: Interpreter<circular_runtime::EffectId> + Send + 'static,
    {
        self.register_boxed(selectors, Box::new(interpreter))
    }

    pub fn register_boxed(
        &mut self,
        selectors: impl IntoIterator<Item = DirectExecutorSelector>,
        interpreter: Box<dyn Interpreter<circular_runtime::EffectId> + Send>,
    ) -> Result<(), DirectExecutorRegistrationError> {
        let selectors = selectors.into_iter().collect::<Vec<_>>();
        if selectors.is_empty() {
            return Err(DirectExecutorRegistrationError::EmptySelectors);
        }

        let mut unique = BTreeSet::new();
        for selector in &selectors {
            if !selector.is_valid_registration() {
                return Err(DirectExecutorRegistrationError::UnsupportedSelector(
                    selector.clone(),
                ));
            }
            if !unique.insert(selector.clone()) {
                return Err(DirectExecutorRegistrationError::DuplicateSelector(
                    selector.clone(),
                ));
            }
            if self.selectors.contains_key(selector) {
                return Err(DirectExecutorRegistrationError::SelectorAlreadyRegistered(
                    selector.clone(),
                ));
            }
        }

        let mut interpreter = interpreter;
        if let Some(waker) = &self.outcome_waker {
            interpreter.set_outcome_waker(waker.clone());
        }
        let index = self.executors.len();
        self.executors.push(interpreter);
        for selector in selectors {
            self.selectors.insert(selector, index);
        }
        Ok(())
    }

    pub(crate) fn begin_cancel(&mut self) {
        for interpreter in &mut self.executors {
            interpreter.begin_cancel();
        }
    }

    pub(crate) fn pause(&mut self, force: bool) {
        for interpreter in &mut self.executors {
            interpreter.pause(force);
        }
    }

    pub(crate) fn resume(&mut self) {
        for interpreter in &mut self.executors {
            interpreter.resume();
        }
    }

    pub(crate) fn with_process_leases(&mut self) {
        self.process_worker_leases = Some(Arc::new(Mutex::new(BTreeMap::new())));
    }

    pub(crate) fn submit_leased(
        &mut self,
        effect_id: circular_runtime::EffectId,
        effect: &Effect,
        lease: ProcessLease,
    ) -> Result<(), DirectExecutorSubmitError> {
        if let Some(leases) = &self.process_worker_leases {
            leases
                .lock()
                .expect("process worker leases")
                .insert(effect_id.clone(), lease);
        }
        let submitted = self.submit(effect_id.clone(), effect);
        if submitted.is_err()
            && let Some(leases) = &self.process_worker_leases
        {
            leases
                .lock()
                .expect("process worker leases")
                .remove(&effect_id);
        }
        submitted
    }

    pub fn submit(
        &mut self,
        effect_id: circular_runtime::EffectId,
        effect: &Effect,
    ) -> Result<(), DirectExecutorSubmitError> {
        let selector = DirectExecutorSelector::for_effect(effect)
            .map_err(DirectExecutorSubmitError::EngineOwnedEffect)?;
        let index = self
            .selectors
            .get(&selector)
            .copied()
            .ok_or(DirectExecutorSubmitError::MissingExecutor(selector))?;
        self.executors[index]
            .submit(effect_id, effect)
            .map_err(DirectExecutorSubmitError::ExecutorRejected)
    }

    pub fn submit_peer(
        &mut self,
        effect_id: circular_runtime::EffectId,
        effect: &PeerEffect,
    ) -> Result<(), DirectExecutorSubmitError> {
        let selector = DirectExecutorSelector::for_peer(effect);
        let index = self
            .selectors
            .get(&selector)
            .copied()
            .ok_or(DirectExecutorSubmitError::MissingExecutor(selector))?;
        self.executors[index]
            .submit_peer(effect_id, effect)
            .map_err(DirectExecutorSubmitError::ExecutorRejected)
    }

    pub fn next_outcome(&mut self) -> Option<EffectOutcome<circular_runtime::EffectId>> {
        self.next_outcome_with_detail().map(|(outcome, _)| outcome)
    }

    fn next_outcome_with_detail(
        &mut self,
    ) -> Option<(EffectOutcome<circular_runtime::EffectId>, Option<String>)> {
        self.poll_executors()
            .map(|settled| self.guard_outcome(settled))
    }

    fn poll_executors(
        &mut self,
    ) -> Option<(EffectOutcome<circular_runtime::EffectId>, Option<String>)> {
        let count = self.executors.len();
        if count == 0 {
            return None;
        }
        let start = self.next_poll % count;
        let settled = (start..count).chain(0..start).find_map(|index| {
            self.executors[index]
                .next_outcome_with_detail()
                .map(|outcome| (index, outcome))
        });
        settled.map(|(index, outcome)| {
            self.next_poll = index + 1;
            outcome
        })
    }
}

#[cfg(test)]
mod vault_security_tests {
    use super::*;

    #[test]
    fn security_http_transport_has_independent_literal_boundaries() {
        for (raw, expected) in [
            ("https://api.example.test/v1", true),
            ("http://api.example.test/v1", false),
            ("http://127.0.0.1:9000/", true),
            ("http://127.5.6.7/", true),
            ("http://[::1]:9000/", true),
            ("http://localhost:9000/", true),
            ("http://localhost.attacker.test/", false),
            ("http://192.168.1.2/", false),
            ("http://[2001:db8::1]/", false),
            ("http://2130706433/", false),
        ] {
            assert_eq!(
                permits_http_transport(&circular_runtime::HttpUrl::try_new(raw).unwrap()),
                expected,
                "{raw}"
            );
        }
        assert!(
            log::STATIC_MAX_LEVEL <= log::LevelFilter::Info,
            "HTTP library debug diagnostics contain credentials"
        );
    }
}
