use serde_json::{Value, json};
use std::{io, path::Path};

const VERSION: &str = "2.1.258";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaudeRegistryEntry {
    pid: u32,
    process_start: String,
    socket: String,
    session: String,
    name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaudeMessage {
    pub provider_id: String,
    pub from: String,
    pub from_name: String,
    pub from_mode: String,
    pub body: String,
}

fn invalid(detail: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, detail)
}
fn string<'a>(value: &'a Value, key: &str) -> io::Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty() && !text.contains('\0'))
        .ok_or_else(|| invalid("Claude fixture requires a nonempty string field"))
}
fn attribute(value: &str) -> io::Result<()> {
    if value.is_empty()
        || value
            .chars()
            .any(|c| matches!(c, '"' | '<' | '>' | '&' | '\r' | '\n' | '\0'))
    {
        return Err(invalid(
            "Claude tag attribute escaping has not been measured",
        ));
    }
    Ok(())
}
fn uds(value: &str) -> io::Result<()> {
    let path = value
        .strip_prefix("uds:")
        .ok_or_else(|| invalid("Claude reply address must be uds"))?;
    if !Path::new(path).is_absolute() {
        return Err(invalid("Claude socket path must be absolute"));
    }
    attribute(value)
}

impl ClaudeRegistryEntry {
    /// Decode a registry candidate; validation of the running process and socket
    /// is deliberately separate so an old PID's file cannot become a live peer.
    pub fn decode(path: &Path, bytes: &[u8]) -> io::Result<Self> {
        let value: Value = serde_json::from_slice(bytes)?;
        if value.get("version").and_then(Value::as_str) != Some(VERSION)
            || value.get("peerProtocol").and_then(Value::as_u64) != Some(1)
        {
            return Err(invalid(
                "Claude registry protocol version has not been measured",
            ));
        }
        let pid = value
            .get("pid")
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok())
            .filter(|pid| *pid > 0 && *pid <= i32::MAX as u32)
            .ok_or_else(|| invalid("Claude registry PID is invalid"))?;
        if path.file_name().and_then(|name| name.to_str()) != Some(&format!("{pid}.json")) {
            return Err(invalid("Claude registry filename and PID differ"));
        }
        let socket = string(&value, "messagingSocketPath")?;
        uds(&format!("uds:{socket}"))?;
        let process_start = string(&value, "procStart")?;
        if process_start.trim() != process_start {
            return Err(invalid("Claude registry process start is not normalized"));
        }
        Ok(Self {
            pid,
            process_start: process_start.to_owned(),
            socket: socket.to_owned(),
            session: string(&value, "sessionId")?.to_owned(),
            name: string(&value, "name")?.to_owned(),
        })
    }

    pub const fn pid(&self) -> u32 {
        self.pid
    }
    pub fn socket(&self) -> &Path {
        Path::new(&self.socket)
    }
    pub fn session(&self) -> &str {
        &self.session
    }
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The caller supplies census and socket observations. This does not claim
    /// ListAgents acceptance: that requires the outside socket experiment.
    pub fn matches_process(&self, process_start: Option<&str>) -> bool {
        process_start == Some(self.process_start.as_str())
    }
}

/// Read each numeric PID registry afresh. No .key file is opened. Census and
/// socket checks belong to the adapter's discovery pass, independent of backlog.
/// An unreadable or incompatible entry is retained as a diagnostic, not silently
/// accepted as a peer; one bad entry does not hide later valid candidates.
pub fn registry_candidates(
    directory: &Path,
) -> io::Result<Vec<(std::path::PathBuf, io::Result<ClaudeRegistryEntry>)>> {
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json")
            && path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| !stem.is_empty() && stem.bytes().all(|b| b.is_ascii_digit()))
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths
        .into_iter()
        .map(|path| {
            let result = (|| {
                if !std::fs::symlink_metadata(&path)?.file_type().is_file() {
                    return Err(invalid("Claude registry candidate is not a regular file"));
                }
                ClaudeRegistryEntry::decode(&path, &std::fs::read(&path)?)
            })();
            (path, result)
        })
        .collect())
}

