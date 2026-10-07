
use crate::daemon::{runtime_arrival_retention::RuntimeArrivalLimits, webhook};
use circular_runtime::{
    AgentHarnessName, HttpHosts, NormalizedPath, NotificationChannel, ProcessTargets, ProgramName,
};
use engine::ClaimedOwnerLocalDaemon;
use engine::execution_profile::ProductExecutionProfile;
use std::collections::BTreeSet;
use std::io::Write as _;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt};
use std::path::Path;
use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

const CONFIG_DOCUMENT_NAME: &str = "config.toml";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OperatingDefault {
    table: &'static str,
    key: &'static str,
    value: u64,
}

impl OperatingDefault {
    fn path(&self) -> String {
        format!("{}.{}", self.table, self.key)
    }
}

impl std::fmt::Display for OperatingDefault {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}={}", self.path(), self.value)
    }
}

const PROCESS_DEADLINE_SECS: OperatingDefault = OperatingDefault {
    table: "process",
    key: "deadline_secs",
    value: 1_800,
};
const NOTIFY_HTTP_TIMEOUT_SECS: OperatingDefault = OperatingDefault {
    table: "notify",
    key: "http_timeout_secs",
    value: 20,
};
const ARRIVALS_MAX_MIB: OperatingDefault = OperatingDefault {
    table: "runtime_arrivals",
    key: "arrivals_max_mib",
    value: 256,
};
const ARRIVALS_MAX_RECORDS: OperatingDefault = OperatingDefault {
    table: "runtime_arrivals",
    key: "arrivals_max_records",
    value: 500_000,
};
const TOTAL_MAX_MIB: OperatingDefault = OperatingDefault {
    table: "runtime_arrivals",
    key: "total_max_mib",
    value: 2_048,
};

const REFERENCE_AGENT_HARNESS: &str = "reference";

const AGENT_CONTACT_LOG: &str = "agent-contacts.log";

const INITIAL_HTTP_HOSTS: [&str; 3] = ["localhost:*", "127.0.0.1:*", "[::1]:*"];

fn append_agent_contact(path: &Path, detail: &str) {
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        Ok(mut file) => {
            let _ = writeln!(file, "contact {detail}");
        }
        Err(error) => eprintln!("circular-daemon: agent contact log unavailable: {error}"),
    }
}

pub(crate) struct DaemonConfig {
    secret_custody: SecretCustody,
    peer_adapters: Vec<PeerAdapterBinding>,
    http_hosts: HttpHosts,
    process_targets: ProcessTargets,
    process_deadline: std::time::Duration,
    process_max_concurrent: Option<std::num::NonZeroUsize>,
    process_queue_capacity: Option<std::num::NonZeroUsize>,
    notification_channels: Vec<NotificationBinding>,
    notification_http_timeout: std::time::Duration,
    pub(crate) runtime_arrival_limits: RuntimeArrivalLimits,
    webhook: Option<WebhookBinding>,
    pub(crate) defaults_used: Vec<OperatingDefault>,
    pub(crate) effect_retry: engine::effect_retry::RetrySchedule,
    pub(crate) effect_retry_defaulted: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SecretCustody {
    Disabled,
    FileVaultV0,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentHarnessBinding {
    name: Box<str>,
    program: NormalizedPath,
}

impl AgentHarnessBinding {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn program(&self) -> &Path {
        self.program.as_path()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AgentHarnessBindRejection {
    UnknownHarness { name: String },
    InvalidProgramPath { program: String, reason: String },
    ProgramNotExecutable { program: String },
}

impl circular_protocol::rejection_code::Reasoned for AgentHarnessBindRejection {
    fn reason(&self) -> circular_protocol::rejection_code::RejectionReason {
        use circular_protocol::rejection_code::RejectionReason as R;
        match self {
            Self::UnknownHarness { .. } => R::UnknownHarness,
            Self::InvalidProgramPath { .. } => R::InvalidProgramPath,
            Self::ProgramNotExecutable { .. } => R::ProgramNotExecutable,
        }
    }
}

impl std::fmt::Display for AgentHarnessBindRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownHarness { name } => {
                let names: Vec<&str> = engine::cli_adapter_names().collect();
                write!(
                    formatter,
                    "unknown agent harness {}; {}",
                    circular_core::spelling::Quoted(name),
                    circular_core::spelling::allowed(
                        names
                            .iter()
                            .map(|name| circular_core::spelling::Quoted(name))
                    )
                )
            }
            Self::InvalidProgramPath { program, reason } => {
                write!(
                    formatter,
                    "agent program path {} is not an absolute normalized path: {reason}",
                    circular_core::spelling::Quoted(program)
                )
            }
            Self::ProgramNotExecutable { program } => write!(
                formatter,
                "agent program {} does not exist or is not an executable file",
                circular_core::spelling::Quoted(program)
            ),
        }
    }
}

struct NotificationBinding {
    name: Box<str>,
    sink: NotificationSink,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PeerAdapterBinding {
    BuiltIn,
    Declared(engine::peer_adapter::PeerAdapterClause),
}

impl PeerAdapterBinding {
    fn bind_profile(
        &self,
        profile: ProductExecutionProfile,
        home: &Path,
    ) -> Result<ProductExecutionProfile, String> {
        let Self::Declared(clause) = self else {
            return Ok(profile);
        };
        let adapter = clause.resolve(home);
        let name = adapter.name().as_str();
        let factory = adapter
            .factory()
            .map_err(|error| format!("peer adapter {name}: {error}"))?;
        let settings = adapter.settings().to_string();
        if settings.is_empty() {
            eprintln!("circular-daemon: peer adapter `{name}` registered");
        } else {
            eprintln!("circular-daemon: peer adapter `{name}` registered ({settings})");
        }
        profile
            .with_peer_adapter(adapter.name().clone(), factory)
            .map_err(|error| format!("peer adapter {name}: {error}"))
    }
}

enum NotificationSink {
    SlackWebhook { secret: Box<str> },
    SystemProgram { program: NormalizedPath },
}

struct WebhookBinding {
    bind: std::net::SocketAddr,
    bearer_secret: Box<str>,
}

pub(crate) fn daemon_config_from_state(
    daemon: &ClaimedOwnerLocalDaemon,
) -> Result<DaemonConfig, String> {
    write_initial_config_for_new_state(daemon.state_directory())?;
    load_daemon_config(daemon.state_directory())
}

fn write_initial_config_for_new_state(state_directory: &Path) -> Result<(), String> {
    let journal = engine::state_journal::state_journal_path(state_directory);
    match std::fs::symlink_metadata(&journal) {
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "failed to inspect the state journal {}: {error}",
                journal.display()
            ));
        }
    }
    let path = state_directory.join(CONFIG_DOCUMENT_NAME);
    if read_config_source(&path)?.is_some() {
        return Ok(());
    }
    let hosts = INITIAL_HTTP_HOSTS
        .iter()
        .map(|host| format!("\"{host}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let document = format!(
        "# Hosts a `request` actor may reach. \"host:*\" allows every port of that host.\n\
         [http]\nhosts = [{hosts}]\n"
    );
    parse_daemon_config(&document)
        .map_err(|error| format!("the initial daemon config did not validate: {error}"))?;
    publish_config(&path, document.as_bytes(), 0o600)?;
    eprintln!("circular-daemon: wrote the initial {CONFIG_DOCUMENT_NAME}: http.hosts = [{hosts}]");
    Ok(())
}

