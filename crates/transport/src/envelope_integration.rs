
use crate::{
    MessageLimitExceeded, OpaqueChunk, Reassembler, ReassemblyError, ReassemblyOutcome,
    TransportLimits, chunk_opaque_body,
};
use circular_protocol::{
    DecodedEnvelopeFrame, FrameRejection, StableVerb, decode_envelope_frame, encode_envelope_frame,
};
use std::fmt;

pub fn chunk_session_envelope(
    protocol_version: u16,
    verb: StableVerb,
    correlation: u32,
    payload: &[u8],
    limits: TransportLimits,
) -> Result<Vec<OpaqueChunk>, EnvelopeIntegrationError> {
    let envelope = encode_envelope_frame(protocol_version, verb, correlation, payload)
        .map_err(EnvelopeIntegrationError::Envelope)?;
    chunk_opaque_body(&envelope, limits).map_err(EnvelopeIntegrationError::MessageLimit)
}

#[derive(Debug)]
pub struct SessionEnvelopeReassembler {
    inner: Reassembler,
}

impl SessionEnvelopeReassembler {
    #[must_use]
    pub const fn new(limits: TransportLimits) -> Self {
        Self {
            inner: Reassembler::new(limits),
        }
    }

    pub fn push(
        &mut self,
        chunk: OpaqueChunk,
    ) -> Result<ReassembledEnvelope, EnvelopeIntegrationError> {
        match self
            .inner
            .push(chunk)
            .map_err(EnvelopeIntegrationError::Reassembly)?
        {
            ReassemblyOutcome::Incomplete => Ok(ReassembledEnvelope::Incomplete),
            ReassemblyOutcome::Complete(body) => {
                decode_envelope_frame(&body).map_err(EnvelopeIntegrationError::Envelope)?;
                Ok(ReassembledEnvelope::Complete(body))
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReassembledEnvelope {
    Incomplete,
    Complete(Box<[u8]>),
}

pub fn decode_session_envelope(
    bytes: &[u8],
) -> Result<DecodedEnvelopeFrame<'_>, EnvelopeIntegrationError> {
    decode_envelope_frame(bytes).map_err(EnvelopeIntegrationError::Envelope)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvelopeIntegrationError {
    Envelope(FrameRejection),
    MessageLimit(MessageLimitExceeded),
    Reassembly(ReassemblyError),
}

impl fmt::Display for EnvelopeIntegrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Envelope(rejection) => write!(formatter, "envelope rejected: {rejection}"),
            Self::MessageLimit(error) => {
                write!(formatter, "frame profile limit exceeded: {error:?}")
            }
            Self::Reassembly(error) => write!(formatter, "reassembly rejected: {error:?}"),
        }
    }
}

impl std::error::Error for EnvelopeIntegrationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OWNER_LOCAL_MAX_MESSAGE_BYTES, owner_local_transport_limits};
    use circular_protocol::{
        ENVELOPE_HEADER_BYTES, INITIAL_PROTOCOL_VERSION, QueryVerb, encode_header,
    };

    fn verb() -> StableVerb {
        StableVerb::Query(QueryVerb::Query)
    }

    fn round_trip(payload: &[u8]) -> DecodedEnvelopeFrameOwned {
        let limits = owner_local_transport_limits();
        let chunks = chunk_session_envelope(INITIAL_PROTOCOL_VERSION, verb(), 42, payload, limits)
            .expect("within the ceiling");
        let mut reassembler = SessionEnvelopeReassembler::new(limits);
        let mut completed = None;
        for chunk in chunks {
            match reassembler.push(chunk).expect("a valid fragment sequence") {
                ReassembledEnvelope::Incomplete => {}
                ReassembledEnvelope::Complete(bytes) => completed = Some(bytes),
            }
        }
        let bytes = completed.expect("the last fragment yields the envelope");
        let decoded = decode_session_envelope(&bytes).expect("bytes that already passed");
        DecodedEnvelopeFrameOwned {
            correlation: decoded.header().correlation(),
            payload_length: decoded.header().payload_length(),
            payload: decoded.payload().to_vec(),
        }
    }

    struct DecodedEnvelopeFrameOwned {
        correlation: u32,
        payload_length: u32,
        payload: Vec<u8>,
    }

    #[test]
    fn an_envelope_survives_every_fragmentation_shape() {
        let frame_body = owner_local_transport_limits().frame_body().get();
        for size in [
            0,
            1,
            frame_body - ENVELOPE_HEADER_BYTES - 1,
            frame_body - ENVELOPE_HEADER_BYTES,
            frame_body,
            frame_body * 3 + 7,
        ] {
            let payload = vec![0xa5_u8; size];
            let decoded = round_trip(&payload);
            assert_eq!(decoded.payload, payload, "{size}-byte payload");
            assert_eq!(decoded.payload_length as usize, size);
            assert_eq!(decoded.correlation, 42);
        }
    }

    #[test]
    fn the_narrower_of_the_two_limits_is_the_effective_one() {
        let oversize = vec![0_u8; OWNER_LOCAL_MAX_MESSAGE_BYTES];
        let refused = chunk_session_envelope(
            INITIAL_PROTOCOL_VERSION,
            verb(),
            1,
            &oversize,
            owner_local_transport_limits(),
        );
        assert!(
            matches!(refused, Err(EnvelopeIntegrationError::MessageLimit(_))),
            "adding the header exceeds the message ceiling"
        );

        let exact = vec![0_u8; OWNER_LOCAL_MAX_MESSAGE_BYTES - ENVELOPE_HEADER_BYTES];
        assert!(
            chunk_session_envelope(
                INITIAL_PROTOCOL_VERSION,
                verb(),
                1,
                &exact,
                owner_local_transport_limits()
            )
            .is_ok()
        );
    }

    #[test]
    fn bytes_that_reassemble_but_are_not_an_envelope_are_refused() {
        let limits = owner_local_transport_limits();
        let mut reassembler = SessionEnvelopeReassembler::new(limits);
        let not_an_envelope = vec![0_u8; ENVELOPE_HEADER_BYTES];

        let refused = reassembler.push(OpaqueChunk::new(
            crate::Segment::Whole,
            not_an_envelope.as_slice(),
        ));
        assert!(
            matches!(
                refused,
                Err(EnvelopeIntegrationError::Envelope(FrameRejection::Header(
                    _
                )))
            ),
            "zero fill does not become an envelope even after it passes reassembly: {refused:?}"
        );
    }

    #[test]
    fn a_head_that_lies_about_its_length_dies_at_the_seam() {
        let limits = owner_local_transport_limits();
        let mut bytes = encode_header(INITIAL_PROTOCOL_VERSION, verb(), 9, 99).to_vec();
        bytes.extend_from_slice(b"only-a-few");

        let mut reassembler = SessionEnvelopeReassembler::new(limits);
        let refused = reassembler.push(OpaqueChunk::new(crate::Segment::Whole, bytes.as_slice()));
        assert!(matches!(
            refused,
            Err(EnvelopeIntegrationError::Envelope(
                FrameRejection::PayloadShorterThanDeclared { .. }
            ))
        ));
    }

    #[test]
    fn each_layer_keeps_its_own_rejection_identity() {
        let limits = owner_local_transport_limits();

        let envelope_layer = chunk_session_envelope(
            0,
            verb(),
            1,
            b"x",
            limits,
        );
        let chunks = envelope_layer.expect("the encoding itself is not a ceiling problem");
        let mut reassembler = SessionEnvelopeReassembler::new(limits);
        let mut refused = None;
        for chunk in chunks {
            if let Err(error) = reassembler.push(chunk) {
                refused = Some(error);
            }
        }
        assert!(matches!(
            refused,
            Some(EnvelopeIntegrationError::Envelope(_))
        ));
    }
}
