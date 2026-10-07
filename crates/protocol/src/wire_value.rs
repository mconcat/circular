
use circular_core::{Ceilings, CodecError, Value, decode, encode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PayloadRejection {
    Codec(CodecError),
    NotAnObject,
    MissingKey(&'static str),
    UnknownKey(String),
    WrongCarrier { key: &'static str },
    UnknownArm { tag: i64 },
    ArmNotAdmitted,
    NotCanonical { key: &'static str },
    KeyDisagreesWithDeclaration,
    BeyondWidth { key: &'static str },
    ReservedLocalSpelling,
}

impl std::fmt::Display for PayloadRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Codec(error) => write!(formatter, "the body is not a value: {error}"),
            Self::NotAnObject => formatter.write_str("the body is not an object"),
            Self::MissingKey(key) => write!(formatter, "`{key}` is missing"),
            Self::UnknownKey(key) => write!(formatter, "`{key}` is not a published key"),
            Self::WrongCarrier { key } => write!(formatter, "`{key}` has the wrong value kind"),
            Self::UnknownArm { tag } => write!(formatter, "tag {tag} is not an assigned arm"),
            Self::ArmNotAdmitted => formatter.write_str("this arm is not admitted here"),
            Self::NotCanonical { key } => write!(formatter, "`{key}` is not in canonical form"),
            Self::KeyDisagreesWithDeclaration => {
                formatter.write_str("the key disagrees with its declaration")
            }
            Self::BeyondWidth { key } => {
                write!(
                    formatter,
                    "`{key}` is wider than the 64-bit integer carrier"
                )
            }
            Self::ReservedLocalSpelling => formatter
                .write_str("the actor name uses the spelling reserved for synthesized boundaries"),
        }
    }
}

impl From<WireError> for PayloadRejection {
    fn from(error: WireError) -> Self {
        match (error.kind, error.at) {
            (WireErrorKind::UnknownField(field), _) => Self::UnknownKey(field),
            (WireErrorKind::MissingField, Some(key)) => Self::MissingKey(key),
            (WireErrorKind::NotObject, Some(key)) => Self::WrongCarrier { key },
            (WireErrorKind::NotObject | WireErrorKind::MissingField, None) => Self::NotAnObject,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WireError {
    at: Option<&'static str>,
    kind: WireErrorKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WireErrorKind {
    NotObject,
    MissingField,
    UnknownField(String),
}

impl WireError {
    #[must_use]
    pub(crate) const fn not_object(at: Option<&'static str>) -> Self {
        Self {
            at,
            kind: WireErrorKind::NotObject,
        }
    }

    #[must_use]
    pub(crate) const fn missing(key: &'static str) -> Self {
        Self {
            at: Some(key),
            kind: WireErrorKind::MissingField,
        }
    }

    #[must_use]
    pub(crate) const fn unknown(field: String) -> Self {
        Self {
            at: None,
            kind: WireErrorKind::UnknownField(field),
        }
    }

    #[must_use]
    pub const fn at(&self) -> Option<&'static str> {
        self.at
    }

    #[must_use]
    pub const fn kind(&self) -> &WireErrorKind {
        &self.kind
    }
}

impl std::fmt::Display for WireError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.kind, self.at) {
            (WireErrorKind::NotObject, None) => formatter.write_str("is not an Object"),
            (WireErrorKind::NotObject, Some(key)) => write!(formatter, "{key:?} is not an Object"),
            (WireErrorKind::MissingField, Some(key)) => write!(formatter, "has no {key:?}"),
            (WireErrorKind::MissingField, None) => formatter.write_str("has a missing field"),
            (WireErrorKind::UnknownField(field), _) => {
                write!(formatter, "carries unknown field {field:?}")
            }
        }
    }
}

impl std::error::Error for WireError {}

pub(crate) fn unit_arm(tag: i64) -> Value {
    Value::Int(tag)
}

pub(crate) fn arm(tag: i64, argument: Value) -> Value {
    args_arm(tag, [argument])
}

pub(crate) fn args_arm(tag: i64, arguments: impl IntoIterator<Item = Value>) -> Value {
    Value::Array(std::iter::once(Value::Int(tag)).chain(arguments).collect())
}

pub(crate) fn decode_arm(
    value: Value,
    key: &'static str,
) -> Result<(i64, Vec<Value>), PayloadRejection> {
    match value {
        Value::Int(tag) => Ok((tag, Vec::new())),
        Value::Array(mut parts) if parts.len() >= 2 => match parts.remove(0) {
            Value::Int(tag) => Ok((tag, parts)),
            _ => Err(PayloadRejection::WrongCarrier { key }),
        },
        _ => Err(PayloadRejection::WrongCarrier { key }),
    }
}

pub(crate) fn object(
    bytes: &[u8],
    ceilings: Ceilings,
) -> Result<Vec<(String, Value)>, PayloadRejection> {
    Ok(object_from_value(
        decode(bytes, ceilings).map_err(PayloadRejection::Codec)?,
    )?)
}

pub(crate) fn object_from_value(value: Value) -> Result<Vec<(String, Value)>, WireError> {
    match value {
        Value::Object(object) => Ok(object.into_map().into_iter().collect()),
        _ => Err(WireError::not_object(None)),
    }
}

pub(crate) fn take(
    fields: &mut Vec<(String, Value)>,
    key: &'static str,
) -> Result<Value, WireError> {
    optional(fields, key).ok_or(WireError::missing(key))
}

pub(crate) fn bytes_of(value: Value, key: &'static str) -> Result<Vec<u8>, PayloadRejection> {
    match value {
        Value::Bytes(bytes) => Ok(bytes),
        _ => Err(PayloadRejection::WrongCarrier { key }),
    }
}

pub(crate) fn exhausted(fields: Vec<(String, Value)>) -> Result<(), WireError> {
    match fields.into_iter().next() {
        None => Ok(()),
        Some((key, _)) => Err(WireError::unknown(key)),
    }
}

pub(crate) fn text_of(value: Value, key: &'static str) -> Result<String, PayloadRejection> {
    match value {
        Value::String(text) => Ok(text),
        _ => Err(PayloadRejection::WrongCarrier { key }),
    }
}

pub(crate) fn bool_of(value: Value, key: &'static str) -> Result<bool, PayloadRejection> {
    match value {
        Value::Bool(flag) => Ok(flag),
        _ => Err(PayloadRejection::WrongCarrier { key }),
    }
}

pub(crate) fn object_fields(
    value: Value,
    key: &'static str,
) -> Result<Vec<(String, Value)>, WireError> {
    match value {
        Value::Object(object) => Ok(object.into_map().into_iter().collect()),
        _ => Err(WireError::not_object(Some(key))),
    }
}

pub(crate) fn int_of(value: Value, key: &'static str) -> Result<i64, PayloadRejection> {
    match value {
        Value::Int(number) => Ok(number),
        _ => Err(PayloadRejection::WrongCarrier { key }),
    }
}

pub(crate) fn unsigned_of(value: Value, key: &'static str) -> Result<u64, PayloadRejection> {
    u64::try_from(int_of(value, key)?).map_err(|_| PayloadRejection::WrongCarrier { key })
}

const fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let next = a % b;
        a = b;
        b = next;
    }
    a
}

