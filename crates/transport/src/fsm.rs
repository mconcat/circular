//! Pure, injected-policy transport state machines.
//!
//! This module deliberately does not define canonical frame bytes or tags, a
//! concrete `ChannelId`, diagnostic numbers, sockets, retry timing, or
//! authentication. It does name the closed semantic operation sum required by
//! the transport contract so adapters need not invent a fourth operation.

use std::fmt;
use std::num::NonZeroUsize;

/// Positive, caller-supplied body bounds for one frame and one message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransportLimits {
    frame_body: NonZeroUsize,
    message: NonZeroUsize,
}

impl TransportLimits {
    pub const fn new(
        frame_body: NonZeroUsize,
        message: NonZeroUsize,
    ) -> Result<Self, TransportLimitsError> {
        if message.get() < frame_body.get() {
            Err(TransportLimitsError::MessageSmallerThanFrame)
        } else {
            Ok(Self {
                frame_body,
                message,
            })
        }
    }

    #[must_use]
    pub const fn frame_body(self) -> NonZeroUsize {
        self.frame_body
    }

    #[must_use]
    pub const fn message(self) -> NonZeroUsize {
        self.message
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportLimitsError {
    MessageSmallerThanFrame,
}

impl fmt::Display for TransportLimitsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MessageSmallerThanFrame => {
                formatter.write_str("message bound must be at least the frame body bound")
            }
        }
    }
}

impl std::error::Error for TransportLimitsError {}

/// Semantic segmentation only; no numeric wire tags are assigned here.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Segment {
    Whole,
    First,
    Middle,
    Last,
}

/// The complete semantic operation vocabulary. Numeric tags and control-body
/// bytes remain blocked on the canonical frame profile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameOperation {
    Data,
    Open,
    Close,
}

/// One opaque body fragment before canonical framing is chosen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpaqueChunk {
    segment: Segment,
    body: Box<[u8]>,
}

impl OpaqueChunk {
    #[must_use]
    pub fn new(segment: Segment, body: impl Into<Box<[u8]>>) -> Self {
        Self {
            segment,
            body: body.into(),
        }
    }

    #[must_use]
    pub const fn segment(&self) -> Segment {
        self.segment
    }

    #[must_use]
    pub const fn body(&self) -> &[u8] {
        &self.body
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessageLimitExceeded {
    actual: usize,
    limit: usize,
}

impl MessageLimitExceeded {
    #[must_use]
    pub const fn actual(self) -> usize {
        self.actual
    }

    #[must_use]
    pub const fn limit(self) -> usize {
        self.limit
    }
}

impl fmt::Display for MessageLimitExceeded {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "message body length {} exceeds injected limit {}",
            self.actual, self.limit
        )
    }
}

impl std::error::Error for MessageLimitExceeded {}

