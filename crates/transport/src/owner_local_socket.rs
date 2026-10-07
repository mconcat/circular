//! OS-backed OwnerLocal Unix-domain socket boundary.
//!
//! This module owns only the local IPC security and lifecycle cut. It does not
//! encode protocol envelopes, choose a frame profile, open a network listener,
//! or invent remote authentication. A successful connection proves all of the
//! following before bytes are exposed:
//!
//! - the socket root is the current effective user's non-symlink `0700`
//!   directory;
//! - the socket pathname is that user's non-symlink Unix socket with mode
//!   `0600`;
//! - the kernel reports that the connected peer has the same effective UID.
//!
//! The socket pathname has that mode from the moment it exists. `bind(2)`
//! creates a socket with the process umask, so the daemon binds under a sibling
//! binding name, installs `0600` there, and only then publishes the same socket
//! at the endpoint name with one `linkat(2)`. A client that connects as soon as
//! the endpoint name appears is never refused for its mode. The
//! kernel keeps the binding name as the socket's own address, so `getsockname`
//! and `lsof` report the binding name although only the endpoint name exists.
//!
//! A persistent owner-only claim file is protected with a non-blocking OS file
//! lock. The lock is released by the kernel when the process dies, so a later
//! daemon can distinguish a crashed daemon's stale pathname from an active
//! owner without a PID file, heartbeat, expiry, or manual unlock operation.

use crate::owner_local_permissions::PERMISSION_AND_SPECIAL_BITS;
use crate::{
    LocalByteStream, LocalEndpoint, LocalEndpointEvidence, OWNER_ROOT_MODE, OwnerLocalEvidence,
    validate_owner_root_mode,
};
use nix::errno::Errno;
use nix::fcntl::{AtFlags, Flock, FlockArg, OFlag, open, openat};
use nix::sys::stat::{Mode, fchmod};
use nix::unistd::{UnlinkatFlags, geteuid, linkat, unlinkat};
use std::convert::Infallible;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File, FileType, Metadata, Permissions};
use std::io::{self, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Component, Path, PathBuf};

const OWNER_FILE_MODE: u32 = 0o600;

/// Largest pathname payload accepted by the portable OwnerLocal profile.
///
/// Darwin's `sockaddr_un.sun_path` is 104 bytes and pathname sockets require
/// one trailing NUL byte. Linux permits a longer pathname, but the first
/// release is macOS and the next target must not silently change endpoint
/// identity, so the profile uses the Darwin bound on both supported targets.
pub const OWNER_LOCAL_SOCKET_PATH_MAX_BYTES: usize = 103;

fn owner_file_mode() -> Mode {
    Mode::S_IRUSR | Mode::S_IWUSR
}

/// A deployment-owned path for exactly one OwnerLocal daemon endpoint.
///
/// The root must be absolute so connection identity cannot depend on a
/// process working directory. `socket_name` is a single normal path component;
/// callers cannot use it to escape the verified root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerLocalSocketSpec {
    root: PathBuf,
    socket_name: OsString,
    binding_name: OsString,
    claim_name: OsString,
}

/// The name the daemon binds its socket under before publishing it at
/// `socket_name`; clients never open it.
///
/// It has the same byte length as `socket_name` — the last byte becomes `~`, or
/// `-` when it already is `~` — so every endpoint pathname the profile accepts
/// can be bound under it too ([`OWNER_LOCAL_SOCKET_PATH_MAX_BYTES`]). It cannot
/// be `.`, `..` or the claim name, and it always differs from `socket_name`.
fn binding_name_for(socket_name: &OsStr) -> OsString {
    let mut bytes = socket_name.as_bytes().to_vec();
    if let Some(last) = bytes.last_mut() {
        *last = if *last == b'~' { b'-' } else { b'~' };
    }
    OsString::from_vec(bytes)
}

impl OwnerLocalSocketSpec {
    pub fn try_new(
        root: impl Into<PathBuf>,
        socket_name: impl Into<OsString>,
    ) -> Result<Self, OwnerLocalSocketError> {
        let root = root.into();
        if !root.is_absolute() {
            return Err(OwnerLocalSocketError::RootNotAbsolute);
        }

        let socket_name = socket_name.into();
        let mut components = Path::new(&socket_name).components();
        if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
            return Err(OwnerLocalSocketError::InvalidSocketName);
        }

        let socket_path_bytes = root.join(&socket_name).as_os_str().as_bytes().len();
        if socket_path_bytes > OWNER_LOCAL_SOCKET_PATH_MAX_BYTES {
            return Err(OwnerLocalSocketError::SocketPathTooLong {
                bytes: socket_path_bytes,
                maximum: OWNER_LOCAL_SOCKET_PATH_MAX_BYTES,
            });
        }

        let binding_name = binding_name_for(&socket_name);
        let mut claim_name = socket_name.clone();
        claim_name.push(".owner-local.lock");
        Ok(Self {
            root,
            socket_name,
            binding_name,
            claim_name,
        })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn socket_name(&self) -> &OsStr {
        &self.socket_name
    }

    #[must_use]
    pub fn socket_path(&self) -> PathBuf {
        self.root.join(&self.socket_name)
    }

    #[must_use]
    pub fn claim_path(&self) -> PathBuf {
        self.root.join(&self.claim_name)
    }

    fn binding_path(&self) -> PathBuf {
        self.root.join(&self.binding_name)
    }
}

/// Which owner-local object a custody check is about.
///
/// The same rules (kind, owner, mode, identity) apply to the root directory, the
/// claim file and the socket pathname. The object and the violation are two axes
/// rather than one flattened arm per pair.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Custody {
    /// The owner-only root directory.
    Root,
    /// The process-lifetime claim file under the root.
    Claim,
    /// The socket pathname under the root.
    Socket,
}

impl Custody {
    const fn noun(self) -> &'static str {
        match self {
            Self::Root => "owner-local socket root",
            Self::Claim => "owner-local daemon claim",
            Self::Socket => "owner-local endpoint",
        }
    }

    /// The file kind this object must be.
    const fn kind(self) -> &'static str {
        match self {
            Self::Root => "a directory",
            Self::Claim => "a regular file",
            Self::Socket => "a Unix socket",
        }
    }

    const fn violated(self, violation: Violation) -> OwnerLocalSocketError {
        OwnerLocalSocketError::Custody {
            object: self,
            violation,
        }
    }
}

