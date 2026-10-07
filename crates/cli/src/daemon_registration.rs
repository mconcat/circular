
const PROGRAM: &str = "circular daemon";

fn usage() -> String {
    format!(
        "{}\nInstallation creates missing state directories with mode 0700 before validating them. Existing permissions are never changed.\n",
        circular_transport::owner_local_usage(
            PROGRAM,
            "Install or uninstall the per-user LaunchAgent that starts circular-daemon for one state",
            "<install|uninstall>",
            "install",
        )
    )
}

pub fn run(arguments: Vec<std::ffi::OsString>) -> std::process::ExitCode {
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

#[cfg(target_vendor = "apple")]
mod platform {
    use crate::shell::launch_agent::{
        LaunchAgentInvocation, LaunchAgentNamespace, LaunchAgentProgram, LaunchAgentServiceKind,
        LaunchAgentStateOption, PersistentRegistrationChange, PersistentRegistrationRemoval,
        UserLaunchAgentArtifact, UserLaunchAgentRegistrar,
    };
    use crate::shell::location::MacOsUserLocationResolver;
    use circular_transport::{DaemonArguments, DaemonArgumentsError, OWNER_LOCAL_SOCKET_NAME};
    use std::process::ExitCode;

    mod exit {
        pub const NONE: u8 = 0;
        pub const FIX_REQUEST: u8 = 2;
        pub const CHOOSE_LOCATION: u8 = 3;
    }

    const BUNDLE_NAMESPACE: &str = "dev.circular";

    const DAEMON_PROGRAM: &str = "circular-daemon";

    pub fn run(raw_arguments: Vec<std::ffi::OsString>) -> ExitCode {
        let mut arguments = raw_arguments.into_iter();
        let Some(verb) = arguments.next() else {
            return fail(exit::FIX_REQUEST, &usage("no verb was given"));
        };
        let verb = verb.to_string_lossy().into_owned();

        let daemon_arguments = match DaemonArguments::parse(arguments) {
            Ok(parsed) => parsed,
            Err(error) => return fail(exit::FIX_REQUEST, &usage(&argument_diagnostic(&error))),
        };

        match verb.as_str() {
            "install" => act(&daemon_arguments, Action::Install),
            "uninstall" => act(&daemon_arguments, Action::Uninstall),
            other => fail(
                exit::FIX_REQUEST,
                &usage(&format!("unknown verb {other:?}")),
            ),
        }
    }

    enum Action {
        Install,
        Uninstall,
    }

    fn act(daemon_arguments: &DaemonArguments, action: Action) -> ExitCode {
        let resolver = match MacOsUserLocationResolver::for_current_user() {
            Ok(resolver) => resolver,
            Err(error) => return fail(exit::CHOOSE_LOCATION, &format!("{error}")),
        };
        let home = resolver.canonical_home().to_path_buf();

        let resolver = if matches!(action, Action::Install) {
            let executable = match std::env::current_exe().and_then(std::fs::canonicalize) {
                Ok(path) => path,
                Err(error) => {
                    return fail(
                        exit::CHOOSE_LOCATION,
                        &format!("cannot resolve running daemon: {error}"),
                    );
                }
            };
            let Some(bin) = executable
                .parent()
                .filter(|parent| parent.file_name().is_some_and(|name| name == "bin"))
            else {
                return fail(
                    exit::CHOOSE_LOCATION,
                    "registration requires circular-daemon inside an installed image's bin directory",
                );
            };
            let Some(prefix) = bin.parent() else {
                return fail(
                    exit::CHOOSE_LOCATION,
                    "the installed image has no root directory",
                );
            };
            resolver.with_install_directory(prefix.to_path_buf())
        } else {
            resolver
        };

        let (resolver, state_location) =
            match resolver.with_state_directory(daemon_arguments.state_directory()) {
                Ok(bound) => bound,
                Err(error) => {
                    return fail(
                        exit::CHOOSE_LOCATION,
                        &format!("state location refused: {error}"),
                    );
                }
            };
        let install_location = resolver.selected_install();

        let daemon_program = match LaunchAgentProgram::try_new(
            install_location
                .directory()
                .join("bin")
                .join(DAEMON_PROGRAM),
        ) {
            Ok(program) => program,
            Err(error) => return fail(exit::CHOOSE_LOCATION, &format!("{error:?}")),
        };

        let daemon_invocation = match LaunchAgentInvocation::try_new(
            daemon_program,
            state_location,
            LaunchAgentStateOption::canonical(),
            Vec::<String>::new(),
        ) {
            Ok(invocation) => invocation,
            Err(error) => return fail(exit::FIX_REQUEST, &format!("{error:?}")),
        };

        let namespace =
            LaunchAgentNamespace::try_new(BUNDLE_NAMESPACE).expect("published prefix is valid");
        let daemon_artifact = match UserLaunchAgentArtifact::render_for_service(
            &resolver,
            &namespace,
            LaunchAgentServiceKind::Daemon,
            install_location.clone(),
            daemon_invocation,
        ) {
            Ok(artifact) => artifact,
            Err(error) => return fail(exit::CHOOSE_LOCATION, &format!("{error:?}")),
        };

        let registrar = match UserLaunchAgentRegistrar::try_new(&resolver, &install_location) {
            Ok(registrar) => registrar,
            Err(error) => return fail(exit::CHOOSE_LOCATION, &format!("{error:?}")),
        };

        match action {
            Action::Install => {
                if let Err(error) =
                    registrar.validate_program(daemon_artifact.invocation().program())
                {
                    return fail(
                        exit::CHOOSE_LOCATION,
                        &format!("packaged {DAEMON_PROGRAM} is unavailable: {error:?}"),
                    );
                }

                if let Err(error) =
                    resolver.prepare_registration_state(daemon_artifact.state_location())
                {
                    return fail(
                        exit::CHOOSE_LOCATION,
                        &format!("state location refused: {error}"),
                    );
                }
                let accepted_state =
                    match daemon_arguments.accept_state_directory(&home, resolver.user().get()) {
                        Ok(state) => state,
                        Err(error) => {
                            return fail(
                                exit::CHOOSE_LOCATION,
                                &format!("state location refused: {error}"),
                            );
                        }
                    };
                if let Err(error) = circular_transport::OwnerLocalSocketSpec::try_new(
                    accepted_state.path(),
                    OWNER_LOCAL_SOCKET_NAME,
                ) {
                    return fail(
                        exit::CHOOSE_LOCATION,
                        &format!("state location refused: {error}"),
                    );
                }

                let daemon_change = match registrar.register(&daemon_artifact) {
                    Ok(change) => change,
                    Err(error) => {
                        return fail(
                            exit::CHOOSE_LOCATION,
                            &format!("daemon registration failed: {error:?}"),
                        );
                    }
                };
                report_change(
                    LaunchAgentServiceKind::Daemon,
                    daemon_change,
                    &daemon_artifact,
                );
                eprintln!(
                    "circular daemon: owner-local socket {}",
                    daemon_artifact
                        .state_location()
                        .directory()
                        .join(OWNER_LOCAL_SOCKET_NAME)
                        .display()
                );
                eprintln!(
                    "circular daemon: no process was started — the plist is the whole postcondition"
                );
                ExitCode::from(exit::NONE)
            }
            Action::Uninstall => {
                let daemon_removal = match registrar.unregister(&daemon_artifact) {
                    Ok(removal) => removal,
                    Err(error) => {
                        return fail(
                            exit::CHOOSE_LOCATION,
                            &format!("daemon unregistration failed: {error:?}"),
                        );
                    }
                };
                report_removal(
                    LaunchAgentServiceKind::Daemon,
                    daemon_removal,
                    &daemon_artifact,
                );
                ExitCode::from(exit::NONE)
            }
        }
    }

    fn report_change(
        service: LaunchAgentServiceKind,
        change: PersistentRegistrationChange,
        artifact: &UserLaunchAgentArtifact,
    ) {
        eprintln!(
            "circular daemon: {} {} {}",
            service.as_str(),
            match change {
                PersistentRegistrationChange::Created => "wrote",
                PersistentRegistrationChange::Replaced => "replaced",
                PersistentRegistrationChange::Unchanged => "left unchanged",
            },
            artifact.plist_path().display()
        );
    }

    fn report_removal(
        service: LaunchAgentServiceKind,
        removal: PersistentRegistrationRemoval,
        artifact: &UserLaunchAgentArtifact,
    ) {
        eprintln!(
            "circular daemon: {} {} {}",
            service.as_str(),
            match removal {
                PersistentRegistrationRemoval::Removed => "removed",
                PersistentRegistrationRemoval::AlreadyAbsent => "found no",
            },
            artifact.plist_path().display()
        );
    }

    fn usage(detail: &str) -> String {
        format!("{detail}\n\n{}", super::usage())
    }

    fn argument_diagnostic(error: &DaemonArgumentsError) -> String {
        format!("{error}")
    }

    fn fail(code: u8, message: &str) -> ExitCode {
        eprintln!("circular daemon: {message}");
        ExitCode::from(code)
    }
}

#[cfg(not(target_vendor = "apple"))]
mod platform {
    use std::process::ExitCode;

    pub fn run(_arguments: Vec<std::ffi::OsString>) -> ExitCode {
        eprintln!(
            "circular daemon: per-user LaunchAgent registration exists on Apple Silicon macOS only"
        );
        ExitCode::from(3)
    }
}
