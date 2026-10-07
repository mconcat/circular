
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvError, RecvTimeoutError, SyncSender, TryRecvError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use circular_core::{Boundary, Ceilings, Value, encode};
use circular_protocol::{
    DecodedEnvelopeFrame, EventInjectionVerb, INITIAL_PROTOCOL_VERSION, StableVerb,
    encode_envelope_frame,
};
use circular_transport::is_socket_timeout;
use std::collections::{BTreeMap, btree_map::Entry};
use subtle::{Choice, ConstantTimeEq};

const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = circular_core::MAX_TEXT_BODY_BYTES;
const IO_DEADLINE: Duration = Duration::from_secs(5);
const REQUEST_DEADLINE: Duration = Duration::from_secs(30);
const WEBHOOK_CORRELATION: u32 = 0;
const WEBHOOK_INJECT: StableVerb = StableVerb::EventInjection(EventInjectionVerb::Inject);

#[derive(Clone, Copy)]
struct RequestTimeouts {
    io: Duration,
    request: Duration,
}

pub(crate) struct WebhookInjectAdapter {
    mount: String,
}

impl WebhookInjectAdapter {
    pub(crate) fn new(mount: impl Into<String>) -> Self {
        Self {
            mount: mount.into(),
        }
    }

    pub(crate) fn envelope(
        &self,
        body: &str,
        idempotency: &[u8],
    ) -> Result<WebhookInjectEnvelope, String> {
        let payload = Value::object([("body", Value::String(body.to_owned()))])
            .expect("webhook body field is unique");
        let inject = Value::object([
            ("idempotency", Value::bytes(idempotency.to_vec())),
            (
                "mount",
                circular_protocol::declaration_payload::PlanExportKey::root(self.mount.clone())
                    .to_value(),
            ),
            ("payload", payload),
        ])
        .expect("Inject fields are unique");
        let payload = encode(&inject, Ceilings::for_boundary(Boundary::Wire))
            .map_err(|error| format!("could not encode webhook Inject payload: {error:?}"))?;
        let frame = encode_envelope_frame(
            INITIAL_PROTOCOL_VERSION,
            WEBHOOK_INJECT,
            WEBHOOK_CORRELATION,
            &payload,
        )
        .map_err(|error| format!("could not build webhook Inject envelope: {error}"))?;
        Ok(WebhookInjectEnvelope { frame })
    }
}

pub(crate) struct WebhookInjectEnvelope {
    frame: Vec<u8>,
}

impl WebhookInjectEnvelope {
    pub(crate) fn decode(&self) -> DecodedEnvelopeFrame<'_> {
        circular_protocol::decode_envelope_frame(&self.frame)
            .expect("the adapter retains the complete envelope it just encoded")
    }
}

pub struct WebhookRequest {
    mount: String,
    body: String,
    idempotency: Vec<u8>,
    response: SyncSender<WebhookResponse>,
}

impl WebhookRequest {
    pub fn mount(&self) -> &str {
        &self.mount
    }

    pub fn body(&self) -> &str {
        &self.body
    }

    pub fn idempotency(&self) -> &[u8] {
        &self.idempotency
    }

    pub fn answer(self, response: WebhookResponse) {
        let _ = self.response.send(response);
    }
}

pub enum WebhookResponse {
    Accepted,
    NoRun,
    UnknownMount,
    NotAccepting,
    Failed,
}

fn accept_is_transient(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::Interrupted | std::io::ErrorKind::ConnectionAborted
    ) || matches!(
        error.raw_os_error(),
        Some(nix::libc::EMFILE | nix::libc::ENFILE | nix::libc::ENOBUFS)
    )
}

pub struct WebhookGateway {
    address: SocketAddr,
    requests: Receiver<WebhookRequest>,
    shutdown: Arc<AtomicBool>,
    shutdown_wake: Arc<engine::wake::Wake>,
    worker: Option<JoinHandle<()>>,
}

