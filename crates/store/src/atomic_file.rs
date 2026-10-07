
use std::ffi::OsString;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use circular_core::{Boundary, Ceilings, CodecError, DuplicateKeyError, Value, decode, encode};
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub enum AtomicFileError {
    Open { source: io::Error },
    Read { source: io::Error },
    Decode { source: CodecError },
    EnvelopeNotObject,
    MissingPayload,
    InvalidChecksum,
    UnknownEnvelopeField(String),
    EncodePayload { source: CodecError },
    ChecksumMismatch,
    BuildEnvelope { source: DuplicateKeyError },
    EncodeEnvelope { source: CodecError },
    CreateTemporary { source: io::Error },
    WriteTemporary { source: io::Error },
    SyncTemporary { source: io::Error },
    Rename { source: io::Error },
    DirectorySync { source: io::Error },
}

impl fmt::Display for AtomicFileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open { source } => write!(formatter, "cannot open checksummed file: {source}"),
            Self::Read { source } => write!(formatter, "cannot read checksummed file: {source}"),
            Self::Decode { source } => {
                write!(formatter, "checksummed file does not decode: {source}")
            }
            Self::EnvelopeNotObject => {
                formatter.write_str("checksummed file envelope is not an object")
            }
            Self::MissingPayload => formatter.write_str("checksummed file has no payload"),
            Self::InvalidChecksum => {
                formatter.write_str("checksummed file has no 32-byte checksum")
            }
            Self::UnknownEnvelopeField(field) => {
                write!(
                    formatter,
                    "checksummed file has unknown envelope field {field:?}"
                )
            }
            Self::EncodePayload { source } => {
                write!(
                    formatter,
                    "checksummed file payload does not encode: {source}"
                )
            }
            Self::ChecksumMismatch => {
                formatter.write_str("checksummed file checksum does not match")
            }
            Self::BuildEnvelope { source } => {
                write!(
                    formatter,
                    "cannot build checksummed file envelope: {source}"
                )
            }
            Self::EncodeEnvelope { source } => {
                write!(
                    formatter,
                    "checksummed file envelope does not encode: {source}"
                )
            }
            Self::CreateTemporary { source } => {
                write!(
                    formatter,
                    "cannot create checksummed temporary file: {source}"
                )
            }
            Self::WriteTemporary { source } => {
                write!(
                    formatter,
                    "cannot write checksummed temporary file: {source}"
                )
            }
            Self::SyncTemporary { source } => {
                write!(
                    formatter,
                    "cannot sync checksummed temporary file: {source}"
                )
            }
            Self::DirectorySync { source } => write!(
                formatter,
                "checksummed file was renamed but directory durability is unconfirmed: {source}"
            ),
            Self::Rename { source } => {
                write!(
                    formatter,
                    "cannot rename checksummed temporary file: {source}"
                )
            }
        }
    }
}

impl std::error::Error for AtomicFileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Open { source }
            | Self::Read { source }
            | Self::CreateTemporary { source }
            | Self::WriteTemporary { source }
            | Self::SyncTemporary { source }
            | Self::Rename { source }
            | Self::DirectorySync { source } => Some(source),
            Self::Decode { source }
            | Self::EncodePayload { source }
            | Self::EncodeEnvelope { source } => Some(source),
            Self::BuildEnvelope { source } => Some(source),
            Self::EnvelopeNotObject
            | Self::MissingPayload
            | Self::InvalidChecksum
            | Self::UnknownEnvelopeField(_)
            | Self::ChecksumMismatch => None,
        }
    }
}

pub struct ChecksummedFile {
    path: PathBuf,
}

