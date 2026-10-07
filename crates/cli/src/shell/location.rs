//! Apple Silicon macOS realization of logical Shell locations.
//!
//! Logical state and install references stay in the disjoint types defined by
//! `location_selection`.  This module maps them below a canonical, injected
//! user home. Registration may bind an explicit physical state path instead;
//! neither path selection consults the current working directory.

use crate::{
    InstallLocationCatalog, InstallLocationRef, StateLocationCatalog, StateLocationRef,
    StateLocationRegistration, select_install,
};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, DirBuilder, FileType, Permissions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const PRODUCT_DIRECTORY: &str = "Circular";
const STATE_DIRECTORY: &str = "States";
const INSTALL_DIRECTORY: &str = "Installations";
const CONVENTIONAL_INSTALL_PATH: [&str; 3] = [".local", "opt", "circular"];
/// The one current selection inside the conventional install location: the link
/// `scripts/install.sh` points at the installation it selected.
const CURRENT_SELECTION: &str = "current";
const CONVENTIONAL_LOCATION_NAME: &str = "default";
const ID_PATH: &str = "/usr/bin/id";

/// The largest single path component supported by the macOS filesystems on
/// which this realization is installed.
pub const MACOS_NAME_MAX: usize = 255;

/// Evidence that the selected product target is the supported first-release
/// platform.
///
/// Constructing this value is required before a macOS location resolver or a
/// launch-agent registrar can be built.  Intel macOS is intentionally not an
/// accepted alias.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppleSiliconMacOs {
    _private: (),
}

impl AppleSiliconMacOs {
    /// Validates a Rust target-style OS and architecture pair.
    pub fn validate(os: &str, architecture: &str) -> Result<Self, UnsupportedPlatform> {
        if os != "macos" {
            return Err(UnsupportedPlatform::OperatingSystem {
                found: os.to_owned().into_boxed_str(),
            });
        }
        if architecture != "aarch64" && architecture != "arm64" {
            return Err(UnsupportedPlatform::Architecture {
                found: architecture.to_owned().into_boxed_str(),
            });
        }
        Ok(Self { _private: () })
    }

    /// Validates the target for which the running executable was compiled.
    pub fn current_target() -> Result<Self, UnsupportedPlatform> {
        Self::validate(std::env::consts::OS, std::env::consts::ARCH)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnsupportedPlatform {
    OperatingSystem { found: Box<str> },
    Architecture { found: Box<str> },
}

impl fmt::Display for UnsupportedPlatform {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OperatingSystem { found } => {
                write!(
                    formatter,
                    "unsupported operating system `{found}`; expected macos"
                )
            }
            Self::Architecture { found } => write!(
                formatter,
                "unsupported macOS architecture `{found}`; expected Apple Silicon"
            ),
        }
    }
}

impl std::error::Error for UnsupportedPlatform {}

/// A non-root current-user identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MacOsUserId(u32);

impl MacOsUserId {
    /// Derives the effective user rather than accepting a caller-selected UID.
    ///
    /// `id -u` reports the effective UID (`id -ru` would report the real UID).
    /// The absolute system-tool path also keeps this query independent of
    /// `PATH` and the current working directory.
    pub fn current() -> Result<Self, EffectiveUserIdError> {
        let output = Command::new(ID_PATH)
            .arg("-u")
            .output()
            .map_err(EffectiveUserIdError::Query)?;
        if !output.status.success() {
            return Err(EffectiveUserIdError::CommandFailed {
                status: output.status.code(),
            });
        }
        parse_effective_user_id(&output.stdout)
    }

    fn try_new(value: u32) -> Result<Self, RootUserNotSupported> {
        if value == 0 {
            Err(RootUserNotSupported)
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RootUserNotSupported;

impl fmt::Display for RootUserNotSupported {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the current-user macOS realization must not run as root")
    }
}

impl std::error::Error for RootUserNotSupported {}

#[derive(Debug)]
pub enum EffectiveUserIdError {
    Query(io::Error),
    CommandFailed { status: Option<i32> },
    InvalidOutput { output: Box<str> },
    Root(RootUserNotSupported),
}

impl fmt::Display for EffectiveUserIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Query(source) => write!(formatter, "failed to query effective user ID: {source}"),
            Self::CommandFailed { status } => {
                write!(
                    formatter,
                    "effective-user query exited with status {status:?}"
                )
            }
            Self::InvalidOutput { output } => {
                write!(
                    formatter,
                    "effective-user query returned invalid UID `{output}`"
                )
            }
            Self::Root(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for EffectiveUserIdError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Query(source) => Some(source),
            Self::Root(error) => Some(error),
            _ => None,
        }
    }
}

