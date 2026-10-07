
use crate::canonical_frame::{
    OWNER_LOCAL_FRAME_HEADER_BYTES, OwnerLocalChannelId, OwnerLocalFrame, OwnerLocalFrameDecode,
    OwnerLocalFrameError, decode_owner_local_frame_exact, decode_owner_local_frame_prefix,
    owner_local_transport_limits,
};
use crate::envelope_integration::{
    EnvelopeIntegrationError, ReassembledEnvelope, SessionEnvelopeReassembler,
    chunk_session_envelope,
};
use crate::fsm::OpaqueChunk;
use crate::{LocalByteStream, is_socket_timeout};
use circular_protocol::StableVerb;
use std::fmt;
use std::io;

pub fn write_envelope<S>(
    stream: &mut S,
    channel: OwnerLocalChannelId,
    protocol_version: u16,
    verb: StableVerb,
    correlation: u32,
    payload: &[u8],
) -> Result<(), SessionIoError>
where
    S: LocalByteStream<Error = io::Error>,
{
    let chunks = chunk_session_envelope(
        protocol_version,
        verb,
        correlation,
        payload,
        owner_local_transport_limits(),
    )
    .map_err(SessionIoError::Seam)?;

    for chunk in chunks {
        let frame = OwnerLocalFrame::from_chunk(channel, chunk).map_err(SessionIoError::Frame)?;
        write_all(stream, &frame.encode())?;
    }
    Ok(())
}

pub fn read_envelope<S>(stream: &mut S) -> Result<Box<[u8]>, SessionIoError>
where
    S: LocalByteStream<Error = io::Error>,
{
    read_envelope_polling(stream, &mut || false)
}

pub fn read_envelope_polling<S, F>(
    stream: &mut S,
    idle: &mut F,
) -> Result<Box<[u8]>, SessionIoError>
where
    S: LocalByteStream<Error = io::Error>,
    F: FnMut() -> bool,
{
    let mut reassembler = SessionEnvelopeReassembler::new(owner_local_transport_limits());
    let mut at_boundary = true;

    loop {
        let mut whole = vec![0_u8; OWNER_LOCAL_FRAME_HEADER_BYTES];
        read_exact_polling(stream, &mut whole, &mut at_boundary, idle)?;

        loop {
            match decode_owner_local_frame_prefix(&whole) {
                Ok(OwnerLocalFrameDecode::Complete { .. }) => break,
                Ok(OwnerLocalFrameDecode::NeedMore { required }) => {
                    let already = whole.len();
                    whole.resize(required, 0);
                    read_exact_polling(stream, &mut whole[already..], &mut at_boundary, idle)?;
                }
                Err(error) => return Err(SessionIoError::Frame(error)),
            }
        }

        let frame = decode_owner_local_frame_exact(&whole).map_err(SessionIoError::Frame)?;
        let chunk = OpaqueChunk::new(frame.segment(), frame.body());
        match reassembler.push(chunk).map_err(SessionIoError::Seam)? {
            ReassembledEnvelope::Incomplete => {}
            ReassembledEnvelope::Complete(bytes) => return Ok(bytes),
        }
    }
}

fn write_all<S>(stream: &mut S, mut bytes: &[u8]) -> Result<(), SessionIoError>
where
    S: LocalByteStream<Error = io::Error>,
{
    while !bytes.is_empty() {
        let written = stream.write(bytes).map_err(SessionIoError::Io)?;
        if written == 0 {
            return Err(SessionIoError::PeerClosed);
        }
        bytes = &bytes[written..];
    }
    Ok(())
}

fn read_exact_polling<S, F>(
    stream: &mut S,
    mut destination: &mut [u8],
    at_boundary: &mut bool,
    idle: &mut F,
) -> Result<(), SessionIoError>
where
    S: LocalByteStream<Error = io::Error>,
    F: FnMut() -> bool,
{
    while !destination.is_empty() {
        match stream.read(destination) {
            Ok(0) => return Err(SessionIoError::PeerClosed),
            Ok(read) => {
                *at_boundary = false;
                destination = &mut destination[read..];
            }
            Err(error) if *at_boundary && is_socket_timeout(&error) => {
                if !idle() {
                    return Err(SessionIoError::Io(error));
                }
            }
            Err(error) => return Err(SessionIoError::Io(error)),
        }
    }
    Ok(())
}

#[derive(Debug)]
pub enum SessionIoError {
    Seam(EnvelopeIntegrationError),
    Frame(OwnerLocalFrameError),
    Io(io::Error),
    PeerClosed,
}