/// What a custody check found wrong with one owner-local object.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Violation {
    /// Nothing is at the path.
    Missing,
    /// The path is a symbolic link; it is never followed.
    Symlink,
    /// The path is not the file kind the object must be.
    WrongKind,
    /// The owner UID differs from the current effective UID.
    Owner { expected: u32, actual: u32 },
    /// The permission and special bits differ from the exact required mode.
    Mode { expected: u32, actual: u32 },
    /// The object changed identity (device and inode) while in use.
    IdentityChanged,
}

circular_core::closed_table! {
    /// The system-call step an owner-local IO failure happened at.
    ///
    /// The spelling is the step's name in diagnostics.
    pub enum IoStep {
        RootInspection => "root inspection",
        RootCreation => "root creation",
        RootPermissionInstallation => "root permission installation",
        RootValidation => "root validation",
        RootOpen => "root open",
        OpenRootValidation => "open root validation",
        OpenRootRevalidation => "open root revalidation",
        RootPathRevalidation => "root path revalidation",
        EndpointUnlink => "endpoint unlink",
        ClaimInspection => "claim inspection",
        ClaimOpen => "claim open",
        ClaimCreation => "claim creation",
        ClaimPermissionInstallation => "claim permission installation",
        ClaimValidation => "claim validation",
        ClaimAcquisition => "claim acquisition",
        EndpointInspection => "endpoint inspection",
        EndpointLivenessProbe => "existing endpoint liveness probe",
        ExclusiveBind => "exclusive bind",
        EndpointPermissionInstallation => "endpoint permission installation",
        EndpointPublication => "endpoint publication",
        ListenerModeChange => "listener mode change",
        Accept => "accept",
        PeerCredentialRead => "peer credential read",
        Connect => "connect",
        StreamModeChange => "stream mode change",
        StreamReadDeadline => "stream read deadline",
        StreamReadPollInterval => "stream read poll interval",
        StreamClone => "stream clone",
    }
}

/// A failure while constructing, binding, or using the OwnerLocal boundary.
#[derive(Debug)]
pub enum OwnerLocalSocketError {
    RootNotAbsolute,
    InvalidSocketName,
    SocketPathTooLong {
        bytes: usize,
        maximum: usize,
    },
    /// A custody check on the root, the claim or the socket pathname failed.
    Custody {
        object: Custody,
        violation: Violation,
    },
    ClaimAlreadyHeld,
    NoEndpointPresent,
    LiveSocketAlreadyPresent,
    PeerOwnerMismatch {
        expected: u32,
        actual: u32,
    },
    /// A polling read interval must be nonzero. Passing zero to the platform
    /// `SO_RCVTIMEO` boundary is not an immediate poll; Darwin rejects it with
    /// `EINVAL`, while Rust may reject it before the syscall on other Unix
    /// targets. Callers that need an immediate probe must use nonblocking I/O.
    InvalidReadPollInterval,
    Io {
        step: IoStep,
        source: io::Error,
    },
}

impl OwnerLocalSocketError {
    fn io(step: IoStep, source: impl Into<io::Error>) -> Self {
        Self::Io {
            step,
            source: source.into(),
        }
    }
}

impl fmt::Display for OwnerLocalSocketError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RootNotAbsolute => {
                formatter.write_str("owner-local socket root must be absolute")
            }
            Self::InvalidSocketName => {
                formatter.write_str("owner-local socket name must be one normal path component")
            }
            Self::SocketPathTooLong { bytes, maximum } => write!(
                formatter,
                "owner-local socket path is {bytes} bytes; the portable maximum is {maximum}"
            ),
            Self::Custody { object, violation } => {
                let noun = object.noun();
                match violation {
                    Violation::Missing => write!(formatter, "{noun} does not exist"),
                    Violation::Symlink => {
                        write!(formatter, "{noun} must not be a symbolic link")
                    }
                    Violation::WrongKind => write!(formatter, "{noun} is not {}", object.kind()),
                    Violation::Owner { expected, actual } => write!(
                        formatter,
                        "{noun} UID {actual} does not match current UID {expected}"
                    ),
                    Violation::Mode { expected, actual } => write!(
                        formatter,
                        "{noun} mode {actual:#o} does not match required {expected:#o}"
                    ),
                    Violation::IdentityChanged => {
                        write!(formatter, "{noun} identity changed during use")
                    }
                }
            }
            Self::ClaimAlreadyHeld => {
                formatter.write_str("another owner-local daemon holds the endpoint claim")
            }
            Self::NoEndpointPresent => {
                formatter.write_str("no owner-local endpoint is present at this state location")
            }
            Self::LiveSocketAlreadyPresent => {
                formatter.write_str("a live Unix socket already occupies the owner-local endpoint")
            }
            Self::PeerOwnerMismatch { expected, actual } => write!(
                formatter,
                "Unix peer effective UID {actual} does not match owner UID {expected}"
            ),
            Self::InvalidReadPollInterval => {
                formatter.write_str("owner-local read poll interval must be nonzero")
            }
            Self::Io { step, source } => {
                write!(formatter, "owner-local {step} failed: {source}")
            }
        }
    }
}

impl std::error::Error for OwnerLocalSocketError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    fn of(metadata: &Metadata) -> Self {
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

#[derive(Debug)]
struct OpenOwnerRoot {
    path: PathBuf,
    directory: File,
    identity: FileIdentity,
    owner_uid: u32,
}

impl OpenOwnerRoot {
    fn create_or_open(path: &Path, owner_uid: u32) -> Result<Self, OwnerLocalSocketError> {
        match fs::symlink_metadata(path) {
            Ok(metadata) => validate_root_metadata(&metadata, owner_uid)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(path)
                    .map_err(|source| OwnerLocalSocketError::io(IoStep::RootCreation, source))?;
                fs::set_permissions(path, Permissions::from_mode(OWNER_ROOT_MODE)).map_err(
                    |source| OwnerLocalSocketError::io(IoStep::RootPermissionInstallation, source),
                )?;
                let metadata = fs::symlink_metadata(path)
                    .map_err(|source| OwnerLocalSocketError::io(IoStep::RootValidation, source))?;
                validate_root_metadata(&metadata, owner_uid)?;
            }
            Err(source) => return Err(OwnerLocalSocketError::io(IoStep::RootInspection, source)),
        }
        Self::open_validated(path, owner_uid)
    }

