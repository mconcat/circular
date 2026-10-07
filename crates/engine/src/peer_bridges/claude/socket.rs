//! Native Unix transport for the measured Claude peer protocol.
//!
//! Socket tests are in the outside execution list. The 2026-09-09 B-4 native
//! measurement requires inbound acknowledgement by closing without response
//! bytes. Transport closure is not itself durable Circular admission.
use super::{ClaudeMessage, ClaudeRegistryEntry, VERSION, attribute, invalid, registry_candidates};
use serde_json::json;
use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::Shutdown,
    os::{
        fd::OwnedFd,
        unix::{
            fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Existing [[peer.adapter]] provider fields after path expansion.
#[derive(Clone, Debug)]
pub struct ClaudeSocketConfig {
    pub sessions_dir: PathBuf,
    pub socket_dir: PathBuf,
    pub display_name: String,
    pub from_mode: String,
}

#[derive(Debug)]
struct OwnedPath {
    path: PathBuf,
    device: u64,
    inode: u64,
}
impl OwnedPath {
    fn capture(path: PathBuf) -> io::Result<Self> {
        let metadata = fs::symlink_metadata(&path)?;
        Ok(Self {
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}
impl Drop for OwnedPath {
    fn drop(&mut self) {
        if let Ok(metadata) = fs::symlink_metadata(&self.path)
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn process_start(pid: u32) -> io::Result<String> {
    let mut command = Command::new("ps");
    crate::direct_effect::explicit_child_environment(&mut command);
    let output = command
        .args(["-p", &pid.to_string(), "-o", "lstart="])
        .output()?;
    if !output.status.success() {
        return Err(invalid("Claude peer process is no longer running"));
    }
    let start =
        String::from_utf8(output.stdout).map_err(|_| invalid("process start is not UTF-8"))?;
    let start = start.trim();
    if start.is_empty() {
        return Err(invalid("process start is missing"));
    }
    Ok(start.to_owned())
}

/// The same liveness pass without owning an advertisement — a bridge discovers
/// before it binds. Candidates that fail the PID start or socket check are kept
/// as diagnostics so one dead entry cannot hide the live sessions behind it.
pub fn live_registry(
    sessions_dir: &Path,
) -> io::Result<Vec<(PathBuf, io::Result<ClaudeRegistryEntry>)>> {
    let mut candidates = registry_candidates(sessions_dir)?;
    for (_, candidate) in &mut candidates {
        if let Ok(entry) = candidate {
            let observed = process_start(entry.pid());
            let checked = observed.and_then(|start| {
                if !entry.matches_process(Some(&start)) {
                    return Err(invalid("Claude peer PID was reused"));
                }
                let metadata = fs::symlink_metadata(entry.socket())?;
                if !metadata.file_type().is_socket() {
                    return Err(invalid("Claude peer endpoint is not a socket"));
                }
                Ok(())
            });
            if let Err(error) = checked {
                *candidate = Err(error);
            }
        }
    }
    Ok(candidates)
}

/// A protocol-native UUID, unrelated to Circular effect or record identities.
/// The measured traffic carries one in `sessionId` and one in `msg_id`.
pub fn protocol_uuid() -> io::Result<String> {
    let mut bytes = [0; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

struct PendingConnection {
    stream: UnixStream,
    bytes: Vec<u8>,
    opened: Instant,
}

/// One complete native message with its still-open acknowledgement connection.
/// Dropping it closes the connection; it does not mint durable admission.
pub struct ClaudeInboundConnection {
    pub message: ClaudeMessage,
    stream: UnixStream,
}
impl ClaudeInboundConnection {
    /// After the caller has completed durable custody, acknowledge by closing.
    /// B-4 native measurement: writing either JSON ack or echo causes ECONNRESET.
    /// This API deliberately accepts no response bytes and performs no write.
    pub fn acknowledge(self) {
        drop(self.stream);
    }
}

/// Owns only this process's registry and Unix socket. Existing paths are never
/// overwritten or removed. One native peer is advertised for this transport.
pub struct ClaudeSocketPeer {
    listener: UnixListener,
    registry: OwnedPath,
    socket: OwnedPath,
    config: ClaudeSocketConfig,
    name: String,
    pending: VecDeque<PendingConnection>,
}
impl ClaudeSocketPeer {
    pub fn advertise(config: ClaudeSocketConfig, peer_name: Option<&str>) -> io::Result<Self> {
        for directory in [&config.sessions_dir, &config.socket_dir] {
            if !directory.is_absolute() || !directory.is_dir() {
                return Err(invalid(
                    "Claude adapter requires existing absolute directories",
                ));
            }
        }
        let name = peer_name.unwrap_or(&config.display_name).to_owned();
        attribute(&name)?;
        attribute(&config.from_mode)?;
        let pid = std::process::id();
        let socket_path = config.socket_dir.join(format!("{pid}.sock"));
        let registry_path = config.sessions_dir.join(format!("{pid}.json"));
        let listener = UnixListener::bind(&socket_path)?;
        let socket = OwnedPath::capture(socket_path)?;
        fs::set_permissions(&socket.path, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&registry_path)?;
        let registry = OwnedPath::capture(registry_path)?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid("system clock precedes Unix epoch"))?
            .as_millis();
        let now = u64::try_from(now).map_err(|_| invalid("registry time exceeds u64"))?;
        let record = json!({
            "cwd": std::env::current_dir()?.to_str().ok_or_else(|| invalid("cwd is not UTF-8"))?,
            "entrypoint": "cli", "kind": "interactive",
            "messagingSocketPath": socket.path.to_str().ok_or_else(|| invalid("socket path is not UTF-8"))?,
            "name": name, "nameSource": "user", "peerProtocol": 1, "pid": pid,
            "procStart": process_start(pid)?, "sessionId": protocol_uuid()?,
            "startedAt": now, "status": "idle", "statusUpdatedAt": now,
            "updatedAt": now, "version": VERSION,
        });
        serde_json::to_writer(&mut file, &record)?;
        file.flush()?;
        file.sync_all()?;
        Ok(Self {
            listener,
            registry,
            socket,
            config,
            name,
            pending: VecDeque::new(),
        })
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket.path
    }
    pub fn registry_path(&self) -> &Path {
        &self.registry.path
    }

    /// Read current registry candidates, then compare PID start and filesystem
    /// socket facts. ListAgents' own liveness probe is an outside measurement.
    pub fn discover(&self) -> io::Result<Vec<(PathBuf, io::Result<ClaudeRegistryEntry>)>> {
        live_registry(&self.config.sessions_dir)
    }

    /// Write one message to the peer's actual messagingSocketPath and return raw
    /// response bytes. Callers must not turn transport success into a receipt.
    pub fn send(
        &self,
        peer: &ClaudeRegistryEntry,
        provider_id: &str,
        body: &str,
        timeout: Duration,
        max_response_bytes: usize,
    ) -> io::Result<Vec<u8>> {
        let observed = process_start(peer.pid())?;
        if !peer.matches_process(Some(&observed)) {
            return Err(invalid("Claude peer PID was reused"));
        }
        let from = format!(
            "uds:{}",
            self.socket
                .path
                .to_str()
                .ok_or_else(|| invalid("socket path is not UTF-8"))?
        );
        let line = ClaudeMessage {
            provider_id: provider_id.into(),
            from,
            from_name: self.name.clone(),
            from_mode: self.config.from_mode.clone(),
            body: body.into(),
        }
        .encode()?;
        let mut stream = UnixStream::connect(peer.socket())?;
        stream.set_write_timeout(Some(timeout))?;
        stream.set_read_timeout(Some(timeout))?;
        stream.write_all(&line)?;
        stream.shutdown(Shutdown::Write)?;
        let mut response = Vec::new();
        let mut byte = [0];
        loop {
            match stream.read(&mut byte)? {
                0 => return Ok(response),
                _ if response.len() == max_response_bytes => {
                    return Err(invalid("Claude response exceeds capture capacity"));
                }
                _ => {
                    response.push(byte[0]);
                    if byte[0] == b'\n' {
                        return Ok(response);
                    }
                }
            }
        }
    }

    pub fn readiness(
        &self,
        capacity: usize,
        timeout: Duration,
    ) -> io::Result<(Vec<OwnedFd>, Option<Instant>)> {
        if capacity == 0 {
            return Ok((Vec::new(), None));
        }
        let mut fds = Vec::with_capacity(self.pending.len() + 1);
        if self.pending.len() < capacity {
            fds.push(OwnedFd::from(self.listener.try_clone()?));
        }
        for connection in &self.pending {
            fds.push(OwnedFd::from(connection.stream.try_clone()?));
        }
        let deadline = self
            .pending
            .iter()
            .map(|connection| connection.opened + timeout)
            .min();
        Ok((fds, deadline))
    }

    /// Bounded nonblocking intake. Partial lines stay owned here across polls;
    /// empty discovery probes produce no message and slow peers cannot monopolize
    /// a poll. Capacity and byte/time ceilings are supplied by the adapter.
    pub fn poll(
        &mut self,
        capacity: usize,
        max_line_bytes: usize,
        timeout: Duration,
    ) -> Vec<io::Result<ClaudeInboundConnection>> {
        let mut ready = Vec::new();
        for _ in self.pending.len()..capacity {
            match self.listener.accept() {
                Ok((stream, _)) => match stream.set_nonblocking(true) {
                    Ok(()) => self.pending.push_back(PendingConnection {
                        stream,
                        bytes: Vec::new(),
                        opened: Instant::now(),
                    }),
                    Err(error) => ready.push(Err(error)),
                },
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    ready.push(Err(error));
                    break;
                }
            }
        }
        for _ in 0..self.pending.len() {
            let mut connection = self.pending.pop_front().expect("initial pending count");
            let mut buffer = [0; 4096];
            let mut complete = false;
            let mut failed = None;
            let mut closed = false;
            if connection.opened.elapsed() >= timeout {
                failed = Some(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Claude peer line timed out",
                ));
            } else {
                match connection.stream.read(&mut buffer) {
                    Ok(0) => closed = true,
                    Ok(count) => {
                        if connection.bytes.len().saturating_add(count) > max_line_bytes {
                            failed = Some(invalid("Claude peer line exceeds inbox frame capacity"));
                        } else {
                            connection.bytes.extend_from_slice(&buffer[..count]);
                            complete = connection.bytes.contains(&b'\n');
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => failed = Some(error),
                }
            }
            if let Some(error) = failed {
                ready.push(Err(error));
            } else if complete || closed {
                match ClaudeMessage::decode(&connection.bytes) {
                    Ok(Some(message)) => ready.push(Ok(ClaudeInboundConnection {
                        message,
                        stream: connection.stream,
                    })),
                    Ok(None) => {}
                    Err(error) => ready.push(Err(error)),
                }
            } else {
                self.pending.push_back(connection);
            }
        }
        ready
    }
}