fn parse_effective_user_id(output: &[u8]) -> Result<MacOsUserId, EffectiveUserIdError> {
    let text = std::str::from_utf8(output).map(str::trim).map_err(|_| {
        EffectiveUserIdError::InvalidOutput {
            output: String::from_utf8_lossy(output)
                .into_owned()
                .into_boxed_str(),
        }
    })?;
    let value = text
        .parse::<u32>()
        .map_err(|_| EffectiveUserIdError::InvalidOutput {
            output: text.to_owned().into_boxed_str(),
        })?;
    MacOsUserId::try_new(value).map_err(EffectiveUserIdError::Root)
}

/// A logical name that is also safe as one macOS path and launchd-label
/// component.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MacOsLocationName(Box<str>);

impl MacOsLocationName {
    pub fn try_new(value: impl Into<Box<str>>) -> Result<Self, InvalidLocationName> {
        let value = value.into().to_ascii_lowercase().into_boxed_str();
        if value.is_empty() {
            return Err(InvalidLocationName::Empty);
        }
        if value.as_ref() == "." || value.as_ref() == ".." {
            return Err(InvalidLocationName::ReservedComponent);
        }
        if value.len() > MACOS_NAME_MAX {
            return Err(InvalidLocationName::TooLong {
                bytes: value.len(),
                maximum: MACOS_NAME_MAX,
            });
        }
        if let Some((index, byte)) = value
            .bytes()
            .enumerate()
            .find(|(_, byte)| !byte.is_ascii_alphanumeric() && !matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(InvalidLocationName::InvalidByte { index, byte });
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MacOsLocationName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InvalidLocationName {
    Empty,
    ReservedComponent,
    TooLong { bytes: usize, maximum: usize },
    InvalidByte { index: usize, byte: u8 },
}

impl fmt::Display for InvalidLocationName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("a location name cannot be empty"),
            Self::ReservedComponent => formatter.write_str("`.` and `..` are not location names"),
            Self::TooLong { bytes, maximum } => write!(
                formatter,
                "location name is {bytes} bytes; the macOS maximum is {maximum}"
            ),
            Self::InvalidByte { index, byte } => write!(
                formatter,
                "location name byte {index} (0x{byte:02x}) is not label/path safe"
            ),
        }
    }
}

impl std::error::Error for InvalidLocationName {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedMacOsStateLocation {
    logical: StateLocationRef<MacOsLocationName>,
    directory: PathBuf,
    output_directory: PathBuf,
}

impl ResolvedMacOsStateLocation {
    #[must_use]
    pub const fn logical(&self) -> &StateLocationRef<MacOsLocationName> {
        &self.logical
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    #[must_use]
    pub fn output_directory(&self) -> &Path {
        &self.output_directory
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedMacOsInstallLocation {
    logical: InstallLocationRef<MacOsLocationName>,
    directory: PathBuf,
}

impl ResolvedMacOsInstallLocation {
    #[must_use]
    pub const fn logical(&self) -> &InstallLocationRef<MacOsLocationName> {
        &self.logical
    }

    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

#[derive(Debug)]
pub enum CurrentUserLocationError {
    HomeNotNamed,
    Platform(UnsupportedPlatform),
    Location(LocationIoError),
}

impl From<UnsupportedPlatform> for CurrentUserLocationError {
    fn from(error: UnsupportedPlatform) -> Self {
        Self::Platform(error)
    }
}

impl From<LocationIoError> for CurrentUserLocationError {
    fn from(error: LocationIoError) -> Self {
        Self::Location(error)
    }
}

impl fmt::Display for CurrentUserLocationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HomeNotNamed => formatter
                .write_str("HOME is not set, so this user's home directory cannot be resolved"),
            Self::Platform(error) => write!(formatter, "{error}"),
            Self::Location(error) => write!(formatter, "{error}"),
        }
    }
}

/// CWD-independent physical location provider for one macOS user.
#[derive(Clone, Debug)]
pub struct MacOsUserLocationResolver {
    platform: AppleSiliconMacOs,
    user: MacOsUserId,
    canonical_home: PathBuf,
    install_directory: Option<PathBuf>,
    explicit_state: Option<ResolvedMacOsStateLocation>,
}

impl MacOsUserLocationResolver {
    pub fn for_current_user() -> Result<Self, CurrentUserLocationError> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or(CurrentUserLocationError::HomeNotNamed)?;
        let platform = AppleSiliconMacOs::current_target()?;
        Ok(Self::try_new(platform, &home)?)
    }

    /// Derives the process effective UID, resolves symlink aliases in the
    /// injected home exactly once, and verifies that the home belongs to that
    /// non-root effective user.
    pub fn try_new(
        platform: AppleSiliconMacOs,
        home: impl AsRef<Path>,
    ) -> Result<Self, LocationIoError> {
        let user = MacOsUserId::current().map_err(LocationIoError::EffectiveUserId)?;
        Self::try_new_for_user(platform, user, home)
    }

    /// Test-only seam for exercising owner mismatch behavior without allowing
    /// product code to select another user's launchd domain.
    #[cfg(test)]
    fn try_new_for_test(
        platform: AppleSiliconMacOs,
        user: MacOsUserId,
        home: impl AsRef<Path>,
    ) -> Result<Self, LocationIoError> {
        Self::try_new_for_user(platform, user, home)
    }