impl WebhookGateway {
    pub fn bind(
        address: SocketAddr,
        secret_vault: Arc<engine::SecretVault>,
        bearer_resource: Box<str>,
    ) -> std::io::Result<Self> {
        if !is_loopback_bind_address(address) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "webhook bind address {address} is not loopback; Remote trust classification is not assembled"
                ),
            ));
        }
        if !engine_secrets::engine_integration::vault_contains_resource(
            &secret_vault,
            &bearer_resource,
        ) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("secret vault resource {bearer_resource:?} is missing"),
            ));
        }
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let (sender, requests) = mpsc::sync_channel(64);
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let shutdown_wake = Arc::new(engine::wake::Wake::new()?);
        let worker_wake = Arc::clone(&shutdown_wake);
        let worker = std::thread::spawn(move || {
            use std::os::fd::AsFd;
            let sequence = AtomicU64::new(1);
            let backoff = engine::effect_retry::ReceiverBackoff::default();
            while !worker_shutdown.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, peer)) => {
                        if stream.set_nonblocking(false).is_err() {
                            continue;
                        }
                        handle_connection(
                            &mut stream,
                            peer,
                            &secret_vault,
                            &bearer_resource,
                            &sender,
                            &sequence,
                            &backoff,
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        worker_wake.drain();
                        if worker_shutdown.load(Ordering::Relaxed) {
                            break;
                        }
                        if engine::wake::wait_readable(&[listener.as_fd(), worker_wake.as_fd()])
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(error) if accept_is_transient(&error) => {
                        eprintln!("circular-daemon: webhook accept waits: {error}");
                        worker_wake.drain();
                        if worker_shutdown.load(Ordering::Relaxed) {
                            break;
                        }
                        if engine::wake::wait_readable(&[listener.as_fd(), worker_wake.as_fd()])
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(error) => {
                        eprintln!("circular-daemon: webhook endpoint stopped accepting: {error}");
                        break;
                    }
                }
            }
        });
        Ok(Self {
            address,
            requests,
            shutdown,
            shutdown_wake,
            worker: Some(worker),
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub(crate) fn recv(&self) -> Result<WebhookRequest, RecvError> {
        self.requests.recv()
    }

    pub(crate) fn closing(&self) -> WebhookClosing {
        WebhookClosing {
            shutdown: Arc::clone(&self.shutdown),
            shutdown_wake: Arc::clone(&self.shutdown_wake),
        }
    }

    pub fn try_recv(&self) -> Result<WebhookRequest, TryRecvError> {
        self.requests.try_recv()
    }
}

pub(crate) struct WebhookClosing {
    shutdown: Arc<AtomicBool>,
    shutdown_wake: Arc<engine::wake::Wake>,
}

impl Drop for WebhookClosing {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.shutdown_wake.notify();
    }
}

fn is_loopback_bind_address(address: SocketAddr) -> bool {
    address.ip().is_loopback()
}

