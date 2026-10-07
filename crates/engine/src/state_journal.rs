
use std::path::{Path, PathBuf};

pub const STATE_JOURNAL_FILE_NAME: &str = "journal.sqlite3";
pub const ARRIVAL_JOURNAL_NAMESPACE: &str = "runtime-arrivals";
pub const AUTHORING_JOURNAL_NAMESPACE: &str = "authoring";
pub const RUN_HISTORY_JOURNAL_NAMESPACE: &str = "run-history";

#[must_use]
pub fn state_journal_path(directory: &Path) -> PathBuf {
    directory.join(STATE_JOURNAL_FILE_NAME)
}

pub fn state_journal_file_bytes(path: &Path) -> Result<u64, String> {
    let rendered = path.as_os_str().to_string_lossy();
    [
        path.to_path_buf(),
        PathBuf::from(format!("{rendered}-wal")),
        PathBuf::from(format!("{rendered}-shm")),
    ]
    .into_iter()
    .try_fold(0_u64, |total, allocation| {
        match std::fs::symlink_metadata(&allocation) {
            Ok(metadata) if metadata.file_type().is_file() => total
                .checked_add(metadata.len())
                .ok_or_else(|| format!("SQLite unit size overflow at {}", path.display())),
            Ok(_) => Err(format!(
                "SQLite allocation is not a regular file: {}",
                allocation.display()
            )),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(total),
            Err(error) => Err(format!(
                "cannot inspect SQLite allocation {}: {error}",
                allocation.display()
            )),
        }
    })
}