    fn try_new_for_user(
        platform: AppleSiliconMacOs,
        user: MacOsUserId,
        home: impl AsRef<Path>,
    ) -> Result<Self, LocationIoError> {
        let supplied = home.as_ref();
        if !supplied.is_absolute() {
            return Err(LocationIoError::HomeNotAbsolute {
                path: supplied.to_path_buf(),
            });
        }
        let canonical_home = fs::canonicalize(supplied).map_err(|source| LocationIoError::Io {
            operation: LocationIoOperation::CanonicalizeHome,
            path: supplied.to_path_buf(),
            source,
        })?;
        validate_owned_directory(&canonical_home, user, false)?;
        Ok(Self {
            platform,
            user,
            canonical_home,
            install_directory: None,
            explicit_state: None,
        })
    }

    #[must_use]
    pub const fn platform(&self) -> AppleSiliconMacOs {
        self.platform
    }

    #[must_use]
    pub const fn user(&self) -> MacOsUserId {
        self.user
    }

    #[must_use]
    pub fn canonical_home(&self) -> &Path {
        &self.canonical_home
    }

    #[must_use]
    pub fn conventional_state(&self) -> StateLocationRef<MacOsLocationName> {
        StateLocationRef::from_normalized(conventional_name())
    }

    #[must_use]
    pub fn conventional_install(&self) -> InstallLocationRef<MacOsLocationName> {
        InstallLocationRef::from_normalized(conventional_name())
    }

    #[must_use]
    pub fn resolve_state(
        &self,
        logical: &StateLocationRef<MacOsLocationName>,
    ) -> ResolvedMacOsStateLocation {
        if let Some(state) = &self.explicit_state {
            if state.logical() == logical {
                return state.clone();
            }
        }
        let name = logical.identity().as_str();
        ResolvedMacOsStateLocation {
            logical: logical.clone(),
            directory: self
                .application_support_root()
                .join(STATE_DIRECTORY)
                .join(name),
            output_directory: self.logs_root().join(name),
        }
    }

    /// Bind the supplied physical state path without treating its basename as
    /// a managed location. The label is local registration bookkeeping only.
    /// Resolve an existing ancestor so /tmp aliases and first-run creation do
    /// not change the label between register and unregister.
    pub(crate) fn with_state_directory(
        mut self,
        directory: &Path,
    ) -> Result<(Self, ResolvedMacOsStateLocation), LocationIoError> {
        let canonical = self.registration_state_path(directory)?;
        let managed_root = self.application_support_root().join(STATE_DIRECTORY);
        let managed_name = canonical
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| MacOsLocationName::try_new(name).ok())
            .filter(|name| managed_root.join(name.as_str()) == canonical);
        let name = managed_name.unwrap_or_else(|| {
            let digest = Sha256::digest(canonical.as_os_str().as_encoded_bytes());
            MacOsLocationName::try_new(format!("direct-{digest:x}"))
                .expect("hexadecimal path digest is label-safe")
        });
        let state = ResolvedMacOsStateLocation {
            logical: StateLocationRef::from_normalized(name.clone()),
            directory: directory.to_path_buf(),
            output_directory: self.logs_root().join(name.as_str()),
        };
        self.explicit_state = Some(state.clone());
        Ok((self, state))
    }

    fn registration_state_path(&self, directory: &Path) -> Result<PathBuf, LocationIoError> {
        if !directory.is_absolute()
            || directory
                .components()
                .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
        {
            return Err(LocationIoError::NonNormalComponent {
                path: directory.to_path_buf(),
            });
        }
        let mut ancestor = directory;
        let mut missing = Vec::new();
        let mut canonical = loop {
            match fs::canonicalize(ancestor) {
                Ok(path) => break path,
                Err(source) if source.kind() == io::ErrorKind::NotFound => {
                    let Some(name) = ancestor.file_name() else {
                        return Err(LocationIoError::EscapesUserHome {
                            path: directory.to_path_buf(),
                        });
                    };
                    missing.push(name);
                    ancestor = ancestor
                        .parent()
                        .expect("absolute path component has a parent");
                }
                Err(source) => {
                    return Err(LocationIoError::Io {
                        operation: LocationIoOperation::Inspect,
                        path: ancestor.to_path_buf(),
                        source,
                    });
                }
            }
        };
        for name in missing.into_iter().rev() {
            canonical.push(name);
        }
        if !canonical.starts_with(&self.canonical_home) || canonical == self.canonical_home {
            return Err(LocationIoError::EscapesUserHome {
                path: directory.to_path_buf(),
            });
        }
        Ok(canonical)
    }