impl Drop for WebhookGateway {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.shutdown_wake.notify();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn handle_connection(
    stream: &mut TcpStream,
    peer: SocketAddr,
    secret_vault: &engine::SecretVault,
    bearer_resource: &str,
    sender: &SyncSender<WebhookRequest>,
    sequence: &AtomicU64,
    backoff: &engine::effect_retry::ReceiverBackoff,
) {
    handle_connection_with_timeouts(
        stream,
        peer,
        secret_vault,
        bearer_resource,
        sender,
        sequence,
        RequestTimeouts {
            io: IO_DEADLINE,
            request: REQUEST_DEADLINE,
        },
        backoff,
    );
}

#[allow(clippy::too_many_arguments)]
fn handle_connection_with_timeouts(
    stream: &mut TcpStream,
    peer: SocketAddr,
    secret_vault: &engine::SecretVault,
    bearer_resource: &str,
    sender: &SyncSender<WebhookRequest>,
    sequence: &AtomicU64,
    timeouts: RequestTimeouts,
    backoff: &engine::effect_retry::ReceiverBackoff,
) {
    let _ = stream.set_read_timeout(Some(timeouts.io));
    let _ = stream.set_write_timeout(Some(timeouts.io));
    let request = match read_request(
        stream,
        peer,
        secret_vault,
        bearer_resource,
        sequence,
        timeouts.request,
    ) {
        Ok(request) => request,
        Err(rejection) => {
            respond_and_close(stream, rejection.status, timeouts.io);
            return;
        }
    };
    let (response, received) = mpsc::sync_channel(1);
    let request = WebhookRequest {
        mount: request.mount,
        body: request.body,
        idempotency: request.idempotency,
        response,
    };
    if sender.send(request).is_err() {
        respond_and_close_after(stream, 503, Some(backoff.refused()), timeouts.io);
        return;
    }
    match status_for_response(received.recv_timeout(IO_DEADLINE)) {
        503 => respond_and_close_after(stream, 503, Some(backoff.refused()), timeouts.io),
        202 => {
            backoff.accepted();
            respond_and_close(stream, 202, timeouts.io);
        }
        status => respond_and_close(stream, status, timeouts.io),
    }
}

fn status_for_response(response: Result<WebhookResponse, RecvTimeoutError>) -> u16 {
    match response {
        Ok(WebhookResponse::Accepted) => 202,
        Ok(WebhookResponse::NoRun | WebhookResponse::NotAccepting) => 503,
        Ok(WebhookResponse::UnknownMount) => 404,
        Ok(WebhookResponse::Failed) | Err(RecvTimeoutError::Disconnected) => 500,
        Err(RecvTimeoutError::Timeout) => 504,
    }
}

struct ParsedRequest {
    mount: String,
    body: String,
    idempotency: Vec<u8>,
}

#[derive(Debug, PartialEq)]
struct RequestRejection {
    status: u16,
}

impl From<u16> for RequestRejection {
    fn from(status: u16) -> Self {
        Self { status }
    }
}

fn read_request(
    stream: &mut impl Read,
    peer: SocketAddr,
    secret_vault: &engine::SecretVault,
    bearer_resource: &str,
    sequence: &AtomicU64,
    request_deadline: Duration,
) -> Result<ParsedRequest, RequestRejection> {
    let started = Instant::now();
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end = loop {
        if started.elapsed() >= request_deadline {
            return Err(408.into());
        }
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if is_socket_timeout(&error) => {
                if started.elapsed() >= request_deadline {
                    return Err(408.into());
                }
                continue;
            }
            Err(_) => return Err(400.into()),
        };
        if started.elapsed() >= request_deadline {
            return Err(408.into());
        }
        if read == 0 {
            return Err(400.into());
        }
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            if position + 4 > MAX_HEADER_BYTES {
                return Err(431.into());
            }
            break position + 4;
        }
        if bytes.len() > MAX_HEADER_BYTES {
            return Err(431.into());
        }
    };
    let headers = std::str::from_utf8(&bytes[..header_end]).map_err(|_| 400_u16)?;
    let mut lines = headers.split("\r\n");
    let mut request_line = lines.next().ok_or(400_u16)?.split_whitespace();
    if request_line.next() != Some("POST") {
        return Err(405.into());
    }
    let path = request_line.next().ok_or(400_u16)?;
    if request_line.next() != Some("HTTP/1.1") || request_line.next().is_some() {
        return Err(400.into());
    }
    let mount = path
        .strip_prefix("/v1/ingress/")
        .filter(|mount| !mount.is_empty() && !mount.contains('/'))
        .ok_or(404_u16)?
        .to_owned();
    let mut headers = BTreeMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or(400_u16)?;
        if name.is_empty()
            || !name.is_ascii()
            || name.bytes().any(|byte| byte.is_ascii_whitespace())
        {
            return Err(400.into());
        }
        match headers.entry(name.to_ascii_lowercase()) {
            Entry::Vacant(entry) => {
                entry.insert(value.trim());
            }
            Entry::Occupied(_) => return Err(400.into()),
        }
    }
    let presented = headers
        .get("authorization")
        .and_then(|value| value.strip_prefix("Bearer "));
    let authorized = presented.is_some_and(|presented| {
        engine_secrets::engine_integration::vault_action_material(secret_vault, bearer_resource)
            .is_some_and(|expected| constant_time_token_eq(expected, presented.as_bytes()))
    });
    if !authorized {
        return Err(401.into());
    }
    if headers.contains_key("transfer-encoding") || headers.contains_key("content-encoding") {
        return Err(415.into());
    }
    let length = headers.get("content-length").ok_or(411_u16)?;
    if length.is_empty() || !length.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(400.into());
    }
    let content_length = length.parse::<usize>().map_err(|_| 400_u16)?;
    if content_length > MAX_BODY_BYTES {
        return Err(413.into());
    }
    let idempotency = headers
        .get("idempotency-key")
        .filter(|value| !value.is_empty())
        .map(|value| value.as_bytes().to_vec());
    let mut body = bytes[header_end..].to_vec();
    if body.len() > content_length {
        body.truncate(content_length);
    }
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let capacity = remaining.min(buffer.len());
        if started.elapsed() >= request_deadline {
            return Err(408.into());
        }
        let read = match stream.read(&mut buffer[..capacity]) {
            Ok(read) => read,
            Err(error) if is_socket_timeout(&error) => {
                if started.elapsed() >= request_deadline {
                    return Err(408.into());
                }
                continue;
            }
            Err(_) => return Err(400.into()),
        };
        if started.elapsed() >= request_deadline {
            return Err(408.into());
        }
        if read == 0 {
            return Err(400.into());
        }
        body.extend_from_slice(&buffer[..read]);
    }
    let body = String::from_utf8(body).map_err(|_| 400_u16)?;
    let idempotency = idempotency.unwrap_or_else(|| {
        format!(
            "webhook:{peer}:{}",
            sequence.fetch_add(1, Ordering::Relaxed)
        )
        .into_bytes()
    });
    Ok(ParsedRequest {
        mount,
        body,
        idempotency,
    })
}

