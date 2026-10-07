
use crate::{FrameOperation, OpaqueChunk, Segment, TransportLimits};

/// Injected, finite source of opaque connection-local channel identifiers.
pub trait ChannelIdSource {
    type Id: Clone + Ord;

    fn next_id(&mut self) -> Option<Self::Id>;
}

impl<I: Clone + Ord, T: Iterator<Item = I>> ChannelIdSource for T {
    type Id = I;

    fn next_id(&mut self) -> Option<Self::Id> {
        self.next()
    }
}
use std::fmt;
use std::num::NonZeroUsize;

/// The only framing version emitted by the first OwnerLocal profile.
pub const OWNER_LOCAL_FRAMING_VERSION: u8 = circular_core::compatibility::WIRE_FRAMING_VERSION;

/// `version:u8 · channel:u32be · operation:u8 · segment:u8 · body_len:u32be`.
pub const OWNER_LOCAL_FRAME_HEADER_BYTES: usize = 11;

pub const OWNER_LOCAL_MAX_FRAME_BODY_BYTES: usize = circular_core::MAX_SEGMENT_BODY_BYTES;

pub const OWNER_LOCAL_MAX_MESSAGE_BYTES: usize = circular_core::MAX_REASSEMBLED_BODY_BYTES;

const OPERATION_DATA: u8 = 1;
const OPERATION_OPEN: u8 = 2;
const OPERATION_CLOSE: u8 = 3;

const SEGMENT_WHOLE: u8 = 1;
const SEGMENT_FIRST: u8 = 2;
const SEGMENT_MIDDLE: u8 = 3;
const SEGMENT_LAST: u8 = 4;

/// The finite limits published by the first OwnerLocal framing version.
#[must_use]
pub fn owner_local_transport_limits() -> TransportLimits {
    TransportLimits::new(
        NonZeroUsize::new(OWNER_LOCAL_MAX_FRAME_BODY_BYTES)
            .expect("the published frame bound is positive"),
        NonZeroUsize::new(OWNER_LOCAL_MAX_MESSAGE_BYTES)
            .expect("the published message bound is positive"),
    )
    .expect("the published message bound covers one frame")
}

/// A connection-local identifier that is never reused within that connection.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OwnerLocalChannelId(u32);

impl OwnerLocalChannelId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Monotone, finite channel identifier source for one connection.
///
/// The source is intentionally not cloneable: cloning it would let two
/// allocators issue the same identifier for one connection.
#[derive(Debug, Default)]
pub struct OwnerLocalChannelIdSource {
    next: Option<u32>,
}

impl OwnerLocalChannelIdSource {
    #[must_use]
    pub const fn new() -> Self {
        Self { next: Some(0) }
    }

    #[cfg(test)]
    const fn starting_at(next: u32) -> Self {
        Self { next: Some(next) }
    }
}

impl ChannelIdSource for OwnerLocalChannelIdSource {
    type Id = OwnerLocalChannelId;

    fn next_id(&mut self) -> Option<Self::Id> {
        let value = self.next?;
        self.next = value.checked_add(1);
        Some(OwnerLocalChannelId(value))
    }
}

/// One complete canonical transport frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OwnerLocalFrame {
    channel: OwnerLocalChannelId,
    operation: FrameOperation,
    segment: Segment,
    body: Box<[u8]>,
}

impl OwnerLocalFrame {
    pub fn try_new(
        channel: OwnerLocalChannelId,
        operation: FrameOperation,
        segment: Segment,
        body: impl Into<Box<[u8]>>,
    ) -> Result<Self, OwnerLocalFrameError> {
        let body = body.into();
        validate_frame(operation, segment, body.len())?;
        Ok(Self {
            channel,
            operation,
            segment,
            body,
        })
    }

    pub fn from_chunk(
        channel: OwnerLocalChannelId,
        chunk: OpaqueChunk,
    ) -> Result<Self, OwnerLocalFrameError> {
        Self::try_new(channel, FrameOperation::Data, chunk.segment(), chunk.body())
    }