impl fmt::Display for SessionIoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Seam(error) => write!(formatter, "envelope/frame seam: {error}"),
            Self::Frame(error) => write!(formatter, "frame: {error:?}"),
            Self::Io(error) => write!(formatter, "I/O: {error}"),
            Self::PeerClosed => {
                formatter.write_str("peer closed the stream before completing the envelope")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_protocol::{
        INITIAL_PROTOCOL_VERSION, SessionMechanicsVerb, decode_envelope_frame,
    };

    #[derive(Debug, Default)]
    struct Pipe {
        buffer: Vec<u8>,
        cursor: usize,
        grain: usize,
    }

    impl Pipe {
        fn with_grain(grain: usize) -> Self {
            Self {
                grain,
                ..Self::default()
            }
        }
    }

    impl LocalByteStream for Pipe {
        type Error = io::Error;

        fn read(&mut self, destination: &mut [u8]) -> Result<usize, Self::Error> {
            let available = self.buffer.len() - self.cursor;
            let count = available.min(destination.len()).min(self.grain.max(1));
            destination[..count].copy_from_slice(&self.buffer[self.cursor..self.cursor + count]);
            self.cursor += count;
            Ok(count)
        }

        fn write(&mut self, source: &[u8]) -> Result<usize, Self::Error> {
            let count = source.len().min(self.grain.max(1));
            self.buffer.extend_from_slice(&source[..count]);
            Ok(count)
        }
    }

    #[derive(Debug)]
    struct Expiring {
        pipe: Pipe,
        settle: usize,
        expiries: usize,
    }

    impl LocalByteStream for Expiring {
        type Error = io::Error;

        fn read(&mut self, destination: &mut [u8]) -> Result<usize, Self::Error> {
            if self.settle > 0 {
                self.settle -= 1;
                return self.pipe.read(destination);
            }
            if self.expiries > 0 {
                self.expiries -= 1;
                return Err(io::Error::from(io::ErrorKind::WouldBlock));
            }
            self.pipe.read(destination)
        }

        fn write(&mut self, source: &[u8]) -> Result<usize, Self::Error> {
            self.pipe.write(source)
        }
    }

    fn one_goodbye(grain: usize) -> Pipe {
        let mut pipe = Pipe::with_grain(grain);
        write_envelope(
            &mut pipe,
            OwnerLocalChannelId::new(1),
            INITIAL_PROTOCOL_VERSION,
            goodbye(),
            7,
            &[],
        )
        .expect("write");
        pipe
    }

    #[test]
    fn an_expiry_at_the_boundary_asks_the_caller_and_then_reads_on() {
        let mut stream = Expiring {
            pipe: one_goodbye(usize::MAX),
            settle: 0,
            expiries: 3,
        };
        let mut asked = 0_usize;
        let bytes = read_envelope_polling(&mut stream, &mut || {
            asked += 1;
            true
        })
        .expect("an envelope arrives after idling");
        assert_eq!(asked, 3, "it must ask once per expiry");
        assert_eq!(
            decode_envelope_frame(&bytes)
                .expect("it is an envelope")
                .header()
                .correlation(),
            7
        );
    }

    #[test]
    fn a_caller_that_will_not_wait_gets_the_expiry_as_an_error() {
        let mut stream = Expiring {
            pipe: one_goodbye(usize::MAX),
            settle: 0,
            expiries: 1,
        };
        assert!(
            matches!(read_envelope(&mut stream), Err(SessionIoError::Io(_))),
            "for the side that chose not to wait, the expiry must surface as an error"
        );
    }

    #[test]
    fn an_expiry_after_the_first_byte_is_an_error_even_if_the_caller_would_wait() {
        let mut stream = Expiring {
            pipe: one_goodbye(1),
            settle: 1,
            expiries: 1,
        };
        let mut asked = 0_usize;
        let outcome = read_envelope_polling(&mut stream, &mut || {
            asked += 1;
            true
        });
        assert!(
            matches!(outcome, Err(SessionIoError::Io(_))),
            "an expiry in the middle of a fragment must be an error even for the side that waits"
        );
        assert_eq!(asked, 0, "after leaving the boundary it does not even ask");
    }

    fn goodbye() -> StableVerb {
        StableVerb::SessionMechanics(SessionMechanicsVerb::Goodbye)
    }

    #[test]
    fn an_envelope_written_to_a_stream_reads_back_as_the_same_envelope() {
        let mut pipe = Pipe::with_grain(usize::MAX);
        write_envelope(
            &mut pipe,
            OwnerLocalChannelId::new(1),
            INITIAL_PROTOCOL_VERSION,
            goodbye(),
            0x0102_0304,
            &[],
        )
        .expect("write");

        let bytes = read_envelope(&mut pipe).expect("read");
        let decoded = decode_envelope_frame(&bytes).expect("it is an envelope");
        assert_eq!(decoded.header().verb(), goodbye());
        assert_eq!(decoded.header().correlation(), 0x0102_0304);
        assert_eq!(decoded.header().payload_length(), 0);
    }

    #[test]
    fn one_byte_at_a_time_still_yields_exactly_one_envelope() {
        let mut pipe = Pipe::with_grain(1);
        write_envelope(
            &mut pipe,
            OwnerLocalChannelId::new(7),
            INITIAL_PROTOCOL_VERSION,
            goodbye(),
            9,
            &[],
        )
        .expect("write");

        let bytes = read_envelope(&mut pipe).expect("read");
        assert_eq!(
            decode_envelope_frame(&bytes)
                .expect("it is an envelope")
                .header()
                .correlation(),
            9
        );
    }

    #[test]
    fn reading_one_envelope_does_not_swallow_the_next() {
        let mut pipe = Pipe::with_grain(usize::MAX);
        for correlation in [11, 22] {
            write_envelope(
                &mut pipe,
                OwnerLocalChannelId::new(1),
                INITIAL_PROTOCOL_VERSION,
                goodbye(),
                correlation,
                &[],
            )
            .expect("write");
        }

        for expected in [11, 22] {
            let bytes = read_envelope(&mut pipe).expect("read");
            assert_eq!(
                decode_envelope_frame(&bytes)
                    .expect("it is an envelope")
                    .header()
                    .correlation(),
                expected
            );
        }
    }

    #[test]
    fn a_truncated_stream_is_an_error_and_not_a_partial_envelope() {
        let mut pipe = Pipe::with_grain(usize::MAX);
        write_envelope(
            &mut pipe,
            OwnerLocalChannelId::new(1),
            INITIAL_PROTOCOL_VERSION,
            goodbye(),
            5,
            &[],
        )
        .expect("write");
        pipe.buffer.pop();

        assert!(matches!(
            read_envelope(&mut pipe),
            Err(SessionIoError::PeerClosed)
        ));
    }
}
