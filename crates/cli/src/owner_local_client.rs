
use std::path::Path;

use circular_core::{Boundary, Ceilings};
use circular_protocol::session_payload::SessionRole;
use circular_protocol::{INITIAL_PROTOCOL_VERSION, SessionMechanicsVerb, StableVerb};
use circular_transport::{
    LocalByteStream, OWNER_LOCAL_SOCKET_NAME, OwnerLocalChannelId, OwnerLocalHelloError,
    OwnerLocalSocketError, OwnerLocalSocketSpec, OwnerLocalStream, SessionIoError,
    decode_session_envelope, exchange_owner_local_hello, read_envelope, write_envelope,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionProbe {
    pub protocol_version: u16,
    pub verb: StableVerb,
    pub correlation: u32,
}

impl SessionProbe {
    #[must_use]
    pub const fn minimal(correlation: u32) -> Self {
        Self {
            protocol_version: INITIAL_PROTOCOL_VERSION,
            verb: StableVerb::SessionMechanics(SessionMechanicsVerb::Goodbye),
            correlation,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObservedEnvelope {
    pub protocol_version: u16,
    pub verb: StableVerb,
    pub correlation: u32,
    pub payload_length: u32,
}

pub fn endpoint_of(state_directory: &Path) -> Result<OwnerLocalSocketSpec, OwnerLocalSocketError> {
    OwnerLocalSocketSpec::try_new(state_directory, OWNER_LOCAL_SOCKET_NAME)
}

pub fn establish(
    spec: &OwnerLocalSocketSpec,
    protocol_version: u16,
    minor: u8,
    roles: &[SessionRole],
    correlation: u32,
) -> Result<ObservedEnvelope, ClientError> {
    let mut stream = OwnerLocalStream::connect(spec).map_err(ClientError::Socket)?;
    let bytes = exchange_owner_local_hello(
        &mut stream,
        OwnerLocalChannelId::new(1),
        protocol_version,
        minor,
        roles,
        correlation,
        Ceilings::for_boundary(Boundary::Wire),
    )
    .map_err(ClientError::Hello)?;
    observe(&bytes)
}

pub fn send_one_way(spec: &OwnerLocalSocketSpec, probe: SessionProbe) -> Result<(), ClientError> {
    let mut stream = OwnerLocalStream::connect(spec).map_err(ClientError::Socket)?;
    send(&mut stream, probe)?;

    let mut scratch = [0_u8; 1];
    match stream.read(&mut scratch) {
        Ok(0) => Ok(()),
        Ok(_) => Err(ClientError::UnexpectedResponse),
        Err(error) => Err(ClientError::Session(SessionIoError::Io(error))),
    }
}

pub fn round_trip(
    spec: &OwnerLocalSocketSpec,
    probe: SessionProbe,
) -> Result<ObservedEnvelope, ClientError> {
    let mut stream = OwnerLocalStream::connect(spec).map_err(ClientError::Socket)?;
    send(&mut stream, probe)?;
    receive(&mut stream)
}

pub fn send(
    stream: &mut impl LocalByteStream<Error = std::io::Error>,
    probe: SessionProbe,
) -> Result<(), ClientError> {
    write_envelope(
        stream,
        OwnerLocalChannelId::new(1),
        probe.protocol_version,
        probe.verb,
        probe.correlation,
        &[],
    )
    .map_err(ClientError::Session)
}

pub fn receive(
    stream: &mut impl LocalByteStream<Error = std::io::Error>,
) -> Result<ObservedEnvelope, ClientError> {
    let bytes = read_envelope(stream).map_err(ClientError::Session)?;
    observe(&bytes)
}

fn observe(bytes: &[u8]) -> Result<ObservedEnvelope, ClientError> {
    let decoded = decode_session_envelope(bytes).map_err(|error| {
        ClientError::Session(SessionIoError::Seam(error))
    })?;
    let header = decoded.header();
    Ok(ObservedEnvelope {
        protocol_version: header.protocol_version(),
        verb: header.verb(),
        correlation: header.correlation(),
        payload_length: header.payload_length(),
    })
}

#[derive(Debug)]
pub enum ClientError {
    Hello(OwnerLocalHelloError),
    Socket(OwnerLocalSocketError),
    Session(SessionIoError),
    UnexpectedResponse,
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hello(error) => write!(formatter, "{error}"),
            Self::Socket(error) => write!(formatter, "endpoint: {error}"),
            Self::Session(error) => write!(formatter, "{error}"),
            Self::UnexpectedResponse => {
                formatter.write_str("peer replied to a verb that has no response")
            }
        }
    }
}
