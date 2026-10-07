//! Current-user macOS LaunchAgent registration mechanism.
//!
//! Persistent registration (`register`/`unregister`) only changes a plist in
//! `~/Library/LaunchAgents`.  This library does not start, stop, or signal a
//! job, does not use `sudo`, and does not combine registration with start or
//! stop.

use super::location::{
    MACOS_NAME_MAX, MacOsLocationName, MacOsUserId, MacOsUserLocationResolver,
    ResolvedMacOsInstallLocation, ResolvedMacOsStateLocation,
};
use std::fmt;
use std::fs::{self, File, OpenOptions, Permissions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const PLIST_SUFFIX: &str = ".plist";
const TEMP_FILE_PREFIX: &str = ".circular-register";

static NEXT_TEMPORARY: AtomicU64 = AtomicU64::new(0);

/// A caller-owned reverse-DNS-style namespace used to derive state-keyed
/// service labels.  The release bundle identifier is intentionally injected;
/// this mechanism does not invent it.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LaunchAgentNamespace(Box<str>);

impl LaunchAgentNamespace {
    pub fn try_new(value: impl Into<Box<str>>) -> Result<Self, InvalidLaunchAgentNamespace> {
        let value = value.into().to_ascii_lowercase().into_boxed_str();
        if value.is_empty() {
            return Err(InvalidLaunchAgentNamespace::Empty);
        }
        if value.starts_with('.') || value.ends_with('.') || value.contains("..") {
            return Err(InvalidLaunchAgentNamespace::EmptySegment);
        }
        if let Some((index, byte)) = value
            .bytes()
            .enumerate()
            .find(|(_, byte)| !byte.is_ascii_alphanumeric() && !matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(InvalidLaunchAgentNamespace::InvalidByte { index, byte });
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvalidLaunchAgentNamespace {
    Empty,
    EmptySegment,
    InvalidByte { index: usize, byte: u8 },
}

impl fmt::Display for InvalidLaunchAgentNamespace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("launch-agent namespace cannot be empty"),
            Self::EmptySegment => {
                formatter.write_str("launch-agent namespace contains an empty segment")
            }
            Self::InvalidByte { index, byte } => write!(
                formatter,
                "launch-agent namespace byte {index} (0x{byte:02x}) is invalid"
            ),
        }
    }
}

impl std::error::Error for InvalidLaunchAgentNamespace {}

circular_core::closed_table! {
    /// Closed product process class carried by one per-state LaunchAgent.
    ///
    /// The service kind is not an arbitrary label fragment: it controls the
    /// collision-free launchd label, log files, and process scheduling class. This
    /// keeps the daemon headless.
    #[derive(Ord, PartialOrd)]
    pub enum LaunchAgentServiceKind {
        Daemon => "daemon",
    }
}

impl LaunchAgentServiceKind {
    const fn process_type(self) -> &'static str {
        match self {
            Self::Daemon => "Background",
        }
    }

    const fn stdout_file(self) -> &'static str {
        match self {
            Self::Daemon => "stdout.log",
        }
    }

    const fn stderr_file(self) -> &'static str {
        match self {
            Self::Daemon => "stderr.log",
        }
    }
}

/// A complete launchd service label derived from one state location.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LaunchAgentLabel(Box<str>);

impl LaunchAgentLabel {
    pub fn for_service_state(
        namespace: &LaunchAgentNamespace,
        service: LaunchAgentServiceKind,
        state: &MacOsLocationName,
    ) -> Result<Self, LaunchAgentLabelTooLong> {
        let value = format!(
            "{}.{}.{}",
            namespace.as_str(),
            service.as_str(),
            state.as_str()
        );
        let filename_bytes = value.len() + PLIST_SUFFIX.len();
        if filename_bytes > MACOS_NAME_MAX {
            return Err(LaunchAgentLabelTooLong {
                filename_bytes,
                maximum: MACOS_NAME_MAX,
            });
        }
        Ok(Self(value.into_boxed_str()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for LaunchAgentLabel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchAgentLabelTooLong {
    pub filename_bytes: usize,
    pub maximum: usize,
}

impl fmt::Display for LaunchAgentLabelTooLong {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "launch-agent plist name is {} bytes; the macOS maximum is {}",
            self.filename_bytes, self.maximum
        )
    }
}

impl std::error::Error for LaunchAgentLabelTooLong {}

/// An absolute, lexically normalized, UTF-8 executable path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchAgentProgram(PathBuf);

impl LaunchAgentProgram {
    pub fn try_new(path: impl Into<PathBuf>) -> Result<Self, InvalidProgramPath> {
        let path = path.into();
        validate_absolute_normal_path(&path)?;
        let Some(text) = path.to_str() else {
            return Err(InvalidProgramPath::NotUtf8 { path });
        };
        validate_xml_text(text).map_err(|character| InvalidProgramPath::InvalidXmlCharacter {
            path: path.clone(),
            character,
        })?;
        Ok(Self(path))
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }

    /// The validated UTF-8 form suitable for an exact argv carrier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0
            .to_str()
            .expect("LaunchAgentProgram construction validates UTF-8")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvalidProgramPath {
    NotAbsolute { path: PathBuf },
    NonNormalComponent { path: PathBuf },
    NotUtf8 { path: PathBuf },
    InvalidXmlCharacter { path: PathBuf, character: char },
}

impl fmt::Display for InvalidProgramPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAbsolute { path } => {
                write!(
                    formatter,
                    "product program is not absolute: {}",
                    path.display()
                )
            }
            Self::NonNormalComponent { path } => write!(
                formatter,
                "product program has a non-normal component: {}",
                path.display()
            ),
            Self::NotUtf8 { path } => {
                write!(
                    formatter,
                    "product program is not UTF-8: {}",
                    path.display()
                )
            }
            Self::InvalidXmlCharacter { path, character } => write!(
                formatter,
                "product program {} contains XML-invalid U+{:04X}",
                path.display(),
                *character as u32
            ),
        }
    }
}

