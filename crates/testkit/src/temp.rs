
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ORDINAL: AtomicU64 = AtomicU64::new(0);

fn process_nonce() -> u32 {
    static NONCE: OnceLock<u32> = OnceLock::new();
    *NONCE.get_or_init(|| {
        use std::hash::{BuildHasher, Hasher};
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u32(std::process::id());
        let wide = hasher.finish();
        (wide ^ (wide >> 32)) as u32
    })
}

#[derive(Debug)]
pub struct StateDir {
    path: PathBuf,
    parent: Option<PathBuf>,
}

impl StateDir {
    #[must_use]
    pub fn new(label: &str) -> Self {
        Self::create(std::env::temp_dir(), label)
    }

    #[must_use]
    pub fn under_home(label: &str) -> Self {
        let home = std::env::home_dir().expect("this user has a home directory");
        let home = home
            .canonicalize()
            .unwrap_or_else(|error| panic!("home {}: {error}", home.display()));
        Self::create(home, &format!(".circular-testkit-{label}"))
    }

    #[must_use]
    pub fn lent(path: PathBuf) -> Self {
        Self { path, parent: None }
    }

    fn create(parent: PathBuf, label: &str) -> Self {
        let ordinal = NEXT_ORDINAL.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            "{label}-{}-{:08x}-{ordinal}",
            std::process::id(),
            process_nonce()
        ));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
            .create(&path)
            .unwrap_or_else(|error| panic!("state directory {}: {error}", path.display()));
        Self {
            path,
            parent: Some(parent),
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        if self.parent.is_some() && self.path.parent() == self.parent.as_deref() {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StateDir;

    #[test]
    fn a_lent_root_leaves_the_directory_to_its_owner() {
        let owner = StateDir::new("circular-testkit-lent");
        drop(StateDir::lent(owner.path().to_path_buf()));
        assert!(
            owner.path().exists(),
            "the borrowed root deleted its owner's directory"
        );
    }

    #[test]
    fn a_root_removes_its_directory_when_it_ends() {
        let path = {
            let root = StateDir::new("circular-testkit-drop");
            std::fs::write(root.path().join("state"), b"x").expect("root is writable");
            root.path().to_path_buf()
        };
        assert!(!path.exists(), "the root left its own directory behind");
    }
}