    fn open_existing(path: &Path, owner_uid: u32) -> Result<Self, OwnerLocalSocketError> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(Custody::Root.violated(Violation::Missing));
            }
            Err(source) => return Err(OwnerLocalSocketError::io(IoStep::RootInspection, source)),
        };
        validate_root_metadata(&metadata, owner_uid)?;
        Self::open_validated(path, owner_uid)
    }

    fn open_validated(path: &Path, owner_uid: u32) -> Result<Self, OwnerLocalSocketError> {
        let descriptor = open(
            path,
            OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .map_err(|source| OwnerLocalSocketError::io(IoStep::RootOpen, source))?;
        let directory = File::from(descriptor);
        let descriptor_metadata = directory
            .metadata()
            .map_err(|source| OwnerLocalSocketError::io(IoStep::OpenRootValidation, source))?;
        validate_root_metadata(&descriptor_metadata, owner_uid)?;

        let path_metadata = fs::symlink_metadata(path)
            .map_err(|source| OwnerLocalSocketError::io(IoStep::RootPathRevalidation, source))?;
        validate_root_metadata(&path_metadata, owner_uid)?;
        let identity = FileIdentity::of(&descriptor_metadata);
        if identity != FileIdentity::of(&path_metadata) {
            return Err(Custody::Root.violated(Violation::IdentityChanged));
        }

        Ok(Self {
            path: path.to_owned(),
            directory,
            identity,
            owner_uid,
        })
    }

    fn revalidate(&self) -> Result<(), OwnerLocalSocketError> {
        let descriptor_metadata = self
            .directory
            .metadata()
            .map_err(|source| OwnerLocalSocketError::io(IoStep::OpenRootRevalidation, source))?;
        validate_root_metadata(&descriptor_metadata, self.owner_uid)?;
        if self.identity != FileIdentity::of(&descriptor_metadata) {
            return Err(Custody::Root.violated(Violation::IdentityChanged));
        }

        let path_metadata = fs::symlink_metadata(&self.path)
            .map_err(|source| OwnerLocalSocketError::io(IoStep::RootPathRevalidation, source))?;
        validate_root_metadata(&path_metadata, self.owner_uid)?;
        if self.identity != FileIdentity::of(&path_metadata) {
            return Err(Custody::Root.violated(Violation::IdentityChanged));
        }
        Ok(())
    }

    fn unlink(&self, name: &OsStr) -> Result<(), OwnerLocalSocketError> {
        unlinkat(&self.directory, name, UnlinkatFlags::NoRemoveDir)
            .map_err(|source| OwnerLocalSocketError::io(IoStep::EndpointUnlink, source))
    }
}

fn validate_root_metadata(
    metadata: &Metadata,
    owner_uid: u32,
) -> Result<(), OwnerLocalSocketError> {
    if metadata.file_type().is_symlink() {
        return Err(Custody::Root.violated(Violation::Symlink));
    }
    if !metadata.is_dir() {
        return Err(Custody::Root.violated(Violation::WrongKind));
    }
    if metadata.uid() != owner_uid {
        return Err(Custody::Root.violated(Violation::Owner {
            expected: owner_uid,
            actual: metadata.uid(),
        }));
    }
    validate_owner_root_mode(metadata.mode()).map_err(|mismatch| {
        Custody::Root.violated(Violation::Mode {
            expected: OWNER_ROOT_MODE,
            actual: mismatch.actual,
        })
    })
}

fn open_claim(root: &OpenOwnerRoot, name: &OsStr) -> Result<Flock<File>, OwnerLocalSocketError> {
    let create_flags =
        OFlag::O_RDWR | OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC;
    let (descriptor, created) = match openat(&root.directory, name, create_flags, owner_file_mode())
    {
        Ok(descriptor) => (descriptor, true),
        Err(Errno::EEXIST) => {
            let path = root.path.join(name);
            let metadata = fs::symlink_metadata(&path)
                .map_err(|source| OwnerLocalSocketError::io(IoStep::ClaimInspection, source))?;
            if metadata.file_type().is_symlink() {
                return Err(Custody::Claim.violated(Violation::Symlink));
            }
            let descriptor = openat(
                &root.directory,
                name,
                OFlag::O_RDWR | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
                Mode::empty(),
            )
            .map_err(|source| OwnerLocalSocketError::io(IoStep::ClaimOpen, source))?;
            (descriptor, false)
        }
        Err(source) => return Err(OwnerLocalSocketError::io(IoStep::ClaimCreation, source)),
    };

    if created {
        fchmod(&descriptor, owner_file_mode()).map_err(|source| {
            OwnerLocalSocketError::io(IoStep::ClaimPermissionInstallation, source)
        })?;
    }
    let file = File::from(descriptor);
    validate_claim_metadata(
        &file
            .metadata()
            .map_err(|source| OwnerLocalSocketError::io(IoStep::ClaimValidation, source))?,
        root.owner_uid,
    )?;
    root.revalidate()?;

    match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
        Ok(claim) => Ok(claim),
        Err((_file, Errno::EWOULDBLOCK)) => Err(OwnerLocalSocketError::ClaimAlreadyHeld),
        Err((_file, source)) => Err(OwnerLocalSocketError::io(IoStep::ClaimAcquisition, source)),
    }
}

fn validate_claim_metadata(
    metadata: &Metadata,
    owner_uid: u32,
) -> Result<(), OwnerLocalSocketError> {
    if !metadata.is_file() {
        return Err(Custody::Claim.violated(Violation::WrongKind));
    }
    if metadata.uid() != owner_uid {
        return Err(Custody::Claim.violated(Violation::Owner {
            expected: owner_uid,
            actual: metadata.uid(),
        }));
    }
    let actual = metadata.mode() & PERMISSION_AND_SPECIAL_BITS;
    if actual != OWNER_FILE_MODE {
        return Err(Custody::Claim.violated(Violation::Mode {
            expected: OWNER_FILE_MODE,
            actual,
        }));
    }
    Ok(())
}

fn validate_socket_metadata(
    metadata: &Metadata,
    owner_uid: u32,
) -> Result<FileIdentity, OwnerLocalSocketError> {
    let identity = validate_socket_actor_metadata(metadata, owner_uid)?;
    let actual = metadata.mode() & PERMISSION_AND_SPECIAL_BITS;
    if actual != OWNER_FILE_MODE {
        return Err(Custody::Socket.violated(Violation::Mode {
            expected: OWNER_FILE_MODE,
            actual,
        }));
    }
    Ok(identity)
}

fn validate_socket_actor_metadata(
    metadata: &Metadata,
    owner_uid: u32,
) -> Result<FileIdentity, OwnerLocalSocketError> {
    let file_type: FileType = metadata.file_type();
    if file_type.is_symlink() {
        return Err(Custody::Socket.violated(Violation::Symlink));
    }
    if !file_type.is_socket() {
        return Err(Custody::Socket.violated(Violation::WrongKind));
    }
    if metadata.uid() != owner_uid {
        return Err(Custody::Socket.violated(Violation::Owner {
            expected: owner_uid,
            actual: metadata.uid(),
        }));
    }
    Ok(FileIdentity::of(metadata))
}

