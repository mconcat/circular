
use crate::wire_value::PayloadRejection;
use crate::{
    DeclarationVerb, EventInjectionVerb, LedgerTransitionVerb, LifecycleVerb, Partition, QueryVerb,
    ReplayControlVerb, SessionMechanicsVerb, SubscriptionVerb,
};
use std::fmt;

pub const PROTOCOL_VERSION_BYTES: usize = 2;
pub const PARTITION_TAG_BYTES: usize = 1;
pub const VERB_TAG_BYTES: usize = 1;
pub const CORRELATION_BYTES: usize = 4;
pub const PAYLOAD_LENGTH_BYTES: usize = 4;

pub const ENVELOPE_HEADER_BYTES: usize = PROTOCOL_VERSION_BYTES
    + PARTITION_TAG_BYTES
    + VERB_TAG_BYTES
    + CORRELATION_BYTES
    + PAYLOAD_LENGTH_BYTES;

pub const VERSION_INVARIANT_PREFIX_BYTES: usize =
    PROTOCOL_VERSION_BYTES + PARTITION_TAG_BYTES + VERB_TAG_BYTES + CORRELATION_BYTES;

pub const INITIAL_PROTOCOL_VERSION: u16 = circular_core::compatibility::WIRE_PROTOCOL_VERSION;

pub const MAX_LIVE_CORRELATIONS: u32 = 4096;

/// Session-owned live keys. Completion releases a key; allocation never wraps
/// or evicts another request when the declared simultaneous bound is reached.
#[derive(Default, Debug)]
pub struct LiveCorrelations(std::collections::BTreeSet<u32>);

impl LiveCorrelations {
    pub fn try_open(&mut self, correlation: u32) -> bool {
        self.0.len() < MAX_LIVE_CORRELATIONS as usize && self.0.insert(correlation)
    }

    pub fn contains(&self, correlation: u32) -> bool {
        self.0.contains(&correlation)
    }

    pub fn finish(&mut self, correlation: u32) {
        self.0.remove(&correlation);
    }
}

pub const RESERVED_CAPABILITY_PARTITION_TAG: u8 = 10;

pub const RESERVED_CAPABILITY_VERB_TAG_FIRST: u8 = 45;

pub const RESERVED_CAPABILITY_VERB_TAG_COUNT: u8 = 32;

/// First tag after the capability-reserved block.
pub const RESERVED_CAPABILITY_VERB_TAG_END: u8 =
    RESERVED_CAPABILITY_VERB_TAG_FIRST + RESERVED_CAPABILITY_VERB_TAG_COUNT;

pub const VERB_TAG_APPEND_FRONTIER: u8 = 87;

pub const RETIRED_VERB_TAGS: [u8; 9] = [20, 35, 36, 37, 38, 39, 40, 81, 82];

pub const RETIRED_PARTITION_TAGS: [u8; 1] = [7];

const PARTITION_TAGS: [(Partition, u8); 9] = [
    (Partition::SessionMechanics, 1),
    (Partition::Declaration, 2),
    (Partition::Query, 3),
    (Partition::Subscription, 4),
    (Partition::EventInjection, 5),
    (Partition::LedgerTransition, 6),
    (Partition::ReplayControl, 8),
    (Partition::Experimental, 9),
    (Partition::Lifecycle, 11),
];

