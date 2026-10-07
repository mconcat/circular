
use crate::byte_reader::{BigEndian, ByteReadError, ByteReader};
use crate::value::{FloatValue, ObjectValue, Value, ValueKind};
use std::collections::BTreeMap;
use std::fmt;

pub const VALUE_FORMAT_VERSION: u16 = 1;

pub const CANONICAL_VALUE_TAG: &str = "value.v1";

const CANONICAL_NAN_BITS: u64 = 0x7ff8_0000_0000_0000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Ceilings {
    max_bytes: usize,
    max_depth: usize,
    max_container_entries: usize,
    max_string_bytes: usize,
}

impl Ceilings {
    /// Maximum encoded value size for bounded producers.
    #[must_use]
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }

    #[must_use]
    pub const fn max_depth(self) -> usize {
        self.max_depth
    }

    #[must_use]
    pub const fn max_container_entries(self) -> usize {
        self.max_container_entries
    }

    #[must_use]
    pub const fn max_string_bytes(self) -> usize {
        self.max_string_bytes
    }

    pub(crate) const PROVISIONAL: Self = Self {
        max_bytes: 1 << 20,
        max_depth: 64,
        max_container_entries: 65_535,
        max_string_bytes: 1 << 20,
    };

    pub const UNBOUNDED: Self = Self {
        max_bytes: usize::MAX,
        max_depth: usize::MAX,
        max_container_entries: usize::MAX,
        max_string_bytes: usize::MAX,
    };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Boundary {
    Wire,
    Journal,
    ActorState,
    Config,
    Identity,
}

impl Ceilings {
    #[must_use]
    pub const fn for_boundary(boundary: Boundary) -> Self {
        match boundary {
            Boundary::Wire
            | Boundary::Journal
            | Boundary::ActorState
            | Boundary::Config
            | Boundary::Identity => Self::PROVISIONAL,
        }
    }
}

pub const MAX_VALUES_PER_BODY: usize = 16;

pub const MAX_SEGMENTS_PER_VALUE: usize = 16;

pub const MAX_REASSEMBLED_BODY_BYTES: usize =
    Ceilings::for_boundary(Boundary::Wire).max_bytes() * MAX_VALUES_PER_BODY;

pub const MAX_SEGMENT_BODY_BYTES: usize =
    Ceilings::for_boundary(Boundary::Wire).max_bytes() / MAX_SEGMENTS_PER_VALUE;

pub const MAX_TEXT_BODY_BYTES: usize = Ceilings::for_boundary(Boundary::Wire).max_string_bytes();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecError {
    Truncated,
    TrailingBytes,
    UnknownTag(u8),
    InvalidBooleanBody(u8),
    NonDesignatedNan(u64),
    InvalidUtf8,
    ObjectKeyOrder,
    UnsupportedFormatVersion(u16),
    FrameLengthMismatch,
    CeilingExceeded(Ceiling),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ceiling {
    Bytes,
    Depth,
    ContainerEntries,
    StringBytes,
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => formatter.write_str("value bytes ended before filling the field"),
            Self::TrailingBytes => formatter.write_str("bytes remain after the top-level value"),
            Self::UnknownTag(tag) => write!(formatter, "unknown value tag {tag}"),
            Self::InvalidBooleanBody(byte) => {
                write!(formatter, "a Bool body must be 00 or 01: {byte:#04x}")
            }
            Self::NonDesignatedNan(bits) => {
                write!(formatter, "non-canonical NaN bit pattern {bits:#018x}")
            }
            Self::InvalidUtf8 => formatter.write_str("string is not strict UTF-8"),
            Self::ObjectKeyOrder => {
                formatter.write_str("Object keys are not in strictly increasing raw byte order")
            }
            Self::UnsupportedFormatVersion(version) => {
                write!(formatter, "unpublished value format version {version}")
            }
            Self::FrameLengthMismatch => formatter
                .write_str("the frame's declared length differs from the actual byte count"),
            Self::CeilingExceeded(ceiling) => write!(formatter, "ceiling exceeded: {ceiling:?}"),
        }
    }
}

impl std::error::Error for CodecError {}

type Result<T> = std::result::Result<T, CodecError>;

type ValueReader<'bytes> = ByteReader<'bytes, BigEndian>;

impl From<ByteReadError> for CodecError {
    fn from(_: ByteReadError) -> Self {
        Self::Truncated
    }
}