/// The custody a socket pathname must already satisfy: the full rule
/// ([`validate_socket_metadata`]) or the actor rule without the mode
/// ([`validate_socket_actor_metadata`]).
type SocketCustody = fn(&Metadata, u32) -> Result<FileIdentity, OwnerLocalSocketError>;

fn inspect_socket_with(
    path: &Path,
    owner_uid: u32,
    custody: SocketCustody,
) -> Result<Option<FileIdentity>, OwnerLocalSocketError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => custody(&metadata, owner_uid).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(OwnerLocalSocketError::io(
            IoStep::EndpointInspection,
            source,
        )),
    }
}

fn inspect_socket(
    path: &Path,
    owner_uid: u32,
) -> Result<Option<FileIdentity>, OwnerLocalSocketError> {
    inspect_socket_with(path, owner_uid, validate_socket_metadata)
}

fn inspect_socket_actor(
    path: &Path,
    owner_uid: u32,
) -> Result<Option<FileIdentity>, OwnerLocalSocketError> {
    inspect_socket_with(path, owner_uid, validate_socket_actor_metadata)
}

/// Unlinks `name` under the open root only while it still names the socket
/// with `expected` identity.
fn unlink_socket_if_identity_matches(
    root: &OpenOwnerRoot,
    name: &OsStr,
    expected: FileIdentity,
) -> Result<bool, OwnerLocalSocketError> {
    match inspect_socket_actor(&root.path.join(name), root.owner_uid)? {
        Some(actual) if actual == expected => {
            root.unlink(name)?;
            Ok(true)
        }
        None | Some(_) => Ok(false),
    }
}

fn verify_socket_identity(
    path: &Path,
    owner_uid: u32,
    expected: FileIdentity,
) -> Result<(), OwnerLocalSocketError> {
    let Some(actual) = inspect_socket(path, owner_uid)? else {
        return Err(Custody::Socket.violated(Violation::IdentityChanged));
    };
    if actual != expected {
        return Err(Custody::Socket.violated(Violation::IdentityChanged));
    }
    Ok(())
}

/// Removes a dead socket a crashed daemon left at `name`, only while the claim
/// is held, only when nothing answers it, and only while it is still the same
/// socket that was inspected.
fn remove_stale_socket_if_present(
    root: &OpenOwnerRoot,
    name: &OsStr,
    custody: SocketCustody,
) -> Result<(), OwnerLocalSocketError> {
    let path = root.path.join(name);
    let Some(identity) = inspect_socket_with(&path, root.owner_uid, custody)? else {
        return Ok(());
    };
    root.revalidate()?;

    match UnixStream::connect(&path) {
        Ok(stream) => {
            drop(stream);
            Err(OwnerLocalSocketError::LiveSocketAlreadyPresent)
        }
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
            ) =>
        {
            match inspect_socket_with(&path, root.owner_uid, custody)? {
                None => Ok(()),
                Some(current) if current == identity => root.unlink(name),
                Some(_) => Err(Custody::Socket.violated(Violation::IdentityChanged)),
            }
        }
        Err(source) => Err(OwnerLocalSocketError::io(
            IoStep::EndpointLivenessProbe,
            source,
        )),
    }
}

/// A daemon-side process-lifetime claim that has not opened its endpoint yet.
///
/// Claiming and opening are separate so recovery can run under the exclusive
/// file lock without publishing a socket that clients could mistake for a
/// ready daemon.  The value is deliberately neither `Clone` nor constructible
/// from a raw file lock.
#[derive(Debug)]
pub struct OwnerLocalClaim {
    spec: OwnerLocalSocketSpec,
    root: OpenOwnerRoot,
    claim: Flock<File>,
}

impl OwnerLocalClaim {
    /// Creates the owner-only root if absent, acquires its process-lifetime
    /// claim, and removes only verified stale sockets under the endpoint and
    /// binding names.
    pub fn claim(spec: OwnerLocalSocketSpec) -> Result<Self, OwnerLocalSocketError> {
        let owner_uid = current_euid();
        let root = OpenOwnerRoot::create_or_open(spec.root(), owner_uid)?;
        let claim = open_claim(&root, &spec.claim_name)?;
        remove_stale_socket_if_present(&root, spec.socket_name(), validate_socket_metadata)?;
        remove_stale_socket_if_present(&root, &spec.binding_name, validate_socket_actor_metadata)?;
        root.revalidate()?;
        Ok(Self { spec, root, claim })
    }

    #[must_use]
    pub fn spec(&self) -> &OwnerLocalSocketSpec {
        &self.spec
    }

    #[must_use]
    pub const fn owner_uid(&self) -> u32 {
        self.root.owner_uid
    }

    /// Opens the endpoint after the caller has completed recovery.
    ///
    /// The socket is bound under the binding name, narrowed to `0600` there, and
    /// published at the endpoint name with one `linkat(2)` in the open root. The
    /// endpoint name therefore never exists with another mode, and publication,
    /// like the bind it follows, never replaces whatever already occupies a name.
    pub fn open_endpoint(self) -> Result<OwnerLocalListener, OwnerLocalSocketError> {
        let Self { spec, root, claim } = self;
        root.revalidate()?;

        let binding_path = spec.binding_path();
        let listener = UnixListener::bind(&binding_path)
            .map_err(|source| OwnerLocalSocketError::io(IoStep::ExclusiveBind, source))?;
        let bound_identity = match inspect_socket_actor(&binding_path, root.owner_uid) {
            Ok(Some(identity)) => identity,
            Ok(None) => return Err(Custody::Socket.violated(Violation::IdentityChanged)),
            Err(error) => return Err(error),
        };

        let published = (|| {
            fs::set_permissions(&binding_path, Permissions::from_mode(OWNER_FILE_MODE)).map_err(
                |source| OwnerLocalSocketError::io(IoStep::EndpointPermissionInstallation, source),
            )?;
            verify_socket_identity(&binding_path, root.owner_uid, bound_identity)?;
            root.revalidate()?;

            linkat(
                &root.directory,
                spec.binding_name.as_os_str(),
                &root.directory,
                spec.socket_name(),
                AtFlags::empty(),
            )
            .map_err(|source| OwnerLocalSocketError::io(IoStep::EndpointPublication, source))?;
            verify_socket_identity(&spec.socket_path(), root.owner_uid, bound_identity)?;
            if !unlink_socket_if_identity_matches(&root, &spec.binding_name, bound_identity)? {
                return Err(Custody::Socket.violated(Violation::IdentityChanged));
            }
            root.revalidate()
        })();
        if let Err(error) = published {
            let _ = unlink_socket_if_identity_matches(&root, &spec.binding_name, bound_identity);
            let _ = unlink_socket_if_identity_matches(&root, spec.socket_name(), bound_identity);
            return Err(error);
        }

        Ok(OwnerLocalListener {
            spec,
            root,
            _claim: claim,
            listener,
            socket_identity: bound_identity,
        })
    }
}