fn load_daemon_config(state_directory: &Path) -> Result<DaemonConfig, String> {
    let path = state_directory.join(CONFIG_DOCUMENT_NAME);
    let text = read_config_source(&path)?.unwrap_or_default();
    parse_daemon_config(&text)
        .map_err(|error| format!("ConfigRejected: daemon config {}: {error}", path.display()))
}

fn read_config_source(path: &Path) -> Result<Option<String>, String> {
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect daemon config {}: {error}",
                path.display()
            ));
        }
    };
    let mode = metadata.permissions().mode() & 0o7777;
    if mode & 0o077 != 0 {
        return Err(format!(
            "daemon config {} must not set group/other permission bits: {mode:04o}",
            path.display()
        ));
    }
    let source = std::fs::read_to_string(&path)
        .map_err(|error| format!("failed to read daemon config {}: {error}", path.display()))?;
    Ok(Some(source))
}

fn saved_agent_harnesses(
    state_directory: &Path,
) -> Result<Vec<Result<AgentHarnessBinding, String>>, String> {
    let path = state_directory.join(CONFIG_DOCUMENT_NAME);
    let rejected =
        |error: String| format!("ConfigRejected: daemon config {}: {error}", path.display());
    let Some(source) = read_config_source(&path)? else {
        return Ok(Vec::new());
    };
    Ok(agent_harness_lines(&source)
        .map_err(rejected)?
        .into_iter()
        .map(|line| line.map_err(rejected))
        .collect())
}

pub(crate) fn configured_agent_harnesses(
    state_directory: &Path,
) -> Result<Vec<AgentHarnessBinding>, String> {
    let mut refused = Vec::new();
    let mut bindings = Vec::new();
    for line in saved_agent_harnesses(state_directory)? {
        match line {
            Ok(binding) => bindings.push(binding),
            Err(reason) => refused.push(reason),
        }
    }
    if refused.is_empty() {
        Ok(bindings)
    } else {
        Err(refused.join("; "))
    }
}

fn agent_harness_lines(source: &str) -> Result<Vec<Result<AgentHarnessBinding, String>>, String> {
    let document = source
        .parse::<DocumentMut>()
        .map_err(|error| format!("invalid TOML: {error}"))?;
    Ok(document
        .get("agent")
        .map(parse_agent)
        .transpose()?
        .unwrap_or_default())
}

pub(crate) fn validate_agent_harness(
    name: String,
    program: String,
) -> Result<AgentHarnessBinding, AgentHarnessBindRejection> {
    if engine::cli_adapter_for(&name).is_none() {
        return Err(AgentHarnessBindRejection::UnknownHarness { name });
    }
    let program = NormalizedPath::new(&program).map_err(|error| {
        AgentHarnessBindRejection::InvalidProgramPath {
            program: program.clone(),
            reason: error.to_string(),
        }
    })?;
    if !engine::cli_program_is_executable(program.as_path()) {
        return Err(AgentHarnessBindRejection::ProgramNotExecutable {
            program: program.as_path().display().to_string(),
        });
    }

    Ok(AgentHarnessBinding {
        name: name.into(),
        program,
    })
}

pub(crate) fn admit_agent_harness(
    name: String,
    program: Option<String>,
) -> Result<Option<AgentHarnessBinding>, AgentHarnessBindRejection> {
    match program {
        Some(program) => validate_agent_harness(name, program).map(Some),
        None if engine::cli_adapter_for(&name).is_none() => {
            Err(AgentHarnessBindRejection::UnknownHarness { name })
        }
        None => Ok(None),
    }
}

