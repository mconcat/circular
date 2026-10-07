
use crate::environment::ResourceName;
use nix::errno::Errno;
use nix::fcntl::{OFlag, open, openat};
use nix::sys::stat::Mode;
use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use zeroize::Zeroizing;

pub struct InMemorySecretBytes {
    bytes: Zeroizing<Vec<u8>>,
}

impl InMemorySecretBytes {
    #[must_use]
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: Zeroizing::new(bytes.into()),
        }
    }

    pub(crate) fn material_for_action(&self) -> &[u8] {
        self.bytes.as_slice()
    }
}

pub struct SecretVault {
    entries: BTreeMap<ResourceName, InMemorySecretBytes>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SecretVaultError {
    RootMissing,
    RootAccessDenied,
    RootSymlink,
    RootNotDirectory,
    RootReplaced,
    RootIo {
        errno: i32,
    },
    RootPermissions {
        mode: u32,
    },
    EntryNotRegularFile {
        name: Box<str>,
    },
    EntryAccessDenied {
        name: Box<str>,
    },
    EntryIo {
        name: Box<str>,
        errno: i32,
    },
    EntryPermissions {
        name: Box<str>,
        mode: u32,
    },
    EntrySymlink {
        name: Box<str>,
    },
    EmptyValue {
        name: Box<str>,
    },
    InvalidName {
        name: Box<str>,
    },
    OwnerMismatch,
}

impl fmt::Display for SecretVaultError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OwnerMismatch => {
                formatter.write_str("secret vault owner must match the effective user")
            }
            Self::RootMissing => formatter.write_str("secret vault root does not exist"),
            Self::RootAccessDenied => {
                formatter.write_str("secret vault root cannot be opened: permission denied")
            }
            Self::RootSymlink => {
                formatter.write_str("secret vault root is a symlink; it is not followed")
            }
            Self::RootNotDirectory => formatter.write_str("secret vault root is not a directory"),
            Self::RootReplaced => {
                formatter.write_str("secret vault root was replaced while it was being loaded")
            }
            Self::RootIo { errno } => {
                write!(formatter, "secret vault root failed with errno {errno}")
            }
            Self::EntryAccessDenied { name } => write!(
                formatter,
                "secret vault entry {name:?} cannot be opened: permission denied"
            ),
            Self::EntryIo { name, errno } => write!(
                formatter,
                "secret vault entry {name:?} failed with errno {errno}"
            ),
            Self::RootPermissions { mode } => {
                write!(
                    formatter,
                    "secret vault root mode must be 0700, found {mode:04o}"
                )
            }
            Self::EntryNotRegularFile { name } => {
                write!(
                    formatter,
                    "secret vault entry {name:?} is not a regular file"
                )
            }
            Self::EntryPermissions { name, mode } => write!(
                formatter,
                "secret vault entry {name:?} mode must be 0600, found {mode:04o}"
            ),
            Self::EntrySymlink { name } => {
                write!(formatter, "secret vault entry {name:?} is a symlink")
            }
            Self::EmptyValue { name } => {
                write!(formatter, "secret vault entry {name:?} has an empty value")
            }
            Self::InvalidName { name } => {
                write!(formatter, "secret vault entry name {name:?} is invalid")
            }
        }
    }
}

impl std::error::Error for SecretVaultError {}

impl SecretVault {
    pub fn load(root: &Path) -> Result<Self, SecretVaultError> {
        Self::load_owned(root, nix::unistd::geteuid().as_raw())
    }