    /// Create missing directories at mode 0700. Existing directories are
    /// inspected, never chmod'd. The daemon's owner-local acceptance remains
    /// the final authority for the state root's exact permission bits.
    pub(crate) fn prepare_registration_state(
        &self,
        state: &ResolvedMacOsStateLocation,
    ) -> Result<(), LocationIoError> {
        let canonical = self.registration_state_path(state.directory())?;
        let mut current = self.canonical_home.clone();
        for component in canonical
            .strip_prefix(&self.canonical_home)
            .expect("validated home")
            .components()
        {
            current.push(component);
            ensure_directory_exists(&current)?;
            validate_owned_directory(&current, self.user, false)?;
        }
        Ok(())
    }

    /// Bind the installation containing the running native executable. Program
    /// validation remains in the registrar; no installation is created here.
    pub(crate) fn with_install_directory(mut self, directory: PathBuf) -> Self {
        self.install_directory = Some(directory);
        self
    }

    #[must_use]
    pub fn resolve_install(
        &self,
        logical: &InstallLocationRef<MacOsLocationName>,
    ) -> ResolvedMacOsInstallLocation {
        ResolvedMacOsInstallLocation {
            logical: logical.clone(),
            directory: if logical == &self.conventional_install() {
                self.install_directory.clone().unwrap_or_else(|| {
                    CONVENTIONAL_INSTALL_PATH
                        .iter()
                        .fold(self.canonical_home.clone(), |path, part| path.join(part))
                        .join(CURRENT_SELECTION)
                })
            } else {
                self.application_support_root()
                    .join(INSTALL_DIRECTORY)
                    .join(logical.identity().as_str())
            },
        }
    }

    #[must_use]
    pub fn selected_install(&self) -> ResolvedMacOsInstallLocation {
        let discovered = InstallLocationCatalog::<MacOsLocationName>::try_new([])
            .expect("an empty install-location catalog is valid");
        let logical = select_install(None, &discovered, || self.conventional_install())
            .expect("an empty catalog without an explicit reference selects the conventional one");
        self.resolve_install(&logical)
    }

    /// Discovers only state locations in this resolver's managed namespace.
    ///
    /// Discovery is read-only: a missing managed hierarchy is an empty catalog,
    /// and this function never creates or repairs directories. Every existing
    /// hierarchy component and every registered location must retain the same
    /// owner/symlink/privacy invariants as [`Self::prepare_state`]. The directory
    /// name is accepted only when it is already the canonical spelling of a
    /// [`MacOsLocationName`]; discovery never turns an arbitrary path basename
    /// into a logical location identity.
    pub fn discover_state_locations(
        &self,
    ) -> Result<StateLocationCatalog<MacOsLocationName>, StateLocationDiscoveryError> {
        let root = self.application_support_root().join(STATE_DIRECTORY);
        let relative = root
            .strip_prefix(&self.canonical_home)
            .expect("the compiled state root is below the canonical home");
        let mut current = self.canonical_home.clone();

        for (index, component) in relative.components().enumerate() {
            let Component::Normal(component) = component else {
                return Err(StateLocationDiscoveryError::Hierarchy(
                    LocationIoError::NonNormalComponent { path: root },
                ));
            };
            current.push(component);
            match fs::symlink_metadata(&current) {
                Ok(_) => validate_owned_directory(&current, self.user, index >= 2)
                    .map_err(StateLocationDiscoveryError::Hierarchy)?,
                Err(source) if source.kind() == io::ErrorKind::NotFound => {
                    return StateLocationCatalog::try_new([])
                        .map_err(|_| unreachable!("an empty state-location catalog is valid"));
                }
                Err(source) => {
                    return Err(StateLocationDiscoveryError::Read {
                        path: current,
                        source,
                    });
                }
            }
        }

        let entries = fs::read_dir(&root).map_err(|source| StateLocationDiscoveryError::Read {
            path: root.clone(),
            source,
        })?;
        let mut registrations = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| StateLocationDiscoveryError::Read {
                path: root.clone(),
                source,
            })?;
            let path = entry.path();
            let Some(spelling) = entry.file_name().to_str().map(str::to_owned) else {
                return Err(StateLocationDiscoveryError::NonUnicodeName { path });
            };
            let name = MacOsLocationName::try_new(spelling.clone()).map_err(|source| {
                StateLocationDiscoveryError::InvalidName {
                    path: path.clone(),
                    source,
                }
            })?;
            if name.as_str() != spelling {
                return Err(StateLocationDiscoveryError::NonCanonicalName {
                    path,
                    normalized: name,
                });
            }
            validate_owned_directory(&path, self.user, true).map_err(|source| {
                StateLocationDiscoveryError::UnsafeLocation {
                    path: path.clone(),
                    source,
                }
            })?;
            registrations.push(StateLocationRegistration::new(
                StateLocationRef::from_normalized(name),
            ));
        }

        StateLocationCatalog::try_new(registrations).map_err(|error| match error {
            crate::LocationCatalogError::DuplicateRegistration { location } => {
                StateLocationDiscoveryError::DuplicateIdentity {
                    identity: location.into_identity(),
                }
            }
            crate::LocationCatalogError::MultipleDefaults { .. } => {
                unreachable!("discovery never marks a default")
            }
        })
    }

    /// Creates and tightens only Circular-owned state and output directories.
    /// Existing system/user Library directories are validated but not chmod'd.
    pub fn prepare_state(&self, state: &ResolvedMacOsStateLocation) -> Result<(), LocationIoError> {
        self.ensure_descendant(state.directory(), 2)?;
        self.prepare_output_destination(state)
    }

    /// Creates the boot-safe output destination without opening or creating
    /// the state location itself.  Daemon registration uses this narrower
    /// operation so registration remains independent from starting a run.
    pub fn prepare_output_destination(
        &self,
        state: &ResolvedMacOsStateLocation,
    ) -> Result<(), LocationIoError> {
        self.ensure_descendant(state.output_directory(), 2)
    }

    /// Creates and tightens only the Circular-owned install hierarchy.
    pub fn prepare_install(
        &self,
        install: &ResolvedMacOsInstallLocation,
    ) -> Result<(), LocationIoError> {
        self.ensure_descendant(install.directory(), 2)
    }

    /// Ensures the current user's standard LaunchAgents directory exists.
    /// Existing permissions are accepted when other users cannot write it.
    pub fn prepare_launch_agents_directory(&self) -> Result<PathBuf, LocationIoError> {
        let path = self.launch_agents_directory();
        self.ensure_descendant(&path, usize::MAX)?;
        Ok(path)
    }

    #[must_use]
    pub fn launch_agents_directory(&self) -> PathBuf {
        self.canonical_home.join("Library").join("LaunchAgents")
    }

    fn application_support_root(&self) -> PathBuf {
        self.canonical_home
            .join("Library")
            .join("Application Support")
            .join(PRODUCT_DIRECTORY)
    }

    fn logs_root(&self) -> PathBuf {
        self.canonical_home
            .join("Library")
            .join("Logs")
            .join(PRODUCT_DIRECTORY)
    }

    fn ensure_descendant(
        &self,
        target: &Path,
        private_from_component: usize,
    ) -> Result<(), LocationIoError> {
        let relative = target.strip_prefix(&self.canonical_home).map_err(|_| {
            LocationIoError::EscapesUserHome {
                path: target.to_path_buf(),
            }
        })?;
        let mut current = self.canonical_home.clone();

        for (index, component) in relative.components().enumerate() {
            let Component::Normal(component) = component else {
                return Err(LocationIoError::NonNormalComponent {
                    path: target.to_path_buf(),
                });
            };
            current.push(component);
            ensure_directory_exists(&current)?;
            let private = index >= private_from_component;
            validate_owned_directory(&current, self.user, private)?;
            if private {
                fs::set_permissions(&current, Permissions::from_mode(0o700)).map_err(|source| {
                    LocationIoError::Io {
                        operation: LocationIoOperation::SetPermissions,
                        path: current.clone(),
                        source,
                    }
                })?;
                validate_owned_directory(&current, self.user, true)?;
            }
        }
        Ok(())
    }
}

