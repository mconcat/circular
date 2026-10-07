
use std::fmt;

use crate::{Shape, Value};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScalarText<'a> {
    Text(&'a str),
    Int(i64),
    Bool(bool),
}

impl fmt::Display for ScalarText<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text(text) => formatter.write_str(text),
            Self::Int(number) => write!(formatter, "{number}"),
            Self::Bool(flag) => write!(formatter, "{flag}"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SegmentText<'a> {
    Child(&'a str),
    Instance {
        of: &'a str,
        key: Vec<ScalarText<'a>>,
    },
}

impl fmt::Display for SegmentText<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Child(name) => formatter.write_str(name),
            Self::Instance { of, key } => {
                write!(formatter, "{of}[")?;
                for (index, scalar) in key.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{scalar}")?;
                }
                formatter.write_str("]")
            }
        }
    }
}

pub trait SpelledSegment {
    fn spelled(&self) -> SegmentText<'_>;
}

pub struct ScopeText<'a, S>(pub &'a [S]);

impl<S: SpelledSegment> fmt::Display for ScopeText<'_, S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            return formatter.write_str("the root scope");
        }
        formatter.write_str("scope ")?;
        write_path(formatter, self.0)
    }
}

pub struct ActorText<'a, S, L> {
    scope: &'a [S],
    local: L,
}

pub fn actor<S, L>(scope: &[S], local: L) -> ActorText<'_, S, L> {
    ActorText { scope, local }
}

impl<S: SpelledSegment, L: fmt::Display> fmt::Display for ActorText<'_, S, L> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.scope.is_empty() {
            write_path(formatter, self.scope)?;
            formatter.write_str("/")?;
        }
        write!(formatter, "{}", self.local)
    }
}

fn write_path<S: SpelledSegment>(formatter: &mut fmt::Formatter<'_>, scope: &[S]) -> fmt::Result {
    for (index, segment) in scope.iter().enumerate() {
        if index > 0 {
            formatter.write_str("/")?;
        }
        write!(formatter, "{}", segment.spelled())?;
    }
    Ok(())
}

pub struct Allowed<I>(I);

pub fn allowed<I>(items: I) -> Allowed<I>
where
    I: IntoIterator + Clone,
    I::Item: fmt::Display,
{
    Allowed(items)
}

impl<I> fmt::Display for Allowed<I>
where
    I: IntoIterator + Clone,
    I::Item: fmt::Display,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("allowed: ")?;
        let mut empty = true;
        for (index, item) in self.0.clone().into_iter().enumerate() {
            if index > 0 {
                formatter.write_str(", ")?;
            }
            write!(formatter, "{item}")?;
            empty = false;
        }
        if empty {
            formatter.write_str("none")?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Quoted<'a>(pub &'a str);

impl fmt::Display for Quoted<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("\"")?;
        for character in self.0.chars() {
            match character {
                '"' => formatter.write_str("\\\"")?,
                '\\' => formatter.write_str("\\\\")?,
                '\n' => formatter.write_str("\\n")?,
                '\r' => formatter.write_str("\\r")?,
                '\t' => formatter.write_str("\\t")?,
                control if control.is_control() => {
                    write!(formatter, "\\u{:04x}", u32::from(control))?;
                }
                other => write!(formatter, "{other}")?,
            }
        }
        formatter.write_str("\"")
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ValueText<'a>(pub &'a Value);

impl fmt::Display for ValueText<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Value::Null => formatter.write_str("null"),
            Value::Bool(flag) => write!(formatter, "{flag}"),
            Value::Int(number) => write!(formatter, "{number}"),
            Value::UInt(number) => write!(formatter, "{number}"),
            Value::Float(number) => write!(formatter, "{number}"),
            Value::String(text) => write!(formatter, "{}", Quoted(text)),
            Value::Bytes(bytes) => write!(formatter, "<{} bytes>", bytes.len()),
            Value::Array(items) => {
                formatter.write_str("[")?;
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{}", ValueText(item))?;
                }
                formatter.write_str("]")
            }
            Value::Object(object) => {
                formatter.write_str("{")?;
                for (index, (key, item)) in object.iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{}: {}", Quoted(key), ValueText(item))?;
                }
                formatter.write_str("}")
            }
        }
    }
}

pub struct ShapeText<'a, N>(pub &'a Shape<N>);

impl<N: fmt::Display> fmt::Display for ShapeText<'_, N> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Shape::Any => formatter.write_str("Any"),
            Shape::Base(base) => write!(formatter, "{base}"),
            Shape::Array(item) => write!(formatter, "array<{}>", ShapeText(item.as_ref())),
            Shape::Object { fields, open } => {
                formatter.write_str("object<")?;
                for (index, (name, shape)) in fields.as_slice().iter().enumerate() {
                    if index > 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "{name}: {}", ShapeText(shape))?;
                }
                if *open {
                    if !fields.as_slice().is_empty() {
                        formatter.write_str(", ")?;
                    }
                    formatter.write_str("...")?;
                }
                formatter.write_str(">")
            }
            Shape::Var(name) => write!(formatter, "{name}"),
        }
    }
}