    #[must_use]
    pub const fn channel(&self) -> OwnerLocalChannelId {
        self.channel
    }

    #[must_use]
    pub const fn operation(&self) -> FrameOperation {
        self.operation
    }

    #[must_use]
    pub const fn segment(&self) -> Segment {
        self.segment
    }

    #[must_use]
    pub const fn body(&self) -> &[u8] {
        &self.body
    }

    /// Encodes the exact first-version byte layout.
    #[must_use]
    pub fn encode(&self) -> Box<[u8]> {
        let body_len = u32::try_from(self.body.len())
            .expect("the published frame bound fits in the u32 length field");
        let mut bytes = Vec::with_capacity(OWNER_LOCAL_FRAME_HEADER_BYTES + self.body.len());
        bytes.push(OWNER_LOCAL_FRAMING_VERSION);
        bytes.extend_from_slice(&self.channel.get().to_be_bytes());
        bytes.push(operation_tag(self.operation));
        bytes.push(segment_tag(self.segment));
        bytes.extend_from_slice(&body_len.to_be_bytes());
        bytes.extend_from_slice(&self.body);
        bytes.into_boxed_slice()
    }
}

fn validate_frame(
    operation: FrameOperation,
    segment: Segment,
    body_len: usize,
) -> Result<(), OwnerLocalFrameError> {
    if body_len > OWNER_LOCAL_MAX_FRAME_BODY_BYTES {
        return Err(OwnerLocalFrameError::FrameBodyTooLarge {
            actual: body_len,
            maximum: OWNER_LOCAL_MAX_FRAME_BODY_BYTES,
        });
    }
    if operation != FrameOperation::Data && segment != Segment::Whole {
        return Err(OwnerLocalFrameError::FragmentedControl { operation, segment });
    }
    if operation == FrameOperation::Data && segment != Segment::Whole && body_len == 0 {
        return Err(OwnerLocalFrameError::EmptyFragment { segment });
    }
    Ok(())
}

const fn operation_tag(operation: FrameOperation) -> u8 {
    match operation {
        FrameOperation::Data => OPERATION_DATA,
        FrameOperation::Open => OPERATION_OPEN,
        FrameOperation::Close => OPERATION_CLOSE,
    }
}

const fn segment_tag(segment: Segment) -> u8 {
    match segment {
        Segment::Whole => SEGMENT_WHOLE,
        Segment::First => SEGMENT_FIRST,
        Segment::Middle => SEGMENT_MIDDLE,
        Segment::Last => SEGMENT_LAST,
    }
}

fn decode_operation(tag: u8) -> Result<FrameOperation, OwnerLocalFrameError> {
    match tag {
        OPERATION_DATA => Ok(FrameOperation::Data),
        OPERATION_OPEN => Ok(FrameOperation::Open),
        OPERATION_CLOSE => Ok(FrameOperation::Close),
        _ => Err(OwnerLocalFrameError::UnknownOperation(tag)),
    }
}

fn decode_segment(tag: u8) -> Result<Segment, OwnerLocalFrameError> {
    match tag {
        SEGMENT_WHOLE => Ok(Segment::Whole),
        SEGMENT_FIRST => Ok(Segment::First),
        SEGMENT_MIDDLE => Ok(Segment::Middle),
        SEGMENT_LAST => Ok(Segment::Last),
        _ => Err(OwnerLocalFrameError::UnknownSegment(tag)),
    }
}

/// Prefix decoding never publishes a partial frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OwnerLocalFrameDecode {
    NeedMore {
        required: usize,
    },
    Complete {
        frame: OwnerLocalFrame,
        consumed: usize,
    },
}

