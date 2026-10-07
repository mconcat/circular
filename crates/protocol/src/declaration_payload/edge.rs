
use circular_core::{Ceilings, Ticks, Value, decode};
use std::fmt;
use std::time::Duration;

use crate::authored_value::Config;

#[cfg(test)]
use super::actor::decode_retire_actor;
use crate::scope_identity::{AddressContext, AddressRef, PlanActorKey, decode_plan_actor_key};
use crate::wire_value::{
    PayloadRejection, decode_arm, decode_ratio, exhausted, int_of, object_fields,
    object_from_value, optional, take, text_of, unsigned_of,
};

pub type PortId = String;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeclaredDelay {
    numerator: u64,
    denominator: u64,
}

impl DeclaredDelay {
    pub const ZERO: Self = Self {
        numerator: 0,
        denominator: 1,
    };

    pub const fn try_new(numerator: u64, denominator: u64) -> Result<Self, DeclaredDelayError> {
        if denominator == 0 {
            return Err(DeclaredDelayError::ZeroDenominator);
        }
        let divisor = greatest_common_divisor(numerator, denominator);
        if divisor != 1 {
            return Err(DeclaredDelayError::NotReduced { divisor });
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }

    #[must_use]
    pub const fn from_whole_seconds(seconds: u64) -> Self {
        Self {
            numerator: seconds,
            denominator: 1,
        }
    }

    #[must_use]
    pub const fn numerator(self) -> u64 {
        self.numerator
    }

    #[must_use]
    pub const fn denominator(self) -> u64 {
        self.denominator
    }

    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.numerator == 0
    }

    pub fn try_duration(self) -> Result<Duration, DeclaredDelayDurationError> {
        let seconds = self.numerator / self.denominator;
        let remainder = self.numerator % self.denominator;
        let scaled = u128::from(remainder) * 1_000_000_000_u128;
        let denominator = u128::from(self.denominator);
        if scaled % denominator != 0 {
            return Err(DeclaredDelayDurationError::BelowNanosecondResolution);
        }
        let nanos = scaled / denominator;
        let nanos =
            u32::try_from(nanos).map_err(|_| DeclaredDelayDurationError::DurationOutOfRange)?;
        Ok(Duration::new(seconds, nanos))
    }
}

const fn greatest_common_divisor(mut left: u64, mut right: u64) -> u64 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeclaredDelayError {
    ZeroDenominator,
    NotReduced { divisor: u64 },
}

impl fmt::Display for DeclaredDelayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDenominator => formatter.write_str("wire delay denominator is zero"),
            Self::NotReduced { divisor } => write!(
                formatter,
                "wire delay is not reduced (greatest common divisor {divisor})"
            ),
        }
    }
}

impl std::error::Error for DeclaredDelayError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeclaredDelayDurationError {
    BelowNanosecondResolution,
    DurationOutOfRange,
}

impl fmt::Display for DeclaredDelayDurationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BelowNanosecondResolution => {
                formatter.write_str("wire delay cannot be represented at nanosecond resolution")
            }
            Self::DurationOutOfRange => {
                formatter.write_str("wire delay exceeds the platform duration range")
            }
        }
    }
}

impl std::error::Error for DeclaredDelayDurationError {}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Delivery {
    BestEffort { on_full: Shed },
    Lossless,
    Durable,
}