/// Fixed header-bounded comparison work. Both unequal-length and unequal-byte
/// tokens pass through the same subtle operations; only the final Choice becomes
/// a bool. This is a structural guarantee, not a wall-clock timing assertion.
fn constant_time_token_eq(expected: &[u8], presented: &[u8]) -> bool {
    let within_limit = Choice::from(
        ((expected.len() <= MAX_HEADER_BYTES) & (presented.len() <= MAX_HEADER_BYTES)) as u8,
    );
    let mut equal = expected.len().ct_eq(&presented.len()) & within_limit;
    for index in 0..MAX_HEADER_BYTES {
        let expected = expected.get(index).copied().unwrap_or_default();
        let presented = presented.get(index).copied().unwrap_or_default();
        equal &= expected.ct_eq(&presented);
    }
    bool::from(equal)
}

fn respond_and_close(stream: &mut TcpStream, status: u16, io_timeout: Duration) {
    respond_and_close_after(stream, status, None, io_timeout);
}

fn respond_and_close_after(
    stream: &mut TcpStream,
    status: u16,
    retry_after: Option<u64>,
    io_timeout: Duration,
) {
    write_response_after(stream, status, retry_after);
    engine::http_close::close_after_response(stream, MAX_BODY_BYTES, io_timeout);
}

