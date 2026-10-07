
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

static SHUTDOWN_WAKE_FD: AtomicI32 = AtomicI32::new(-1);

#[derive(Clone, Copy, Debug)]
pub(crate) struct Shutdown;

impl Shutdown {
    pub(crate) fn install() -> Result<Self, String> {
        SHUTDOWN_REQUESTED.store(false, Ordering::SeqCst);
        platform::install()?;
        Ok(Self)
    }

    fn body(self) -> circular_store::ProductShutdownBody {
        circular_store::ProductShutdownBody {
            reason: if self.requested() {
                circular_store::RestartReason::Signal
            } else {
                circular_store::RestartReason::Normal
            },
        }
    }

    /// All exits retain System until its final fact has committed.
    pub(crate) fn persist_system(
        self,
        system: &crate::daemon::ledger::SystemRuntime,
        revision: circular_core::RevisionEpochId,
    ) -> Result<(), String> {
        system.pipeline.shutdown(revision, self.body())
    }

    /// The standing run records its own shutdown at the revision it stands at.
    pub(crate) fn persist_run(
        self,
        mut standing: crate::daemon::ledger::ServerRun,
    ) -> Result<(), String> {
        let body = self.body();
        let reason = body.reason.as_str();
        standing.persist_shutdown(body)?;
        eprintln!("circular-daemon: shutdown body committed: reason={reason}");
        Ok(())
    }

    pub(crate) fn requested(self) -> bool {
        requested()
    }

    pub(crate) fn wake_on_request(self, wake: &engine::wake::Wake) {
        use std::os::fd::AsRawFd;
        SHUTDOWN_WAKE_FD.store(wake.notify_fd().as_raw_fd(), Ordering::SeqCst);
        if self.requested() {
            wake.notify();
        }
    }
}

pub(crate) fn requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::SeqCst)
}

#[allow(unsafe_code)]
extern "C" fn request_shutdown(_signal: std::ffi::c_int) {
    SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
    let wake = SHUTDOWN_WAKE_FD.load(Ordering::SeqCst);
    if wake >= 0 {
        let byte = 0_u8;
        unsafe {
            nix::libc::write(wake, std::ptr::from_ref(&byte).cast(), 1);
        }
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
mod platform {
    use std::ffi::c_int;
    use std::io;

    use super::request_shutdown;

    const SIGINT: c_int = 2;
    const SIGTERM: c_int = 15;
    const SIG_ERR: usize = usize::MAX;

    unsafe extern "C" {
        fn signal(signal: c_int, handler: usize) -> usize;
    }

    pub(super) fn install() -> Result<(), String> {
        let handler = request_shutdown as *const () as usize;
        let interrupt = unsafe { signal(SIGINT, handler) };
        if interrupt == SIG_ERR {
            return Err(format!(
                "cannot install SIGINT shutdown handler: {}",
                io::Error::last_os_error()
            ));
        }
        let terminate = unsafe { signal(SIGTERM, handler) };
        if terminate == SIG_ERR {
            return Err(format!(
                "cannot install SIGTERM shutdown handler: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(())
    }
}

#[cfg(not(unix))]
mod platform {
    pub(super) fn install() -> Result<(), String> {
        Err("graceful shutdown is not implemented on this platform".to_owned())
    }
}