impl ChecksummedFile {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Option<Value>, AtomicFileError> {
        let mut file = match File::open(&self.path) {
            Ok(file) => file,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(AtomicFileError::Open { source }),
        };
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|source| AtomicFileError::Read { source })?;
        let outer = decode(&bytes, Ceilings::for_boundary(Boundary::Journal))
            .map_err(|source| AtomicFileError::Decode { source })?;
        let Value::Object(object) = outer else {
            return Err(AtomicFileError::EnvelopeNotObject);
        };
        let mut fields = object.into_map();
        let payload = fields
            .remove("payload")
            .ok_or(AtomicFileError::MissingPayload)?;
        let expected = match fields.remove("sha256") {
            Some(Value::Bytes(bytes)) if bytes.len() == 32 => bytes,
            _ => return Err(AtomicFileError::InvalidChecksum),
        };
        if let Some((unknown, _)) = fields.into_iter().next() {
            return Err(AtomicFileError::UnknownEnvelopeField(unknown));
        }
        let payload_bytes = encode(&payload, Ceilings::for_boundary(Boundary::Journal))
            .map_err(|source| AtomicFileError::EncodePayload { source })?;
        let actual = Sha256::digest(&payload_bytes);
        if actual.as_slice() != expected.as_slice() {
            return Err(AtomicFileError::ChecksumMismatch);
        }
        Ok(Some(payload))
    }

    /// Success confirms both file contents and the renamed directory entry.
    /// DirectorySync means rename completed, but its durability is unconfirmed.
    pub fn publish(&self, payload: &Value) -> Result<(), AtomicFileError> {
        self.publish_with_directory_sync(payload, |path| File::open(path)?.sync_all())
    }

    fn publish_with_directory_sync(
        &self,
        payload: &Value,
        sync_directory: impl FnOnce(&Path) -> io::Result<()>,
    ) -> Result<(), AtomicFileError> {
        let payload_bytes = encode(payload, Ceilings::for_boundary(Boundary::Journal))
            .map_err(|source| AtomicFileError::EncodePayload { source })?;
        let envelope = Value::object([
            ("payload", payload.clone()),
            (
                "sha256",
                Value::Bytes(Sha256::digest(&payload_bytes).to_vec()),
            ),
        ])
        .map_err(|source| AtomicFileError::BuildEnvelope { source })?;
        let bytes = encode(&envelope, Ceilings::for_boundary(Boundary::Journal))
            .map_err(|source| AtomicFileError::EncodeEnvelope { source })?;

        let temporary = self.temporary_path();
        let before_rename = (|| -> Result<(), AtomicFileError> {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            let mut file = options
                .open(&temporary)
                .map_err(|source| AtomicFileError::CreateTemporary { source })?;
            file.write_all(&bytes)
                .map_err(|source| AtomicFileError::WriteTemporary { source })?;
            file.sync_all()
                .map_err(|source| AtomicFileError::SyncTemporary { source })?;
            std::fs::rename(&temporary, &self.path)
                .map_err(|source| AtomicFileError::Rename { source })?;
            Ok(())
        })();
        if let Err(error) = before_rename {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }

        sync_directory(self.directory()).map_err(|source| AtomicFileError::DirectorySync { source })
    }

    fn directory(&self) -> &Path {
        self.path
            .parent()
            .filter(|directory| !directory.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
    }

    fn temporary_path(&self) -> PathBuf {
        let mut name = OsString::from(".");
        name.push(
            self.path
                .file_name()
                .unwrap_or_else(|| self.path.as_os_str()),
        );
        name.push(format!(".{}.tmp", std::process::id()));
        self.path.with_file_name(name)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    const FIXED_ENVELOPE: &[u8] = &[
        0x08, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x07, 0x70, 0x61, 0x79, 0x6c, 0x6f, 0x61,
        0x64, 0x06, 0x00, 0x00, 0x00, 0x03, 0x01, 0x02, 0x03, 0x00, 0x00, 0x00, 0x06, 0x73, 0x68,
        0x61, 0x32, 0x35, 0x36, 0x06, 0x00, 0x00, 0x00, 0x20, 0x95, 0xc2, 0xa0, 0x88, 0x53, 0xf8,
        0x1e, 0x5c, 0xf7, 0x3c, 0x64, 0xd6, 0x22, 0xab, 0xba, 0x01, 0x25, 0xd5, 0xba, 0xba, 0xec,
        0x4b, 0x6c, 0x73, 0xb8, 0x0c, 0xc6, 0x16, 0xca, 0xab, 0x06, 0x4c,
    ];

    struct TestDirectory(circular_testkit::temp::StateDir);

    impl TestDirectory {
        fn new() -> Self {
            Self(circular_testkit::temp::StateDir::new(
                "circular-checksummed-file",
            ))
        }

        fn file(&self) -> ChecksummedFile {
            ChecksummedFile::new(self.0.path().join("state.value"))
        }
    }

    #[test]
    fn fixed_payload_envelope_bytes_do_not_change() {
        let directory = TestDirectory::new();
        let file = directory.file();
        let payload = Value::Bytes(vec![1, 2, 3]);

        fs::write(file.path(), FIXED_ENVELOPE).expect("old-format fixture");
        assert_eq!(file.load().expect("fixture loads"), Some(payload.clone()));

        fs::remove_file(file.path()).expect("replace fixture through publisher");
        file.publish(&payload).expect("publish");
        assert_eq!(
            fs::read(file.path()).expect("published bytes"),
            FIXED_ENVELOPE
        );
    }

    #[test]
    fn missing_file_is_absent_even_when_torn_temporary_exists() {
        let directory = TestDirectory::new();
        let file = directory.file();
        let temporary = file.temporary_path();
        fs::write(&temporary, b"torn").expect("torn temporary");

        assert_eq!(file.load().expect("missing committed file"), None);

        fs::write(file.path(), FIXED_ENVELOPE).expect("committed file");
        assert_eq!(
            file.load().expect("committed file wins"),
            Some(Value::Bytes(vec![1, 2, 3]))
        );
        assert!(temporary.exists());
    }

    #[test]
    fn directory_sync_failure_returns_error_after_visible_rename() {
        let directory = TestDirectory::new();
        let file = directory.file();
        file.publish(&Value::Int(1)).unwrap();
        let error = file
            .publish_with_directory_sync(&Value::Int(2), |path| {
                assert_eq!(path, directory.0.path());
                assert_eq!(file.load().unwrap(), Some(Value::Int(2)));
                Err(io::Error::other("injected directory sync failure"))
            })
            .unwrap_err();
        assert!(matches!(error, AtomicFileError::DirectorySync { .. }));
        assert!(
            error
                .to_string()
                .contains("was renamed but directory durability is unconfirmed")
        );
        assert_eq!(file.load().unwrap(), Some(Value::Int(2)));
        assert!(!file.temporary_path().exists());
    }

    #[test]
    fn corrupt_file_is_rejected() {
        let directory = TestDirectory::new();
        let file = directory.file();
        fs::write(file.path(), [0]).expect("corrupt file");

        assert!(matches!(
            file.load(),
            Err(AtomicFileError::Decode {
                source: CodecError::UnknownTag(0)
            })
        ));
    }

    #[test]
    fn checksum_mismatch_is_rejected() {
        let directory = TestDirectory::new();
        let file = directory.file();
        let mut bytes = FIXED_ENVELOPE.to_vec();
        bytes[23] ^= 1;
        fs::write(file.path(), bytes).expect("mismatched file");

        assert!(matches!(
            file.load(),
            Err(AtomicFileError::ChecksumMismatch)
        ));
    }
}