pub(crate) fn write_agent_harness(
    state_directory: &Path,
    name: &str,
    binding: Option<&AgentHarnessBinding>,
) -> Result<(), String> {
    let path = state_directory.join(CONFIG_DOCUMENT_NAME);
    let source = read_config_source(&path)?;
    let mode = match &source {
        Some(_) => std::fs::metadata(&path)
            .map(|metadata| metadata.permissions().mode() & 0o7777)
            .map_err(|error| {
                format!(
                    "failed to inspect daemon config {}: {error}",
                    path.display()
                )
            })?,
        None => 0o600,
    };
    let source = source.unwrap_or_default();
    let invalid = |what: &str, error: String| {
        format!(
            "ConfigRejected: {what} daemon config {}: {error}",
            path.display()
        )
    };
    let mut document = source
        .parse::<DocumentMut>()
        .map_err(|error| invalid("the", format!("invalid TOML: {error}")))?;
    edit_agent_harness(&mut document, name, binding).map_err(|error| invalid("the", error))?;
    let rendered = document.to_string();
    agent_harness_lines(&rendered).map_err(|error| invalid("the rewritten", error))?;
    if rendered != source {
        publish_config(&path, rendered.as_bytes(), mode)?;
    }
    Ok(())
}

fn edit_agent_harness(
    document: &mut DocumentMut,
    name: &str,
    binding: Option<&AgentHarnessBinding>,
) -> Result<(), String> {
    if binding.is_none()
        && document
            .get("agent")
            .and_then(Item::as_table)
            .is_none_or(|agent| agent.get("harnesses").is_none())
    {
        return Ok(());
    }
    if document.get("agent").is_none() {
        document.insert("agent", Item::Table(Table::new()));
    }
    let agent = document
        .get_mut("agent")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| "agent must be a table".to_owned())?;
    if agent.get("harnesses").is_none() {
        agent.insert("harnesses", Item::Value(Value::Array(Array::new())));
    }
    let harnesses = agent
        .get_mut("harnesses")
        .and_then(Item::as_array_mut)
        .ok_or_else(|| "agent.harnesses must be an array".to_owned())?;
    let matching = harnesses
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            value
                .as_inline_table()
                .and_then(|table| table.get("name"))
                .and_then(Value::as_str)
                .filter(|existing| *existing == name)
                .map(|_| index)
        })
        .collect::<Vec<_>>();
    let keep = match binding {
        Some(binding) => {
            let mut row = InlineTable::new();
            row.insert("name", Value::from(binding.name()));
            row.insert(
                "program",
                Value::from(binding.program().to_string_lossy().into_owned()),
            );
            match matching.first().copied() {
                Some(first) => {
                    harnesses.replace(first, row);
                }
                None => harnesses.push(row),
            }
            1
        }
        None => 0,
    };
    for duplicate in matching.into_iter().skip(keep).rev() {
        harnesses.remove(duplicate);
    }
    Ok(())
}

pub(crate) fn agent_binding_material(
    state_directory: &Path,
    deadline: std::time::Duration,
    binding: &AgentHarnessBinding,
) -> Result<
    (
        AgentHarnessName,
        NormalizedPath,
        engine::execution_profile::AgentExecutorFactory,
    ),
    String,
> {
    let name = AgentHarnessName::try_from_normalized(binding.name.clone())
        .map_err(|error| format!("invalid harness name: {error}"))?;
    let spec = engine::CliHarnessSpec::for_harness(
        &binding.name,
        binding.program.as_path().to_path_buf(),
        state_directory.to_path_buf(),
        state_directory.join("workspace"),
        deadline,
    )
    .ok_or_else(|| format!("agent harness {:?} has no adapter", binding.name))?;
    let contact_log = state_directory.join(AGENT_CONTACT_LOG);
    let factory: engine::execution_profile::AgentExecutorFactory = std::sync::Arc::new(move || {
        let contact_log = contact_log.clone();
        Box::new(
            engine::CliAgentExecutor::new(spec.clone()).with_contact_witness(
                move |has_provider_session| {
                    append_agent_contact(
                        &contact_log,
                        if has_provider_session {
                            "session=yes"
                        } else {
                            "session=no"
                        },
                    );
                },
            ),
        ) as Box<dyn circular_runtime::Interpreter<circular_runtime::EffectId> + Send>
    });
    Ok((name, binding.program.clone(), factory))
}

pub(crate) fn with_saved_agent_bindings(
    execution: &ProductExecutionProfile,
    state_directory: &Path,
) -> ProductExecutionProfile {
    let mut execution = execution.clone();
    let lines = match saved_agent_harnesses(state_directory) {
        Ok(lines) => lines,
        Err(reason) => {
            eprintln!("circular-daemon: no saved agent harness binding stands: {reason}");
            return execution;
        }
    };
    for line in lines {
        let material = line.and_then(|binding| {
            let deadline = execution.agent_deadline().ok_or_else(|| {
                "the execution profile has no process deadline for a saved harness binding"
                    .to_owned()
            })?;
            agent_binding_material(state_directory, deadline, &binding)
        });
        match material {
            Ok((name, program, factory)) => {
                eprintln!(
                    "circular-daemon: agent harness {:?} — CLI subprocess executor",
                    name.as_str()
                );
                execution = execution.with_agent_binding(name, Some((program, factory)));
            }
            Err(reason) => {
                eprintln!(
                    "circular-daemon: a saved agent harness binding does not stand: {reason}"
                );
            }
        }
    }
    execution
}

pub(crate) fn detected_agent_harnesses()
-> Result<Vec<(&'static str, Option<AgentHarnessBinding>)>, String> {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| {
            "this user's home is not named in the environment, so a harness candidate path \
             cannot be resolved"
                .to_owned()
        })?;
    Ok(engine::cli_adapter_candidates(&home)
        .map(|(name, candidates)| {
            let found = candidates.iter().find_map(|candidate| {
                validate_agent_harness(name.to_owned(), candidate.to_str()?.to_owned()).ok()
            });
            (name, found)
        })
        .collect())
}

