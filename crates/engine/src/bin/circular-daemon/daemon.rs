
mod actor_access;
mod actor_catalog;
mod approval;
mod authoring_store;
mod daemon_health;
mod ledger;
mod query_catalog;
#[cfg(any(target_vendor = "apple", target_os = "linux"))]
mod read_world;
mod record_rows;
mod restart_query;
mod run_control_store;
mod runtime_arrival_retention;
#[path = "../shared/shutdown.rs"]
mod shutdown;
mod subscription_catalog;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use query_catalog::wire_rows as query_wire_rows;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use subscription_catalog::wire_rows as subscription_wire_rows;
 mod unregistered;
mod webhook;

use engine::authoring_assembly::ledger as authoring;

#[cfg(any(target_vendor = "apple", target_os = "linux"))]
mod declaration;
#[cfg(any(target_vendor = "apple", target_os = "linux"))]
mod environment;
#[cfg(any(target_vendor = "apple", target_os = "linux"))]
mod injection;
#[cfg(any(target_vendor = "apple", target_os = "linux"))]
mod query;
#[cfg(any(target_vendor = "apple", target_os = "linux"))]
mod replay;
mod replay_clock;
#[cfg(any(target_vendor = "apple", target_os = "linux"))]
mod run_control;
#[cfg(any(target_vendor = "apple", target_os = "linux"))]
mod session;
mod session_policy;
#[cfg(any(target_vendor = "apple", target_os = "linux"))]
mod subscription;
const PROGRAM: &str = "circular-daemon";

fn usage() -> String {
    format!(
        "{}\nLaunchAgents: circular daemon <install|uninstall> --state <directory>\n",
        circular_transport::daemon_usage(PROGRAM, "Circular owner-local engine")
    )
}

pub(super) fn run() -> std::process::ExitCode {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if matches!(
        arguments.first().and_then(|arg| arg.to_str()),
        Some("install" | "uninstall")
    ) {
        return circular_cli::daemon_registration::run(arguments);
    }
    match circular_transport::OwnerLocalInvocation::parse(arguments) {
        circular_transport::OwnerLocalInvocation::Help => {
            print!("{}", usage());
            std::process::ExitCode::SUCCESS
        }
        circular_transport::OwnerLocalInvocation::Version => {
            print!("{}", circular_transport::owner_local_version(PROGRAM));
            std::process::ExitCode::SUCCESS
        }
        circular_transport::OwnerLocalInvocation::Arguments(arguments) => platform::run(arguments),
    }
}

#[cfg(any(target_vendor = "apple", target_os = "linux"))]
mod platform {
    use crate::daemon::declaration::NEXT_EPOCH;
    use crate::daemon::environment::{
        daemon_config_from_state, product_execution_profile, secret_vault_from_state,
        webhook_gateway_from_config,
    };
    use crate::daemon::session::serve_session;
    use crate::daemon::shutdown::Shutdown;
    use crate::daemon::{authoring_store, ledger, webhook};
    use circular_transport::{
        DaemonArguments, DaemonArgumentsError, OwnerLocalSocketError, StateDirectoryRejection,
        current_effective_user,
    };
    use engine::authoring_assembly::ledger as authoring;
    use engine::execution_profile::ProductExecutionProfile;
    use engine::{ClaimedOwnerLocalDaemon, DaemonStartError};
    use std::path::PathBuf;
    use std::process::ExitCode;

    mod exit {
        pub const NONE: u8 = 0;
        pub const FIX_REQUEST: u8 = 2;
        pub const CHOOSE_LOCATION: u8 = 3;
        pub const WAIT: u8 = 4;
    }

    pub fn run(raw_arguments: Vec<std::ffi::OsString>) -> ExitCode {
        let arguments = match DaemonArguments::parse(raw_arguments) {
            Ok(arguments) => arguments,
            Err(error) => return fail(exit::FIX_REQUEST, &argument_diagnostic(&error)),
        };

        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return fail(
                exit::CHOOSE_LOCATION,
                "this user's home is not named in the environment, so no state \
                 location can be accepted",
            );
        };

        let shutdown = match Shutdown::install() {
            Ok(shutdown) => shutdown,
            Err(error) => return fail(exit::CHOOSE_LOCATION, &error),
        };