/// The daemon-side exclusive OwnerLocal listener.
///
/// It is deliberately neither `Clone` nor constructible from a raw listener:
/// one value owns the process-lifetime claim, verified pathname, and cleanup.
#[derive(Debug)]
pub struct OwnerLocalListener {
    spec: OwnerLocalSocketSpec,
    root: OpenOwnerRoot,
    _claim: Flock<File>,
    listener: UnixListener,
    socket_identity: FileIdentity,
}

impl std::os::fd::AsFd for OwnerLocalListener {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        std::os::fd::AsFd::as_fd(&self.listener)
    }
}

impl std::os::fd::AsFd for OwnerLocalStream {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        std::os::fd::AsFd::as_fd(&self.stream)
    }
}

impl OwnerLocalListener {
    #[must_use]
    pub fn spec(&self) -> &OwnerLocalSocketSpec {
        &self.spec
    }

    #[must_use]
    pub const fn owner_uid(&self) -> u32 {
        self.root.owner_uid
    }

    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<(), OwnerLocalSocketError> {
        self.listener
            .set_nonblocking(nonblocking)
            .map_err(|source| OwnerLocalSocketError::io(IoStep::ListenerModeChange, source))
    }

    /// Accepts one stream only after revalidating the pathname boundary and
    /// comparing the kernel-reported peer effective UID with the daemon owner.
    pub fn accept(&self) -> Result<OwnerLocalStream, OwnerLocalSocketError> {
        self.revalidate_boundary()?;
        let (stream, _address) = self
            .listener
            .accept()
            .map_err(|source| OwnerLocalSocketError::io(IoStep::Accept, source))?;
        let peer_uid = peer_euid(&stream)
            .map_err(|source| OwnerLocalSocketError::io(IoStep::PeerCredentialRead, source))?;
        verify_peer_owner(self.root.owner_uid, peer_uid)?;
        self.revalidate_boundary()?;
        Ok(OwnerLocalStream {
            stream,
            owner_uid: self.root.owner_uid,
            peer_uid,
            endpoint_path: self.spec.socket_path(),
        })
    }

    fn revalidate_boundary(&self) -> Result<(), OwnerLocalSocketError> {
        self.root.revalidate()?;
        verify_socket_identity(
            &self.spec.socket_path(),
            self.root.owner_uid,
            self.socket_identity,
        )
    }

    /// Removes the endpoint while still holding the claim. The claim file is
    /// intentionally retained so all incarnations lock the same stable inode.
    pub fn close(self) -> Result<(), OwnerLocalSocketError> {
        self.cleanup_current_socket()
    }

    fn cleanup_current_socket(&self) -> Result<(), OwnerLocalSocketError> {
        self.root.revalidate()?;
        if unlink_socket_if_identity_matches(
            &self.root,
            self.spec.socket_name(),
            self.socket_identity,
        )? {
            Ok(())
        } else {
            Err(Custody::Socket.violated(Violation::IdentityChanged))
        }
    }
}

impl Drop for OwnerLocalListener {
    fn drop(&mut self) {
        let _ = self.cleanup_current_socket();
    }
}

/// A connected stream whose peer UID has already been verified by the kernel.
#[derive(Debug)]
pub struct OwnerLocalStream {
    stream: UnixStream,
    owner_uid: u32,
    peer_uid: u32,
    endpoint_path: PathBuf,
}

impl OwnerLocalStream {
    /// Connects to an existing OwnerLocal endpoint and verifies both the path
    /// boundary and the daemon's kernel-reported effective UID.
    pub fn connect(spec: &OwnerLocalSocketSpec) -> Result<Self, OwnerLocalSocketError> {
        let owner_uid = current_euid();
        let root = OpenOwnerRoot::open_existing(spec.root(), owner_uid)?;
        let socket_identity = inspect_socket(&spec.socket_path(), owner_uid)?
            .ok_or(OwnerLocalSocketError::NoEndpointPresent)?;
        root.revalidate()?;

        let stream = UnixStream::connect(spec.socket_path())
            .map_err(|source| OwnerLocalSocketError::io(IoStep::Connect, source))?;
        let peer_uid = peer_euid(&stream)
            .map_err(|source| OwnerLocalSocketError::io(IoStep::PeerCredentialRead, source))?;
        verify_peer_owner(owner_uid, peer_uid)?;
        root.revalidate()?;
        verify_socket_identity(&spec.socket_path(), owner_uid, socket_identity)?;
        Ok(Self {
            stream,
            owner_uid,
            peer_uid,
            endpoint_path: spec.socket_path(),
        })
    }

    #[must_use]
    pub const fn owner_uid(&self) -> u32 {
        self.owner_uid
    }

    #[must_use]
    pub const fn peer_uid(&self) -> u32 {
        self.peer_uid
    }

    /// Returns the already-verified endpoint shape consumed by the abstract
    /// channel lifecycle. `Infallible` makes a non-owner local address
    /// impossible in this concrete product seam.
    ///
    /// The address slot is uninhabited, and an empty `match` is the proof: the
    /// `UserLocal` arm can produce a value of *any* type because no value can
    /// reach it.
    ///
    /// ```
    /// use transport::LocalEndpoint;
    /// use std::convert::Infallible;
    /// use std::path::PathBuf;
    ///
    /// fn owner_path(endpoint: LocalEndpoint<PathBuf, Infallible>) -> PathBuf {
    ///     match endpoint {
    ///         LocalEndpoint::OwnerLocal { path } => path,
    ///         LocalEndpoint::UserLocal { address } => match address {},
    ///     }
    /// }
    /// ```
    ///
    /// So no non-owner address can be widened into the endpoint type an
    /// OwnerLocal stream produces — the assembly has no place to put one.
    ///
    /// ```compile_fail
    /// use transport::LocalEndpoint;
    /// use std::convert::Infallible;
    /// use std::path::PathBuf;
    ///
    /// fn forge(address: String) -> LocalEndpoint<PathBuf, Infallible> {
    ///     LocalEndpoint::UserLocal { address }
    /// }
    /// ```
    #[must_use]
    pub fn transport_endpoint(&self) -> LocalEndpoint<PathBuf, Infallible> {
        LocalEndpoint::OwnerLocal {
            path: self.endpoint_path.clone(),
        }
    }

