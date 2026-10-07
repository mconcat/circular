
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ORDINAL: AtomicU64 = AtomicU64::new(0);

fn process_nonce() -> u64 {
    static NONCE: OnceLock<u64> = OnceLock::new();
    *NONCE.get_or_init(|| {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).expect("unique temporary directory entropy");
        u64::from_le_bytes(bytes)
    })
}

pub(crate) fn unique_temp_path(label: &str) -> PathBuf {
    let ordinal = NEXT_ORDINAL.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "{label}-{}-{:016x}-{ordinal}",
        std::process::id(),
        process_nonce()
    ))
}

#[derive(Debug)]
pub(crate) struct UniqueTempDir {
    path: PathBuf,
}

impl UniqueTempDir {
    pub(crate) fn try_new(label: &str) -> std::io::Result<Self> {
        let path = unique_temp_path(label);
        std::fs::create_dir(&path)?;
        Ok(Self { path })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn new(label: &str) -> Self {
        Self::try_new(label)
            .unwrap_or_else(|error| panic!("temporary directory for {label}: {error}"))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for UniqueTempDir {
    fn drop(&mut self) {
        if self.path.starts_with(std::env::temp_dir()) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{UniqueTempDir, unique_temp_path};

    #[test]
    fn a_root_removes_its_directory_when_it_ends() {
        let path = {
            let root = UniqueTempDir::new("circular-temp-drop");
            std::fs::write(root.path().join("state"), b"x").expect("root is writable");
            root.path().to_path_buf()
        };
        assert!(!path.exists(), "the root left its own directory behind");
    }

    #[test]
    fn two_roots_under_one_label_never_share_a_directory() {
        let first = UniqueTempDir::new("circular-temp-unique");
        let second = UniqueTempDir::new("circular-temp-unique");
        assert_ne!(first.path(), second.path());
    }
}