/// Decodes one frame from the beginning of a byte stream.
pub fn decode_owner_local_frame_prefix(
    bytes: &[u8],
) -> Result<OwnerLocalFrameDecode, OwnerLocalFrameError> {
    if bytes.len() < OWNER_LOCAL_FRAME_HEADER_BYTES {
        return Ok(OwnerLocalFrameDecode::NeedMore {
            required: OWNER_LOCAL_FRAME_HEADER_BYTES,
        });
    }
    if bytes[0] != OWNER_LOCAL_FRAMING_VERSION {
        return Err(OwnerLocalFrameError::UnsupportedVersion(bytes[0]));
    }
    let channel = OwnerLocalChannelId::new(u32::from_be_bytes(
        bytes[1..5]
            .try_into()
            .expect("the complete header contains four channel bytes"),
    ));
    let operation = decode_operation(bytes[5])?;
    let segment = decode_segment(bytes[6])?;
    let body_len = u32::from_be_bytes(
        bytes[7..11]
            .try_into()
            .expect("the complete header contains four length bytes"),
    ) as usize;
    validate_frame(operation, segment, body_len)?;
    let required = OWNER_LOCAL_FRAME_HEADER_BYTES + body_len;
    if bytes.len() < required {
        return Ok(OwnerLocalFrameDecode::NeedMore { required });
    }
    let frame = OwnerLocalFrame::try_new(
        channel,
        operation,
        segment,
        bytes[OWNER_LOCAL_FRAME_HEADER_BYTES..required].to_vec(),
    )?;
    Ok(OwnerLocalFrameDecode::Complete {
        frame,
        consumed: required,
    })
}

/// Decodes exactly one frame and rejects trailing bytes.
pub fn decode_owner_local_frame_exact(
    bytes: &[u8],
) -> Result<OwnerLocalFrame, OwnerLocalFrameError> {
    match decode_owner_local_frame_prefix(bytes)? {
        OwnerLocalFrameDecode::NeedMore { required } => Err(OwnerLocalFrameError::Truncated {
            actual: bytes.len(),
            required,
        }),
        OwnerLocalFrameDecode::Complete { frame, consumed } if consumed == bytes.len() => Ok(frame),
        OwnerLocalFrameDecode::Complete { consumed, .. } => {
            Err(OwnerLocalFrameError::TrailingBytes {
                trailing: bytes.len() - consumed,
            })
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OwnerLocalFrameError {
    UnsupportedVersion(u8),
    UnknownOperation(u8),
    UnknownSegment(u8),
    FrameBodyTooLarge {
        actual: usize,
        maximum: usize,
    },
    FragmentedControl {
        operation: FrameOperation,
        segment: Segment,
    },
    EmptyFragment {
        segment: Segment,
    },
    Truncated {
        actual: usize,
        required: usize,
    },
    TrailingBytes {
        trailing: usize,
    },
}

impl fmt::Display for OwnerLocalFrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "unsupported OwnerLocal framing version {version}"
                )
            }
            Self::UnknownOperation(tag) => write!(formatter, "unknown frame operation tag {tag}"),
            Self::UnknownSegment(tag) => write!(formatter, "unknown frame segment tag {tag}"),
            Self::FrameBodyTooLarge { actual, maximum } => write!(
                formatter,
                "frame body length {actual} exceeds the OwnerLocal maximum {maximum}"
            ),
            Self::FragmentedControl { operation, segment } => write!(
                formatter,
                "control operation {operation:?} cannot use segment {segment:?}"
            ),
            Self::EmptyFragment { segment } => {
                write!(
                    formatter,
                    "data segment {segment:?} cannot have an empty body"
                )
            }
            Self::Truncated { actual, required } => write!(
                formatter,
                "frame is truncated at {actual} bytes; {required} bytes are required"
            ),
            Self::TrailingBytes { trailing } => {
                write!(formatter, "exact frame has {trailing} trailing bytes")
            }
        }
    }
}