pub fn encode(value: &Value, ceilings: Ceilings) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    encode_into(value, ceilings, 1, &mut output)?;
    Ok(output)
}

fn encode_into(
    value: &Value,
    ceilings: Ceilings,
    depth: usize,
    output: &mut Vec<u8>,
) -> Result<()> {
    if depth > ceilings.max_depth {
        return Err(CodecError::CeilingExceeded(Ceiling::Depth));
    }
    push_byte(output, value.kind().tag(), ceilings)?;
    match value {
        Value::Null => {}
        Value::Bool(flag) => push_byte(output, u8::from(*flag), ceilings)?,
        Value::Int(number) => push_slice(output, &number.to_be_bytes(), ceilings)?,
        Value::UInt(number) => push_slice(output, &number.to_be_bytes(), ceilings)?,
        Value::Float(number) => push_slice(output, &number.to_bits().to_be_bytes(), ceilings)?,
        Value::String(text) => encode_len_prefixed(output, text.as_bytes(), ceilings, true)?,
        Value::Bytes(bytes) => encode_len_prefixed(output, bytes, ceilings, false)?,
        Value::Array(items) => {
            encode_count(output, items.len(), ceilings)?;
            for item in items {
                encode_into(item, ceilings, depth + 1, output)?;
            }
        }
        Value::Object(object) => {
            encode_count(output, object.len(), ceilings)?;
            for (key, item) in object.iter() {
                encode_len_prefixed(output, key.as_bytes(), ceilings, true)?;
                encode_into(item, ceilings, depth + 1, output)?;
            }
        }
    }
    Ok(())
}

fn encode_count(output: &mut Vec<u8>, count: usize, ceilings: Ceilings) -> Result<()> {
    if count > ceilings.max_container_entries {
        return Err(CodecError::CeilingExceeded(Ceiling::ContainerEntries));
    }
    let count =
        u32::try_from(count).map_err(|_| CodecError::CeilingExceeded(Ceiling::ContainerEntries))?;
    push_slice(output, &count.to_be_bytes(), ceilings)
}

fn encode_len_prefixed(
    output: &mut Vec<u8>,
    bytes: &[u8],
    ceilings: Ceilings,
    is_text: bool,
) -> Result<()> {
    let ceiling = if is_text {
        Ceiling::StringBytes
    } else {
        Ceiling::Bytes
    };
    if bytes.len() > ceilings.max_string_bytes {
        return Err(CodecError::CeilingExceeded(ceiling));
    }
    let length = u32::try_from(bytes.len()).map_err(|_| CodecError::CeilingExceeded(ceiling))?;
    push_slice(output, &length.to_be_bytes(), ceilings)?;
    push_slice(output, bytes, ceilings)
}

fn push_byte(output: &mut Vec<u8>, byte: u8, ceilings: Ceilings) -> Result<()> {
    push_slice(output, &[byte], ceilings)
}

fn push_slice(output: &mut Vec<u8>, bytes: &[u8], ceilings: Ceilings) -> Result<()> {
    if output.len() + bytes.len() > ceilings.max_bytes {
        return Err(CodecError::CeilingExceeded(Ceiling::Bytes));
    }
    output.extend_from_slice(bytes);
    Ok(())
}

pub fn decode(bytes: &[u8], ceilings: Ceilings) -> Result<Value> {
    if bytes.len() > ceilings.max_bytes {
        return Err(CodecError::CeilingExceeded(Ceiling::Bytes));
    }
    let mut reader = ValueReader::new(bytes);
    let value = decode_value(&mut reader, ceilings, 1)?;
    if !reader.finished() {
        return Err(CodecError::TrailingBytes);
    }
    Ok(value)
}