/// A read-only managed state-location catalog failure.
#[derive(Debug)]
pub enum StateLocationDiscoveryError {
    Hierarchy(LocationIoError),
    Read {
        path: PathBuf,
        source: io::Error,
    },
    NonUnicodeName {
        path: PathBuf,
    },
    InvalidName {
        path: PathBuf,
        source: InvalidLocationName,
    },
    NonCanonicalName {
        path: PathBuf,
        normalized: MacOsLocationName,
    },
    UnsafeLocation {
        path: PathBuf,
        source: LocationIoError,
    },
    DuplicateIdentity {
        identity: MacOsLocationName,
    },
}

impl fmt::Display for StateLocationDiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hierarchy(source) => {
                write!(
                    formatter,
                    "managed state-location hierarchy is unsafe: {source}"
                )
            }
            Self::Read { path, source } => write!(
                formatter,
                "failed to read managed state locations at {}: {source}",
                path.display()
            ),
            Self::NonUnicodeName { path } => write!(
                formatter,
                "managed state-location name is not Unicode: {}",
                path.display()
            ),
            Self::InvalidName { path, source } => write!(
                formatter,
                "managed state-location name is invalid at {}: {source}",
                path.display()
            ),
            Self::NonCanonicalName { path, normalized } => write!(
                formatter,
                "managed state-location name at {} is not canonical; expected `{normalized}`",
                path.display()
            ),
            Self::UnsafeLocation { path, source } => write!(
                formatter,
                "managed state location at {} is unsafe: {source}",
                path.display()
            ),
            Self::DuplicateIdentity { identity } => write!(
                formatter,
                "managed state-location identity `{identity}` is registered more than once"
            ),
        }
    }
}

impl std::error::Error for StateLocationDiscoveryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Hierarchy(source) | Self::UnsafeLocation { source, .. } => Some(source),
            Self::Read { source, .. } => Some(source),
            Self::InvalidName { source, .. } => Some(source),
            Self::NonUnicodeName { .. }
            | Self::NonCanonicalName { .. }
            | Self::DuplicateIdentity { .. } => None,
        }
    }
}

