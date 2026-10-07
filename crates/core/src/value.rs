use crate::event::BaseShape;
use std::collections::{BTreeMap, btree_map::Entry};
use std::error::Error;
use std::fmt;
use std::num::NonZeroUsize;

crate::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum ValueKind: u8 {
        Null = 1 => "null",
        Bool = 2 => "boolean",
        Int = 3 => "int",
        Float = 4 => "double",
        String = 5 => "string",
        Bytes = 6 => "bytes",
        Array = 7 => "array",
        Object = 8 => "object",
        UInt = 9 => "uint",
    }
}

impl ValueKind {
    #[must_use]
    pub const fn is_scalar(self) -> bool {
        !matches!(self, Self::Array | Self::Object)
    }

    #[must_use]
    pub const fn base_shape(self) -> Option<BaseShape> {
        match self {
            Self::Null => Some(BaseShape::Null),
            Self::Bool => Some(BaseShape::Bool),
            Self::Int => Some(BaseShape::Int),
            Self::Float => Some(BaseShape::Float),
            Self::String => Some(BaseShape::String),
            Self::Bytes => Some(BaseShape::Bytes),
            Self::UInt => Some(BaseShape::UInt),
            Self::Array | Self::Object => None,
        }
    }
}

impl From<BaseShape> for ValueKind {
    fn from(shape: BaseShape) -> Self {
        match shape {
            BaseShape::Null => Self::Null,
            BaseShape::Bool => Self::Bool,
            BaseShape::Int => Self::Int,
            BaseShape::Float => Self::Float,
            BaseShape::String => Self::String,
            BaseShape::Bytes => Self::Bytes,
            BaseShape::UInt => Self::UInt,
        }
    }
}

const CANONICAL_NAN_BITS: u64 = 0x7ff8_0000_0000_0000;

#[derive(Clone, Copy, Debug)]
pub struct FloatValue(f64);

impl FloatValue {
    #[must_use]
    pub fn new(value: f64) -> Self {
        if value.is_nan() {
            Self(f64::from_bits(CANONICAL_NAN_BITS))
        } else {
            Self(value)
        }
    }

    #[must_use]
    pub const fn get(self) -> f64 {
        self.0
    }

    #[must_use]
    pub const fn to_bits(self) -> u64 {
        self.0.to_bits()
    }
}

impl From<f64> for FloatValue {
    fn from(value: f64) -> Self {
        Self::new(value)
    }
}

impl PartialEq for FloatValue {
    fn eq(&self, other: &Self) -> bool {
        self.to_bits() == other.to_bits()
    }
}

impl Eq for FloatValue {}

impl fmt::Display for FloatValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, formatter)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectValue {
    entries: BTreeMap<String, Value>,
}

impl ObjectValue {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    #[must_use]
    pub const fn from_map(entries: BTreeMap<String, Value>) -> Self {
        Self { entries }
    }

    pub fn try_from_entries<I, K>(entries: I) -> Result<Self, DuplicateKeyError>
    where
        I: IntoIterator<Item = (K, Value)>,
        K: Into<String>,
    {
        let mut object = BTreeMap::new();
        for (key, value) in entries {
            let key = key.into();
            match object.entry(key) {
                Entry::Vacant(entry) => {
                    entry.insert(value);
                }
                Entry::Occupied(entry) => {
                    return Err(DuplicateKeyError {
                        key: entry.key().clone(),
                    });
                }
            }
        }
        Ok(Self { entries: object })
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.get(key)
    }

    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &Value)> {
        self.entries
            .iter()
            .map(|(key, value)| (key.as_str(), value))
    }

    pub fn keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    #[must_use]
    pub const fn as_map(&self) -> &BTreeMap<String, Value> {
        &self.entries
    }

    #[must_use]
    pub fn into_map(self) -> BTreeMap<String, Value> {
        self.entries
    }
}

impl TryFrom<Vec<(String, Value)>> for ObjectValue {
    type Error = DuplicateKeyError;

    fn try_from(entries: Vec<(String, Value)>) -> Result<Self, Self::Error> {
        Self::try_from_entries(entries)
    }
}

impl From<BTreeMap<String, Value>> for ObjectValue {
    fn from(entries: BTreeMap<String, Value>) -> Self {
        Self::from_map(entries)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DuplicateKeyError {
    key: String,
}

impl DuplicateKeyError {
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub fn into_key(self) -> String {
        self.key
    }
}

impl fmt::Display for DuplicateKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "duplicate object key {:?}", self.key)
    }
}

impl Error for DuplicateKeyError {}

#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(FloatValue),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<Value>),
    UInt(u64),
    Object(ObjectValue),
}

