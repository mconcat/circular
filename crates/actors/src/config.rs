
use std::collections::{BTreeMap, BTreeSet, btree_map::Entry as MapEntry};
use std::error::Error;
use std::fmt;

use circular_core::spelling::{self, Quoted};
use circular_core::{Fields, FloatValue, Millis, NonZeroMillis, NotObject, UnknownField};
use circular_core::{Value, ValueKind};
use circular_expr::{ConfigPath, EvalMode, Segment, ValuePath};
use circular_protocol::port_type::decode_port_flow;
use circular_runtime::{AgentHarnessName, EmptyAgentName, PeerAdapterName, PeerTextError};

use crate::ports::PortId;
use crate::spec::{DisplayTextError, Label};
use crate::types::{BaseShape, GroundFlow, Name, Shape, flow_from_port_type, value_matches_shape};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PayloadRoot;

impl PayloadRoot {
    #[must_use]
    pub fn path() -> PayloadPath {
        ValuePath::new(Self, [])
    }
}

pub type PayloadPath = ValuePath<PayloadRoot>;

pub fn payload_path_from_value(value: &Value) -> Result<PayloadPath, PayloadPathError> {
    let Some(segments) = value.as_array() else {
        return Err(PayloadPathError::NotAnArray { got: value.kind() });
    };

    let mut path = ValuePath::new(PayloadRoot, []);
    for (at, segment) in segments.iter().enumerate() {
        if let Some(key) = segment.as_str() {
            path = path.join_key(key);
        } else if let Some(index) = segment.as_int() {
            let index =
                u64::try_from(index).map_err(|_| PayloadPathError::NegativeIndex { at, index })?;
            path = path.join_index(index);
        } else {
            return Err(PayloadPathError::SegmentNotKeyOrIndex {
                at,
                got: segment.kind(),
            });
        }
    }
    Ok(path)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadPathError {
    NotAnArray {
        got: ValueKind,
    },
    SegmentNotKeyOrIndex {
        at: usize,
        got: ValueKind,
    },
    NegativeIndex {
        at: usize,
        index: i64,
    },
}

impl fmt::Display for PayloadPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnArray { got } => {
                write!(formatter, "exact payload path must be an array, got {got}")
            }
            Self::SegmentNotKeyOrIndex { at, got } => write!(
                formatter,
                "path segment {at} must be a string key or an integer index, got {got}"
            ),
            Self::NegativeIndex { at, index } => write!(
                formatter,
                "path segment {at} index must be nonnegative, got {index}"
            ),
        }
    }
}

impl Error for PayloadPathError {}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum IntervalDomain {
    Milliseconds,
    NonZeroMilliseconds,
}

circular_core::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum IntegerMinimum: i64 {
        Zero = 0,
        One = 1,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntegerBound(i64);

impl IntegerBound {
    pub const fn try_new(value: i64) -> Result<Self, IntegerBoundError> {
        if value < 0 {
            return Err(IntegerBoundError::Negative);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IntegerBoundError {
    Negative,
}

impl fmt::Display for IntegerBoundError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Negative => "integer bound must be nonnegative",
        };
        formatter.write_str(message)
    }
}

impl Error for IntegerBoundError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntegerCount {
    minimum: IntegerMinimum,
    maximum: Option<IntegerBound>,
}

impl IntegerCount {
    pub fn try_new(
        minimum: IntegerMinimum,
        maximum: Option<IntegerBound>,
    ) -> Result<Self, IntegerCountError> {
        if let Some(maximum) = maximum
            && maximum.get() < minimum.tag()
        {
            return Err(IntegerCountError { minimum, maximum });
        }
        Ok(Self { minimum, maximum })
    }

    #[must_use]
    pub const fn minimum(&self) -> IntegerMinimum {
        self.minimum
    }

    #[must_use]
    pub const fn maximum(&self) -> Option<IntegerBound> {
        self.maximum
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IntegerCountError {
    minimum: IntegerMinimum,
    maximum: IntegerBound,
}

impl IntegerCountError {
    #[must_use]
    pub const fn minimum(&self) -> IntegerMinimum {
        self.minimum
    }

    #[must_use]
    pub const fn maximum(&self) -> IntegerBound {
        self.maximum
    }
}

impl fmt::Display for IntegerCountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "integer-count maximum {} is below the minimum {}",
            self.maximum.get(),
            self.minimum.tag()
        )
    }
}

impl Error for IntegerCountError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClosedTags {
    members: BTreeSet<String>,
}

impl ClosedTags {
    pub fn try_from_members<I, S>(members: I) -> Result<Self, ClosedTagsError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut canonical = BTreeSet::new();
        for member in members {
            let member = member.into();
            if !canonical.insert(member.clone()) {
                return Err(ClosedTagsError::DuplicateMember(member));
            }
        }
        if canonical.is_empty() {
            return Err(ClosedTagsError::Empty);
        }
        Ok(Self { members: canonical })
    }

    #[must_use]
    pub fn contains(&self, value: &str) -> bool {
        self.members.contains(value)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.members.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &str> {
        self.members.iter().map(String::as_str)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClosedTagsError {
    Empty,
    DuplicateMember(String),
}

impl fmt::Display for ClosedTagsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("closed tags must not be empty"),
            Self::DuplicateMember(member) => {
                write!(formatter, "duplicate closed-tag member {member:?}")
            }
        }
    }
}

impl Error for ClosedTagsError {}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TextDomain {
    Label,
    Name,
    AgentHarnessName,
    ToolName,
    ModelProviderName,
    ModelName,
    PeerAdapterName,
}

impl TextDomain {
    #[must_use]
    pub const fn noun(self) -> &'static str {
        match self {
            Self::Label => "label",
            Self::Name => "name",
            Self::AgentHarnessName => "agent harness name",
            Self::ToolName => "tool name",
            Self::ModelProviderName => "model provider name",
            Self::ModelName => "model name",
            Self::PeerAdapterName => "peer adapter name",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum TextParser {
    Label,
    AgentHarnessName,
    PeerAdapterName,
}

impl TextDomain {
    const fn parser(self) -> Result<TextParser, UnresolvedConfigConstraint> {
        match self {
            Self::Label => Ok(TextParser::Label),
            Self::AgentHarnessName => Ok(TextParser::AgentHarnessName),
            Self::PeerAdapterName => Ok(TextParser::PeerAdapterName),
            Self::Name => Err(UnresolvedConfigConstraint::NameParser),
            Self::ToolName => Err(UnresolvedConfigConstraint::ToolNameParser),
            Self::ModelProviderName => Err(UnresolvedConfigConstraint::ModelProviderNameParser),
            Self::ModelName => Err(UnresolvedConfigConstraint::ModelNameParser),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigConstraint {
    Interval(IntervalDomain),
    IntegerCount(IntegerCount),
    FiniteNumber,
    ClosedUnitInterval,
    ClosedTags(ClosedTags),
    CanonicalText(TextDomain),
    CanonicalTypeExpr,
    ExactPayloadPath,
    /// A canonical event stream whose item is in the existing BaseShape vocabulary.
    CanonicalBaseStream,
    IntervalList(IntervalDomain),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigDecodeError {
    Unresolved(UnresolvedConfigConstraint),
    IntervalKind {
        actual: ValueKind,
    },
    IntervalListKind {
        actual: ValueKind,
    },
    NegativeInterval {
        actual: i64,
    },
    ZeroInterval,
    CountKind {
        actual: ValueKind,
    },
    CountBelowMinimum {
        actual: i64,
        minimum: i64,
    },
    CountAboveMaximum {
        actual: i64,
        maximum: i64,
    },
    FiniteKind {
        actual: ValueKind,
    },
    NonFinite {
        actual: FloatValue,
    },
    UnitIntervalKind {
        actual: ValueKind,
    },
    NonFiniteUnitInterval {
        actual: FloatValue,
    },
    OutsideUnitInterval {
        actual: FloatValue,
    },
    TagKind {
        actual: ValueKind,
    },
    UnknownTag {
        actual: String,
        allowed: Vec<String>,
    },
    CanonicalTextKind {
        domain: TextDomain,
        actual: ValueKind,
    },
    InvalidLabel {
        actual: String,
        source: DisplayTextError,
    },
    InvalidAgentHarnessName(EmptyAgentName),
    InvalidPeerAdapterName(PeerTextError),
    InvalidTypeExpr(circular_protocol::port_type::PortTypeCodecError),
    NonGroundTypeExpr,
    PayloadPath(PayloadPathError),
}

impl fmt::Display for ConfigDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unresolved(error) => error.fmt(formatter),
            Self::IntervalKind { actual } => {
                write!(formatter, "interval must be an integer, got {actual}")
            }
            Self::IntervalListKind { actual } => {
                write!(formatter, "interval list must be an array, got {actual}")
            }
            Self::NegativeInterval { actual } => {
                write!(formatter, "interval must be nonnegative, got {actual}")
            }
            Self::ZeroInterval => formatter.write_str("interval must be nonzero"),
            Self::CountKind { actual } => {
                write!(formatter, "count must be an integer, got {actual}")
            }
            Self::CountBelowMinimum { actual, minimum } => {
                write!(formatter, "count {actual} is below minimum {minimum}")
            }
            Self::CountAboveMaximum { actual, maximum } => {
                write!(formatter, "count {actual} is above maximum {maximum}")
            }
            Self::FiniteKind { actual } => {
                write!(formatter, "finite number must be a float, got {actual}")
            }
            Self::NonFinite { actual } => write!(formatter, "number must be finite, got {actual}"),
            Self::UnitIntervalKind { actual } => {
                write!(formatter, "unit interval must be a float, got {actual}")
            }
            Self::NonFiniteUnitInterval { actual } => {
                write!(formatter, "unit interval must be finite, got {actual}")
            }
            Self::OutsideUnitInterval { actual } => {
                write!(
                    formatter,
                    "unit interval must be between 0 and 1, got {actual}"
                )
            }
            Self::TagKind { actual } => {
                write!(formatter, "closed tag must be a string, got {actual}")
            }
            Self::UnknownTag { actual, allowed } => write!(
                formatter,
                "unknown closed tag {}; {}",
                Quoted(actual),
                spelling::allowed(allowed.iter().map(|tag| Quoted(tag)))
            ),
            Self::CanonicalTextKind { domain, actual } => {
                write!(
                    formatter,
                    "{} must be a string, got {actual}",
                    domain.noun()
                )
            }
            Self::InvalidLabel { actual, source } => {
                write!(formatter, "invalid label {}: {source}", Quoted(actual))
            }
            Self::InvalidAgentHarnessName(error) => {
                write!(formatter, "invalid agent harness name: {error}")
            }
            Self::InvalidPeerAdapterName(error) => {
                write!(formatter, "invalid peer adapter name: {error}")
            }
            Self::InvalidTypeExpr(error) => write!(formatter, "invalid type expression: {error}"),
            Self::NonGroundTypeExpr => {
                formatter.write_str("type expression contains an unresolved variable")
            }
            Self::PayloadPath(error) => error.fmt(formatter),
        }
    }
}

impl Error for ConfigDecodeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Unresolved(error) => Some(error),
            Self::InvalidLabel { source, .. } => Some(source),
            Self::InvalidAgentHarnessName(error) => Some(error),
            Self::InvalidPeerAdapterName(error) => Some(error),
            Self::InvalidTypeExpr(error) => Some(error),
            Self::PayloadPath(error) => Some(error),
            Self::IntervalKind { .. }
            | Self::IntervalListKind { .. }
            | Self::NegativeInterval { .. }
            | Self::ZeroInterval
            | Self::CountKind { .. }
            | Self::CountBelowMinimum { .. }
            | Self::CountAboveMaximum { .. }
            | Self::FiniteKind { .. }
            | Self::NonFinite { .. }
            | Self::UnitIntervalKind { .. }
            | Self::NonFiniteUnitInterval { .. }
            | Self::OutsideUnitInterval { .. }
            | Self::TagKind { .. }
            | Self::UnknownTag { .. }
            | Self::CanonicalTextKind { .. }
            | Self::NonGroundTypeExpr => None,
        }
    }
}