    /// Projects the kernel/path proof into the abstract transport evidence
    /// carrier without asking a higher layer to reconstruct boolean trust
    /// inputs.
    ///
    /// The evidence carrier is uninhabited on its non-owner arm for the same
    /// reason, so a caller cannot hand this seam evidence that was collected
    /// for anything but an owner-only path.
    ///
    /// ```compile_fail
    /// use transport::{LocalEndpointEvidence, UserLocalEvidence};
    /// use std::convert::Infallible;
    /// use std::path::PathBuf;
    ///
    /// fn forge(
    ///     evidence: UserLocalEvidence<u32, String>,
    /// ) -> LocalEndpointEvidence<u32, PathBuf, Infallible> {
    ///     LocalEndpointEvidence::UserLocal(evidence)
    /// }
    /// ```
    #[must_use]
    pub fn transport_evidence(&self) -> LocalEndpointEvidence<u32, PathBuf, Infallible> {
        let evidence = OwnerLocalEvidence::try_new(
            self.endpoint_path.clone(),
            self.peer_uid,
            self.owner_uid,
            true,
        )
        .expect("an OwnerLocalStream stores only verified owner evidence");
        LocalEndpointEvidence::OwnerLocal(evidence)
    }

    pub fn set_nonblocking(&self, nonblocking: bool) -> Result<(), OwnerLocalSocketError> {
        self.stream
            .set_nonblocking(nonblocking)
            .map_err(|source| OwnerLocalSocketError::io(IoStep::StreamModeChange, source))
    }

    /// Bounds how long one read may wait for the peer.
    ///
    /// A server that reads before it answers has a shape the previous
    /// close-immediately server did not: a peer that connects and then says
    /// nothing occupies the reader.  With a single-threaded accept loop that
    /// one peer stops every other peer, so the wait has to be bounded
    /// somewhere.  This is the place — the bound belongs to whoever owns the
    /// loop, and the transport only makes it expressible.
    ///
    /// The duration itself is **not** this module's to choose; callers pass
    /// one.  `None` restores the blocking default.
    pub fn set_read_deadline(
        &self,
        deadline: Option<std::time::Duration>,
    ) -> Result<(), OwnerLocalSocketError> {
        self.stream
            .set_read_timeout(deadline)
            .map_err(|source| OwnerLocalSocketError::io(IoStep::StreamReadDeadline, source))
    }

    /// Set the nonzero relative interval used by a polling envelope reader.
    ///
    /// This is deliberately distinct from [`Self::set_read_deadline`]. A
    /// polling timeout is not a terminal peer deadline: expiry wakes the
    /// caller's idle callback and the next read continues. Zero cannot express
    /// that contract through `SO_RCVTIMEO`, so it is rejected here before a
    /// platform-specific `EINVAL` can tear down an established session.
    pub fn set_read_poll_interval(
        &self,
        interval: std::time::Duration,
    ) -> Result<(), OwnerLocalSocketError> {
        if interval.is_zero() {
            return Err(OwnerLocalSocketError::InvalidReadPollInterval);
        }
        self.stream
            .set_read_timeout(Some(interval))
            .map_err(|source| OwnerLocalSocketError::io(IoStep::StreamReadPollInterval, source))
    }

    pub fn try_clone(&self) -> Result<Self, OwnerLocalSocketError> {
        Ok(Self {
            stream: self
                .stream
                .try_clone()
                .map_err(|source| OwnerLocalSocketError::io(IoStep::StreamClone, source))?,
            owner_uid: self.owner_uid,
            peer_uid: self.peer_uid,
            endpoint_path: self.endpoint_path.clone(),
        })
    }
}

impl LocalByteStream for OwnerLocalStream {
    type Error = io::Error;

    fn read(&mut self, destination: &mut [u8]) -> Result<usize, Self::Error> {
        self.stream.read(destination)
    }

    fn write(&mut self, source: &[u8]) -> Result<usize, Self::Error> {
        self.stream.write(source)
    }
}

fn current_euid() -> u32 {
    geteuid().as_raw()
}

fn verify_peer_owner(expected: u32, actual: u32) -> Result<(), OwnerLocalSocketError> {
    if expected == actual {
        Ok(())
    } else {
        Err(OwnerLocalSocketError::PeerOwnerMismatch { expected, actual })
    }
}

