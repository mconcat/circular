
use crate::{OWNER_ROOT_MODE, OwnerRootModeMismatch, validate_owner_root_mode};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};

/// One option in the canonical owner-local daemon argv contract.
///
/// [`DAEMON_OPTIONS`] is also the source for the public usage renderer, so a
/// parser spelling cannot drift away from the list shown to users.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DaemonOption {
    pub flag: &'static str,
    pub value: &'static str,
    pub meaning: &'static str,
    pub required: bool,
}

/// Every option [`DaemonArguments::parse`] recognises, in usage order.
pub const DAEMON_OPTIONS: &[DaemonOption] = &[
    DaemonOption {
        flag: "--state",
        value: "<abs dir>",
        meaning: "owner-local daemon state directory",
        required: true,
    },
    DaemonOption {
        flag: "--reference-agent",
        value: "",
        meaning: "enable the deterministic reference executor, not an actual provider (default: off)",
        required: false,
    },
];

pub const CANONICAL_STATE_OPTION: &str = DAEMON_OPTIONS[0].flag;

/// Tokens that request usage instead of performing an owner-local action.
pub const OWNER_LOCAL_HELP_OPTIONS: [&str; 2] = ["--help", "-h"];

/// The token that requests build identity instead of performing an action.
pub const OWNER_LOCAL_VERSION_OPTION: &str = "--version";

/// Existing package-version spelling, retained as an alias of the build identity's version.
pub const CIRCULAR_VERSION: &str = crate::build_identity::PACKAGE_VERSION;

/// The public-information request, or the unchanged arguments for execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OwnerLocalInvocation {
    Help,
    Version,
    Arguments(Vec<OsString>),
}

impl OwnerLocalInvocation {
    /// Classifies help/version before any platform or filesystem work.
    ///
    /// Help has precedence when both information tokens occur, matching the
    /// `circular-ui` command-line surface. Otherwise the exact tokens are
    /// returned for the existing parser; this layer adds no action argument.
    pub fn parse<I, S>(arguments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let arguments = arguments
            .into_iter()
            .map(Into::into)
            .collect::<Vec<OsString>>();
        if arguments.iter().any(|argument| {
            OWNER_LOCAL_HELP_OPTIONS
                .iter()
                .any(|help| argument.as_encoded_bytes() == help.as_bytes())
        }) {
            return Self::Help;
        }
        if arguments
            .iter()
            .any(|argument| argument.as_encoded_bytes() == OWNER_LOCAL_VERSION_OPTION.as_bytes())
        {
            return Self::Version;
        }
        Self::Arguments(arguments)
    }

    #[must_use]
    pub fn from_env() -> Self {
        Self::parse(std::env::args_os().skip(1))
    }
}

/// Renders the common English help shape from the canonical option table.
///
/// `leading_arguments` accounts for a program's already-existing verb, while
/// every option spelling and value placeholder comes from [`DAEMON_OPTIONS`].
#[must_use]
pub fn owner_local_usage(
    program: &str,
    summary: &str,
    leading_arguments: &str,
    example_leading_arguments: &str,
) -> String {
    render_usage(
        program,
        summary,
        leading_arguments,
        example_leading_arguments,
        &DAEMON_OPTIONS[..1],
    )
}

/// Renders daemon help, including its explicit reference-executor opt-in.
#[must_use]
pub fn daemon_usage(program: &str, summary: &str) -> String {
    render_usage(program, summary, "", "", DAEMON_OPTIONS)
}