fn publish_config(path: &Path, bytes: &[u8], mode: u32) -> Result<(), String> {
    let temporary = path.with_file_name(format!(
        ".{CONFIG_DOCUMENT_NAME}.{}.tmp",
        std::process::id()
    ));
    let published = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temporary)
            .map_err(|error| {
                format!(
                    "failed to create temporary daemon config {}: {error}",
                    temporary.display()
                )
            })?;
        file.write_all(bytes).map_err(|error| {
            format!(
                "failed to write temporary daemon config {}: {error}",
                temporary.display()
            )
        })?;
        file.sync_all().map_err(|error| {
            format!(
                "failed to sync temporary daemon config {}: {error}",
                temporary.display()
            )
        })?;
        std::fs::rename(&temporary, path).map_err(|error| {
            format!(
                "failed to publish daemon config {}: {error}",
                path.display()
            )
        })
    })();
    if published.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    published
}

fn parse_daemon_config(source: &str) -> Result<DaemonConfig, String> {
    let document = source
        .parse::<DocumentMut>()
        .map_err(|error| format!("invalid TOML: {error}"))?;
    reject_unknown_table(
        document.as_table(),
        "root",
        &[
            "agent",
            "peer",
            "http",
            "process",
            "notify",
            "runtime_arrivals",
            "journal",
            "webhook",
            "secrets",
            "effects",
        ],
    )?;

    if let Some(item) = document.get("journal") {
        let table = required_table(item, "journal")?;
        if table.contains_key("retain") {
            return Err(
                "unknown config key journal.retain. Remove journal.retain from config.toml."
                    .to_owned(),
            );
        }
        reject_unknown_table(table, "journal", &[])?;
    }

    let peer_adapters = document
        .get("peer")
        .map(parse_peer)
        .transpose()?
        .unwrap_or_default();
    let http_hosts = document
        .get("http")
        .map(parse_http)
        .transpose()?
        .unwrap_or_default();
    let mut defaults_used = Vec::new();
    let (process_targets, process_deadline, process_max_concurrent, process_queue_capacity) =
        parse_process(document.get("process"), &mut defaults_used)?;
    let (notification_channels, notification_http_timeout) =
        parse_notify(document.get("notify"), &mut defaults_used)?;
    let runtime_arrival_limits =
        parse_runtime_arrivals(document.get("runtime_arrivals"), &mut defaults_used)?;
    let webhook = document.get("webhook").map(parse_webhook).transpose()?;
    let (effect_retry, effect_retry_defaulted) = parse_effects(document.get("effects"))?;
    let secret_custody = document
        .get("secrets")
        .map(parse_secrets)
        .transpose()?
        .unwrap_or(SecretCustody::Disabled);

    Ok(DaemonConfig {
        secret_custody,
        peer_adapters,
        http_hosts,
        process_targets,
        process_deadline,
        process_max_concurrent,
        process_queue_capacity,
        notification_channels,
        notification_http_timeout,
        runtime_arrival_limits,
        webhook,
        defaults_used,
        effect_retry,
        effect_retry_defaulted,
    })
}

fn parse_effects(
    item: Option<&Item>,
) -> Result<(engine::effect_retry::RetrySchedule, bool), String> {
    let key = circular_actors::retry_config::RETRY_FIELD;
    let Some(item) = item else {
        return Ok((engine::effect_retry::RetrySchedule::creator_default(), true));
    };
    let table = required_table(item, "effects")?;
    reject_unknown_table(table, "effects", &[key])?;
    let Some(value) = table.get(key) else {
        return Ok((engine::effect_retry::RetrySchedule::creator_default(), true));
    };
    let refused = || {
        format!(
            "effects.{key}: ConfigRejected (expected an array of whole milliseconds, each at least 1)"
        )
    };
    let items = value.as_array().ok_or_else(refused)?;
    let value = circular_core::Value::array(items.iter().map(|item| match item.as_integer() {
        Some(ms) => circular_core::Value::Int(ms),
        None => circular_core::Value::Null,
    }));
    let waits = circular_actors::retry_config::schedule(&value).map_err(|_| refused())?;
    Ok((
        engine::effect_retry::RetrySchedule::from_checked_ms(waits),
        false,
    ))
}

fn parse_runtime_arrivals(
    item: Option<&Item>,
    defaults_used: &mut Vec<OperatingDefault>,
) -> Result<RuntimeArrivalLimits, String> {
    let table = item
        .map(|item| required_table(item, "runtime_arrivals"))
        .transpose()?;
    if let Some(table) = table {
        reject_unknown_table(
            table,
            "runtime_arrivals",
            &[
                ARRIVALS_MAX_MIB.key,
                ARRIVALS_MAX_RECORDS.key,
                TOTAL_MAX_MIB.key,
            ],
        )?;
    }
    let arrivals_max_mib = operating_value(table, ARRIVALS_MAX_MIB, defaults_used)?;
    let arrivals_max_records = operating_value(table, ARRIVALS_MAX_RECORDS, defaults_used)?;
    let total_max_mib = operating_value(table, TOTAL_MAX_MIB, defaults_used)?;
    RuntimeArrivalLimits::from_mib(arrivals_max_mib, arrivals_max_records, total_max_mib)
}

fn parse_agent(item: &Item) -> Result<Vec<Result<AgentHarnessBinding, String>>, String> {
    let table = required_table(item, "agent")?;
    reject_unknown_table(table, "agent", &["harnesses"])?;
    let Some(harnesses) = table.get("harnesses") else {
        return Ok(Vec::new());
    };
    let harnesses = harnesses
        .as_array()
        .ok_or_else(|| "agent.harnesses must be an array".to_owned())?;
    Ok(harnesses
        .iter()
        .enumerate()
        .map(|(index, value)| parse_agent_harness(index, value))
        .collect())
}