    fn load_owned(root: &Path, owner: u32) -> Result<Self, SecretVaultError> {
        let root_file = std::fs::File::from(
            open(
                root,
                OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
                Mode::empty(),
            )
            .map_err(|errno| root_open_failure(root, errno))?,
        );
        let root_metadata = root_file
            .metadata()
            .map_err(|error| root_failure(errno_of(&error)))?;
        ensure_owner(&root_metadata, owner)?;
        if !root_metadata.file_type().is_dir() {
            return Err(SecretVaultError::RootNotDirectory);
        }
        let root_mode = permission_mode(&root_metadata);
        if root_mode != 0o700 {
            return Err(SecretVaultError::RootPermissions { mode: root_mode });
        }

        let directory = std::fs::read_dir(root).map_err(|error| root_failure(errno_of(&error)))?;
        let mut entries = BTreeMap::new();
        for entry in directory {
            let entry = entry.map_err(|error| root_failure(errno_of(&error)))?;
            let raw_name = entry.file_name();
            let display_name = raw_name.to_string_lossy().into_owned().into_boxed_str();
            let Some(raw_name) = raw_name.to_str() else {
                return Err(SecretVaultError::InvalidName { name: display_name });
            };
            let Some(name) = ResourceName::parse(raw_name) else {
                return Err(SecretVaultError::InvalidName { name: display_name });
            };
            let fd = openat(
                &root_file,
                raw_name,
                OFlag::O_RDONLY | OFlag::O_NOFOLLOW | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
                Mode::empty(),
            )
            .map_err(|errno| entry_failure(&display_name, errno))?;
            let mut file = std::fs::File::from(fd);
            let metadata = file
                .metadata()
                .map_err(|error| entry_failure(&display_name, errno_of(&error)))?;
            ensure_owner(&metadata, owner)?;
            if !metadata.file_type().is_file() {
                return Err(SecretVaultError::EntryNotRegularFile { name: display_name });
            }
            let mode = permission_mode(&metadata);
            if mode != 0o600 {
                return Err(SecretVaultError::EntryPermissions {
                    name: display_name,
                    mode,
                });
            }
            let mut value = Vec::new();
            file.read_to_end(&mut value)
                .map_err(|error| entry_failure(raw_name, errno_of(&error)))?;
            if value.last() == Some(&b'\n') {
                value.pop();
            }
            if value.is_empty() {
                return Err(SecretVaultError::EmptyValue {
                    name: raw_name.into(),
                });
            }
            entries.insert(name, InMemorySecretBytes::new(value));
        }
        let current =
            std::fs::symlink_metadata(root).map_err(|error| root_failure(errno_of(&error)))?;
        if current.dev() != root_metadata.dev() || current.ino() != root_metadata.ino() {
            return Err(SecretVaultError::RootReplaced);
        }
        Ok(Self { entries })
    }

    #[must_use]
    pub(crate) fn resolve(&self, name: &ResourceName) -> Option<&InMemorySecretBytes> {
        self.entries.get(name)
    }