fn render_usage(
    program: &str,
    summary: &str,
    leading_arguments: &str,
    example_leading_arguments: &str,
    options: &[DaemonOption],
) -> String {
    fn push_arguments(text: &mut String, leading_arguments: &str, options: &[DaemonOption]) {
        if !leading_arguments.is_empty() {
            text.push(' ');
            text.push_str(leading_arguments);
        }
        for option in options.iter().filter(|option| option.required) {
            text.push(' ');
            text.push_str(option.flag);
            text.push(' ');
            text.push_str(option.value);
        }
    }

    let mut text = format!("{program} — {summary}\n\nUsage:\n  {program}");
    push_arguments(&mut text, leading_arguments, options);
    for option in options.iter().filter(|option| !option.required) {
        text.push_str(&format!(" [{}]", option.flag));
    }
    text.push_str(&format!(
        "\n  {program} --help\n  {program} {OWNER_LOCAL_VERSION_OPTION}\n\nOptions:\n"
    ));
    let width = options
        .iter()
        .map(|option| {
            option.flag.len() + usize::from(!option.value.is_empty()) + option.value.len()
        })
        .max()
        .unwrap_or(0)
        .max("-h, --help".len())
        .max(OWNER_LOCAL_VERSION_OPTION.len());
    for option in options {
        let head = if option.value.is_empty() {
            option.flag.to_owned()
        } else {
            format!("{} {}", option.flag, option.value)
        };
        let requirement = if option.required { " (required)" } else { "" };
        text.push_str(&format!(
            "  {head:<width$}  {}{requirement}\n",
            option.meaning
        ));
    }
    text.push_str(&format!(
        "  {:<width$}  print this usage on stdout and exit 0\n",
        "-h, --help"
    ));
    text.push_str(&format!(
        "  {OWNER_LOCAL_VERSION_OPTION:<width$}  print the version on stdout and exit 0\n\n\
         State directory rules:\n  \
         - an absolute path under your home directory, owned by you\n  \
         - an existing directory with exactly mode {OWNER_ROOT_MODE:04o} and no special bits\n\n\
         Example:\n  {program}"
    ));
    push_arguments(&mut text, example_leading_arguments, options);
    text.push('\n');
    text
}

/// Renders artifact identity followed by the four compatibility axes.
#[must_use]
pub fn owner_local_version(program: &'static str) -> String {
    format!(
        "{}\n{}",
        crate::build_identity(program),
        circular_core::compatibility::current()
    )
}

pub const OWNER_LOCAL_SOCKET_NAME: &str = "daemon.sock";

#[cfg(unix)]
#[must_use]
pub fn current_effective_user() -> u32 {
    nix::unistd::geteuid().as_raw()
}

const INLINE_STATE_PREFIX: &str = "--state=";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DaemonArguments {
    state_directory: PathBuf,
    reference_agent: bool,
}

impl DaemonArguments {
    pub fn parse<I, S>(arguments: I) -> Result<Self, DaemonArgumentsError>
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        let mut state_directory: Option<PathBuf> = None;
        let mut reference_agent = false;
        let mut arguments = arguments.into_iter().map(Into::into).enumerate();

        while let Some((position, token)) = arguments.next() {
            if token.as_encoded_bytes() == DAEMON_OPTIONS[1].flag.as_bytes() && !reference_agent {
                reference_agent = true;
                continue;
            }
            if token.as_encoded_bytes() == CANONICAL_STATE_OPTION.as_bytes() {
                if state_directory.is_some() {
                    return Err(DaemonArgumentsError::RepeatedStateOption { position });
                }
                let Some((_, value)) = arguments.next() else {
                    return Err(DaemonArgumentsError::MissingStateValue);
                };
                if value.as_encoded_bytes().starts_with(b"-") {
                    return Err(DaemonArgumentsError::StateValueLooksLikeOption {
                        token: value.clone(),
                    });
                }
                state_directory = Some(PathBuf::from(value));
                continue;
            }
            if token
                .as_encoded_bytes()
                .starts_with(INLINE_STATE_PREFIX.as_bytes())
            {
                return Err(DaemonArgumentsError::InlineStateValue { token });
            }
            return Err(DaemonArgumentsError::UnexpectedToken { position, token });
        }