impl Value {
    #[must_use]
    pub const fn kind(&self) -> ValueKind {
        match self {
            Self::Null => ValueKind::Null,
            Self::Bool(_) => ValueKind::Bool,
            Self::Int(_) => ValueKind::Int,
            Self::UInt(_) => ValueKind::UInt,
            Self::Float(_) => ValueKind::Float,
            Self::String(_) => ValueKind::String,
            Self::Bytes(_) => ValueKind::Bytes,
            Self::Array(_) => ValueKind::Array,
            Self::Object(_) => ValueKind::Object,
        }
    }

    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        self.kind().as_str()
    }

    #[must_use]
    pub const fn bool(value: bool) -> Self {
        Self::Bool(value)
    }

    #[must_use]
    pub const fn int(value: i64) -> Self {
        Self::Int(value)
    }

    #[must_use]
    pub fn float(value: f64) -> Self {
        Self::Float(FloatValue::new(value))
    }

    #[must_use]
    pub const fn uint(value: u64) -> Self {
        Self::UInt(value)
    }

    #[must_use]
    pub fn string(value: impl Into<String>) -> Self {
        Self::String(value.into())
    }

    #[must_use]
    pub fn bytes(value: impl Into<Vec<u8>>) -> Self {
        Self::Bytes(value.into())
    }

    #[must_use]
    pub fn array(values: impl IntoIterator<Item = Value>) -> Self {
        Self::Array(values.into_iter().collect())
    }

    pub fn object<I, K>(entries: I) -> Result<Self, DuplicateKeyError>
    where
        I: IntoIterator<Item = (K, Value)>,
        K: Into<String>,
    {
        ObjectValue::try_from_entries(entries).map(Self::Object)
    }

    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(value) => Some(*value),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(value) => Some(value.get()),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(value) => Some(value),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(values) => Some(values),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_object(&self) -> Option<&ObjectValue> {
        match self {
            Self::Object(value) => Some(value),
            _ => None,
        }
    }

    pub fn strict_eq_with_budget(
        &self,
        other: &Self,
        budget: &mut EvaluationBudget,
    ) -> Result<bool, BudgetExhausted> {
        strict_eq(self, other, Some(budget))
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        strict_eq(self, other, None).expect("unbounded structural equality cannot exhaust")
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct EvaluationBudget {
    limit: NonZeroUsize,
    remaining: usize,
}

impl EvaluationBudget {
    #[must_use]
    pub const fn new(limit: NonZeroUsize) -> Self {
        Self {
            limit,
            remaining: limit.get(),
        }
    }

    #[must_use]
    pub const fn limit(&self) -> NonZeroUsize {
        self.limit
    }

    #[must_use]
    pub const fn consumed(&self) -> usize {
        self.limit.get() - self.remaining
    }

    #[must_use]
    pub const fn remaining(&self) -> usize {
        self.remaining
    }

    pub fn enter(&mut self) -> Result<(), BudgetExhausted> {
        if self.remaining == 0 {
            return Err(BudgetExhausted {
                limit: self.limit,
                consumed: self.consumed(),
            });
        }
        self.remaining -= 1;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BudgetExhausted {
    limit: NonZeroUsize,
    consumed: usize,
}

impl BudgetExhausted {
    #[must_use]
    pub const fn limit(self) -> NonZeroUsize {
        self.limit
    }

    #[must_use]
    pub const fn consumed(self) -> usize {
        self.consumed
    }
}

impl fmt::Display for BudgetExhausted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "evaluation exhausted its {}-step budget after {} steps",
            self.limit, self.consumed
        )
    }
}

impl Error for BudgetExhausted {}

enum EqualityFrame<'value> {
    Pair(&'value Value, &'value Value),
    ArrayItems {
        left: &'value [Value],
        right: &'value [Value],
        index: usize,
    },
    ObjectEntries {
        left: std::collections::btree_map::Iter<'value, String, Value>,
        right: std::collections::btree_map::Iter<'value, String, Value>,
    },
}

