
use crate::temp::StateDir;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
pub enum StartFailure {
    Prepare(std::io::Error),
    Spawn(std::io::Error),
    Exited {
        code: Option<i32>,
        diagnostics: String,
    },
    NotReady { diagnostics: String },
}

pub struct Daemon {
    child: Child,
    diagnostics: Arc<Mutex<String>>,
    drain: Option<std::thread::JoinHandle<()>>,
    state: PathBuf,
    home: StateDir,
}

impl Daemon {
    pub fn start(binary: &Path) -> Result<Self, StartFailure> {
        let home = StateDir::under_home("daemon");
        let state = home.path().join("state");
        prepare_state(&state).map_err(StartFailure::Prepare)?;
        let mut child = Command::new(binary)
            .arg("--state")
            .arg(&state)
            .env("HOME", home.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(StartFailure::Spawn)?;
        let diagnostics = Arc::new(Mutex::new(String::new()));
        let (said, heard) = std::sync::mpsc::channel::<()>();
        let drain = child.stderr.take().map(|pipe| {
            let diagnostics = Arc::clone(&diagnostics);
            std::thread::spawn(move || {
                for line in std::io::BufReader::new(pipe).lines() {
                    let Ok(line) = line else { break };
                    if let Ok(mut text) = diagnostics.lock() {
                        text.push_str(&line);
                        text.push('\n');
                    }
                    let _ = said.send(());
                }
            })
        });
        let mut daemon = Self {
            child,
            diagnostics,
            drain,
            state,
            home,
        };
        let socket = daemon.socket();
        loop {
            if socket.exists() {
                return Ok(daemon);
            }
            match heard.recv_timeout(crate::FACT_LIVENESS) {
                Ok(()) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    let status = daemon.child.wait().map_err(StartFailure::Spawn)?;
                    if socket.exists() {
                        return Ok(daemon);
                    }
                    return Err(StartFailure::Exited {
                        code: status.code(),
                        diagnostics: daemon.join_diagnostics(),
                    });
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(StartFailure::NotReady {
                        diagnostics: daemon.join_diagnostics(),
                    });
                }
            }
        }
    }

    #[must_use]
    pub fn home(&self) -> &Path {
        self.home.path()
    }

    #[must_use]
    pub fn state(&self) -> &Path {
        &self.state
    }

    #[must_use]
    pub fn socket(&self) -> PathBuf {
        self.state.join("daemon.sock")
    }

    #[must_use]
    pub fn stop(mut self) -> String {
        self.reap();
        self.join_diagnostics()
    }

    fn reap(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn join_diagnostics(&mut self) -> String {
        if let Some(drain) = self.drain.take() {
            self.reap();
            let _ = drain.join();
        }
        self.diagnostics
            .lock()
            .map(|text| text.clone())
            .unwrap_or_default()
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.reap();
        if let Some(drain) = self.drain.take() {
            let _ = drain.join();
        }
    }
}

fn prepare_state(state: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(state)
}