fn write_response_after(stream: &mut impl Write, status: u16, retry_after: Option<u64>) {
    let reason = match status {
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Error",
    };
    let body = format!("{status} {reason}\n");
    let retry_after = retry_after
        .map(|seconds| format!("Retry-After: {seconds}\r\n"))
        .unwrap_or_default();
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain\r\n{retry_after}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_protocol::declaration_payload::decode_inject;
    use circular_protocol::{EventInjectionVerb, Partition};
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::path::PathBuf;

    static NEXT_VAULT: AtomicU64 = AtomicU64::new(0);
    const TEST_BEARER_RESOURCE: &str = "configured.webhook.bearer";

    struct VaultFixture(PathBuf);

    impl VaultFixture {
        fn load(entries: &[(&str, &[u8])]) -> Arc<engine::SecretVault> {
            let sequence = NEXT_VAULT.fetch_add(1, Ordering::Relaxed);
            let fixture = Self(std::env::temp_dir().join(format!(
                "circular-webhook-vault-{}-{sequence}",
                std::process::id()
            )));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&fixture.0)
                .expect("webhook vault fixture root");
            for (name, value) in entries {
                let path = fixture.0.join(name);
                std::fs::write(&path, value).expect("webhook vault fixture value");
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                    .expect("webhook vault fixture value mode");
            }
            Arc::new(engine::SecretVault::load(&fixture.0).expect("webhook vault fixture loads"))
        }
    }

    impl Drop for VaultFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn webhook_vault(token: &[u8]) -> Arc<engine::SecretVault> {
        VaultFixture::load(&[(TEST_BEARER_RESOURCE, token)])
    }

    #[test]
    fn adapter_builds_only_event_injection_inject_envelopes() {
        let adapter = WebhookInjectAdapter::new("incidents");
        let envelope = adapter
            .envelope(r#"{"body":"disk full"}"#, b"incident-1")
            .expect("webhook request becomes an envelope");
        let decoded = envelope.decode();

        assert_eq!(decoded.header().partition(), Partition::EventInjection);
        assert_eq!(
            decoded.header().verb(),
            StableVerb::EventInjection(EventInjectionVerb::Inject)
        );
        let inject = decode_inject(decoded.payload(), Ceilings::for_boundary(Boundary::Wire))
            .expect("the fixed envelope carries an Inject payload");
        assert_eq!(
            inject.mount,
            circular_protocol::declaration_payload::PlanExportKey {
                scope: Vec::new(),
                local: "incidents".to_owned(),
            }
        );
        assert_eq!(inject.idempotency, b"incident-1");
        assert_eq!(
            inject.payload,
            Value::object([("body", Value::String(r#"{"body":"disk full"}"#.to_owned()))])
                .expect("one field")
        );
    }

    #[test]
    fn constant_time_compare_distinguishes_length_prefix_and_exact_match() {
        assert!(!constant_time_token_eq(b"secret", b"secret-longer"));
        assert!(!constant_time_token_eq(b"secret", b"secrex"));
        assert!(constant_time_token_eq(b"secret", b"secret"));
    }

    fn parse_bytes(bytes: Vec<u8>) -> (Result<ParsedRequest, RequestRejection>, u64) {
        struct FragmentedRead(std::io::Cursor<Vec<u8>>);
        impl Read for FragmentedRead {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                let length = buffer.len().min(4095);
                self.0.read(&mut buffer[..length])
            }
        }
        let mut stream = FragmentedRead(std::io::Cursor::new(bytes));
        let result = read_request(
            &mut stream,
            "127.0.0.1:1".parse().unwrap(),
            &webhook_vault(b"secret"),
            TEST_BEARER_RESOURCE,
            &AtomicU64::new(1),
            Duration::from_secs(5),
        );
        (result, stream.0.position())
    }

    fn request(headers: &str, body: &[u8]) -> Vec<u8> {
        let mut bytes =
            format!("POST /v1/ingress/incidents HTTP/1.1\r\n{headers}\r\n").into_bytes();
        bytes.extend_from_slice(body);
        bytes
    }

    #[test]
    fn duplicate_headers_reject_both_orders_and_case_variants() {
        for headers in [
            "Authorization: Bearer secret\r\naUtHoRiZaTiOn: Bearer wrong\r\nContent-Length: 0\r\n",
            "Authorization: Bearer wrong\r\nAuthorization: Bearer secret\r\nContent-Length: 0\r\n",
            "Authorization: Bearer secret\r\nContent-Length: 0\r\ncontent-length: 0\r\n",
            "Authorization: Bearer secret\r\nContent-Length: 0\r\nContent-Length: 1\r\n",
            "Authorization: Bearer secret\r\nContent-Length: 1\r\nContent-Length: 0\r\n",
            "Authorization: Bearer secret\r\nContent-Length: 0\r\nIdempotency-Key: first\r\nidempotency-key: second\r\n",
            "Authorization: Bearer secret\r\nContent-Length: 0\r\nX-Trace: first\r\nx-trace: second\r\n",
        ] {
            let (result, _) = parse_bytes(request(headers, b""));
            assert_eq!(result.err().unwrap().status, 400);
        }
        let (result, _) = parse_bytes(request(
            "Authorization: Bearer secret\r\nContent-Length: 2\r\nIdempotency-Key: first\r\n",
            b"{}",
        ));
        let parsed = result.unwrap();
        assert_eq!(parsed.idempotency, b"first");
        assert_eq!(parsed.body, "{}");
        assert_eq!(parsed.mount, "incidents");
    }

    #[test]
    fn body_limit_is_inclusive_and_excess_is_rejected_before_body_read() {
        let body = vec![b'x'; 1024 * 1024];
        let (result, _) = parse_bytes(request(
            "Authorization: Bearer secret\r\nContent-Length: 1048576\r\n",
            &body,
        ));
        assert_eq!(result.unwrap().body.as_bytes(), body);
        let headers = request(
            "Authorization: Bearer secret\r\nContent-Length: 1048577\r\n",
            b"",
        );
        let header_bytes = headers.len();
        let (result, read) = parse_bytes(headers);
        assert_eq!(result.err().unwrap(), RequestRejection { status: 413 });
        assert_eq!(
            read as usize, header_bytes,
            "reject without reading the declared body"
        );
    }

    #[test]
    fn ambiguous_body_framing_uses_existing_http_rejections() {
        for (extra, status) in [
            ("", 411),
            ("Content-Length: -1\r\n", 400),
            ("Content-Length: +1\r\n", 400),
            ("Content-Length: abc\r\n", 400),
            ("Content-Length: 0\r\nTransfer-Encoding: chunked\r\n", 415),
            ("Content-Length: 0\r\nContent-Encoding: gzip\r\n", 415),
        ] {
            let (result, _) = parse_bytes(request(
                &format!("Authorization: Bearer secret\r\n{extra}"),
                b"",
            ));
            assert_eq!(result.err().unwrap().status, status);
        }
    }

    #[test]
    fn header_limit_counts_headers_even_when_the_last_read_contains_body() {
        for header_size in [16_300, 16_384, 16_385] {
            let headers = "Authorization: Bearer secret\r\nContent-Length: 1000\r\nX-Pad: ";
            let unpadded = request(&format!("{headers}\r\n"), b"").len();
            let bytes = request(
                &format!("{headers}{}\r\n", "x".repeat(header_size - unpadded)),
                &vec![b'b'; 1000],
            );
            let (result, _) = parse_bytes(bytes);
            if header_size <= 16_384 {
                assert_eq!(result.unwrap().body, "b".repeat(1000));
            } else {
                assert_eq!(result.err().unwrap().status, 431);
            }
        }
    }

    #[test]
    fn oversized_rejection_has_literal_response() {
        let mut response = Vec::new();
        write_response_after(&mut response, 413, None);
        assert_eq!(response, b"HTTP/1.1 413 Content Too Large\r\nContent-Type: text/plain\r\nContent-Length: 22\r\nConnection: close\r\n\r\n413 Content Too Large\n");
    }

    fn read_declared_response(stream: &mut TcpStream) -> String {
        let mut bytes = Vec::new();
        let mut byte = [0_u8; 1];
        let header_end = loop {
            let read = stream.read(&mut byte).expect("read response headers");
            assert_ne!(read, 0, "gateway closed before writing a response");
            bytes.push(byte[0]);
            if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                break position + 4;
            }
        };
        let headers = std::str::from_utf8(&bytes[..header_end])
            .expect("UTF-8 response headers")
            .to_owned();
        let length = headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length:"))
            .and_then(|value| value.trim().parse::<usize>().ok())
            .expect("declared response length");
        let mut body = vec![0_u8; length];
        stream
            .read_exact(&mut body)
            .expect("read the declared response body");
        bytes.extend_from_slice(&body);
        String::from_utf8(bytes).expect("UTF-8 response")
    }

    #[test]
    fn gateway_requires_the_configured_vault_resource() {
        let vault = VaultFixture::load(&[("another.secret", b"not-a-webhook-token")]);
        let result = WebhookGateway::bind(
            "127.0.0.1:0".parse().expect("loopback address"),
            vault,
            TEST_BEARER_RESOURCE.into(),
        );
        let error = match result {
            Ok(_) => panic!("gateway must not bind without its configured bearer resource"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains(TEST_BEARER_RESOURCE));
    }

    #[test]
    fn gateway_accepts_loopback_and_rejects_every_non_loopback_shape() {
        assert!(is_loopback_bind_address(
            "127.0.0.1:0".parse().expect("IPv4 loopback address")
        ));
        assert!(is_loopback_bind_address(
            "[::1]:0".parse().expect("IPv6 loopback address")
        ));
        let gateway = WebhookGateway::bind(
            "127.0.0.1:0".parse().expect("IPv4 loopback address"),
            webhook_vault(b"secret"),
            TEST_BEARER_RESOURCE.into(),
        )
        .expect("IPv4 loopback binds");
        assert!(gateway.address().ip().is_loopback());

        for address in ["0.0.0.0:0", "[::]:0", "192.0.2.1:0"] {
            let result = WebhookGateway::bind(
                address.parse().expect("test socket address"),
                webhook_vault(b"secret"),
                TEST_BEARER_RESOURCE.into(),
            );
            let error = match result {
                Ok(_) => panic!("non-loopback address {address} must be rejected"),
                Err(error) => error,
            };
            assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
            assert!(error.to_string().contains("Remote trust classification"));
        }
    }

    #[test]
    fn owner_answer_timeout_and_disconnection_have_distinct_statuses() {
        assert_eq!(status_for_response(Err(RecvTimeoutError::Timeout)), 504);
        assert_eq!(
            status_for_response(Err(RecvTimeoutError::Disconnected)),
            500
        );
    }
}