    /// Only a yes/no boundary check leaves custody; no material or summary is returned.
    #[must_use]
    pub fn contains_material(&self, bytes: &[u8]) -> bool {
        self.entries.values().any(|secret| {
            let material = secret.material_for_action();
            !material.is_empty() && bytes.windows(material.len()).any(|part| part == material)
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn ensure_owner(metadata: &std::fs::Metadata, owner: u32) -> Result<(), SecretVaultError> {
    if metadata.uid() != owner {
        Err(SecretVaultError::OwnerMismatch)
    } else {
        Ok(())
    }
}

fn errno_of(error: &std::io::Error) -> Errno {
    Errno::from_raw(error.raw_os_error().unwrap_or(0))
}

fn root_failure(errno: Errno) -> SecretVaultError {
    match errno {
        Errno::ENOENT => SecretVaultError::RootMissing,
        Errno::EACCES | Errno::EPERM => SecretVaultError::RootAccessDenied,
        Errno::ELOOP => SecretVaultError::RootSymlink,
        Errno::ENOTDIR => SecretVaultError::RootNotDirectory,
        other => SecretVaultError::RootIo {
            errno: other as i32,
        },
    }
}

fn root_open_failure(root: &Path, errno: Errno) -> SecretVaultError {
    match std::fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() => SecretVaultError::RootSymlink,
        _ => root_failure(errno),
    }
}

fn entry_failure(name: &str, errno: Errno) -> SecretVaultError {
    let name: Box<str> = name.into();
    match errno {
        Errno::ELOOP => SecretVaultError::EntrySymlink { name },
        Errno::EACCES | Errno::EPERM => SecretVaultError::EntryAccessDenied { name },
        Errno::ENXIO | Errno::ENODEV | Errno::EOPNOTSUPP | Errno::EISDIR => {
            SecretVaultError::EntryNotRegularFile { name }
        }
        other => SecretVaultError::EntryIo {
            name,
            errno: other as i32,
        },
    }
}

fn permission_mode(metadata: &std::fs::Metadata) -> u32 {
    metadata.permissions().mode() & 0o7777
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct FixtureDirectory(PathBuf);

    impl FixtureDirectory {
        fn new() -> Self {
            let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "circular-secret-vault-{}-{sequence}",
                std::process::id()
            ));
            std::fs::create_dir(&path).expect("vault fixture root");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                .expect("vault fixture root mode");
            Self(path)
        }

        fn file(&self, name: &str, value: &[u8], mode: u32) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, value).expect("vault fixture value");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))
                .expect("vault fixture value mode");
            path
        }
    }

    impl Drop for FixtureDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn security_vault_rejects_foreign_owner_and_root_alias_without_exposing_material() {
        let root = FixtureDirectory::new();
        const SECRET: &[u8] = b"owner-check-private-material-442ef";
        root.file("api-key", SECRET, 0o600);
        assert!(
            SecretVault::load(&root.0)
                .unwrap()
                .contains_material(SECRET)
        );
        let failure = SecretVault::load_owned(&root.0, u32::MAX).err().unwrap();
        assert_eq!(failure, SecretVaultError::OwnerMismatch);
        assert!(
            !format!("{failure:?}")
                .as_bytes()
                .windows(SECRET.len())
                .any(|b| b == SECRET)
        );
        let entry = std::fs::File::open(root.0.join("api-key")).unwrap();
        assert_eq!(
            ensure_owner(&entry.metadata().unwrap(), u32::MAX),
            Err(SecretVaultError::OwnerMismatch)
        );
        let alias_root = FixtureDirectory::new();
        let alias = alias_root.0.join("alias");
        symlink(&root.0, &alias).unwrap();
        assert_eq!(
            SecretVault::load(&alias).err(),
            Some(SecretVaultError::RootSymlink)
        );
    }

    #[test]
    fn load_strips_one_trailing_newline_and_resolves_zeroizing_values() {
        let root = FixtureDirectory::new();
        root.file("slack-bot", b"Bearer fixture-token\n", 0o600);
        root.file("two-newlines", b"value\n\n", 0o600);

        let vault = SecretVault::load(&root.0).expect("valid vault loads atomically");
        assert_eq!(vault.len(), 2);
        assert!(!vault.is_empty());
        assert_eq!(
            vault
                .resolve(&ResourceName::parse("slack-bot").expect("valid name"))
                .expect("secret resolves")
                .material_for_action(),
            b"Bearer fixture-token"
        );
        assert_eq!(
            vault
                .resolve(&ResourceName::parse("two-newlines").expect("valid name"))
                .expect("secret resolves")
                .material_for_action(),
            b"value\n"
        );
        assert!(
            vault
                .resolve(&ResourceName::parse("missing").expect("valid name"))
                .is_none()
        );
    }

    #[test]
    fn load_rejects_every_closed_root_and_entry_failure() {
        let root = FixtureDirectory::new();
        std::fs::set_permissions(&root.0, std::fs::Permissions::from_mode(0o755))
            .expect("change root mode");
        assert_eq!(
            SecretVault::load(&root.0).err(),
            Some(SecretVaultError::RootPermissions { mode: 0o755 })
        );

        let not_directory = root.file("not-directory", b"value", 0o600);
        assert_eq!(
            SecretVault::load(&not_directory).err(),
            Some(SecretVaultError::RootNotDirectory)
        );

        std::fs::set_permissions(&root.0, std::fs::Permissions::from_mode(0o700))
            .expect("restore root mode");
        std::fs::remove_file(&not_directory).expect("clear root fixture");
        root.file("open", b"value", 0o644);
        assert_eq!(
            SecretVault::load(&root.0).err(),
            Some(SecretVaultError::EntryPermissions {
                name: "open".into(),
                mode: 0o644,
            })
        );

        std::fs::remove_file(root.0.join("open")).expect("clear permission fixture");
        root.file("empty", b"\n", 0o600);
        assert_eq!(
            SecretVault::load(&root.0).err(),
            Some(SecretVaultError::EmptyValue {
                name: "empty".into()
            })
        );

        std::fs::remove_file(root.0.join("empty")).expect("clear empty fixture");
        root.file("bad\nname", b"value", 0o600);
        assert_eq!(
            SecretVault::load(&root.0).err(),
            Some(SecretVaultError::InvalidName {
                name: "bad\nname".into()
            })
        );

        std::fs::remove_file(root.0.join("bad\nname")).expect("clear invalid-name fixture");
        std::fs::create_dir(root.0.join("nested")).expect("nested entry");
        assert_eq!(
            SecretVault::load(&root.0).err(),
            Some(SecretVaultError::EntryNotRegularFile {
                name: "nested".into()
            })
        );

        std::fs::remove_dir(root.0.join("nested")).expect("clear nested fixture");
        let target = root.file("target", b"value", 0o600);
        symlink(&target, root.0.join("linked")).expect("symlink fixture");
        assert_eq!(
            SecretVault::load(&root.0).err(),
            Some(SecretVaultError::EntrySymlink {
                name: "linked".into()
            })
        );
    }
}