/// Splits an opaque body without assigning any frame-header representation.
pub fn chunk_opaque_body(
    body: &[u8],
    limits: TransportLimits,
) -> Result<Vec<OpaqueChunk>, MessageLimitExceeded> {
    if body.len() > limits.message.get() {
        return Err(MessageLimitExceeded {
            actual: body.len(),
            limit: limits.message.get(),
        });
    }
    let frame_limit = limits.frame_body.get();
    if body.len() <= frame_limit {
        return Ok(vec![OpaqueChunk::new(Segment::Whole, body)]);
    }

    let chunk_count = body.len().div_ceil(frame_limit);
    Ok(body
        .chunks(frame_limit)
        .enumerate()
        .map(|(index, bytes)| {
            let segment = if index == 0 {
                Segment::First
            } else if index + 1 == chunk_count {
                Segment::Last
            } else {
                Segment::Middle
            };
            OpaqueChunk::new(segment, bytes)
        })
        .collect())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReassemblyState {
    Idle,
    Collecting,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReassemblyOutcome {
    Incomplete,
    Complete(Box<[u8]>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReassemblyError {
    FrameLimitExceeded {
        actual: usize,
        limit: usize,
    },
    MessageLimitExceeded {
        actual: usize,
        limit: usize,
    },
    UnexpectedSegment {
        state: ReassemblyState,
        segment: Segment,
    },
}

impl fmt::Display for ReassemblyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FrameLimitExceeded { actual, limit } => {
                write!(
                    formatter,
                    "frame body length {actual} exceeds injected limit {limit}"
                )
            }
            Self::MessageLimitExceeded { actual, limit } => {
                write!(
                    formatter,
                    "message body length {actual} exceeds injected limit {limit}"
                )
            }
            Self::UnexpectedSegment { state, segment } => {
                write!(formatter, "segment {segment:?} is invalid while {state:?}")
            }
        }
    }
}

impl std::error::Error for ReassemblyError {}

/// Per-channel bounded reassembly state.
#[derive(Debug)]
pub struct Reassembler {
    limits: TransportLimits,
    partial: Option<Vec<u8>>,
}

impl Reassembler {
    #[must_use]
    pub const fn new(limits: TransportLimits) -> Self {
        Self {
            limits,
            partial: None,
        }
    }

    #[must_use]
    pub const fn state(&self) -> ReassemblyState {
        if self.partial.is_some() {
            ReassemblyState::Collecting
        } else {
            ReassemblyState::Idle
        }
    }

    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.partial.as_ref().map_or(0, Vec::len)
    }

    pub fn push(&mut self, chunk: OpaqueChunk) -> Result<ReassemblyOutcome, ReassemblyError> {
        if chunk.body.len() > self.limits.frame_body.get() {
            return self.reject(ReassemblyError::FrameLimitExceeded {
                actual: chunk.body.len(),
                limit: self.limits.frame_body.get(),
            });
        }

        match (self.state(), chunk.segment) {
            (ReassemblyState::Idle, Segment::Whole) => {
                self.check_message_len(chunk.body.len())?;
                Ok(ReassemblyOutcome::Complete(chunk.body))
            }
            (ReassemblyState::Idle, Segment::First) => {
                self.check_message_len(chunk.body.len())?;
                self.partial = Some(chunk.body.into_vec());
                Ok(ReassemblyOutcome::Incomplete)
            }
            (ReassemblyState::Collecting, Segment::Middle) => self.append(chunk.body, false),
            (ReassemblyState::Collecting, Segment::Last) => self.append(chunk.body, true),
            (state, segment) => self.reject(ReassemblyError::UnexpectedSegment { state, segment }),
        }
    }

    fn append(
        &mut self,
        body: Box<[u8]>,
        complete: bool,
    ) -> Result<ReassemblyOutcome, ReassemblyError> {
        let current = self.partial.as_ref().map_or(0, Vec::len);
        let actual = current.saturating_add(body.len());
        self.check_message_len(actual)?;
        let partial = self
            .partial
            .as_mut()
            .expect("collecting state has a buffer");
        partial.extend_from_slice(&body);
        if complete {
            Ok(ReassemblyOutcome::Complete(
                self.partial
                    .take()
                    .expect("completed buffer")
                    .into_boxed_slice(),
            ))
        } else {
            Ok(ReassemblyOutcome::Incomplete)
        }
    }

    fn check_message_len(&mut self, actual: usize) -> Result<(), ReassemblyError> {
        if actual > self.limits.message.get() {
            self.partial = None;
            Err(ReassemblyError::MessageLimitExceeded {
                actual,
                limit: self.limits.message.get(),
            })
        } else {
            Ok(())
        }
    }

    fn reject<T>(&mut self, error: ReassemblyError) -> Result<T, ReassemblyError> {
        self.partial = None;
        Err(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(frame: usize, message: usize) -> TransportLimits {
        TransportLimits::new(
            NonZeroUsize::new(frame).expect("positive frame bound"),
            NonZeroUsize::new(message).expect("positive message bound"),
        )
        .expect("message bound covers a frame")
    }

    #[test]
    fn chunk_round_trip_holds_for_every_small_injected_frame_bound() {
        for frame_limit in 1..=8 {
            for body_len in 0..=32 {
                let body = (0..body_len).map(|value| value as u8).collect::<Vec<_>>();
                let configured = limits(frame_limit, 32);
                let chunks = chunk_opaque_body(&body, configured).unwrap();
                assert!(chunks.iter().all(|chunk| chunk.body().len() <= frame_limit));
                let mut reassembler = Reassembler::new(configured);
                let mut result = None;
                for chunk in chunks {
                    match reassembler.push(chunk).unwrap() {
                        ReassemblyOutcome::Incomplete => {}
                        ReassemblyOutcome::Complete(message) => result = Some(message),
                    }
                    assert!(reassembler.buffered_len() <= configured.message().get());
                }
                assert_eq!(result.as_deref(), Some(body.as_slice()));
                assert_eq!(reassembler.state(), ReassemblyState::Idle);
            }
        }
    }

    #[test]
    fn malformed_segment_or_limit_rejection_discards_only_current_partial() {
        let mut reassembler = Reassembler::new(limits(3, 5));
        assert!(matches!(
            reassembler.push(OpaqueChunk::new(Segment::Middle, [1_u8])),
            Err(ReassemblyError::UnexpectedSegment {
                state: ReassemblyState::Idle,
                segment: Segment::Middle
            })
        ));
        assert_eq!(reassembler.state(), ReassemblyState::Idle);

        assert_eq!(
            reassembler
                .push(OpaqueChunk::new(Segment::First, [1_u8, 2, 3]))
                .unwrap(),
            ReassemblyOutcome::Incomplete
        );
        assert!(matches!(
            reassembler.push(OpaqueChunk::new(Segment::First, [4_u8])),
            Err(ReassemblyError::UnexpectedSegment {
                state: ReassemblyState::Collecting,
                segment: Segment::First
            })
        ));
        assert_eq!(reassembler.state(), ReassemblyState::Idle);

        assert!(matches!(
            reassembler.push(OpaqueChunk::new(Segment::Whole, [0_u8; 4])),
            Err(ReassemblyError::FrameLimitExceeded {
                actual: 4,
                limit: 3
            })
        ));
    }

    #[test]
    fn message_limit_is_checked_before_buffer_growth() {
        let mut reassembler = Reassembler::new(limits(3, 5));
        reassembler
            .push(OpaqueChunk::new(Segment::First, [1_u8, 2, 3]))
            .unwrap();
        assert!(matches!(
            reassembler.push(OpaqueChunk::new(Segment::Last, [4_u8, 5, 6])),
            Err(ReassemblyError::MessageLimitExceeded {
                actual: 6,
                limit: 5
            })
        ));
        assert_eq!(reassembler.buffered_len(), 0);
    }
}