        let daemon =
            match ClaimedOwnerLocalDaemon::claim(&arguments, &home, current_effective_user()) {
                Ok(daemon) => daemon,
                Err(error) => return fail(start_exit_code(&error), &error.to_string()),
            };

        eprintln!(
            "circular-daemon: build {}",
            circular_transport::build_identity(crate::daemon::PROGRAM)
        );
        eprintln!(
            "circular-daemon: claimed {} at {}",
            daemon.state_directory().display(),
            daemon.socket_path().display()
        );

        let config = match daemon_config_from_state(&daemon) {
            Ok(config) => config,
            Err(error) => {
                daemon.release();
                return fail(exit::CHOOSE_LOCATION, &error);
            }
        };
        for default in &config.defaults_used {
            eprintln!("circular-daemon: config default used: {default}");
        }
        if config.effect_retry_defaulted {
            eprintln!(
                "circular-daemon: config default used: effects.retry_delays={:?}",
                config.effect_retry.ms()
            );
        }
        let secret_vault = match secret_vault_from_state(&daemon, &config) {
            Ok(secret_vault) => secret_vault,
            Err(error) => {
                daemon.release();
                return fail(exit::CHOOSE_LOCATION, &error);
            }
        };
        let execution = match product_execution_profile(
            &daemon,
            &config,
            secret_vault.clone(),
            arguments.reference_agent(),
        ) {
            Ok(execution) => execution,
            Err(error) => {
                daemon.release();
                return fail(exit::CHOOSE_LOCATION, &error);
            }
        };
        let webhook = match webhook_gateway_from_config(&config, secret_vault) {
            Ok(gateway) => gateway,
            Err(error) => {
                daemon.release();
                return fail(exit::CHOOSE_LOCATION, &error);
            }
        };
        match serve(
            daemon,
            execution,
            webhook,
            config.runtime_arrival_limits,
            shutdown,
        ) {
            Ok(()) => ExitCode::from(exit::NONE),
            Err(error) => fail(exit::CHOOSE_LOCATION, &error),
        }
    }

    pub(crate) struct World {
        pub(crate) server: Option<ledger::ServerRun>,
        pub(crate) authoring: authoring::AuthoringState,
        pub(crate) system: Option<std::sync::Arc<ledger::SystemRuntime>>,
        pub(crate) state_directory: PathBuf,
        pub(crate) runtime_arrival_limits:
            crate::daemon::runtime_arrival_retention::RuntimeArrivalLimits,
    }

    fn serve(
        daemon: ClaimedOwnerLocalDaemon,
        execution: ProductExecutionProfile,
        webhook: Option<webhook::WebhookGateway>,
        runtime_arrival_limits: crate::daemon::runtime_arrival_retention::RuntimeArrivalLimits,
        shutdown: Shutdown,
    ) -> Result<(), String> {
        for (vocabulary, name) in crate::daemon::unregistered::ALL {
            eprintln!(
                "circular-daemon: {vocabulary} {name:?} resolves without a registration                  vocabulary — provisional"
            );
        }
        let booted = match boot_recover(
            daemon.state_directory(),
            &execution,
            crate::daemon::shutdown::requested,
        )? {
            BootOutcome::Stood(booted) => booted,
            BootOutcome::Interrupted {
                system, revision, ..
            } => {
                eprintln!("circular-daemon: shutdown signal observed during recovery");
                shutdown.persist_system(&system, revision)?;
                return Ok(());
            }
        };
        let owners = stand_owners(booted, daemon.state_directory(), runtime_arrival_limits)?;

        let daemon = daemon
            .open_endpoint()
            .map_err(|error| format!("endpoint open failed after recovery: {error}"))?;

        let webhook_input = owners.world.start_input_owners(webhook)?;
        daemon
            .set_nonblocking(true)
            .map_err(|error| format!("could not bound the accept wait: {error}"))?;
        let wake = owners.descriptors_returned.clone();
        shutdown.wake_on_request(&wake);
        eprintln!("circular-daemon: serving");
        while !shutdown.requested() {
            match accept_once(&daemon, &owners) {
                AcceptStep::Handed | AcceptStep::Refused => {}
                AcceptStep::NoPeer => {
                    wake.drain();
                    if shutdown.requested() {
                        break;
                    }
                    use std::os::fd::AsFd;
                    if let Err(error) = engine::wake::wait_readable(&[daemon.as_fd(), wake.as_fd()])
                    {
                        return Err(format!("accept wait failed: {error}"));
                    }
                }
                AcceptStep::Exhausted(reason) => {
                    eprintln!("circular-daemon: accept waits for returned descriptors: {reason}");
                    wake.drain();
                    if shutdown.requested() {
                        break;
                    }
                    if let Err(error) = engine::wake::wait_readable(&[wake.as_fd()]) {
                        return Err(format!("accept wait failed: {error}"));
                    }
                }
                AcceptStep::EndpointStopped(reason) => {
                    eprintln!("circular-daemon: endpoint stopped accepting: {reason}");
                    return daemon
                        .close()
                        .map_err(|close| format!("endpoint cleanup failed: {close}"));
                }
            }
        }

        eprintln!("circular-daemon: shutdown signal observed by accept loop");
        drop(webhook_input);
        persist_standing_run(&owners.world, shutdown)?;
        daemon
            .close()
            .map_err(|error| format!("endpoint cleanup failed during shutdown: {error}"))
    }

    pub(crate) fn stand_owners(
        booted: Booted,
        state_directory: &std::path::Path,
        runtime_arrival_limits: crate::daemon::runtime_arrival_retention::RuntimeArrivalLimits,
    ) -> Result<SessionOwners, String> {
        let Booted {
            authoring_store,
            authoring,
            system,
            server,
            restored_prefix,
            execution,
        } = booted;

        if let Err(error) =
            crate::daemon::runtime_arrival_retention::startup_warnings(state_directory)
        {
            eprintln!("circular-daemon: runtime arrival inventory warning failed: {error}");
        }

        let world = std::sync::Arc::new(
            crate::daemon::read_world::SharedWorld::with_restored_prefix(
                World {
                    server,
                    authoring,
                    system,
                    state_directory: state_directory.to_path_buf(),
                    runtime_arrival_limits,
                },
                restored_prefix,
            ),
        );
        let sessions = std::sync::Arc::downgrade(&world);
        execution.set_publication_wake(std::sync::Arc::new(move || {
            if let Some(world) = sessions.upgrade() {
                world.wake_sessions();
            }
        }));
        SessionOwners::new(world, authoring_store, execution)
    }

    pub(crate) fn persist_standing_run(
        world: &crate::daemon::read_world::SharedWorld,
        shutdown: Shutdown,
    ) -> Result<(), String> {
        let gate = world.commit_gate()?;
        let (standing, authoring_cursor, system) = {
            let mut world = world.run_write().expect("run projection owner poisoned");
            (
                world.server.take(),
                world.world_authoring_cursor(),
                world.system.clone(),
            )
        };
        drop(gate);
        if let Some(standing) = standing {
            shutdown.persist_run(standing)?;
        } else if let Some(system) = system {
            let revision = engine::RevisionEpochId::new(authoring_cursor)
                .ok_or("shutdown revision is unavailable")?;
            shutdown.persist_system(&system, revision)?;
        }
        Ok(())
    }

    pub(crate) struct Booted {
        pub(crate) authoring_store: authoring_store::AuthoringStore,
        pub(crate) authoring: authoring::AuthoringState,
        pub(crate) system: Option<std::sync::Arc<ledger::SystemRuntime>>,
        pub(crate) server: Option<ledger::ServerRun>,
        pub(crate) restored_prefix: Option<ledger::ServerRead>,
        pub(crate) execution: ProductExecutionProfile,
    }

    pub(crate) enum BootOutcome {
        Stood(Booted),
        Interrupted {
            system: std::sync::Arc<ledger::SystemRuntime>,
            revision: circular_core::RevisionEpochId,
        },
    }

    pub(crate) fn boot_recover(
        state_directory: &std::path::Path,
        base: &ProductExecutionProfile,
        interrupted: fn() -> bool,
    ) -> Result<BootOutcome, String> {
        let mut server: Option<ledger::ServerRun> = None;
        let authoring_store = authoring_store::AuthoringStore::open(state_directory)
            .map_err(|error| error.startup_diagnostic())?;
        let path = engine::state_journal::state_journal_path(state_directory);
        if circular_store::SqliteJournal::namespace_payload_bytes(
            &path,
            engine::state_journal::RUN_HISTORY_JOURNAL_NAMESPACE,
        )
        .map_err(|error| error.to_string())?
        .is_some()
        {
            return Err("refused retired run-history records; states written by earlier builds are not read".into());
        }
        let authoring = authoring_store
            .load()
            .map_err(|error| format!("refused corrupt authoring state: {error}"))?;
        eprintln!("circular-daemon: restored committed authoring state");
        let execution = base.clone();
        NEXT_EPOCH.fetch_max(
            authoring.next_epoch_number(),
            std::sync::atomic::Ordering::Relaxed,
        );
        let manifest = engine::state_manifest::read_state_manifest(&path)?;
        let mut system = None;
        let restored_prefix = None;
        if manifest.is_some() {
            let stream = crate::daemon::run_control_store::state_stream(Some(state_directory))?;
            let revision = circular_core::RevisionEpochId::new(authoring.cursor())
                .ok_or("restart authoring revision is unavailable")?;
            let restart = engine::restart_journal::RestartBoot::capture_for(execution.boot_id())?;
            let (owner, origin) =
                ledger::SystemRuntime::reopen(state_directory, stream, &execution)?;
            let recovered = restart.read_after(state_directory, 0)?;
            owner.pipeline.record_facts(
                revision,
                vec![crate::kernel::system::DaemonFact::Restart {
                    origin,
                    body: recovered.body.clone(),
                }],
            )?;
            match ledger::recover_at_boot(
                state_directory,
                &execution,
                &owner,
                ledger::RecoveryInterrupt::new(interrupted),
            ) {
                Ok(restored) => server = restored,
                Err(reason) if interrupted() => {
                    eprintln!("circular-daemon: recovery stopped: {reason}");
                    return Ok(BootOutcome::Interrupted {
                        system: owner,
                        revision,
                    });
                }
                Err(reason) => eprintln!("circular-daemon: recorded recovery failure: {reason}"),
            }
            system = Some(owner);
        } else {
            let restart = authoring
                .revision()
                .map(|_| {
                    engine::restart_journal::RestartBoot::capture_for(execution.boot_id())?
                        .read_after(state_directory, 0)
                        .map(|recovered| recovered.body)
                })
                .transpose()?;
            server = crate::daemon::declaration::stand_first_activation(
                &authoring,
                &execution,
                state_directory,
                &mut system,
                restart,
            )?;
        }
        Ok(BootOutcome::Stood(Booted {
            authoring_store,
            authoring,
            system,
            server,
            restored_prefix,
            execution,
        }))
    }

    #[derive(Clone)]
    pub(crate) struct SessionOwners {
        pub(crate) descriptors_returned: std::sync::Arc<engine::wake::Wake>,
        pub(crate) world: std::sync::Arc<crate::daemon::read_world::SharedWorld>,
        pub(crate) authoring_store: std::sync::Arc<authoring_store::AuthoringStore>,
        pub(crate) execution: std::sync::Arc<ProductExecutionProfile>,
        pub(crate) session_registry:
            std::sync::Arc<std::sync::Mutex<crate::daemon::session_policy::DaemonSessionRegistry>>,
        pub(crate) time: std::sync::Arc<crate::daemon::replay_clock::SystemTime>,
    }

    impl SessionOwners {
        pub(crate) fn new(
            world: std::sync::Arc<crate::daemon::read_world::SharedWorld>,
            authoring_store: authoring_store::AuthoringStore,
            execution: ProductExecutionProfile,
        ) -> Result<Self, String> {
            Ok(Self {
                descriptors_returned: std::sync::Arc::new(
                    engine::wake::Wake::new()
                        .map_err(|error| format!("could not open the accept wake: {error}"))?,
                ),
                world,
                authoring_store: std::sync::Arc::new(authoring_store),
                execution: std::sync::Arc::new(execution),
                session_registry: std::sync::Arc::new(std::sync::Mutex::new(
                    crate::daemon::session_policy::DaemonSessionRegistry::default(),
                )),
                time: crate::daemon::replay_clock::SystemTime::stand()
                    .map_err(|error| format!("could not stand the system time actor: {error}"))?,
            })
        }
    }

    pub(crate) enum AcceptStep {
        Handed,
        NoPeer,
        Refused,
        Exhausted(String),
        EndpointStopped(String),
    }

    fn accept_is_transient(source: &std::io::Error) -> bool {
        matches!(
            source.kind(),
            std::io::ErrorKind::Interrupted | std::io::ErrorKind::ConnectionAborted
        ) || matches!(
            source.raw_os_error(),
            Some(nix::libc::EMFILE | nix::libc::ENFILE | nix::libc::ENOBUFS)
        )
    }

    pub(crate) fn accept_once(
        daemon: &circular_transport::OwnerLocalListener,
        owners: &SessionOwners,
    ) -> AcceptStep {
        match daemon.accept() {
            Ok(stream) => {
                if let Err(error) = stream.set_nonblocking(false) {
                    eprintln!("circular-daemon: could not restore blocking reads: {error}");
                    return AcceptStep::Refused;
                }
                let owners = owners.clone();
                match std::thread::Builder::new()
                    .name("owner-session".to_owned())
                    .spawn(move || {
                        let mut stream = stream;
                        serve_session(&mut stream, &owners);
                        drop(stream);
                        eprintln!("circular-daemon: closed an owner connection");
                        owners.descriptors_returned.notify();
                    }) {
                    Ok(_) => AcceptStep::Handed,
                    Err(error) => AcceptStep::Exhausted(format!("session thread: {error}")),
                }
            }
            Err(OwnerLocalSocketError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::WouldBlock =>
            {
                AcceptStep::NoPeer
            }
            Err(OwnerLocalSocketError::PeerOwnerMismatch { expected, actual }) => {
                eprintln!("circular-daemon: refused a peer owned by uid {actual}, not {expected}");
                AcceptStep::Refused
            }
            Err(OwnerLocalSocketError::Io { step, source }) if accept_is_transient(&source) => {
                AcceptStep::Exhausted(format!("{step}: {source}"))
            }
            Err(error) => AcceptStep::EndpointStopped(error.to_string()),
        }
    }

    fn start_exit_code(error: &DaemonStartError) -> u8 {
        match error {
            DaemonStartError::StateDirectory(
                StateDirectoryRejection::Unreadable { .. }
                | StateDirectoryRejection::OutsideHome { .. }
                | StateDirectoryRejection::NotADirectory { .. }
                | StateDirectoryRejection::ForeignOwner { .. }
                | StateDirectoryRejection::NotOwnerOnly { .. },
            ) => exit::CHOOSE_LOCATION,
            DaemonStartError::Endpoint(
                OwnerLocalSocketError::ClaimAlreadyHeld
                | OwnerLocalSocketError::LiveSocketAlreadyPresent,
            ) => exit::WAIT,
            DaemonStartError::Endpoint(_) => exit::CHOOSE_LOCATION,
        }
    }

    fn argument_diagnostic(error: &DaemonArgumentsError) -> String {
        error.to_string()
    }

    fn fail(code: u8, message: &str) -> ExitCode {
        eprintln!("circular-daemon: {message}");
        ExitCode::from(code)
    }

    pub(crate) fn record_owner_failed(reason: &str) -> ! {
        eprintln!(
            "circular-daemon: record owner could not open code={}: {reason}",
            exit::CHOOSE_LOCATION
        );
        std::process::exit(i32::from(exit::CHOOSE_LOCATION));
    }
}

#[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
mod platform {
    use std::process::ExitCode;

    /// The OwnerLocal endpoint has one supported realization today.  Refusing
    /// here keeps the unsupported build from looking like a daemon that merely
    /// failed to start.
    pub fn run(_arguments: Vec<std::ffi::OsString>) -> ExitCode {
        eprintln!(
            "circular-daemon: this platform has no OwnerLocal endpoint realization; \
             the supported target is Apple Silicon macOS"
        );
        ExitCode::from(3)
    }
}