const VERB_TAGS: [(u8, StableVerb); 45] = [
    (1, StableVerb::SessionMechanics(SessionMechanicsVerb::Hello)),
    (
        2,
        StableVerb::SessionMechanics(SessionMechanicsVerb::HelloAck),
    ),
    (
        3,
        StableVerb::SessionMechanics(SessionMechanicsVerb::Goodbye),
    ),
    (4, StableVerb::Declaration(DeclarationVerb::BeginEpoch)),
    (5, StableVerb::Declaration(DeclarationVerb::ValidateEpoch)),
    (6, StableVerb::Declaration(DeclarationVerb::CommitEpoch)),
    (7, StableVerb::Declaration(DeclarationVerb::AbortEpoch)),
    (8, StableVerb::Declaration(DeclarationVerb::UpsertActor)),
    (9, StableVerb::Declaration(DeclarationVerb::RetireActor)),
    (10, StableVerb::Declaration(DeclarationVerb::UpsertEdge)),
    (11, StableVerb::Declaration(DeclarationVerb::RetireEdge)),
    (12, StableVerb::Declaration(DeclarationVerb::UpsertScope)),
    (13, StableVerb::Declaration(DeclarationVerb::RetireScope)),
    (
        14,
        StableVerb::Declaration(DeclarationVerb::UpsertExportMount),
    ),
    (
        15,
        StableVerb::Declaration(DeclarationVerb::RetireExportMount),
    ),
    (
        16,
        StableVerb::Declaration(DeclarationVerb::UpsertAnnotation),
    ),
    (
        17,
        StableVerb::Declaration(DeclarationVerb::RetireAnnotation),
    ),
    (
        18,
        StableVerb::Declaration(DeclarationVerb::SetPresentation),
    ),
    (19, StableVerb::Declaration(DeclarationVerb::SetFlags)),
    (21, StableVerb::Declaration(DeclarationVerb::CommandResult)),
    (22, StableVerb::Query(QueryVerb::Query)),
    (23, StableVerb::Query(QueryVerb::QueryResult)),
    (24, StableVerb::Subscription(SubscriptionVerb::Subscribe)),
    (25, StableVerb::Subscription(SubscriptionVerb::SubscribeAck)),
    (26, StableVerb::Subscription(SubscriptionVerb::Credit)),
    (27, StableVerb::Subscription(SubscriptionVerb::Unsubscribe)),
    (28, StableVerb::Subscription(SubscriptionVerb::Frame)),
    (
        29,
        StableVerb::Subscription(SubscriptionVerb::SubscriptionEnded),
    ),
    (30, StableVerb::EventInjection(EventInjectionVerb::Inject)),
    (
        31,
        StableVerb::EventInjection(EventInjectionVerb::InjectAck),
    ),
    (
        32,
        StableVerb::LedgerTransition(LedgerTransitionVerb::ApprovalDecide),
    ),
    (
        33,
        StableVerb::LedgerTransition(LedgerTransitionVerb::SetObservationControl),
    ),
    (
        34,
        StableVerb::LedgerTransition(LedgerTransitionVerb::TransitionResult),
    ),
    (
        41,
        StableVerb::ReplayControl(ReplayControlVerb::ReplayStart),
    ),
    (
        42,
        StableVerb::ReplayControl(ReplayControlVerb::ReplayRewind),
    ),
    (43, StableVerb::ReplayControl(ReplayControlVerb::ReplayEnd)),
    (
        44,
        StableVerb::ReplayControl(ReplayControlVerb::ReplayResult),
    ),
    (77, StableVerb::Lifecycle(LifecycleVerb::Resume)),
    (78, StableVerb::Lifecycle(LifecycleVerb::Pause)),
    (79, StableVerb::Lifecycle(LifecycleVerb::LifecycleResult)),
    (80, StableVerb::Declaration(DeclarationVerb::MoveToScope)),
    (83, StableVerb::Declaration(DeclarationVerb::UpsertTemplate)),
    (84, StableVerb::Declaration(DeclarationVerb::RetireTemplate)),
    (85, StableVerb::Query(QueryVerb::QueryClose)),
    (
        86,
        StableVerb::LedgerTransition(LedgerTransitionVerb::SetAgentHarness),
    ),
];

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum StableVerb {
    SessionMechanics(SessionMechanicsVerb),
    Declaration(DeclarationVerb),
    Query(QueryVerb),
    Subscription(SubscriptionVerb),
    EventInjection(EventInjectionVerb),
    LedgerTransition(LedgerTransitionVerb),
    ReplayControl(ReplayControlVerb),
    Lifecycle(LifecycleVerb),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Body {
    Absent,
    Value,
}

impl StableVerb {
    #[must_use]
    pub const fn partition(self) -> Partition {
        match self {
            Self::SessionMechanics(_) => Partition::SessionMechanics,
            Self::Declaration(_) => Partition::Declaration,
            Self::Query(_) => Partition::Query,
            Self::Subscription(_) => Partition::Subscription,
            Self::EventInjection(_) => Partition::EventInjection,
            Self::LedgerTransition(_) => Partition::LedgerTransition,
            Self::ReplayControl(_) => Partition::ReplayControl,
            Self::Lifecycle(_) => Partition::Lifecycle,
        }
    }

    #[must_use]
    pub const fn body(self) -> Body {
        match self {
            Self::SessionMechanics(SessionMechanicsVerb::Goodbye)
            | Self::Query(QueryVerb::QueryClose)
            | Self::Subscription(SubscriptionVerb::Unsubscribe)
            | Self::ReplayControl(ReplayControlVerb::ReplayEnd) => Body::Absent,
            Self::SessionMechanics(
                SessionMechanicsVerb::Hello | SessionMechanicsVerb::HelloAck,
            )
            | Self::Query(QueryVerb::Query | QueryVerb::QueryResult)
            | Self::Subscription(
                SubscriptionVerb::Subscribe
                | SubscriptionVerb::SubscribeAck
                | SubscriptionVerb::Credit
                | SubscriptionVerb::Frame
                | SubscriptionVerb::SubscriptionEnded,
            )
            | Self::ReplayControl(
                ReplayControlVerb::ReplayStart
                | ReplayControlVerb::ReplayRewind
                | ReplayControlVerb::ReplayResult,
            )
            | Self::Declaration(_)
            | Self::EventInjection(_)
            | Self::LedgerTransition(_)
            | Self::Lifecycle(_) => Body::Value,
        }
    }

    pub fn all() -> impl Iterator<Item = Self> {
        SessionMechanicsVerb::ALL
            .into_iter()
            .map(Self::SessionMechanics)
            .chain(DeclarationVerb::ALL.into_iter().map(Self::Declaration))
            .chain(QueryVerb::ALL.into_iter().map(Self::Query))
            .chain(SubscriptionVerb::ALL.into_iter().map(Self::Subscription))
            .chain(
                EventInjectionVerb::ALL
                    .into_iter()
                    .map(Self::EventInjection),
            )
            .chain(
                LedgerTransitionVerb::ALL
                    .into_iter()
                    .map(Self::LedgerTransition),
            )
            .chain(ReplayControlVerb::ALL.into_iter().map(Self::ReplayControl))
            .chain(LifecycleVerb::ALL.into_iter().map(Self::Lifecycle))
    }

    pub fn open_absent_body(self, bytes: &[u8]) -> Result<(), PayloadRejection> {
        match (self.body(), bytes) {
            (Body::Absent, []) => Ok(()),
            _ => Err(PayloadRejection::UnknownKey(String::new())),
        }
    }
}

#[must_use]
pub const fn partition_tag(partition: Partition) -> u8 {
    let mut index = 0;
    while index < PARTITION_TAGS.len() {
        if PARTITION_TAGS[index].0 as u8 == partition as u8 {
            return PARTITION_TAGS[index].1;
        }
        index += 1;
    }
    unreachable!()
}

#[must_use]
pub fn verb_tag(verb: StableVerb) -> u8 {
    VERB_TAGS
        .iter()
        .find(|(_, candidate)| *candidate == verb)
        .map(|(tag, _)| *tag)
        .expect("the ledger covers every stable verb")
}

#[must_use]
pub fn verb_from_tag(tag: u8) -> Option<StableVerb> {
    VERB_TAGS
        .iter()
        .find(|(candidate, _)| *candidate == tag)
        .map(|(_, verb)| *verb)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnvelopeHeader {
    protocol_version: u16,
    verb: StableVerb,
    correlation: u32,
    payload_length: u32,
}

impl EnvelopeHeader {
    #[must_use]
    pub const fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    #[must_use]
    pub const fn verb(&self) -> StableVerb {
        self.verb
    }

    #[must_use]
    pub const fn partition(&self) -> Partition {
        self.verb.partition()
    }

    #[must_use]
    pub const fn correlation(&self) -> u32 {
        self.correlation
    }

    #[must_use]
    pub const fn payload_length(&self) -> u32 {
        self.payload_length
    }
}

#[must_use]
pub fn encode_header(
    protocol_version: u16,
    verb: StableVerb,
    correlation: u32,
    payload_length: u32,
) -> [u8; ENVELOPE_HEADER_BYTES] {
    let mut bytes = [0_u8; ENVELOPE_HEADER_BYTES];
    bytes[0..2].copy_from_slice(&protocol_version.to_be_bytes());
    bytes[2] = partition_tag(verb.partition());
    bytes[3] = verb_tag(verb);
    bytes[4..8].copy_from_slice(&correlation.to_be_bytes());
    bytes[8..12].copy_from_slice(&payload_length.to_be_bytes());
    bytes
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VersionInvariantPrefix {
    protocol_version: u16,
    partition_tag: u8,
    verb_tag: u8,
    correlation: u32,
}

impl VersionInvariantPrefix {
    #[must_use]
    pub const fn protocol_version(&self) -> u16 {
        self.protocol_version
    }

    #[must_use]
    pub const fn correlation(&self) -> u32 {
        self.correlation
    }
}

pub fn decode_version_invariant_prefix(
    bytes: &[u8],
) -> Result<VersionInvariantPrefix, HeaderRejection> {
    if bytes.len() < VERSION_INVARIANT_PREFIX_BYTES {
        return Err(HeaderRejection::Truncated {
            need: VERSION_INVARIANT_PREFIX_BYTES,
            got: bytes.len(),
        });
    }
    Ok(VersionInvariantPrefix {
        protocol_version: u16::from_be_bytes([bytes[0], bytes[1]]),
        partition_tag: bytes[2],
        verb_tag: bytes[3],
        correlation: u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
    })
}

pub fn decode_header(bytes: &[u8]) -> Result<EnvelopeHeader, HeaderRejection> {
    let prefix = decode_version_invariant_prefix(bytes)?;
    if bytes.len() < ENVELOPE_HEADER_BYTES {
        return Err(HeaderRejection::Truncated {
            need: ENVELOPE_HEADER_BYTES,
            got: bytes.len(),
        });
    }
    if prefix.protocol_version == 0 {
        return Err(HeaderRejection::ZeroProtocolVersion);
    }
    if prefix.protocol_version != INITIAL_PROTOCOL_VERSION {
        return Err(HeaderRejection::UnknownProtocolVersion {
            version: prefix.protocol_version,
        });
    }
    let Some(verb) = verb_from_tag(prefix.verb_tag) else {
        return Err(HeaderRejection::UnknownVerbTag {
            tag: prefix.verb_tag,
        });
    };
    let expected = partition_tag(verb.partition());
    if prefix.partition_tag != expected {
        return Err(HeaderRejection::PartitionTagDisagreesWithVerb {
            declared: prefix.partition_tag,
            expected,
        });
    }
    Ok(EnvelopeHeader {
        protocol_version: prefix.protocol_version,
        verb,
        correlation: prefix.correlation,
        payload_length: u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecodedEnvelopeFrame<'frame> {
    header: EnvelopeHeader,
    payload: &'frame [u8],
}

impl<'frame> DecodedEnvelopeFrame<'frame> {
    #[must_use]
    pub const fn header(&self) -> &EnvelopeHeader {
        &self.header
    }

    #[must_use]
    pub const fn payload(&self) -> &'frame [u8] {
        self.payload
    }
}

pub fn encode_envelope_frame(
    protocol_version: u16,
    verb: StableVerb,
    correlation: u32,
    payload: &[u8],
) -> Result<Vec<u8>, FrameRejection> {
    let declared = u32::try_from(payload.len()).map_err(|_| FrameRejection::PayloadTooLong {
        available: payload.len(),
    })?;
    let header = encode_header(protocol_version, verb, correlation, declared);
    let mut frame = Vec::with_capacity(ENVELOPE_HEADER_BYTES + payload.len());
    frame.extend_from_slice(&header);
    frame.extend_from_slice(payload);
    Ok(frame)
}

pub fn decode_envelope_frame(bytes: &[u8]) -> Result<DecodedEnvelopeFrame<'_>, FrameRejection> {
    let header = decode_header(bytes).map_err(FrameRejection::Header)?;
    let declared = header.payload_length() as usize;
    let available = bytes.len() - ENVELOPE_HEADER_BYTES;
    if available < declared {
        return Err(FrameRejection::PayloadShorterThanDeclared {
            declared: header.payload_length(),
            available,
        });
    }
    if available > declared {
        return Err(FrameRejection::TrailingBytes {
            declared: header.payload_length(),
            available,
        });
    }
    Ok(DecodedEnvelopeFrame {
        header,
        payload: &bytes[ENVELOPE_HEADER_BYTES..],
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameRejection {
    Header(HeaderRejection),
    PayloadShorterThanDeclared { declared: u32, available: usize },
    TrailingBytes { declared: u32, available: usize },
    PayloadTooLong { available: usize },
}

impl fmt::Display for FrameRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Header(rejection) => write!(formatter, "{rejection}"),
            Self::PayloadShorterThanDeclared {
                declared,
                available,
            } => write!(
                formatter,
                "payload declares {declared} bytes but only {available} bytes remain"
            ),
            Self::TrailingBytes {
                declared,
                available,
            } => write!(
                formatter,
                "payload declares {declared} bytes but {available} bytes remain"
            ),
            Self::PayloadTooLong { available } => write!(
                formatter,
                "payload of {available} bytes exceeds the length field's value space"
            ),
        }
    }
}

impl std::error::Error for FrameRejection {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HeaderRejection {
    Truncated { need: usize, got: usize },
    ZeroProtocolVersion,
    UnknownProtocolVersion { version: u16 },
    UnknownVerbTag { tag: u8 },
    PartitionTagDisagreesWithVerb { declared: u8, expected: u8 },
}

impl fmt::Display for HeaderRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { need, got } => {
                write!(
                    formatter,
                    "envelope header requires {need} bytes; received {got}"
                )
            }
            Self::ZeroProtocolVersion => {
                formatter.write_str("protocol version 0 is not a published version")
            }
            Self::UnknownProtocolVersion { version } => {
                write!(formatter, "unknown protocol version {version}")
            }
            Self::UnknownVerbTag { tag } => write!(formatter, "unknown verb tag {tag}"),
            Self::PartitionTagDisagreesWithVerb { declared, expected } => write!(
                formatter,
                "partition tag {declared} differs from the verb's required tag {expected}"
            ),
        }
    }
}

impl std::error::Error for HeaderRejection {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::STABLE_VERB_COUNT;
    use std::collections::BTreeSet;

    fn every_stable_verb() -> Vec<StableVerb> {
        StableVerb::all().collect()
    }

    #[test]
    fn the_verb_ledger_is_a_bijection_around_the_reserved_capability_block() {
        assert_eq!(VERB_TAGS.len(), STABLE_VERB_COUNT);

        let tags = VERB_TAGS
            .iter()
            .map(|(tag, _)| *tag)
            .collect::<BTreeSet<_>>();
        assert_eq!(tags.len(), STABLE_VERB_COUNT, "no two verbs share a number");
        assert_eq!(tags.first(), Some(&1), "0 is no verb's tag");
        assert_eq!(tags.last(), Some(&86));

        let verbs = VERB_TAGS
            .iter()
            .map(|(_, verb)| *verb)
            .collect::<BTreeSet<_>>();
        assert_eq!(verbs.len(), STABLE_VERB_COUNT, "no verb uses two numbers");
    }

    #[test]
    fn retired_declaration_tags_are_unknown() {
        for tag in [81, 82] {
            let header = [0, 1, 2, tag, 0, 0, 0, 7, 0, 0, 0, 1];
            assert_eq!(
                decode_header(&header),
                Err(HeaderRejection::UnknownVerbTag { tag })
            );
        }
    }

    #[test]
    fn a_reserved_tag_is_refused_exactly_like_an_unassigned_one() {
        for tag in RESERVED_CAPABILITY_VERB_TAG_FIRST..RESERVED_CAPABILITY_VERB_TAG_END {
            assert_eq!(verb_from_tag(tag), None);
            let mut bytes = encode_header(
                INITIAL_PROTOCOL_VERSION,
                StableVerb::Query(QueryVerb::Query),
                7,
                0,
            );
            bytes[3] = tag;
            assert_eq!(
                decode_header(&bytes),
                Err(HeaderRejection::UnknownVerbTag { tag })
            );
        }

        let unassigned = VERB_TAG_APPEND_FRONTIER;
        let mut bytes = encode_header(
            INITIAL_PROTOCOL_VERSION,
            StableVerb::Query(QueryVerb::Query),
            7,
            0,
        );
        bytes[3] = unassigned;
        assert_eq!(
            decode_header(&bytes),
            Err(HeaderRejection::UnknownVerbTag { tag: unassigned }),
            "an unassigned tag outside the reserved range is refused for the same reason"
        );
    }

    #[test]
    fn an_absent_body_is_zero_bytes_and_nothing_else() {
        let null = circular_core::encode(
            &circular_core::Value::Null,
            circular_core::Ceilings::for_boundary(circular_core::Boundary::Wire),
        )
        .expect("Null encodes");
        for verb in every_stable_verb()
            .into_iter()
            .filter(|verb| verb.body() == Body::Absent)
        {
            assert_eq!(verb.open_absent_body(&[]), Ok(()), "{verb:?}");
            assert!(verb.open_absent_body(&null).is_err(), "{verb:?}");
        }
    }

    #[test]
    fn every_stable_verb_round_trips_through_its_bytes() {
        for verb in every_stable_verb() {
            let bytes = encode_header(INITIAL_PROTOCOL_VERSION, verb, 0x0102_0304, 0x0000_00ff);
            assert_eq!(bytes.len(), ENVELOPE_HEADER_BYTES);
            let header = decode_header(&bytes).expect("a published tag is read");
            assert_eq!(header.verb(), verb);
            assert_eq!(header.partition(), verb.partition());
            assert_eq!(header.correlation(), 0x0102_0304);
            assert_eq!(header.payload_length(), 0xff);
            assert_eq!(header.protocol_version(), INITIAL_PROTOCOL_VERSION);
        }
    }

    #[test]
    fn a_head_that_disagrees_with_its_verb_is_refused() {
        let mut bytes = encode_header(1, StableVerb::Query(QueryVerb::Query), 1, 0);
        bytes[2] = partition_tag(Partition::Declaration);
        assert_eq!(
            decode_header(&bytes),
            Err(HeaderRejection::PartitionTagDisagreesWithVerb {
                declared: 2,
                expected: 3,
            })
        );
    }

    #[test]
    fn zero_filled_bytes_are_refused_at_byte_zero() {
        let zeroes = [0_u8; ENVELOPE_HEADER_BYTES];
        assert_eq!(
            decode_header(&zeroes),
            Err(HeaderRejection::ZeroProtocolVersion),
            "zero fill dies at the version field"
        );
        let long_zeroes = [0_u8; ENVELOPE_HEADER_BYTES * 4];
        assert_eq!(
            decode_header(&long_zeroes),
            Err(HeaderRejection::ZeroProtocolVersion)
        );
    }

    #[test]
    fn every_truncation_is_refused_and_none_yields_a_header() {
        let bytes = encode_header(1, StableVerb::Query(QueryVerb::Query), 5, 0);
        for cut in 0..ENVELOPE_HEADER_BYTES {
            let rejection =
                decode_header(&bytes[..cut]).expect_err("a truncation does not become a header");
            assert!(matches!(rejection, HeaderRejection::Truncated { .. }));
        }
        assert!(decode_header(&bytes).is_ok());
    }

    #[test]
    fn an_unknown_version_still_yields_its_correlation() {
        let mut bytes = encode_header(1, StableVerb::Query(QueryVerb::Query), 0xdead_beef, 0);
        bytes[0..2].copy_from_slice(&999_u16.to_be_bytes());

        let prefix = decode_version_invariant_prefix(&bytes)
            .expect("the prefix does not depend on the version");
        assert_eq!(prefix.protocol_version(), 999);
        assert_eq!(
            prefix.correlation(),
            0xdead_beef,
            "an envelope with an unknown version still yields its correlation key"
        );

        assert_eq!(
            decode_header(&bytes),
            Err(HeaderRejection::UnknownProtocolVersion { version: 999 }),
            "and still, as a whole header, it is refused"
        );
    }

    #[test]
    fn the_prefix_is_readable_from_fewer_bytes_than_a_whole_header() {
        let bytes = encode_header(1, StableVerb::Query(QueryVerb::Query), 42, 7);
        let prefix = decode_version_invariant_prefix(&bytes[..VERSION_INVARIANT_PREFIX_BYTES])
            .expect("the prefix does not require the length field");
        assert_eq!(prefix.correlation(), 42);
        const { assert!(VERSION_INVARIANT_PREFIX_BYTES < ENVELOPE_HEADER_BYTES) };
    }

    #[test]
    fn a_declared_length_longer_than_the_frame_is_refused() {
        let mut frame = encode_envelope_frame(
            INITIAL_PROTOCOL_VERSION,
            StableVerb::Query(QueryVerb::Query),
            1,
            b"abc",
        )
        .expect("within the length field");
        frame[8..12].copy_from_slice(&9_u32.to_be_bytes());

        assert_eq!(
            decode_envelope_frame(&frame),
            Err(FrameRejection::PayloadShorterThanDeclared {
                declared: 9,
                available: 3,
            })
        );
    }

    #[test]
    fn bytes_past_the_declared_payload_are_refused() {
        let mut frame = encode_envelope_frame(
            INITIAL_PROTOCOL_VERSION,
            StableVerb::Query(QueryVerb::Query),
            1,
            b"abc",
        )
        .expect("within the length field");
        frame.push(b'!');

        assert_eq!(
            decode_envelope_frame(&frame),
            Err(FrameRejection::TrailingBytes {
                declared: 3,
                available: 4,
            })
        );
    }

    #[test]
    fn the_length_field_cannot_be_chosen_by_the_caller() {
        let frame = encode_envelope_frame(
            INITIAL_PROTOCOL_VERSION,
            StableVerb::Query(QueryVerb::Query),
            1,
            b"twelve bytes",
        )
        .expect("within the length field");
        assert_eq!(
            u32::from_be_bytes([frame[8], frame[9], frame[10], frame[11]]),
            12
        );
        assert!(decode_envelope_frame(&frame).is_ok());
    }

    #[test]
    fn the_published_widths_add_up_to_the_published_header_size() {
        assert_eq!(ENVELOPE_HEADER_BYTES, 12);
        assert_eq!(VERSION_INVARIANT_PREFIX_BYTES, 8);
        assert_eq!(PROTOCOL_VERSION_BYTES, 2);
        assert_eq!(CORRELATION_BYTES, 4);
        const { assert!((MAX_LIVE_CORRELATIONS as u64) < 1_u64 << (CORRELATION_BYTES * 8)) };
    }
}

#[cfg(test)]
mod live_correlation_tests {
    use super::*;

    #[test]
    fn live_correlations_refuse_4097_and_duplicate_then_reuse_finished_key() {
        let mut live = LiveCorrelations::default();
        for key in 1..=4096 {
            assert!(live.try_open(key));
        }
        assert!(!live.try_open(4097));
        assert!(!live.try_open(12));
        live.finish(12);
        assert!(live.try_open(4097));
        assert!(!live.try_open(12));
        live.finish(4097);
        assert!(live.try_open(12));
    }
}
