//! The owner-local daemon executable; implementation is owned once by the engine crate.
fn main() -> std::process::ExitCode {
    engine::run_owner_local_daemon()
}
