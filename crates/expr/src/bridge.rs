
use cel::objects::{Key, Map};
use circular_core::{ObjectValue, Value};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LowerError {
    UnmappedType {
        got: &'static str,
    },
    NonStringKey {
        got: &'static str,
    },
    DuplicateKey {
        key: String,
    },
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnmappedType { got } => {
                write!(formatter, "CEL type {got} has no Value counterpart")
            }
            Self::NonStringKey { got } => {
                write!(formatter, "map key is not a string: {got}")
            }
            Self::DuplicateKey { key } => write!(formatter, "duplicate key: {key}"),
        }
    }
}

impl std::error::Error for LowerError {}

#[must_use]
pub fn lift(value: &Value) -> cel::Value {
    match value {
        Value::Null => cel::Value::Null,
        Value::Bool(inner) => cel::Value::Bool(*inner),
        Value::Int(inner) => cel::Value::Int(*inner),
        Value::UInt(inner) => cel::Value::UInt(*inner),
        Value::Float(inner) => cel::Value::Float(inner.get()),
        Value::String(inner) => cel::Value::String(Arc::new(inner.clone())),
        Value::Bytes(inner) => cel::Value::Bytes(Arc::new(inner.clone())),
        Value::Array(inner) => cel::Value::List(Arc::new(inner.iter().map(lift).collect())),
        Value::Object(inner) => {
            let entries: HashMap<Key, cel::Value> = inner
                .iter()
                .map(|(key, item)| (Key::String(Arc::new(key.to_owned())), lift(item)))
                .collect();
            cel::Value::Map(Map {
                map: Arc::new(entries),
            })
        }
    }
}

pub fn lower(value: &cel::Value) -> Result<Value, LowerError> {
    Ok(match value {
        cel::Value::Null => Value::Null,
        cel::Value::Bool(inner) => Value::bool(*inner),
        cel::Value::Int(inner) => Value::int(*inner),
        cel::Value::UInt(inner) => Value::uint(*inner),
        cel::Value::Float(inner) => Value::float(*inner),
        cel::Value::String(inner) => Value::string(inner.as_str()),
        cel::Value::Bytes(inner) => Value::bytes(inner.as_slice()),
        cel::Value::List(inner) => Value::array(
            inner
                .iter()
                .map(lower)
                .collect::<Result<Vec<_>, LowerError>>()?,
        ),
        cel::Value::Map(inner) => {
            let mut entries = Vec::with_capacity(inner.map.len());
            for (key, item) in inner.map.iter() {
                let Key::String(key) = key else {
                    return Err(LowerError::NonStringKey { got: key_kind(key) });
                };
                entries.push((key.as_str().to_owned(), lower(item)?));
            }
            let entries = ObjectValue::try_from_entries(entries).map_err(|error| {
                LowerError::DuplicateKey {
                    key: error.into_key(),
                }
            })?;
            Value::Object(entries)
        }
        other => {
            return Err(LowerError::UnmappedType {
                got: unmapped_name(other),
            });
        }
    })
}

const fn key_kind(key: &Key) -> &'static str {
    match key {
        Key::Int(_) => "int",
        Key::Uint(_) => "uint",
        Key::Bool(_) => "bool",
        Key::String(_) => "string",
    }
}

fn unmapped_name(value: &cel::Value) -> &'static str {
    match value {
        cel::Value::Function(..) => "function",
        cel::Value::Opaque(_) => "opaque",
        _ => "unsupported CEL type",
    }
}

#[cfg(test)]
mod tests {
    use super::{LowerError, lift, lower};
    use cel::objects::{Key, Map};
    use circular_core::{ObjectValue, Value};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn sample_values() -> Vec<Value> {
        vec![
            Value::Null,
            Value::bool(true),
            Value::bool(false),
            Value::int(0),
            Value::int(-1),
            Value::int(i64::MIN),
            Value::int(i64::MAX),
            Value::int(9_007_199_254_740_993),
            Value::float(0.0),
            Value::float(-0.0),
            Value::float(f64::INFINITY),
            Value::float(f64::NEG_INFINITY),
            Value::float(f64::NAN),
            Value::float(1.5),
            Value::string(""),
            Value::string("café · emoji 😀 · \0"),
            Value::bytes(Vec::new()),
            Value::bytes(vec![0, 255, 128]),
            Value::array([]),
            Value::array([
                Value::int(1),
                Value::Null,
                Value::array([Value::bool(true)]),
            ]),
            Value::Object(ObjectValue::new()),
            Value::object([
                ("a", Value::int(1)),
                ("b", Value::array([Value::string("x")])),
                ("", Value::Null),
            ])
            .unwrap(),
        ]
    }

    #[test]
    fn lifting_never_fails() {
        for value in sample_values() {
            let _: cel::Value = lift(&value);
        }
    }

    #[test]
    fn negative_zero_is_not_folded_into_zero() {
        let negative = lower(&lift(&Value::float(-0.0))).expect("total function");
        let positive = lower(&lift(&Value::float(0.0))).expect("total function");

        assert_ne!(negative, positive);
        assert_eq!(
            negative.as_float().map(f64::to_bits),
            Some((-0.0f64).to_bits())
        );
    }

    #[test]
    fn nan_stays_canonical_through_the_round_trip() {
        let lifted = lift(&Value::float(f64::NAN));
        let round = lower(&lifted).expect("total function");

        assert_eq!(round, Value::float(f64::NAN));
        assert_eq!(
            round,
            Value::float(f64::from_bits(0x7ff8_0000_0000_0001)),
            "a different NaN bit pattern folds to the same canonical form"
        );
    }

    #[test]
    fn absence_and_explicit_null_stay_apart() {
        let absent = Value::Object(ObjectValue::new());
        let explicit = Value::object([("k", Value::Null)]).unwrap();

        assert_ne!(absent, explicit);

        let absent = lower(&lift(&absent)).expect("total function");
        let explicit = lower(&lift(&explicit)).expect("total function");
        assert_ne!(absent, explicit);

        assert_eq!(absent.as_object().map(ObjectValue::len), Some(0));
        assert_eq!(explicit.as_object().map(ObjectValue::len), Some(1));
    }

    #[test]
    fn types_without_an_image_are_rejected_with_a_reason() {
        assert_eq!(
            lower(&cel::Value::Function(Arc::new("f".to_owned()), None)),
            Err(LowerError::UnmappedType { got: "function" })
        );
    }

    #[test]
    fn a_uint_survives_the_round_trip_above_the_int_ceiling() {
        for raw in [0u64, 1, i64::MAX as u64 + 1, u64::MAX] {
            let value = Value::uint(raw);
            assert_eq!(lower(&lift(&value)), Ok(value.clone()), "{raw}");
        }
        assert_ne!(lower(&lift(&Value::uint(7))), Ok(Value::int(7)));
    }

    #[test]
    fn non_string_map_keys_are_rejected_with_a_reason() {
        let mut entries = HashMap::new();
        entries.insert(Key::Int(1), cel::Value::Bool(true));
        let map = cel::Value::Map(Map {
            map: Arc::new(entries),
        });

        assert_eq!(lower(&map), Err(LowerError::NonStringKey { got: "int" }));
    }

    #[test]
    fn rejection_reaches_inside_containers() {
        let nested = cel::Value::List(Arc::new(vec![
            cel::Value::Int(1),
            cel::Value::List(Arc::new(vec![cel::Value::Function(
                Arc::new("f".to_owned()),
                None,
            )])),
        ]));

        assert_eq!(
            lower(&nested),
            Err(LowerError::UnmappedType { got: "function" })
        );
    }
}
