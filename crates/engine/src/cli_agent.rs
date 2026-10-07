
use crate::harness_boundary::{
    BoundaryDeclaration, EgressWindow, TurnBoundary, WINDOW_ENVIRONMENT, bounded_command, no_home,
    node_temp,
};
use crate::harness_event::{HarnessBoundaryKind, boundary_denied};
use circular_plan::{ActorId, LocalKey};
use circular_runtime::{
    AgentHarnessName, AgentPayload, AgentProgressRecord, AgentSessionId, AgentStepNext,
    AgentStepRequest, AgentStepResult, Effect, EffectFailure, EffectOutcome, Interpreter,
    InterpreterFault, OutcomePayload, SubmitError,
};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

const STDERR_DETAIL_LIMIT_BYTES: usize = 8 * 1024;

const NODE_FOLDER_NAME_LIMIT_BYTES: usize = 255;

pub(crate) const HARNESS_CHILD_ENVIRONMENT: [&str; 4] = ["HOME", "PATH", "USER", "LOGNAME"];

pub(crate) fn executor_owns_environment(name: &str) -> bool {
    HARNESS_CHILD_ENVIRONMENT.contains(&name)
        || WINDOW_ENVIRONMENT.contains(&name)
        || name == "TMPDIR"
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EnvValue {
    NodeTemp,
    Literal(String),
}
const STDERR_TRUNCATED_PREFIX: &str = "[stderr truncated; showing tail]\n";
const PROVIDER_OUTPUT_TRUNCATED_PREFIX: &str = "[provider output truncated; showing tail]\n";

pub struct CliInvocation {
    pub args: Vec<String>,
    pub stdin: Option<Vec<u8>>,
}

pub struct CliTurn {
    pub output: String,
    pub session: Option<String>,
}

pub trait CliHarnessAdapter: Send + Sync {
    fn name(&self) -> &str;

    fn invocation(&self, session: Option<&str>, prompt: &str) -> CliInvocation;

    fn parse(&self, stdout: &[u8]) -> Result<CliTurn, EffectFailure>;

    fn failure_detail(&self, stdout: &[u8]) -> Option<String> {
        provider_output_detail(stdout)
    }

    fn env_remove(&self) -> &[String] {
        &[]
    }

    fn env_set(&self) -> &[(String, EnvValue)] {
        &[]
    }

    fn boundary(&self) -> &BoundaryDeclaration {
        static EMPTY: BoundaryDeclaration = BoundaryDeclaration::EMPTY;
        &EMPTY
    }
}

fn invalid() -> EffectFailure {
    EffectFailure::InterpreterFault(InterpreterFault::InvalidInput)
}

#[must_use]
pub fn adapter_for(name: &str) -> Option<&'static dyn CliHarnessAdapter> {
    crate::harness_adapter::declared_adapters()
        .iter()
        .find(|adapter| adapter.name() == name)
        .map(|adapter| adapter as &'static dyn CliHarnessAdapter)
}

pub fn adapter_names() -> impl Iterator<Item = &'static str> {
    crate::harness_adapter::declared_adapters()
        .iter()
        .map(|adapter| adapter.name())
}

