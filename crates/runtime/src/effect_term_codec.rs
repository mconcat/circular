
use crate::{
    AgentHarnessName, AgentInvokeSpec, AgentPayload, AgentSessionId, AgentStepRequest,
    AgentToolCallId, AgentToolResult, ApprovalSpec, ApprovalTicket, ByteRange, Capability,
    ConcreteExternalEffectTag, ConcreteOutcomePayload, Divergence, EffectFailure, EffectTerm,
    FileReadSpec, FileWriteMode, FileWriteSpec, HttpHeader, HttpHeaderValue, HttpMethod,
    HttpRequestSpec, HttpUrl, InterpreterFault, NormalizedPath, NotificationChannel,
    NotificationSpec, ProcessResult, ProcessSpec, ProgramName,
};
use crate::{
    BindRequest, DiscoverRequest, InboundPolicy, OpaqueProviderFields, PeerActorIncarnation,
    PeerAdapterName, PeerAddress, PeerBindingId, PeerBody, PeerCursor, PeerDisplayName,
    PeerEffectTerm, PeerId, PeerMessageId, PeerRealmId, SendRequest,
};
use circular_core::{ByteReader, LittleEndian};
use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;

pub const TERM_CODEC_VERSION: u8 = 1;

const PEER_DISCOVER_TAG: u8 = 8;
const PEER_BIND_TAG: u8 = 9;
const PEER_SEND_TAG: u8 = 10;
const PEER_UNBIND_TAG: u8 = 11;
const PEER_RECEIVE_TAG: u8 = 12;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TermCodecError {
    Empty,
    UnknownVersion { found: u8 },
    UnknownTag { position: &'static str, found: u8 },
    /// A checked writer cannot represent this term in its existing u32 lengths.
    LengthOverflow,
    Truncated { position: &'static str },
    TrailingBytes { remaining: usize },
    RejectedByConstructor { position: &'static str },
}

#[must_use]
pub fn encode_term(term: &EffectTerm) -> Vec<u8> {
    let mut output = Vec::new();
    encode_term_into(&mut output, term);
    output
}

/// Check the unchanged term grammar before allocating or invoking its infallible writer.
/// The outcome record uses this to reject oversized input before append and outcome-hook delivery.
pub fn try_encode_term(term: &EffectTerm) -> Result<Vec<u8>, TermCodecError> {
    let mut size = TermSize::default();
    encode_term_into(&mut size, term);
    if size.invalid_identity {
        return Err(rejected("effect_id"));
    }
    if size.overflow {
        return Err(TermCodecError::LengthOverflow);
    }
    let mut output = Vec::with_capacity(size.bytes);
    encode_term_into(&mut output, term);
    Ok(output)
}

pub(crate) trait TermOutput {
    fn reject_identity(&mut self);
    fn push(&mut self, byte: u8);
    fn extend_from_slice(&mut self, bytes: &[u8]);
    fn count(&mut self, count: usize);
}
impl TermOutput for Vec<u8> {
    fn reject_identity(&mut self) {
        panic!("invalid canonical effect identity");
    }
    fn push(&mut self, byte: u8) {
        Vec::push(self, byte);
    }
    fn extend_from_slice(&mut self, bytes: &[u8]) {
        Vec::extend_from_slice(self, bytes);
    }
    fn count(&mut self, count: usize) {
        let count = u32::try_from(count).expect("canonical term count exceeds u32");
        Vec::extend_from_slice(self, &count.to_le_bytes());
    }
}
#[derive(Default)]
struct TermSize {
    invalid_identity: bool,
    bytes: usize,
    overflow: bool,
}
impl TermSize {
    fn add(&mut self, bytes: usize) {
        match self.bytes.checked_add(bytes) {
            Some(total) if total <= u32::MAX as usize => self.bytes = total,
            _ => self.overflow = true,
        }
    }
}
impl TermOutput for TermSize {
    fn reject_identity(&mut self) {
        self.invalid_identity = true;
    }
    fn push(&mut self, _: u8) {
        self.add(1);
    }
    fn extend_from_slice(&mut self, bytes: &[u8]) {
        self.add(bytes.len());
    }
    fn count(&mut self, count: usize) {
        self.overflow |= u32::try_from(count).is_err();
        self.add(4);
    }
}

fn encode_term_into(output: &mut impl TermOutput, term: &EffectTerm) {
    output.push(TERM_CODEC_VERSION);
    match term {
        EffectTerm::Peer(term) => encode_peer(output, term),
        EffectTerm::Http { ticket, spec } => {
            output.push(1);
            encode_ticket(output, ticket.as_ref());
            output.push(spec.method().tag());
            put_str(output, spec.url().as_str());
            put_str(output, spec.url().host());
            put_count(output, spec.headers().len());
            for header in spec.headers() {
                put_str(output, header.name());
                match header.value() {
                    HttpHeaderValue::Plain(value) => {
                        output.push(1);
                        put_str(output, value);
                    }
                    HttpHeaderValue::Secret(resource) => {
                        output.push(2);
                        put_str(output, resource);
                    }
                }
            }
            put_bytes(output, spec.body());
        }
        EffectTerm::FileRead { ticket, spec } => {
            output.push(2);
            encode_ticket(output, ticket.as_ref());
            put_bytes(output, spec.path().as_path().as_os_str().as_bytes());
            match spec.range() {
                None => output.push(0),
                Some(range) => {
                    output.push(1);
                    put_u64(output, range.start());
                    put_u64(output, range.length());
                }
            }
        }
        EffectTerm::FileWrite { ticket, spec } => {
            output.push(3);
            encode_ticket(output, ticket.as_ref());
            put_bytes(output, spec.path().as_path().as_os_str().as_bytes());
            put_bytes(output, spec.body());
            output.push(spec.mode().tag());
        }
        EffectTerm::Spawn { ticket, spec } => {
            output.push(4);
            encode_ticket(output, ticket.as_ref());
            put_str(output, spec.program().as_str());
            let arguments = spec.arguments();
            put_count(output, arguments.len());
            for argument in arguments {
                put_str(output, argument);
            }
            put_bytes(output, spec.stdin());
        }
        EffectTerm::Notify { ticket, spec } => {
            output.push(5);
            encode_ticket(output, ticket.as_ref());
            put_str(output, spec.channel().as_str());
            put_str(output, spec.title());
            put_str(output, spec.body());
        }
        EffectTerm::AgentInvoke { ticket, invoke } => {
            output.push(6);
            encode_ticket(output, ticket.as_ref());
            encode_agent_invoke(output, invoke);
        }
        EffectTerm::RequestApproval(spec) => {
            output.push(7);
            put_effect_id(output, spec.target_effect());
        }
    }
}

pub fn decode_term(bytes: &[u8]) -> Result<EffectTerm, TermCodecError> {
    let Some((&version, _)) = bytes.split_first() else {
        return Err(TermCodecError::Empty);
    };
    if version != TERM_CODEC_VERSION {
        return Err(TermCodecError::UnknownVersion { found: version });
    }

    let mut reader = TermReader::new(&bytes[1..]);
    let tag = read_u8(&mut reader, "term.tag")?;
    let term = match tag {
        1 => decode_http(&mut reader)?,
        2 => decode_file_read(&mut reader)?,
        3 => decode_file_write(&mut reader)?,
        4 => decode_spawn(&mut reader)?,
        5 => decode_notify(&mut reader)?,
        6 => decode_agent_term(&mut reader)?,
        7 => decode_request_approval(&mut reader)?,
        PEER_DISCOVER_TAG | PEER_BIND_TAG | PEER_SEND_TAG | PEER_UNBIND_TAG | PEER_RECEIVE_TAG => {
            EffectTerm::Peer(decode_peer(tag, &mut reader)?)
        }
        found => {
            return Err(TermCodecError::UnknownTag {
                position: "term.tag",
                found,
            });
        }
    };

    if reader.finished() {
        Ok(term)
    } else {
        Err(TermCodecError::TrailingBytes {
            remaining: reader.remaining(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TermApproval {
    Request,
    Ticket(ApprovalTicket),
}

pub fn decode_term_approval(bytes: &[u8]) -> Result<Option<TermApproval>, TermCodecError> {
    let Some((&version, rest)) = bytes.split_first() else {
        return Err(TermCodecError::Empty);
    };
    if version != TERM_CODEC_VERSION {
        return Err(TermCodecError::UnknownVersion { found: version });
    }
    let mut reader = TermReader::new(rest);
    match read_u8(&mut reader, "term.tag")? {
        1..=6 => Ok(decode_ticket(&mut reader)?.map(TermApproval::Ticket)),
        7 => Ok(Some(TermApproval::Request)),
        PEER_DISCOVER_TAG | PEER_BIND_TAG | PEER_SEND_TAG | PEER_UNBIND_TAG | PEER_RECEIVE_TAG => {
            Ok(None)
        }
        found => Err(TermCodecError::UnknownTag {
            position: "term.tag",
            found,
        }),
    }
}

fn encode_peer(output: &mut impl TermOutput, term: &PeerEffectTerm) {
    match term {
        PeerEffectTerm::Discover(request) => {
            output.push(PEER_DISCOVER_TAG);
            put_str(output, request.adapter().as_str());
            put_bytes(output, request.realm().as_bytes());
            encode_peer_name(output, request.display_name());
        }
        PeerEffectTerm::Bind(request) => {
            output.push(PEER_BIND_TAG);
            put_str(output, request.adapter().as_str());
            put_bytes(output, request.actor().actor());
            put_bytes(output, request.actor().incarnation());
            put_bytes(output, request.realm().as_bytes());
            encode_peer_name(output, request.requested_name());
            match request.inbound_policy() {
                InboundPolicy::AnyKnownPeer => output.push(1),
                InboundPolicy::Exact(addresses) => {
                    output.push(2);
                    put_count(output, addresses.len());
                    for address in addresses {
                        encode_peer_address(output, address);
                    }
                }
            }
            put_u64(
                output,
                u64::try_from(request.inbox_capacity().get()).expect("capacity fits u64"),
            );
        }
        PeerEffectTerm::Send(request) => {
            output.push(PEER_SEND_TAG);
            put_bytes(output, request.binding().as_bytes());
            put_bytes(output, request.message().as_bytes());
            encode_peer_address(output, request.target());
            put_str(output, request.body().as_str());
            match request.correlation() {
                None => output.push(0),
                Some(message) => {
                    output.push(1);
                    put_bytes(output, message.as_bytes());
                }
            }
            put_count(output, request.provider_fields().iter().len());
            for (key, value) in request.provider_fields().iter() {
                put_str(output, key);
                put_bytes(output, value);
            }
        }
        PeerEffectTerm::Receive { binding, after } => {
            output.push(PEER_RECEIVE_TAG);
            put_bytes(output, binding.as_bytes());
            match after {
                None => output.push(0),
                Some(cursor) => {
                    output.push(1);
                    output.extend_from_slice(&cursor.get().to_le_bytes());
                }
            }
        }
        PeerEffectTerm::Unbind(binding) => {
            output.push(PEER_UNBIND_TAG);
            put_bytes(output, binding.as_bytes());
        }
    }
}

fn encode_peer_name(output: &mut impl TermOutput, name: Option<&PeerDisplayName>) {
    match name {
        None => output.push(0),
        Some(name) => {
            output.push(1);
            put_str(output, name.as_str());
        }
    }
}

fn encode_peer_address(output: &mut impl TermOutput, address: &PeerAddress) {
    put_str(output, address.adapter().as_str());
    put_bytes(output, address.realm().as_bytes());
    put_bytes(output, address.peer().as_bytes());
}

fn decode_peer_name(
    reader: &mut TermReader<'_>,
) -> Result<Option<PeerDisplayName>, TermCodecError> {
    let position = "peer.name";
    match read_u8(reader, position)? {
        0 => Ok(None),
        1 => PeerDisplayName::try_new(read_str(reader, position)?)
            .map(Some)
            .map_err(|_| rejected(position)),
        tag => unknown_tag(position, tag),
    }
}

fn decode_peer_address(reader: &mut TermReader<'_>) -> Result<PeerAddress, TermCodecError> {
    let adapter = PeerAdapterName::try_new(read_str(reader, "peer.address.adapter")?)
        .map_err(|_| rejected("peer.address.adapter"))?;
    let realm = PeerRealmId::try_new(read_sized_bytes(reader, "peer.address.realm")?)
        .map_err(|_| rejected("peer.address.realm"))?;
    let peer = PeerId::try_new(read_sized_bytes(reader, "peer.address.peer")?)
        .map_err(|_| rejected("peer.address.peer"))?;
    Ok(PeerAddress::new(adapter, realm, peer))
}

fn decode_peer(tag: u8, reader: &mut TermReader<'_>) -> Result<PeerEffectTerm, TermCodecError> {
    match tag {
        PEER_DISCOVER_TAG => {
            let adapter = PeerAdapterName::try_new(read_str(reader, "peer.adapter")?)
                .map_err(|_| rejected("peer.adapter"))?;
            let realm = PeerRealmId::try_new(read_sized_bytes(reader, "peer.realm")?)
                .map_err(|_| rejected("peer.realm"))?;
            Ok(PeerEffectTerm::Discover(DiscoverRequest::new(
                adapter,
                realm,
                decode_peer_name(reader)?,
            )))
        }
        PEER_BIND_TAG => {
            let adapter = PeerAdapterName::try_new(read_str(reader, "peer.adapter")?)
                .map_err(|_| rejected("peer.adapter"))?;
            let actor = read_sized_bytes(reader, "peer.actor")?;
            let incarnation = read_sized_bytes(reader, "peer.incarnation")?;
            let actor = PeerActorIncarnation::try_new(actor, incarnation)
                .map_err(|_| rejected("peer.actor_incarnation"))?;
            let realm = PeerRealmId::try_new(read_sized_bytes(reader, "peer.realm")?)
                .map_err(|_| rejected("peer.realm"))?;
            let name = decode_peer_name(reader)?;
            let policy = match read_u8(reader, "peer.inbound_policy")? {
                1 => InboundPolicy::AnyKnownPeer,
                2 => {
                    let count = read_u32(reader, "peer.addresses.count")?;
                    let mut addresses = std::collections::BTreeSet::new();
                    for _ in 0..count {
                        let address = decode_peer_address(reader)?;
                        if addresses.last().is_some_and(|last| last >= &address) {
                            return Err(rejected("peer.addresses.order"));
                        }
                        addresses.insert(address);
                    }
                    InboundPolicy::Exact(addresses)
                }
                found => return unknown_tag("peer.inbound_policy", found),
            };
            let capacity = read_u64(reader, "peer.inbox_capacity")?;
            let capacity = usize::try_from(capacity)
                .ok()
                .and_then(std::num::NonZeroUsize::new)
                .ok_or_else(|| rejected("peer.inbox_capacity"))?;
            Ok(PeerEffectTerm::Bind(BindRequest::new(
                adapter, actor, realm, name, policy, capacity,
            )))
        }
        PEER_SEND_TAG => {
            let binding = PeerBindingId::try_new(read_sized_bytes(reader, "peer.binding")?)
                .map_err(|_| rejected("peer.binding"))?;
            let message = PeerMessageId::try_new(read_sized_bytes(reader, "peer.message")?)
                .map_err(|_| rejected("peer.message"))?;
            let target = decode_peer_address(reader)?;
            let body = PeerBody::try_new(read_str(reader, "peer.body")?)
                .map_err(|_| rejected("peer.body"))?;
            let correlation = match read_u8(reader, "peer.correlation")? {
                0 => None,
                1 => Some(
                    PeerMessageId::try_new(read_sized_bytes(reader, "peer.correlation")?)
                        .map_err(|_| rejected("peer.correlation"))?,
                ),
                found => return unknown_tag("peer.correlation", found),
            };
            let count = read_u32(reader, "peer.provider_fields.count")?;
            let mut fields = Vec::new();
            for _ in 0..count {
                let key = read_str(reader, "peer.provider_fields.key")?;
                let value = read_sized_bytes(reader, "peer.provider_fields.value")?;
                if fields.last().is_some_and(|(previous, _)| previous >= &key) {
                    return Err(rejected("peer.provider_fields.order"));
                }
                fields.push((key, value));
            }
            let fields = OpaqueProviderFields::try_new(fields)
                .map_err(|_| rejected("peer.provider_fields"))?;
            Ok(PeerEffectTerm::Send(SendRequest::new(
                binding,
                message,
                target,
                body,
                correlation,
                fields,
            )))
        }
        PEER_RECEIVE_TAG => {
            let binding = PeerBindingId::try_new(read_sized_bytes(reader, "peer.binding")?)
                .map_err(|_| TermCodecError::RejectedByConstructor {
                    position: "peer.binding",
                })?;
            let after = match read_u8(reader, "peer.after")? {
                0 => None,
                1 => Some(PeerCursor::new(read_u64(reader, "peer.after")?)),
                found => {
                    return Err(TermCodecError::UnknownTag {
                        position: "peer.after",
                        found,
                    });
                }
            };
            Ok(PeerEffectTerm::Receive { binding, after })
        }
        PEER_UNBIND_TAG => PeerBindingId::try_new(read_sized_bytes(reader, "peer.binding")?)
            .map(PeerEffectTerm::Unbind)
            .map_err(|_| rejected("peer.binding")),
        found => unknown_tag("term.tag", found),
    }
}

fn encode_ticket(output: &mut impl TermOutput, ticket: Option<&ApprovalTicket>) {
    match ticket {
        None => output.push(0),
        Some(ticket) => {
            output.push(1);
            put_effect_id(output, ticket.ledger_item());
            put_effect_id(output, ticket.target_effect());
        }
    }
}

fn encode_agent_invoke(output: &mut impl TermOutput, invoke: &AgentInvokeSpec) {
    put_str(output, invoke.harness().as_str());
    match invoke.session() {
        None => output.push(0),
        Some(session) => {
            output.push(1);
            put_str(output, session.harness().as_str());
            put_bytes(output, session.opaque());
        }
    }
    match invoke.request() {
        AgentStepRequest::UserTurn(payload) => {
            output.push(1);
            put_bytes(output, payload.as_bytes());
        }
        AgentStepRequest::ToolResult { call, result } => {
            output.push(2);
            put_bytes(output, call.as_bytes());
            encode_agent_tool_result(output, result);
        }
    }
}

fn encode_agent_tool_result(output: &mut impl TermOutput, result: &AgentToolResult) {
    put_bytes(output, result.call().as_bytes());
    output.push(result.effect().tag());
    match result.result() {
        Ok(payload) => {
            output.push(1);
            encode_concrete_outcome(output, payload);
        }
        Err(failure) => {
            output.push(2);
            encode_effect_failure(output, failure);
        }
    }
}

fn encode_concrete_outcome(output: &mut impl TermOutput, payload: &ConcreteOutcomePayload) {
    match payload {
        ConcreteOutcomePayload::FileBytes(bytes) => {
            output.push(1);
            put_bytes(output, bytes);
        }
        ConcreteOutcomePayload::WrittenLength(length) => {
            output.push(2);
            put_u64(output, *length);
        }
        ConcreteOutcomePayload::ProcessResult(result) => {
            output.push(3);
            put_i32(output, result.exit_code());
            put_bytes(output, result.stdout());
            put_bytes(output, result.stderr());
        }
    }
}

pub(crate) fn encode_effect_failure(output: &mut impl TermOutput, failure: &EffectFailure) {
    match failure {
        EffectFailure::ParameterDenied { capability } => {
            output.push(1);
            let ordinal = Capability::ALL
                .iter()
                .position(|candidate| candidate == capability)
                .expect("Capability::ALL contains every declared capability");
            const _: () = assert!(
                Capability::COUNT < u8::MAX as usize,
                "the capability tag does not fit in one byte"
            );
            #[allow(clippy::cast_possible_truncation)]
            output.push((ordinal + 1) as u8);
        }
        EffectFailure::TransportTerminal => output.push(2),
        EffectFailure::ApprovalRequired => output.push(3),
        EffectFailure::Diverged(divergence) => {
            output.push(4);
            output.push(divergence.tag());
        }
        EffectFailure::EndpointGone => output.push(5),
        EffectFailure::Peer(kind) => {
            output.push(7);
            output.push(kind.tag());
        }
        EffectFailure::InterpreterFault(fault) => {
            output.push(6);
            output.push(fault.tag());
        }
        EffectFailure::RetryExhausted { attempts } => {
            output.push(8);
            output.extend_from_slice(&attempts.to_le_bytes());
        }
        EffectFailure::TransportUnreached => output.push(9),
        EffectFailure::RemoteDeferred => output.push(10),
    }
}

fn decode_http(reader: &mut TermReader<'_>) -> Result<EffectTerm, TermCodecError> {
    let ticket = decode_ticket(reader)?;
    let method = read_tag(reader, "http.method", HttpMethod::from_tag)?;
    let value = read_str(reader, "http.url.value")?;
    let stored_host = read_str(reader, "http.url.host")?;
    let url = HttpUrl::try_new(value).map_err(|_| rejected("http.url.value"))?;
    if url.host() != stored_host {
        return Err(rejected("http.url.host"));
    }

    let header_count = read_u32(reader, "http.headers")?;
    let mut headers = Vec::new();
    for _ in 0..header_count {
        let name = read_str(reader, "http.header.name")?;
        let value_tag = read_u8(reader, "http.header.value")?;
        let header = match value_tag {
            1 => HttpHeader::try_new(name, read_str(reader, "http.header.value")?),
            2 => HttpHeader::try_new_secret(name, read_str(reader, "http.header.value")?),
            found => return unknown_tag("http.header.value", found),
        }
        .map_err(|_| rejected("http.header"))?;
        headers.push(header);
    }
    let body = read_sized_bytes(reader, "http.body")?;
    let spec =
        HttpRequestSpec::try_new(method, url, headers, body).map_err(|_| rejected("http.spec"))?;
    Ok(EffectTerm::Http { ticket, spec })
}

fn decode_file_read(reader: &mut TermReader<'_>) -> Result<EffectTerm, TermCodecError> {
    let ticket = decode_ticket(reader)?;
    let path = decode_path(reader, "file_read.path")?;
    let range = match read_u8(reader, "file_read.range")? {
        0 => None,
        1 => Some(ByteRange::new(
            read_u64(reader, "file_read.range.start")?,
            read_u64(reader, "file_read.range.length")?,
        )),
        found => return unknown_tag("file_read.range", found),
    };
    Ok(EffectTerm::FileRead {
        ticket,
        spec: FileReadSpec::new(path, range),
    })
}

fn decode_file_write(reader: &mut TermReader<'_>) -> Result<EffectTerm, TermCodecError> {
    let ticket = decode_ticket(reader)?;
    let path = decode_path(reader, "file_write.path")?;
    let body = read_sized_bytes(reader, "file_write.body")?;
    let mode = read_tag(reader, "file_write.mode", FileWriteMode::from_tag)?;
    Ok(EffectTerm::FileWrite {
        ticket,
        spec: FileWriteSpec::new(path, body, mode),
    })
}

fn decode_spawn(reader: &mut TermReader<'_>) -> Result<EffectTerm, TermCodecError> {
    let ticket = decode_ticket(reader)?;
    let program = ProgramName::from_normalized(read_str(reader, "spawn.program")?);
    let argument_count = read_u32(reader, "spawn.arguments")?;
    let mut arguments = Vec::new();
    for _ in 0..argument_count {
        arguments.push(Box::<str>::from(read_str(reader, "spawn.argument")?));
    }
    let stdin = read_sized_bytes(reader, "spawn.stdin")?;
    Ok(EffectTerm::Spawn {
        ticket,
        spec: ProcessSpec::new(program, arguments, stdin),
    })
}

fn decode_notify(reader: &mut TermReader<'_>) -> Result<EffectTerm, TermCodecError> {
    let ticket = decode_ticket(reader)?;
    let channel = NotificationChannel::from_normalized(read_str(reader, "notify.channel")?);
    let title = read_str(reader, "notify.title")?;
    let body = read_str(reader, "notify.body")?;
    Ok(EffectTerm::Notify {
        ticket,
        spec: NotificationSpec::new(channel, title, body),
    })
}

fn decode_agent_term(reader: &mut TermReader<'_>) -> Result<EffectTerm, TermCodecError> {
    let ticket = decode_ticket(reader)?;
    let harness = decode_agent_name(reader, "agent.harness")?;
    let session = match read_u8(reader, "agent.session")? {
        0 => None,
        1 => {
            let session_harness = decode_agent_name(reader, "agent.session.harness")?;
            let opaque = read_sized_bytes(reader, "agent.session.opaque")?;
            Some(AgentSessionId::new(session_harness, opaque))
        }
        found => return unknown_tag("agent.session", found),
    };
    let request = decode_agent_request(reader)?;
    let invoke = AgentInvokeSpec::try_new(harness, session, request)
        .map_err(|_| rejected("agent.invoke"))?;
    Ok(EffectTerm::AgentInvoke { ticket, invoke })
}

fn decode_agent_request(reader: &mut TermReader<'_>) -> Result<AgentStepRequest, TermCodecError> {
    match read_u8(reader, "agent.request")? {
        1 => Ok(AgentStepRequest::user_turn(AgentPayload::new(
            read_sized_bytes(reader, "agent.request.user_turn")?,
        ))),
        2 => {
            let call = decode_call_id(reader, "agent.request.call")?;
            let result = decode_agent_tool_result(reader)?;
            AgentStepRequest::tool_result(call, result)
                .map_err(|_| rejected("agent.request.tool_result"))
        }
        found => unknown_tag("agent.request", found),
    }
}

fn decode_agent_tool_result(
    reader: &mut TermReader<'_>,
) -> Result<AgentToolResult, TermCodecError> {
    let call = decode_call_id(reader, "agent.tool_result.call")?;
    let effect = read_tag(
        reader,
        "agent.tool_result.effect",
        ConcreteExternalEffectTag::from_tag,
    )?;
    match read_u8(reader, "agent.tool_result.result")? {
        1 => {
            let payload = decode_concrete_outcome(reader)?;
            AgentToolResult::succeeded(call, effect, payload)
                .map_err(|_| rejected("agent.tool_result.result"))
        }
        2 => Ok(AgentToolResult::failed(
            call,
            effect,
            decode_effect_failure(reader)?,
        )),
        found => unknown_tag("agent.tool_result.result", found),
    }
}

fn decode_concrete_outcome(
    reader: &mut TermReader<'_>,
) -> Result<ConcreteOutcomePayload, TermCodecError> {
    match read_u8(reader, "agent.tool_result.ok")? {
        1 => Ok(ConcreteOutcomePayload::FileBytes(
            read_sized_bytes(reader, "agent.tool_result.ok.file_bytes")?.into(),
        )),
        2 => Ok(ConcreteOutcomePayload::WrittenLength(read_u64(
            reader,
            "agent.tool_result.ok.written_length",
        )?)),
        3 => Ok(ConcreteOutcomePayload::ProcessResult(
            ProcessResult::direct(
                read_i32(reader, "agent.tool_result.ok.process.exit")?,
                read_sized_bytes(reader, "agent.tool_result.ok.process.stdout")?,
                read_sized_bytes(reader, "agent.tool_result.ok.process.stderr")?,
            ),
        )),
        found => unknown_tag("agent.tool_result.ok", found),
    }
}

pub(crate) fn decode_effect_failure(
    reader: &mut TermReader<'_>,
) -> Result<EffectFailure, TermCodecError> {
    match read_u8(reader, "agent.tool_result.err")? {
        1 => {
            let found = read_u8(reader, "agent.tool_result.err.capability")?;
            let capability = found
                .checked_sub(1)
                .and_then(|index| Capability::ALL.get(usize::from(index)))
                .copied()
                .ok_or(TermCodecError::UnknownTag {
                    position: "agent.tool_result.err.capability",
                    found,
                })?;
            Ok(EffectFailure::ParameterDenied { capability })
        }
        2 => Ok(EffectFailure::TransportTerminal),
        3 => Ok(EffectFailure::ApprovalRequired),
        4 => {
            let divergence = read_tag(
                reader,
                "agent.tool_result.err.divergence",
                Divergence::from_tag,
            )?;
            Ok(EffectFailure::Diverged(divergence))
        }
        5 => Ok(EffectFailure::EndpointGone),
        6 => {
            let fault = read_tag(
                reader,
                "agent.tool_result.err.interpreter_fault",
                InterpreterFault::from_tag,
            )?;
            Ok(EffectFailure::InterpreterFault(fault))
        }
        7 => {
            let kind = read_tag(
                reader,
                "agent.tool_result.err.peer",
                crate::PeerFailureKind::from_tag,
            )?;
            Ok(EffectFailure::Peer(kind))
        }
        8 => Ok(EffectFailure::RetryExhausted {
            attempts: read_u32(reader, "agent.tool_result.err.retry_attempts")?,
        }),
        9 => Ok(EffectFailure::TransportUnreached),
        10 => Ok(EffectFailure::RemoteDeferred),
        found => unknown_tag("agent.tool_result.err", found),
    }
}

fn decode_request_approval(reader: &mut TermReader<'_>) -> Result<EffectTerm, TermCodecError> {
    let target_effect = read_effect_id(reader, "request_approval.target_effect")?;
    Ok(EffectTerm::RequestApproval(ApprovalSpec::new(
        target_effect,
    )))
}

fn decode_ticket(reader: &mut TermReader<'_>) -> Result<Option<ApprovalTicket>, TermCodecError> {
    match read_u8(reader, "ticket")? {
        0 => Ok(None),
        1 => {
            let ledger_item = read_effect_id(reader, "ticket.ledger_item")?;
            let target_effect = read_effect_id(reader, "ticket.target_effect")?;
            Ok(Some(ApprovalTicket::restore(ledger_item, target_effect)))
        }
        found => unknown_tag("ticket", found),
    }
}

fn decode_path(
    reader: &mut TermReader<'_>,
    position: &'static str,
) -> Result<NormalizedPath, TermCodecError> {
    let encoded = read_sized_bytes(reader, position)?;
    let path = PathBuf::from(OsString::from_vec(encoded.to_vec()));
    NormalizedPath::new(path).map_err(|_| rejected(position))
}

fn decode_agent_name(
    reader: &mut TermReader<'_>,
    position: &'static str,
) -> Result<AgentHarnessName, TermCodecError> {
    AgentHarnessName::try_from_normalized(read_str(reader, position)?)
        .map_err(|_| rejected(position))
}

fn decode_call_id(
    reader: &mut TermReader<'_>,
    position: &'static str,
) -> Result<AgentToolCallId, TermCodecError> {
    AgentToolCallId::try_from_bytes(read_sized_bytes(reader, position)?)
        .map_err(|_| rejected(position))
}

fn put_count(output: &mut impl TermOutput, count: usize) {
    output.count(count);
}

fn put_bytes(output: &mut impl TermOutput, bytes: &[u8]) {
    put_count(output, bytes.len());
    output.extend_from_slice(bytes);
}

fn put_str(output: &mut impl TermOutput, value: &str) {
    put_bytes(output, value.as_bytes());
}

fn put_u64(output: &mut impl TermOutput, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn put_i32(output: &mut impl TermOutput, value: i32) {
    output.extend_from_slice(&value.to_le_bytes());
}

pub(crate) type TermReader<'a> = ByteReader<'a, LittleEndian>;

const fn truncated(position: &'static str) -> TermCodecError {
    TermCodecError::Truncated { position }
}

pub(crate) fn read_u8(
    reader: &mut TermReader<'_>,
    position: &'static str,
) -> Result<u8, TermCodecError> {
    reader.byte().map_err(|_| truncated(position))
}

pub(crate) fn read_u16(
    reader: &mut TermReader<'_>,
    position: &'static str,
) -> Result<u16, TermCodecError> {
    reader.u16().map_err(|_| truncated(position))
}

pub(crate) fn read_u32(
    reader: &mut TermReader<'_>,
    position: &'static str,
) -> Result<u32, TermCodecError> {
    reader.u32().map_err(|_| truncated(position))
}

pub(crate) fn read_u64(
    reader: &mut TermReader<'_>,
    position: &'static str,
) -> Result<u64, TermCodecError> {
    reader.u64().map_err(|_| truncated(position))
}

pub(crate) fn read_i32(
    reader: &mut TermReader<'_>,
    position: &'static str,
) -> Result<i32, TermCodecError> {
    reader.i32().map_err(|_| truncated(position))
}

pub(crate) fn read_sized_bytes<'a>(
    reader: &mut TermReader<'a>,
    position: &'static str,
) -> Result<&'a [u8], TermCodecError> {
    reader.len_prefixed().map_err(|_| truncated(position))
}

pub(crate) fn read_str<'a>(
    reader: &mut TermReader<'a>,
    position: &'static str,
) -> Result<&'a str, TermCodecError> {
    std::str::from_utf8(read_sized_bytes(reader, position)?).map_err(|_| rejected(position))
}

pub(crate) fn read_tag<T>(
    reader: &mut TermReader<'_>,
    position: &'static str,
    from_tag: fn(u8) -> Option<T>,
) -> Result<T, TermCodecError> {
    let found = read_u8(reader, position)?;
    from_tag(found).ok_or(TermCodecError::UnknownTag { position, found })
}

pub(crate) fn read_flag(
    reader: &mut TermReader<'_>,
    position: &'static str,
) -> Result<bool, TermCodecError> {
    reader
        .flag()
        .map_err(|_| truncated(position))?
        .map_err(|found| TermCodecError::UnknownTag { position, found })
}

fn unknown_tag<T>(position: &'static str, found: u8) -> Result<T, TermCodecError> {
    Err(TermCodecError::UnknownTag { position, found })
}

const fn rejected(position: &'static str) -> TermCodecError {
    TermCodecError::RejectedByConstructor { position }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ApprovalDecision, ApprovalDecisionDisposition, ApprovalMachine, ApprovalState};

    const HTTP_VECTOR: &[u8] = &[
        0x01, 0x01, 0x00, 0x01, 0x09, 0x00, 0x00, 0x00, b'h', b't', b't', b'p', b's', b':', b'/',
        b'/', b'a', 0x01, 0x00, 0x00, 0x00, b'a', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    const FILE_READ_VECTOR: &[u8] = &[0x01, 0x02, 0x00, 0x02, 0x00, 0x00, 0x00, b'/', b'a', 0x00];
    const FILE_WRITE_VECTOR: &[u8] = &[
        0x01, 0x03, 0x00, 0x02, 0x00, 0x00, 0x00, b'/', b'a', 0x01, 0x00, 0x00, 0x00, 0xff, 0x02,
    ];
    const SPAWN_VECTOR: &[u8] = &[
        0x01, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, b'p', 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
        0x00, b'a', 0x01, 0x00, 0x00, 0x00, 0x00,
    ];
    const NOTIFY_VECTOR: &[u8] = &[
        0x01, 0x05, 0x00, 0x01, 0x00, 0x00, 0x00, b'c', 0x01, 0x00, 0x00, 0x00, b't', 0x01, 0x00,
        0x00, 0x00, b'b',
    ];
    const AGENT_VECTOR: &[u8] = &[
        0x01, 0x06, 0x00, 0x01, 0x00, 0x00, 0x00, b'h', 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0xaa,
    ];
    const REQUEST_APPROVAL_VECTOR: &[u8] = &[
        0x01, 0x07, 0x98, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x04, 0x08, 0x00, 0x00, 0x00,
        0x02, 0x00, 0x00, 0x00, 0x05, 0x6c, 0x6f, 0x63, 0x61, 0x6c, 0x05, 0x00, 0x00, 0x00, 0x01,
        0x73, 0x00, 0x00, 0x00, 0x05, 0x73, 0x63, 0x6f, 0x70, 0x65, 0x07, 0x00, 0x00, 0x00, 0x00,
        0x07, 0x00, 0x00, 0x00, 0x01, 0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x07,
        0x00, 0x00, 0x00, 0x03, 0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x06,
        0x00, 0x00, 0x00, 0x46, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x22, 0x08, 0x00, 0x00, 0x00, 0x02, 0x00,
        0x00, 0x00, 0x05, 0x6c, 0x6f, 0x63, 0x61, 0x6c, 0x05, 0x00, 0x00, 0x00, 0x01, 0x73, 0x00,
        0x00, 0x00, 0x05, 0x73, 0x63, 0x6f, 0x70, 0x65, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x09,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x09,
    ];

    #[test]
    fn round_trip_property_covers_all_term_and_agent_arms() {
        let url = HttpUrl::try_new("https://example.test:8443/path").expect("valid URL");
        let headers = [
            HttpHeader::try_new("accept", "application/octet-stream").expect("plain header"),
            HttpHeader::try_new_secret("authorization", "service-token").expect("secret header"),
        ];
        assert_round_trip(EffectTerm::Http {
            ticket: Some(ticket(7, 11)),
            spec: HttpRequestSpec::try_new(HttpMethod::Post, url, headers, [0x00, 0xff])
                .expect("POST body"),
        });

        let non_utf8_path = PathBuf::from(OsString::from_vec(b"/tmp/\xff".to_vec()));
        assert_round_trip(EffectTerm::FileRead {
            ticket: None,
            spec: FileReadSpec::new(
                NormalizedPath::new(non_utf8_path).expect("absolute normalized path"),
                Some(ByteRange::new(3, 5)),
            ),
        });
        assert_round_trip(EffectTerm::FileWrite {
            ticket: Some(ticket(13, 17)),
            spec: FileWriteSpec::new(path("/write"), [0x10, 0x20], FileWriteMode::Create),
        });
        assert_round_trip(EffectTerm::Spawn {
            ticket: None,
            spec: ProcessSpec::new(
                ProgramName::from_normalized("tool"),
                ["--flag", "value"],
                [0x80, 0x00],
            ),
        });
        assert_round_trip(EffectTerm::Notify {
            ticket: Some(ticket(19, 23)),
            spec: NotificationSpec::new(
                NotificationChannel::from_normalized("desktop"),
                "title",
                "body",
            ),
        });

        let harness = agent_name("harness");
        let invoke = AgentInvokeSpec::try_new(
            harness.clone(),
            Some(AgentSessionId::new(harness, [0x91, 0x92])),
            AgentStepRequest::user_turn(AgentPayload::new([0x41, 0x42])),
        )
        .expect("matching session harness");
        assert_round_trip(EffectTerm::AgentInvoke {
            ticket: Some(ticket(29, 31)),
            invoke,
        });

        for (effect, payload) in [
            (
                ConcreteExternalEffectTag::FileRead,
                ConcreteOutcomePayload::FileBytes(Box::new([0x01, 0x02])),
            ),
            (
                ConcreteExternalEffectTag::FileWrite,
                ConcreteOutcomePayload::WrittenLength(37),
            ),
            (
                ConcreteExternalEffectTag::Spawn,
                ConcreteOutcomePayload::ProcessResult(ProcessResult::direct(
                    -9,
                    [0x03],
                    [0x04, 0x05],
                )),
            ),
        ] {
            let call = call_id(b"ok");
            let result = AgentToolResult::succeeded(call.clone(), effect, payload)
                .expect("effect and success payload match");
            assert_round_trip(agent_term(
                AgentStepRequest::tool_result(call, result).expect("matching call"),
            ));
        }

        for capability in Capability::ALL {
            assert_round_trip(agent_failure(EffectFailure::ParameterDenied { capability }));
        }
        for failure in [
            EffectFailure::TransportTerminal,
            EffectFailure::ApprovalRequired,
            EffectFailure::Diverged(Divergence::MissingRecord),
            EffectFailure::Diverged(Divergence::EffectMismatch),
            EffectFailure::EndpointGone,
            EffectFailure::RetryExhausted { attempts: 10 },
            EffectFailure::TransportUnreached,
            EffectFailure::RemoteDeferred,
        ] {
            assert_round_trip(agent_failure(failure));
        }
        for fault in [
            InterpreterFault::NotFound,
            InterpreterFault::PermissionDenied,
            InterpreterFault::AlreadyExists,
            InterpreterFault::InvalidInput,
            InterpreterFault::BrokenPipe,
            InterpreterFault::ResourceExhausted,
            InterpreterFault::Interrupted,
            InterpreterFault::Other,
        ] {
            assert_round_trip(agent_failure(EffectFailure::InterpreterFault(fault)));
        }

        assert_round_trip(EffectTerm::RequestApproval(ApprovalSpec::new(
            crate::test_effect(41),
        )));
    }

    #[test]
    fn checked_term_size_rejects_counts_and_total_overflow_without_allocating() {
        let mut count = TermSize::default();
        count.count(u32::MAX as usize + 1);
        assert!(count.overflow);
        let mut total = TermSize::default();
        total.add(u32::MAX as usize);
        assert!(!total.overflow);
        total.push(0);
        assert!(total.overflow);
        let mut native = TermSize::default();
        native.add(usize::MAX);
        assert!(native.overflow);
    }

    #[test]
    fn seven_known_answer_vectors_are_sealed_constants() {
        let terms_and_vectors = [
            (known_http(), HTTP_VECTOR),
            (known_file_read(), FILE_READ_VECTOR),
            (known_file_write(), FILE_WRITE_VECTOR),
            (known_spawn(), SPAWN_VECTOR),
            (known_notify(), NOTIFY_VECTOR),
            (known_agent(), AGENT_VECTOR),
            (known_request_approval(), REQUEST_APPROVAL_VECTOR),
        ];

        for (term, vector) in terms_and_vectors {
            assert_eq!(encode_term(&term), vector);
            assert_eq!(try_encode_term(&term), Ok(vector.to_vec()));
            assert_eq!(decode_term(vector), Ok(term));
        }
    }

    #[test]
    fn empty_input_has_its_own_error() {
        assert_eq!(decode_term(&[]), Err(TermCodecError::Empty));
    }

    #[test]
    fn unknown_versions_zero_and_two_are_rejected_without_translation() {
        for found in [0, 2] {
            assert_eq!(
                decode_term(&[found]),
                Err(TermCodecError::UnknownVersion { found })
            );
        }
    }

    #[test]
    fn peer_name_conflict_round_trips_at_appended_tag() {
        let failure = EffectFailure::Peer(crate::PeerFailureKind::NameConflict);
        let mut encoded = Vec::new();
        encode_effect_failure(&mut encoded, &failure);
        assert_eq!(encoded, [7, 14]);
        let mut reader = TermReader::new(&encoded);
        assert_eq!(decode_effect_failure(&mut reader).unwrap(), failure);
        assert!(reader.finished());
        assert_eq!(failure.kind_tag(), "peer_name_conflict");
    }

    #[test]
    fn option_tags_reject_values_above_one() {
        assert_unknown_tag(vec![TERM_CODEC_VERSION, 1, 2], "ticket", 2);

        let mut range = vec![TERM_CODEC_VERSION, 2, 0];
        put_bytes(&mut range, b"/a");
        range.push(2);
        assert_unknown_tag(range, "file_read.range", 2);

        let mut session = vec![TERM_CODEC_VERSION, 6, 0];
        put_str(&mut session, "h");
        session.push(2);
        assert_unknown_tag(session, "agent.session", 2);
    }

    #[test]
    fn every_proper_prefix_of_a_known_answer_is_truncated() {
        for vector in known_vectors() {
            for cut in 1..vector.len() {
                assert!(
                    matches!(
                        decode_term(&vector[..cut]),
                        Err(TermCodecError::Truncated { .. })
                    ),
                    "cut {cut} of {vector:?} was not Truncated"
                );
            }
        }
    }

    #[test]
    fn values_rejected_by_existing_constructors_have_one_closed_error() {
        let mut relative_path = vec![TERM_CODEC_VERSION, 2, 0];
        put_bytes(&mut relative_path, b"relative");
        relative_path.push(0);
        assert_eq!(decode_term(&relative_path), Err(rejected("file_read.path")));

        let mut host_mismatch = vec![TERM_CODEC_VERSION, 1, 0, 1];
        put_str(&mut host_mismatch, "https://a");
        put_str(&mut host_mismatch, "b");
        put_count(&mut host_mismatch, 0);
        put_bytes(&mut host_mismatch, b"");
        assert_eq!(decode_term(&host_mismatch), Err(rejected("http.url.host")));

        let mut empty_harness = vec![TERM_CODEC_VERSION, 6, 0];
        put_str(&mut empty_harness, "");
        assert_eq!(decode_term(&empty_harness), Err(rejected("agent.harness")));

        let mut mismatched_payload = agent_tool_result_prefix();
        mismatched_payload.extend_from_slice(&[1, 1, 2]);
        put_u64(&mut mismatched_payload, 1);
        assert_eq!(
            decode_term(&mismatched_payload),
            Err(rejected("agent.tool_result.result"))
        );
    }

    fn known_vectors() -> [&'static [u8]; 7] {
        [
            HTTP_VECTOR,
            FILE_READ_VECTOR,
            FILE_WRITE_VECTOR,
            SPAWN_VECTOR,
            NOTIFY_VECTOR,
            AGENT_VECTOR,
            REQUEST_APPROVAL_VECTOR,
        ]
    }

    fn known_http() -> EffectTerm {
        EffectTerm::Http {
            ticket: None,
            spec: HttpRequestSpec::try_new(
                HttpMethod::Get,
                HttpUrl::try_new("https://a").expect("valid URL"),
                [],
                [],
            )
            .expect("empty GET body"),
        }
    }

    fn known_file_read() -> EffectTerm {
        EffectTerm::FileRead {
            ticket: None,
            spec: FileReadSpec::new(path("/a"), None),
        }
    }

    fn known_file_write() -> EffectTerm {
        EffectTerm::FileWrite {
            ticket: None,
            spec: FileWriteSpec::new(path("/a"), [0xff], FileWriteMode::Replace),
        }
    }

    fn known_spawn() -> EffectTerm {
        EffectTerm::Spawn {
            ticket: None,
            spec: ProcessSpec::new(ProgramName::from_normalized("p"), ["a"], [0x00]),
        }
    }

    fn known_notify() -> EffectTerm {
        EffectTerm::Notify {
            ticket: None,
            spec: NotificationSpec::new(NotificationChannel::from_normalized("c"), "t", "b"),
        }
    }

    fn known_agent() -> EffectTerm {
        agent_term(AgentStepRequest::user_turn(AgentPayload::new([0xaa])))
    }

    fn known_request_approval() -> EffectTerm {
        EffectTerm::RequestApproval(ApprovalSpec::new(crate::test_effect(9)))
    }

    fn assert_round_trip(term: EffectTerm) {
        let encoded = encode_term(&term);
        let approval = match &term {
            EffectTerm::Http { ticket, .. }
            | EffectTerm::FileRead { ticket, .. }
            | EffectTerm::FileWrite { ticket, .. }
            | EffectTerm::Spawn { ticket, .. }
            | EffectTerm::Notify { ticket, .. }
            | EffectTerm::AgentInvoke { ticket, .. } => ticket.clone().map(TermApproval::Ticket),
            EffectTerm::RequestApproval(_) => Some(TermApproval::Request),
            EffectTerm::Peer(_) => None,
        };
        assert_eq!(decode_term_approval(&encoded), Ok(approval));
        assert_eq!(decode_term(&encoded), Ok(term));
    }

    fn path(value: &str) -> NormalizedPath {
        NormalizedPath::new(value).expect("absolute normalized path")
    }

    fn ticket(ledger_item: u64, target_effect: u64) -> ApprovalTicket {
        let mut machine = ApprovalMachine::requested(
            crate::test_effect(ledger_item),
            crate::test_effect(target_effect),
        );
        let ApprovalDecisionDisposition::Applied(ApprovalState::Approved(ticket)) =
            machine.decide(ApprovalDecision::Approve)
        else {
            panic!("requested approval must accept its first decision")
        };
        ticket
    }

    fn agent_name(value: &str) -> AgentHarnessName {
        AgentHarnessName::try_from_normalized(value).expect("nonempty harness")
    }

    fn call_id(value: &[u8]) -> AgentToolCallId {
        AgentToolCallId::try_from_bytes(value).expect("nonempty call")
    }

    fn agent_term(request: AgentStepRequest) -> EffectTerm {
        EffectTerm::AgentInvoke {
            ticket: None,
            invoke: AgentInvokeSpec::try_new(agent_name("h"), None, request)
                .expect("valid agent request"),
        }
    }

    fn agent_failure(failure: EffectFailure) -> EffectTerm {
        let call = call_id(b"err");
        let result =
            AgentToolResult::failed(call.clone(), ConcreteExternalEffectTag::FileRead, failure);
        agent_term(AgentStepRequest::tool_result(call, result).expect("matching call"))
    }

    fn http_header_value_prefix() -> Vec<u8> {
        let mut input = vec![TERM_CODEC_VERSION, 1, 0, 1];
        put_str(&mut input, "https://a");
        put_str(&mut input, "a");
        put_count(&mut input, 1);
        put_str(&mut input, "x");
        input
    }

    fn agent_request_prefix() -> Vec<u8> {
        let mut input = vec![TERM_CODEC_VERSION, 6, 0];
        put_str(&mut input, "h");
        input.push(0);
        input
    }

    fn agent_tool_result_prefix() -> Vec<u8> {
        let mut input = agent_request_prefix();
        input.push(2);
        put_bytes(&mut input, b"c");
        put_bytes(&mut input, b"c");
        input
    }

    fn assert_unknown_tag(input: Vec<u8>, position: &'static str, found: u8) {
        assert_eq!(
            decode_term(&input),
            Err(TermCodecError::UnknownTag { position, found })
        );
    }
}

fn put_effect_id(output: &mut impl TermOutput, key: &crate::EffectId) {
    match crate::EffectId::encode(key) {
        Ok(bytes) => put_bytes(output, &bytes),
        Err(_) => output.reject_identity(),
    }
}
fn read_effect_id(
    reader: &mut TermReader<'_>,
    position: &'static str,
) -> Result<crate::EffectId, TermCodecError> {
    crate::EffectId::decode(read_sized_bytes(reader, position)?).map_err(|_| rejected(position))
}
