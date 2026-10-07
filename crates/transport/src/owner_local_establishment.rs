
use circular_core::{Ceilings, CodecError};
use circular_protocol::session_payload::{SessionRole, hello};
use circular_protocol::{EnvelopeHeader, SessionMechanicsVerb, StableVerb};

use crate::{
    LocalByteStream, OwnerLocalChannelId, SessionIoError, decode_session_envelope, read_envelope,
    write_envelope,
};

pub fn exchange_owner_local_hello(
    stream: &mut impl LocalByteStream<Error = std::io::Error>,
    channel: OwnerLocalChannelId,
    protocol_version: u16,
    minor: u8,
    roles: &[SessionRole],
    correlation: u32,
    ceilings: Ceilings,
) -> Result<Box<[u8]>, OwnerLocalHelloError> {
    let body =
        hello(protocol_version, minor, roles, ceilings).map_err(OwnerLocalHelloError::Body)?;
    write_envelope(
        stream,
        channel,
        protocol_version,
        StableVerb::SessionMechanics(SessionMechanicsVerb::Hello),
        correlation,
        &body,
    )
    .map_err(OwnerLocalHelloError::Session)?;
    let bytes = read_envelope(stream).map_err(OwnerLocalHelloError::Session)?;
    let decoded = decode_session_envelope(&bytes)
        .map_err(|error| OwnerLocalHelloError::Session(SessionIoError::Seam(error)))?;
    let header = decoded.header();
    if header.protocol_version() != protocol_version
        || header.verb() != StableVerb::SessionMechanics(SessionMechanicsVerb::HelloAck)
        || header.correlation() != correlation
    {
        return Err(OwnerLocalHelloError::UnexpectedAnswer {
            expected_version: protocol_version,
            expected_correlation: correlation,
            observed: *header,
        });
    }
    Ok(bytes)
}

#[derive(Debug)]
pub enum OwnerLocalHelloError {
    Body(CodecError),
    Session(SessionIoError),
    UnexpectedAnswer {
        expected_version: u16,
        expected_correlation: u32,
        observed: EnvelopeHeader,
    },
}

impl std::fmt::Display for OwnerLocalHelloError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Body(error) => write!(formatter, "Hello body: {error:?}"),
            Self::Session(error) => write!(formatter, "{error}"),
            Self::UnexpectedAnswer {
                expected_version,
                expected_correlation,
                observed,
            } => write!(
                formatter,
                "expected HelloAck version {expected_version} correlation {expected_correlation}, received {observed:?}"
            ),
        }
    }
}

impl std::error::Error for OwnerLocalHelloError {}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::Boundary;
    use circular_protocol::{INITIAL_PROTOCOL_VERSION, QueryVerb};
    use std::io::{Cursor, Read};

    #[derive(Default)]
    struct Duplex {
        incoming: Cursor<Vec<u8>>,
        outgoing: Vec<u8>,
    }

    impl LocalByteStream for Duplex {
        type Error = std::io::Error;

        fn read(&mut self, destination: &mut [u8]) -> Result<usize, Self::Error> {
            let length = destination.len().min(3);
            self.incoming.read(&mut destination[..length])
        }

        fn write(&mut self, source: &[u8]) -> Result<usize, Self::Error> {
            let length = source.len().min(3);
            self.outgoing.extend_from_slice(&source[..length]);
            Ok(length)
        }
    }

    fn response(version: u16, verb: StableVerb, correlation: u32) -> Duplex {
        let mut peer = Duplex::default();
        for body in [b"ack".as_slice(), b"next".as_slice()] {
            write_envelope(
                &mut peer,
                OwnerLocalChannelId::new(1),
                version,
                verb,
                correlation,
                body,
            )
            .unwrap();
        }
        Duplex {
            incoming: Cursor::new(peer.outgoing),
            outgoing: Vec::new(),
        }
    }

    fn exchange(stream: &mut Duplex) -> Result<Box<[u8]>, OwnerLocalHelloError> {
        exchange_owner_local_hello(
            stream,
            OwnerLocalChannelId::new(1),
            INITIAL_PROTOCOL_VERSION,
            0,
            &[SessionRole::Reader],
            7,
            Ceilings::for_boundary(Boundary::Wire),
        )
    }

    #[test]
    fn exchanges_one_pair_and_preserves_body_stream_and_next_envelope() {
        let mut stream = response(
            INITIAL_PROTOCOL_VERSION,
            StableVerb::SessionMechanics(SessionMechanicsVerb::HelloAck),
            7,
        );
        let ack = exchange(&mut stream).unwrap();
        assert_eq!(decode_session_envelope(&ack).unwrap().payload(), b"ack");
        let next = read_envelope(&mut stream).unwrap();
        assert_eq!(decode_session_envelope(&next).unwrap().payload(), b"next");
        let mut peer = Duplex {
            incoming: Cursor::new(stream.outgoing),
            outgoing: Vec::new(),
        };
        let request = read_envelope(&mut peer).unwrap();
        let request = decode_session_envelope(&request).unwrap();
        assert_eq!(
            request.header().verb(),
            StableVerb::SessionMechanics(SessionMechanicsVerb::Hello)
        );
        assert_eq!(request.header().correlation(), 7);
        assert_eq!(
            request.payload(),
            hello(
                INITIAL_PROTOCOL_VERSION,
                0,
                &[SessionRole::Reader],
                Ceilings::for_boundary(Boundary::Wire)
            )
            .unwrap()
        );
        assert!(matches!(
            read_envelope(&mut peer),
            Err(SessionIoError::PeerClosed)
        ));
    }

    #[test]
    fn refuses_an_answer_with_the_wrong_version_verb_or_correlation() {
        let ack = StableVerb::SessionMechanics(SessionMechanicsVerb::HelloAck);
        assert!(matches!(
            exchange(&mut response(INITIAL_PROTOCOL_VERSION + 1, ack, 7)),
            Err(OwnerLocalHelloError::Session(SessionIoError::Seam(_)))
        ));
        for (version, verb, correlation) in [
            (
                INITIAL_PROTOCOL_VERSION,
                StableVerb::Query(QueryVerb::QueryResult),
                7,
            ),
            (INITIAL_PROTOCOL_VERSION, ack, 8),
        ] {
            assert!(matches!(
                exchange(&mut response(version, verb, correlation)),
                Err(OwnerLocalHelloError::UnexpectedAnswer { .. })
            ));
        }
    }
}
