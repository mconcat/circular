
use crate::cli_agent::{
    CliHarnessAdapter, CliInvocation, CliTurn, EnvValue, bounded_provider_detail,
    executor_owns_environment, provider_output_detail,
};
use crate::harness_boundary::{BoundaryDeclaration, DeclaredPath, EgressEntry};
use circular_runtime::{EffectFailure, InterpreterFault};
use serde_json::Value as Json;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use toml_edit::{DocumentMut, Item, TableLike};

const DECLARATIONS: [(&str, &str); 3] = [
    (
        "claude.toml",
        include_str!("../harness-adapters/claude.toml"),
    ),
    ("codex.toml", include_str!("../harness-adapters/codex.toml")),
    ("pi.toml", include_str!("../harness-adapters/pi.toml")),
];

const HOST: &str = include_str!("../harness-adapters/host.toml");

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct DeclarationError(String);

impl std::fmt::Display for DeclarationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

pub(crate) fn error(message: impl Into<String>) -> DeclarationError {
    DeclarationError(message.into())
}

fn invalid() -> EffectFailure {
    EffectFailure::InterpreterFault(InterpreterFault::InvalidInput)
}

pub(crate) fn declared_adapters() -> &'static [DeclaredAdapter] {
    static DECLARED: OnceLock<Vec<DeclaredAdapter>> = OnceLock::new();
    DECLARED.get_or_init(|| {
        let adapters = DECLARATIONS
            .iter()
            .map(|(file, source)| {
                DeclaredAdapter::parse(source)
                    .unwrap_or_else(|error| panic!("harness declaration {file}: {error}"))
            })
            .collect::<Vec<_>>();
        for (index, adapter) in adapters.iter().enumerate() {
            assert!(
                adapters[..index]
                    .iter()
                    .all(|earlier| earlier.name != adapter.name),
                "harness declaration name {:?} is declared twice",
                adapter.name
            );
        }
        adapters
    })
}

pub(crate) fn host_secrets() -> &'static [DeclaredPath] {
    static SECRETS: OnceLock<Vec<DeclaredPath>> = OnceLock::new();
    SECRETS.get_or_init(|| {
        parse_host(HOST).unwrap_or_else(|error| panic!("harness host declaration: {error}"))
    })
}

fn parse_host(source: &str) -> Result<Vec<DeclaredPath>, DeclarationError> {
    let document = source
        .parse::<DocumentMut>()
        .map_err(|error| DeclarationError(format!("not TOML: {error}")))?;
    let root = Section::of(document.as_item(), "host", &["secrets"])?;
    root.required("secrets")?;
    paths(&root, "secrets")
}

fn paths(section: &Section<'_>, key: &str) -> Result<Vec<DeclaredPath>, DeclarationError> {
    section
        .strings(key)?
        .iter()
        .map(|text| {
            DeclaredPath::parse(text).ok_or_else(|| {
                error(format!(
                    "{}.{key} entry {text:?} is neither \"~/…\" nor absolute, or climbs with ..",
                    section.at
                ))
            })
        })
        .collect()
}

fn boundary(section: &Section<'_>) -> Result<BoundaryDeclaration, DeclarationError> {
    let egress = section
        .strings("egress")?
        .iter()
        .map(|text| {
            EgressEntry::parse(text).ok_or_else(|| {
                error(format!(
                    "{}.egress entry {text:?} is not host, host:port or host:*",
                    section.at
                ))
            })
        })
        .collect::<Result<_, _>>()?;
    Ok(BoundaryDeclaration {
        egress,
        write: paths(section, "write")?,
        write_denied: paths(section, "write_denied")?,
        login: paths(section, "login")?,
    })
}