circular_core::closed_table! {
    pub enum Shed: i64 {
        DropNewest = 1,
        DropOldest = 2,
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PositiveCapacity(usize);

impl PositiveCapacity {
    pub const fn new(value: usize) -> Result<Self, PositiveCapacityError> {
        if value == 0 {
            Err(PositiveCapacityError)
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PositiveCapacityError;

impl fmt::Display for PositiveCapacityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("edge capacity must be positive")
    }
}

impl std::error::Error for PositiveCapacityError {}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct WirePolicy {
    pub delivery: Delivery,
    pub capacity: Option<PositiveCapacity>,
}

impl WirePolicy {
    pub const DEFAULT_EDGE: Self = Self {
        delivery: Delivery::Lossless,
        capacity: match PositiveCapacity::new(64) {
            Ok(capacity) => Some(capacity),
            Err(_) => panic!("64 is positive"),
        },
    };

    #[must_use]
    pub const fn new(delivery: Delivery, capacity: Option<PositiveCapacity>) -> Self {
        Self { delivery, capacity }
    }

    #[must_use]
    pub const fn delivery(self) -> Delivery {
        self.delivery
    }

    #[must_use]
    pub const fn capacity(self) -> Option<PositiveCapacity> {
        self.capacity
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PreprocessKind {
    Map,
    Filter,
    Bang,
    Parse,
    Flatten,
}

impl PreprocessKind {
    pub const ALL: [Self; 5] = [
        Self::Map,
        Self::Filter,
        Self::Bang,
        Self::Parse,
        Self::Flatten,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Map => "map",
            Self::Filter => "filter",
            Self::Bang => "bang",
            Self::Parse => "parse",
            Self::Flatten => "flatten",
        }
    }

    #[must_use]
    pub fn from_wire(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == text)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PreprocessStep {
    pub kind: PreprocessKind,
    pub config: Config,
}

impl PreprocessStep {
    #[must_use]
    pub const fn new(kind: PreprocessKind, config: Config) -> Self {
        Self { kind, config }
    }

    #[must_use]
    pub const fn kind(&self) -> PreprocessKind {
        self.kind
    }

    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }
}

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct PreprocessChain(Box<[PreprocessStep]>);

impl PreprocessChain {
    #[must_use]
    pub fn new(steps: impl Into<Box<[PreprocessStep]>>) -> Self {
        Self(steps.into())
    }

    #[must_use]
    pub fn steps(&self) -> &[PreprocessStep] {
        &self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct EdgeAttrs {
    pub delay: DeclaredDelay,
    pub policy: WirePolicy,
    pub preprocess: PreprocessChain,
}

impl EdgeAttrs {
    #[must_use]
    pub fn new(delay: Ticks, policy: WirePolicy) -> Self {
        Self {
            delay: DeclaredDelay::from_whole_seconds(delay.get()),
            policy,
            preprocess: PreprocessChain::default(),
        }
    }

    #[must_use]
    pub fn new_rational(delay: DeclaredDelay, policy: WirePolicy) -> Self {
        Self {
            delay,
            policy,
            preprocess: PreprocessChain::default(),
        }
    }

    #[must_use]
    pub const fn delay(&self) -> DeclaredDelay {
        self.delay
    }

    #[must_use]
    pub const fn policy(&self) -> WirePolicy {
        self.policy
    }

    #[must_use]
    pub fn with_preprocess(mut self, chain: PreprocessChain) -> Self {
        self.preprocess = chain;
        self
    }

    #[must_use]
    pub const fn preprocess(&self) -> &PreprocessChain {
        &self.preprocess
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EdgeDeclaration {
    pub from: (PlanActorKey, PortId),
    pub to: (PlanActorKey, PortId),
    pub ordinal: u16,
    pub attrs: EdgeAttrs,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeclaredEdgeKey {
    pub from: (PlanActorKey, PortId),
    pub to: (PlanActorKey, PortId),
    pub ordinal: u16,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum EdgeKey {
    Declared(DeclaredEdgeKey),
    Outcome(PlanActorKey),
}

#[must_use]
pub fn edge_identity_value(edge: &EdgeKey) -> Value {
    let endpoint = |(actor, port): &(PlanActorKey, PortId)| {
        Value::object([
            ("actor", crate::scope_identity::plan_actor_key_value(actor)),
            ("port", Value::String(port.clone())),
        ])
        .expect("the two keys differ")
    };
    match edge {
        EdgeKey::Declared(key) => Value::Array(vec![
            Value::Int(1),
            endpoint(&key.from),
            endpoint(&key.to),
            Value::Int(i64::from(key.ordinal)),
        ]),
        EdgeKey::Outcome(target) => Value::Array(vec![
            Value::Int(2),
            crate::scope_identity::plan_actor_key_value(target),
        ]),
    }
}

fn decode_identity_endpoint(value: Value) -> Result<(PlanActorKey, PortId), PayloadRejection> {
    let mut fields = object_fields(value, "endpoint")?;
    let actor = decode_plan_actor_key(take(&mut fields, "actor")?)?;
    let port = text_of(take(&mut fields, "port")?, "port")?;
    exhausted(fields)?;
    Ok((actor, port))
}

pub fn decode_edge_identity(value: Value) -> Result<EdgeKey, PayloadRejection> {
    let Value::Array(mut parts) = value else {
        return Err(PayloadRejection::WrongCarrier { key: "edge" });
    };
    let tag = match parts.first() {
        Some(Value::Int(tag)) => *tag,
        _ => return Err(PayloadRejection::WrongCarrier { key: "edge" }),
    };
    match (tag, parts.len()) {
        (1, 4) => {
            let ordinal = match parts.remove(3) {
                Value::Int(ordinal) => u16::try_from(ordinal)
                    .map_err(|_| PayloadRejection::WrongCarrier { key: "ordinal" })?,
                _ => return Err(PayloadRejection::WrongCarrier { key: "ordinal" }),
            };
            let to = decode_identity_endpoint(parts.remove(2))?;
            let from = decode_identity_endpoint(parts.remove(1))?;
            Ok(EdgeKey::Declared(DeclaredEdgeKey { from, to, ordinal }))
        }
        (2, 2) => Ok(EdgeKey::Outcome(decode_plan_actor_key(parts.remove(1))?)),
        (1 | 2, _) => Err(PayloadRejection::WrongCarrier { key: "edge" }),
        (other, _) => Err(PayloadRejection::UnknownArm { tag: other }),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UpsertEdge {
    pub edge: AddressRef<DeclaredEdgeKey>,
    pub declaration: EdgeDeclaration,
}

fn decode_delay(value: Value) -> Result<DeclaredDelay, PayloadRejection> {
    let (num, den) = decode_ratio(value, "delay")?;
    DeclaredDelay::try_new(num, den).map_err(|_| PayloadRejection::NotCanonical { key: "delay" })
}

fn decode_delivery(value: Value) -> Result<Delivery, PayloadRejection> {
    let (tag, mut arguments) = decode_arm(value, "delivery")?;
    match arguments.len() {
        0 => match tag {
            2 => Ok(Delivery::Lossless),
            3 => Ok(Delivery::Durable),
            other => Err(PayloadRejection::UnknownArm { tag: other }),
        },
        1 => {
            let on_full = int_of(arguments.pop().expect("one argument"), "on_full")?;
            let shed =
                Shed::from_tag(on_full).ok_or(PayloadRejection::UnknownArm { tag: on_full })?;
            match tag {
                1 => Ok(Delivery::BestEffort { on_full: shed }),
                other => Err(PayloadRejection::UnknownArm { tag: other }),
            }
        }
        _ => Err(PayloadRejection::WrongCarrier { key: "delivery" }),
    }
}

fn decode_policy(value: Value) -> Result<WirePolicy, PayloadRejection> {
    let mut fields = object_fields(value, "policy")?;
    let capacity = match optional(&mut fields, "capacity") {
        Some(capacity) => {
            let raw = unsigned_of(capacity, "capacity")?;
            let narrowed = usize::try_from(raw)
                .map_err(|_| PayloadRejection::WrongCarrier { key: "capacity" })?;
            Some(
                PositiveCapacity::new(narrowed)
                    .map_err(|_| PayloadRejection::WrongCarrier { key: "capacity" })?,
            )
        }
        None => None,
    };
    let delivery = decode_delivery(take(&mut fields, "delivery")?)?;
    exhausted(fields)?;
    Ok(WirePolicy { delivery, capacity })
}

pub(crate) fn decode_endpoint(value: Value) -> Result<(PlanActorKey, PortId), PayloadRejection> {
    let Value::Array(mut parts) = value else {
        return Err(PayloadRejection::WrongCarrier { key: "endpoint" });
    };
    if parts.len() != 2 {
        return Err(PayloadRejection::WrongCarrier { key: "endpoint" });
    }
    let port = text_of(parts.remove(1), "endpoint")?;
    let actor = decode_plan_actor_key(parts.remove(0))?;
    Ok((actor, port))
}

fn decode_preprocess(value: Value) -> Result<PreprocessChain, PayloadRejection> {
    let Value::Array(steps) = value else {
        return Err(PayloadRejection::WrongCarrier { key: "preprocess" });
    };
    steps
        .into_iter()
        .map(|step| {
            let mut fields = object_fields(step, "preprocess step")?;
            let kind = PreprocessKind::from_wire(&text_of(take(&mut fields, "kind")?, "kind")?)
                .ok_or(PayloadRejection::WrongCarrier { key: "kind" })?;
            let config = take(&mut fields, "config")?;
            if !matches!(config, Value::Object(_)) {
                return Err(PayloadRejection::WrongCarrier { key: "config" });
            }
            let config = Config::from_wire_value(&config)
                .map_err(|_| PayloadRejection::WrongCarrier { key: "config" })?;
            exhausted(fields)?;
            Ok(PreprocessStep { kind, config })
        })
        .collect::<Result<Vec<_>, PayloadRejection>>()
        .map(PreprocessChain::new)
}

fn decode_edge_declaration(value: Value) -> Result<EdgeDeclaration, PayloadRejection> {
    let mut fields = object_fields(value, "declaration")?;
    let attrs = {
        let mut attr_fields = object_fields(take(&mut fields, "attrs")?, "attrs")?;
        let delay = decode_delay(take(&mut attr_fields, "delay")?)?;
        let policy = decode_policy(take(&mut attr_fields, "policy")?)?;
        let preprocess = match optional(&mut attr_fields, "preprocess") {
            Some(preprocess) => decode_preprocess(preprocess)?,
            None => PreprocessChain::default(),
        };
        exhausted(attr_fields)?;
        EdgeAttrs {
            delay,
            policy,
            preprocess,
        }
    };
    let from = decode_endpoint(take(&mut fields, "from")?)?;
    let ordinal = u16::try_from(int_of(take(&mut fields, "ordinal")?, "ordinal")?)
        .map_err(|_| PayloadRejection::WrongCarrier { key: "ordinal" })?;
    let to = decode_endpoint(take(&mut fields, "to")?)?;
    exhausted(fields)?;
    Ok(EdgeDeclaration {
        from,
        to,
        ordinal,
        attrs,
    })
}

fn decode_edge_key(value: Value) -> Result<DeclaredEdgeKey, PayloadRejection> {
    match decode_edge_identity(value)? {
        EdgeKey::Declared(key) => Ok(key),
        EdgeKey::Outcome(_) => Err(PayloadRejection::ArmNotAdmitted),
    }
}

pub fn decode_upsert_edge(
    bytes: &[u8],
    context: AddressContext,
    ceilings: Ceilings,
) -> Result<UpsertEdge, PayloadRejection> {
    upsert_edge_from_value(
        decode(bytes, ceilings).map_err(PayloadRejection::Codec)?,
        context,
    )
}

pub fn upsert_edge_from_value(
    value: Value,
    context: AddressContext,
) -> Result<UpsertEdge, PayloadRejection> {
    let mut fields = object_from_value(value)?;
    let declaration = decode_edge_declaration(take(&mut fields, "declaration")?)?;

    let Value::Array(mut parts) = take(&mut fields, "edge")? else {
        return Err(PayloadRejection::WrongCarrier { key: "address" });
    };
    if parts.len() != 2 {
        return Err(PayloadRejection::WrongCarrier { key: "address" });
    }
    let key = decode_edge_key(parts.remove(1))?;
    let tag = int_of(parts.remove(0), "address")?;
    exhausted(fields)?;

    if key.from != declaration.from
        || key.to != declaration.to
        || key.ordinal != declaration.ordinal
    {
        return Err(PayloadRejection::KeyDisagreesWithDeclaration);
    }

    let edge = match tag {
        1 => AddressRef::Absolute(key),
        2 => AddressRef::EpochLocal(key),
        3 => AddressRef::Relative(key),
        other => return Err(PayloadRejection::UnknownArm { tag: other }),
    };

    if !context.admits(&edge) {
        return Err(PayloadRejection::ArmNotAdmitted);
    }

    Ok(UpsertEdge { edge, declaration })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetireEdge {
    pub edge: AddressRef<DeclaredEdgeKey>,
}

pub fn retire_edge_from_value(
    value: Value,
    context: AddressContext,
) -> Result<RetireEdge, PayloadRejection> {
    let mut fields = object_from_value(value)?;

    let Value::Array(mut parts) = take(&mut fields, "edge")? else {
        return Err(PayloadRejection::WrongCarrier { key: "address" });
    };
    if parts.len() != 2 {
        return Err(PayloadRejection::WrongCarrier { key: "address" });
    }
    let key = decode_edge_key(parts.remove(1))?;
    let tag = int_of(parts.remove(0), "address")?;
    exhausted(fields)?;

    let edge = match tag {
        1 => AddressRef::Absolute(key),
        2 => AddressRef::EpochLocal(key),
        3 => AddressRef::Relative(key),
        other => return Err(PayloadRejection::UnknownArm { tag: other }),
    };

    if !context.admits(&edge) {
        return Err(PayloadRejection::ArmNotAdmitted);
    }

    Ok(RetireEdge { edge })
}

pub fn decode_retire_edge(
    bytes: &[u8],
    context: AddressContext,
    ceilings: Ceilings,
) -> Result<RetireEdge, PayloadRejection> {
    retire_edge_from_value(
        decode(bytes, ceilings).map_err(PayloadRejection::Codec)?,
        context,
    )
}

#[cfg(test)]
mod upsert_edge_tests {
    use super::*;
    use circular_core::{ObjectValue, encode};

    const CEILINGS: Ceilings = Ceilings::for_boundary(circular_core::Boundary::Wire);

    fn key(local: &str) -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                ("local".to_owned(), Value::String(local.to_owned())),
                (
                    "scope".to_owned(),
                    Value::Array(vec![Value::Array(vec![
                        Value::Int(1),
                        Value::String("gate".to_owned()),
                    ])]),
                ),
            ])
            .expect("two keys"),
        )
    }

    fn endpoint(local: &str, port: &str) -> Value {
        Value::Array(vec![key(local), Value::String(port.to_owned())])
    }

    fn delay(num: i64, den: i64) -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                ("den".to_owned(), Value::Int(den)),
                ("num".to_owned(), Value::Int(num)),
            ])
            .expect("two keys"),
        )
    }

    fn policy() -> Value {
        Value::Object(
            ObjectValue::try_from_entries([("delivery".to_owned(), Value::Int(2))])
                .expect("one key"),
        )
    }

    fn identity_endpoint(local: &str, port: &str) -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                ("actor".to_owned(), key(local)),
                ("port".to_owned(), Value::String(port.to_owned())),
            ])
            .expect("two keys"),
        )
    }

    fn edge_key(from: &str, to: &str, ordinal: i64) -> Value {
        Value::Array(vec![
            Value::Int(1),
            identity_endpoint(from, "out"),
            identity_endpoint(to, "in"),
            Value::Int(ordinal),
        ])
    }

    fn declaration(from: &str, to: &str, ordinal: i64, delay_value: Value) -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                (
                    "attrs".to_owned(),
                    Value::Object(
                        ObjectValue::try_from_entries([
                            ("delay".to_owned(), delay_value),
                            ("policy".to_owned(), policy()),
                        ])
                        .expect("two keys"),
                    ),
                ),
                ("from".to_owned(), endpoint(from, "out")),
                ("ordinal".to_owned(), Value::Int(ordinal)),
                ("to".to_owned(), endpoint(to, "in")),
            ])
            .expect("four keys"),
        )
    }

    fn body(key_value: Value, declaration_value: Value) -> Vec<u8> {
        let object = ObjectValue::try_from_entries([
            ("declaration".to_owned(), declaration_value),
            (
                "edge".to_owned(),
                Value::Array(vec![Value::Int(1), key_value]),
            ),
        ])
        .expect("two keys");
        encode(&Value::Object(object), CEILINGS).expect("encodes")
    }

    #[test]
    fn an_upsert_edge_opens_with_the_canonical_shape() {
        let bytes = body(
            edge_key("bang", "map", 0),
            declaration("bang", "map", 0, delay(0, 1)),
        );
        let decoded =
            decode_upsert_edge(&bytes, AddressContext::Mutation, CEILINGS).expect("decodes");
        assert_eq!(decoded.declaration.ordinal, 0);
        assert_eq!(decoded.declaration.attrs.delay, DeclaredDelay::ZERO);
        assert_eq!(
            decoded.declaration.attrs.policy.delivery,
            Delivery::Lossless
        );
        assert_eq!(decoded.declaration.attrs.policy.capacity, None);
        assert!(decoded.declaration.attrs.preprocess.is_empty());
    }

    fn with_preprocess(steps: Value) -> Value {
        let mut declaration = declaration("bang", "map", 0, delay(0, 1))
            .as_object()
            .unwrap()
            .clone()
            .into_map();
        let mut attrs = declaration
            .remove("attrs")
            .unwrap()
            .as_object()
            .unwrap()
            .clone()
            .into_map();
        attrs.insert("preprocess".to_owned(), steps);
        declaration.insert("attrs".to_owned(), Value::Object(attrs.into()));
        Value::Object(declaration.into())
    }

    #[test]
    fn preprocess_unknown_kind_and_malformed_steps_are_refused() {
        for (steps, expected) in [
            (
                Value::Null,
                PayloadRejection::WrongCarrier { key: "preprocess" },
            ),
            (
                Value::array([Value::object([
                    ("kind", Value::string("tap")),
                    (
                        "config",
                        Value::object(Vec::<(&str, Value)>::new()).unwrap(),
                    ),
                ])
                .unwrap()]),
                PayloadRejection::WrongCarrier { key: "kind" },
            ),
            (
                Value::array([Value::object([("kind", Value::string("bang"))]).unwrap()]),
                PayloadRejection::MissingKey("config"),
            ),
        ] {
            let bytes = body(edge_key("bang", "map", 0), with_preprocess(steps));
            assert_eq!(
                decode_upsert_edge(&bytes, AddressContext::Mutation, CEILINGS),
                Err(expected)
            );
        }
    }

    #[test]
    fn astronomy_flatten_uses_the_existing_string_kind_and_value_codec() {
        let config = Value::object([("at", Value::array([Value::string("spans")]))]).unwrap();
        let bytes = body(
            edge_key("bang", "map", 0),
            with_preprocess(Value::array([Value::object([
                ("kind", Value::string("flatten")),
                ("config", config.clone()),
            ])
            .unwrap()])),
        );
        let decoded = decode_upsert_edge(&bytes, AddressContext::Mutation, CEILINGS).unwrap();
        assert_eq!(decoded.declaration.attrs.preprocess.len(), 1);
        assert_eq!(
            decoded.declaration.attrs.preprocess.steps()[0].kind,
            PreprocessKind::Flatten
        );
        assert_eq!(
            decoded.declaration.attrs.preprocess.steps()[0]
                .config
                .to_wire_value()
                .expect("a validated record goes back to a wire value"),
            config
        );
        assert_eq!(
            PreprocessKind::ALL.map(PreprocessKind::as_str),
            ["map", "filter", "bang", "parse", "flatten"]
        );
    }

    #[test]
    fn all_is_the_only_list_of_preprocess_kinds() {
        for kind in PreprocessKind::ALL {
            match kind {
                PreprocessKind::Map
                | PreprocessKind::Filter
                | PreprocessKind::Bang
                | PreprocessKind::Parse
                | PreprocessKind::Flatten => {}
            }
        }
        assert_eq!(PreprocessKind::ALL.len(), 5);
        let spellings: std::collections::BTreeSet<_> = PreprocessKind::ALL
            .iter()
            .map(|kind| kind.as_str())
            .collect();
        assert_eq!(spellings.len(), PreprocessKind::ALL.len());
    }

    #[test]
    fn the_decoder_accepts_exactly_the_published_spellings() {
        for kind in PreprocessKind::ALL {
            assert_eq!(PreprocessKind::from_wire(kind.as_str()), Some(kind));
            let bytes = body(
                edge_key("bang", "map", 0),
                with_preprocess(Value::array([Value::object([
                    ("kind", Value::string(kind.as_str())),
                    ("config", Value::object([] as [(&str, Value); 0]).unwrap()),
                ])
                .unwrap()])),
            );
            assert_eq!(
                decode_upsert_edge(&bytes, AddressContext::Mutation, CEILINGS)
                    .unwrap()
                    .declaration
                    .attrs
                    .preprocess
                    .steps()[0]
                    .kind,
                kind
            );
        }
        assert_eq!(PreprocessKind::from_wire("Map"), None);
        let unknown = body(
            edge_key("bang", "map", 0),
            with_preprocess(Value::array([Value::object([
                ("kind", Value::string("route")),
                ("config", Value::object([] as [(&str, Value); 0]).unwrap()),
            ])
            .unwrap()])),
        );
        assert!(decode_upsert_edge(&unknown, AddressContext::Mutation, CEILINGS).is_err());
    }

    #[test]
    fn explicit_empty_preprocess_decodes_to_an_empty_chain() {
        let bytes = body(
            edge_key("bang", "map", 0),
            with_preprocess(Value::array([])),
        );
        assert!(
            decode_upsert_edge(&bytes, AddressContext::Mutation, CEILINGS)
                .unwrap()
                .declaration
                .attrs
                .preprocess
                .is_empty()
        );
    }

    #[test]
    fn duplicate_preprocess_and_step_fields_are_refused_in_bytes() {
        let step = Value::object([
            ("kind", Value::string("bang")),
            (
                "config",
                Value::object(Vec::<(&str, Value)>::new()).unwrap(),
            ),
        ])
        .unwrap();
        let declaration = with_preprocess(Value::array([step.clone()]));
        let attrs = declaration.as_object().unwrap().get("attrs").unwrap();
        for (object, duplicate) in [(attrs, "preprocess"), (&step, "kind"), (&step, "config")] {
            let original = encode(object, CEILINGS).unwrap();
            let fields = object.as_object().unwrap();
            let mut malformed = vec![original[0]];
            malformed.extend_from_slice(&u32::try_from(fields.len() + 1).unwrap().to_be_bytes());
            for (key, value) in fields.iter() {
                let field =
                    encode(&Value::object([(key, value.clone())]).unwrap(), CEILINGS).unwrap();
                malformed.extend_from_slice(&field[5..]);
                if key == duplicate {
                    malformed.extend_from_slice(&field[5..]);
                }
            }
            let mut bytes = body(edge_key("bang", "map", 0), declaration.clone());
            let offset = bytes
                .windows(original.len())
                .position(|window| window == original)
                .unwrap();
            bytes.splice(offset..offset + original.len(), malformed);
            assert_eq!(
                decode_upsert_edge(&bytes, AddressContext::Mutation, CEILINGS),
                Err(PayloadRejection::Codec(
                    circular_core::CodecError::ObjectKeyOrder
                )),
                "{duplicate}"
            );
        }
    }

    #[test]
    fn an_edge_may_reference_a_synth_boundary_actor() {
        let synth = "_bni1_0123456789abcdef0123456789";
        let bytes = body(
            edge_key(synth, "map", 0),
            declaration(synth, "map", 0, delay(0, 1)),
        );
        let decoded =
            decode_upsert_edge(&bytes, AddressContext::Mutation, CEILINGS).expect("edge decodes");
        assert!(matches!(
            &decoded.declaration.from.0.local,
            crate::scope_identity::ActorLocal::Synth(_)
        ));
        assert_eq!(decoded.declaration.from.0.local.as_str(), synth);
    }

    #[test]
    fn a_zero_denominator_is_refused() {
        let bytes = body(edge_key("a", "b", 0), declaration("a", "b", 0, delay(1, 0)));
        assert_eq!(
            decode_upsert_edge(&bytes, AddressContext::Mutation, CEILINGS),
            Err(PayloadRejection::WrongCarrier { key: "den" })
        );
    }

    #[test]
    fn capacity_absence_is_structural() {
        let with_capacity = Value::Object(
            ObjectValue::try_from_entries([
                ("capacity".to_owned(), Value::Int(64)),
                (
                    "delivery".to_owned(),
                    Value::Array(vec![Value::Int(1), Value::Int(1)]),
                ),
            ])
            .expect("two keys"),
        );
        let declaration_value = Value::Object(
            ObjectValue::try_from_entries([
                (
                    "attrs".to_owned(),
                    Value::Object(
                        ObjectValue::try_from_entries([
                            ("delay".to_owned(), delay(1, 48000)),
                            ("policy".to_owned(), with_capacity),
                        ])
                        .expect("two keys"),
                    ),
                ),
                ("from".to_owned(), endpoint("a", "out")),
                ("ordinal".to_owned(), Value::Int(1)),
                ("to".to_owned(), endpoint("b", "in")),
            ])
            .expect("four keys"),
        );
        let bytes = body(edge_key("a", "b", 1), declaration_value);
        let decoded =
            decode_upsert_edge(&bytes, AddressContext::Mutation, CEILINGS).expect("decodes");
        assert_eq!(
            decoded.declaration.attrs.policy.capacity,
            PositiveCapacity::new(64).ok()
        );
        assert_eq!(
            decoded.declaration.attrs.policy.delivery,
            Delivery::BestEffort {
                on_full: Shed::DropNewest
            }
        );
        assert_eq!(
            decoded.declaration.attrs.delay,
            DeclaredDelay::try_new(1, 48000).expect("an authored delay is a reduced rational")
        );
    }
}

