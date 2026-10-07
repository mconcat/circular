//! Closed, lossless port Flow/Shape carrier for read-side projections.
//!
//! Runtime and actor-registration crates keep their own strongly typed names.
//! This module is the single physical `Value` codec shared by daemon producers
//! and app consumers, so neither side maintains a parallel tag table.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use circular_core::{BaseShape, NonZeroTicks, Shape, Value};

use crate::wire_value::{WireError, exhausted, object_from_value, take};

pub const PORT_SHAPE_MAX_DEPTH: usize = 16;
/// Canonical object-shape width ceiling.
pub const PORT_OBJECT_MAX_FIELDS: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortShape {
    Any,
    Base(BaseShape),
    Array(Box<Self>),
    Object {
        fields: Vec<PortShapeField>,
        open: bool,
    },
    Variable(String),
}

impl PortShape {
    /// Lossless projection from the core shape vocabulary once its name type
    /// has already crossed into owned strings.
    #[must_use]
    pub fn from_core(shape: &Shape<String>) -> Self {
        match shape {
            Shape::Any => Self::Any,
            Shape::Base(base) => Self::Base(*base),
            Shape::Array(item) => Self::Array(Box::new(Self::from_core(item))),
            Shape::Object { fields, open } => Self::Object {
                fields: fields
                    .as_slice()
                    .iter()
                    .map(|(name, shape)| PortShapeField {
                        name: name.clone(),
                        shape: Self::from_core(shape),
                    })
                    .collect(),
                open: *open,
            },
            Shape::Var(name) => Self::Variable(name.clone()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortShapeField {
    pub name: String,
    pub shape: PortShape,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortFlow {
    Stream(PortShape),
    Signal { item: PortShape, rate: PortRate },
}

impl PortFlow {
    #[must_use]
    pub const fn item(&self) -> &PortShape {
        match self {
            Self::Stream(item) | Self::Signal { item, .. } => item,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortRate {
    Period(NonZeroTicks),
    Variable(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortTypeCodecError {
    pub detail: String,
}

impl fmt::Display for PortTypeCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl Error for PortTypeCodecError {}

pub fn encode_port_shape(shape: &PortShape) -> Result<Value, PortTypeCodecError> {
    validate_port_shape(shape, 0)?;
    encode_port_shape_unchecked(shape)
}

fn encode_port_shape_unchecked(shape: &PortShape) -> Result<Value, PortTypeCodecError> {
    match shape {
        PortShape::Any => Ok(Value::array([Value::Int(1)])),
        PortShape::Base(base) => Ok(Value::array([
            Value::Int(2),
            Value::String(base.as_str().to_owned()),
        ])),
        PortShape::Array(item) => Ok(Value::array([
            Value::Int(3),
            encode_port_shape_unchecked(item)?,
        ])),
        PortShape::Object { fields, open } => {
            let fields = fields
                .iter()
                .map(|field| {
                    Value::object([
                        ("name", Value::String(field.name.clone())),
                        ("shape", encode_port_shape_unchecked(&field.shape)?),
                    ])
                    .map_err(|error| codec_error(format!("port shape field: {error:?}")))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Value::array([
                Value::Int(4),
                Value::Array(fields),
                Value::Bool(*open),
            ]))
        }
        PortShape::Variable(name) => Ok(Value::array([Value::Int(5), Value::String(name.clone())])),
    }
}

pub fn decode_port_shape(value: Value) -> Result<PortShape, PortTypeCodecError> {
    decode_port_shape_at(value, 0)
}

fn decode_port_shape_at(value: Value, depth: usize) -> Result<PortShape, PortTypeCodecError> {
    if depth > PORT_SHAPE_MAX_DEPTH {
        return fail(format!("port shape depth exceeds {PORT_SHAPE_MAX_DEPTH}"));
    }
    let Value::Array(arm) = value else {
        return fail("port shape is not an Array");
    };
    match arm.as_slice() {
        [Value::Int(1)] => Ok(PortShape::Any),
        [Value::Int(2), Value::String(base)] => Ok(PortShape::Base(decode_base(base)?)),
        [Value::Int(3), item] => Ok(PortShape::Array(Box::new(decode_port_shape_at(
            item.clone(),
            depth + 1,
        )?))),
        [Value::Int(4), Value::Array(raw_fields), Value::Bool(open)] => {
            if raw_fields.len() > PORT_OBJECT_MAX_FIELDS {
                return fail(format!(
                    "port object shape has {} fields; maximum is {PORT_OBJECT_MAX_FIELDS}",
                    raw_fields.len()
                ));
            }
            let mut seen = BTreeSet::new();
            let mut fields = Vec::with_capacity(raw_fields.len());
            for raw in raw_fields {
                let mut raw = object_from_value(raw.clone()).map_err(port_shape_field)?;
                let name = text(
                    take(&mut raw, "name").map_err(port_shape_field)?,
                    "field name",
                )?;
                if name.is_empty() {
                    return fail("port object shape has an empty field name");
                }
                let shape = decode_port_shape_at(
                    take(&mut raw, "shape").map_err(port_shape_field)?,
                    depth + 1,
                )?;
                exhausted(raw).map_err(port_shape_field)?;
                if !seen.insert(name.clone()) {
                    return fail(format!("port object shape repeats field {name:?}"));
                }
                fields.push(PortShapeField { name, shape });
            }
            Ok(PortShape::Object {
                fields,
                open: *open,
            })
        }
        [Value::Int(5), Value::String(name)] if !name.is_empty() => {
            Ok(PortShape::Variable(name.clone()))
        }
        _ => fail(format!("port shape has an unknown closed arm: {arm:?}")),
    }
}

fn validate_port_shape(shape: &PortShape, depth: usize) -> Result<(), PortTypeCodecError> {
    if depth > PORT_SHAPE_MAX_DEPTH {
        return fail(format!("port shape depth exceeds {PORT_SHAPE_MAX_DEPTH}"));
    }
    match shape {
        PortShape::Array(item) => validate_port_shape(item, depth + 1),
        PortShape::Object { fields, .. } => {
            if fields.len() > PORT_OBJECT_MAX_FIELDS {
                return fail(format!(
                    "port object shape has {} fields; maximum is {PORT_OBJECT_MAX_FIELDS}",
                    fields.len()
                ));
            }
            let mut names = BTreeSet::new();
            for field in fields {
                if field.name.is_empty() || !names.insert(field.name.as_str()) {
                    return fail(format!(
                        "port object shape has an empty or duplicate field {:?}",
                        field.name
                    ));
                }
                validate_port_shape(&field.shape, depth + 1)?;
            }
            Ok(())
        }
        PortShape::Variable(name) if name.is_empty() => {
            fail("port shape has an empty variable name")
        }
        PortShape::Any | PortShape::Base(_) | PortShape::Variable(_) => Ok(()),
    }
}

pub fn encode_port_flow(flow: &PortFlow) -> Result<Value, PortTypeCodecError> {
    match flow {
        PortFlow::Stream(item) => Ok(Value::array([Value::Int(1), encode_port_shape(item)?])),
        PortFlow::Signal { item, rate } => Ok(Value::array([
            Value::Int(2),
            encode_port_shape(item)?,
            encode_port_rate(rate)?,
        ])),
    }
}

pub fn decode_port_flow(value: Value) -> Result<PortFlow, PortTypeCodecError> {
    let Value::Array(arm) = value else {
        return fail("port flow is not an Array");
    };
    match arm.as_slice() {
        [Value::Int(1), item] => Ok(PortFlow::Stream(decode_port_shape(item.clone())?)),
        [Value::Int(2), item, rate] => Ok(PortFlow::Signal {
            item: decode_port_shape(item.clone())?,
            rate: decode_port_rate(rate.clone())?,
        }),
        _ => fail(format!("port flow has an unknown closed arm: {arm:?}")),
    }
}

fn encode_port_rate(rate: &PortRate) -> Result<Value, PortTypeCodecError> {
    match rate {
        PortRate::Period(period) => {
            let ticks = i64::try_from(period.get().get())
                .map_err(|_| codec_error("port signal period exceeds the Value::Int wire"))?;
            Ok(Value::array([Value::Int(1), Value::Int(ticks)]))
        }
        PortRate::Variable(name) => Ok(Value::array([Value::Int(2), Value::String(name.clone())])),
    }
}

fn decode_port_rate(value: Value) -> Result<PortRate, PortTypeCodecError> {
    let Value::Array(arm) = value else {
        return fail("port rate is not an Array");
    };
    match arm.as_slice() {
        [Value::Int(1), Value::Int(ticks)] if *ticks > 0 => {
            let ticks = u64::try_from(*ticks).expect("positive i64 fits u64");
            Ok(PortRate::Period(
                NonZeroTicks::new(ticks).expect("positive period"),
            ))
        }
        [Value::Int(2), Value::String(name)] => Ok(PortRate::Variable(name.clone())),
        _ => fail(format!("port rate has an unknown or zero arm: {arm:?}")),
    }
}

fn decode_base(name: &str) -> Result<BaseShape, PortTypeCodecError> {
    BaseShape::from_str(name)
        .ok_or_else(|| codec_error(format!("port shape has unknown base {name:?}")))
}

fn port_shape_field(error: WireError) -> PortTypeCodecError {
    codec_error(format!("port shape field {error}"))
}

fn text(value: Value, label: &str) -> Result<String, PortTypeCodecError> {
    match value {
        Value::String(value) => Ok(value),
        _ => fail(format!("{label} is not a String")),
    }
}

fn codec_error(detail: impl Into<String>) -> PortTypeCodecError {
    PortTypeCodecError {
        detail: detail.into(),
    }
}

fn fail<T>(detail: impl Into<String>) -> Result<T, PortTypeCodecError> {
    Err(codec_error(detail))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested_shape() -> PortShape {
        PortShape::Object {
            fields: vec![
                PortShapeField {
                    name: "items".to_owned(),
                    shape: PortShape::Array(Box::new(PortShape::Object {
                        fields: vec![PortShapeField {
                            name: "value".to_owned(),
                            shape: PortShape::Variable("T".to_owned()),
                        }],
                        open: false,
                    })),
                },
                PortShapeField {
                    name: "source".to_owned(),
                    shape: PortShape::Base(BaseShape::String),
                },
            ],
            open: true,
        }
    }

    #[test]
    fn nested_shape_and_both_signal_rate_arms_round_trip_losslessly() {
        for flow in [
            PortFlow::Stream(nested_shape()),
            PortFlow::Signal {
                item: nested_shape(),
                rate: PortRate::Period(NonZeroTicks::new(8).expect("period")),
            },
            PortFlow::Signal {
                item: PortShape::Variable("Sample".to_owned()),
                rate: PortRate::Variable("R".to_owned()),
            },
        ] {
            let value = encode_port_flow(&flow).expect("encode");
            assert_eq!(decode_port_flow(value).expect("decode"), flow);
        }
    }

    #[test]
    fn unknown_arms_extra_fields_zero_rate_and_duplicate_fields_are_rejected() {
        assert!(decode_port_flow(Value::array([Value::Int(9)])).is_err());
        assert!(
            decode_port_flow(Value::array([
                Value::Int(2),
                Value::array([Value::Int(1)]),
                Value::array([Value::Int(1), Value::Int(0)]),
            ]))
            .is_err()
        );
        let field = |extra: bool| {
            let mut values = vec![
                ("name", Value::String("same".to_owned())),
                ("shape", Value::array([Value::Int(1)])),
            ];
            if extra {
                values.push(("extra", Value::Null));
            }
            Value::object(values).expect("unique fields")
        };
        let object =
            |fields| Value::array([Value::Int(4), Value::Array(fields), Value::Bool(false)]);
        assert!(decode_port_shape(object(vec![field(true)])).is_err());
        assert!(decode_port_shape(object(vec![field(false), field(false)])).is_err());
    }
}