fn env_set(env: &Section<'_>) -> Result<Vec<(String, EnvValue)>, DeclarationError> {
    let Some(item) = env.get("set") else {
        return Ok(Vec::new());
    };
    let table = item
        .as_table_like()
        .ok_or_else(|| error(format!("{}.set must be a table", env.at)))?;
    table
        .iter()
        .map(|(name, value)| {
            let at = format!("{}.set.{name}", env.at);
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Err(error(format!("{at} is not an environment name")));
            }
            if executor_owns_environment(name) {
                return Err(error(format!("{at} is owned by the executor")));
            }
            let value = value
                .as_str()
                .ok_or_else(|| error(format!("{at} must be a string")))?;
            let value = if value == "{tmp}" {
                EnvValue::NodeTemp
            } else {
                EnvValue::Literal(value.to_owned())
            };
            Ok((name.to_owned(), value))
        })
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Arg {
    Literal(String),
    Session,
    Prompt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PromptPlacement {
    Stdin,
    Argument,
}

#[derive(Clone, Debug, PartialEq)]
struct LineRule {
    when: Json,
    field: Vec<String>,
}

impl LineRule {
    fn matches(&self, line: &Json) -> bool {
        fn holds(pattern: &Json, value: &Json) -> bool {
            match pattern {
                Json::Object(expected) => expected.iter().all(|(key, pattern)| {
                    value.get(key).is_some_and(|value| holds(pattern, value))
                }),
                scalar => scalar == value,
            }
        }
        holds(&self.when, line)
    }

    fn read_text(&self, line: &Json) -> Option<String> {
        self.field
            .iter()
            .try_fold(line, |value, key| value.get(key))
            .and_then(Json::as_str)
            .map(str::to_owned)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Output {
    Jsonl {
        session: Option<LineRule>,
        answer: LineRule,
        failure: Option<LineRule>,
    },
    Text { session: Option<String> },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DeclaredAdapter {
    name: String,
    new: Vec<Arg>,
    resume: Vec<Arg>,
    prompt: PromptPlacement,
    env_remove: Vec<String>,
    env_set: Vec<(String, EnvValue)>,
    output: Output,
    boundary: BoundaryDeclaration,
    candidates: Vec<DeclaredPath>,
}

pub(crate) struct Section<'a> {
    at: String,
    table: &'a dyn TableLike,
}

impl<'a> Section<'a> {
    pub(crate) fn of(
        item: &'a Item,
        at: impl Into<String>,
        keys: &[&str],
    ) -> Result<Self, DeclarationError> {
        let at = at.into();
        let table = item
            .as_table_like()
            .ok_or_else(|| error(format!("{at} must be a table")))?;
        if let Some((unknown, _)) = table.iter().find(|(key, _)| !keys.contains(key)) {
            return Err(error(format!("{at}.{unknown} is not a declaration key")));
        }
        Ok(Self { at, table })
    }

    pub(crate) fn get(&self, key: &str) -> Option<&'a Item> {
        self.table.get(key)
    }

    fn required(&self, key: &str) -> Result<&'a Item, DeclarationError> {
        self.get(key)
            .ok_or_else(|| error(format!("{}.{key} is required", self.at)))
    }

    pub(crate) fn string(&self, key: &str) -> Result<String, DeclarationError> {
        self.required(key)?
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| error(format!("{}.{key} must be a string", self.at)))
    }

    fn strings(&self, key: &str) -> Result<Vec<String>, DeclarationError> {
        let Some(item) = self.get(key) else {
            return Ok(Vec::new());
        };
        let array = item
            .as_array()
            .ok_or_else(|| error(format!("{}.{key} must be an array", self.at)))?;
        array
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| error(format!("{}.{key} holds only strings", self.at)))
            })
            .collect()
    }

    fn section(&self, key: &str, keys: &[&str]) -> Result<Option<Section<'a>>, DeclarationError> {
        self.get(key)
            .map(|item| Section::of(item, format!("{}.{key}", self.at), keys))
            .transpose()
    }
}

fn pattern(item: &Item, at: &str) -> Result<Json, DeclarationError> {
    if let Some(table) = item.as_table_like() {
        return table
            .iter()
            .map(|(key, item)| Ok((key.to_owned(), pattern(item, &format!("{at}.{key}"))?)))
            .collect::<Result<serde_json::Map<_, _>, _>>()
            .map(Json::Object);
    }
    match item.as_value() {
        Some(toml_edit::Value::String(text)) => Ok(Json::String(text.value().clone())),
        Some(toml_edit::Value::Boolean(flag)) => Ok(Json::Bool(*flag.value())),
        Some(toml_edit::Value::Integer(number)) => Ok(Json::from(*number.value())),
        _ => Err(error(format!(
            "{at} must be a table, string, boolean or integer"
        ))),
    }
}