fn decode_value(reader: &mut ValueReader<'_>, ceilings: Ceilings, depth: usize) -> Result<Value> {
    if depth > ceilings.max_depth {
        return Err(CodecError::CeilingExceeded(Ceiling::Depth));
    }
    let tag = reader.byte()?;
    let Some(kind) = ValueKind::from_tag(tag) else {
        return Err(CodecError::UnknownTag(tag));
    };
    match kind {
        ValueKind::Null => Ok(Value::Null),
        ValueKind::Bool => reader
            .flag()?
            .map(Value::Bool)
            .map_err(CodecError::InvalidBooleanBody),
        ValueKind::Int => Ok(Value::Int(reader.i64()?)),
        ValueKind::UInt => Ok(Value::UInt(reader.u64()?)),
        ValueKind::Float => {
            let bits = reader.u64()?;
            let number = f64::from_bits(bits);
            if number.is_nan() && bits != CANONICAL_NAN_BITS {
                return Err(CodecError::NonDesignatedNan(bits));
            }
            Ok(Value::Float(FloatValue::new(number)))
        }
        ValueKind::String => Ok(Value::String(decode_text(reader, ceilings)?)),
        ValueKind::Bytes => {
            let length = decode_length(reader, ceilings, Ceiling::Bytes)?;
            Ok(Value::Bytes(reader.take(length)?.to_vec()))
        }
        ValueKind::Array => {
            let count = decode_count(reader, ceilings)?;
            let mut items = Vec::with_capacity(count.min(1024));
            for _ in 0..count {
                items.push(decode_value(reader, ceilings, depth + 1)?);
            }
            Ok(Value::Array(items))
        }
        ValueKind::Object => {
            let count = decode_count(reader, ceilings)?;
            let mut entries = BTreeMap::new();
            let mut previous: Option<String> = None;
            for _ in 0..count {
                let key = decode_text(reader, ceilings)?;
                if let Some(previous) = &previous
                    && key.as_bytes() <= previous.as_bytes()
                {
                    return Err(CodecError::ObjectKeyOrder);
                }
                let value = decode_value(reader, ceilings, depth + 1)?;
                previous = Some(key.clone());
                entries.insert(key, value);
            }
            Ok(Value::Object(ObjectValue::from_map(entries)))
        }
    }
}

fn decode_count(reader: &mut ValueReader<'_>, ceilings: Ceilings) -> Result<usize> {
    let count = reader.u32()? as usize;
    if count > ceilings.max_container_entries {
        return Err(CodecError::CeilingExceeded(Ceiling::ContainerEntries));
    }
    Ok(count)
}

fn decode_length(
    reader: &mut ValueReader<'_>,
    ceilings: Ceilings,
    ceiling: Ceiling,
) -> Result<usize> {
    let length = reader.u32()? as usize;
    if length > ceilings.max_string_bytes {
        return Err(CodecError::CeilingExceeded(ceiling));
    }
    Ok(length)
}

fn decode_text(reader: &mut ValueReader<'_>, ceilings: Ceilings) -> Result<String> {
    let length = decode_length(reader, ceilings, Ceiling::StringBytes)?;
    let bytes = reader.take(length)?;
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| CodecError::InvalidUtf8)
}

pub fn encode_frame(value: &Value, ceilings: Ceilings) -> Result<Vec<u8>> {
    let inner = encode(value, ceilings)?;
    let length =
        u32::try_from(inner.len()).map_err(|_| CodecError::CeilingExceeded(Ceiling::Bytes))?;
    let mut output = Vec::with_capacity(6 + inner.len());
    output.extend_from_slice(&VALUE_FORMAT_VERSION.to_be_bytes());
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(&inner);
    Ok(output)
}