impl std::error::Error for InvalidProgramPath {}

/// The injected canonical option name that introduces a state-location path.
///
/// The option spelling remains owned by the eventual daemon CLI contract.  A
/// launch-agent invocation cannot be constructed without this typed field, and
/// artifact rendering pairs it with the resolver-derived state directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchAgentStateOption(Box<str>);

impl LaunchAgentStateOption {
    /// The one spelling the product daemon parses.
    ///
    /// Rendering and parsing must not drift, so both sides read the same
    /// constant.  A registration built with any other spelling still renders,
    /// because this type stays a general mechanism, but it will not start the
    /// product daemon — the round-trip regression in this module pins the
    /// canonical pair.
    #[must_use]
    pub fn canonical() -> Self {
        Self::try_new(circular_transport::CANONICAL_STATE_OPTION)
            .expect("the canonical option is a valid long option")
    }

    pub fn try_new(value: impl Into<Box<str>>) -> Result<Self, InvalidLaunchAgentStateOption> {
        let value = value.into();
        if !value.starts_with("--") || value.len() == 2 || value.contains('=') {
            return Err(InvalidLaunchAgentStateOption::InvalidShape);
        }
        if !value[2..]
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(InvalidLaunchAgentStateOption::InvalidShape);
        }
        validate_xml_text(&value).map_err(|character| {
            InvalidLaunchAgentStateOption::InvalidXmlCharacter { character }
        })?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvalidLaunchAgentStateOption {
    InvalidShape,
    InvalidXmlCharacter { character: char },
}

impl fmt::Display for InvalidLaunchAgentStateOption {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidShape => formatter
                .write_str("state option must be a lowercase long option without an inline value"),
            Self::InvalidXmlCharacter { character } => write!(
                formatter,
                "state option contains XML-invalid U+{:04X}",
                *character as u32
            ),
        }
    }
}

impl std::error::Error for InvalidLaunchAgentStateOption {}

/// The exact product-service invocation supplied by the Shell start-argument
/// owner.
///
/// Additional arguments are opaque here: their owners serialize them, and this
/// layer preserves them byte-for-byte as UTF-8 strings.  The product
/// registration passes none; the daemon reads only `--state` and the optional
/// `--reference-agent` flag (`circular_transport::DaemonArguments`).  State
/// selection is not opaque: it is a required structured field and is rendered
/// before those arguments.  This layer neither discovers nor defaults any
/// argument.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LaunchAgentInvocation {
    program: LaunchAgentProgram,
    state_location: ResolvedMacOsStateLocation,
    state_option: LaunchAgentStateOption,
    additional_arguments: Box<[Box<str>]>,
}

impl LaunchAgentInvocation {
    pub fn try_new<I, S>(
        program: LaunchAgentProgram,
        state_location: ResolvedMacOsStateLocation,
        state_option: LaunchAgentStateOption,
        additional_arguments: I,
    ) -> Result<Self, InvalidLaunchAgentArgument>
    where
        I: IntoIterator<Item = S>,
        S: Into<Box<str>>,
    {
        let additional_arguments = additional_arguments
            .into_iter()
            .map(|argument| {
                let argument = argument.into();
                validate_xml_text(&argument).map_err(|character| {
                    InvalidLaunchAgentArgument::InvalidXmlCharacter { character }
                })?;
                Ok(argument)
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_boxed_slice();
        Ok(Self {
            program,
            state_location,
            state_option,
            additional_arguments,
        })
    }

    #[must_use]
    pub const fn program(&self) -> &LaunchAgentProgram {
        &self.program
    }

    #[must_use]
    pub const fn state_location(&self) -> &ResolvedMacOsStateLocation {
        &self.state_location
    }

    #[must_use]
    pub const fn state_option(&self) -> &LaunchAgentStateOption {
        &self.state_option
    }

    pub fn additional_arguments(&self) -> impl ExactSizeIterator<Item = &str> {
        self.additional_arguments.iter().map(AsRef::as_ref)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvalidLaunchAgentArgument {
    InvalidXmlCharacter { character: char },
}

impl fmt::Display for InvalidLaunchAgentArgument {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidXmlCharacter { character } => write!(
                formatter,
                "launch-agent argument contains XML-invalid U+{:04X}",
                *character as u32
            ),
        }
    }
}

impl std::error::Error for InvalidLaunchAgentArgument {}

/// A fully rendered, state-keyed persistent registration artifact.
///
/// Fields are private so callers cannot supply a plist path unrelated to its
/// state-derived label or bypass XML construction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserLaunchAgentArtifact {
    user: MacOsUserId,
    state_location: ResolvedMacOsStateLocation,
    install_location: ResolvedMacOsInstallLocation,
    label: LaunchAgentLabel,
    invocation: LaunchAgentInvocation,
    plist_path: PathBuf,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    plist: Box<[u8]>,
}

impl UserLaunchAgentArtifact {
    pub fn render_for_service(
        resolver: &MacOsUserLocationResolver,
        namespace: &LaunchAgentNamespace,
        service: LaunchAgentServiceKind,
        install_location: ResolvedMacOsInstallLocation,
        invocation: LaunchAgentInvocation,
    ) -> Result<Self, LaunchAgentArtifactError> {
        let state_location = invocation.state_location().clone();
        let expected = resolver.resolve_state(state_location.logical());
        if expected != state_location {
            return Err(LaunchAgentArtifactError::StateResolverMismatch);
        }
        let expected_install = resolver.resolve_install(install_location.logical());
        if expected_install != install_location {
            return Err(LaunchAgentArtifactError::InstallResolverMismatch);
        }
        if invocation
            .program()
            .as_path()
            .strip_prefix(install_location.directory())
            .is_err()
        {
            return Err(LaunchAgentArtifactError::ProgramOutsideInstall {
                program: invocation.program().as_path().to_path_buf(),
                install: install_location.directory().to_path_buf(),
            });
        }
        let label = LaunchAgentLabel::for_service_state(
            namespace,
            service,
            state_location.logical().identity(),
        )?;
        let plist_path = resolver
            .launch_agents_directory()
            .join(format!("{}{PLIST_SUFFIX}", label.as_str()));
        let stdout_path = state_location
            .output_directory()
            .join(service.stdout_file());
        let stderr_path = state_location
            .output_directory()
            .join(service.stderr_file());
        let plist = render_plist(&label, service, &invocation, &stdout_path, &stderr_path)?
            .into_bytes()
            .into_boxed_slice();
        Ok(Self {
            user: resolver.user(),
            state_location,
            install_location,
            label,
            invocation,
            plist_path,
            stdout_path,
            stderr_path,
            plist,
        })
    }

