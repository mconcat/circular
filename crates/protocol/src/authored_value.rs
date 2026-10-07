
use circular_core::{Boundary, CANONICAL_VALUE_TAG, Ceilings, ObjectValue, Value, decode, encode};
use std::collections::BTreeSet;
use std::fmt;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Name(Box<str>);

impl Name {
    #[must_use]
    pub fn from_normalized(value: impl Into<Box<str>>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ConfigRecord(Box<[(Name, ConfigValue)]>);

impl ConfigRecord {
    pub fn try_new(entries: Vec<(Name, ConfigValue)>) -> Result<Self, ConfigError> {
        let mut names = BTreeSet::new();
        for (name, _) in &entries {
            if !names.insert(name.clone()) {
                return Err(ConfigError::DuplicateName(name.clone()));
            }
        }
        Ok(Self(entries.into_boxed_slice()))
    }

    #[must_use]
    pub fn entries(&self) -> &[(Name, ConfigValue)] {
        &self.0
    }

    #[must_use]
    pub fn empty() -> Self {
        Self(Box::new([]))
    }

    pub fn from_wire_object(object: &ObjectValue) -> Result<Self, ConfigValueError> {
        let mut entries = Vec::new();
        for (key, item) in object.clone().into_map() {
            entries.push((
                Name::from_normalized(key.clone()),
                ConfigValue::from_wire_value(&item)?,
            ));
        }
        Self::try_new(entries).map_err(ConfigValueError::Record)
    }

    pub fn to_wire_value(&self) -> Result<Value, ConfigValueError> {
        Value::object(
            self.entries()
                .iter()
                .map(|(name, value)| Ok((name.as_str().to_owned(), value.to_wire_value()?)))
                .collect::<Result<Vec<_>, ConfigValueError>>()?,
        )
        .map_err(|_| ConfigValueError::RecordDoesNotEncode)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum ConfigValue {
    Scalar { tag: Name, bytes: Box<[u8]> },
    List(Box<[ConfigValue]>),
    Record(ConfigRecord),
}

impl ConfigValue {
    pub fn from_wire_value(value: &Value) -> Result<Self, ConfigValueError> {
        match value {
            Value::Object(object) => ConfigRecord::from_wire_object(object).map(Self::Record),
            Value::Array(items) => {
                let mut list = Vec::with_capacity(items.len());
                for item in items {
                    list.push(Self::from_wire_value(item)?);
                }
                Ok(Self::List(list.into_boxed_slice()))
            }
            scalar => Ok(Self::Scalar {
                tag: Name::from_normalized(CANONICAL_VALUE_TAG),
                bytes: encode(scalar, Ceilings::for_boundary(Boundary::Config))
                    .map_err(|_| ConfigValueError::ScalarDoesNotEncode)?
                    .into_boxed_slice(),
            }),
        }
    }

    pub fn to_wire_value(&self) -> Result<Value, ConfigValueError> {
        match self {
            Self::Scalar { tag, bytes } => {
                if tag.as_str() != CANONICAL_VALUE_TAG {
                    return Err(ConfigValueError::UnknownScalarTag(tag.clone()));
                }
                decode(bytes, Ceilings::for_boundary(Boundary::Config))
                    .map_err(|_| ConfigValueError::ScalarDoesNotDecode)
            }
            Self::List(values) => values
                .iter()
                .map(Self::to_wire_value)
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            Self::Record(record) => record.to_wire_value(),
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Config(ConfigRecord);

impl Config {
    pub fn try_new(entries: Vec<(Name, ConfigValue)>) -> Result<Self, ConfigError> {
        ConfigRecord::try_new(entries).map(Self)
    }

    #[must_use]
    pub const fn record(&self) -> &ConfigRecord {
        &self.0
    }

    pub fn from_wire_value(value: &Value) -> Result<Self, ConfigValueError> {
        match value {
            Value::Object(object) => ConfigRecord::from_wire_object(object).map(Self),
            Value::Null => Ok(Self::default()),
            other => Err(ConfigValueError::NotAnObject {
                kind: kind_of(other),
            }),
        }
    }

    pub fn to_wire_value(&self) -> Result<Value, ConfigValueError> {
        self.0.to_wire_value()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self(ConfigRecord::empty())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigError {
    DuplicateName(Name),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateName(name) => write!(formatter, "duplicate config record name: {name}"),
        }
    }
}

impl std::error::Error for ConfigError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigValueError {
    NotAnObject { kind: &'static str },
    Record(ConfigError),
    ScalarDoesNotEncode,
    ScalarDoesNotDecode,
    RecordDoesNotEncode,
    UnknownScalarTag(Name),
}

impl fmt::Display for ConfigValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnObject { kind } => write!(
                formatter,
                "config must be an object or absent; received: {kind}"
            ),
            Self::Record(error) => write!(formatter, "invalid config: {error:?}"),
            Self::ScalarDoesNotEncode => formatter.write_str("cannot encode config value"),
            Self::ScalarDoesNotDecode => formatter.write_str("plan config scalar does not decode"),
            Self::RecordDoesNotEncode => formatter.write_str("plan config record does not encode"),
            Self::UnknownScalarTag(tag) => {
                write!(formatter, "plan config scalar has unknown tag {tag:?}")
            }
        }
    }
}

impl std::error::Error for ConfigValueError {}

const fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::String(_) => "string",
        Value::Bytes(_) => "bytes",
        Value::Array(_) => "array",
        _ => "unknown kind",
    }
}

#[cfg(test)]
mod authored_value_tests {
    use super::*;

    #[test]
    fn an_open_value_enters_the_record_only_through_the_constructor() {
        let value = Value::object([
            ("transform", Value::string("event + 1")),
            ("at", Value::Array(vec![Value::string("rows")])),
        ])
        .expect("two keys");
        let config = Config::from_wire_value(&value).expect("an object becomes a record");
        assert_eq!(config.record().entries().len(), 2);
        assert_eq!(
            config.to_wire_value().expect("it goes back"),
            value,
            "the round trip must give the same value"
        );
    }

    #[test]
    fn absence_is_the_empty_record_and_a_scalar_is_refused() {
        assert_eq!(
            Config::from_wire_value(&Value::Null).expect("absence is an empty config"),
            Config::default()
        );
        assert_eq!(
            Config::from_wire_value(&Value::Int(17)),
            Err(ConfigValueError::NotAnObject { kind: "int" })
        );
    }
}