fn line_rule(item: &Item, at: &str) -> Result<LineRule, DeclarationError> {
    let rule = Section::of(item, at, &["when", "field"])?;
    let when = pattern(rule.required("when")?, &format!("{at}.when"))?;
    if !when.is_object() {
        return Err(error(format!("{at}.when must be a table")));
    }
    let field = rule.string("field")?;
    let field = field.split('.').map(str::to_owned).collect::<Vec<_>>();
    if field.iter().any(String::is_empty) {
        return Err(error(format!("{at}.field has an empty path segment")));
    }
    Ok(LineRule { when, field })
}

fn template(section: &Section<'_>, key: &str) -> Result<Vec<Arg>, DeclarationError> {
    section.required(key)?;
    Ok(section
        .strings(key)?
        .into_iter()
        .map(|arg| match arg.as_str() {
            "{session}" => Arg::Session,
            "{prompt}" => Arg::Prompt,
            _ => Arg::Literal(arg),
        })
        .collect())
}

impl DeclaredAdapter {
    pub(crate) fn parse(source: &str) -> Result<Self, DeclarationError> {
        let document = source
            .parse::<DocumentMut>()
            .map_err(|error| DeclarationError(format!("not TOML: {error}")))?;
        let root = Section {
            at: "declaration".to_owned(),
            table: document.as_table(),
        };
        if let Some((unknown, _)) = root.table.iter().find(|(key, _)| {
            !["name", "invoke", "env", "output", "boundary", "detect"].contains(key)
        }) {
            return Err(error(format!("{unknown} is not a declaration key")));
        }
        let name = root.string("name")?;
        if name.is_empty() {
            return Err(error("name is empty"));
        }

        let invoke = root
            .section("invoke", &["new", "resume", "prompt"])?
            .ok_or_else(|| error("invoke is required"))?;
        let new = template(&invoke, "new")?;
        let resume = template(&invoke, "resume")?;
        let prompt = match invoke.string("prompt")?.as_str() {
            "stdin" => PromptPlacement::Stdin,
            "argument" => PromptPlacement::Argument,
            other => {
                return Err(error(format!(
                    "invoke.prompt {other:?} is neither \"stdin\" nor \"argument\""
                )));
            }
        };
        let count = |args: &[Arg], wanted: &Arg| args.iter().filter(|arg| *arg == wanted).count();
        let prompts = usize::from(prompt == PromptPlacement::Argument);
        if count(&new, &Arg::Prompt) != prompts || count(&resume, &Arg::Prompt) != prompts {
            return Err(error(format!(
                "invoke.new and invoke.resume must each hold {{prompt}} exactly {prompts} time(s)"
            )));
        }
        if count(&new, &Arg::Session) != 0 {
            return Err(error("invoke.new cannot hold {session}"));
        }
        if count(&resume, &Arg::Session) > 1 {
            return Err(error("invoke.resume holds {session} at most once"));
        }

        let env = root.section("env", &["remove", "set"])?;
        let env_remove = env
            .as_ref()
            .map(|env| env.strings("remove"))
            .transpose()?
            .unwrap_or_default();
        let env_set = env.as_ref().map(env_set).transpose()?.unwrap_or_default();
        let boundary = boundary(
            &root
                .section("boundary", &["egress", "write", "write_denied", "login"])?
                .ok_or_else(|| error("boundary is required"))?,
        )?;

        let candidates = root
            .section("detect", &["candidates"])?
            .map(|detect| paths(&detect, "candidates"))
            .transpose()?
            .unwrap_or_default();

        let output = root
            .section("output", &["format", "session", "final", "failure"])?
            .ok_or_else(|| error("output is required"))?;
        let output = match output.string("format")?.as_str() {
            "jsonl" => Output::Jsonl {
                session: output
                    .get("session")
                    .map(|item| line_rule(item, "output.session"))
                    .transpose()?,
                answer: line_rule(output.required("final")?, "output.final")?,
                failure: output
                    .get("failure")
                    .map(|item| line_rule(item, "output.failure"))
                    .transpose()?,
            },
            "text" => {
                if output.get("final").is_some() || output.get("failure").is_some() {
                    return Err(error(
                        "output.final and output.failure need a structured format",
                    ));
                }
                Output::Text {
                    session: output
                        .section("session", &["constant"])?
                        .map(|session| session.string("constant"))
                        .transpose()?,
                }
            }
            other => {
                return Err(error(format!(
                    "output.format {other:?} is neither \"jsonl\" nor \"text\""
                )));
            }
        };

        Ok(Self {
            name,
            new,
            resume,
            prompt,
            env_remove,
            env_set,
            output,
            boundary,
            candidates,
        })
    }