pub fn adapter_candidates(
    home: &std::path::Path,
) -> impl Iterator<Item = (&'static str, Vec<PathBuf>)> + '_ {
    crate::harness_adapter::declared_adapters()
        .iter()
        .map(move |adapter| (adapter.name(), adapter.candidates(home)))
}

#[derive(Clone)]
pub struct CliHarnessSpec {
    adapter: &'static dyn CliHarnessAdapter,
    program: PathBuf,
    state: PathBuf,
    workspace: PathBuf,
    max_output_bytes: usize,
    deadline: Duration,
}

impl CliHarnessSpec {
    #[must_use]
    pub fn for_harness(
        name: &str,
        program: PathBuf,
        state: PathBuf,
        workspace: PathBuf,
        deadline: Duration,
    ) -> Option<Self> {
        Some(Self {
            adapter: adapter_for(name)?,
            program,
            state,
            workspace,
            max_output_bytes: 1024 * 1024,
            deadline,
        })
    }
}

#[must_use]
pub fn program_is_executable(program: &std::path::Path) -> bool {
    std::fs::metadata(program).is_ok_and(|metadata| {
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt as _;
            metadata.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = true;
        metadata.is_file() && executable
    })
}

fn node_folder_name(owner: &ActorId) -> Option<String> {
    fn encode(text: &str, out: &mut String) {
        for byte in text.bytes() {
            if byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-') {
                out.push(char::from(byte));
            } else {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    let ActorId::Scoped { scope, local } = owner else {
        return None;
    };
    let mut name = String::new();
    if scope.depth() > 0 {
        encode(&scope.to_string(), &mut name);
        name.push('.');
    }
    match local {
        LocalKey::Named(local) => encode(local.as_str(), &mut name),
        LocalKey::Ephemeral(uuid) => {
            name.push('~');
            for byte in uuid.as_bytes() {
                name.push_str(&format!("{byte:02x}"));
            }
        }
    }
    Some(name)
}

fn node_folder(workspace: &std::path::Path, owner: &ActorId) -> Result<PathBuf, EffectFailure> {
    let name = node_folder_name(owner).ok_or(EffectFailure::ParameterDenied {
        capability: circular_runtime::Capability::AgentHarness,
    })?;
    if name.len() > NODE_FOLDER_NAME_LIMIT_BYTES {
        return Err(invalid());
    }
    let folder = workspace.join("agents").join(name);
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder
        .create(&folder)
        .map_err(|error| EffectFailure::InterpreterFault(InterpreterFault::from(error.kind())))?;
    Ok(folder)
}

fn stderr_detail(stderr: &[u8], capture_truncated: bool) -> String {
    let decoded = String::from_utf8_lossy(stderr);
    if !capture_truncated && decoded.len() <= STDERR_DETAIL_LIMIT_BYTES {
        return decoded.into_owned();
    }

    let tail_bytes = STDERR_DETAIL_LIMIT_BYTES - STDERR_TRUNCATED_PREFIX.len();
    let mut start = decoded.len().saturating_sub(tail_bytes);
    while !decoded.is_char_boundary(start) {
        start += 1;
    }
    let mut detail = String::with_capacity(STDERR_DETAIL_LIMIT_BYTES);
    detail.push_str(STDERR_TRUNCATED_PREFIX);
    detail.push_str(&decoded[start..]);
    detail
}

pub(crate) fn bounded_provider_detail(detail: &str) -> String {
    if detail.len() <= STDERR_DETAIL_LIMIT_BYTES {
        return detail.to_owned();
    }

    let tail_bytes = STDERR_DETAIL_LIMIT_BYTES - PROVIDER_OUTPUT_TRUNCATED_PREFIX.len();
    let mut start = detail.len().saturating_sub(tail_bytes);
    while !detail.is_char_boundary(start) {
        start += 1;
    }
    let mut bounded = String::with_capacity(STDERR_DETAIL_LIMIT_BYTES);
    bounded.push_str(PROVIDER_OUTPUT_TRUNCATED_PREFIX);
    bounded.push_str(&detail[start..]);
    bounded
}

pub(crate) fn provider_output_detail(stdout: &[u8]) -> Option<String> {
    let decoded = String::from_utf8_lossy(stdout);
    (!decoded.is_empty()).then(|| bounded_provider_detail(&decoded))
}

fn exit_failure_detail(
    adapter: &dyn CliHarnessAdapter,
    stdout: &[u8],
    stderr: &[u8],
    stderr_truncated: bool,
) -> Option<String> {
    let provider = adapter.failure_detail(stdout);
    let stderr = (!stderr.is_empty()).then(|| stderr_detail(stderr, stderr_truncated));
    match (provider, stderr) {
        (Some(provider), Some(stderr)) => Some(format!("{provider}\n{stderr}")),
        (provider, stderr) => provider.or(stderr),
    }
}

type TurnSettlement = (
    Result<OutcomePayload, EffectFailure>,
    Vec<AgentProgressRecord>,
    Option<String>,
    Option<bool>,
);

fn run_turn(
    spec: &CliHarnessSpec,
    owner: &ActorId,
    harness: &AgentHarnessName,
    session: Option<&AgentSessionId>,
    request: &AgentStepRequest,
    cancellation: &crate::direct_effect::SubprocessCancellation,
) -> TurnSettlement {
    if cancellation.is_cancelled() {
        return (
            Err(EffectFailure::InterpreterFault(
                InterpreterFault::Interrupted,
            )),
            Vec::new(),
            None,
            None,
        );
    }
    let mut attempted = false;
    let mut window = None;
    let driven = (|| {
        let bare = |failure| (failure, None);
        let AgentStepRequest::UserTurn(payload) = request else {
            return Err(bare(invalid()));
        };
        let prompt = std::str::from_utf8(payload.as_bytes()).map_err(|_| bare(invalid()))?;
        let session = session
            .map(|session| {
                std::str::from_utf8(session.opaque())
                    .map(str::to_owned)
                    .map_err(|_| bare(invalid()))
            })
            .transpose()?;
        let CliInvocation { args, stdin } = spec.adapter.invocation(session.as_deref(), prompt);
        let folder = node_folder(&spec.workspace, owner).map_err(bare)?;
        let temp = node_temp(&folder).map_err(bare)?;
        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            let (failure, detail) = no_home();
            (failure, Some(detail))
        })?;
        let declared = spec.adapter.boundary();
        let boundary = TurnBoundary::draw(
            declared,
            crate::harness_adapter::host_secrets(),
            &folder,
            &spec.state,
            &home,
        );
        let opened = EgressWindow::open(&declared.egress).map_err(|error| {
            bare(EffectFailure::InterpreterFault(InterpreterFault::from(
                error.kind(),
            )))
        })?;
        let proxy = format!("http://127.0.0.1:{}", opened.port());
        let profile = boundary.profile(opened.port());
        window = Some(opened);

        let mut command = bounded_command(&spec.program, &args, &profile)
            .map_err(|(failure, detail)| (failure, Some(detail)))?;
        command.current_dir(&folder).env("TMPDIR", &temp);
        for name in WINDOW_ENVIRONMENT {
            command.env(name, &proxy);
        }
        for (name, value) in spec.adapter.env_set() {
            match value {
                EnvValue::NodeTemp => command.env(name, &temp),
                EnvValue::Literal(value) => command.env(name, value),
            };
        }
        crate::direct_effect::inherit_child_environment(&mut command, &HARNESS_CHILD_ENVIRONMENT);
        for name in spec.adapter.env_remove() {
            command.env_remove(name);
        }
        crate::direct_effect::drive_cancellable_subprocess(
            &mut command,
            stdin.as_deref(),
            spec.max_output_bytes,
            Some(spec.deadline),
            true,
            cancellation,
            &mut attempted,
        )
        .map_err(bare)
    })();
    let progress = window
        .as_mut()
        .map(EgressWindow::close)
        .unwrap_or_default()
        .iter()
        .map(|denial| boundary_denied(HarnessBoundaryKind::Egress, &denial.at, denial.count))
        .collect::<Vec<_>>();
    let driven = match driven {
        Ok(driven) => driven,
        Err((failure, detail)) => {
            return (Err(failure), progress, detail, attempted.then_some(false));
        }
    };
    if !driven.status.success() {
        return (
            Err(EffectFailure::TransportTerminal),
            progress,
            exit_failure_detail(
                spec.adapter,
                &driven.stdout,
                &driven.stderr,
                driven.stderr_truncated,
            ),
            Some(false),
        );
    }

    let result = (|| {
        let turn = spec.adapter.parse(&driven.stdout)?;
        if turn.output.trim().is_empty() {
            return Err(invalid());
        }
        let provider_session = turn.session.is_some();
        let session = AgentSessionId::new(
            harness.clone(),
            turn.session.map_or_else(
                || b"cli-session".to_vec(),
                String::into_bytes,
            ),
        );
        let result = AgentStepResult::try_new(
            harness,
            session,
            progress.clone(),
            AgentStepNext::Final {
                output: AgentPayload::new(turn.output.into_bytes()),
                metadata: AgentPayload::default(),
            },
        )
        .map(OutcomePayload::AgentStepResult)
        .map_err(|_| invalid());
        Ok((result?, provider_session))
    })();
    match result {
        Ok((result, provider_session)) => (Ok(result), Vec::new(), None, Some(provider_session)),
        Err(failure) => (
            Err(failure),
            progress,
            spec.adapter.failure_detail(&driven.stdout),
            Some(false),
        ),
    }
}

pub struct CliAgentExecutor {
    outcome_wake: crate::direct_effect::OutcomeWake,
    spec: Arc<CliHarnessSpec>,
    submitted: BTreeSet<circular_runtime::EffectId>,
    pending: BTreeSet<circular_runtime::EffectId>,
    results: Receiver<(EffectOutcome<circular_runtime::EffectId>, Option<String>)>,
    sender: Sender<(EffectOutcome<circular_runtime::EffectId>, Option<String>)>,
    contact_witness: Option<Arc<dyn Fn(bool) + Send + Sync>>,
    cancellation: Arc<crate::direct_effect::SubprocessCancellation>,
    workers: Vec<crate::direct_effect::EffectWorker<circular_runtime::EffectId>>,
}

impl CliAgentExecutor {
    #[must_use]
    pub fn new(spec: CliHarnessSpec) -> Self {
        let (sender, results) = channel();
        Self {
            outcome_wake: crate::direct_effect::OutcomeWake::default(),
            spec: Arc::new(spec),
            submitted: BTreeSet::new(),
            pending: BTreeSet::new(),
            results,
            sender,
            contact_witness: None,
            cancellation: Arc::new(crate::direct_effect::SubprocessCancellation::default()),
            workers: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_contact_witness(mut self, witness: impl Fn(bool) + Send + Sync + 'static) -> Self {
        self.contact_witness = Some(Arc::new(witness));
        self
    }

    fn reap_finished_workers(&mut self) {
        let lost = crate::direct_effect::reap_effect_workers(&mut self.workers);
        self.settle_lost_workers(lost);
    }

    fn cancel_and_join(&mut self) {
        self.cancellation.cancel();
        let mut lost = Vec::new();
        for worker in self.workers.drain(..) {
            if let Some(fact) = worker.reap() {
                lost.push(fact);
            }
        }
        self.settle_lost_workers(lost);
    }

    fn settle_lost_workers(&mut self, lost: Vec<(circular_runtime::EffectId, String)>) {
        for (correlation, detail) in lost {
            let _ = self.sender.send((
                EffectOutcome::new(
                    correlation,
                    Err(EffectFailure::InterpreterFault(InterpreterFault::Other)),
                ),
                Some(detail),
            ));
            self.outcome_wake.notify();
        }
    }
}

impl Drop for CliAgentExecutor {
    fn drop(&mut self) {
        self.cancel_and_join();
    }
}

impl Interpreter<circular_runtime::EffectId> for CliAgentExecutor {
    fn set_outcome_waker(&mut self, waker: std::task::Waker) {
        self.outcome_wake.bind(waker);
    }

    fn submit(
        &mut self,
        correlation: circular_runtime::EffectId,
        effect: &Effect,
    ) -> Result<(), SubmitError<circular_runtime::EffectId>> {
        self.reap_finished_workers();
        if !program_is_executable(&self.spec.program) {
            return Err(SubmitError::ProgramNotExecutable);
        }
        if !self.submitted.insert(correlation.clone()) {
            return Err(SubmitError::DuplicateEffectId(correlation));
        }
        self.pending.insert(correlation.clone());
        let Effect::AgentInvoke { invoke, .. } = effect else {
            let _ = self.sender.send((
                EffectOutcome::new(
                    correlation,
                    Err(EffectFailure::InterpreterFault(InterpreterFault::Other)),
                ),
                None,
            ));
            return Ok(());
        };
        if self.cancellation.is_cancelled() {
            let _ = self.sender.send((
                EffectOutcome::new(
                    correlation,
                    Err(EffectFailure::InterpreterFault(
                        InterpreterFault::Interrupted,
                    )),
                ),
                None,
            ));
            return Ok(());
        }
        let spec = Arc::clone(&self.spec);
        let owner = correlation.actor().clone();
        let harness = invoke.harness().clone();
        let session = invoke.session().cloned();
        let request = invoke.request().clone();
        let contact_witness = self.contact_witness.clone();
        let cancellation = Arc::clone(&self.cancellation);
        let failed = correlation.clone();
        match crate::direct_effect::spawn_effect_worker(
            None,
            correlation,
            self.sender.clone(),
            self.outcome_wake.clone(),
            move || {
                let (result, failure_progress, detail, provider_session) = run_turn(
                    &spec,
                    &owner,
                    &harness,
                    session.as_ref(),
                    &request,
                    &cancellation,
                );
                if let (Some(witness), Some(provider_session)) = (contact_witness, provider_session)
                {
                    witness(provider_session);
                }
                (result, failure_progress, detail)
            },
        ) {
            Ok(worker) => self.workers.push(worker),
            Err(_) => {
                let _ = self.sender.send((
                    EffectOutcome::new(
                        failed,
                        Err(EffectFailure::InterpreterFault(
                            InterpreterFault::ResourceExhausted,
                        )),
                    ),
                    None,
                ));
                self.outcome_wake.notify();
            }
        }
        Ok(())
    }

    fn submit_peer(
        &mut self,
        correlation: circular_runtime::EffectId,
        _effect: &circular_runtime::PeerEffect,
    ) -> Result<(), SubmitError<circular_runtime::EffectId>> {
        if !self.submitted.insert(correlation.clone()) {
            return Err(SubmitError::DuplicateEffectId(correlation));
        }
        self.pending.insert(correlation.clone());
        let _ = self.sender.send((
            EffectOutcome::new(correlation, Err(EffectFailure::EndpointGone)),
            None,
        ));
        Ok(())
    }

    fn next_outcome(&mut self) -> Option<EffectOutcome<circular_runtime::EffectId>> {
        self.reap_finished_workers();
        self.next_outcome_with_detail().map(|(outcome, _)| outcome)
    }

    fn next_outcome_with_detail(
        &mut self,
    ) -> Option<(EffectOutcome<circular_runtime::EffectId>, Option<String>)> {
        self.reap_finished_workers();
        let result = self.results.try_recv().ok()?;
        self.pending.remove(result.0.correlation());
        Some(result)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::process::Command;
    use std::time::Instant;

    struct SleepingAdapter;

    impl CliHarnessAdapter for SleepingAdapter {
        fn name(&self) -> &str {
            "sleep-fixture"
        }

        fn invocation(&self, _session: Option<&str>, _prompt: &str) -> CliInvocation {
            CliInvocation {
                args: vec!["60".to_owned()],
                stdin: None,
            }
        }

        fn parse(&self, _stdout: &[u8]) -> Result<CliTurn, EffectFailure> {
            panic!("the sleeping fixture must be interrupted before parsing")
        }
    }

    static SLEEPING_ADAPTER: SleepingAdapter = SleepingAdapter;

    struct PanickingParseAdapter;

    impl CliHarnessAdapter for PanickingParseAdapter {
        fn name(&self) -> &str {
            "panic-fixture"
        }

        fn invocation(&self, _session: Option<&str>, _prompt: &str) -> CliInvocation {
            CliInvocation {
                args: Vec::new(),
                stdin: None,
            }
        }

        fn parse(&self, _stdout: &[u8]) -> Result<CliTurn, EffectFailure> {
            panic!("cli parse panicked")
        }
    }

    static PANICKING_PARSE_ADAPTER: PanickingParseAdapter = PanickingParseAdapter;

    struct ChannelWake(std::sync::mpsc::Sender<()>);

    impl std::task::Wake for ChannelWake {
        fn wake(self: Arc<Self>) {
            let _ = self.0.send(());
        }

        fn wake_by_ref(self: &Arc<Self>) {
            let _ = self.0.send(());
        }
    }

    fn wake_owner(executor: &mut CliAgentExecutor) -> std::sync::mpsc::Receiver<()> {
        let (sender, woken) = std::sync::mpsc::channel();
        executor.set_outcome_waker(std::task::Waker::from(Arc::new(ChannelWake(sender))));
        woken
    }

    fn fixture_directory(label: &str) -> PathBuf {
        let path = crate::unique_temp_dir::unique_temp_path(&format!("circular-cli-agent-{label}"));
        std::fs::create_dir_all(&path).expect("fixture directory");
        path
    }

    fn fixture_state(directory: &std::path::Path) -> PathBuf {
        directory.join("state")
    }

    fn fixture_workspace(directory: &std::path::Path) -> PathBuf {
        fixture_state(directory).join("workspace")
    }

    fn test_effect_node(directory: &std::path::Path) -> PathBuf {
        fixture_workspace(directory)
            .join("agents")
            .join("test-effect")
    }

    fn script(directory: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).expect("write fixture CLI");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
            .expect("make fixture CLI executable");
        path
    }

    fn wait_for_pid(path: &std::path::Path) -> u32 {
        loop {
            if let Ok(pid) = std::fs::read_to_string(path)
                && let Ok(pid) = pid.trim().parse()
            {
                return pid;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn process_exists(pid: u32) -> bool {
        use nix::errno::Errno;
        use nix::sys::signal::kill;
        use nix::unistd::Pid;

        let pid = i32::try_from(pid).expect("fixture pid fits i32");
        match kill(Pid::from_raw(pid), None) {
            Ok(()) | Err(Errno::EPERM) => true,
            Err(Errno::ESRCH) => false,
            Err(error) => panic!("could not inspect fixture pid {pid}: {error}"),
        }
    }

    fn assert_process_gone(pid: u32) {
        while process_exists(pid) {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn harness(name: &str) -> AgentHarnessName {
        AgentHarnessName::try_from_normalized(name).expect("nonempty harness name")
    }

    fn spec(name: &str, program: PathBuf, directory: PathBuf) -> CliHarnessSpec {
        CliHarnessSpec::for_harness(
            name,
            program,
            fixture_state(&directory),
            fixture_workspace(&directory),
            Duration::from_secs(30),
        )
        .expect("boarded harness name")
    }

    fn sleeping_spec(program: PathBuf, directory: PathBuf) -> CliHarnessSpec {
        CliHarnessSpec {
            adapter: &SLEEPING_ADAPTER,
            program,
            state: fixture_state(&directory),
            workspace: fixture_workspace(&directory),
            max_output_bytes: 64,
            deadline: Duration::from_secs(30),
        }
    }

    fn invoke_effect(
        harness: &AgentHarnessName,
        session: Option<AgentSessionId>,
        turn: &[u8],
    ) -> Effect {
        let grant = circular_runtime::AgentHarnessGrant::agent_harness([harness.clone()]);
        let invoke = circular_runtime::AgentInvokeSpec::try_new(
            harness.clone(),
            session,
            AgentStepRequest::user_turn(AgentPayload::new(turn.to_vec())),
        )
        .expect("user turns carry no call identity");
        Effect::agent_invoke(
            circular_runtime::GrantIssuer::new().issue(&grant),
            None,
            invoke,
        )
    }

    fn settled(
        executor: &mut CliAgentExecutor,
        correlation: circular_runtime::EffectId,
        effect: &Effect,
    ) -> EffectOutcome<circular_runtime::EffectId> {
        executor
            .submit(correlation, effect)
            .expect("submission is accepted");
        loop {
            if let Some(outcome) = executor.next_outcome() {
                return outcome;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn settled_with_detail(
        executor: &mut CliAgentExecutor,
        correlation: circular_runtime::EffectId,
        effect: &Effect,
    ) -> (EffectOutcome<circular_runtime::EffectId>, Option<String>) {
        executor
            .submit(correlation, effect)
            .expect("submission is accepted");
        loop {
            if let Some(completion) = executor.next_outcome_with_detail() {
                return completion;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn step_result(outcome: &EffectOutcome<circular_runtime::EffectId>) -> &AgentStepResult {
        match outcome.result() {
            Ok(OutcomePayload::AgentStepResult(result)) => result,
            other => panic!("a CLI turn settles as a step result, got {other:?}"),
        }
    }

    #[test]
    fn the_adapter_table_is_the_only_vocabulary() {
        assert_eq!(
            adapter_names().collect::<Vec<_>>(),
            vec!["claude", "codex", "pi"]
        );
        assert!(adapter_for("opencode").is_none());
        assert_eq!(
            adapter_for("codex").map(CliHarnessAdapter::name),
            Some("codex")
        );
    }

    #[test]
    fn the_submission_returns_before_the_turn_finishes() {
        let directory = fixture_directory("async");
        let program = script(
            &directory,
            "claude",
            "sleep 0.4\nprintf '{\"type\":\"result\",\"result\":\"slow\",\"session_id\":\"s1\",\"is_error\":false}'\n",
        );
        let mut executor = CliAgentExecutor::new(spec("claude", program, directory.clone()));
        let name = harness("claude");
        let started = Instant::now();
        executor
            .submit(crate::test_effect(1), &invoke_effect(&name, None, b"hello"))
            .expect("submission is accepted");
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "submit must not block on the subprocess"
        );
        assert!(
            executor.next_outcome().is_none(),
            "the outcome is not ready yet"
        );
        let outcome = loop {
            if let Some(outcome) = executor.next_outcome() {
                break outcome;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let result = step_result(&outcome);
        assert!(matches!(
            result.next(),
            AgentStepNext::Final { output, .. } if output.as_bytes() == b"slow"
        ));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn wall_clock_deadline_interrupts_a_sleeping_cli_and_settles() {
        let directory = fixture_directory("deadline");
        let spec = CliHarnessSpec {
            adapter: &SLEEPING_ADAPTER,
            program: PathBuf::from("/bin/sleep"),
            state: fixture_state(&directory),
            workspace: fixture_workspace(&directory),
            max_output_bytes: 64,
            deadline: Duration::from_millis(100),
        };
        let mut executor = CliAgentExecutor::new(spec);
        let name = harness("sleep-fixture");
        let started = Instant::now();
        let outcome = settled(
            &mut executor,
            crate::test_effect(1),
            &invoke_effect(&name, None, b"hang"),
        );

        assert!(
            started.elapsed() < Duration::from_secs(5),
            "a sleeping CLI must settle at its wall-clock deadline"
        );
        assert_eq!(
            outcome.result(),
            &Err(EffectFailure::InterpreterFault(
                InterpreterFault::Interrupted
            ))
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn cancellation_before_submission_prevents_provider_spawn() {
        let directory = fixture_directory("cancel-before-spawn");
        let marker = test_effect_node(&directory).join("spawned");
        let program = script(
            &directory,
            "provider",
            "printf spawned > spawned\nsleep 60\n",
        );
        let mut executor = CliAgentExecutor::new(sleeping_spec(program, directory.clone()));
        let name = harness("sleep-fixture");

        executor.begin_cancel();
        let outcome = settled(
            &mut executor,
            crate::test_effect(1),
            &invoke_effect(&name, None, b"never starts"),
        );

        assert_eq!(
            outcome.result(),
            &Err(EffectFailure::InterpreterFault(
                InterpreterFault::Interrupted
            ))
        );
        assert!(!marker.exists(), "cancellation must win before spawn");
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn claude_answers_carry_the_session_and_resume_uses_it() {
        let directory = fixture_directory("claude");
        let program = script(
            &directory,
            "claude",
            "printf '%s ' \"$@\" >> argv.log\nprintf '\\n' >> argv.log\ncat > prompt.log\nprintf '{\"type\":\"result\",\"result\":\"done\",\"session_id\":\"abc-123\",\"is_error\":false}'\n",
        );
        let mut executor = CliAgentExecutor::new(spec("claude", program, directory.clone()));
        let name = harness("claude");

        let outcome = settled(
            &mut executor,
            crate::test_effect(1),
            &invoke_effect(&name, None, b"first turn"),
        );
        let result = step_result(&outcome);
        assert_eq!(result.session().opaque(), b"abc-123");
        assert_eq!(
            std::fs::read(test_effect_node(&directory).join("prompt.log"))
                .expect("prompt reached stdin"),
            b"first turn"
        );

        let outcome = settled(
            &mut executor,
            crate::test_effect(2),
            &invoke_effect(&name, Some(result.session().clone()), b"second turn"),
        );
        let _ = step_result(&outcome);
        let argv = std::fs::read_to_string(test_effect_node(&directory).join("argv.log"))
            .expect("argv recorded");
        let second = argv.lines().nth(1).expect("two invocations recorded");
        assert!(
            second.contains("--resume abc-123"),
            "the second turn must resume the answered session: {second}"
        );
        for line in argv.lines() {
            assert!(
                line.contains(
                    "--strict-mcp-config --permission-mode bypassPermissions --settings {\"disableAllHooks\":true,\"sandbox\":{\"enabled\":false}}"
                ),
                "every claude turn blocks the user's MCP servers and hooks: {line}"
            );
        }
        assert!(
            !argv
                .lines()
                .next()
                .expect("first line")
                .contains("--resume")
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn a_node_folder_name_is_one_injective_component_per_actor() {
        use circular_plan::{Name, ScopeId, ScopeSeg, SystemActor, Uuid};
        let named = |scope: ScopeId, name: &str| ActorId::Scoped {
            scope,
            local: LocalKey::Named(Name::from_normalized(name)),
        };
        let team = ScopeId::from_segments(vec![ScopeSeg::Child(Name::from_normalized("team"))])
            .expect("one child scope");
        let cases = [
            (named(ScopeId::root(), "triage"), "triage"),
            (named(ScopeId::root(), "Triage"), "%54riage"),
            (named(ScopeId::root(), "a.b"), "a%2Eb"),
            (named(ScopeId::root(), "%2E"), "%252%45"),
            (named(team.clone(), "x"), "%2Fc4%3Ateam.x"),
            (
                ActorId::Scoped {
                    scope: ScopeId::root(),
                    local: LocalKey::Ephemeral(Uuid::from_bytes([0xab; 16])),
                },
                "~abababababababababababababababab",
            ),
        ];
        for (actor, expected) in &cases {
            assert_eq!(node_folder_name(actor).as_deref(), Some(*expected));
        }
        assert_eq!(
            node_folder_name(&ActorId::System(SystemActor::Stream)),
            None
        );
    }

    #[test]
    fn each_agent_node_turns_in_its_own_folder_so_continuation_stays_its_own() {
        use circular_plan::{Name, NamedActorId, ScopeId, ScopeSeg};
        let directory = fixture_directory("node-folders");
        let program = script(
            &directory,
            "pi",
            "pwd -P >> cwd.log\nprintf '%s ' \"$@\" >> argv.log\nprintf '\\n' >> argv.log\nprintf 'answer'\n",
        );
        let mut executor = CliAgentExecutor::new(spec("pi", program, directory.clone()));
        let name = harness("pi");
        let alpha = NamedActorId::new(ScopeId::root(), Name::from_normalized("alpha"));
        let team = ScopeId::from_segments(vec![ScopeSeg::Child(Name::from_normalized("team"))])
            .expect("one child scope");
        let beta = NamedActorId::new(team, Name::from_normalized("Beta"));
        let effect = |actor: &NamedActorId, index: u64| {
            let depth = actor.scope().depth();
            circular_runtime::EffectId::from_components(
                actor.clone().into(),
                vec![circular_plan::Generation::new(0); depth + 1],
                circular_runtime::EffectOccasion::Poll(circular_core::Tick::new(5), 0),
                index,
            )
            .expect("one generation per scope level")
        };

        let first = settled(
            &mut executor,
            effect(&alpha, 1),
            &invoke_effect(&name, None, b"alpha first"),
        );
        let alpha_session = step_result(&first).session().clone();
        let _ = step_result(&settled(
            &mut executor,
            effect(&beta, 1),
            &invoke_effect(&name, None, b"beta first"),
        ));
        let _ = step_result(&settled(
            &mut executor,
            effect(&alpha, 2),
            &invoke_effect(&name, Some(alpha_session), b"alpha again"),
        ));

        let agents = fixture_workspace(&directory).join("agents");
        let alpha_folder = agents.join("alpha");
        let beta_folder = agents.join("%2Fc4%3Ateam.%42eta");
        let lines = |folder: &std::path::Path, file: &str| {
            std::fs::read_to_string(folder.join(file))
                .unwrap_or_else(|error| panic!("{file} in {}: {error}", folder.display()))
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let real = |folder: &std::path::Path| {
            std::fs::canonicalize(folder)
                .expect("node folder exists")
                .display()
                .to_string()
        };
        assert_eq!(
            lines(&alpha_folder, "cwd.log"),
            [real(&alpha_folder), real(&alpha_folder)]
        );
        assert_eq!(lines(&beta_folder, "cwd.log"), [real(&beta_folder)]);
        assert_eq!(
            lines(&alpha_folder, "argv.log"),
            ["-p alpha first ", "-p --continue alpha again "]
        );
        assert_eq!(lines(&beta_folder, "argv.log"), ["-p beta first "]);
        assert!(
            !fixture_workspace(&directory).join("cwd.log").exists(),
            "no turn runs in the shared workspace"
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn a_nonzero_exit_and_a_malformed_answer_are_distinct_failures() {
        let directory = fixture_directory("failure");
        let failing = script(&directory, "claude-fails", "exit 3\n");
        let mut executor = CliAgentExecutor::new(spec("claude", failing, directory.clone()));
        let name = harness("claude");
        let outcome = settled(
            &mut executor,
            crate::test_effect(1),
            &invoke_effect(&name, None, b"turn"),
        );
        assert_eq!(outcome.result(), &Err(EffectFailure::TransportTerminal));

        let claude = adapter_for("claude").expect("boarded");
        assert!(matches!(
            claude.parse(b"not json"),
            Err(EffectFailure::InterpreterFault(
                InterpreterFault::InvalidInput
            ))
        ));
        assert!(matches!(
            claude.parse(b"{\"type\":\"result\",\"result\":\"x\",\"is_error\":true}"),
            Err(EffectFailure::TransportTerminal)
        ));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn a_provider_json_error_keeps_its_failure_arm_and_carries_the_body() {
        let directory = fixture_directory("provider-error-detail");
        let failing = script(
            &directory,
            "claude-provider-error",
            "printf '{\"type\":\"result\",\"result\":\"unknown model: typo-model\",\"is_error\":true}'\n",
        );
        let mut executor = CliAgentExecutor::new(spec("claude", failing, directory.clone()));
        let name = harness("claude");
        let (outcome, detail) = settled_with_detail(
            &mut executor,
            crate::test_effect(1),
            &invoke_effect(&name, None, b"turn"),
        );

        assert_eq!(outcome.result(), &Err(EffectFailure::TransportTerminal));
        assert_eq!(detail.as_deref(), Some("unknown model: typo-model"));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn malformed_provider_output_carries_human_detail_beside_the_fault_tag() {
        let directory = fixture_directory("malformed-provider-detail");
        let failing = script(
            &directory,
            "claude-malformed",
            "printf 'provider response was not json'\n",
        );
        let mut executor = CliAgentExecutor::new(spec("claude", failing, directory.clone()));
        let name = harness("claude");
        let (outcome, detail) = settled_with_detail(
            &mut executor,
            crate::test_effect(1),
            &invoke_effect(&name, None, b"turn"),
        );

        assert_eq!(
            outcome.result(),
            &Err(EffectFailure::InterpreterFault(
                InterpreterFault::InvalidInput
            ))
        );
        assert_eq!(detail.as_deref(), Some("provider response was not json"));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn a_nonzero_exit_carries_its_stderr_tail_as_detail() {
        let directory = fixture_directory("stderr-detail");
        let failing = script(
            &directory,
            "claude-fails",
            "printf 'not logged in: run claude login\\n' >&2\nexit 3\n",
        );
        let mut executor = CliAgentExecutor::new(spec("claude", failing, directory.clone()));
        let name = harness("claude");
        let (outcome, detail) = settled_with_detail(
            &mut executor,
            crate::test_effect(1),
            &invoke_effect(&name, None, b"turn"),
        );

        assert_eq!(outcome.result(), &Err(EffectFailure::TransportTerminal));
        assert_eq!(detail.as_deref(), Some("not logged in: run claude login\n"));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn a_nonzero_exit_carries_the_provider_body_and_the_stderr_tail() {
        for (label, script_body, expected) in [
            (
                "stdout-only",
                "printf '{\"type\":\"result\",\"result\":\"Not logged in - Please run /login\",\"is_error\":true}'\nexit 1\n",
                "Not logged in - Please run /login",
            ),
            (
                "stdout-and-stderr",
                "printf '{\"type\":\"result\",\"result\":\"quota exhausted\",\"is_error\":true}'\nprintf 'retry later\\n' >&2\nexit 1\n",
                "quota exhausted\nretry later\n",
            ),
        ] {
            let directory = fixture_directory(&format!("exit-body-{label}"));
            let failing = script(&directory, "claude-exit-body", script_body);
            let mut executor = CliAgentExecutor::new(spec("claude", failing, directory.clone()));
            let name = harness("claude");
            let (outcome, detail) = settled_with_detail(
                &mut executor,
                crate::test_effect(1),
                &invoke_effect(&name, None, b"turn"),
            );

            assert_eq!(
                outcome.result(),
                &Err(EffectFailure::TransportTerminal),
                "{label}"
            );
            assert_eq!(detail.as_deref(), Some(expected), "{label}");
            let _ = std::fs::remove_dir_all(directory);
        }
    }

    #[test]
    fn an_oversized_stderr_detail_is_a_marked_bounded_tail() {
        let directory = fixture_directory("stderr-detail-limit");
        let stderr = format!("{}TAIL-SENTINEL", "x".repeat(STDERR_DETAIL_LIMIT_BYTES * 2));
        let failing = script(
            &directory,
            "claude-fails-large",
            &format!("printf '{stderr}' >&2\nexit 4\n"),
        );
        let mut harness_spec = spec("claude", failing, directory.clone());
        harness_spec.max_output_bytes = STDERR_DETAIL_LIMIT_BYTES + 1024;
        let mut executor = CliAgentExecutor::new(harness_spec);
        let name = harness("claude");
        let (outcome, detail) = settled_with_detail(
            &mut executor,
            crate::test_effect(1),
            &invoke_effect(&name, None, b"turn"),
        );

        assert_eq!(outcome.result(), &Err(EffectFailure::TransportTerminal));
        let detail = detail.expect("a failed CLI carries bounded stderr detail");
        assert!(detail.len() <= STDERR_DETAIL_LIMIT_BYTES);
        assert!(detail.starts_with(STDERR_TRUNCATED_PREFIX));
        assert!(detail.ends_with("TAIL-SENTINEL"));
        let _ = std::fs::remove_dir_all(directory);
    }

    #[test]
    fn a_successful_cli_does_not_carry_stderr_detail() {
        let directory = fixture_directory("successful-stderr");
        let successful = script(
            &directory,
            "claude-warns",
            concat!(
                "printf 'a harmless warning\\n' >&2\n",
                "printf '{\"type\":\"result\",\"result\":\"done\",\"session_id\":\"s1\",\"is_error\":false}'\n",
            ),
        );
        let mut executor = CliAgentExecutor::new(spec("claude", successful, directory.clone()));
        let name = harness("claude");
        let (outcome, detail) = settled_with_detail(
            &mut executor,
            crate::test_effect(1),
            &invoke_effect(&name, None, b"turn"),
        );

        let result = step_result(&outcome);
        assert!(matches!(
            result.next(),
            AgentStepNext::Final { output, .. } if output.as_bytes() == b"done"
        ));
        assert_eq!(detail, None);
        let _ = std::fs::remove_dir_all(directory);
    }

    fn recorded_rc(node: &std::path::Path, name: &str) -> String {
        std::fs::read_to_string(node.join(name))
            .unwrap_or_else(|error| panic!("{name} was not recorded: {error}"))
    }
}