        let state_directory = state_directory.ok_or(DaemonArgumentsError::MissingStateOption)?;
        if !state_directory.is_absolute() {
            return Err(DaemonArgumentsError::RelativeStateDirectory {
                path: state_directory,
            });
        }
        Ok(Self {
            state_directory,
            reference_agent,
        })
    }

    #[must_use]
    pub fn state_directory(&self) -> &Path {
        &self.state_directory
    }

    /// Whether argv explicitly enabled the deterministic reference executor.
    #[must_use]
    pub fn reference_agent(&self) -> bool {
        self.reference_agent
    }

    #[cfg(unix)]
    pub fn accept_state_directory(
        &self,
        home: &Path,
        effective_user: u32,
    ) -> Result<AcceptedStateDirectory, StateDirectoryRejection> {
        use std::os::unix::fs::MetadataExt;

        let canonical_home =
            std::fs::canonicalize(home).map_err(|source| StateDirectoryRejection::Unreadable {
                path: home.to_path_buf(),
                source: source.to_string(),
            })?;
        let canonical = std::fs::canonicalize(&self.state_directory).map_err(|source| {
            StateDirectoryRejection::Unreadable {
                path: self.state_directory.clone(),
                source: source.to_string(),
            }
        })?;
        if !canonical.starts_with(&canonical_home) || canonical == canonical_home {
            return Err(StateDirectoryRejection::OutsideHome {
                path: canonical,
                home: canonical_home,
            });
        }

        let metadata = std::fs::symlink_metadata(&canonical).map_err(|source| {
            StateDirectoryRejection::Unreadable {
                path: canonical.clone(),
                source: source.to_string(),
            }
        })?;
        if !metadata.is_dir() {
            return Err(StateDirectoryRejection::NotADirectory { path: canonical });
        }
        if metadata.uid() != effective_user {
            return Err(StateDirectoryRejection::ForeignOwner {
                path: canonical,
                owner: metadata.uid(),
                expected: effective_user,
            });
        }
        validate_owner_root_mode(metadata.mode()).map_err(|source| {
            StateDirectoryRejection::NotOwnerOnly {
                path: canonical.clone(),
                source,
            }
        })?;
        Ok(AcceptedStateDirectory { path: canonical })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedStateDirectory {
    path: PathBuf,
}

impl AcceptedStateDirectory {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DaemonArgumentsError {
    MissingStateOption,
    MissingStateValue,
    RepeatedStateOption { position: usize },
    InlineStateValue { token: OsString },
    StateValueLooksLikeOption { token: OsString },
    UnexpectedToken { position: usize, token: OsString },
    RelativeStateDirectory { path: PathBuf },
}

impl fmt::Display for DaemonArgumentsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingStateOption => write!(
                formatter,
                "daemon argv must carry exactly one {CANONICAL_STATE_OPTION} option"
            ),
            Self::MissingStateValue => write!(
                formatter,
                "{CANONICAL_STATE_OPTION} must be followed by a state directory"
            ),
            Self::RepeatedStateOption { position } => write!(
                formatter,
                "{CANONICAL_STATE_OPTION} appears more than once (argument {position})"
            ),
            Self::InlineStateValue { token } => write!(
                formatter,
                "inline option values are not part of the daemon contract: {}",
                Displayed(token)
            ),
            Self::StateValueLooksLikeOption { token } => write!(
                formatter,
                "{CANONICAL_STATE_OPTION} value must not begin with '-': {}",
                Displayed(token)
            ),
            Self::UnexpectedToken { position, token } => write!(
                formatter,
                "unexpected daemon argument {position}: {}",
                Displayed(token)
            ),
            Self::RelativeStateDirectory { path } => {
                write!(formatter, "state directory must be absolute: {path:?}")
            }
        }
    }
}

impl std::error::Error for DaemonArgumentsError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateDirectoryRejection {
    Unreadable {
        path: PathBuf,
        source: String,
    },
    OutsideHome {
        path: PathBuf,
        home: PathBuf,
    },
    NotADirectory {
        path: PathBuf,
    },
    ForeignOwner {
        path: PathBuf,
        owner: u32,
        expected: u32,
    },
    NotOwnerOnly {
        path: PathBuf,
        source: OwnerRootModeMismatch,
    },
}

impl fmt::Display for StateDirectoryRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable { path, source } => {
                write!(
                    formatter,
                    "state directory {path:?} is unreadable: {source}"
                )
            }
            Self::OutsideHome { path, home } => write!(
                formatter,
                "state directory {path:?} is not under this user's home {home:?}"
            ),
            Self::NotADirectory { path } => {
                write!(formatter, "state location {path:?} is not a directory")
            }
            Self::ForeignOwner {
                path,
                owner,
                expected,
            } => write!(
                formatter,
                "state directory {path:?} is owned by uid {owner}, not {expected}"
            ),
            Self::NotOwnerOnly { path, source } => write!(formatter, "{path:?}: {source}"),
        }
    }
}

impl std::error::Error for StateDirectoryRejection {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NotOwnerOnly { source, .. } => Some(source),
            _ => None,
        }
    }
}

struct Displayed<'a>(&'a OsStr);