impl std::error::Error for OwnerLocalFrameError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Reassembler, ReassemblyOutcome, chunk_opaque_body};

    #[test]
    fn first_version_golden_layout_is_exact_and_big_endian() {
        let frame = OwnerLocalFrame::try_new(
            OwnerLocalChannelId::new(0x0102_0304),
            FrameOperation::Data,
            Segment::Whole,
            [0xaa, 0xbb, 0xcc],
        )
        .unwrap();
        let expected = [
            0x01, 0x01, 0x02, 0x03, 0x04, 0x01, 0x01, 0x00, 0x00, 0x00, 0x03, 0xaa, 0xbb, 0xcc,
        ];
        assert_eq!(frame.encode().as_ref(), expected);
        assert_eq!(decode_owner_local_frame_exact(&expected).unwrap(), frame);
        let mut version_zero = expected;
        version_zero[0] = 0;
        let mut operation_zero = expected;
        operation_zero[5] = 0;
        let mut segment_zero = expected;
        segment_zero[6] = 0;
        let mut trailing = expected.to_vec();
        trailing.push(0xdd);
        circular_testkit::codec_laws(
            |frame: &OwnerLocalFrame| Ok::<_, std::convert::Infallible>(frame.encode().to_vec()),
            decode_owner_local_frame_exact,
            &[&expected],
            &[
                &version_zero,
                &operation_zero,
                &segment_zero,
                &expected[..expected.len() - 1],
                &trailing,
            ],
        );
    }

    #[test]
    fn operation_and_segment_tags_are_closed_and_zero_is_rejected() {
        for (operation, operation_tag) in [
            (FrameOperation::Data, OPERATION_DATA),
            (FrameOperation::Open, OPERATION_OPEN),
            (FrameOperation::Close, OPERATION_CLOSE),
        ] {
            let frame = OwnerLocalFrame::try_new(
                OwnerLocalChannelId::new(9),
                operation,
                Segment::Whole,
                [],
            )
            .unwrap();
            assert_eq!(frame.encode()[5], operation_tag);
        }
        let valid = OwnerLocalFrame::try_new(
            OwnerLocalChannelId::new(1),
            FrameOperation::Data,
            Segment::Whole,
            [1],
        )
        .unwrap()
        .encode();
        for (index, expected) in [
            (0, OwnerLocalFrameError::UnsupportedVersion(0)),
            (5, OwnerLocalFrameError::UnknownOperation(0)),
            (6, OwnerLocalFrameError::UnknownSegment(0)),
        ] {
            let mut bytes = valid.to_vec();
            bytes[index] = 0;
            assert_eq!(decode_owner_local_frame_exact(&bytes), Err(expected));
        }
    }

    #[test]
    fn every_truncated_prefix_needs_more_and_exact_decode_rejects_it() {
        let encoded = OwnerLocalFrame::try_new(
            OwnerLocalChannelId::new(3),
            FrameOperation::Data,
            Segment::Whole,
            [1, 2, 3, 4],
        )
        .unwrap()
        .encode();
        for length in 0..encoded.len() {
            assert!(matches!(
                decode_owner_local_frame_prefix(&encoded[..length]),
                Ok(OwnerLocalFrameDecode::NeedMore { required }) if required > length
            ));
            assert!(matches!(
                decode_owner_local_frame_exact(&encoded[..length]),
                Err(OwnerLocalFrameError::Truncated { actual, required })
                    if actual == length && required > actual
            ));
        }
    }

    #[test]
    fn prefix_decode_consumes_one_frame_and_exact_decode_rejects_trailing_data() {
        let first = OwnerLocalFrame::try_new(
            OwnerLocalChannelId::new(1),
            FrameOperation::Data,
            Segment::Whole,
            [7, 8],
        )
        .unwrap();
        let second = OwnerLocalFrame::try_new(
            OwnerLocalChannelId::new(2),
            FrameOperation::Close,
            Segment::Whole,
            [],
        )
        .unwrap();
        let mut stream = first.encode().to_vec();
        stream.extend_from_slice(&second.encode());
        assert!(matches!(
            decode_owner_local_frame_prefix(&stream),
            Ok(OwnerLocalFrameDecode::Complete { frame, consumed })
                if frame == first && consumed == first.encode().len()
        ));
        assert_eq!(
            decode_owner_local_frame_exact(&stream),
            Err(OwnerLocalFrameError::TrailingBytes {
                trailing: second.encode().len(),
            })
        );
    }

    #[test]
    fn oversize_is_rejected_from_the_header_before_waiting_for_the_body() {
        let mut header = vec![0; OWNER_LOCAL_FRAME_HEADER_BYTES];
        header[0] = OWNER_LOCAL_FRAMING_VERSION;
        header[5] = OPERATION_DATA;
        header[6] = SEGMENT_WHOLE;
        let oversized = u32::try_from(OWNER_LOCAL_MAX_FRAME_BODY_BYTES + 1).unwrap();
        header[7..11].copy_from_slice(&oversized.to_be_bytes());
        assert_eq!(
            decode_owner_local_frame_prefix(&header),
            Err(OwnerLocalFrameError::FrameBodyTooLarge {
                actual: OWNER_LOCAL_MAX_FRAME_BODY_BYTES + 1,
                maximum: OWNER_LOCAL_MAX_FRAME_BODY_BYTES,
            })
        );
    }

    #[test]
    fn control_frames_are_whole_and_fragmented_data_is_nonempty() {
        assert_eq!(
            OwnerLocalFrame::try_new(
                OwnerLocalChannelId::new(1),
                FrameOperation::Open,
                Segment::First,
                [1],
            ),
            Err(OwnerLocalFrameError::FragmentedControl {
                operation: FrameOperation::Open,
                segment: Segment::First,
            })
        );
        assert_eq!(
            OwnerLocalFrame::try_new(
                OwnerLocalChannelId::new(1),
                FrameOperation::Data,
                Segment::Last,
                [],
            ),
            Err(OwnerLocalFrameError::EmptyFragment {
                segment: Segment::Last,
            })
        );
    }

    #[test]
    fn published_limits_fragment_encode_decode_and_reassemble_losslessly() {
        let body = (0..(OWNER_LOCAL_MAX_FRAME_BODY_BYTES + 31))
            .map(|value| (value % 251) as u8)
            .collect::<Vec<_>>();
        let chunks = chunk_opaque_body(&body, owner_local_transport_limits()).unwrap();
        assert_eq!(chunks.len(), 2);
        let mut reassembler = Reassembler::new(owner_local_transport_limits());
        let mut completed = None;
        for chunk in chunks {
            let frame = OwnerLocalFrame::from_chunk(OwnerLocalChannelId::new(17), chunk).unwrap();
            let decoded = decode_owner_local_frame_exact(&frame.encode()).unwrap();
            match reassembler
                .push(OpaqueChunk::new(decoded.segment(), decoded.body()))
                .unwrap()
            {
                ReassemblyOutcome::Incomplete => {}
                ReassemblyOutcome::Complete(value) => completed = Some(value),
            }
        }
        assert_eq!(completed.as_deref(), Some(body.as_slice()));
    }

    #[test]
    fn channel_ids_are_monotone_and_exhaust_without_reuse() {
        let mut source = OwnerLocalChannelIdSource::starting_at(u32::MAX - 1);
        assert_eq!(
            source.next_id(),
            Some(OwnerLocalChannelId::new(u32::MAX - 1))
        );
        assert_eq!(source.next_id(), Some(OwnerLocalChannelId::new(u32::MAX)));
        assert_eq!(source.next_id(), None);
        assert_eq!(source.next_id(), None);
    }

    #[test]
    fn published_bounds_are_finite_and_fit_the_header_length_field() {
        let limits = owner_local_transport_limits();
        assert_eq!(limits.frame_body().get(), OWNER_LOCAL_MAX_FRAME_BODY_BYTES);
        assert_eq!(limits.message().get(), OWNER_LOCAL_MAX_MESSAGE_BYTES);
        assert!(limits.frame_body().get() <= u32::MAX as usize);
        assert!(limits.frame_body().get() < limits.message().get());
    }
}
