
const PROGRAM: &str = "circular-client";

fn usage() -> String {
    circular_transport::owner_local_usage(
        PROGRAM,
        "Circular owner-local protocol test client",
        "",
        "",
    )
}

fn main() -> std::process::ExitCode {
    match circular_transport::OwnerLocalInvocation::from_env() {
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
    use circular_protocol::INITIAL_PROTOCOL_VERSION;
    use circular_protocol::session_payload::SessionRole;
    use circular_transport::{DaemonArguments, DaemonArgumentsError, current_effective_user};
    use cli::owner_local_client::{ClientError, endpoint_of, establish};
    use std::path::PathBuf;
    use std::process::ExitCode;

    mod exit {
        pub const NONE: u8 = 0;
        pub const FIX_REQUEST: u8 = 2;
        pub const CHOOSE_LOCATION: u8 = 3;
        pub const PEER_CONTRACT: u8 = 5;
    }

    const PROBE_CORRELATION: u32 = 0x0102_0304;

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

        let accepted = match arguments.accept_state_directory(&home, current_effective_user()) {
            Ok(accepted) => accepted,
            Err(rejection) => return fail(exit::CHOOSE_LOCATION, &rejection.to_string()),
        };

        let spec = match endpoint_of(accepted.path()) {
            Ok(spec) => spec,
            Err(error) => return fail(exit::CHOOSE_LOCATION, &error.to_string()),
        };

        match establish(
            &spec,
            INITIAL_PROTOCOL_VERSION,
            0,
            &[SessionRole::Reader],
            PROBE_CORRELATION,
        ) {
            Ok(observed) => {
                eprintln!(
                    "circular-client: established with {} — answer {:?} correlation {} payload {} bytes",
                    spec.socket_path().display(),
                    observed.verb,
                    observed.correlation,
                    observed.payload_length
                );
                ExitCode::from(exit::NONE)
            }
            Err(error) => fail(client_exit_code(&error), &error.to_string()),
        }
    }

    fn client_exit_code(error: &ClientError) -> u8 {
        match error {
            ClientError::Socket(_)
            | ClientError::Session(_)
            | ClientError::Hello(circular_transport::OwnerLocalHelloError::Session(_)) => {
                exit::CHOOSE_LOCATION
            }
            ClientError::UnexpectedResponse
            | ClientError::Hello(circular_transport::OwnerLocalHelloError::Body(_))
            | ClientError::Hello(circular_transport::OwnerLocalHelloError::UnexpectedAnswer {
                ..
            }) => exit::PEER_CONTRACT,
        }
    }

    fn argument_diagnostic(error: &DaemonArgumentsError) -> String {
        error.to_string()
    }

    fn fail(code: u8, message: &str) -> ExitCode {
        eprintln!("circular-client: {message}");
        ExitCode::from(code)
    }
}

#[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
mod platform {
    use std::process::ExitCode;

    pub fn run(_arguments: Vec<std::ffi::OsString>) -> ExitCode {
        eprintln!(
            "circular-client: this platform has no OwnerLocal endpoint realization; \
             the supported target is Apple Silicon macOS"
        );
        ExitCode::from(3)
    }
}
