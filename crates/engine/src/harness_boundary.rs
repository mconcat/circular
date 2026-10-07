
use circular_runtime::{Capability, EffectFailure, InterpreterFault};
use std::io::{Read as _, Write as _};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DeclaredPath {
    Home(PathBuf),
    Absolute(PathBuf),
}

impl DeclaredPath {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let (path, home) = match text.strip_prefix("~/") {
            Some(rest) => (Path::new(rest), true),
            None if text.starts_with('/') => (Path::new(text), false),
            None => return None,
        };
        if path.as_os_str().is_empty()
            || path
                .components()
                .any(|component| matches!(component, Component::ParentDir))
        {
            return None;
        }
        Some(if home {
            Self::Home(path.to_path_buf())
        } else {
            Self::Absolute(path.to_path_buf())
        })
    }

    pub(crate) fn resolve(&self, home: &Path) -> PathBuf {
        match self {
            Self::Home(rest) => home.join(rest),
            Self::Absolute(path) => path.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EgressEntry(String);

impl EgressEntry {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        if !circular_runtime::authority_entry_is_valid(text) {
            return None;
        }
        let (host, port) = match text.rsplit_once(':') {
            Some((host, "*")) => (host, "*".to_owned()),
            Some((host, port)) => (host, port.parse::<u16>().ok()?.to_string()),
            None => (text, "443".to_owned()),
        };
        let host = host.to_ascii_lowercase();
        let valid = !host.is_empty()
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'));
        valid.then(|| Self(format!("{host}:{port}")))
    }

    fn admits(&self, host: &str, port: u16) -> bool {
        circular_runtime::authority_admits(
            &self.0,
            &format!("{}:{port}", host.to_ascii_lowercase()),
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BoundaryDeclaration {
    pub(crate) egress: Vec<EgressEntry>,
    pub(crate) write: Vec<DeclaredPath>,
    pub(crate) write_denied: Vec<DeclaredPath>,
    pub(crate) login: Vec<DeclaredPath>,
}

impl BoundaryDeclaration {
    pub(crate) const EMPTY: Self = Self {
        egress: Vec::new(),
        write: Vec::new(),
        write_denied: Vec::new(),
        login: Vec::new(),
    };
}

fn real_path(path: &Path) -> PathBuf {
    let mut existing = path;
    let mut rest = Vec::new();
    loop {
        if let Ok(real) = std::fs::canonicalize(existing) {
            return rest.iter().rev().fold(real, |acc, part| acc.join(part));
        }
        match (existing.parent(), existing.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_os_string());
                existing = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

fn quoted(path: &Path) -> String {
    let text = path.to_string_lossy();
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        if matches!(character, '"' | '\\') {
            out.push('\\');
        }
        out.push(character);
    }
    out.push('"');
    out
}

fn regex_escaped(path: &Path) -> String {
    let mut out = String::new();
    for character in path.to_string_lossy().chars() {
        if "\\^$.|?*+()[]{}\"".contains(character) {
            out.push('\\');
        }
        out.push(character);
    }
    out
}

fn subpaths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| format!("(subpath {})", quoted(path)))
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Debug)]
pub(crate) struct TurnBoundary {
    node: PathBuf,
    write: Vec<PathBuf>,
    write_denied: Vec<PathBuf>,
    closed: Vec<PathBuf>,
    login: Vec<PathBuf>,
}

impl TurnBoundary {
    pub(crate) fn draw(
        declared: &BoundaryDeclaration,
        secrets: &[DeclaredPath],
        node: &Path,
        state: &Path,
        home: &Path,
    ) -> Self {
        let resolve = |paths: &[DeclaredPath]| {
            paths
                .iter()
                .map(|path| real_path(&path.resolve(home)))
                .collect::<Vec<_>>()
        };
        let mut closed = resolve(secrets);
        closed.push(real_path(state));
        Self {
            node: real_path(node),
            write: resolve(&declared.write),
            write_denied: resolve(&declared.write_denied),
            closed,
            login: resolve(&declared.login),
        }
    }

    pub(crate) fn profile(&self, window_port: u16) -> String {
        let mut rules = vec![
            "(version 1)".to_owned(),
            "(allow default)".to_owned(),
            "(deny network*)".to_owned(),
            format!("(allow network-outbound (remote ip \"localhost:{window_port}\"))"),
            "(deny file-write*)".to_owned(),
            "(allow file-write* (literal \"/dev/null\") (literal \"/dev/zero\") \
             (regex #\"^/dev/tty\") (regex #\"^/dev/fd/\"))"
                .to_owned(),
        ];
        let mut writable = vec![self.node.clone()];
        writable.extend(self.write.iter().cloned());
        rules.push(format!("(allow file-write* {})", subpaths(&writable)));
        if !self.write_denied.is_empty() {
            rules.push(format!(
                "(deny file-write* {})",
                subpaths(&self.write_denied)
            ));
        }
        rules.push(format!(
            "(deny file-read* file-write* {})",
            subpaths(&self.closed)
        ));
        rules.push(format!(
            "(allow file-read* file-write* {})",
            subpaths(std::slice::from_ref(&self.node))
        ));
        for login in &self.login {
            rules.push(format!(
                "(allow file-read* file-write* (regex #\"^{}(\\..*|-.*|_.*)?$\"))",
                regex_escaped(login)
            ));
        }
        rules.join("\n")
    }
}

pub(crate) fn bounded_command(
    program: &Path,
    args: &[String],
    profile: &str,
) -> Result<std::process::Command, (EffectFailure, String)> {
    #[cfg(target_os = "macos")]
    {
        let mut command = std::process::Command::new("/usr/bin/sandbox-exec");
        command.arg("-p").arg(profile).arg(program).args(args);
        Ok(command)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (program, args, profile);
        Err((
            EffectFailure::ParameterDenied {
                capability: Capability::AgentHarness,
            },
            "this platform draws no OS boundary for agent harness turns".to_owned(),
        ))
    }
}

pub(crate) fn no_home() -> (EffectFailure, String) {
    (
        EffectFailure::ParameterDenied {
            capability: Capability::AgentHarness,
        },
        "HOME is not set, so the harness boundary cannot be drawn".to_owned(),
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EgressDenial {
    pub(crate) at: String,
    pub(crate) count: u64,
}

#[derive(Default)]
struct WindowState {
    closed: bool,
    streams: Vec<(u64, TcpStream)>,
    next: u64,
    deciding: usize,
    denied: Vec<EgressDenial>,
}

struct WindowShared {
    allowed: Vec<EgressEntry>,
    state: Mutex<WindowState>,
    decided: Condvar,
}

impl WindowShared {
    fn state(&self) -> MutexGuard<'_, WindowState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn deny(&self, at: String) {
        let mut state = self.state();
        match state.denied.iter_mut().find(|denial| denial.at == at) {
            Some(denial) => denial.count += 1,
            None => state.denied.push(EgressDenial { at, count: 1 }),
        }
    }

    fn decided(&self) {
        let mut state = self.state();
        state.deciding -= 1;
        self.decided.notify_all();
    }

    fn register(state: &mut WindowState, stream: &TcpStream) -> Option<u64> {
        let clone = stream.try_clone().ok().filter(|_| !state.closed);
        let Some(clone) = clone else {
            let _ = stream.shutdown(Shutdown::Both);
            return None;
        };
        let id = state.next;
        state.next += 1;
        state.streams.push((id, clone));
        Some(id)
    }

    fn forget(&self, ids: &[u64]) {
        let mut state = self.state();
        state.streams.retain(|(id, stream)| {
            let done = ids.contains(id);
            if done {
                let _ = stream.shutdown(Shutdown::Both);
            }
            !done
        });
    }
}

pub(crate) struct EgressWindow {
    port: u16,
    shared: Arc<WindowShared>,
    acceptor: Option<JoinHandle<()>>,
}

const HEAD_LIMIT_BYTES: usize = 16 * 1024;

impl EgressWindow {
    pub(crate) fn open(allowed: &[EgressEntry]) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let shared = Arc::new(WindowShared {
            allowed: allowed.to_vec(),
            state: Mutex::new(WindowState::default()),
            decided: Condvar::new(),
        });
        let acceptor = {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("harness-egress-window".to_owned())
                .spawn(move || accept(&listener, &shared))?
        };
        Ok(Self {
            port,
            shared,
            acceptor: Some(acceptor),
        })
    }

    #[must_use]
    pub(crate) const fn port(&self) -> u16 {
        self.port
    }

    pub(crate) fn close(&mut self) -> Vec<EgressDenial> {
        let Some(acceptor) = self.acceptor.take() else {
            return Vec::new();
        };
        {
            let mut state = self.shared.state();
            state.closed = true;
            for (_, stream) in state.streams.drain(..) {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        let _ = acceptor.join();
        let mut state = self.shared.state();
        while state.deciding > 0 {
            state = self
                .shared
                .decided
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        std::mem::take(&mut state.denied)
    }
}

impl Drop for EgressWindow {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn accept(listener: &TcpListener, shared: &Arc<WindowShared>) {
    for incoming in listener.incoming() {
        let Ok(stream) = incoming else {
            continue;
        };
        let id = {
            let mut state = shared.state();
            if state.closed {
                return;
            }
            let Some(id) = WindowShared::register(&mut state, &stream) else {
                continue;
            };
            state.deciding += 1;
            id
        };
        let handler = Arc::clone(shared);
        if std::thread::Builder::new()
            .name("harness-egress-connection".to_owned())
            .spawn(move || {
                let upstream = serve(stream, &handler);
                handler.forget(
                    &[Some(id), upstream]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>(),
                );
            })
            .is_err()
        {
            shared.decided();
            shared.forget(&[id]);
        }
    }
}

struct Request {
    connect: bool,
    host: String,
    port: u16,
    at: String,
}

enum Head {
    Read(Vec<u8>, usize),
    Oversized(Vec<u8>),
    Gone,
}

fn read_head(client: &mut TcpStream) -> Head {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            return Head::Read(buffer, end + 4);
        }
        if buffer.len() > HEAD_LIMIT_BYTES {
            return Head::Oversized(buffer);
        }
        match client.read(&mut chunk) {
            Ok(0) | Err(_) => return Head::Gone,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    }
}

fn parse_request(head: &[u8]) -> Result<Request, String> {
    let line = head
        .split(|byte| *byte == b'\n')
        .next()
        .map(|line| String::from_utf8_lossy(line).trim_end().to_owned())
        .unwrap_or_default();
    let mut parts = line.split(' ');
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Err(line);
    };
    let authority_port = |authority: &str, default: u16| -> Option<(String, u16)> {
        match authority.rsplit_once(':') {
            Some((host, port)) => Some((host.to_owned(), port.parse().ok()?)),
            None => Some((authority.to_owned(), default)),
        }
    };
    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = authority_port(target, 443).ok_or_else(|| target.to_owned())?;
        return Ok(Request {
            connect: true,
            at: format!("{host}:{port}"),
            host,
            port,
        });
    }
    let Some(rest) = target.strip_prefix("http://") else {
        return Err(target.to_owned());
    };
    let authority = rest.split('/').next().unwrap_or_default();
    let (host, port) = authority_port(authority, 80).ok_or_else(|| target.to_owned())?;
    Ok(Request {
        connect: false,
        at: format!("{host}:{port}"),
        host,
        port,
    })
}

fn serve(mut client: TcpStream, shared: &Arc<WindowShared>) -> Option<u64> {
    let (head, end) = match read_head(&mut client) {
        Head::Read(head, end) => (head, end),
        Head::Oversized(head) => {
            shared.deny(parse_request(&head).map_or_else(|at| at, |request| request.at));
            shared.decided();
            let _ = client.write_all(
                b"HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            return None;
        }
        Head::Gone => {
            shared.decided();
            return None;
        }
    };
    let request = match parse_request(&head[..end]) {
        Ok(request) => request,
        Err(at) => {
            shared.deny(at);
            shared.decided();
            let _ = client.write_all(
                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            return None;
        }
    };
    let admitted = !request.host.is_empty()
        && shared
            .allowed
            .iter()
            .any(|entry| entry.admits(&request.host, request.port));
    if !admitted {
        shared.deny(request.at);
        shared.decided();
        let _ = client
            .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return None;
    }
    shared.decided();
    let Ok(upstream) = TcpStream::connect((request.host.as_str(), request.port)) else {
        let _ = client.write_all(
            b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return None;
    };
    let upstream_id = WindowShared::register(&mut shared.state(), &upstream)?;
    relay(client, upstream, &head, end, request.connect);
    Some(upstream_id)
}

fn relay(mut client: TcpStream, upstream: TcpStream, head: &[u8], end: usize, connect: bool) {
    let mut upstream_writer = upstream;
    let sent = if connect {
        client
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .and_then(|()| upstream_writer.write_all(&head[end..]))
    } else {
        upstream_writer.write_all(head)
    };
    if sent.is_err() {
        return;
    }
    let (Ok(mut client_reader), Ok(mut upstream_reader)) =
        (client.try_clone(), upstream_writer.try_clone())
    else {
        return;
    };
    let outbound = std::thread::Builder::new()
        .name("harness-egress-pipe".to_owned())
        .spawn(move || {
            let _ = std::io::copy(&mut client_reader, &mut upstream_writer);
            let _ = upstream_writer.shutdown(Shutdown::Write);
        });
    let _ = std::io::copy(&mut upstream_reader, &mut client);
    let _ = client.shutdown(Shutdown::Write);
    if let Ok(outbound) = outbound {
        let _ = outbound.join();
    }
}

pub(crate) const WINDOW_ENVIRONMENT: [&str; 4] =
    ["HTTPS_PROXY", "HTTP_PROXY", "https_proxy", "http_proxy"];

pub(crate) const NODE_TEMP_FOLDER: &str = ".tmp";

pub(crate) fn node_temp(node: &Path) -> Result<PathBuf, EffectFailure> {
    let temp = node.join(NODE_TEMP_FOLDER);
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder
        .create(&temp)
        .map_err(|error| EffectFailure::InterpreterFault(InterpreterFault::from(error.kind())))?;
    Ok(temp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_paths_are_home_or_absolute_and_never_climb() {
        assert_eq!(
            DeclaredPath::parse("~/.claude/projects"),
            Some(DeclaredPath::Home(PathBuf::from(".claude/projects")))
        );
        assert_eq!(
            DeclaredPath::parse("/opt/x"),
            Some(DeclaredPath::Absolute(PathBuf::from("/opt/x")))
        );
        for refused in ["relative", "~", "~/", "~/../x", "/a/../b", ""] {
            assert_eq!(DeclaredPath::parse(refused), None, "{refused:?}");
        }
    }

    #[test]
    fn an_egress_entry_is_a_host_and_a_port_that_defaults_to_443() {
        let entry = EgressEntry::parse("API.Example.com").unwrap();
        assert!(entry.admits("api.example.com", 443));
        assert!(!entry.admits("api.example.com", 80));
        assert!(
            EgressEntry::parse("example.com:80")
                .unwrap()
                .admits("example.com", 80)
        );
        let every_port = EgressEntry::parse("example.com:*").unwrap();
        assert!(every_port.admits("Example.com", 80));
        assert!(every_port.admits("example.com", 8443));
        assert!(!every_port.admits("api.example.com", 443));
        for refused in ["", "a b", "x:notaport", "https://x", "*.example.com", ":*"] {
            assert_eq!(EgressEntry::parse(refused), None, "{refused:?}");
        }
    }

    fn send(port: u16, request: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut answer = String::new();
        let _ = stream.read_to_string(&mut answer);
        answer
    }
}