    pub(crate) fn candidates(&self, home: &Path) -> Vec<PathBuf> {
        self.candidates
            .iter()
            .map(|candidate| candidate.resolve(home))
            .collect()
    }

    fn lines(stdout: &[u8]) -> Result<impl Iterator<Item = Json> + '_, EffectFailure> {
        let text = std::str::from_utf8(stdout).map_err(|_| invalid())?;
        Ok(text
            .lines()
            .filter_map(|line| serde_json::from_str::<Json>(line).ok()))
    }
}

impl CliHarnessAdapter for DeclaredAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn invocation(&self, session: Option<&str>, prompt: &str) -> CliInvocation {
        let template = if session.is_some() {
            &self.resume
        } else {
            &self.new
        };
        let args = template
            .iter()
            .map(|arg| match arg {
                Arg::Literal(literal) => literal.clone(),
                Arg::Session => session.unwrap_or_default().to_owned(),
                Arg::Prompt => prompt.to_owned(),
            })
            .collect();
        CliInvocation {
            args,
            stdin: (self.prompt == PromptPlacement::Stdin).then(|| prompt.as_bytes().to_vec()),
        }
    }

    fn parse(&self, stdout: &[u8]) -> Result<CliTurn, EffectFailure> {
        match &self.output {
            Output::Text { session } => {
                let text = std::str::from_utf8(stdout).map_err(|_| invalid())?;
                Ok(CliTurn {
                    output: text.trim().to_owned(),
                    session: session.clone(),
                })
            }
            Output::Jsonl {
                session: session_rule,
                answer: answer_rule,
                failure,
            } => {
                let mut session = None;
                let mut answer = None;
                let mut failed = false;
                for line in Self::lines(stdout)? {
                    if failure.as_ref().is_some_and(|rule| rule.matches(&line)) {
                        failed = true;
                    }
                    if let Some(rule) = session_rule.as_ref().filter(|rule| rule.matches(&line)) {
                        session = rule.read_text(&line);
                    }
                    if answer_rule.matches(&line) {
                        answer = answer_rule.read_text(&line);
                    }
                }
                if failed {
                    return Err(EffectFailure::TransportTerminal);
                }
                Ok(CliTurn {
                    output: answer.ok_or_else(invalid)?,
                    session,
                })
            }
        }
    }

    fn failure_detail(&self, stdout: &[u8]) -> Option<String> {
        if let Output::Jsonl {
            failure: Some(rule),
            ..
        } = &self.output
        {
            let reported = Self::lines(stdout).ok().and_then(|lines| {
                lines
                    .filter(|line| rule.matches(line))
                    .filter_map(|line| rule.read_text(&line))
                    .filter(|detail| !detail.is_empty())
                    .last()
            });
            if let Some(reported) = reported {
                return Some(bounded_provider_detail(&reported));
            }
        }
        provider_output_detail(stdout)
    }

    fn env_remove(&self) -> &[String] {
        &self.env_remove
    }

    fn env_set(&self) -> &[(String, EnvValue)] {
        &self.env_set
    }

    fn boundary(&self) -> &BoundaryDeclaration {
        &self.boundary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(name: &str) -> &'static DeclaredAdapter {
        declared_adapters()
            .iter()
            .find(|adapter| adapter.name == name)
            .expect("boarded declaration")
    }

    #[test]
    fn every_boarded_declaration_parses_under_its_own_name() {
        let names = declared_adapters()
            .iter()
            .map(|adapter| adapter.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(names, ["claude", "codex", "pi"]);
    }

    #[test]
    fn an_unknown_key_is_refused_rather_than_ignored() {
        let source = "name = \"x\"\n[invoke]\nnew = [\"{prompt}\"]\nresume = [\"{prompt}\"]\nprompt = \"argument\"\nretries = 3\n[output]\nformat = \"text\"\n";
        assert_eq!(
            DeclaredAdapter::parse(source),
            Err(DeclarationError(
                "declaration.invoke.retries is not a declaration key".to_owned()
            ))
        );
        let source = "name = \"x\"\nsandbox = true\n[invoke]\nnew = []\nresume = []\nprompt = \"stdin\"\n[output]\nformat = \"text\"\n";
        assert_eq!(
            DeclaredAdapter::parse(source),
            Err(DeclarationError(
                "sandbox is not a declaration key".to_owned()
            ))
        );
    }

    #[test]
    fn a_prompt_argument_must_have_exactly_one_place() {
        let source = "name = \"x\"\n[invoke]\nnew = [\"-p\"]\nresume = [\"{prompt}\"]\nprompt = \"argument\"\n[output]\nformat = \"text\"\n";
        assert!(DeclaredAdapter::parse(source).is_err());
        let source = "name = \"x\"\n[invoke]\nnew = [\"{session}\"]\nresume = []\nprompt = \"stdin\"\n[output]\nformat = \"text\"\n";
        assert!(DeclaredAdapter::parse(source).is_err());
    }

    #[test]
    fn claude_turns_carry_the_user_settings_block_on_both_templates() {
        let claude = adapter("claude");
        let first = claude.invocation(None, "hello");
        assert_eq!(
            first.args,
            [
                "-p",
                "--output-format",
                "json",
                "--strict-mcp-config",
                "--permission-mode",
                "bypassPermissions",
                "--settings",
                "{\"disableAllHooks\":true,\"sandbox\":{\"enabled\":false}}",
            ]
        );
        assert_eq!(first.stdin.as_deref(), Some(b"hello".as_slice()));
        let resumed = claude.invocation(Some("abc-123"), "next");
        assert_eq!(
            resumed.args,
            [
                "-p",
                "--output-format",
                "json",
                "--strict-mcp-config",
                "--permission-mode",
                "bypassPermissions",
                "--settings",
                "{\"disableAllHooks\":true,\"sandbox\":{\"enabled\":false}}",
                "--resume",
                "abc-123",
            ]
        );
        assert_eq!(claude.env_remove(), ["ANTHROPIC_API_KEY"]);
        assert_eq!(
            claude.env_set(),
            [
                ("CLAUDE_CODE_TMPDIR".to_owned(), EnvValue::NodeTemp),
                (
                    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".to_owned(),
                    EnvValue::Literal("1".to_owned())
                ),
            ]
        );
    }

    #[test]
    fn the_host_closes_credential_stores_browser_profiles_and_the_default_state() {
        let secrets = host_secrets();
        for expected in [
            "~/.ssh",
            "~/.aws",
            "~/Library/Keychains",
            "~/Library/Cookies",
            "~/Library/Application Support/Circular",
        ] {
            assert!(
                secrets.contains(&DeclaredPath::parse(expected).unwrap()),
                "{expected}"
            );
        }
    }

    #[test]
    fn a_declaration_without_a_boundary_table_is_refused() {
        let source = "name = \"x\"\n[invoke]\nnew = []\nresume = []\nprompt = \"stdin\"\n[output]\nformat = \"text\"\n";
        assert_eq!(
            DeclaredAdapter::parse(source),
            Err(DeclarationError("boundary is required".to_owned()))
        );
        let bad_path = format!("{source}[boundary]\nwrite = [\"relative/x\"]\n");
        assert!(DeclaredAdapter::parse(&bad_path).is_err());
        let bad_host = format!("{source}[boundary]\negress = [\"https://x\"]\n");
        assert!(DeclaredAdapter::parse(&bad_host).is_err());
        let unknown = format!("{source}[boundary]\nread = []\n");
        assert_eq!(
            DeclaredAdapter::parse(&unknown),
            Err(DeclarationError(
                "declaration.boundary.read is not a declaration key".to_owned()
            ))
        );
        assert!(DeclaredAdapter::parse(&format!("{source}[boundary]\n")).is_ok());
    }

    #[test]
    fn a_declaration_cannot_set_an_executor_owned_environment_name() {
        for owned in ["HOME", "TMPDIR", "HTTPS_PROXY", "http_proxy", "PATH"] {
            let source = format!(
                "name = \"x\"\n[invoke]\nnew = []\nresume = []\nprompt = \"stdin\"\n[env]\nset = {{ {owned} = \"v\" }}\n[output]\nformat = \"text\"\n[boundary]\n"
            );
            assert_eq!(
                DeclaredAdapter::parse(&source),
                Err(DeclarationError(format!(
                    "declaration.env.set.{owned} is owned by the executor"
                ))),
            );
        }
    }

    #[test]
    fn claude_json_result_line_yields_answer_session_and_failure() {
        let claude = adapter("claude");
        let turn = claude
            .parse(b"{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"ok\",\"session_id\":\"821e\"}")
            .expect("a result line parses");
        assert_eq!(turn.output, "ok");
        assert_eq!(turn.session.as_deref(), Some("821e"));

        let failed = b"{\"type\":\"result\",\"is_error\":true,\"result\":\"Not logged in\",\"session_id\":\"098\"}";
        assert_eq!(
            claude.parse(failed).map(|turn| turn.output),
            Err(EffectFailure::TransportTerminal)
        );
        assert_eq!(
            claude.failure_detail(failed).as_deref(),
            Some("Not logged in")
        );
        assert_eq!(
            claude
                .parse(b"provider response was not json")
                .map(|turn| turn.output),
            Err(invalid())
        );
    }

    #[test]
    fn codex_jsonl_yields_the_last_agent_message_and_thread() {
        let stdout = concat!(
            "{\"type\":\"thread.started\",\"thread_id\":\"t-9\"}\n",
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"reasoning\",\"text\":\"…\"}}\n",
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"first\"}}\n",
            "not json noise\n",
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"final\"}}\n",
        );
        let turn = adapter("codex")
            .parse(stdout.as_bytes())
            .expect("pinned JSONL parses");
        assert_eq!(turn.session.as_deref(), Some("t-9"));
        assert_eq!(turn.output, "final");
    }

    #[test]
    fn a_failed_codex_turn_is_a_provider_failure_with_its_message() {
        let stdout = concat!(
            "{\"type\":\"thread.started\",\"thread_id\":\"t-1\"}\n",
            "{\"type\":\"turn.started\"}\n",
            "{\"type\":\"turn.failed\",\"error\":{\"message\":\"unexpected status 401 Unauthorized\"}}\n",
        );
        let codex = adapter("codex");
        assert_eq!(
            codex.parse(stdout.as_bytes()).map(|turn| turn.output),
            Err(EffectFailure::TransportTerminal)
        );
        assert_eq!(
            codex.failure_detail(stdout.as_bytes()).as_deref(),
            Some("unexpected status 401 Unauthorized")
        );
    }

    #[test]
    fn codex_invocations_skip_the_git_check_ignore_user_config_and_resume_by_thread() {
        let codex = adapter("codex");
        let first = codex.invocation(None, "triage this");
        assert_eq!(
            first.args,
            [
                "exec",
                "--json",
                "--skip-git-repo-check",
                "--ignore-user-config",
                "--dangerously-bypass-approvals-and-sandbox",
                "triage this"
            ]
        );
        assert_eq!(first.stdin, None);
        assert_eq!(
            codex.invocation(Some("t-9"), "next").args,
            [
                "exec",
                "resume",
                "--json",
                "--skip-git-repo-check",
                "--ignore-user-config",
                "--dangerously-bypass-approvals-and-sandbox",
                "t-9",
                "next"
            ]
        );
    }

    #[test]
    fn pi_stdout_is_the_final_text_and_continuation_is_folder_scoped() {
        let pi = adapter("pi");
        let turn = pi.parse(b"  the answer\n").expect("plain stdout parses");
        assert_eq!(turn.output, "the answer");
        assert_eq!(pi.invocation(None, "p").args, ["-p", "p"]);
        assert_eq!(
            pi.invocation(turn.session.as_deref(), "p").args,
            ["-p", "--continue", "p"],
            "an answered turn continues the node folder's session"
        );
    }
}