fn parse_agent_harness(index: usize, value: &Value) -> Result<AgentHarnessBinding, String> {
    let path = format!("agent.harnesses[{index}]");
    let harness = value
        .as_inline_table()
        .ok_or_else(|| format!("{path} must be an inline table {{ name, program }}"))?;
    reject_unknown_inline(harness, &path, &["name", "program"])?;
    let name = required_inline_string(harness, "name", &format!("{path}.name"))?;
    if !engine::cli_adapter_names().any(|known| known == name) {
        return Err(format!(
            "{path}.name is not a known agent harness (allowed: {}): {name}",
            engine::cli_adapter_names().collect::<Vec<_>>().join("·")
        ));
    }
    let program = normalized_path(
        required_inline_string(harness, "program", &format!("{path}.program"))?,
        &format!("{path}.program"),
    )?;
    Ok(AgentHarnessBinding {
        name: name.into(),
        program,
    })
}

fn parse_peer(item: &Item) -> Result<Vec<PeerAdapterBinding>, String> {
    let table = required_table(item, "peer")?;
    reject_unknown_table(table, "peer", &["adapter"])?;
    let Some(adapters) = table.get("adapter") else {
        return Ok(Vec::new());
    };
    let adapters = adapters
        .as_array_of_tables()
        .ok_or_else(|| "peer.adapter must be an array of tables ([[peer.adapter]])".to_owned())?;
    let mut names = BTreeSet::new();
    let mut parsed = Vec::with_capacity(adapters.len());
    for (index, adapter) in adapters.iter().enumerate() {
        let path = format!("peer.adapter[{index}]");
        let name = required_string(adapter, "name", &format!("{path}.name"))?.trim();
        if !names.insert(name.to_owned()) {
            return Err(format!("peer.adapter contains duplicate adapter {name:?}"));
        }
        let binding = if name == engine::execution_profile::BUILT_IN_PEER_ADAPTER {
            reject_unknown_table(adapter, &path, &["name"])?;
            PeerAdapterBinding::BuiltIn
        } else if let Some(declaration) = engine::peer_adapter::peer_adapter_declaration(name) {
            let keys = std::iter::once("name")
                .chain(declaration.setting_keys())
                .collect::<Vec<_>>();
            reject_unknown_table(adapter, &path, &keys)?;
            PeerAdapterBinding::Declared(declaration.clause(|key| {
                adapter
                    .get(key)
                    .map(|_| {
                        required_string(adapter, key, &format!("{path}.{key}")).map(Into::into)
                    })
                    .transpose()
            })?)
        } else {
            return Err(format!("{path}.name must be {}", known_peer_adapters()));
        };
        parsed.push(binding);
    }
    Ok(parsed)
}

fn known_peer_adapters() -> String {
    let names = engine::peer_adapter::selectable_peer_adapter_names().collect::<Vec<_>>();
    match names.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{}, or {last}", rest.join(", ")),
        _ => names.join(""),
    }
}

fn parse_http(item: &Item) -> Result<HttpHosts, String> {
    let table = required_table(item, "http")?;
    reject_unknown_table(table, "http", &["hosts"])?;
    let hosts = required_array(table, "hosts", "http.hosts")?;
    let mut parsed: Vec<Box<str>> = Vec::with_capacity(hosts.len());
    for (index, value) in hosts.iter().enumerate() {
        let host = value
            .as_str()
            .ok_or_else(|| format!("http.hosts[{index}] must be a string"))?
            .trim();
        if host.is_empty() {
            return Err(format!("http.hosts[{index}] must not be empty"));
        }
        if !circular_runtime::authority_entry_is_valid(host) {
            return Err(format!(
                "http.hosts[{index}] {host:?} is not host, host:port or host:*"
            ));
        }
        if parsed.iter().any(|seen| seen.as_ref() == host) {
            return Err(format!("http.hosts contains duplicate host {host:?}"));
        }
        parsed.push(host.into());
    }
    Ok(HttpHosts::exact(parsed))
}

fn parse_process(
    item: Option<&Item>,
    defaults_used: &mut Vec<OperatingDefault>,
) -> Result<
    (
        ProcessTargets,
        std::time::Duration,
        Option<std::num::NonZeroUsize>,
        Option<std::num::NonZeroUsize>,
    ),
    String,
> {
    let table = item
        .map(|item| required_table(item, "process"))
        .transpose()?;
    if let Some(table) = table {
        reject_unknown_table(
            table,
            "process",
            &[
                "allowlist",
                PROCESS_DEADLINE_SECS.key,
                "max_concurrent",
                "queue_capacity",
            ],
        )?;
    }
    let mut targets = Vec::new();
    if let Some(allowlist) = table.and_then(|table| table.get("allowlist")) {
        let allowlist = allowlist
            .as_array()
            .ok_or_else(|| "process.allowlist must be an array".to_owned())?;
        targets.reserve(allowlist.len());
        for (index, value) in allowlist.iter().enumerate() {
            let path = format!("process.allowlist[{index}]");
            let program = value
                .as_str()
                .ok_or_else(|| format!("{path} must be a string"))?;
            let program = normalized_path(program, &path)?;
            targets.push(ProgramName::from_normalized(
                program.as_path().to_string_lossy().into_owned(),
            ));
        }
    }
    let deadline = std::time::Duration::from_secs(operating_value(
        table,
        PROCESS_DEADLINE_SECS,
        defaults_used,
    )?);
    let max_concurrent = table
        .and_then(|table| table.get("max_concurrent"))
        .map(|value| {
            value
                .as_integer()
                .and_then(|value| usize::try_from(value).ok())
                .and_then(std::num::NonZeroUsize::new)
                .ok_or_else(|| "process.max_concurrent must be a positive integer".to_owned())
        })
        .transpose()?;
    if !targets.is_empty() && max_concurrent.is_none() {
        return Err(
            "process.max_concurrent is required when the process executor is enabled".into(),
        );
    }
    let queue_capacity = table
        .and_then(|table| table.get("queue_capacity"))
        .map(|value| {
            value
                .as_integer()
                .and_then(|value| usize::try_from(value).ok())
                .and_then(std::num::NonZeroUsize::new)
                .ok_or_else(|| "process.queue_capacity must be a positive integer".to_owned())
        })
        .transpose()?;
    Ok((
        ProcessTargets::exact(targets),
        deadline,
        max_concurrent,
        queue_capacity,
    ))
}

