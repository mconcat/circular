
use crate::value::{ObjectValue, Value, ValueKind};
use std::collections::BTreeSet;
use std::fmt;

#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FieldPath(Vec<String>);

impl FieldPath {
    #[must_use]
    pub const fn root() -> Self {
        Self(Vec::new())
    }

    #[must_use]
    pub fn key(&self, key: impl Into<String>) -> Self {
        let mut segments = self.0.clone();
        segments.push(key.into());
        Self(segments)
    }

    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.0
    }
}

impl fmt::Display for FieldPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0.join("."))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotObject {
    pub at: FieldPath,
    pub actual: ValueKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownField {
    pub at: FieldPath,
    pub key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FieldRejectionKind {
    NotObject { actual: ValueKind },
    Missing,
    Unknown,
    Kind {
        expected: ValueKind,
        actual: ValueKind,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldRejection {
    pub at: FieldPath,
    pub kind: FieldRejectionKind,
}

impl From<NotObject> for FieldRejection {
    fn from(rejection: NotObject) -> Self {
        Self {
            at: rejection.at,
            kind: FieldRejectionKind::NotObject {
                actual: rejection.actual,
            },
        }
    }
}

impl From<UnknownField> for FieldRejection {
    fn from(rejection: UnknownField) -> Self {
        Self {
            at: rejection.at.key(rejection.key),
            kind: FieldRejectionKind::Unknown,
        }
    }
}

impl fmt::Display for FieldRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            FieldRejectionKind::NotObject { actual } => {
                write!(formatter, "{} must be an object, got {actual}", self.at)
            }
            FieldRejectionKind::Missing => write!(formatter, "{} is missing", self.at),
            FieldRejectionKind::Unknown => write!(formatter, "{} is not a known key", self.at),
            FieldRejectionKind::Kind { expected, actual } => {
                write!(formatter, "{} must be {expected}, got {actual}", self.at)
            }
        }
    }
}

impl std::error::Error for FieldRejection {}

pub trait FromValue: Sized {
    fn from_value(value: &Value, at: &FieldPath) -> Result<Self, FieldRejection>;
}

fn kind(at: &FieldPath, expected: ValueKind, actual: &Value) -> FieldRejection {
    FieldRejection {
        at: at.clone(),
        kind: FieldRejectionKind::Kind {
            expected,
            actual: actual.kind(),
        },
    }
}

impl FromValue for Value {
    fn from_value(value: &Value, _at: &FieldPath) -> Result<Self, FieldRejection> {
        Ok(value.clone())
    }
}

impl FromValue for String {
    fn from_value(value: &Value, at: &FieldPath) -> Result<Self, FieldRejection> {
        value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| kind(at, ValueKind::String, value))
    }
}

impl FromValue for bool {
    fn from_value(value: &Value, at: &FieldPath) -> Result<Self, FieldRejection> {
        match value {
            Value::Bool(value) => Ok(*value),
            _ => Err(kind(at, ValueKind::Bool, value)),
        }
    }
}

#[derive(Debug)]
pub struct Fields<'v> {
    object: &'v ObjectValue,
    at: FieldPath,
    known: BTreeSet<&'v str>,
}

impl<'v> Fields<'v> {
    pub fn open(value: &'v Value) -> Result<Self, NotObject> {
        Self::open_at(value, FieldPath::root())
    }

    pub fn open_at(value: &'v Value, at: FieldPath) -> Result<Self, NotObject> {
        match value {
            Value::Object(object) => Ok(Self::object_at(object, at)),
            other => Err(NotObject {
                at,
                actual: other.kind(),
            }),
        }
    }

    #[must_use]
    pub fn object_at(object: &'v ObjectValue, at: FieldPath) -> Self {
        Self {
            object,
            at,
            known: BTreeSet::new(),
        }
    }

    #[must_use]
    pub const fn at(&self) -> &FieldPath {
        &self.at
    }

    pub fn take(&mut self, key: &str) -> Option<&'v Value> {
        let (key, value) = self.object.as_map().get_key_value(key)?;
        self.known.insert(key.as_str());
        Some(value)
    }

    pub fn skip(&mut self, key: &str) {
        let _ = self.take(key);
    }

    pub fn required<T: FromValue>(&mut self, key: &str) -> Result<T, FieldRejection> {
        let at = self.at.key(key);
        match self.take(key) {
            Some(value) => T::from_value(value, &at),
            None => Err(FieldRejection {
                at,
                kind: FieldRejectionKind::Missing,
            }),
        }
    }

    pub fn optional<T: FromValue>(&mut self, key: &str) -> Result<Option<T>, FieldRejection> {
        let at = self.at.key(key);
        self.take(key)
            .map(|value| T::from_value(value, &at))
            .transpose()
    }

    #[must_use]
    pub fn unknown(&self) -> Option<UnknownField> {
        self.object
            .keys()
            .find(|key| !self.known.contains(key))
            .map(|key| UnknownField {
                at: self.at.clone(),
                key: key.to_owned(),
            })
    }

    pub fn finish(self) -> Result<(), UnknownField> {
        self.unknown().map_or(Ok(()), Err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object(entries: &[(&str, Value)]) -> Value {
        Value::object(entries.iter().cloned()).unwrap()
    }

    fn at(keys: &[&str]) -> FieldPath {
        keys.iter()
            .fold(FieldPath::root(), |path, key| path.key(*key))
    }

    #[test]
    fn one_reader_answers_not_object_missing_kind_and_unknown_each_at_its_place() {
        assert_eq!(
            Fields::open(&Value::Int(3)).unwrap_err(),
            NotObject {
                at: FieldPath::root(),
                actual: ValueKind::Int,
            }
        );

        let value = object(&[
            ("name", Value::string("orders")),
            ("count", Value::Int(2)),
            ("extra", Value::Bool(true)),
            ("zeta", Value::Null),
        ]);
        let mut fields = Fields::open(&value).unwrap();
        assert_eq!(fields.required::<String>("name").unwrap(), "orders");
        assert_eq!(
            fields.required::<String>("count").unwrap_err(),
            FieldRejection {
                at: at(&["count"]),
                kind: FieldRejectionKind::Kind {
                    expected: ValueKind::String,
                    actual: ValueKind::Int,
                },
            }
        );
        assert_eq!(
            fields.required::<bool>("absent").unwrap_err(),
            FieldRejection {
                at: at(&["absent"]),
                kind: FieldRejectionKind::Missing,
            }
        );
        assert_eq!(fields.optional::<bool>("also_absent").unwrap(), None);
        fields.skip("zeta");
        assert_eq!(
            fields.finish().unwrap_err(),
            UnknownField {
                at: FieldPath::root(),
                key: "extra".to_owned(),
            }
        );
    }

    #[test]
    fn a_nested_reader_names_the_whole_path() {
        let inner = object(&[("mode", Value::Int(1))]);
        let fields = Fields::open_at(&inner, at(&["secrets", "custody"])).unwrap();
        assert_eq!(
            FieldRejection::from(fields.finish().unwrap_err()),
            FieldRejection {
                at: at(&["secrets", "custody", "mode"]),
                kind: FieldRejectionKind::Unknown,
            }
        );
        assert_eq!(
            at(&["secrets", "custody", "mode"]).to_string(),
            "secrets.custody.mode"
        );
    }
}