pub(crate) fn decode_ratio(
    value: Value,
    key: &'static str,
) -> Result<(u64, u64), PayloadRejection> {
    let mut fields = object_fields(value, key)?;
    let den = unsigned_of(take(&mut fields, "den")?, "den")?;
    let num = unsigned_of(take(&mut fields, "num")?, "num")?;
    exhausted(fields)?;

    if den == 0 {
        return Err(PayloadRejection::WrongCarrier { key: "den" });
    }
    let divisor = if num == 0 { den } else { gcd(num, den) };
    if divisor != 1 {
        return Err(PayloadRejection::NotCanonical { key });
    }

    Ok((num, den))
}

pub(crate) fn decode_canonical_set<T>(
    value: Value,
    key: &'static str,
    ceilings: Ceilings,
    mut element: impl FnMut(Value) -> Result<T, PayloadRejection>,
) -> Result<Vec<T>, PayloadRejection> {
    let Value::Array(items) = value else {
        return Err(PayloadRejection::WrongCarrier { key });
    };
    let mut previous: Option<Vec<u8>> = None;
    let mut decoded = Vec::with_capacity(items.len());
    for item in items {
        let bytes = encode(&item, ceilings).map_err(PayloadRejection::Codec)?;
        if previous.as_ref().is_some_and(|before| *before >= bytes) {
            return Err(PayloadRejection::NotCanonical { key });
        }
        previous = Some(bytes);
        decoded.push(element(item)?);
    }
    Ok(decoded)
}

pub(crate) fn decode_canonical_value_set<T>(
    value: Value,
    key: &'static str,
    element: impl FnMut(Value) -> Result<T, PayloadRejection>,
) -> Result<Vec<T>, PayloadRejection> {
    decode_canonical_set(value, key, Ceilings::UNBOUNDED, element)
}

pub(crate) fn optional(fields: &mut Vec<(String, Value)>, key: &'static str) -> Option<Value> {
    fields
        .iter()
        .position(|(name, _)| name == key)
        .map(|index| fields.remove(index).1)
}