fn parse_notify(
    item: Option<&Item>,
    defaults_used: &mut Vec<OperatingDefault>,
) -> Result<(Vec<NotificationBinding>, std::time::Duration), String> {
    let table = item
        .map(|item| required_table(item, "notify"))
        .transpose()?;
    if let Some(table) = table {
        reject_unknown_table(table, "notify", &[NOTIFY_HTTP_TIMEOUT_SECS.key, "channel"])?;
    }
    let timeout = std::time::Duration::from_secs(operating_value(
        table,
        NOTIFY_HTTP_TIMEOUT_SECS,
        defaults_used,
    )?);
    let Some(channels) = table.and_then(|table| table.get("channel")) else {
        return Ok((Vec::new(), timeout));
    };
    let channels = channels.as_array_of_tables().ok_or_else(|| {
        "notify.channel must be an array of tables ([[notify.channel]])".to_owned()
    })?;
    Ok((parse_notification_channels(channels)?, timeout))
}

fn parse_notification_channels(
    channels: &ArrayOfTables,
) -> Result<Vec<NotificationBinding>, String> {
    let mut names = BTreeSet::new();
    let mut parsed = Vec::with_capacity(channels.len());
    for (index, channel) in channels.iter().enumerate() {
        let path = format!("notify.channel[{index}]");
        reject_unknown_table(channel, &path, &["name", "sink"])?;
        let name = required_string(channel, "name", &format!("{path}.name"))?.trim();
        if name.is_empty() {
            return Err(format!("{path}.name must not be empty"));
        }
        if !names.insert(name.to_owned()) {
            return Err(format!(
                "notify.channel contains duplicate channel {name:?}"
            ));
        }
        let sink = channel
            .get("sink")
            .and_then(Item::as_inline_table)
            .ok_or_else(|| format!("{path}.sink must be an inline table"))?;
        reject_unknown_inline(
            sink,
            &format!("{path}.sink"),
            &["slack_webhook", "system_program"],
        )?;
        let sink = match (sink.get("slack_webhook"), sink.get("system_program")) {
            (Some(reference), None) => NotificationSink::SlackWebhook {
                secret: parse_secret_reference(reference, &format!("{path}.sink.slack_webhook"))?,
            },
            (None, Some(program)) => NotificationSink::SystemProgram {
                program: normalized_path(
                    program.as_str().ok_or_else(|| {
                        format!("{path}.sink.system_program must be an absolute path string")
                    })?,
                    &format!("{path}.sink.system_program"),
                )?,
            },
            (Some(_), Some(_)) => {
                return Err(format!("{path}.sink must select exactly one sink"));
            }
            (None, None) => return Err(format!("{path}.sink must specify a sink")),
        };
        parsed.push(NotificationBinding {
            name: name.into(),
            sink,
        });
    }
    Ok(parsed)
}

fn parse_webhook(item: &Item) -> Result<WebhookBinding, String> {
    let table = required_table(item, "webhook")?;
    reject_unknown_table(table, "webhook", &["bind", "bearer"])?;
    let bind = required_string(table, "bind", "webhook.bind")?
        .parse()
        .map_err(|_| "webhook.bind must be a socket address".to_owned())?;
    let bearer = table
        .get("bearer")
        .and_then(Item::as_value)
        .ok_or_else(|| "webhook.bearer must be { secret = \"name\" }".to_owned())?;
    Ok(WebhookBinding {
        bind,
        bearer_secret: parse_secret_reference(bearer, "webhook.bearer")?,
    })
}