impl fmt::Display for Displayed<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_information_is_classified_before_action_arguments() {
        for help in OWNER_LOCAL_HELP_OPTIONS {
            assert_eq!(
                OwnerLocalInvocation::parse([help]),
                OwnerLocalInvocation::Help
            );
            assert_eq!(
                OwnerLocalInvocation::parse(["--state", "/a", help]),
                OwnerLocalInvocation::Help,
                "help is terminal wherever it occurs, as it is for circular-ui"
            );
        }
        assert_eq!(
            OwnerLocalInvocation::parse(["--state", "/a", OWNER_LOCAL_VERSION_OPTION]),
            OwnerLocalInvocation::Version
        );
        assert_eq!(
            OwnerLocalInvocation::parse(["--state", "/a"]),
            OwnerLocalInvocation::Arguments(vec![OsString::from("--state"), OsString::from("/a")])
        );
    }

    #[test]
    fn usage_and_parser_read_the_same_option_table() {
        assert_eq!(CANONICAL_STATE_OPTION, DAEMON_OPTIONS[0].flag);
        let usage = owner_local_usage(
            "circular-agent",
            "Register or unregister Circular's per-user LaunchAgents",
            "<register|unregister>",
            "register",
        );
        for option in &DAEMON_OPTIONS[..1] {
            assert!(
                usage.contains(&format!("{} {}", option.flag, option.value)),
                "usage omitted canonical option {option:?}"
            );
        }
        assert!(usage.contains("circular-agent <register|unregister> --state <abs dir>"));
        assert!(!usage.contains("--reference-agent"));
        let daemon = daemon_usage("circular-daemon", "Circular owner-local engine");
        assert!(daemon.contains("--state <abs dir> [--reference-agent]"));
        for option in DAEMON_OPTIONS {
            assert!(daemon.contains(option.flag));
            assert!(daemon.contains(option.meaning));
        }
        assert!(usage.contains("-h, --help"));
        assert!(usage.contains(OWNER_LOCAL_VERSION_OPTION));
        assert_eq!(
            owner_local_version("circular-daemon"),
            format!(
                "{}\n{}",
                crate::build_identity("circular-daemon"),
                circular_core::compatibility::current()
            )
        );
    }

    #[test]
    fn reference_agent_is_an_explicit_valueless_flag() {
        for arguments in [
            vec!["--reference-agent", "--state", "/a"],
            vec!["--state", "/a", "--reference-agent"],
        ] {
            assert!(DaemonArguments::parse(arguments).unwrap().reference_agent());
        }
        for arguments in [
            vec!["--state", "/a", "--reference-agent", "true"],
            vec!["--state", "/a", "--reference-agent", "false"],
            vec!["--state", "/a", "--reference-agent=true"],
            vec!["--state", "/a", "--reference-agent", "--reference-agent"],
        ] {
            assert!(matches!(
                DaemonArguments::parse(arguments),
                Err(DaemonArgumentsError::UnexpectedToken { .. })
            ));
        }
        assert_eq!(
            DaemonArguments::parse(["--reference-agent"]),
            Err(DaemonArgumentsError::MissingStateOption)
        );
    }

    #[test]
    fn every_broken_shape_is_a_named_rejection() {
        let cases: [(Vec<&str>, DaemonArgumentsError); 6] = [
            (vec![], DaemonArgumentsError::MissingStateOption),
            (vec!["--state"], DaemonArgumentsError::MissingStateValue),
            (
                vec!["--state", "/a", "--state", "/b"],
                DaemonArgumentsError::RepeatedStateOption { position: 2 },
            ),
            (
                vec!["--state=/a"],
                DaemonArgumentsError::InlineStateValue {
                    token: OsString::from("--state=/a"),
                },
            ),
            (
                vec!["--state", "-x"],
                DaemonArgumentsError::StateValueLooksLikeOption {
                    token: OsString::from("-x"),
                },
            ),
            (
                vec!["--state", "relative/state"],
                DaemonArgumentsError::RelativeStateDirectory {
                    path: PathBuf::from("relative/state"),
                },
            ),
        ];

        for (arguments, expected) in cases {
            assert_eq!(
                DaemonArguments::parse(arguments.clone()),
                Err(expected),
                "{arguments:?}"
            );
        }
    }

    #[test]
    fn an_unknown_token_names_its_position_and_stops_there() {
        assert_eq!(
            DaemonArguments::parse(["--verbose", "--state", "/a"]),
            Err(DaemonArgumentsError::UnexpectedToken {
                position: 0,
                token: OsString::from("--verbose"),
            })
        );
        assert_eq!(
            DaemonArguments::parse(["--state", "/a", "extra"]),
            Err(DaemonArgumentsError::UnexpectedToken {
                position: 2,
                token: OsString::from("extra"),
            })
        );
    }

    #[cfg(unix)]
    mod acceptance {
        use super::*;
        use std::fs::{DirBuilder, Permissions};
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        use std::sync::atomic::{AtomicU32, Ordering};

        static NEXT: AtomicU32 = AtomicU32::new(0);

        struct Home(PathBuf);

        impl Home {
            fn new() -> Option<Self> {
                if current_effective_user() == 0 {
                    return None;
                }
                let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
                let home = std::env::temp_dir().join(format!(
                    "circular-daemon-argv-{}-{sequence}",
                    std::process::id()
                ));
                let mut builder = DirBuilder::new();
                builder.mode(0o700);
                builder.create(&home).ok()?;
                Some(Self(home))
            }

            fn child(&self, name: &str, mode: u32) -> PathBuf {
                let path = self.0.join(name);
                let mut builder = DirBuilder::new();
                builder.mode(mode);
                builder.create(&path).expect("child directory");
                path
            }
        }

        impl Drop for Home {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }

        fn arguments(path: &Path) -> DaemonArguments {
            DaemonArguments::parse([OsString::from("--state"), path.as_os_str().to_owned()])
                .expect("absolute path")
        }

        #[test]
        fn an_owner_only_directory_under_this_home_is_accepted() {
            let Some(home) = Home::new() else { return };
            let uid = current_effective_user();
            let state = home.child("state", 0o700);

            let accepted = arguments(&state)
                .accept_state_directory(&home.0, uid)
                .expect("owner-only directory under home");
            assert_eq!(
                accepted.path(),
                std::fs::canonicalize(&state).expect("canonical").as_path()
            );
        }

        #[test]
        fn a_directory_outside_this_home_is_refused() {
            let Some(home) = Home::new() else { return };
            let Some(other) = Home::new() else { return };
            let uid = current_effective_user();
            let state = other.child("state", 0o700);

            assert!(matches!(
                arguments(&state).accept_state_directory(&home.0, uid),
                Err(StateDirectoryRejection::OutsideHome { .. })
            ));
        }

        #[test]
        fn the_home_itself_is_not_a_state_directory() {
            let Some(home) = Home::new() else { return };
            let uid = current_effective_user();

            assert!(matches!(
                arguments(&home.0).accept_state_directory(&home.0, uid),
                Err(StateDirectoryRejection::OutsideHome { .. })
            ));
        }

        #[test]
        fn non_owner_special_or_missing_owner_bits_refuse_the_directory() {
            let Some(home) = Home::new() else { return };
            let uid = current_effective_user();
            let state = home.child("state", 0o700);

            for mode in [0o750_u32, 0o705, 0o777, 0o701, 0o1700, 0o600] {
                std::fs::set_permissions(&state, Permissions::from_mode(mode))
                    .expect("mode change");
                assert!(
                    matches!(
                        arguments(&state).accept_state_directory(&home.0, uid),
                        Err(StateDirectoryRejection::NotOwnerOnly {
                            source: OwnerRootModeMismatch { actual },
                            ..
                        }) if actual == mode
                    ),
                    "mode {mode:04o} must be refused"
                );
            }
            std::fs::set_permissions(&state, Permissions::from_mode(0o700)).expect("restore");
        }

        #[test]
        fn a_file_is_not_a_state_directory_and_a_missing_path_is_unreadable() {
            let Some(home) = Home::new() else { return };
            let uid = current_effective_user();

            let file = home.0.join("state-file");
            std::fs::write(&file, b"not a directory").expect("test file");
            assert!(matches!(
                arguments(&file).accept_state_directory(&home.0, uid),
                Err(StateDirectoryRejection::NotADirectory { .. })
            ));

            let missing = home.0.join("absent");
            assert!(matches!(
                arguments(&missing).accept_state_directory(&home.0, uid),
                Err(StateDirectoryRejection::Unreadable { .. })
            ));
        }
    }

    #[test]
    fn there_is_no_short_form_and_no_environment_fallback() {
        assert!(matches!(
            DaemonArguments::parse(["-s", "/a"]),
            Err(DaemonArgumentsError::UnexpectedToken { .. })
        ));
        assert_eq!(
            DaemonArguments::parse(Vec::<&str>::new()),
            Err(DaemonArgumentsError::MissingStateOption),
            "with no argument it fails without looking for a location in the environment"
        );
    }
}