pub fn decode_frame(bytes: &[u8], ceilings: Ceilings) -> Result<Value> {
    let mut reader = ValueReader::new(bytes);
    let version = reader.u16()?;
    if version != VALUE_FORMAT_VERSION {
        return Err(CodecError::UnsupportedFormatVersion(version));
    }
    let declared = reader.u32()? as usize;
    let inner = &bytes[6..];
    if inner.len() != declared {
        return Err(CodecError::FrameLengthMismatch);
    }
    decode(inner, ceilings)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CEILINGS: Ceilings = Ceilings::for_boundary(Boundary::Journal);

    #[test]
    fn the_derived_bounds_are_whole_multiples_of_one_value_ceiling() {
        let value = Ceilings::for_boundary(Boundary::Wire).max_bytes();
        assert_eq!(MAX_REASSEMBLED_BODY_BYTES / value, MAX_VALUES_PER_BODY);
        assert_eq!(MAX_REASSEMBLED_BODY_BYTES % value, 0);
        assert_eq!(value / MAX_SEGMENT_BODY_BYTES, MAX_SEGMENTS_PER_VALUE);
        assert_eq!(value % MAX_SEGMENT_BODY_BYTES, 0);
    }

    #[test]
    fn the_unbounded_ceilings_reject_nothing() {
        assert_eq!(Ceilings::UNBOUNDED.max_bytes(), usize::MAX);
        assert_eq!(Ceilings::UNBOUNDED.max_container_entries(), usize::MAX);
        assert_eq!(Ceilings::UNBOUNDED.max_string_bytes(), usize::MAX);
        let deep = Value::array((0..1_000).map(|n| Value::UInt(n)));
        let bytes = encode(&deep, Ceilings::UNBOUNDED).expect("unbounded encodes");
        assert_eq!(
            decode(&bytes, Ceilings::UNBOUNDED).expect("unbounded decodes"),
            deep
        );
    }

    fn round_trip(value: &Value) -> Vec<u8> {
        let bytes = encode(value, CEILINGS).expect("fixture encodes");
        assert_eq!(&decode(&bytes, CEILINGS).expect("fixture decodes"), value);
        bytes
    }

    #[test]
    fn every_kind_round_trips_under_its_declared_tag() {
        let values = [
            Value::Null,
            Value::Bool(true),
            Value::Int(-7),
            Value::float(1.5),
            Value::string("x"),
            Value::bytes(vec![0xde, 0xad]),
            Value::Array(vec![Value::Null]),
            Value::object([("k", Value::Null)]).expect("object"),
            Value::uint(7),
        ];
        for (index, value) in values.iter().enumerate() {
            let bytes = round_trip(value);
            assert_eq!(bytes[0] as usize, index + 1, "{:?}", value.kind());
        }
        assert_eq!(ValueKind::ALL.len(), values.len());
    }

    #[test]
    fn an_integer_and_a_float_never_share_bytes() {
        assert_ne!(
            encode(&Value::Int(2), CEILINGS).unwrap(),
            encode(&Value::float(2.0), CEILINGS).unwrap()
        );
    }

    #[test]
    fn a_non_designated_nan_is_rejected_not_folded() {
        let mut bytes = encode(&Value::float(f64::NAN), CEILINGS).unwrap();
        bytes[8] = 0x42;
        assert!(matches!(
            decode(&bytes, CEILINGS),
            Err(CodecError::NonDesignatedNan(_))
        ));
    }

    #[test]
    fn object_keys_must_strictly_increase_by_raw_utf8() {
        let ordered = encode(
            &Value::object([("a", Value::Null), ("b", Value::Null)]).unwrap(),
            CEILINGS,
        )
        .unwrap();
        assert!(decode(&ordered, CEILINGS).is_ok());

        let mut reversed = ordered.clone();
        let (first, second) = (reversed[9], reversed[9 + 6]);
        reversed[9] = second;
        reversed[9 + 6] = first;
        assert_eq!(decode(&reversed, CEILINGS), Err(CodecError::ObjectKeyOrder));

        let mut duplicated = ordered;
        duplicated[9 + 6] = duplicated[9];
        assert_eq!(
            decode(&duplicated, CEILINGS),
            Err(CodecError::ObjectKeyOrder)
        );
    }

    #[test]
    fn every_rejection_class_yields_no_value() {
        for (bytes, expected) in [
            (vec![], CodecError::Truncated),
            (vec![0], CodecError::UnknownTag(0)),
            (vec![10], CodecError::UnknownTag(10)),
            (vec![1, 0], CodecError::TrailingBytes),
            (vec![2], CodecError::Truncated),
            (vec![2, 2], CodecError::InvalidBooleanBody(2)),
            (vec![3, 0, 0], CodecError::Truncated),
            (vec![5, 0, 0, 0, 2, 0xc0, 0x80], CodecError::InvalidUtf8),
            (vec![7, 0, 0, 0, 1], CodecError::Truncated),
        ] {
            assert_eq!(decode(&bytes, CEILINGS), Err(expected), "{bytes:?}");
        }
    }

    #[test]
    fn the_depth_ceiling_rejects_instead_of_truncating() {
        let shallow = Ceilings {
            max_bytes: 1 << 20,
            max_depth: 2,
            max_container_entries: 16,
            max_string_bytes: 1 << 10,
        };
        let nested = Value::Array(vec![Value::Array(vec![Value::Null])]);
        assert_eq!(
            encode(&nested, shallow),
            Err(CodecError::CeilingExceeded(Ceiling::Depth))
        );
    }

    #[test]
    fn nested_containers_round_trip_without_eating_their_sibling() {
        let value = Value::object([
            ("a", Value::Array(vec![Value::Int(1), Value::float(-0.0)])),
            ("b", Value::object([("c", Value::bytes(vec![1]))]).unwrap()),
        ])
        .unwrap();
        round_trip(&value);
    }
}