fn parse_secrets(item: &Item) -> Result<SecretCustody, String> {
    let table = required_table(item, "secrets")?;
    reject_unknown_table(table, "secrets", &["custody"])?;
    let custody = table
        .get("custody")
        .and_then(Item::as_inline_table)
        .ok_or_else(|| "secrets.custody must be an inline table".to_owned())?;
    reject_unknown_inline(
        custody,
        "secrets.custody",
        &["keychain", "kek_file", "file_vault_v0"],
    )?;
    match (
        custody.get("keychain"),
        custody.get("kek_file"),
        custody.get("file_vault_v0"),
    ) {
        (Some(keychain), None, None) => {
            let keychain = keychain
                .as_inline_table()
                .ok_or_else(|| "secrets.custody.keychain must be an inline table".to_owned())?;
            reject_unknown_inline(keychain, "secrets.custody.keychain", &["account"])?;
            let account =
                required_inline_string(keychain, "account", "secrets.custody.keychain.account")?;
            if account.is_empty() {
                return Err("secrets.custody.keychain.account must not be empty".to_owned());
            }
            Err(
                "secrets.custody.keychain is not wired to daemon credential resolution; \
                 persistent daemon authorization and background-state evidence are missing; \
                 no file vault fallback is allowed"
                    .to_owned(),
            )
        }
        (None, Some(kek_file), None) => {
            let kek_file = kek_file
                .as_inline_table()
                .ok_or_else(|| "secrets.custody.kek_file must be an inline table".to_owned())?;
            reject_unknown_inline(kek_file, "secrets.custody.kek_file", &["path", "mode"])?;
            normalized_path(
                required_inline_string(kek_file, "path", "secrets.custody.kek_file.path")?,
                "secrets.custody.kek_file.path",
            )?;
            let mode = required_inline_string(kek_file, "mode", "secrets.custody.kek_file.mode")?;
            if mode != "0600" {
                return Err("secrets.custody.kek_file.mode must be \"0600\"".to_owned());
            }
            Err(
                "secrets.custody.kek_file is not wired to daemon credential resolution; \
                 a sealed encrypted-envelope source, authenticated codec, and OS-protected \
                 key opener are missing; no file vault fallback is allowed"
                    .to_owned(),
            )
        }
        (None, None, Some(compatibility)) => {
            let compatibility = compatibility.as_inline_table().ok_or_else(|| {
                "secrets.custody.file_vault_v0 must be an empty inline table".to_owned()
            })?;
            reject_unknown_inline(compatibility, "secrets.custody.file_vault_v0", &[])?;
            Ok(SecretCustody::FileVaultV0)
        }
        (None, None, None) => Err("secrets.custody must specify a custody method".to_owned()),
        _ => Err(
            "secrets.custody must select exactly one of keychain, kek_file, and file_vault_v0"
                .to_owned(),
        ),
    }
}

fn parse_secret_reference(value: &Value, path: &str) -> Result<Box<str>, String> {
    let reference = value
        .as_inline_table()
        .ok_or_else(|| format!("{path} must be {{ secret = \"name\" }}, not a plaintext string"))?;
    reject_unknown_inline(reference, path, &["secret"])?;
    let name = required_inline_string(reference, "secret", &format!("{path}.secret"))?;
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err(format!("{path}.secret must be a valid vault resource name"));
    }
    Ok(name.into())
}

fn normalized_path(raw: &str, path: &str) -> Result<NormalizedPath, String> {
    NormalizedPath::new(raw)
        .map_err(|error| format!("failed to normalize program path {raw:?} in {path}: {error}"))
}

fn required_table<'a>(item: &'a Item, path: &str) -> Result<&'a Table, String> {
    item.as_table()
        .ok_or_else(|| format!("{path} must be a table"))
}

fn required_array<'a>(table: &'a Table, key: &str, path: &str) -> Result<&'a Array, String> {
    table
        .get(key)
        .and_then(Item::as_array)
        .ok_or_else(|| format!("{path} must be an array"))
}

fn required_string<'a>(table: &'a Table, key: &str, path: &str) -> Result<&'a str, String> {
    table
        .get(key)
        .and_then(Item::as_str)
        .ok_or_else(|| format!("{path} must be a string"))
}

fn required_inline_string<'a>(
    table: &'a InlineTable,
    key: &str,
    path: &str,
) -> Result<&'a str, String> {
    table
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{path} must be a string"))
}

fn operating_value(
    table: Option<&Table>,
    default: OperatingDefault,
    defaults_used: &mut Vec<OperatingDefault>,
) -> Result<u64, String> {
    let Some(value) = table.and_then(|table| table.get(default.key)) else {
        defaults_used.push(default);
        return Ok(default.value);
    };
    let path = default.path();
    let value = value
        .as_integer()
        .ok_or_else(|| format!("{path} must be a positive integer"))?;
    u64::try_from(value)
        .ok()
        .filter(|value| *value != 0)
        .ok_or_else(|| format!("{path} must be an integer greater than zero"))
}

fn reject_unknown_table(table: &Table, path: &str, allowed: &[&str]) -> Result<(), String> {
    for (key, _) in table.iter() {
        if !allowed.contains(&key) {
            return Err(format!("unknown config key {path}.{key}"));
        }
    }
    Ok(())
}

fn reject_unknown_inline(table: &InlineTable, path: &str, allowed: &[&str]) -> Result<(), String> {
    for (key, _) in table.iter() {
        if !allowed.contains(&key) {
            return Err(format!("unknown config key {path}.{key}"));
        }
    }
    Ok(())
}