    #[must_use]
    pub const fn state_location(&self) -> &ResolvedMacOsStateLocation {
        &self.state_location
    }

    #[must_use]
    pub const fn install_location(&self) -> &ResolvedMacOsInstallLocation {
        &self.install_location
    }

    #[must_use]
    pub const fn label(&self) -> &LaunchAgentLabel {
        &self.label
    }

    #[must_use]
    pub const fn invocation(&self) -> &LaunchAgentInvocation {
        &self.invocation
    }

    #[must_use]
    pub fn plist_path(&self) -> &Path {
        &self.plist_path
    }

    #[must_use]
    pub fn stdout_path(&self) -> &Path {
        &self.stdout_path
    }

    #[must_use]
    pub fn stderr_path(&self) -> &Path {
        &self.stderr_path
    }

    #[must_use]
    pub fn plist(&self) -> &[u8] {
        &self.plist
    }
}

#[derive(Debug)]
pub enum LaunchAgentArtifactError {
    LabelTooLong(LaunchAgentLabelTooLong),
    StateResolverMismatch,
    InstallResolverMismatch,
    ProgramOutsideInstall { program: PathBuf, install: PathBuf },
    NonUtf8ResolvedPath { path: PathBuf },
    InvalidXmlPathCharacter { path: PathBuf, character: char },
}

impl From<LaunchAgentLabelTooLong> for LaunchAgentArtifactError {
    fn from(value: LaunchAgentLabelTooLong) -> Self {
        Self::LabelTooLong(value)
    }
}

impl fmt::Display for LaunchAgentArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LabelTooLong(error) => error.fmt(formatter),
            Self::StateResolverMismatch => formatter.write_str(
                "state location was resolved by a different macOS user-location provider",
            ),
            Self::InstallResolverMismatch => formatter.write_str(
                "install location was resolved by a different macOS user-location provider",
            ),
            Self::ProgramOutsideInstall { program, install } => write!(
                formatter,
                "product program {} is outside install location {}",
                program.display(),
                install.display()
            ),
            Self::NonUtf8ResolvedPath { path } => write!(
                formatter,
                "launch-agent plist path is not UTF-8: {}",
                path.display()
            ),
            Self::InvalidXmlPathCharacter { path, character } => write!(
                formatter,
                "launch-agent plist path {} contains XML-invalid U+{:04X}",
                path.display(),
                *character as u32
            ),
        }
    }
}

impl std::error::Error for LaunchAgentArtifactError {}

/// The durable-file result only.  It carries no runtime state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistentRegistrationChange {
    Created,
    Replaced,
    Unchanged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistentRegistrationRemoval {
    Removed,
    AlreadyAbsent,
}

/// Filesystem-only current-user registration writer.
#[derive(Debug)]
pub struct UserLaunchAgentRegistrar<'a> {
    resolver: &'a MacOsUserLocationResolver,
    install_location: &'a ResolvedMacOsInstallLocation,
}

impl<'a> UserLaunchAgentRegistrar<'a> {
    pub fn try_new(
        resolver: &'a MacOsUserLocationResolver,
        install_location: &'a ResolvedMacOsInstallLocation,
    ) -> Result<Self, LaunchAgentIoError> {
        let expected = resolver.resolve_install(install_location.logical());
        if expected != *install_location {
            return Err(LaunchAgentIoError::InstallResolverMismatch);
        }
        Ok(Self {
            resolver,
            install_location,
        })
    }