fn interval_millis(value: &Value) -> Result<Millis, ConfigDecodeError> {
    match value {
        Value::Int(raw) => u64::try_from(*raw)
            .map(Millis::new)
            .map_err(|_| ConfigDecodeError::NegativeInterval { actual: *raw }),
        Value::UInt(raw) => Ok(Millis::new(*raw)),
        _ => Err(ConfigDecodeError::IntervalKind {
            actual: value.kind(),
        }),
    }
}

fn nonzero_interval_millis(value: &Value) -> Result<NonZeroMillis, ConfigDecodeError> {
    NonZeroMillis::new(interval_millis(value)?.get()).map_err(|_| ConfigDecodeError::ZeroInterval)
}

fn interval_items(value: &Value) -> Result<&[Value], ConfigDecodeError> {
    match value {
        Value::Array(items) => Ok(items),
        _ => Err(ConfigDecodeError::IntervalListKind {
            actual: value.kind(),
        }),
    }
}

impl IntegerCount {
    fn decode_count(&self, value: &Value) -> Result<i64, ConfigDecodeError> {
        let integer = value.as_int().ok_or(ConfigDecodeError::CountKind {
            actual: value.kind(),
        })?;
        let minimum = self.minimum.tag();
        if integer < minimum {
            return Err(ConfigDecodeError::CountBelowMinimum {
                actual: integer,
                minimum,
            });
        }
        if let Some(maximum) = self.maximum
            && integer > maximum.get()
        {
            return Err(ConfigDecodeError::CountAboveMaximum {
                actual: integer,
                maximum: maximum.get(),
            });
        }
        Ok(integer)
    }
}

impl ClosedTags {
    fn decode_tag<'v>(&self, value: &'v Value) -> Result<&'v str, ConfigDecodeError> {
        let tag = value.as_str().ok_or(ConfigDecodeError::TagKind {
            actual: value.kind(),
        })?;
        if !self.contains(tag) {
            return Err(ConfigDecodeError::UnknownTag {
                actual: tag.to_owned(),
                allowed: self.iter().map(str::to_owned).collect(),
            });
        }
        Ok(tag)
    }
}

impl ConfigConstraint {
    pub fn judge(&self, value: &Value) -> Result<(), ConfigDecodeError> {
        fn judged<K: SlotKind>(kind: &K, value: &Value) -> Result<(), ConfigDecodeError> {
            kind.decode(value).map(drop)
        }
        match self {
            Self::Interval(IntervalDomain::Milliseconds) => judged(&Interval, value),
            Self::Interval(IntervalDomain::NonZeroMilliseconds) => judged(&NonZeroInterval, value),
            Self::IntegerCount(count) => judged(count, value),
            Self::FiniteNumber => judged(&Finite, value),
            Self::ClosedUnitInterval => judged(&UnitInterval, value),
            Self::ClosedTags(tags) => judged(tags, value),
            Self::CanonicalText(domain) => match domain.parser() {
                Err(unresolved) => Err(ConfigDecodeError::Unresolved(unresolved)),
                Ok(TextParser::Label) => judged(&LabelText, value),
                Ok(TextParser::AgentHarnessName) => judged(&HarnessName, value),
                Ok(TextParser::PeerAdapterName) => judged(&PeerAdapter, value),
            },
            Self::CanonicalTypeExpr => judged(&TypeExpr, value),
            Self::CanonicalBaseStream => judged(&BaseStreamTypeExpr, value),
            Self::ExactPayloadPath => judged(&ExactPath, value),
            Self::IntervalList(domain) => interval_items(value)?
                .iter()
                .try_for_each(|item| Self::Interval(*domain).judge(item)),
        }
    }

    pub fn accepts_value(&self, value: &Value) -> Result<bool, UnresolvedConfigConstraint> {
        match self.judge(value) {
            Err(ConfigDecodeError::Unresolved(unresolved)) => Err(unresolved),
            judged => Ok(judged.is_ok()),
        }
    }

    #[must_use]
    pub const fn unresolved_dependency(&self) -> Option<UnresolvedConfigConstraint> {
        match self {
            Self::CanonicalText(domain) => match domain.parser() {
                Ok(_) => None,
                Err(unresolved) => Some(unresolved),
            },
            Self::Interval(_)
            | Self::IntervalList(_)
            | Self::IntegerCount(_)
            | Self::FiniteNumber
            | Self::ClosedUnitInterval
            | Self::ClosedTags(_)
            | Self::CanonicalTypeExpr
            | Self::CanonicalBaseStream
            | Self::ExactPayloadPath => None,
        }
    }

    #[must_use]
    const fn required_base_shape(&self) -> Option<BaseShape> {
        match self {
            Self::Interval(_) | Self::IntervalList(_) | Self::IntegerCount(_) => {
                Some(BaseShape::Int)
            }
            Self::FiniteNumber | Self::ClosedUnitInterval => Some(BaseShape::Float),
            Self::ClosedTags(_) | Self::CanonicalText(_) => Some(BaseShape::String),
            Self::CanonicalTypeExpr | Self::CanonicalBaseStream | Self::ExactPayloadPath => None,
        }
    }

    #[must_use]
    const fn over_items(&self) -> bool {
        matches!(self, Self::IntervalList(_))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnresolvedConfigConstraint {
    NameParser,
    AgentHarnessNameParser,
    ToolNameParser,
    ModelProviderNameParser,
    ModelNameParser,
    CanonicalTypeExprValueEncoding,
}

impl fmt::Display for UnresolvedConfigConstraint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let dependency = match self {
            Self::NameParser => "canonical authoring name parser",
            Self::AgentHarnessNameParser => "canonical agent-harness-name parser",
            Self::ToolNameParser => "canonical tool-name parser",
            Self::ModelProviderNameParser => "canonical model-provider-name parser",
            Self::ModelNameParser => "canonical model-name parser",
            Self::CanonicalTypeExprValueEncoding => "canonical type-expression Value encoding",
        };
        write!(
            formatter,
            "unresolved config constraint dependency: {dependency}"
        )
    }
}

impl Error for UnresolvedConfigConstraint {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigSpace {
    shape: Shape,
    constraint: Option<ConfigConstraint>,
}

impl ConfigSpace {
    #[must_use]
    pub const fn unconstrained(shape: Shape) -> Self {
        Self {
            shape,
            constraint: None,
        }
    }

    pub fn try_new(shape: Shape, constraint: ConfigConstraint) -> Result<Self, ConfigSpaceError> {
        if let Some(required) = constraint.required_base_shape() {
            let constrained = match (&shape, constraint.over_items()) {
                (Shape::Array(item), true) => Some(item.as_ref()),
                (Shape::Any, _) | (_, false) => Some(&shape),
                (_, true) => None,
            };
            let shape_is_compatible = match constrained {
                None => false,
                Some(Shape::Any) => true,
                Some(Shape::Base(actual)) => *actual == required,
                Some(Shape::Object { .. } | Shape::Array(_) | Shape::Var(_)) => false,
            };
            if !shape_is_compatible {
                return Err(ConfigSpaceError::IncompatibleShape { required });
            }
        }

        Ok(Self {
            shape,
            constraint: Some(constraint),
        })
    }

    #[must_use]
    pub const fn shape(&self) -> &Shape {
        &self.shape
    }

    #[must_use]
    pub const fn constraint(&self) -> Option<&ConfigConstraint> {
        self.constraint.as_ref()
    }

    pub fn unary_readiness(&self) -> Result<(), UnresolvedConfigConstraint> {
        match self
            .constraint
            .as_ref()
            .and_then(ConfigConstraint::unresolved_dependency)
        {
            Some(unresolved) => Err(unresolved),
            None => Ok(()),
        }
    }