fn conventional_name() -> MacOsLocationName {
    MacOsLocationName::try_new(CONVENTIONAL_LOCATION_NAME)
        .expect("the compiled conventional name is valid")
}

fn ensure_directory_exists(path: &Path) -> Result<(), LocationIoError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            let mut builder = DirBuilder::new();
            builder.mode(0o700);
            builder.create(path).map_err(|source| LocationIoError::Io {
                operation: LocationIoOperation::CreateDirectory,
                path: path.to_path_buf(),
                source,
            })
        }
        Err(source) => Err(LocationIoError::Io {
            operation: LocationIoOperation::Inspect,
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn validate_owned_directory(
    path: &Path,
    user: MacOsUserId,
    private: bool,
) -> Result<(), LocationIoError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| LocationIoError::Io {
        operation: LocationIoOperation::Inspect,
        path: path.to_path_buf(),
        source,
    })?;
    validate_directory_kind(path, &metadata.file_type())?;
    if metadata.uid() != user.get() {
        return Err(LocationIoError::WrongOwner {
            path: path.to_path_buf(),
            expected: user.get(),
            actual: metadata.uid(),
        });
    }
    let mode = metadata.mode() & 0o777;
    if mode & 0o022 != 0 {
        return Err(LocationIoError::WritableByOtherUser {
            path: path.to_path_buf(),
            mode,
        });
    }
    if private && mode & 0o077 != 0 {
        return Err(LocationIoError::NotPrivate {
            path: path.to_path_buf(),
            mode,
        });
    }
    Ok(())
}

