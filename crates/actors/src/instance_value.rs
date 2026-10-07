
use circular_core::Value;
use circular_runtime::{InstanceKey, InstanceScalar};

pub const INSTANCE_KEY_KINDS: [&str; 4] = ["text", "int", "bool", "tuple"];

pub const KIND: &str = "kind";
pub const VALUE: &str = "value";

#[must_use]
pub fn instance_key_to_value(key: &InstanceKey) -> Value {
    match key {
        InstanceKey::Scalar(scalar) => scalar_to_value(scalar),
        InstanceKey::Tuple(items) => tagged(
            "tuple",
            Value::Array(items.iter().map(scalar_to_value).collect()),
        ),
    }
}

fn scalar_to_value(scalar: &InstanceScalar) -> Value {
    match scalar {
        InstanceScalar::Text(text) => tagged("text", Value::string(text.as_ref())),
        InstanceScalar::Int(value) => tagged("int", Value::int(*value)),
        InstanceScalar::Bool(value) => tagged("bool", Value::Bool(*value)),
    }
}

fn tagged(kind: &str, value: Value) -> Value {
    Value::object([
        (KIND.to_owned(), Value::string(kind)),
        (VALUE.to_owned(), value),
    ])
    .expect("kind and value are distinct names")
}

#[must_use]
pub fn instance_key_from_payload_scalar(value: &Value) -> Option<InstanceKey> {
    let scalar = match value {
        Value::String(text) => InstanceScalar::Text(text.as_str().into()),
        Value::Int(number) => InstanceScalar::Int(*number),
        Value::Bool(flag) => InstanceScalar::Bool(*flag),
        _ => return None,
    };
    Some(InstanceKey::Scalar(scalar))
}

pub fn instance_key_from_value(value: &Value) -> Result<InstanceKey, InstanceKeyValueError> {
    let (kind, payload) = split(value)?;
    if kind == "tuple" {
        let Some(items) = payload.as_array() else {
            return Err(InstanceKeyValueError::TupleIsNotAnArray);
        };
        let items = items
            .iter()
            .map(scalar_from_value)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(InstanceKey::Tuple(items.into_boxed_slice()))
    } else {
        scalar_from_value(value).map(InstanceKey::Scalar)
    }
}

fn scalar_from_value(value: &Value) -> Result<InstanceScalar, InstanceKeyValueError> {
    let (kind, payload) = split(value)?;
    match kind {
        "text" => payload
            .as_str()
            .map(|text| InstanceScalar::Text(text.into()))
            .ok_or(InstanceKeyValueError::PayloadDoesNotMatchKind { kind: "text" }),
        "int" => payload
            .as_int()
            .map(InstanceScalar::Int)
            .ok_or(InstanceKeyValueError::PayloadDoesNotMatchKind { kind: "int" }),
        "bool" => match payload {
            Value::Bool(flag) => Ok(InstanceScalar::Bool(*flag)),
            _ => Err(InstanceKeyValueError::PayloadDoesNotMatchKind { kind: "bool" }),
        },
        "tuple" => Err(InstanceKeyValueError::NestedTuple),
        _ => Err(InstanceKeyValueError::UnknownKind),
    }
}

fn split(value: &Value) -> Result<(&str, &Value), InstanceKeyValueError> {
    let Some(object) = value.as_object() else {
        return Err(InstanceKeyValueError::NotAnObject);
    };
    let kind = object
        .get(KIND)
        .and_then(Value::as_str)
        .ok_or(InstanceKeyValueError::MissingKind)?;
    let payload = object
        .get(VALUE)
        .ok_or(InstanceKeyValueError::MissingValue)?;
    Ok((kind, payload))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstanceKeyValueError {
    NotAnObject,
    MissingKind,
    MissingValue,
    UnknownKind,
    PayloadDoesNotMatchKind {
        kind: &'static str,
    },
    TupleIsNotAnArray,
    NestedTuple,
}

impl core::fmt::Display for InstanceKeyValueError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotAnObject => formatter.write_str("instance key is not an object"),
            Self::MissingKind => formatter.write_str("instance key is missing kind"),
            Self::MissingValue => formatter.write_str("instance key is missing value"),
            Self::UnknownKind => {
                formatter.write_str("instance key kind is not one of the four supported variants")
            }
            Self::PayloadDoesNotMatchKind { kind } => {
                write!(
                    formatter,
                    "instance key value does not match the {kind} variant"
                )
            }
            Self::TupleIsNotAnArray => formatter.write_str("tuple variant value is not an array"),
            Self::NestedTuple => formatter.write_str("nested tuples are not instance keys"),
        }
    }
}

impl std::error::Error for InstanceKeyValueError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_arm() -> Vec<InstanceKey> {
        vec![
            InstanceKey::Scalar(InstanceScalar::Text("".into())),
            InstanceKey::Scalar(InstanceScalar::Text("sample-001".into())),
            InstanceKey::Scalar(InstanceScalar::Int(i64::MIN)),
            InstanceKey::Scalar(InstanceScalar::Int(i64::MAX)),
            InstanceKey::Scalar(InstanceScalar::Bool(false)),
            InstanceKey::Scalar(InstanceScalar::Bool(true)),
            InstanceKey::Tuple(Box::new([])),
            InstanceKey::Tuple(Box::new([
                InstanceScalar::Text("claude".into()),
                InstanceScalar::Int(-7),
                InstanceScalar::Bool(true),
            ])),
        ]
    }

    #[test]
    fn every_arm_round_trips_through_the_surface_value() {
        for key in every_arm() {
            let value = instance_key_to_value(&key);
            assert_eq!(instance_key_from_value(&value), Ok(key.clone()), "{key:?}");
        }
    }

    #[test]
    fn the_kind_names_are_the_declared_four() {
        for key in every_arm() {
            let value = instance_key_to_value(&key);
            let kind = value
                .as_object()
                .and_then(|object| object.get(KIND))
                .and_then(Value::as_str)
                .expect("every arm carries kind");
            assert!(INSTANCE_KEY_KINDS.contains(&kind), "{kind}");
        }
    }

    #[test]
    fn every_rejection_is_its_own_arm() {
        let object = |kind: &str, value: Value| {
            Value::object([
                (KIND.to_owned(), Value::string(kind)),
                (VALUE.to_owned(), value),
            ])
            .unwrap()
        };

        assert_eq!(
            instance_key_from_value(&Value::int(1)),
            Err(InstanceKeyValueError::NotAnObject)
        );
        assert_eq!(
            instance_key_from_value(&Value::object([(VALUE.to_owned(), Value::int(1))]).unwrap()),
            Err(InstanceKeyValueError::MissingKind)
        );
        assert_eq!(
            instance_key_from_value(
                &Value::object([(KIND.to_owned(), Value::string("int"))]).unwrap()
            ),
            Err(InstanceKeyValueError::MissingValue)
        );
        assert_eq!(
            instance_key_from_value(&object("float", Value::float(1.0))),
            Err(InstanceKeyValueError::UnknownKind)
        );
        assert_eq!(
            instance_key_from_value(&object("tuple", Value::int(1))),
            Err(InstanceKeyValueError::TupleIsNotAnArray)
        );
        assert_eq!(
            instance_key_from_value(&object(
                "tuple",
                Value::Array(vec![object("tuple", Value::Array(Vec::new()))])
            )),
            Err(InstanceKeyValueError::NestedTuple)
        );
    }
}