fn strict_eq(
    left: &Value,
    right: &Value,
    mut budget: Option<&mut EvaluationBudget>,
) -> Result<bool, BudgetExhausted> {
    let mut work = vec![EqualityFrame::Pair(left, right)];
    while let Some(frame) = work.pop() {
        let (left, right) = match frame {
            EqualityFrame::Pair(left, right) => (left, right),
            EqualityFrame::ArrayItems { left, right, index } => {
                let Some((left_item, right_item)) = left.get(index).zip(right.get(index)) else {
                    continue;
                };
                if let Some(budget) = budget.as_deref_mut() {
                    budget.enter()?;
                }
                work.push(EqualityFrame::ArrayItems {
                    left,
                    right,
                    index: index + 1,
                });
                (left_item, right_item)
            }
            EqualityFrame::ObjectEntries {
                mut left,
                mut right,
            } => {
                let Some(((left_key, left_value), (right_key, right_value))) =
                    left.next().zip(right.next())
                else {
                    continue;
                };
                if let Some(budget) = budget.as_deref_mut() {
                    budget.enter()?;
                }
                if left_key != right_key {
                    return Ok(false);
                }
                work.push(EqualityFrame::ObjectEntries { left, right });
                (left_value, right_value)
            }
        };

        match (left, right) {
            (Value::Null, Value::Null) => {}
            (Value::Bool(left), Value::Bool(right)) if left == right => {}
            (Value::Int(left), Value::Int(right)) if left == right => {}
            (Value::UInt(left), Value::UInt(right)) if left == right => {}
            (Value::Float(left), Value::Float(right)) if left == right => {}
            (Value::String(left), Value::String(right)) if left == right => {}
            (Value::Bytes(left), Value::Bytes(right)) if left == right => {}
            (Value::Array(left), Value::Array(right)) if left.len() == right.len() => {
                work.push(EqualityFrame::ArrayItems {
                    left,
                    right,
                    index: 0,
                });
            }
            (Value::Object(left), Value::Object(right)) if left.len() == right.len() => {
                work.push(EqualityFrame::ObjectEntries {
                    left: left.as_map().iter(),
                    right: right.as_map().iter(),
                });
            }
            _ => return Ok(false),
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{BaseShape, CANONICAL_NAN_BITS, EvaluationBudget, ObjectValue, Value, ValueKind};
    use std::num::NonZeroUsize;

    #[test]
    fn nine_value_forms_are_distinct() {
        let values = [
            Value::Null,
            Value::Bool(false),
            Value::Int(0),
            Value::float(0.0),
            Value::String(String::new()),
            Value::Bytes(Vec::new()),
            Value::Array(Vec::new()),
            Value::Object(ObjectValue::new()),
            Value::uint(0),
        ];

        assert_eq!(values.each_ref().map(|value| value.kind()), ValueKind::ALL);
        assert_eq!(
            values.each_ref().map(|value| value.type_name()),
            [
                "null", "boolean", "int", "double", "string", "bytes", "array", "object", "uint",
            ]
        );
    }

    #[test]
    fn a_new_kind_is_equal_to_itself() {
        assert_eq!(Value::uint(7), Value::uint(7));
        assert_ne!(Value::uint(7), Value::uint(8));
        assert_ne!(Value::uint(7), Value::Int(7));
    }

    #[test]
    fn an_integer_is_never_equal_to_a_float() {
        assert_ne!(Value::Int(2), Value::float(2.0));
        assert_ne!(Value::Int(2).kind(), Value::float(2.0).kind());
    }

    #[test]
    fn object_rejects_duplicate_keys_instead_of_overwriting() {
        let error = Value::object([("same", Value::Int(1)), ("same", Value::Int(2))])
            .expect_err("duplicate key must be rejected");

        assert_eq!(error.key(), "same");
    }

    #[test]
    fn strict_equality_uses_an_explicit_stack_and_exact_child_budget() {
        let left = Value::Array(vec![Value::Int(1), Value::String("x".to_owned())]);
        let right = left.clone();
        let mut exact = EvaluationBudget::new(NonZeroUsize::new(2).unwrap());
        assert_eq!(left.strict_eq_with_budget(&right, &mut exact), Ok(true));
        assert_eq!(exact.consumed(), 2);

        let mut short = EvaluationBudget::new(NonZeroUsize::new(1).unwrap());
        let exhausted = left
            .strict_eq_with_budget(&right, &mut short)
            .expect_err("the second array element requires the second step");
        assert_eq!(exhausted.limit().get(), 1);
        assert_eq!(exhausted.consumed(), 1);

        let mut deep_left = Value::Null;
        let mut deep_right = Value::Null;
        for _ in 0..512 {
            deep_left = Value::Array(vec![deep_left]);
            deep_right = Value::Array(vec![deep_right]);
        }
        let mut deep_budget = EvaluationBudget::new(NonZeroUsize::new(512).unwrap());
        assert_eq!(
            deep_left.strict_eq_with_budget(&deep_right, &mut deep_budget),
            Ok(true)
        );
        assert_eq!(deep_budget.consumed(), 512);
    }

    #[test]
    fn object_order_is_canonical_and_not_insertion_order() {
        let left = ObjectValue::try_from_entries([
            ("😀", Value::Null),
            ("é", Value::Null),
            ("z", Value::Null),
        ])
        .expect("unique keys");
        let right = ObjectValue::try_from_entries([
            ("z", Value::Null),
            ("é", Value::Null),
            ("😀", Value::Null),
        ])
        .expect("unique keys");

        assert_eq!(left, right);
        assert_eq!(left.keys().collect::<Vec<_>>(), ["z", "é", "😀"]);
    }

    #[test]
    fn rust_string_counts_unicode_scalars_without_surrogates() {
        let value = Value::string("a😀€");

        assert_eq!(value.as_str().map(|text| text.chars().count()), Some(3));
    }
}