impl ClaudeMessage {
    /// Exactly the observed JSON object plus one newline. The provider ID is
    /// supplied by the caller; no Circular durable identity is minted here.
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        uds(&self.from)?;
        attribute(&self.from_name)?;
        attribute(&self.from_mode)?;
        if self.provider_id.is_empty() || self.provider_id.contains('\0') {
            return Err(invalid("Claude message ID is empty or contains NUL"));
        }
        let content = format!(
            "<cross-session-message from=\"{}\" from-name=\"{}\" from-mode=\"{}\">\n{}\n</cross-session-message>",
            self.from, self.from_name, self.from_mode, self.body
        );
        let mut bytes = serde_json::to_vec(&json!({
            "msgV": 1, "msg_id": self.provider_id, "type": "user",
            "message": {"role": "user", "content": content},
            "priority": "next", "from": self.from
        }))?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Empty discovery connections have no message. All other connections must
    /// supply exactly one complete line; unsupported framing remains a diagnostic.
    pub fn decode(line: &[u8]) -> io::Result<Option<Self>> {
        if line.is_empty() {
            return Ok(None);
        }
        let body = line
            .strip_suffix(b"\n")
            .ok_or_else(|| invalid("Claude message line is incomplete"))?;
        if body.contains(&b'\n') {
            return Err(invalid("Claude connection contains more than one line"));
        }
        let value: Value = serde_json::from_slice(body)?;
        if value.get("msgV").and_then(Value::as_u64) != Some(1)
            || value.get("type").and_then(Value::as_str) != Some("user")
            || value.get("priority").and_then(Value::as_str) != Some("next")
            || value.pointer("/message/role").and_then(Value::as_str) != Some("user")
        {
            return Err(invalid("Claude message shape has not been measured"));
        }
        let from = string(&value, "from")?;
        uds(from)?;
        let content = value
            .pointer("/message/content")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("Claude message content is missing"))?;
        let content = content
            .strip_prefix("<cross-session-message from=\"")
            .ok_or_else(|| invalid("Claude message wrapper is missing"))?;
        let (inner_from, content) = content
            .split_once("\" from-name=\"")
            .ok_or_else(|| invalid("Claude message from-name is missing"))?;
        let (from_name, content) = content
            .split_once("\" from-mode=\"")
            .ok_or_else(|| invalid("Claude message from-mode is missing"))?;
        let (from_mode, content) = content
            .split_once("\">\n")
            .ok_or_else(|| invalid("Claude message wrapper is incomplete"))?;
        if inner_from != from {
            return Err(invalid("Claude inner and outer reply addresses differ"));
        }
        attribute(from_name)?;
        attribute(from_mode)?;
        let body = content
            .strip_suffix("\n</cross-session-message>")
            .ok_or_else(|| invalid("Claude message closing wrapper is missing"))?;
        Ok(Some(Self {
            provider_id: string(&value, "msg_id")?.to_owned(),
            from: from.to_owned(),
            from_name: from_name.to_owned(),
            from_mode: from_mode.to_owned(),
            body: body.to_owned(),
        }))
    }
}

#[cfg(unix)]
pub mod socket;

/// B-4 — the live adapter over the measured transport.
#[cfg(unix)]
pub mod bridge;

#[cfg(unix)]
pub(crate) fn declared_bridge(
    name: circular_runtime::PeerAdapterName,
    settings: &crate::peer_adapter::PeerSettings,
) -> Result<crate::execution_profile::PeerAdapterFactory, String> {
    let config = socket::ClaudeSocketConfig {
        sessions_dir: settings.path("sessions_dir").to_path_buf(),
        socket_dir: settings.path("socket_dir").to_path_buf(),
        display_name: settings.text("display_name").to_owned(),
        from_mode: settings.text("from_mode").to_owned(),
    };
    let slot = bridge::ClaudeAdvertisementSlot::new();
    let factory: crate::execution_profile::PeerAdapterFactory = std::sync::Arc::new(move || {
        Box::new(bridge::ClaudePeerBridge::register(
            name.clone(),
            config.clone(),
            slot.clone(),
        ))
    });
    Ok(factory)
}

#[cfg(not(unix))]
pub(crate) fn declared_bridge(
    _name: circular_runtime::PeerAdapterName,
    _settings: &crate::peer_adapter::PeerSettings,
) -> Result<crate::execution_profile::PeerAdapterFactory, String> {
    Err("this peer bridge speaks over Unix domain sockets".to_owned())
}