    pub fn accepts(&self, value: &Value) -> Result<bool, UnresolvedConfigConstraint> {
        if !value_matches_shape(value, &self.shape) {
            return Ok(false);
        }
        match &self.constraint {
            Some(constraint) => constraint.accepts_value(value),
            None => Ok(true),
        }
    }
}

impl ConfigSpace {
    #[must_use]
    pub fn allowed(&self) -> String {
        let phrase = match &self.constraint {
            Some(ConfigConstraint::ClosedTags(tags)) => {
                let tags: Vec<&str> = tags.iter().collect();
                return spelling::allowed(tags.iter().map(|tag| Quoted(tag))).to_string();
            }
            Some(ConfigConstraint::Interval(IntervalDomain::Milliseconds)) => {
                "whole milliseconds".to_owned()
            }
            Some(ConfigConstraint::Interval(IntervalDomain::NonZeroMilliseconds)) => {
                "whole milliseconds of at least 1".to_owned()
            }
            Some(ConfigConstraint::IntervalList(IntervalDomain::Milliseconds)) => {
                "a list of whole milliseconds".to_owned()
            }
            Some(ConfigConstraint::IntervalList(IntervalDomain::NonZeroMilliseconds)) => {
                "a list of whole milliseconds, each at least 1".to_owned()
            }
            Some(ConfigConstraint::IntegerCount(count)) => match count.maximum() {
                Some(maximum) => format!(
                    "integers from {} to {}",
                    count.minimum().tag(),
                    maximum.get()
                ),
                None => format!("integers of at least {}", count.minimum().tag()),
            },
            Some(ConfigConstraint::FiniteNumber) => "finite floats".to_owned(),
            Some(ConfigConstraint::ClosedUnitInterval) => "floats from 0 to 1".to_owned(),
            Some(ConfigConstraint::CanonicalText(domain)) => format!("a {}", domain.noun()),
            Some(ConfigConstraint::CanonicalTypeExpr) => "a type expression".to_owned(),
            Some(ConfigConstraint::CanonicalBaseStream) => {
                "a stream type expression over a base shape".to_owned()
            }
            Some(ConfigConstraint::ExactPayloadPath) => "a payload path".to_owned(),
            None => spelling::ShapeText(&self.shape).to_string(),
        };
        spelling::allowed([phrase]).to_string()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigSpaceError {
    IncompatibleShape { required: BaseShape },
}

impl fmt::Display for ConfigSpaceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self::IncompatibleShape { required } = self;
        write!(formatter, "config constraint requires the {required} shape")
    }
}

impl Error for ConfigSpaceError {}

#[derive(Clone, Debug, PartialEq)]
pub enum Required {
    Mandatory,
    Optional {
        default: Value,
    },
    Omittable {
        absent: crate::types::Flow,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnippetSlot {
    mode: EvalMode,
    inlets: Box<[PortId]>,
}

impl SnippetSlot {
    #[must_use]
    pub const fn new(mode: EvalMode, inlets: Box<[PortId]>) -> Self {
        Self { mode, inlets }
    }

    #[must_use]
    pub const fn mode(&self) -> EvalMode {
        self.mode
    }

    #[must_use]
    pub const fn inlets(&self) -> &[PortId] {
        &self.inlets
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConfigSlot {
    space: ConfigSpace,
    required: Required,
    snippet: Option<SnippetSlot>,
    preserve_omission: bool,
    /// The value a create form opens a mandatory slot at. It is the registration's
    /// suggestion, published in the create draft (`CreateInputSchema::draft`); the slot stays
    /// mandatory, so admission never writes it for an author who left the slot out.
    start: Option<Value>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub group: Option<String>,
}

impl ConfigSlot {
    pub fn try_new(
        space: ConfigSpace,
        required: Required,
        snippet: Option<SnippetSlot>,
    ) -> Result<Self, ConfigSlotError> {
        if let Required::Optional { default } = &required {
            match space.accepts(default) {
                Ok(true) => {}
                Ok(false) => return Err(ConfigSlotError::DefaultOutsideSpace),
                Err(unresolved) => {
                    return Err(ConfigSlotError::DefaultConstraintUnresolved(unresolved));
                }
            }
            if snippet.is_some() {
                return Err(ConfigSlotError::SnippetDefaultValidationUnavailable);
            }
        }

        Ok(Self {
            space,
            required,
            snippet,
            preserve_omission: false,
            start: None,
            label: None,
            description: None,
            group: None,
        })
    }

    pub(crate) fn preserving_omission(mut self) -> Self {
        assert!(matches!(self.required, Required::Optional { .. }));
        self.preserve_omission = true;
        self
    }

    #[must_use]
    pub const fn space(&self) -> &ConfigSpace {
        &self.space
    }

    #[must_use]
    pub const fn required(&self) -> &Required {
        &self.required
    }

    #[must_use]
    pub const fn snippet(&self) -> Option<&SnippetSlot> {
        self.snippet.as_ref()
    }

    #[must_use]
    pub(crate) fn with_text(mut self, label: &str, description: &str) -> Self {
        self.label = Some(label.to_owned());
        self.description = Some(description.to_owned());
        self
    }

    #[must_use]
    pub(crate) fn in_group(mut self, group: &str) -> Self {
        self.group = Some(group.to_owned());
        self
    }

    /// The value a create form opens this mandatory slot at. Only a mandatory slot has
    /// one (an optional slot's default already is its starting value), and it lies in the slot's
    /// space, so a form left at it is admitted.
    #[must_use]
    pub(crate) fn starting_at(mut self, value: Value) -> Self {
        assert!(matches!(self.required, Required::Mandatory));
        assert_eq!(self.space.accepts(&value), Ok(true));
        self.start = Some(value);
        self
    }

    /// The value a create form opens this slot at, when the registration suggests one.
    #[must_use]
    pub const fn start(&self) -> Option<&Value> {
        self.start.as_ref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigSlotError {
    DefaultOutsideSpace,
    DefaultConstraintUnresolved(UnresolvedConfigConstraint),
    SnippetDefaultValidationUnavailable,
}

impl fmt::Display for ConfigSlotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::DefaultOutsideSpace => {
                "optional config default does not belong to the declared config space"
            }
            Self::DefaultConstraintUnresolved(unresolved) => {
                return write!(
                    formatter,
                    "optional config default requires unresolved constraint dependency: {unresolved}"
                );
            }
            Self::SnippetDefaultValidationUnavailable => {
                "optional snippet default cannot be admitted before the canonical snippet parser"
            }
        };
        formatter.write_str(message)
    }
}

impl Error for ConfigSlotError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SuppressRule {
    NamedOnly,
    Equal(CompareTarget),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompareTarget {
    Fixed(PayloadPath),
    FromConfig(ConfigPath),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SuppressDecl {
    rules: BTreeMap<Name, SuppressRule>,
}

impl SuppressDecl {
    pub fn try_from_entries(
        entries: impl IntoIterator<Item = (Name, SuppressRule)>,
    ) -> Result<Self, SuppressDeclError> {
        let mut rules = BTreeMap::new();
        for (reason, rule) in entries {
            match rules.entry(reason) {
                MapEntry::Vacant(entry) => {
                    entry.insert(rule);
                }
                MapEntry::Occupied(entry) => {
                    return Err(SuppressDeclError::DuplicateReason(entry.key().clone()));
                }
            }
        }
        if rules.is_empty() {
            return Err(SuppressDeclError::Empty);
        }
        Ok(Self { rules })
    }

    #[must_use]
    pub fn get(&self, reason: &Name) -> Option<&SuppressRule> {
        self.rules.get(reason)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&Name, &SuppressRule)> {
        self.rules.iter()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SuppressDeclError {
    Empty,
    DuplicateReason(Name),
}

impl fmt::Display for SuppressDeclError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("suppression declaration must not be empty"),
            Self::DuplicateReason(reason) => {
                write!(formatter, "duplicate suppression reason {reason:?}")
            }
        }
    }
}

impl Error for SuppressDeclError {}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct IncompleteRelations;

#[derive(Clone, Debug, PartialEq)]
struct IncompleteConfigSchemaFrame {
    slots: BTreeMap<ConfigPath, ConfigSlot>,
    _relations: IncompleteRelations,
    suppress: Option<SuppressDecl>,
    create_inputs: Option<CompleteCreateInputFrame>,
    create_input_stop_line: Option<&'static str>,
}

/// Relation arms whose canonical admission semantics are complete enough to
/// participate in a registry-owned create-input contract.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CreateInputRelation {
    /// Values of the object at `at` must be pairwise structurally distinct.
    /// Route uses this to keep one match value from naming two output ports;
    /// the arm itself is actor-agnostic and can be reused by any registration.
    UniqueObjectValues { at: ConfigPath },
}

#[derive(Clone, Debug, PartialEq)]
struct CompleteCreateInputFrame {
    relations: Vec<CreateInputRelation>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConfigSchema {
    incomplete: IncompleteConfigSchemaFrame,
}

impl ConfigSchema {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            incomplete: IncompleteConfigSchemaFrame {
                slots: BTreeMap::new(),
                _relations: IncompleteRelations,
                suppress: None,
                create_inputs: Some(CompleteCreateInputFrame {
                    relations: Vec::new(),
                }),
                create_input_stop_line: None,
            },
        }
    }

    pub(crate) fn try_from_parts(
        entries: impl IntoIterator<Item = (ConfigPath, ConfigSlot)>,
        suppress: Option<SuppressDecl>,
    ) -> Result<Self, DuplicateConfigPath> {
        let mut slots = BTreeMap::new();
        for (path, slot) in entries {
            match slots.entry(path) {
                MapEntry::Vacant(entry) => {
                    entry.insert(slot);
                }
                MapEntry::Occupied(entry) => {
                    return Err(DuplicateConfigPath {
                        path: entry.key().clone(),
                    });
                }
            }
        }

        Ok(Self {
            incomplete: IncompleteConfigSchemaFrame {
                slots,
                _relations: IncompleteRelations,
                suppress,
                create_inputs: None,
                create_input_stop_line: None,
            },
        })
    }

    /// Attach the producer-owned reason this incomplete frame cannot yet be
    /// used for actor creation. The reason is projected to clients only as an
    /// unavailable fact; it never promotes the visible slots to admission.
    pub(crate) fn with_create_input_stop_line(mut self, stop_line: &'static str) -> Self {
        assert!(
            !stop_line.trim().is_empty(),
            "create-input stop line must be nonempty"
        );
        assert!(
            self.incomplete.create_inputs.is_none(),
            "a sealed create-input contract cannot also carry a stop line"
        );
        self.incomplete.create_input_stop_line = Some(stop_line);
        self
    }

    /// Build a registration-owned create-input subset whose complete
    /// admission is represented by exact top-level slots, executable unary
    /// constraints, canonical snippet parsers, and the closed relation arms.
    ///
    /// This constructor is crate-private on purpose: a downstream client
    /// cannot promote the incomplete frame merely because it can display its
    /// slots. Registration source code must make the completeness claim.
    pub(crate) fn try_from_create_parts(
        entries: impl IntoIterator<Item = (ConfigPath, ConfigSlot)>,
        relations: impl IntoIterator<Item = CreateInputRelation>,
        suppress: Option<SuppressDecl>,
    ) -> Result<Self, CreateInputSealError> {
        let mut schema =
            Self::try_from_parts(entries, suppress).map_err(CreateInputSealError::DuplicatePath)?;

        for (path, slot) in schema.iter() {
            if top_level_key(path).is_none() {
                return Err(CreateInputSealError::UnsupportedPath(path.clone()));
            }
            if let Some(snippet) = slot.snippet() {
                let mut seen = BTreeSet::new();
                for inlet in snippet.inlets() {
                    if !seen.insert(inlet.as_str()) {
                        return Err(CreateInputSealError::DuplicateSnippetInlet {
                            path: path.clone(),
                            inlet: inlet.as_str().to_owned(),
                        });
                    }
                }
            }
        }
        schema
            .confirmed_unary_readiness()
            .map_err(CreateInputSealError::Unary)?;
        schema
            .suppression_admission_status()
            .map_err(CreateInputSealError::Suppression)?;

        let relations = relations.into_iter().collect::<Vec<_>>();
        let mut relation_paths = BTreeSet::new();
        for relation in &relations {
            let at = match relation {
                CreateInputRelation::UniqueObjectValues { at } => at,
            };
            if !relation_paths.insert(at.clone()) {
                return Err(CreateInputSealError::DuplicateRelation(at.clone()));
            }
            let Some(slot) = schema.get(at) else {
                return Err(CreateInputSealError::MissingRelationTarget(at.clone()));
            };
            if !matches!(slot.space().shape(), Shape::Object { .. }) {
                return Err(CreateInputSealError::RelationTargetNotObject(at.clone()));
            }
        }
        schema.incomplete.create_inputs = Some(CompleteCreateInputFrame { relations });
        Ok(schema)
    }

    #[must_use]
    pub fn get(&self, path: &ConfigPath) -> Option<&ConfigSlot> {
        self.incomplete.slots.get(path)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.incomplete.slots.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.incomplete.slots.is_empty() && self.incomplete.suppress.is_none()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&ConfigPath, &ConfigSlot)> {
        self.incomplete.slots.iter()
    }

    #[must_use]
    pub(crate) fn top_level_slot(&self, key: &str) -> Option<&ConfigSlot> {
        self.iter()
            .find_map(|(path, slot)| (top_level_key(path) == Some(key)).then_some(slot))
    }

    #[must_use]
    pub const fn suppress(&self) -> Option<&SuppressDecl> {
        self.incomplete.suppress.as_ref()
    }

    /// The complete create-input view, only when registration construction
    /// explicitly sealed this schema. The ordinary incomplete frame never
    /// promotes itself from client-visible metadata.
    pub fn create_inputs(&self) -> Result<CreateInputSchema<'_>, CreateInputUnavailable> {
        self.incomplete
            .create_inputs
            .as_ref()
            .map(|complete| CreateInputSchema {
                schema: self,
                complete,
            })
            .ok_or(CreateInputUnavailable {
                stop_line: self.incomplete.create_input_stop_line,
            })
    }

    pub fn confirmed_unary_readiness(&self) -> Result<(), ConfigUnaryReadinessError> {
        for (path, slot) in self.iter() {
            if let Err(dependency) = slot.space().unary_readiness() {
                return Err(ConfigUnaryReadinessError {
                    path: path.clone(),
                    dependency,
                });
            }
        }
        Ok(())
    }

    pub fn suppression_admission_status(&self) -> Result<(), SuppressionAdmissionError> {
        if let Some(declaration) = &self.incomplete.suppress {
            for (reason, rule) in declaration.iter() {
                if let SuppressRule::Equal(CompareTarget::FromConfig(path)) = rule {
                    let Some(slot) = self.get(path) else {
                        return Err(SuppressionAdmissionError::MissingConfigPath {
                            reason: reason.clone(),
                            path: path.clone(),
                        });
                    };
                    if !matches!(
                        slot.space().constraint(),
                        Some(ConfigConstraint::ExactPayloadPath)
                    ) {
                        return Err(SuppressionAdmissionError::TargetConstraintMismatch {
                            reason: reason.clone(),
                            path: path.clone(),
                        });
                    }
                    if slot.snippet().is_some() {
                        return Err(SuppressionAdmissionError::TargetHasSnippet {
                            reason: reason.clone(),
                            path: path.clone(),
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

fn top_level_key(path: &ConfigPath) -> Option<&str> {
    match path.segments() {
        [Segment::Key(key)] => Some(key),
        _ => None,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigRejection {
    NotObject(NotObject),
    Unknown(UnknownField),
    Missing(&'static str),
    Slot {
        slot: &'static str,
        error: ConfigDecodeError,
    },
}

impl fmt::Display for ConfigRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject(rejection) => write!(
                formatter,
                "config must be an object, got {}",
                rejection.actual
            ),
            Self::Unknown(rejection) => {
                write!(
                    formatter,
                    "config.{} is not a registered key",
                    rejection.key
                )
            }
            Self::Missing(slot) => write!(formatter, "config.{slot} is missing"),
            Self::Slot { slot, error } => write!(formatter, "config.{slot}: {error}"),
        }
    }
}

impl Error for ConfigRejection {}

pub(crate) trait SlotKind {
    type Out;
    fn space(&self) -> ConfigSpace;
    fn decode(&self, value: &Value) -> Result<Self::Out, ConfigDecodeError>;
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Interval;

impl SlotKind for Interval {
    type Out = Millis;
    fn space(&self) -> ConfigSpace {
        interval_space(IntervalDomain::Milliseconds)
    }
    fn decode(&self, value: &Value) -> Result<Millis, ConfigDecodeError> {
        interval_millis(value)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NonZeroInterval;

impl SlotKind for NonZeroInterval {
    type Out = NonZeroMillis;
    fn space(&self) -> ConfigSpace {
        interval_space(IntervalDomain::NonZeroMilliseconds)
    }
    fn decode(&self, value: &Value) -> Result<NonZeroMillis, ConfigDecodeError> {
        nonzero_interval_millis(value)
    }
}

pub(crate) fn interval_space(domain: IntervalDomain) -> ConfigSpace {
    ConfigSpace::try_new(
        Shape::Base(BaseShape::Int),
        ConfigConstraint::Interval(domain),
    )
    .expect("an interval constraint has its integer shape")
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct NonZeroIntervalList;

impl SlotKind for NonZeroIntervalList {
    type Out = Box<[NonZeroMillis]>;
    fn space(&self) -> ConfigSpace {
        interval_list_space(IntervalDomain::NonZeroMilliseconds)
    }
    fn decode(&self, value: &Value) -> Result<Box<[NonZeroMillis]>, ConfigDecodeError> {
        interval_items(value)?
            .iter()
            .map(nonzero_interval_millis)
            .collect()
    }
}

pub(crate) fn interval_list_space(domain: IntervalDomain) -> ConfigSpace {
    ConfigSpace::try_new(
        Shape::Array(Box::new(Shape::Base(BaseShape::Int))),
        ConfigConstraint::IntervalList(domain),
    )
    .expect("an interval list constraint has its integer item shape")
}

impl SlotKind for IntegerCount {
    type Out = i64;
    fn space(&self) -> ConfigSpace {
        ConfigSpace::try_new(
            Shape::Base(BaseShape::Int),
            ConfigConstraint::IntegerCount(self.clone()),
        )
        .expect("a count constraint has its integer shape")
    }
    fn decode(&self, value: &Value) -> Result<i64, ConfigDecodeError> {
        self.decode_count(value)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PositiveCount;

impl PositiveCount {
    fn count() -> IntegerCount {
        IntegerCount::try_new(IntegerMinimum::One, None)
            .expect("an unbounded positive integer count is canonical")
    }
}

impl SlotKind for PositiveCount {
    type Out = std::num::NonZeroU64;
    fn space(&self) -> ConfigSpace {
        Self::count().space()
    }
    fn decode(&self, value: &Value) -> Result<std::num::NonZeroU64, ConfigDecodeError> {
        let count = Self::count().decode_count(value)?;
        u64::try_from(count)
            .ok()
            .and_then(std::num::NonZeroU64::new)
            .ok_or(ConfigDecodeError::CountBelowMinimum {
                actual: count,
                minimum: 1,
            })
    }
}

impl SlotKind for ClosedTags {
    type Out = String;
    fn space(&self) -> ConfigSpace {
        ConfigSpace::try_new(
            Shape::Base(BaseShape::String),
            ConfigConstraint::ClosedTags(self.clone()),
        )
        .expect("a closed tag constraint has its string shape")
    }
    fn decode(&self, value: &Value) -> Result<String, ConfigDecodeError> {
        self.decode_tag(value).map(str::to_owned)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Tags(pub(crate) &'static [&'static str]);

impl Tags {
    fn closed(self) -> ClosedTags {
        ClosedTags::try_from_members(self.0.iter().copied())
            .expect("a registered closed tag set is non-empty and unique")
    }
}

impl SlotKind for Tags {
    type Out = String;
    fn space(&self) -> ConfigSpace {
        self.closed().space()
    }
    fn decode(&self, value: &Value) -> Result<String, ConfigDecodeError> {
        self.closed().decode(value)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Spelled<T: 'static> {
    arms: &'static [T],
    spelling: fn(T) -> &'static str,
}

impl<T: Copy> Spelled<T> {
    pub(crate) const fn new(arms: &'static [T], spelling: fn(T) -> &'static str) -> Self {
        Self { arms, spelling }
    }

    fn closed(self) -> ClosedTags {
        ClosedTags::try_from_members(self.arms.iter().map(|arm| (self.spelling)(*arm)))
            .expect("a closed table has non-empty distinct spellings")
    }
}

impl<T: Copy> SlotKind for Spelled<T> {
    type Out = T;
    fn space(&self) -> ConfigSpace {
        self.closed().space()
    }
    fn decode(&self, value: &Value) -> Result<T, ConfigDecodeError> {
        let tag = value.as_str().ok_or(ConfigDecodeError::TagKind {
            actual: value.kind(),
        })?;
        self.arms
            .iter()
            .copied()
            .find(|arm| (self.spelling)(*arm) == tag)
            .ok_or_else(|| ConfigDecodeError::UnknownTag {
                actual: tag.to_owned(),
                allowed: self
                    .arms
                    .iter()
                    .map(|arm| (self.spelling)(*arm).to_owned())
                    .collect(),
            })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Finite;

impl SlotKind for Finite {
    type Out = f64;
    fn space(&self) -> ConfigSpace {
        ConfigSpace::try_new(
            Shape::Base(BaseShape::Float),
            ConfigConstraint::FiniteNumber,
        )
        .expect("the finite-number constraint has its float shape")
    }
    fn decode(&self, value: &Value) -> Result<f64, ConfigDecodeError> {
        let Value::Float(raw) = value else {
            return Err(ConfigDecodeError::FiniteKind {
                actual: value.kind(),
            });
        };
        let number = raw.get();
        if !number.is_finite() {
            return Err(ConfigDecodeError::NonFinite { actual: *raw });
        }
        Ok(number)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct UnitInterval;

impl SlotKind for UnitInterval {
    type Out = f64;
    fn space(&self) -> ConfigSpace {
        ConfigSpace::try_new(
            Shape::Base(BaseShape::Float),
            ConfigConstraint::ClosedUnitInterval,
        )
        .expect("the unit-interval constraint has its float shape")
    }
    fn decode(&self, value: &Value) -> Result<f64, ConfigDecodeError> {
        let Value::Float(raw) = value else {
            return Err(ConfigDecodeError::UnitIntervalKind {
                actual: value.kind(),
            });
        };
        let number = raw.get();
        if !number.is_finite() {
            return Err(ConfigDecodeError::NonFiniteUnitInterval { actual: *raw });
        }
        if !(0.0..=1.0).contains(&number) {
            return Err(ConfigDecodeError::OutsideUnitInterval { actual: *raw });
        }
        Ok(number)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LabelText;

impl SlotKind for LabelText {
    type Out = Label;
    fn space(&self) -> ConfigSpace {
        ConfigSpace::try_new(
            Shape::Base(BaseShape::String),
            ConfigConstraint::CanonicalText(TextDomain::Label),
        )
        .expect("a canonical text constraint has its string shape")
    }
    fn decode(&self, value: &Value) -> Result<Label, ConfigDecodeError> {
        let text = value.as_str().ok_or(ConfigDecodeError::CanonicalTextKind {
            domain: TextDomain::Label,
            actual: value.kind(),
        })?;
        Label::try_from_owned(text.to_owned()).map_err(|source| ConfigDecodeError::InvalidLabel {
            actual: text.to_owned(),
            source,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct HarnessName;

impl SlotKind for HarnessName {
    type Out = AgentHarnessName;
    fn space(&self) -> ConfigSpace {
        ConfigSpace::try_new(
            Shape::Base(BaseShape::String),
            ConfigConstraint::CanonicalText(TextDomain::AgentHarnessName),
        )
        .expect("a canonical text constraint has its string shape")
    }
    fn decode(&self, value: &Value) -> Result<AgentHarnessName, ConfigDecodeError> {
        let text = value.as_str().ok_or(ConfigDecodeError::CanonicalTextKind {
            domain: TextDomain::AgentHarnessName,
            actual: value.kind(),
        })?;
        AgentHarnessName::try_from_normalized(text)
            .map_err(ConfigDecodeError::InvalidAgentHarnessName)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PeerAdapter;

impl SlotKind for PeerAdapter {
    type Out = PeerAdapterName;
    fn space(&self) -> ConfigSpace {
        ConfigSpace::try_new(
            Shape::Base(BaseShape::String),
            ConfigConstraint::CanonicalText(TextDomain::PeerAdapterName),
        )
        .expect("a canonical text constraint has its string shape")
    }
    fn decode(&self, value: &Value) -> Result<PeerAdapterName, ConfigDecodeError> {
        let text = value.as_str().ok_or(ConfigDecodeError::CanonicalTextKind {
            domain: TextDomain::PeerAdapterName,
            actual: value.kind(),
        })?;
        PeerAdapterName::try_new(text).map_err(ConfigDecodeError::InvalidPeerAdapterName)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TypeExpr;

impl SlotKind for TypeExpr {
    type Out = GroundFlow;
    fn space(&self) -> ConfigSpace {
        ConfigSpace::try_new(
            Shape::Array(Box::new(Shape::Any)),
            ConfigConstraint::CanonicalTypeExpr,
        )
        .expect("the type-expression constraint requires no base shape")
    }
    fn decode(&self, value: &Value) -> Result<GroundFlow, ConfigDecodeError> {
        let flow = decode_port_flow(value.clone())
            .map(flow_from_port_type)
            .map_err(ConfigDecodeError::InvalidTypeExpr)?;
        GroundFlow::try_new(flow).map_err(|_| ConfigDecodeError::NonGroundTypeExpr)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BaseStreamTypeExpr;

impl SlotKind for BaseStreamTypeExpr {
    type Out = GroundFlow;

    fn space(&self) -> ConfigSpace {
        ConfigSpace::try_new(
            Shape::Array(Box::new(Shape::Any)),
            ConfigConstraint::CanonicalBaseStream,
        )
        .expect("the base-stream constraint uses the canonical type-expression carrier")
    }

    fn decode(&self, value: &Value) -> Result<GroundFlow, ConfigDecodeError> {
        let flow = TypeExpr.decode(value)?;
        if !matches!(flow.as_flow(), crate::types::Flow::Stream(Shape::Base(_))) {
            return Err(ConfigDecodeError::InvalidTypeExpr(
                circular_protocol::port_type::PortTypeCodecError {
                    detail: "expected a canonical Stream(Base) type expression".into(),
                },
            ));
        }
        Ok(flow)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ExactPath;

impl SlotKind for ExactPath {
    type Out = PayloadPath;
    fn space(&self) -> ConfigSpace {
        ConfigSpace::try_new(
            Shape::Array(Box::new(Shape::Any)),
            ConfigConstraint::ExactPayloadPath,
        )
        .expect("the payload-path constraint requires no base shape")
    }
    fn decode(&self, value: &Value) -> Result<PayloadPath, ConfigDecodeError> {
        payload_path_from_value(value).map_err(ConfigDecodeError::PayloadPath)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Slot<K> {
    name: &'static str,
    kind: K,
}

impl<K: SlotKind> Slot<K> {
    pub(crate) const fn new(name: &'static str, kind: K) -> Self {
        Self { name, kind }
    }

    pub(crate) fn path(&self) -> ConfigPath {
        ConfigPath::root().join_key(self.name)
    }

    pub(crate) fn space(&self) -> ConfigSpace {
        self.kind.space()
    }
}

impl ConfigSchema {
    pub(crate) fn open<'v>(&self, value: &'v Value) -> Result<Fields<'v>, ConfigRejection> {
        let mut fields = Fields::open(value).map_err(ConfigRejection::NotObject)?;
        for (path, _) in self.iter() {
            if let Some(key) = top_level_key(path) {
                fields.skip(key);
            }
        }
        match fields.unknown() {
            Some(unknown) => Err(ConfigRejection::Unknown(unknown)),
            None => Ok(fields),
        }
    }

    fn registered(&self, name: &'static str) -> &ConfigSlot {
        self.top_level_slot(name)
            .unwrap_or_else(|| panic!("factory reads an unregistered config slot: {name}"))
    }

    pub(crate) fn raw<'r, 's: 'r, 'v: 'r>(
        &'s self,
        fields: &mut Fields<'v>,
        name: &'static str,
    ) -> Result<&'r Value, ConfigRejection> {
        match fields.take(name) {
            Some(value) => Ok(value),
            None => match self.registered(name).required() {
                Required::Optional { default } => Ok(default),
                Required::Mandatory | Required::Omittable { .. } => {
                    Err(ConfigRejection::Missing(name))
                }
            },
        }
    }

    pub(crate) fn read<K: SlotKind>(
        &self,
        fields: &mut Fields<'_>,
        slot: &Slot<K>,
    ) -> Result<K::Out, ConfigRejection> {
        let value = self.raw(fields, slot.name)?;
        slot.kind
            .decode(value)
            .map_err(|error| ConfigRejection::Slot {
                slot: slot.name,
                error,
            })
    }
}

/// Borrowed view of the registration-sealed create-input contract.
#[derive(Clone, Copy, Debug)]
pub struct CreateInputSchema<'a> {
    schema: &'a ConfigSchema,
    complete: &'a CompleteCreateInputFrame,
}

impl<'a> CreateInputSchema<'a> {
    pub fn slots(&self) -> impl ExactSizeIterator<Item = (&'a ConfigPath, &'a ConfigSlot)> + 'a {
        self.schema.iter()
    }

    #[must_use]
    pub fn relations(&self) -> &'a [CreateInputRelation] {
        &self.complete.relations
    }

    /// Registry-authored initial draft. Optional defaults are copied from the
    /// slots unless the registration preserves omission. A mandatory path stays
    /// in `missing` — the author must write it — and the draft holds it only at
    /// the registration's starting value, never at a value a client
    /// invents. Empty schemas retain the existing canonical `Null` config.
    #[must_use]
    pub fn draft(&self) -> CreateInputDraft {
        if self.schema.incomplete.slots.is_empty() {
            return CreateInputDraft {
                config: Value::Null,
                missing: Box::new([]),
            };
        }
        let mut config = Vec::new();
        let mut missing = Vec::new();
        for (path, slot) in self.schema.iter() {
            let key = top_level_key(path).expect("sealed create-input path is a top-level key");
            match slot.required() {
                Required::Mandatory => {
                    missing.push(path.clone());
                    if let Some(start) = slot.start() {
                        config.push((key.to_owned(), start.clone()));
                    }
                }
                Required::Omittable { .. } => {}
                Required::Optional { .. } if slot.preserve_omission => {}
                Required::Optional { default } => config.push((key.to_owned(), default.clone())),
            }
        }
        CreateInputDraft {
            config: Value::object(config).expect("sealed slot paths are unique top-level keys"),
            missing: missing.into_boxed_slice(),
        }
    }

    /// Admit and canonicalize one completed draft entirely through producer
    /// semantics. Unknown fields, missing mandatory slots, unary constraints,
    /// snippet parsing, and closed relation arms are all checked here.
    pub fn admit(&self, value: &Value) -> Result<AdmittedCreateConfig, CreateInputAdmissionError> {
        if self.schema.incomplete.slots.is_empty() {
            return match value {
                Value::Null => Ok(AdmittedCreateConfig { value: Value::Null }),
                _ => Err(CreateInputAdmissionError::EmptySchemaRequiresNull),
            };
        }

        let mut fields = self.schema.open(value)?;

        let mut admitted = Vec::with_capacity(self.schema.len());
        for (path, slot) in self.schema.iter() {
            let key = top_level_key(path).expect("sealed create-input path is a top-level key");
            let field = match fields.take(key) {
                Some(value) => value.clone(),
                None => match slot.required() {
                    Required::Mandatory => {
                        return Err(CreateInputAdmissionError::MissingMandatory(path.clone()));
                    }
                    Required::Omittable { .. } => continue,
                    Required::Optional { .. } if slot.preserve_omission => continue,
                    Required::Optional { default } => default.clone(),
                },
            };
            match slot.space().accepts(&field) {
                Ok(true) => {}
                Ok(false) => {
                    return Err(CreateInputAdmissionError::OutsideSpace {
                        path: path.clone(),
                        space: Some(slot.space().clone()),
                    });
                }
                Err(dependency) => {
                    return Err(CreateInputAdmissionError::UnresolvedConstraint {
                        path: path.clone(),
                        dependency,
                    });
                }
            }
            if let Some(snippet) = slot.snippet() {
                let bindings = snippet
                    .inlets()
                    .iter()
                    .map(|inlet| inlet.as_str().to_owned())
                    .chain(
                        snippet
                            .mode()
                            .implicit_bindings()
                            .iter()
                            .map(|name| (*name).to_owned()),
                    )
                    .collect::<BTreeSet<_>>();
                circular_expr::snippet::from_config_value(&field, &bindings, snippet.mode())
                    .map_err(|error| CreateInputAdmissionError::Snippet {
                        path: path.clone(),
                        detail: error.to_string(),
                    })?;
            }
            admitted.push((key.to_owned(), field));
        }
        let admitted = Value::object(admitted).expect("sealed config keys are unique");

        for relation in &self.complete.relations {
            match relation {
                CreateInputRelation::UniqueObjectValues { at } => {
                    let key = top_level_key(at)
                        .expect("sealed relation target is a top-level config key");
                    let object = admitted
                        .as_object()
                        .and_then(|root| root.get(key))
                        .and_then(Value::as_object)
                        .expect("sealed relation target has object shape");
                    let values = object.iter().collect::<Vec<_>>();
                    for (index, (name, value)) in values.iter().enumerate() {
                        if let Some((prior, _)) =
                            values[..index].iter().find(|(_, prior)| *prior == *value)
                        {
                            return Err(CreateInputAdmissionError::DuplicateObjectValue {
                                path: at.clone(),
                                first: (*prior).to_owned(),
                                second: (*name).to_owned(),
                            });
                        }
                    }
                }
            }
        }
        Ok(AdmittedCreateConfig { value: admitted })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreateInputDraft {
    config: Value,
    missing: Box<[ConfigPath]>,
}

impl CreateInputDraft {
    #[must_use]
    pub const fn config(&self) -> &Value {
        &self.config
    }

    #[must_use]
    pub const fn missing(&self) -> &[ConfigPath] {
        &self.missing
    }

    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.missing.is_empty()
    }
}

/// Config value carrying proof that the sealed registry admission accepted it.
#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedCreateConfig {
    value: Value,
}

impl AdmittedCreateConfig {
    #[must_use]
    pub const fn value(&self) -> &Value {
        &self.value
    }

    #[must_use]
    pub fn into_value(self) -> Value {
        self.value
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreateInputUnavailable {
    stop_line: Option<&'static str>,
}

impl CreateInputUnavailable {
    #[must_use]
    pub const fn stop_line(self) -> Option<&'static str> {
        self.stop_line
    }
}

impl fmt::Display for CreateInputUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "registration ConfigSchema has no sealed complete create-input admission contract",
        )?;
        if let Some(stop_line) = self.stop_line {
            write!(formatter, ": {stop_line}")?;
        }
        Ok(())
    }
}

impl Error for CreateInputUnavailable {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CreateInputSealError {
    DuplicatePath(DuplicateConfigPath),
    UnsupportedPath(ConfigPath),
    DuplicateSnippetInlet { path: ConfigPath, inlet: String },
    Unary(ConfigUnaryReadinessError),
    Suppression(SuppressionAdmissionError),
    DuplicateRelation(ConfigPath),
    MissingRelationTarget(ConfigPath),
    RelationTargetNotObject(ConfigPath),
}

impl fmt::Display for CreateInputSealError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicatePath(error) => error.fmt(formatter),
            Self::UnsupportedPath(path) => write!(
                formatter,
                "create-input subset currently requires one top-level key, got {path}"
            ),
            Self::DuplicateSnippetInlet { path, inlet } => write!(
                formatter,
                "create-input snippet at {path} repeats inlet {inlet:?}"
            ),
            Self::Unary(error) => error.fmt(formatter),
            Self::Suppression(error) => error.fmt(formatter),
            Self::DuplicateRelation(path) => {
                write!(formatter, "create-input relation repeats target {path}")
            }
            Self::MissingRelationTarget(path) => {
                write!(formatter, "create-input relation target {path} has no slot")
            }
            Self::RelationTargetNotObject(path) => write!(
                formatter,
                "create-input unique-values target {path} is not an object"
            ),
        }
    }
}

impl Error for CreateInputSealError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CreateInputAdmissionError {
    EmptySchemaRequiresNull,
    RootNotObject,
    UnknownField(String),
    MissingMandatory(ConfigPath),
    OutsideSpace {
        path: ConfigPath,
        space: Option<ConfigSpace>,
    },
    UnresolvedConstraint {
        path: ConfigPath,
        dependency: UnresolvedConfigConstraint,
    },
    Snippet {
        path: ConfigPath,
        detail: String,
    },
    DuplicateObjectValue {
        path: ConfigPath,
        first: String,
        second: String,
    },
}

impl fmt::Display for CreateInputAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySchemaRequiresNull => {
                formatter.write_str("empty create-input schema admits only canonical Null config")
            }
            Self::RootNotObject => formatter.write_str("create-input config root is not an object"),
            Self::UnknownField(field) => {
                write!(formatter, "unknown config field {}", Quoted(field))
            }
            Self::MissingMandatory(path) => {
                write!(formatter, "mandatory config input {path} is missing")
            }
            Self::OutsideSpace { path, space } => {
                write!(
                    formatter,
                    "config input {path} is outside its declared space"
                )?;
                match space {
                    Some(space) => write!(formatter, "; {}", space.allowed()),
                    None => Ok(()),
                }
            }
            Self::UnresolvedConstraint { path, dependency } => write!(
                formatter,
                "config input {path} has unresolved constraint: {dependency}"
            ),
            Self::Snippet { path, detail } => {
                let detail = detail.strip_prefix("ConfigRejected: ").unwrap_or(detail);
                write!(formatter, "config snippet {path} was rejected: {detail}")
            }
            Self::DuplicateObjectValue {
                path,
                first,
                second,
            } => write!(
                formatter,
                "config object {path} maps {} and {} to the same value",
                Quoted(first),
                Quoted(second)
            ),
        }
    }
}

impl Error for CreateInputAdmissionError {}

impl CreateInputAdmissionError {
    #[must_use]
    pub const fn outside_space(path: ConfigPath) -> Self {
        Self::OutsideSpace { path, space: None }
    }
}

impl From<ConfigRejection> for CreateInputAdmissionError {
    fn from(rejection: ConfigRejection) -> Self {
        let path = |slot: &str| ConfigPath::root().join_key(slot);
        match rejection {
            ConfigRejection::NotObject(_) => Self::RootNotObject,
            ConfigRejection::Unknown(unknown) => Self::UnknownField(unknown.key),
            ConfigRejection::Missing(slot) => Self::MissingMandatory(path(slot)),
            ConfigRejection::Slot { slot, .. } => Self::outside_space(path(slot)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DuplicateConfigPath {
    path: ConfigPath,
}

impl DuplicateConfigPath {
    #[must_use]
    pub const fn path(&self) -> &ConfigPath {
        &self.path
    }
}

impl fmt::Display for DuplicateConfigPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "duplicate config path {}", self.path)
    }
}

impl Error for DuplicateConfigPath {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigUnaryReadinessError {
    path: ConfigPath,
    dependency: UnresolvedConfigConstraint,
}

impl ConfigUnaryReadinessError {
    #[must_use]
    pub const fn path(&self) -> &ConfigPath {
        &self.path
    }

    #[must_use]
    pub const fn dependency(&self) -> UnresolvedConfigConstraint {
        self.dependency
    }
}

impl fmt::Display for ConfigUnaryReadinessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "config slot {} has unresolved unary dependency: {}",
            self.path, self.dependency
        )
    }
}

impl Error for ConfigUnaryReadinessError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.dependency)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SuppressionAdmissionError {
    MissingConfigPath { reason: Name, path: ConfigPath },
    TargetConstraintMismatch { reason: Name, path: ConfigPath },
    TargetHasSnippet { reason: Name, path: ConfigPath },
}

impl SuppressionAdmissionError {
    #[must_use]
    pub const fn reason(&self) -> &Name {
        match self {
            Self::MissingConfigPath { reason, .. }
            | Self::TargetConstraintMismatch { reason, .. }
            | Self::TargetHasSnippet { reason, .. } => reason,
        }
    }

    #[must_use]
    pub const fn path(&self) -> &ConfigPath {
        match self {
            Self::MissingConfigPath { path, .. }
            | Self::TargetConstraintMismatch { path, .. }
            | Self::TargetHasSnippet { path, .. } => path,
        }
    }

    #[must_use]
    pub const fn unresolved_dependency(&self) -> Option<UnresolvedConfigConstraint> {
        match self {
            Self::MissingConfigPath { .. }
            | Self::TargetConstraintMismatch { .. }
            | Self::TargetHasSnippet { .. } => None,
        }
    }
}

impl fmt::Display for SuppressionAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingConfigPath { reason, path } => write!(
                formatter,
                "suppression reason `{reason}` refers to missing config path {path}"
            ),
            Self::TargetConstraintMismatch { reason, path } => write!(
                formatter,
                "suppression reason `{reason}` requires ExactPayloadPath at config path {path}"
            ),
            Self::TargetHasSnippet { reason, path } => write!(
                formatter,
                "suppression reason `{reason}` cannot use snippet config path {path}"
            ),
        }
    }
}

impl Error for SuppressionAdmissionError {}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_expr::Segment;

    fn number_space(constraint: ConfigConstraint) -> ConfigSpace {
        let shape = constraint
            .required_base_shape()
            .expect("a numeric constraint requires a shape");
        ConfigSpace::try_new(Shape::Base(shape), constraint)
            .expect("test constraint is an executable number constraint")
    }

    fn mandatory_slot() -> ConfigSlot {
        mandatory_slot_in(
            ConfigSpace::unconstrained(Shape::Base(BaseShape::String)),
            None,
        )
    }

    fn mandatory_slot_in(space: ConfigSpace, snippet: Option<SnippetSlot>) -> ConfigSlot {
        ConfigSlot::try_new(space, Required::Mandatory, snippet)
            .expect("mandatory slot has no default to reject")
    }

    #[test]
    fn payload_root_is_the_empty_exact_path_and_fixed_preserves_segments() {
        let whole = PayloadRoot::path();
        assert!(whole.segments().is_empty());

        let selected = PayloadRoot::path().join_key("items").join_index(2_u64);
        let target = CompareTarget::Fixed(selected.clone());
        assert_eq!(target, CompareTarget::Fixed(selected));
    }

    #[test]
    fn integer_bounds_only_reject_negatives_now_that_the_width_is_integral() {
        assert_eq!(IntegerBound::try_new(-1), Err(IntegerBoundError::Negative));
        assert_eq!(IntegerBound::try_new(0).unwrap().get(), 0);
        assert_eq!(IntegerBound::try_new(9).unwrap().get(), 9);
    }

    #[test]
    fn decode_errors_preserve_each_family_failure() {
        let interval = ConfigConstraint::Interval(IntervalDomain::NonZeroMilliseconds);
        assert_eq!(
            interval.judge(&Value::float(1.0)),
            Err(ConfigDecodeError::IntervalKind {
                actual: ValueKind::Float
            })
        );
        assert_eq!(
            interval.judge(&Value::Int(-1)),
            Err(ConfigDecodeError::NegativeInterval { actual: -1 })
        );
        assert_eq!(
            interval.judge(&Value::UInt(0)),
            Err(ConfigDecodeError::ZeroInterval)
        );

        let count = ConfigConstraint::IntegerCount(
            IntegerCount::try_new(IntegerMinimum::One, Some(IntegerBound::try_new(3).unwrap()))
                .unwrap(),
        );
        assert_eq!(
            count.judge(&Value::UInt(2)),
            Err(ConfigDecodeError::CountKind {
                actual: ValueKind::UInt
            })
        );
        assert_eq!(
            count.judge(&Value::Int(0)),
            Err(ConfigDecodeError::CountBelowMinimum {
                actual: 0,
                minimum: 1
            })
        );
        assert_eq!(
            count.judge(&Value::Int(4)),
            Err(ConfigDecodeError::CountAboveMaximum {
                actual: 4,
                maximum: 3
            })
        );

        assert_eq!(
            ConfigConstraint::FiniteNumber.judge(&Value::Int(1)),
            Err(ConfigDecodeError::FiniteKind {
                actual: ValueKind::Int
            })
        );
        assert_eq!(
            ConfigConstraint::FiniteNumber.judge(&Value::float(f64::INFINITY)),
            Err(ConfigDecodeError::NonFinite {
                actual: FloatValue::new(f64::INFINITY)
            })
        );
        assert_eq!(
            ConfigConstraint::ClosedUnitInterval.judge(&Value::Int(1)),
            Err(ConfigDecodeError::UnitIntervalKind {
                actual: ValueKind::Int
            })
        );
        assert_eq!(
            ConfigConstraint::ClosedUnitInterval.judge(&Value::float(f64::NAN)),
            Err(ConfigDecodeError::NonFiniteUnitInterval {
                actual: FloatValue::new(f64::NAN)
            })
        );
        assert_eq!(
            ConfigConstraint::ClosedUnitInterval.judge(&Value::float(1.5)),
            Err(ConfigDecodeError::OutsideUnitInterval {
                actual: FloatValue::new(1.5)
            })
        );

        let tags = ConfigConstraint::ClosedTags(ClosedTags::try_from_members(["red"]).unwrap());
        assert_eq!(
            tags.judge(&Value::Bool(true)),
            Err(ConfigDecodeError::TagKind {
                actual: ValueKind::Bool
            })
        );
        assert_eq!(
            tags.judge(&Value::string("blue")),
            Err(ConfigDecodeError::UnknownTag {
                actual: "blue".to_owned(),
                allowed: vec!["red".to_owned()],
            })
        );

        let label = ConfigConstraint::CanonicalText(TextDomain::Label);
        assert_eq!(
            label.judge(&Value::Bool(true)),
            Err(ConfigDecodeError::CanonicalTextKind {
                domain: TextDomain::Label,
                actual: ValueKind::Bool
            })
        );
        assert_eq!(
            label.judge(&Value::string("")),
            Err(ConfigDecodeError::InvalidLabel {
                actual: String::new(),
                source: DisplayTextError::EmptyLabel
            })
        );

        assert!(matches!(
            ConfigConstraint::CanonicalTypeExpr.judge(&Value::string("Stream(String)")),
            Err(ConfigDecodeError::InvalidTypeExpr(_))
        ));
        let variable = circular_protocol::port_type::encode_port_flow(
            &circular_protocol::port_type::PortFlow::Stream(
                circular_protocol::port_type::PortShape::Variable("T".to_owned()),
            ),
        )
        .unwrap();
        assert_eq!(
            ConfigConstraint::CanonicalTypeExpr.judge(&variable),
            Err(ConfigDecodeError::NonGroundTypeExpr)
        );
        assert_eq!(
            ConfigConstraint::ExactPayloadPath.judge(&Value::string("items[2]")),
            Err(ConfigDecodeError::PayloadPath(
                PayloadPathError::NotAnArray {
                    got: ValueKind::String
                }
            ))
        );
        assert_eq!(
            ConfigConstraint::CanonicalText(TextDomain::Name).judge(&Value::string("name")),
            Err(ConfigDecodeError::Unresolved(
                UnresolvedConfigConstraint::NameParser
            ))
        );
    }

    #[test]
    fn integer_count_rejects_maximum_below_minimum() {
        let zero = IntegerBound::try_new(0).unwrap();
        let error = IntegerCount::try_new(IntegerMinimum::One, Some(zero))
            .expect_err("maximum zero is below minimum one");
        assert_eq!(error.minimum(), IntegerMinimum::One);
        assert_eq!(error.maximum(), zero);
    }

    #[test]
    fn unbounded_integer_count_accepts_every_nonnegative_integer() {
        let count = IntegerCount::try_new(IntegerMinimum::Zero, None).unwrap();
        let constraint = ConfigConstraint::IntegerCount(count);

        for accepted in [0, 1, i64::MAX] {
            assert_eq!(constraint.accepts_value(&Value::Int(accepted)), Ok(true));
        }
        assert_eq!(constraint.accepts_value(&Value::Int(-1)), Ok(false));
        assert_eq!(constraint.accepts_value(&Value::float(1.0)), Ok(false));
    }

    #[test]
    fn closed_tags_reject_empty_and_duplicates_and_iterate_in_utf8_order() {
        assert_eq!(
            ClosedTags::try_from_members(std::iter::empty::<String>()),
            Err(ClosedTagsError::Empty)
        );
        assert_eq!(
            ClosedTags::try_from_members(["alpha", "beta", "alpha"]),
            Err(ClosedTagsError::DuplicateMember("alpha".into()))
        );

        let tags = ClosedTags::try_from_members(["z", "a", "é"]).unwrap();
        assert_eq!(tags.iter().collect::<Vec<_>>(), ["a", "z", "é"]);
        assert_eq!(tags.len(), 3);
        assert!(!tags.is_empty());

        let constraint = ConfigConstraint::ClosedTags(tags);
        assert_eq!(
            constraint.accepts_value(&Value::String("z".into())),
            Ok(true)
        );
        assert_eq!(
            constraint.accepts_value(&Value::String("Z".into())),
            Ok(false)
        );
        assert_eq!(constraint.accepts_value(&Value::float(0.0)), Ok(false));
    }

    #[test]
    fn canonical_label_delegates_to_the_existing_label_constructor() {
        let constraint = ConfigConstraint::CanonicalText(TextDomain::Label);
        for accepted in ["label", "label with spaces"] {
            assert_eq!(
                constraint.accepts_value(&Value::String(accepted.into())),
                Ok(true)
            );
        }
        for rejected in ["", " leading", "trailing ", "two\nlines", "two\rlines"] {
            assert_eq!(
                constraint.accepts_value(&Value::String(rejected.into())),
                Ok(false)
            );
        }
        assert_eq!(constraint.accepts_value(&Value::Bool(true)), Ok(false));
    }

    #[test]
    fn canonical_agent_harness_name_delegates_to_the_runtime_wrapper() {
        let constraint = ConfigConstraint::CanonicalText(TextDomain::AgentHarnessName);
        assert_eq!(constraint.unresolved_dependency(), None);
        for text in ["unregistered-harness", " ", " leading", "two\nlines"] {
            assert_eq!(
                HarnessName.decode(&Value::String(text.into())),
                Ok(AgentHarnessName::try_from_normalized(text).unwrap())
            );
            assert_eq!(constraint.judge(&Value::String(text.into())), Ok(()));
        }
        assert_eq!(
            constraint.judge(&Value::String(String::new())),
            Err(ConfigDecodeError::InvalidAgentHarnessName(EmptyAgentName))
        );
        assert_eq!(constraint.accepts_value(&Value::Bool(true)), Ok(false));
    }

    #[test]
    fn every_parser_or_value_encoding_stop_line_is_explicitly_unresolved() {
        let pending_text = [
            (TextDomain::Name, UnresolvedConfigConstraint::NameParser),
            (
                TextDomain::ToolName,
                UnresolvedConfigConstraint::ToolNameParser,
            ),
            (
                TextDomain::ModelProviderName,
                UnresolvedConfigConstraint::ModelProviderNameParser,
            ),
            (
                TextDomain::ModelName,
                UnresolvedConfigConstraint::ModelNameParser,
            ),
        ];
        for (domain, unresolved) in pending_text {
            let constraint = ConfigConstraint::CanonicalText(domain);
            assert_eq!(
                constraint.accepts_value(&Value::String("candidate".into())),
                Err(unresolved)
            );
            let space = ConfigSpace::try_new(Shape::Base(BaseShape::String), constraint)
                .expect("pending canonical text remains a typed String carrier");
            assert_eq!(space.unary_readiness(), Err(unresolved));
            assert_eq!(
                space.accepts(&Value::String("candidate".into())),
                Err(unresolved)
            );
            assert_eq!(
                ConfigSlot::try_new(
                    space.clone(),
                    Required::Optional {
                        default: Value::String("candidate".into()),
                    },
                    None,
                ),
                Err(ConfigSlotError::DefaultConstraintUnresolved(unresolved))
            );
            mandatory_slot_in(space, None);
            assert_eq!(
                ConfigSpace::try_new(
                    Shape::Base(BaseShape::Float),
                    ConfigConstraint::CanonicalText(domain),
                ),
                Err(ConfigSpaceError::IncompatibleShape {
                    required: BaseShape::String,
                })
            );
        }

        let type_expr = ConfigConstraint::CanonicalTypeExpr;
        assert_eq!(
            type_expr.accepts_value(&Value::String("Number".into())),
            Ok(false)
        );
        let string_flow = circular_protocol::port_type::encode_port_flow(
            &circular_protocol::port_type::PortFlow::Stream(
                circular_protocol::port_type::PortShape::Base(BaseShape::String),
            ),
        )
        .expect("canonical String Flow");
        assert_eq!(type_expr.accepts_value(&string_flow), Ok(true));
        let variable = circular_protocol::port_type::encode_port_flow(
            &circular_protocol::port_type::PortFlow::Stream(
                circular_protocol::port_type::PortShape::Variable("T".to_owned()),
            ),
        )
        .expect("canonical variable Flow");
        assert_eq!(type_expr.accepts_value(&variable), Ok(false));
        let type_space = ConfigSpace::try_new(Shape::Any, type_expr)
            .expect("known type-expression carrier uses the protocol-owned closed codec");
        assert_eq!(type_space.unary_readiness(), Ok(()));
        mandatory_slot_in(type_space, None);

        let path_space =
            ConfigSpace::try_new(Shape::Any, ConfigConstraint::ExactPayloadPath).unwrap();
        assert_eq!(path_space.unary_readiness(), Ok(()));
    }

    #[test]
    fn the_kind_of_a_segment_is_the_discriminator() {
        let as_key = payload_path_from_value(&Value::array([Value::string("0")])).unwrap();
        let as_index = payload_path_from_value(&Value::array([Value::int(0)])).unwrap();

        assert_ne!(as_key, as_index);
        assert_eq!(as_key.segments(), [Segment::key("0")]);
        assert_eq!(as_index.segments(), [Segment::index(0)]);
    }

    #[test]
    fn the_constraint_agrees_with_the_decoder() {
        let constraint = ConfigConstraint::ExactPayloadPath;
        for accepted in [
            Value::Array(Vec::new()),
            Value::array([Value::string("a")]),
            Value::array([Value::int(0)]),
        ] {
            assert_eq!(
                constraint.accepts_value(&accepted),
                Ok(true),
                "{accepted:?}"
            );
        }
        for rejected in [
            Value::string("a.b"),
            Value::Null,
            Value::array([Value::int(-1)]),
        ] {
            assert_eq!(
                constraint.accepts_value(&rejected),
                Ok(false),
                "{rejected:?}"
            );
        }
    }

    #[test]
    fn config_space_rejects_constraint_shape_mismatches_and_accepts_any_shape() {
        let tags = || ClosedTags::try_from_members(["one"]).unwrap();
        assert_eq!(
            ConfigSpace::try_new(
                Shape::Base(BaseShape::String),
                ConfigConstraint::FiniteNumber,
            ),
            Err(ConfigSpaceError::IncompatibleShape {
                required: BaseShape::Float,
            })
        );
        assert_eq!(
            ConfigSpace::try_new(
                Shape::Base(BaseShape::Float),
                ConfigConstraint::ClosedTags(tags()),
            ),
            Err(ConfigSpaceError::IncompatibleShape {
                required: BaseShape::String,
            })
        );
        assert_eq!(
            ConfigSpace::try_new(
                Shape::Array(Box::new(Shape::Any)),
                ConfigConstraint::ClosedTags(tags()),
            ),
            Err(ConfigSpaceError::IncompatibleShape {
                required: BaseShape::String,
            })
        );

        let any = ConfigSpace::try_new(Shape::Any, ConfigConstraint::FiniteNumber).unwrap();
        assert_eq!(any.accepts(&Value::float(1.0)), Ok(true));
        assert_eq!(any.accepts(&Value::String("1".into())), Ok(false));
    }

    #[test]
    fn confirmed_unary_readiness_reports_the_first_exact_path_and_dependency() {
        let ready_path = ConfigPath::root().join_key("a_ready");
        let pending_path = ConfigPath::root().join_key("b_pending");
        let pending_space = ConfigSpace::try_new(
            Shape::Base(BaseShape::String),
            ConfigConstraint::CanonicalText(TextDomain::Name),
        )
        .expect("pending Name parser remains a typed carrier");
        let schema = ConfigSchema::try_from_parts(
            [
                (ready_path, mandatory_slot()),
                (pending_path.clone(), mandatory_slot_in(pending_space, None)),
            ],
            None,
        )
        .unwrap();

        let error = schema
            .confirmed_unary_readiness()
            .expect_err("Name parser is intentionally unresolved");
        assert_eq!(error.path(), &pending_path);
        assert_eq!(error.dependency(), UnresolvedConfigConstraint::NameParser);

        let ready = ConfigSchema::try_from_parts(
            [(ConfigPath::root().join_key("ready"), mandatory_slot())],
            None,
        )
        .unwrap();
        assert_eq!(ready.confirmed_unary_readiness(), Ok(()));
    }

    #[test]
    fn optional_defaults_must_pass_shape_and_constraint() {
        let bool_space = ConfigSpace::unconstrained(Shape::Base(BaseShape::Bool));
        assert!(
            ConfigSlot::try_new(
                bool_space.clone(),
                Required::Optional {
                    default: Value::Bool(false),
                },
                None,
            )
            .is_ok()
        );
        assert_eq!(
            ConfigSlot::try_new(
                bool_space,
                Required::Optional {
                    default: Value::float(0.0),
                },
                None,
            ),
            Err(ConfigSlotError::DefaultOutsideSpace)
        );

        let unit = number_space(ConfigConstraint::ClosedUnitInterval);
        assert_eq!(
            ConfigSlot::try_new(
                unit,
                Required::Optional {
                    default: Value::float(2.0),
                },
                None,
            ),
            Err(ConfigSlotError::DefaultOutsideSpace)
        );
    }

    #[test]
    fn optional_snippet_defaults_fail_closed_until_the_canonical_parser_exists() {
        let snippet = SnippetSlot::new(
            EvalMode::Predicate,
            vec![PortId::try_new("event").unwrap()].into_boxed_slice(),
        );
        let error = ConfigSlot::try_new(
            ConfigSpace::unconstrained(Shape::Base(BaseShape::String)),
            Required::Optional {
                default: Value::String("true".into()),
            },
            Some(snippet.clone()),
        )
        .expect_err("snippet default cannot skip parser/mode/binding validation");
        assert_eq!(error, ConfigSlotError::SnippetDefaultValidationUnavailable);

        let mandatory = ConfigSlot::try_new(
            ConfigSpace::unconstrained(Shape::Base(BaseShape::String)),
            Required::Mandatory,
            Some(snippet.clone()),
        )
        .expect("mandatory snippet has no default to pre-validate");
        assert_eq!(mandatory.snippet(), Some(&snippet));
    }

    #[test]
    fn suppression_declaration_is_nonempty_unique_and_sorted_by_reason() {
        assert_eq!(
            SuppressDecl::try_from_entries(std::iter::empty()),
            Err(SuppressDeclError::Empty)
        );

        let duplicate = Name::from_static("unchanged");
        assert_eq!(
            SuppressDecl::try_from_entries([
                (duplicate.clone(), SuppressRule::NamedOnly),
                (
                    duplicate.clone(),
                    SuppressRule::Equal(CompareTarget::Fixed(PayloadRoot::path())),
                ),
            ]),
            Err(SuppressDeclError::DuplicateReason(duplicate))
        );

        let declaration = SuppressDecl::try_from_entries([
            (Name::from_static("z_reason"), SuppressRule::NamedOnly),
            (
                Name::from_static("a_reason"),
                SuppressRule::Equal(CompareTarget::Fixed(PayloadRoot::path())),
            ),
        ])
        .unwrap();
        assert_eq!(declaration.len(), 2);
        assert!(!declaration.is_empty());
        assert_eq!(
            declaration
                .iter()
                .map(|(reason, _)| reason.as_str())
                .collect::<Vec<_>>(),
            ["a_reason", "z_reason"]
        );
    }

    #[test]
    fn suppression_is_schema_level_and_exists_even_without_config_slots() {
        let declaration = SuppressDecl::try_from_entries([(
            Name::from_static("unchanged"),
            SuppressRule::Equal(CompareTarget::Fixed(PayloadRoot::path())),
        )])
        .unwrap();
        let schema = ConfigSchema::try_from_parts([], Some(declaration.clone())).unwrap();

        assert_eq!(schema.len(), 0);
        assert!(!schema.is_empty());
        assert_eq!(schema.suppress(), Some(&declaration));
        assert_eq!(schema.suppression_admission_status(), Ok(()));
    }

    #[test]
    fn from_config_suppression_is_admitted_now_that_the_decoder_exists() {
        let selector = ConfigPath::root().join_key("selector");
        let declaration = SuppressDecl::try_from_entries([(
            Name::from_static("unchanged"),
            SuppressRule::Equal(CompareTarget::FromConfig(selector.clone())),
        )])
        .unwrap();
        let selector_space = ConfigSpace::try_new(Shape::Any, ConfigConstraint::ExactPayloadPath)
            .expect("exact-path carrier is preserved before its Value decoder exists");
        let schema = ConfigSchema::try_from_parts(
            [(selector.clone(), mandatory_slot_in(selector_space, None))],
            Some(declaration.clone()),
        )
        .expect("incomplete frame preserves the confirmed FromConfig carrier");

        assert_eq!(schema.suppress(), Some(&declaration));
        assert_eq!(
            schema.suppression_admission_status(),
            Ok(()),
            "a FromConfig suppression that passes the three qualifications is approved"
        );
        let _ = selector;
    }

    #[test]
    fn from_config_suppression_checks_target_qualification_before_decoder_readiness() {
        let selector = ConfigPath::root().join_key("selector");
        let declaration = || {
            SuppressDecl::try_from_entries([(
                Name::from_static("unchanged"),
                SuppressRule::Equal(CompareTarget::FromConfig(selector.clone())),
            )])
            .unwrap()
        };

        let missing = ConfigSchema::try_from_parts([], Some(declaration())).unwrap();
        assert_eq!(
            missing.suppression_admission_status(),
            Err(SuppressionAdmissionError::MissingConfigPath {
                reason: Name::from_static("unchanged"),
                path: selector.clone(),
            })
        );

        let wrong_constraint = ConfigSchema::try_from_parts(
            [(selector.clone(), mandatory_slot())],
            Some(declaration()),
        )
        .unwrap();
        assert_eq!(
            wrong_constraint.suppression_admission_status(),
            Err(SuppressionAdmissionError::TargetConstraintMismatch {
                reason: Name::from_static("unchanged"),
                path: selector.clone(),
            })
        );

        let exact_space = ConfigSpace::try_new(Shape::Any, ConfigConstraint::ExactPayloadPath)
            .expect("exact-path carrier is preserved before its Value decoder exists");
        let snippet = SnippetSlot::new(EvalMode::Transform, Vec::new().into_boxed_slice());
        let snippet_target = ConfigSchema::try_from_parts(
            [(
                selector.clone(),
                mandatory_slot_in(exact_space, Some(snippet)),
            )],
            Some(declaration()),
        )
        .unwrap();
        assert_eq!(
            snippet_target.suppression_admission_status(),
            Err(SuppressionAdmissionError::TargetHasSnippet {
                reason: Name::from_static("unchanged"),
                path: selector,
            })
        );
    }

    #[test]
    fn duplicate_config_paths_are_rejected_instead_of_overwritten() {
        let path = ConfigPath::root().join_key("predicate");
        let error = ConfigSchema::try_from_parts(
            [
                (path.clone(), mandatory_slot()),
                (path.clone(), mandatory_slot()),
            ],
            None,
        )
        .expect_err("duplicate path must not be overwritten");

        assert_eq!(error, DuplicateConfigPath { path });
    }

    #[test]
    fn the_count_constraint_agrees_with_the_shape_layer_on_floats() {
        let counted = number_space(ConfigConstraint::IntegerCount(
            IntegerCount::try_new(IntegerMinimum::Zero, None)
                .expect("zero minimum with no maximum is a valid count"),
        ));

        assert_eq!(counted.accepts(&Value::int(64)), Ok(true));
        assert_eq!(counted.accepts(&Value::float(64.0)), Ok(false));
    }
}

pub fn config_rejection(
    actor: impl fmt::Display,
    path: &str,
    value: Option<&Value>,
    reason: impl fmt::Display,
) -> String {
    fn describe(path: &str, value: Option<&Value>, out: &mut Vec<String>) {
        match value {
            Some(Value::Object(object)) if !object.is_empty() => {
                for (key, value) in object.clone().into_map() {
                    describe(&format!("{path}{}", Segment::Key(key)), Some(&value), out);
                }
            }
            Some(Value::Array(items)) if !items.is_empty() => {
                for (index, value) in (0_u64..).zip(items.iter()) {
                    describe(
                        &format!("{path}{}", Segment::Index(index)),
                        Some(value),
                        out,
                    );
                }
            }
            Some(value) => out.push(format!("{path} = {}", spelling::ValueText(value))),
            None => out.push(format!("{path} = <missing>")),
        }
    }
    let mut context = Vec::new();
    describe(path, value, &mut context);
    let reason = reason.to_string();
    let reason = reason.strip_prefix("ConfigRejected: ").unwrap_or(&reason);
    format!(
        "ConfigRejected: actor `{actor}`; {}; {reason}",
        context.join("; ")
    )
}

impl CreateInputAdmissionError {
    /// Retain typed internal errors while presenting one config refusal.
    pub fn rejection_message(&self, actor: &str, config: &Value) -> String {
        let path = match self {
            Self::UnknownField(field) => ConfigPath::root().join_key(field.clone()),
            Self::MissingMandatory(path)
            | Self::OutsideSpace { path, .. }
            | Self::UnresolvedConstraint { path, .. }
            | Self::Snippet { path, .. }
            | Self::DuplicateObjectValue { path, .. } => path.clone(),
            Self::EmptySchemaRequiresNull | Self::RootNotObject => ConfigPath::root(),
        };
        let value = path
            .segments()
            .iter()
            .try_fold(config, |value, segment| match segment {
                Segment::Key(key) => value.as_object().and_then(|object| object.get(key)),
                Segment::Index(index) => value
                    .as_array()
                    .and_then(|items| items.get(usize::try_from(*index).ok()?)),
            });
        config_rejection(actor, &path.to_string(), value, self)
    }
}

#[cfg(test)]
mod rejection_context_tests {
    use super::*;

    #[test]
    fn admission_rejection_spells_the_path_once_without_debug_form() {
        let tools = Value::object([(
            "tools",
            Value::object([(
                "write",
                Value::object([("approval", Value::string("always"))]).unwrap(),
            )])
            .unwrap(),
        )])
        .unwrap();
        let Err(crate::RegisteredCreateAdmissionError::Config(rejected)) =
            crate::admit_registered_create(circular_core::ActorType::ToolExecutor, &tools)
        else {
            panic!("a per-tool approval outside the closed tags is a config refusal");
        };
        let message = rejected.rejection_message("tools", &tools);
        assert_eq!(
            message,
            "ConfigRejected: actor `tools`; config.tools.write.approval = \"always\"; \
             config input config.tools.write.approval is outside its declared space; \
             allowed: \"none\", \"required\""
        );
        assert_eq!(
            CreateInputAdmissionError::MissingMandatory(
                ConfigPath::root().join_key("tools").join_index(0)
            )
            .to_string(),
            "mandatory config input config.tools[0] is missing"
        );
    }
}