#[cfg(any(target_vendor = "apple", target_os = "freebsd", target_os = "openbsd"))]
fn peer_euid(stream: &UnixStream) -> io::Result<u32> {
    nix::unistd::getpeereid(stream)
        .map(|(uid, _gid)| uid.as_raw())
        .map_err(Into::into)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn peer_euid(stream: &UnixStream) -> io::Result<u32> {
    nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials)
        .map(|credentials| credentials.uid())
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind(spec: OwnerLocalSocketSpec) -> Result<OwnerLocalListener, OwnerLocalSocketError> {
        OwnerLocalClaim::claim(spec)?.open_endpoint()
    }

    #[derive(Debug)]
    struct TestRoot(circular_testkit::temp::StateDir);

    impl TestRoot {
        fn new() -> Self {
            let root = circular_testkit::temp::StateDir::new("owner-local");
            fs::set_permissions(root.path(), Permissions::from_mode(OWNER_ROOT_MODE))
                .expect("owner-only test root");
            Self(root)
        }

        fn path(&self) -> &Path {
            self.0.path()
        }

        fn spec(&self) -> OwnerLocalSocketSpec {
            OwnerLocalSocketSpec::try_new(self.path(), "daemon.sock").expect("valid test spec")
        }
    }

    #[test]
    fn spec_rejects_working_directory_and_root_escape_inputs() {
        assert!(matches!(
            OwnerLocalSocketSpec::try_new("relative", "daemon.sock"),
            Err(OwnerLocalSocketError::RootNotAbsolute)
        ));
        let root = TestRoot::new();
        for name in ["", ".", "..", "nested/daemon.sock"] {
            assert!(matches!(
                OwnerLocalSocketSpec::try_new(&root.path(), name),
                Err(OwnerLocalSocketError::InvalidSocketName)
            ));
        }
    }

    #[test]
    fn spec_rejects_paths_outside_the_portable_unix_socket_abi_bound() {
        let exact_root = PathBuf::from(format!(
            "/{}",
            "r".repeat(OWNER_LOCAL_SOCKET_PATH_MAX_BYTES - 3)
        ));
        let exact = OwnerLocalSocketSpec::try_new(exact_root, "s")
            .expect("pathname exactly at the profile bound");
        assert_eq!(
            exact.socket_path().as_os_str().as_bytes().len(),
            OWNER_LOCAL_SOCKET_PATH_MAX_BYTES
        );

        let overlong_root = PathBuf::from(format!(
            "/{}",
            "r".repeat(OWNER_LOCAL_SOCKET_PATH_MAX_BYTES - 2)
        ));
        assert!(matches!(
            OwnerLocalSocketSpec::try_new(overlong_root, "s"),
            Err(OwnerLocalSocketError::SocketPathTooLong {
                bytes,
                maximum: OWNER_LOCAL_SOCKET_PATH_MAX_BYTES,
            }) if bytes == OWNER_LOCAL_SOCKET_PATH_MAX_BYTES + 1
        ));
    }

    #[test]
    fn bind_installs_exact_owner_only_modes_and_drop_cleans_only_socket() {
        let root = TestRoot::new();
        let spec = root.spec();
        let listener = bind(spec.clone()).expect("secure bind");

        let root_metadata = fs::symlink_metadata(spec.root()).expect("root metadata");
        assert_eq!(
            root_metadata.mode() & PERMISSION_AND_SPECIAL_BITS,
            OWNER_ROOT_MODE
        );
        assert_eq!(root_metadata.uid(), current_euid());
        let socket_metadata = fs::symlink_metadata(spec.socket_path()).expect("socket metadata");
        assert!(socket_metadata.file_type().is_socket());
        assert_eq!(
            socket_metadata.mode() & PERMISSION_AND_SPECIAL_BITS,
            OWNER_FILE_MODE
        );
        assert_eq!(socket_metadata.uid(), current_euid());
        let claim_metadata = fs::symlink_metadata(spec.claim_path()).expect("claim metadata");
        assert!(claim_metadata.is_file());
        assert_eq!(
            claim_metadata.mode() & PERMISSION_AND_SPECIAL_BITS,
            OWNER_FILE_MODE
        );

        drop(listener);
        assert!(!spec.socket_path().exists());
        assert!(spec.claim_path().is_file(), "stable lock inode remains");
    }

    const OPENINGS_WATCHED: usize = 400;

    #[test]
    fn a_client_that_connects_the_moment_the_endpoint_appears_is_never_refused() {
        let root = TestRoot::new();
        let spec = root.spec();
        let path = spec.socket_path();
        let mut refusals = Vec::new();
        for _ in 0..OPENINGS_WATCHED {
            let outcome = std::thread::scope(|scope| {
                let client = scope.spawn(|| {
                    while fs::symlink_metadata(&path).is_err() {}
                    OwnerLocalStream::connect(&spec).map(drop)
                });
                let listener = bind(spec.clone()).expect("secure bind");
                let outcome = client.join().expect("client");
                drop(listener);
                outcome
            });
            if let Err(error) = outcome {
                refusals.push(format!("{error:?}"));
            }
        }
        assert!(
            refusals.is_empty(),
            "{} of {OPENINGS_WATCHED} immediate clients were refused; first: {:?}",
            refusals.len(),
            refusals.first()
        );
    }

    #[test]
    fn bind_creates_only_the_endpoint_root_with_exact_owner_mode() {
        let parent = TestRoot::new();
        let socket_root = parent.path().join("ipc");
        let spec = OwnerLocalSocketSpec::try_new(&socket_root, "daemon.sock").expect("valid spec");
        let listener = bind(spec.clone()).expect("creates endpoint root");

        let metadata = fs::symlink_metadata(&socket_root).expect("created endpoint root");
        assert!(metadata.is_dir());
        assert_eq!(metadata.uid(), current_euid());
        assert_eq!(
            metadata.mode() & PERMISSION_AND_SPECIAL_BITS,
            OWNER_ROOT_MODE
        );
        drop(listener);
    }

    #[test]
    fn second_daemon_fails_immediately_without_touching_live_endpoint() {
        let root = TestRoot::new();
        let spec = root.spec();
        let first = bind(spec.clone()).expect("first daemon");
        let identity = FileIdentity::of(
            &fs::symlink_metadata(spec.socket_path()).expect("live socket metadata"),
        );

        assert!(matches!(
            bind(spec.clone()),
            Err(OwnerLocalSocketError::ClaimAlreadyHeld)
        ));
        assert_eq!(
            FileIdentity::of(&fs::symlink_metadata(spec.socket_path()).expect("still live")),
            identity
        );
        drop(first);
    }

    #[test]
    fn publication_never_replaces_what_occupies_the_endpoint_name() {
        let root = TestRoot::new();
        let spec = root.spec();
        let claim = OwnerLocalClaim::claim(spec.clone()).expect("claim");
        fs::write(spec.socket_path(), b"occupant").expect("occupant after the claim");

        let error = claim
            .open_endpoint()
            .expect_err("an occupied endpoint name is refused");
        assert!(
            matches!(
                &error,
                OwnerLocalSocketError::Io {
                    step: IoStep::EndpointPublication,
                    source,
                } if source.kind() == io::ErrorKind::AlreadyExists
            ),
            "{error:?}"
        );
        assert_eq!(
            fs::read(spec.socket_path()).expect("occupant kept"),
            b"occupant"
        );
        assert!(
            fs::symlink_metadata(spec.binding_path()).is_err(),
            "the binding name is discarded"
        );
    }

    #[test]
    fn symlinks_wrong_actor_types_and_permission_downgrades_fail_closed() {
        let root = TestRoot::new();
        let spec = root.spec();
        fs::write(spec.socket_path(), b"not a socket").expect("regular collision");
        assert!(matches!(
            bind(spec.clone()),
            Err(OwnerLocalSocketError::Custody {
                object: Custody::Socket,
                violation: Violation::WrongKind
            })
        ));
        fs::remove_file(spec.socket_path()).expect("remove collision");

        std::os::unix::fs::symlink(spec.claim_path(), spec.socket_path()).expect("socket symlink");
        assert!(matches!(
            bind(spec.clone()),
            Err(OwnerLocalSocketError::Custody {
                object: Custody::Socket,
                violation: Violation::Symlink
            })
        ));
        fs::remove_file(spec.socket_path()).expect("remove symlink");

        let listener = bind(spec.clone()).expect("secure bind");
        fs::set_permissions(spec.socket_path(), Permissions::from_mode(0o660))
            .expect("downgrade endpoint mode");
        assert!(matches!(
            OwnerLocalStream::connect(&spec),
            Err(OwnerLocalSocketError::Custody {
                object: Custody::Socket,
                violation: Violation::Mode { .. }
            })
        ));
        assert!(matches!(
            listener.accept(),
            Err(OwnerLocalSocketError::Custody {
                object: Custody::Socket,
                violation: Violation::Mode { .. }
            })
        ));
        drop(listener);
        assert!(
            !spec.socket_path().exists(),
            "the exact owned socket is cleaned even after a mode downgrade"
        );
    }

    #[test]
    fn invocation_and_socket_rechecks_share_the_exact_mode_policy() {
        use crate::OwnerRootModeMismatch;
        use crate::{DaemonArguments, StateDirectoryRejection, current_effective_user};

        let home = TestRoot::new();
        let state = home.path().join("state");
        fs::create_dir(&state).expect("state directory");
        fs::set_permissions(&state, Permissions::from_mode(0o700)).expect("private state");
        let arguments =
            DaemonArguments::parse([OsString::from("--state"), state.as_os_str().to_owned()])
                .expect("state arguments");
        let uid = current_effective_user();
        let accepted = arguments
            .accept_state_directory(&home.path(), uid)
            .expect("initial acceptance");
        let root = OpenOwnerRoot::open_existing(accepted.path(), uid).expect("open root");

        for mode in [0o1700, 0o600, 0o750] {
            fs::set_permissions(&state, Permissions::from_mode(mode)).expect("change mode");
            let invocation = arguments
                .accept_state_directory(&home.path(), uid)
                .expect_err("invocation must reject changed mode");
            let socket = OpenOwnerRoot::open_existing(accepted.path(), uid)
                .expect_err("socket must recheck earlier acceptance");
            let recheck = root.revalidate().expect_err("open root must recheck mode");
            assert!(matches!(
                invocation,
                StateDirectoryRejection::NotOwnerOnly {
                    source: OwnerRootModeMismatch { actual },
                    ..
                } if actual == mode
            ));
            for error in [&socket, &recheck] {
                assert!(matches!(
                    error,
                    OwnerLocalSocketError::Custody {
                        object: Custody::Root,
                        violation: Violation::Mode { expected: 0o700, actual },
                    } if *actual == mode
                ));
            }
            fs::set_permissions(&state, Permissions::from_mode(0o700)).expect("restore mode");
            root.revalidate().expect("restored root");
        }
    }

    #[test]
    fn root_and_claim_permission_boundaries_fail_closed() {
        let parent = TestRoot::new();
        let real_root = parent.path().join("real");
        fs::create_dir(&real_root).expect("real root");
        fs::set_permissions(&real_root, Permissions::from_mode(OWNER_ROOT_MODE))
            .expect("real root mode");
        let linked_root = parent.path().join("linked");
        std::os::unix::fs::symlink(&real_root, &linked_root).expect("root symlink");
        let linked_spec =
            OwnerLocalSocketSpec::try_new(&linked_root, "daemon.sock").expect("valid spec");
        assert!(matches!(
            bind(linked_spec),
            Err(OwnerLocalSocketError::Custody {
                object: Custody::Root,
                violation: Violation::Symlink
            })
        ));

        let root = TestRoot::new();
        let spec = root.spec();
        fs::set_permissions(spec.root(), Permissions::from_mode(0o750)).expect("downgrade root");
        assert!(matches!(
            bind(spec.clone()),
            Err(OwnerLocalSocketError::Custody {
                object: Custody::Root,
                violation: Violation::Mode { .. }
            })
        ));
        fs::set_permissions(spec.root(), Permissions::from_mode(OWNER_ROOT_MODE))
            .expect("restore root");

        let target = spec.root().join("claim-target");
        fs::write(&target, b"not a claim").expect("claim target");
        std::os::unix::fs::symlink(&target, spec.claim_path()).expect("claim symlink");
        assert!(matches!(
            bind(spec),
            Err(OwnerLocalSocketError::Custody {
                object: Custody::Claim,
                violation: Violation::Symlink
            })
        ));
    }

    #[test]
    fn peer_identity_mismatch_is_a_terminal_rejection() {
        assert!(matches!(
            verify_peer_owner(100, 101),
            Err(OwnerLocalSocketError::PeerOwnerMismatch {
                expected: 100,
                actual: 101
            })
        ));
        assert!(verify_peer_owner(100, 100).is_ok());
    }

    #[test]
    fn drop_does_not_remove_a_replacement_path() {
        let root = TestRoot::new();
        let spec = root.spec();
        let listener = bind(spec.clone()).expect("secure bind");
        fs::remove_file(spec.socket_path()).expect("remove owned socket");
        fs::write(spec.socket_path(), b"replacement").expect("replacement path");

        drop(listener);
        assert_eq!(
            fs::read(spec.socket_path()).expect("replacement retained"),
            b"replacement"
        );
    }

    #[test]
    fn stream_moves_opaque_bytes_after_bilateral_uid_verification() {
        let root = TestRoot::new();
        let spec = root.spec();
        let listener = bind(spec.clone()).expect("secure bind");
        let mut client = OwnerLocalStream::connect(&spec).expect("verified server");
        let mut server = listener.accept().expect("verified client");

        assert_eq!(client.owner_uid(), current_euid());
        assert_eq!(server.owner_uid(), current_euid());
        assert_eq!(
            crate::establish_local_trust(
                &client.transport_endpoint(),
                &client.transport_evidence(),
                circular_protocol::TransportTrust::LocalOwner,
            ),
            Ok(circular_protocol::TransportTrust::LocalOwner)
        );
        LocalByteStream::write(&mut client, b"opaque").expect("write");
        let mut received = [0_u8; 6];
        let count = LocalByteStream::read(&mut server, &mut received).expect("read");
        assert_eq!(count, received.len());
        assert_eq!(&received, b"opaque");
    }

    #[test]
    fn accepted_stream_can_replace_its_read_deadline_with_polling_bounds() {
        let root = TestRoot::new();
        let spec = root.spec();
        let listener = bind(spec.clone()).expect("secure bind");
        let _client = OwnerLocalStream::connect(&spec).expect("verified server");
        let server = listener.accept().expect("verified client");

        assert!(matches!(
            server.set_read_deadline(Some(std::time::Duration::ZERO)),
            Err(OwnerLocalSocketError::Io { source, .. })
                if source.kind() == io::ErrorKind::InvalidInput
                    || source.raw_os_error() == Some(22)
        ));
        assert!(matches!(
            server.set_read_poll_interval(std::time::Duration::ZERO),
            Err(OwnerLocalSocketError::InvalidReadPollInterval)
        ));

        for deadline in [
            std::time::Duration::from_secs(5),
            std::time::Duration::from_millis(500),
            std::time::Duration::from_millis(1),
            std::time::Duration::from_millis(500),
        ] {
            server
                .set_read_poll_interval(deadline)
                .unwrap_or_else(|error| panic!("{deadline:?} is a valid polling bound: {error}"));
        }
    }
}