#[cfg(test)]
mod retire_tests {
    use super::*;
    use circular_core::{ObjectValue, encode};

    const CEILINGS: Ceilings = Ceilings::for_boundary(circular_core::Boundary::Wire);

    fn key_object(scope: &[&str], local: &str) -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                ("local".to_owned(), Value::String(local.to_owned())),
                (
                    "scope".to_owned(),
                    Value::Array(
                        scope
                            .iter()
                            .map(|name| {
                                Value::Array(vec![Value::Int(1), Value::String((*name).to_owned())])
                            })
                            .collect(),
                    ),
                ),
            ])
            .expect("two keys"),
        )
    }

    fn endpoint(scope: &[&str], local: &str, port: &str) -> Value {
        Value::Array(vec![
            key_object(scope, local),
            Value::String(port.to_owned()),
        ])
    }

    fn identity_endpoint(scope: &[&str], local: &str, port: &str) -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                ("actor".to_owned(), key_object(scope, local)),
                ("port".to_owned(), Value::String(port.to_owned())),
            ])
            .expect("two keys"),
        )
    }

    fn edge_key() -> Value {
        Value::Array(vec![
            Value::Int(1),
            identity_endpoint(&["gate", "inner"], "map", "out"),
            identity_endpoint(&["gate", "inner"], "filter", "in"),
            Value::Int(0),
        ])
    }

    fn one_key(key: &str, value: Value) -> Vec<u8> {
        let object = ObjectValue::try_from_entries([(key.to_owned(), value)]).expect("one key");
        encode(&Value::Object(object), CEILINGS).expect("encodes")
    }

    #[test]
    fn a_retire_actor_carries_only_its_target() {
        let decoded = decode_retire_actor(
            &one_key(
                "actor",
                Value::Array(vec![Value::Int(1), key_object(&["gate", "inner"], "map")]),
            ),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("decodes");

        let AddressRef::Absolute(key) = &decoded.actor else {
            panic!("it is an absolute address");
        };
        assert_eq!(key.local, "map");
        assert_eq!(key.scope.len(), 2, "the two segments were flattened");
    }

    #[test]
    fn a_retire_actor_with_a_declaration_is_rejected() {
        let body = encode(
            &Value::Object(
                ObjectValue::try_from_entries([
                    ("declaration".to_owned(), Value::Null),
                    (
                        "actor".to_owned(),
                        Value::Array(vec![Value::Int(1), key_object(&["gate"], "map")]),
                    ),
                ])
                .expect("two keys"),
            ),
            CEILINGS,
        )
        .expect("encodes");
        assert_eq!(
            decode_retire_actor(&body, AddressContext::Mutation, CEILINGS),
            Err(PayloadRejection::UnknownKey("declaration".to_owned()))
        );
    }

    #[test]
    fn the_edge_key_carrier_is_the_one_upsert_edge_uses() {
        let retired = decode_retire_edge(
            &one_key("edge", Value::Array(vec![Value::Int(1), edge_key()])),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("decodes");

        let declaration = Value::Object(
            ObjectValue::try_from_entries([
                (
                    "attrs".to_owned(),
                    Value::Object(
                        ObjectValue::try_from_entries([
                            (
                                "delay".to_owned(),
                                Value::Object(
                                    ObjectValue::try_from_entries([
                                        ("den".to_owned(), Value::Int(1)),
                                        ("num".to_owned(), Value::Int(0)),
                                    ])
                                    .expect("two keys"),
                                ),
                            ),
                            (
                                "policy".to_owned(),
                                Value::Object(
                                    ObjectValue::try_from_entries([(
                                        "delivery".to_owned(),
                                        Value::Int(2),
                                    )])
                                    .expect("one key"),
                                ),
                            ),
                        ])
                        .expect("two keys"),
                    ),
                ),
                (
                    "from".to_owned(),
                    endpoint(&["gate", "inner"], "map", "out"),
                ),
                ("ordinal".to_owned(), Value::Int(0)),
                (
                    "to".to_owned(),
                    endpoint(&["gate", "inner"], "filter", "in"),
                ),
            ])
            .expect("four keys"),
        );
        let upsert_body = encode(
            &Value::Object(
                ObjectValue::try_from_entries([
                    ("declaration".to_owned(), declaration),
                    (
                        "edge".to_owned(),
                        Value::Array(vec![Value::Int(1), edge_key()]),
                    ),
                ])
                .expect("two keys"),
            ),
            CEILINGS,
        )
        .expect("encodes");
        let upserted =
            decode_upsert_edge(&upsert_body, AddressContext::Mutation, CEILINGS).expect("decodes");

        let (AddressRef::Absolute(left), AddressRef::Absolute(right)) =
            (&retired.edge, &upserted.edge)
        else {
            panic!("both are absolute addresses");
        };
        assert_eq!(left, right, "the keys set and the keys removed disagree");
    }

    #[test]
    fn the_context_partitions_the_arms() {
        let actor = one_key(
            "actor",
            Value::Array(vec![Value::Int(2), key_object(&["gate"], "map")]),
        );
        assert!(decode_retire_actor(&actor, AddressContext::Mutation, CEILINGS).is_ok());
        assert_eq!(
            decode_retire_actor(&actor, AddressContext::AcceptedHistory, CEILINGS),
            Err(PayloadRejection::ArmNotAdmitted)
        );

        let edge = one_key("edge", Value::Array(vec![Value::Int(2), edge_key()]));
        assert!(decode_retire_edge(&edge, AddressContext::Mutation, CEILINGS).is_ok());
        assert_eq!(
            decode_retire_edge(&edge, AddressContext::AcceptedHistory, CEILINGS),
            Err(PayloadRejection::ArmNotAdmitted)
        );
    }
}