pub(crate) fn product_execution_profile(
    daemon: &ClaimedOwnerLocalDaemon,
    config: &DaemonConfig,
    secret_vault: Option<std::sync::Arc<engine::SecretVault>>,
    reference_agent_harness: bool,
) -> Result<ProductExecutionProfile, String> {
    let process_deadline = config.process_deadline;
    let notification_http_timeout = config.notification_http_timeout;
    let workspace = daemon.state_directory().join("workspace");
    std::fs::create_dir_all(&workspace).map_err(|error| {
        format!(
            "failed to create filesystem executor workspace {}: {error}",
            workspace.display()
        )
    })?;
    let workspace = NormalizedPath::new(&workspace).map_err(|error| {
        format!(
            "failed to normalize filesystem executor workspace {}: {error}",
            workspace.display()
        )
    })?;
    let mut execution = ProductExecutionProfile::new();
    execution = execution
        .with_effect_retry(config.effect_retry.clone())
        .with_agent_deadline(process_deadline)
        .with_config_defaults(
            config
                .defaults_used
                .iter()
                .map(|default| (default.path(), default.value)),
        )
        .with_secret_vault(secret_vault.clone());
    let contact_log = daemon.state_directory().join(AGENT_CONTACT_LOG);
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| {
            "this user's home is not named in the environment, so a peer adapter path \
         cannot be resolved"
                .to_owned()
        })?;
    for binding in &config.peer_adapters {
        execution = binding.bind_profile(execution, &home)?;
    }
    eprintln!(
        "circular-daemon: HTTP fetch: {} [http] hosts entries",
        config.http_hosts.len()
    );
    execution = execution.with_http_fetch(
        config.http_hosts.clone(),
        engine::execution_profile::HTTP_MAX_BODY_BYTES,
        engine::execution_profile::HTTP_TIMEOUT,
        secret_vault.clone(),
    );
    if !config.process_targets.is_empty() {
        eprintln!(
            "circular-daemon: process executor enabled for {} exact program(s)",
            config.process_targets.len()
        );
        execution = execution.with_process_executor(
            config.process_targets.clone(),
            workspace.clone(),
            64 * 1024,
            process_deadline,
            config
                .process_max_concurrent
                .expect("process configuration validated"),
            config.process_queue_capacity,
        );
    }
    for binding in &config.notification_channels {
        let channel = NotificationChannel::from_normalized(binding.name.clone());
        let endpoint = match &binding.sink {
            NotificationSink::SlackWebhook { secret } => {
                let vault = secret_vault
                    .clone()
                    .ok_or_else(|| "notification secret vault unavailable".to_owned())?;
                if !engine_secrets::engine_integration::vault_contains_resource(&vault, secret) {
                    return Err("notification secret reference unavailable".to_owned());
                }
                eprintln!(
                    "circular-daemon: Slack notification channel {:?} enabled",
                    binding.name
                );
                engine::NotificationEndpoint::SlackIncomingWebhook {
                    vault,
                    resource: secret.clone(),
                    timeout: notification_http_timeout,
                }
            }
            NotificationSink::SystemProgram { program } => {
                eprintln!(
                    "circular-daemon: system notification channel {:?} enabled",
                    binding.name
                );
                engine::NotificationEndpoint::NotifierProgram {
                    program: program
                        .as_path()
                        .to_string_lossy()
                        .into_owned()
                        .into_boxed_str(),
                    deadline: process_deadline,
                }
            }
        };
        execution = execution.with_notification_endpoint(channel, endpoint);
    }
    if reference_agent_harness {
        let harness = AgentHarnessName::try_from_normalized(REFERENCE_AGENT_HARNESS)
            .map_err(|error| format!("invalid reference harness name: {error}"))?;
        eprintln!(
            "circular-daemon: agent harness {REFERENCE_AGENT_HARNESS:?} — explicitly enabled \
             deterministic reference executor, not an actual provider — contacts at {}",
            contact_log.display()
        );
        execution = execution
            .with_agent_executor(
                harness,
                std::sync::Arc::new(move || {
                    let contact_log = contact_log.clone();
                    Box::new(
                        engine::ReferenceAgentExecutor::new(engine::ReferenceAgentContacts::new())
                            .with_witness(move |ordinal| {
                                append_agent_contact(&contact_log, &ordinal.to_string());
                            }),
                    )
                        as Box<dyn circular_runtime::Interpreter<circular_runtime::EffectId> + Send>
                }),
            )
            .map_err(|error| format!("failed to assemble agent executor: {error}"))?;
    }
    Ok(execution)
}

pub(crate) fn secret_vault_from_state(
    daemon: &ClaimedOwnerLocalDaemon,
    config: &DaemonConfig,
) -> Result<Option<std::sync::Arc<engine::SecretVault>>, String> {
    load_secret_vault(daemon.state_directory(), config.secret_custody)
}

fn load_secret_vault(
    state_directory: &Path,
    custody: SecretCustody,
) -> Result<Option<std::sync::Arc<engine::SecretVault>>, String> {
    let root = state_directory.join("secrets");
    match std::fs::symlink_metadata(&root) {
        Ok(_) if custody == SecretCustody::Disabled => Err(
            "secret vault exists but no custody backend is selected; the vault's 0600 files \
             require explicit [secrets] custody = { file_vault_v0 = {} }"
                .to_owned(),
        ),
        Ok(_) => {
            let vault = engine::SecretVault::load(&root)
                .map_err(|error| format!("failed to load secret vault: {error}"))?;
            eprintln!(
                "circular-daemon: explicit file_vault_v0 custody, {} secret vault resources",
                vault.len()
            );
            Ok(Some(std::sync::Arc::new(vault)))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => match custody {
            SecretCustody::Disabled => Ok(None),
            SecretCustody::FileVaultV0 => {
                Err("secrets.custody.file_vault_v0 requires the secret vault directory".to_owned())
            }
        },
        Err(error) => Err(format!("failed to inspect secret vault root: {error}")),
    }
}

pub(crate) fn webhook_gateway_from_config(
    config: &DaemonConfig,
    secret_vault: Option<std::sync::Arc<engine::SecretVault>>,
) -> Result<Option<webhook::WebhookGateway>, String> {
    let Some(binding) = &config.webhook else {
        return Ok(None);
    };
    let Some(secret_vault) = secret_vault else {
        return Err(format!(
            "webhook.bearer requires secret vault resource {:?}",
            binding.bearer_secret
        ));
    };
    if !engine_secrets::engine_integration::vault_contains_resource(
        &secret_vault,
        &binding.bearer_secret,
    ) {
        return Err(format!(
            "webhook.bearer requires secret vault resource {:?}",
            binding.bearer_secret
        ));
    }
    let gateway =
        webhook::WebhookGateway::bind(binding.bind, secret_vault, binding.bearer_secret.clone())
            .map_err(|error| {
                format!(
                    "failed to open webhook ingress at {}: {error}",
                    binding.bind
                )
            })?;
    eprintln!(
        "circular-daemon: webhook ingress listening at http://{}/v1/ingress/<mount>",
        gateway.address()
    );
    Ok(Some(gateway))
}

