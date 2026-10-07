#![deny(unsafe_code)]

extern crate self as engine;
#[forbid(unsafe_code)]
mod engine_modules;
pub use engine_modules::*;

#[path = "bin/circular-daemon/daemon.rs"]
mod daemon;

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod closed_tables;

/// Run the owner-local daemon's existing command-line entry point.
/// The daemon's assembly and replay modules remain private to this crate.
pub fn run_owner_local_daemon() -> std::process::ExitCode {
    daemon::run()
}