    /// Atomically places a mode-0600 plist in the same directory as its final
    /// destination.  No process is started or signalled.
    pub fn register(
        &self,
        artifact: &UserLaunchAgentArtifact,
    ) -> Result<PersistentRegistrationChange, LaunchAgentIoError> {
        self.validate_artifact_origin(artifact)?;
        validate_program_in_install(
            artifact.invocation.program(),
            self.install_location,
            self.resolver.user(),
        )?;
        self.resolver
            .prepare_output_destination(artifact.state_location())
            .map_err(LaunchAgentIoError::Location)?;
        let launch_agents = self
            .resolver
            .prepare_launch_agents_directory()
            .map_err(LaunchAgentIoError::Location)?;

        let previous = inspect_regular_file(artifact.plist_path(), self.resolver.user())?;
        if let Some(metadata) = &previous {
            let mut contents = Vec::new();
            File::open(artifact.plist_path())
                .and_then(|mut file| file.read_to_end(&mut contents))
                .map_err(|source| LaunchAgentIoError::Io {
                    operation: LaunchAgentIoOperation::Read,
                    path: artifact.plist_path().to_path_buf(),
                    source,
                })?;
            if contents == artifact.plist() && metadata.mode() & 0o777 == 0o600 {
                return Ok(PersistentRegistrationChange::Unchanged);
            }
        }

        let temporary = temporary_path(&launch_agents);
        let write_result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true).mode(0o600);
            let mut file = options
                .open(&temporary)
                .map_err(|source| LaunchAgentIoError::Io {
                    operation: LaunchAgentIoOperation::CreateTemporary,
                    path: temporary.clone(),
                    source,
                })?;
            file.write_all(artifact.plist())
                .map_err(|source| LaunchAgentIoError::Io {
                    operation: LaunchAgentIoOperation::WriteTemporary,
                    path: temporary.clone(),
                    source,
                })?;
            file.set_permissions(Permissions::from_mode(0o600))
                .map_err(|source| LaunchAgentIoError::Io {
                    operation: LaunchAgentIoOperation::SetPermissions,
                    path: temporary.clone(),
                    source,
                })?;
            file.sync_all().map_err(|source| LaunchAgentIoError::Io {
                operation: LaunchAgentIoOperation::SyncTemporary,
                path: temporary.clone(),
                source,
            })?;
            let metadata = file.metadata().map_err(|source| LaunchAgentIoError::Io {
                operation: LaunchAgentIoOperation::Inspect,
                path: temporary.clone(),
                source,
            })?;
            validate_owned_regular_metadata(
                &temporary,
                &metadata,
                self.resolver.user(),
                Some(0o600),
            )?;
            drop(file);
            validate_owned_regular_file(&temporary, self.resolver.user(), Some(0o600))?;
            fs::rename(&temporary, artifact.plist_path()).map_err(|source| {
                LaunchAgentIoError::Io {
                    operation: LaunchAgentIoOperation::AtomicReplace,
                    path: artifact.plist_path().to_path_buf(),
                    source,
                }
            })?;
            sync_directory(&launch_agents)?;
            validate_owned_regular_file(artifact.plist_path(), self.resolver.user(), Some(0o600))
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        write_result?;
        Ok(if previous.is_some() {
            PersistentRegistrationChange::Replaced
        } else {
            PersistentRegistrationChange::Created
        })
    }

    /// Validate an additional packaged executable against the same install
    /// custody rules as the LaunchAgent's primary program. Companion clients
    /// use this for explicit UI targets that are argv, not separate jobs.
    pub fn validate_program(&self, program: &LaunchAgentProgram) -> Result<(), LaunchAgentIoError> {
        validate_program_in_install(program, self.install_location, self.resolver.user())
    }

    /// Removes only the persistent plist.  It never deletes state/output or
    /// changes a currently loaded job.
    pub fn unregister(
        &self,
        artifact: &UserLaunchAgentArtifact,
    ) -> Result<PersistentRegistrationRemoval, LaunchAgentIoError> {
        self.validate_artifact_origin(artifact)?;
        let Some(_) = inspect_regular_file(artifact.plist_path(), self.resolver.user())? else {
            return Ok(PersistentRegistrationRemoval::AlreadyAbsent);
        };
        fs::remove_file(artifact.plist_path()).map_err(|source| LaunchAgentIoError::Io {
            operation: LaunchAgentIoOperation::Remove,
            path: artifact.plist_path().to_path_buf(),
            source,
        })?;
        sync_directory(&self.resolver.launch_agents_directory())?;
        Ok(PersistentRegistrationRemoval::Removed)
    }

    fn validate_artifact_origin(
        &self,
        artifact: &UserLaunchAgentArtifact,
    ) -> Result<(), LaunchAgentIoError> {
        let expected_state = self
            .resolver
            .resolve_state(artifact.state_location().logical());
        let expected_install = self
            .resolver
            .resolve_install(artifact.install_location().logical());
        let expected_path = self
            .resolver
            .launch_agents_directory()
            .join(format!("{}{PLIST_SUFFIX}", artifact.label().as_str()));
        if artifact.user != self.resolver.user()
            || expected_state != *artifact.state_location()
            || expected_install != *artifact.install_location()
            || *artifact.install_location() != *self.install_location
            || expected_path != artifact.plist_path()
        {
            Err(LaunchAgentIoError::ArtifactResolverMismatch)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchAgentIoOperation {
    CanonicalizeProgram,
    Inspect,
    Read,
    CreateTemporary,
    WriteTemporary,
    SetPermissions,
    SyncTemporary,
    AtomicReplace,
    Remove,
    SyncDirectory,
}

#[derive(Debug)]
pub enum LaunchAgentIoError {
    Location(super::location::LocationIoError),
    ArtifactResolverMismatch,
    InstallResolverMismatch,
    ProgramOutsideInstall {
        program: PathBuf,
        install: PathBuf,
    },
    ProgramNotCanonical {
        path: PathBuf,
        canonical: PathBuf,
    },
    ProgramSymbolicLink {
        path: PathBuf,
    },
    ProgramAncestorNotDirectory {
        path: PathBuf,
    },
    ProgramNotRegular {
        path: PathBuf,
    },
    ProgramNotOwnerExecutable {
        path: PathBuf,
        mode: u32,
    },
    UnsafeFileKind {
        path: PathBuf,
    },
    WrongOwner {
        path: PathBuf,
        expected: u32,
        actual: u32,
    },
    UnsafePermissions {
        path: PathBuf,
        mode: u32,
    },
    Io {
        operation: LaunchAgentIoOperation,
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for LaunchAgentIoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Location(error) => error.fmt(formatter),
            Self::ArtifactResolverMismatch => formatter
                .write_str("launch-agent artifact does not belong to this user-location resolver"),
            Self::InstallResolverMismatch => formatter
                .write_str("launch-agent registrar install does not belong to its resolver"),
            Self::ProgramOutsideInstall { program, install } => write!(
                formatter,
                "product program {} is outside registrar install {}",
                program.display(),
                install.display()
            ),
            Self::ProgramNotCanonical { path, canonical } => write!(
                formatter,
                "product program path {} is not canonical (canonical path is {})",
                path.display(),
                canonical.display()
            ),
            Self::ProgramSymbolicLink { path } => write!(
                formatter,
                "product program path contains a symbolic link: {}",
                path.display()
            ),
            Self::ProgramAncestorNotDirectory { path } => write!(
                formatter,
                "product program ancestor is not a directory: {}",
                path.display()
            ),
            Self::ProgramNotRegular { path } => {
                write!(
                    formatter,
                    "product program is not a regular file: {}",
                    path.display()
                )
            }
            Self::ProgramNotOwnerExecutable { path, mode } => write!(
                formatter,
                "product program {} is not owner-executable ({:03o})",
                path.display(),
                mode
            ),
            Self::UnsafeFileKind { path } => write!(
                formatter,
                "persistent registration is not a regular file: {}",
                path.display()
            ),
            Self::WrongOwner {
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "{} is owned by uid {actual}, expected uid {expected}",
                path.display()
            ),
            Self::UnsafePermissions { path, mode } => write!(
                formatter,
                "{} has unsafe permissions {:03o}",
                path.display(),
                mode
            ),
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "launch-agent {operation:?} failed for {}: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for LaunchAgentIoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Location(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn render_plist(
    label: &LaunchAgentLabel,
    service: LaunchAgentServiceKind,
    invocation: &LaunchAgentInvocation,
    stdout_path: &Path,
    stderr_path: &Path,
) -> Result<String, LaunchAgentArtifactError> {
    let program = invocation
        .program()
        .as_path()
        .to_str()
        .expect("program path was UTF-8 validated");
    let stdout = validated_xml_path(stdout_path)?;
    let stderr = validated_xml_path(stderr_path)?;
    let state_directory = validated_xml_path(invocation.state_location().directory())?;
    let mut plist = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n",
    );
    push_key_string(&mut plist, "Label", label.as_str());
    plist.push_str("  <key>ProgramArguments</key>\n  <array>\n");
    push_array_string(&mut plist, program);
    push_array_string(&mut plist, invocation.state_option().as_str());
    push_array_string(&mut plist, state_directory);
    for argument in invocation.additional_arguments() {
        push_array_string(&mut plist, argument);
    }
    plist.push_str("  </array>\n");
    plist.push_str("  <key>RunAtLoad</key>\n  <true/>\n");
    push_key_string(&mut plist, "StandardOutPath", stdout);
    push_key_string(&mut plist, "StandardErrorPath", stderr);
    push_key_string(&mut plist, "ProcessType", service.process_type());
    plist.push_str("</dict>\n</plist>\n");
    Ok(plist)
}

fn validated_xml_path(path: &Path) -> Result<&str, LaunchAgentArtifactError> {
    let value = path
        .to_str()
        .ok_or_else(|| LaunchAgentArtifactError::NonUtf8ResolvedPath {
            path: path.to_path_buf(),
        })?;
    validate_xml_text(value).map_err(|character| {
        LaunchAgentArtifactError::InvalidXmlPathCharacter {
            path: path.to_path_buf(),
            character,
        }
    })?;
    Ok(value)
}

fn push_key_string(output: &mut String, key: &str, value: &str) {
    output.push_str("  <key>");
    push_xml_escaped(output, key);
    output.push_str("</key>\n  <string>");
    push_xml_escaped(output, value);
    output.push_str("</string>\n");
}

fn push_array_string(output: &mut String, value: &str) {
    output.push_str("    <string>");
    push_xml_escaped(output, value);
    output.push_str("</string>\n");
}

fn push_xml_escaped(output: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '\"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            _ => output.push(character),
        }
    }
}