fn validate_directory_kind(path: &Path, file_type: &FileType) -> Result<(), LocationIoError> {
    if file_type.is_symlink() {
        Err(LocationIoError::SymbolicLink {
            path: path.to_path_buf(),
        })
    } else if !file_type.is_dir() {
        Err(LocationIoError::NotDirectory {
            path: path.to_path_buf(),
        })
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocationIoOperation {
    CanonicalizeHome,
    Inspect,
    CreateDirectory,
    SetPermissions,
}

#[derive(Debug)]
pub enum LocationIoError {
    EffectiveUserId(EffectiveUserIdError),
    HomeNotAbsolute {
        path: PathBuf,
    },
    EscapesUserHome {
        path: PathBuf,
    },
    NonNormalComponent {
        path: PathBuf,
    },
    SymbolicLink {
        path: PathBuf,
    },
    NotDirectory {
        path: PathBuf,
    },
    WrongOwner {
        path: PathBuf,
        expected: u32,
        actual: u32,
    },
    WritableByOtherUser {
        path: PathBuf,
        mode: u32,
    },
    NotPrivate {
        path: PathBuf,
        mode: u32,
    },
    Io {
        operation: LocationIoOperation,
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for LocationIoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EffectiveUserId(error) => error.fmt(formatter),
            Self::HomeNotAbsolute { path } => {
                write!(formatter, "user home is not absolute: {}", path.display())
            }
            Self::EscapesUserHome { path } => {
                write!(
                    formatter,
                    "resolved path escapes the user home: {}",
                    path.display()
                )
            }
            Self::NonNormalComponent { path } => write!(
                formatter,
                "resolved path has a non-normal component: {}",
                path.display()
            ),
            Self::SymbolicLink { path } => {
                write!(
                    formatter,
                    "managed directory is a symbolic link: {}",
                    path.display()
                )
            }
            Self::NotDirectory { path } => {
                write!(
                    formatter,
                    "managed path is not a directory: {}",
                    path.display()
                )
            }
            Self::WrongOwner {
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "{} is owned by uid {actual}, expected uid {expected}",
                path.display()
            ),
            Self::WritableByOtherUser { path, mode } => write!(
                formatter,
                "{} has unsafe writable permissions {:03o}",
                path.display(),
                mode
            ),
            Self::NotPrivate { path, mode } => write!(
                formatter,
                "{} is not private to its owner ({:03o})",
                path.display(),
                mode
            ),
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "macOS location {operation:?} failed for {}: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for LocationIoError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::EffectiveUserId(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestHome(PathBuf);

    impl TestHome {
        fn new() -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "circular-cli-location-{}-{sequence}",
                std::process::id()
            ));
            let mut builder = DirBuilder::new();
            builder.mode(0o700);
            builder.create(&path).expect("isolated test home");
            Self(path)
        }

        fn user(&self) -> Option<MacOsUserId> {
            let uid = fs::metadata(&self.0).expect("test home metadata").uid();
            MacOsUserId::try_new(uid).ok()
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("remove isolated test home");
        }
    }

    fn platform() -> AppleSiliconMacOs {
        AppleSiliconMacOs::validate("macos", "aarch64").expect("supported test target")
    }

    #[test]
    fn first_release_platform_is_apple_silicon_macos_only() {
        assert!(AppleSiliconMacOs::validate("macos", "aarch64").is_ok());
        assert!(AppleSiliconMacOs::validate("macos", "arm64").is_ok());
        assert!(matches!(
            AppleSiliconMacOs::validate("macos", "x86_64"),
            Err(UnsupportedPlatform::Architecture { .. })
        ));
        assert!(matches!(
            AppleSiliconMacOs::validate("linux", "aarch64"),
            Err(UnsupportedPlatform::OperatingSystem { .. })
        ));
        assert_eq!(MacOsUserId::try_new(0), Err(RootUserNotSupported));
        assert!(matches!(
            parse_effective_user_id(b"0\n"),
            Err(EffectiveUserIdError::Root(RootUserNotSupported))
        ));
        assert!(matches!(
            parse_effective_user_id(b"not-a-uid\n"),
            Err(EffectiveUserIdError::InvalidOutput { .. })
        ));
    }

    #[test]
    fn names_are_one_safe_component_in_both_disjoint_namespaces() {
        for invalid in ["", ".", "..", "a/b", "a b", "café"] {
            assert!(MacOsLocationName::try_new(invalid).is_err(), "{invalid}");
        }

        let name = MacOsLocationName::try_new("project_1.prod").expect("safe name");
        let state = StateLocationRef::from_normalized(name.clone());
        let install = InstallLocationRef::from_normalized(name);
        assert_eq!(state.identity().as_str(), install.identity().as_str());
        assert_eq!(
            MacOsLocationName::try_new("Primary").expect("normalized name"),
            MacOsLocationName::try_new("primary").expect("normalized name")
        );
    }

    #[test]
    fn product_resolver_derives_the_effective_user_and_refuses_root() {
        let home = TestHome::new();
        match MacOsUserId::current() {
            Ok(user) => {
                let resolver = MacOsUserLocationResolver::try_new(platform(), &home.0)
                    .expect("effective user owns the test home");
                assert_eq!(resolver.user(), user);
            }
            Err(EffectiveUserIdError::Root(RootUserNotSupported)) => assert!(matches!(
                MacOsUserLocationResolver::try_new(platform(), &home.0),
                Err(LocationIoError::EffectiveUserId(
                    EffectiveUserIdError::Root(RootUserNotSupported)
                ))
            )),
            Err(error) => panic!("unexpected effective-user query failure: {error}"),
        }
    }

    #[test]
    fn resolver_is_home_anchored_and_keeps_state_install_and_output_disjoint() {
        let home = TestHome::new();
        let Some(user) = home.user() else {
            return;
        };
        let resolver = MacOsUserLocationResolver::try_new_for_test(platform(), user, &home.0)
            .expect("resolver");
        let state = resolver.resolve_state(&resolver.conventional_state());
        let install = resolver.resolve_install(&resolver.conventional_install());

        assert_eq!(
            state.directory(),
            resolver
                .canonical_home()
                .join("Library/Application Support/Circular/States/default")
        );
        assert_eq!(
            install.directory(),
            resolver
                .canonical_home()
                .join(".local/opt/circular/current")
        );
        assert_eq!(
            state.output_directory(),
            resolver
                .canonical_home()
                .join("Library/Logs/Circular/default")
        );
        assert_ne!(state.directory(), install.directory());
        assert_ne!(state.directory(), state.output_directory());
    }

    #[test]
    fn selected_install_is_the_one_rule_every_surface_receives() {
        let home = TestHome::new();
        let Some(user) = home.user() else {
            return;
        };
        let resolver = MacOsUserLocationResolver::try_new_for_test(platform(), user, &home.0)
            .expect("resolver");

        assert_eq!(
            resolver.selected_install().directory(),
            resolver
                .canonical_home()
                .join(".local/opt/circular/current"),
            "with no installation catalog found, the current selection at the conventional location is used"
        );
        assert_eq!(
            resolver.selected_install().logical(),
            &resolver.conventional_install()
        );

        let prefix = home.0.join("opt/circular");
        let bound = resolver.with_install_directory(prefix.clone());
        assert_eq!(bound.selected_install().directory(), prefix);
    }

    #[test]
    fn state_discovery_is_read_only_when_the_managed_hierarchy_is_absent() {
        let home = TestHome::new();
        let Some(user) = home.user() else {
            return;
        };
        let resolver = MacOsUserLocationResolver::try_new_for_test(platform(), user, &home.0)
            .expect("resolver");
        let managed_root = home.0.join("Library/Application Support/Circular/States");

        let catalog = resolver
            .discover_state_locations()
            .expect("missing hierarchy is an empty catalog");

        assert!(catalog.is_empty());
        assert!(
            !managed_root.exists(),
            "discovery must not materialize state"
        );
    }

    #[test]
    fn state_discovery_emits_canonical_logical_locations_in_identity_order() {
        let home = TestHome::new();
        let Some(user) = home.user() else {
            return;
        };
        let resolver = MacOsUserLocationResolver::try_new_for_test(platform(), user, &home.0)
            .expect("resolver");
        for spelling in ["zeta", "alpha", "middle"] {
            let logical = StateLocationRef::from_normalized(
                MacOsLocationName::try_new(spelling).expect("canonical test name"),
            );
            resolver
                .prepare_state(&resolver.resolve_state(&logical))
                .expect("managed state");
        }

        let catalog = resolver
            .discover_state_locations()
            .expect("managed catalog");
        let identities = catalog
            .iter()
            .map(|location| location.identity().as_str())
            .collect::<Vec<_>>();

        assert_eq!(identities, ["alpha", "middle", "zeta"]);
        assert_eq!(catalog.marked_default(), None);
    }

    #[test]
    fn state_discovery_rejects_a_noncanonical_directory_name_instead_of_renaming_it() {
        let home = TestHome::new();
        let Some(user) = home.user() else {
            return;
        };
        let resolver = MacOsUserLocationResolver::try_new_for_test(platform(), user, &home.0)
            .expect("resolver");
        let seed = resolver.resolve_state(&resolver.conventional_state());
        resolver.prepare_state(&seed).expect("managed hierarchy");
        let path = seed
            .directory()
            .parent()
            .expect("state root")
            .join("Uppercase");
        let mut builder = DirBuilder::new();
        builder.mode(0o700);
        builder.create(&path).expect("noncanonical entry");

        assert!(matches!(
            resolver.discover_state_locations(),
            Err(StateLocationDiscoveryError::NonCanonicalName { path: found, normalized })
                if found == path && normalized.as_str() == "uppercase"
        ));
    }

    #[test]
    fn state_discovery_rejects_an_unsafe_entry_instead_of_silently_skipping_it() {
        let home = TestHome::new();
        let Some(user) = home.user() else {
            return;
        };
        let resolver = MacOsUserLocationResolver::try_new_for_test(platform(), user, &home.0)
            .expect("resolver");
        let seed = resolver.resolve_state(&resolver.conventional_state());
        resolver.prepare_state(&seed).expect("managed hierarchy");
        let path = seed
            .directory()
            .parent()
            .expect("state root")
            .join("not-a-directory");
        fs::write(&path, b"not state").expect("unsafe entry");

        assert!(matches!(
            resolver.discover_state_locations(),
            Err(StateLocationDiscoveryError::UnsafeLocation {
                path: found,
                source: LocationIoError::NotDirectory { .. },
            }) if found == path
        ));
    }

    #[test]
    fn prepared_product_directories_are_private_and_launch_agents_is_not_state() {
        let home = TestHome::new();
        let Some(user) = home.user() else {
            return;
        };
        let resolver = MacOsUserLocationResolver::try_new_for_test(platform(), user, &home.0)
            .expect("resolver");
        let state = resolver.resolve_state(&resolver.conventional_state());
        let install = resolver.resolve_install(&resolver.conventional_install());

        resolver.prepare_state(&state).expect("prepare state");
        resolver.prepare_install(&install).expect("prepare install");
        let launch_agents = resolver
            .prepare_launch_agents_directory()
            .expect("prepare LaunchAgents");

        for directory in [
            state.directory(),
            state.output_directory(),
            install.directory(),
        ] {
            let metadata = fs::symlink_metadata(directory).expect("prepared metadata");
            assert!(metadata.is_dir());
            assert_eq!(metadata.mode() & 0o777, 0o700);
            assert_eq!(metadata.uid(), user.get());
        }
        assert_eq!(
            launch_agents,
            resolver.canonical_home().join("Library/LaunchAgents")
        );
        assert!(!launch_agents.starts_with(state.directory()));
    }

    #[test]
    fn resolver_rejects_a_symlink_inside_the_managed_hierarchy() {
        use std::os::unix::fs::symlink;

        let home = TestHome::new();
        let Some(user) = home.user() else {
            return;
        };
        let resolver = MacOsUserLocationResolver::try_new_for_test(platform(), user, &home.0)
            .expect("resolver");
        fs::create_dir_all(home.0.join("Library/Application Support"))
            .expect("application support");
        symlink(
            home.0.join("elsewhere"),
            home.0.join("Library/Application Support/Circular"),
        )
        .expect("test symlink");

        let state = resolver.resolve_state(&resolver.conventional_state());
        assert!(matches!(
            resolver.prepare_state(&state),
            Err(LocationIoError::SymbolicLink { .. })
        ));
    }

    #[test]
    fn resolver_rejects_a_caller_injected_cross_user_identity() {
        let home = TestHome::new();
        let actual = fs::metadata(&home.0).expect("test home metadata").uid();
        let forged_value = if actual == 1 { 2 } else { 1 };
        let forged = MacOsUserId::try_new(forged_value).expect("non-root forged UID");

        assert!(matches!(
            MacOsUserLocationResolver::try_new_for_test(platform(), forged, &home.0),
            Err(LocationIoError::WrongOwner {
                expected,
                actual: found,
                ..
            }) if expected == forged_value && found == actual
        ));
    }
}
