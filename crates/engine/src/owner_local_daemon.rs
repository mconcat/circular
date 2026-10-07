//! The product daemon's startup path: argv → accepted state location → claim.
//!
//! Opening a daemon is an ordered procedure, and this module owns the
//! first two steps that are reachable today — choosing the state location and
//! acquiring its exclusive claim.  Opening the store, starting the server run,
//! and serving sessions over the claimed endpoint remain with their owners; the
//! session codec they need is not published yet.
//!
//! **What this module refuses to do is as fixed as what it does.**  It reads no
//! environment name for the state location, invents no endpoint address, and
//! accepts no credential identity that an accepted activation did not issue.
//! The last one is why credential bootstrap is absent rather than stubbed: a
//! login-Keychain reference can only come from
//! activation-sealed credential issuer, and no deployment
//! declaration exists yet to produce that activation.  Manufacturing one here
//! would forge the custody seal that design deliberately makes unforgeable.
//!
//! ## Order matters
//!
//! Nothing touches the filesystem until the argv shape is known good, and
//! nothing claims the endpoint until the state directory is known to be this
//! user's.  A daemon that claimed first and validated second would leave a lock
//! behind on every malformed invocation.

use circular_transport::{
    AcceptedStateDirectory, DaemonArguments, OWNER_LOCAL_SOCKET_NAME, OwnerLocalClaim,
    OwnerLocalListener, OwnerLocalSocketError, OwnerLocalSocketSpec, StateDirectoryRejection,
};
use std::fmt;
use std::path::Path;

/// A daemon instance holding the exclusive claim on one state location.
///
/// The claim lives exactly as long as this value.  There is no way to obtain
/// the listener without the accepted directory that justified it, so a claim
/// on an unvalidated location is not constructible.
#[derive(Debug)]
pub struct ClaimedOwnerLocalDaemon {
    state_directory: AcceptedStateDirectory,
    claim: OwnerLocalClaim,
}

impl ClaimedOwnerLocalDaemon {
    /// Runs the reachable prefix of the opening procedure: choose the state
    /// location, then acquire its claim.
    ///
    /// `home` and `effective_user` are supplied by the caller rather than read
    /// here, so this function has no ambient inputs.  The binary supplies the
    /// process's own effective UID and the user's home; a home the caller does
    /// not own cannot widen the check, because the acceptance predicates below
    /// still require the state directory itself to be owner-only and owned by
    /// that same UID.
    pub fn claim(
        arguments: &DaemonArguments,
        home: &Path,
        effective_user: u32,
    ) -> Result<Self, DaemonStartError> {
        let state_directory = arguments
            .accept_state_directory(home, effective_user)
            .map_err(DaemonStartError::StateDirectory)?;
        let spec = OwnerLocalSocketSpec::try_new(state_directory.path(), OWNER_LOCAL_SOCKET_NAME)
            .map_err(DaemonStartError::Endpoint)?;
        let claim = OwnerLocalClaim::claim(spec).map_err(DaemonStartError::Endpoint)?;
        Ok(Self {
            state_directory,
            claim,
        })
    }

    #[must_use]
    pub fn state_directory(&self) -> &Path {
        self.state_directory.path()
    }

    #[must_use]
    pub fn socket_path(&self) -> std::path::PathBuf {
        self.claim.spec().socket_path()
    }

    /// Publishes the owner-local endpoint after product recovery has finished.
    pub fn open_endpoint(self) -> Result<OwnerLocalListener, OwnerLocalSocketError> {
        self.claim.open_endpoint()
    }

    /// Releases a claim that has not published an endpoint.
    pub fn release(self) {
        drop(self);
    }
}

/// Every way the reachable startup prefix can fail, in the order it is checked.
#[derive(Debug)]
pub enum DaemonStartError {
    StateDirectory(StateDirectoryRejection),
    Endpoint(OwnerLocalSocketError),
}