fn validate_xml_text(value: &str) -> Result<(), char> {
    value
        .chars()
        .find(|character| !is_xml_character(*character))
        .map_or(Ok(()), Err)
}

fn is_xml_character(character: char) -> bool {
    matches!(character as u32, 0x9 | 0xA | 0xD | 0x20..=0xD7FF | 0xE000..=0xFFFD | 0x10000..=0x10FFFF)
}

fn validate_absolute_normal_path(path: &Path) -> Result<(), InvalidProgramPath> {
    if !path.is_absolute() {
        return Err(InvalidProgramPath::NotAbsolute {
            path: path.to_path_buf(),
        });
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::RootDir | Component::Normal(_)))
    {
        return Err(InvalidProgramPath::NonNormalComponent {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn validate_program_in_install(
    program: &LaunchAgentProgram,
    install_location: &ResolvedMacOsInstallLocation,
    user: MacOsUserId,
) -> Result<(), LaunchAgentIoError> {
    let path = program.as_path();
    let install = install_location.directory();
    let relative =
        path.strip_prefix(install)
            .map_err(|_| LaunchAgentIoError::ProgramOutsideInstall {
                program: path.to_path_buf(),
                install: install.to_path_buf(),
            })?;
    if relative.as_os_str().is_empty() {
        return Err(LaunchAgentIoError::ProgramNotRegular {
            path: path.to_path_buf(),
        });
    }

    validate_program_component(install, user, false)?;
    let mut current = install.to_path_buf();
    let component_count = relative.components().count();
    for (index, component) in relative.components().enumerate() {
        let Component::Normal(component) = component else {
            return Err(LaunchAgentIoError::ProgramOutsideInstall {
                program: path.to_path_buf(),
                install: install.to_path_buf(),
            });
        };
        current.push(component);
        validate_program_component(&current, user, index + 1 == component_count)?;
    }

    for candidate in [install, path] {
        let canonical = fs::canonicalize(candidate).map_err(|source| LaunchAgentIoError::Io {
            operation: LaunchAgentIoOperation::CanonicalizeProgram,
            path: candidate.to_path_buf(),
            source,
        })?;
        if canonical != candidate {
            return Err(LaunchAgentIoError::ProgramNotCanonical {
                path: candidate.to_path_buf(),
                canonical,
            });
        }
    }
    Ok(())
}

fn validate_program_component(
    path: &Path,
    user: MacOsUserId,
    leaf: bool,
) -> Result<(), LaunchAgentIoError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| LaunchAgentIoError::Io {
        operation: LaunchAgentIoOperation::Inspect,
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(LaunchAgentIoError::ProgramSymbolicLink {
            path: path.to_path_buf(),
        });
    }
    validate_owner(path, &metadata, user)?;
    let mode = metadata.mode() & 0o777;
    if mode & 0o022 != 0 {
        return Err(LaunchAgentIoError::UnsafePermissions {
            path: path.to_path_buf(),
            mode,
        });
    }
    if leaf {
        if !metadata.is_file() {
            return Err(LaunchAgentIoError::ProgramNotRegular {
                path: path.to_path_buf(),
            });
        }
        if mode & 0o100 == 0 {
            return Err(LaunchAgentIoError::ProgramNotOwnerExecutable {
                path: path.to_path_buf(),
                mode,
            });
        }
    } else if !metadata.is_dir() {
        return Err(LaunchAgentIoError::ProgramAncestorNotDirectory {
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn inspect_regular_file(
    path: &Path,
    user: MacOsUserId,
) -> Result<Option<fs::Metadata>, LaunchAgentIoError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(LaunchAgentIoError::UnsafeFileKind {
                    path: path.to_path_buf(),
                });
            }
            validate_owner(path, &metadata, user)?;
            Ok(Some(metadata))
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(LaunchAgentIoError::Io {
            operation: LaunchAgentIoOperation::Inspect,
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn validate_owned_regular_file(
    path: &Path,
    user: MacOsUserId,
    exact_mode: Option<u32>,
) -> Result<(), LaunchAgentIoError> {
    let Some(metadata) = inspect_regular_file(path, user)? else {
        return Err(LaunchAgentIoError::Io {
            operation: LaunchAgentIoOperation::Inspect,
            path: path.to_path_buf(),
            source: io::Error::new(io::ErrorKind::NotFound, "expected registration file"),
        });
    };
    validate_owned_regular_metadata(path, &metadata, user, exact_mode)
}

fn validate_owned_regular_metadata(
    path: &Path,
    metadata: &fs::Metadata,
    user: MacOsUserId,
    exact_mode: Option<u32>,
) -> Result<(), LaunchAgentIoError> {
    if !metadata.is_file() {
        return Err(LaunchAgentIoError::UnsafeFileKind {
            path: path.to_path_buf(),
        });
    }
    validate_owner(path, metadata, user)?;
    let mode = metadata.mode() & 0o777;
    if mode & 0o022 != 0 || exact_mode.is_some_and(|expected| mode != expected) {
        return Err(LaunchAgentIoError::UnsafePermissions {
            path: path.to_path_buf(),
            mode,
        });
    }
    Ok(())
}

fn validate_owner(
    path: &Path,
    metadata: &fs::Metadata,
    user: MacOsUserId,
) -> Result<(), LaunchAgentIoError> {
    if metadata.uid() != user.get() {
        Err(LaunchAgentIoError::WrongOwner {
            path: path.to_path_buf(),
            expected: user.get(),
            actual: metadata.uid(),
        })
    } else {
        Ok(())
    }
}

fn temporary_path(directory: &Path) -> PathBuf {
    let sequence = NEXT_TEMPORARY.fetch_add(1, Ordering::Relaxed);
    directory.join(format!(
        "{TEMP_FILE_PREFIX}-{}-{sequence}.tmp",
        std::process::id()
    ))
}

fn sync_directory(directory: &Path) -> Result<(), LaunchAgentIoError> {
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|source| LaunchAgentIoError::Io {
            operation: LaunchAgentIoOperation::SyncDirectory,
            path: directory.to_path_buf(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StateLocationRef;
    use crate::shell::location::AppleSiliconMacOs;
    use std::fs::DirBuilder;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestEnvironment {
        home: PathBuf,
        program: PathBuf,
    }

    impl TestEnvironment {
        fn new() -> Option<Self> {
            MacOsUserId::current().ok()?;
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let home = std::env::temp_dir().join(format!(
                "circular-cli-launch-agent-{}-{sequence}",
                std::process::id()
            ));
            let mut builder = DirBuilder::new();
            builder.mode(0o700);
            builder.create(&home).expect("isolated test home");
            let resolver = MacOsUserLocationResolver::try_new(
                AppleSiliconMacOs::validate("macos", "aarch64").expect("supported platform"),
                &home,
            )
            .expect("test resolver");
            let install = resolver.resolve_install(&resolver.conventional_install());
            resolver.prepare_install(&install).expect("prepare install");
            let bin = install.directory().join("bin");
            let mut bin_builder = DirBuilder::new();
            bin_builder.mode(0o700);
            bin_builder.create(&bin).expect("private bin directory");
            let program = bin.join("circular-daemon");
            fs::write(&program, b"test executable").expect("test executable");
            fs::set_permissions(&program, Permissions::from_mode(0o700))
                .expect("executable permissions");
            Some(Self { home, program })
        }

        fn resolver(&self) -> MacOsUserLocationResolver {
            MacOsUserLocationResolver::try_new(
                AppleSiliconMacOs::validate("macos", "aarch64").expect("supported platform"),
                &self.home,
            )
            .expect("test resolver")
        }

        fn install(&self, resolver: &MacOsUserLocationResolver) -> ResolvedMacOsInstallLocation {
            resolver.resolve_install(&resolver.conventional_install())
        }

        fn artifact(
            &self,
            resolver: &MacOsUserLocationResolver,
            state_name: &str,
            argument: &str,
        ) -> UserLaunchAgentArtifact {
            let name = MacOsLocationName::try_new(state_name).expect("state name");
            let state = resolver.resolve_state(&StateLocationRef::from_normalized(name));
            let invocation = LaunchAgentInvocation::try_new(
                LaunchAgentProgram::try_new(&self.program).expect("program path"),
                state,
                LaunchAgentStateOption::try_new("--state-location").expect("state option"),
                [argument],
            )
            .expect("invocation");
            UserLaunchAgentArtifact::render_for_service(
                resolver,
                &LaunchAgentNamespace::try_new("dev.circular.test").expect("namespace"),
                LaunchAgentServiceKind::Daemon,
                self.install(resolver),
                invocation,
            )
            .expect("artifact")
        }
    }

    impl Drop for TestEnvironment {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.home);
        }
    }

    /// Pulls the `ProgramArguments` array entries out of a rendered plist.
    ///
    /// Reading them back out of the bytes rather than off the invocation is
    /// the point: launchd sees the bytes, so the round-trip must start there.
    fn program_arguments(plist: &str) -> Vec<String> {
        let start = plist
            .find("<key>ProgramArguments</key>")
            .expect("rendered plist declares ProgramArguments");
        let array = &plist[start..];
        let end = array.find("</array>").expect("the array is closed");
        array[..end]
            .match_indices("<string>")
            .map(|(index, _)| {
                let rest = &array[index + "<string>".len()..];
                let close = rest.find("</string>").expect("each entry is closed");
                rest[..close].to_string()
            })
            .collect()
    }

    #[test]
    fn the_rendered_invocation_parses_back_as_the_canonical_daemon_argv() {
        let Some(environment) = TestEnvironment::new() else {
            return;
        };
        let resolver = environment.resolver();
        let name = MacOsLocationName::try_new("primary").expect("state name");
        let state = resolver.resolve_state(&StateLocationRef::from_normalized(name));
        let invocation = LaunchAgentInvocation::try_new(
            LaunchAgentProgram::try_new(&environment.program).expect("program path"),
            state.clone(),
            LaunchAgentStateOption::canonical(),
            Vec::<String>::new(),
        )
        .expect("invocation");
        let artifact = UserLaunchAgentArtifact::render_for_service(
            &resolver,
            &LaunchAgentNamespace::try_new("dev.circular.test").expect("namespace"),
            LaunchAgentServiceKind::Daemon,
            environment.install(&resolver),
            invocation,
        )
        .expect("artifact");

        let plist = std::str::from_utf8(artifact.plist()).expect("UTF-8 plist");
        let arguments = program_arguments(plist);
        assert_eq!(arguments.len(), 3, "program plus the one canonical option");
        assert_eq!(arguments[0], environment.program.to_str().expect("UTF-8"));

        let parsed = circular_transport::DaemonArguments::parse(&arguments[1..])
            .expect("the rendered argv is the canonical argv");
        assert_eq!(parsed.state_directory(), state.directory());
    }

    #[test]
    fn a_non_canonical_state_option_still_renders_but_does_not_start_the_daemon() {
        let Some(environment) = TestEnvironment::new() else {
            return;
        };
        let resolver = environment.resolver();
        let artifact = environment.artifact(&resolver, "primary", "a");
        let plist = std::str::from_utf8(artifact.plist()).expect("UTF-8 plist");
        let arguments = program_arguments(plist);

        assert!(matches!(
            circular_transport::DaemonArguments::parse(&arguments[1..]),
            Err(circular_transport::DaemonArgumentsError::UnexpectedToken { .. })
        ));
    }

    #[test]
    fn plist_requires_state_path_before_injected_arguments_and_escapes_xml() {
        let Some(environment) = TestEnvironment::new() else {
            return;
        };
        let resolver = environment.resolver();
        let artifact = environment.artifact(&resolver, "primary", "a<&>\"'b");
        let plist = std::str::from_utf8(artifact.plist()).expect("UTF-8 plist");
        let state_option = plist.find("<string>--state-location</string>").unwrap();
        let state_path = plist
            .find(
                artifact
                    .state_location()
                    .directory()
                    .to_str()
                    .expect("UTF-8 state path"),
            )
            .unwrap();
        let injected = plist
            .find("<string>a&lt;&amp;&gt;&quot;&apos;b</string>")
            .unwrap();

        assert!(state_option < state_path && state_path < injected);
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<string>Background</string>"));
        assert!(!plist.contains("KeepAlive"));
        assert!(!plist.contains("ThrottleInterval"));
        assert!(!plist.contains("brew services"));
    }

    #[test]
    fn each_state_location_has_a_distinct_registration_and_output_destination() {
        let Some(environment) = TestEnvironment::new() else {
            return;
        };
        let resolver = environment.resolver();
        let first = environment.artifact(&resolver, "one", "--one");
        let second = environment.artifact(&resolver, "two", "--two");

        assert_ne!(first.label(), second.label());
        assert_ne!(first.plist_path(), second.plist_path());
        assert_ne!(first.stdout_path(), second.stdout_path());
        assert!(first.label().as_str().ends_with(".daemon.one"));
        assert!(second.label().as_str().ends_with(".daemon.two"));
    }

    #[test]
    fn register_replace_and_remove_touch_only_the_persistent_axis() {
        let Some(environment) = TestEnvironment::new() else {
            return;
        };
        let resolver = environment.resolver();
        let install = environment.install(&resolver);
        let registrar = UserLaunchAgentRegistrar::try_new(&resolver, &install).expect("registrar");
        let first = environment.artifact(&resolver, "primary", "old");
        let replacement = environment.artifact(&resolver, "primary", "new");

        assert_eq!(
            registrar.register(&first).expect("first registration"),
            PersistentRegistrationChange::Created
        );
        assert_eq!(
            registrar.register(&first).expect("idempotent registration"),
            PersistentRegistrationChange::Unchanged
        );
        assert_eq!(
            registrar
                .register(&replacement)
                .expect("atomic replacement"),
            PersistentRegistrationChange::Replaced
        );
        assert_eq!(
            fs::read(replacement.plist_path()).expect("registered plist"),
            replacement.plist()
        );
        assert_eq!(
            fs::metadata(replacement.plist_path())
                .expect("plist metadata")
                .mode()
                & 0o777,
            0o600
        );

        let output = replacement
            .stdout_path()
            .parent()
            .expect("output directory");
        assert!(output.is_dir());
        assert_eq!(
            registrar.unregister(&replacement).expect("remove plist"),
            PersistentRegistrationRemoval::Removed
        );
        assert!(output.is_dir(), "unregister must preserve prior output");
        assert_eq!(
            registrar
                .unregister(&replacement)
                .expect("idempotent remove"),
            PersistentRegistrationRemoval::AlreadyAbsent
        );
    }

    #[test]
    fn artifact_refuses_a_program_outside_its_resolved_install() {
        let Some(environment) = TestEnvironment::new() else {
            return;
        };
        let resolver = environment.resolver();
        let state = resolver.resolve_state(&resolver.conventional_state());
        let outside = resolver.canonical_home().join("outside-daemon");
        let invocation = LaunchAgentInvocation::try_new(
            LaunchAgentProgram::try_new(outside).expect("outside program path"),
            state,
            LaunchAgentStateOption::try_new("--state-location").expect("state option"),
            ["run"],
        )
        .expect("invocation");

        assert!(matches!(
            UserLaunchAgentArtifact::render_for_service(
                &resolver,
                &LaunchAgentNamespace::try_new("dev.circular.test").expect("namespace"),
                LaunchAgentServiceKind::Daemon,
                environment.install(&resolver),
                invocation,
            ),
            Err(LaunchAgentArtifactError::ProgramOutsideInstall { .. })
        ));
    }

    #[test]
    fn registrar_is_bound_to_the_artifacts_resolved_install() {
        let Some(environment) = TestEnvironment::new() else {
            return;
        };
        let resolver = environment.resolver();
        let artifact = environment.artifact(&resolver, "primary", "run");
        let other_name = MacOsLocationName::try_new("other").expect("install name");
        let other =
            resolver.resolve_install(&crate::InstallLocationRef::from_normalized(other_name));
        resolver.prepare_install(&other).expect("other install");
        let registrar = UserLaunchAgentRegistrar::try_new(&resolver, &other).expect("registrar");

        assert!(matches!(
            registrar.register(&artifact),
            Err(LaunchAgentIoError::ArtifactResolverMismatch)
        ));
    }

    #[test]
    fn registrar_refuses_mutable_or_symlinked_program_path_components() {
        use std::os::unix::fs::symlink;

        let Some(environment) = TestEnvironment::new() else {
            return;
        };
        let resolver = environment.resolver();
        let install = environment.install(&resolver);
        let registrar = UserLaunchAgentRegistrar::try_new(&resolver, &install).expect("registrar");
        let artifact = environment.artifact(&resolver, "primary", "run");
        fs::set_permissions(&environment.program, Permissions::from_mode(0o722))
            .expect("unsafe program mode");
        assert!(matches!(
            registrar.register(&artifact),
            Err(LaunchAgentIoError::UnsafePermissions { .. })
        ));

        fs::set_permissions(&environment.program, Permissions::from_mode(0o700))
            .expect("restore program mode");
        let bin = environment.program.parent().expect("program parent");
        fs::set_permissions(bin, Permissions::from_mode(0o722)).expect("unsafe ancestor mode");
        assert!(matches!(
            registrar.register(&artifact),
            Err(LaunchAgentIoError::UnsafePermissions { path, .. }) if path == bin
        ));

        fs::set_permissions(bin, Permissions::from_mode(0o700)).expect("restore ancestor mode");
        fs::remove_file(&environment.program).expect("remove regular program");
        let target = install.directory().join("target-daemon");
        fs::write(&target, b"target").expect("target daemon");
        fs::set_permissions(&target, Permissions::from_mode(0o700)).expect("target mode");
        symlink(&target, &environment.program).expect("program symlink");
        assert!(matches!(
            registrar.register(&artifact),
            Err(LaunchAgentIoError::ProgramSymbolicLink { .. })
        ));
    }
}