impl fmt::Display for DaemonStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StateDirectory(rejection) => {
                write!(formatter, "state location refused: {rejection}")
            }
            Self::Endpoint(error) => write!(formatter, "endpoint claim failed: {error}"),
        }
    }
}

impl std::error::Error for DaemonStartError {}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_transport::current_effective_user;
    use std::ffi::OsString;
    use std::fs::DirBuilder;
    use std::os::unix::fs::DirBuilderExt;
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT: AtomicU32 = AtomicU32::new(0);

    struct Home(std::path::PathBuf);

    impl Home {
        fn new() -> Option<Self> {
            if current_effective_user() == 0 {
                return None;
            }
            let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
            let home = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target")
                .canonicalize()
                .expect("workspace target")
                .join(format!("l{:x}-{sequence}", std::process::id()));
            DirBuilder::new().mode(0o700).create(&home).ok()?;
            Some(Self(home))
        }

        fn state(&self, name: &str) -> std::path::PathBuf {
            let path = self.0.join(name);
            DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .expect("state directory");
            path
        }
    }

    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Separates "the sandbox forbids AF_UNIX" from "the claim failed", so the
    /// second one fails the test instead of disappearing into a skip.
    fn af_unix_available(root: &Path) -> bool {
        let probe = root.join("p.sock");
        let _ = std::fs::remove_file(&probe);
        let bound = std::os::unix::net::UnixListener::bind(&probe).is_ok();
        let _ = std::fs::remove_file(&probe);
        bound
    }

    fn arguments(path: &Path) -> DaemonArguments {
        DaemonArguments::parse([OsString::from("--state"), path.as_os_str().to_owned()])
            .expect("absolute state path")
    }

    #[test]
    fn claiming_reserves_the_one_socket_name_without_binding_it() {
        let Some(home) = Home::new() else { return };
        let state = home.state("primary");
        let daemon =
            ClaimedOwnerLocalDaemon::claim(&arguments(&state), &home.0, current_effective_user())
                .expect("claim accepted state");

        let socket = std::fs::canonicalize(&state)
            .expect("canonical state")
            .join(OWNER_LOCAL_SOCKET_NAME);
        assert_eq!(daemon.socket_path(), socket);
        assert!(
            !socket.exists(),
            "claim acquisition must not publish the endpoint before recovery"
        );

        let Ok(listener) = daemon.open_endpoint() else {
            assert!(
                !af_unix_available(&state),
                "AF_UNIX binds here, so a failed endpoint open is a product failure"
            );
            eprintln!("SKIP: this environment forbids AF_UNIX bind");
            return;
        };
        assert!(socket.exists());
        listener.close().expect("release");
    }

    #[test]
    fn a_second_daemon_on_the_same_location_loses_the_claim() {
        let Some(home) = Home::new() else { return };
        let state = home.state("primary");
        let first =
            ClaimedOwnerLocalDaemon::claim(&arguments(&state), &home.0, current_effective_user())
                .expect("first claim");

        let second =
            ClaimedOwnerLocalDaemon::claim(&arguments(&state), &home.0, current_effective_user());
        assert!(
            matches!(second, Err(DaemonStartError::Endpoint(_))),
            "the claim is exclusive while the first daemon holds it"
        );
        first.release();
    }

    #[test]
    fn a_refused_state_location_never_reaches_the_claim() {
        let Some(home) = Home::new() else { return };
        let state = home.state("primary");
        std::fs::set_permissions(&state, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .expect("widen permissions");

        let refused =
            ClaimedOwnerLocalDaemon::claim(&arguments(&state), &home.0, current_effective_user());
        assert!(matches!(
            refused,
            Err(DaemonStartError::StateDirectory(
                StateDirectoryRejection::NotOwnerOnly { .. }
            ))
        ));
        assert!(
            !state.join(OWNER_LOCAL_SOCKET_NAME).exists()
                && !state
                    .join(format!("{OWNER_LOCAL_SOCKET_NAME}.owner-local.lock"))
                    .exists(),
            "a refused location leaves no socket and no claim file behind"
        );
    }
}
