
use std::error::Error;
use std::fmt;

use circular_core::Value;

use crate::boundary_port::SynthBoundaryLocal;
use crate::wire_value::{
    PayloadRejection, args_arm, arm, decode_arm, exhausted, object_fields, take, text_of,
};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ScopeSegment {
    Child(String),
    Instance { of: String, key: InstanceKey },
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InstanceKey {
    Scalar(ScalarKey),
    Tuple(Vec<ScalarKey>),
}

impl InstanceKey {
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Scalar(value) => value.to_value(),
            Self::Tuple(values) => Value::Array(values.iter().map(ScalarKey::to_value).collect()),
        }
    }
}

impl From<ScalarKey> for InstanceKey {
    fn from(value: ScalarKey) -> Self {
        Self::Scalar(value)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ScalarKey {
    Text(String),
    Int(i64),
    Bool(bool),
}

impl ScalarKey {
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Text(text) => Value::String(text.clone()),
            Self::Int(number) => Value::Int(*number),
            Self::Bool(flag) => Value::Bool(*flag),
        }
    }
}

impl ScopeSegment {
    #[must_use]
    pub fn to_value(&self) -> Value {
        match self {
            Self::Child(name) => arm(1, Value::String(name.clone())),
            Self::Instance { of, key } => args_arm(2, [Value::String(of.clone()), key.to_value()]),
        }
    }
}

#[must_use]
pub fn scope_identity_value(segments: &[ScopeSegment]) -> Value {
    Value::Array(segments.iter().map(ScopeSegment::to_value).collect())
}

#[must_use]
pub fn plan_actor_key_value(actor: &PlanActorKey) -> Value {
    Value::object([
        ("local", Value::String(actor.local.as_str().to_owned())),
        ("scope", scope_identity_value(&actor.scope)),
    ])
    .expect("plan actor key fields are unique")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AddressRef<I> {
    Absolute(I),
    EpochLocal(I),
    Relative(I),
}

impl<I> AddressRef<I> {
    /// The inverse of the shared address-arm decoder; the identity owns its value.
    pub fn to_value_with(&self, identity: impl FnOnce(&I) -> Value) -> Value {
        match self {
            Self::Absolute(value) => arm(1, identity(value)),
            Self::EpochLocal(value) => arm(2, identity(value)),
            Self::Relative(value) => arm(3, identity(value)),
        }
    }
}

pub type ScopeAddress = AddressRef<Vec<ScopeSegment>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AddressContext {
    Mutation,
    AcceptedHistory,
    AuthoringSnapshot,
}

impl AddressContext {
    #[must_use]
    pub const fn admits<I>(self, address: &AddressRef<I>) -> bool {
        matches!(
            (self, address),
            (
                Self::Mutation,
                AddressRef::Absolute(_) | AddressRef::EpochLocal(_)
            ) | (Self::AcceptedHistory, AddressRef::Absolute(_))
                | (Self::AuthoringSnapshot, AddressRef::Relative(_))
        )
    }
}

fn decode_scalar_key(value: Value) -> Result<ScalarKey, PayloadRejection> {
    match value {
        Value::String(text) => Ok(ScalarKey::Text(text)),
        Value::Int(number) => Ok(ScalarKey::Int(number)),
        Value::Bool(flag) => Ok(ScalarKey::Bool(flag)),
        _ => Err(PayloadRejection::WrongCarrier {
            key: "instance_key",
        }),
    }
}

pub fn decode_instance_key(value: Value) -> Result<InstanceKey, PayloadRejection> {
    match value {
        Value::Array(values) => values
            .into_iter()
            .map(decode_scalar_key)
            .collect::<Result<Vec<_>, _>>()
            .map(InstanceKey::Tuple),
        scalar => decode_scalar_key(scalar).map(InstanceKey::Scalar),
    }
}

fn decode_segment(value: Value) -> Result<ScopeSegment, PayloadRejection> {
    let (tag, mut arguments) = decode_arm(value, "segment")?;
    match tag {
        1 if arguments.len() == 1 => Ok(ScopeSegment::Child(text_of(
            arguments.pop().expect("one argument"),
            "segment",
        )?)),
        2 if arguments.len() == 2 => {
            let key = decode_instance_key(arguments.pop().expect("second argument"))?;
            let of = text_of(arguments.pop().expect("first argument"), "segment")?;
            Ok(ScopeSegment::Instance { of, key })
        }
        1 | 2 => Err(PayloadRejection::WrongCarrier { key: "segment" }),
        other => Err(PayloadRejection::UnknownArm { tag: other }),
    }
}

pub fn decode_scope_identity(value: Value) -> Result<Vec<ScopeSegment>, PayloadRejection> {
    let Value::Array(segments) = value else {
        return Err(PayloadRejection::WrongCarrier { key: "scope" });
    };
    segments.into_iter().map(decode_segment).collect()
}

pub(crate) fn decode_address<I>(
    value: Value,
    identity: impl FnOnce(Value) -> Result<I, PayloadRejection>,
) -> Result<AddressRef<I>, PayloadRejection> {
    let Value::Array(mut parts) = value else {
        return Err(PayloadRejection::WrongCarrier { key: "address" });
    };
    if parts.len() != 2 {
        return Err(PayloadRejection::WrongCarrier { key: "address" });
    }
    let identity = identity(parts.remove(1))?;
    let Value::Int(tag) = parts.remove(0) else {
        return Err(PayloadRejection::WrongCarrier { key: "address" });
    };
    match tag {
        1 => Ok(AddressRef::Absolute(identity)),
        2 => Ok(AddressRef::EpochLocal(identity)),
        3 => Ok(AddressRef::Relative(identity)),
        other => Err(PayloadRejection::UnknownArm { tag: other }),
    }
}

pub fn decode_scope_address(value: Value) -> Result<ScopeAddress, PayloadRejection> {
    decode_address(value, decode_scope_identity)
}

#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AuthoredLocal(String);

impl fmt::Debug for AuthoredLocal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, formatter)
    }
}

impl AuthoredLocal {
    pub fn try_new(spelling: impl Into<String>) -> Result<Self, ReservedLocalSpelling> {
        let spelling = spelling.into();
        if is_reserved_local_spelling(&spelling) {
            return Err(ReservedLocalSpelling);
        }
        Ok(Self(spelling))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReservedLocalSpelling;

impl fmt::Display for ReservedLocalSpelling {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("authored actor local uses the reserved `_bn` spelling")
    }
}

impl Error for ReservedLocalSpelling {}

#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ActorLocal {
    Authored(AuthoredLocal),
    Synth(SynthBoundaryLocal),
}

impl fmt::Debug for ActorLocal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), formatter)
    }
}

impl ActorLocal {
    #[must_use]
    pub fn parse(spelling: impl Into<String>) -> Self {
        let spelling = spelling.into();
        if is_reserved_local_spelling(&spelling) {
            Self::Synth(SynthBoundaryLocal::from_wire_spelling(spelling))
        } else {
            Self::Authored(
                AuthoredLocal::try_new(spelling)
                    .expect("non-reserved spelling is always an authored local"),
            )
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Authored(local) => local.as_str(),
            Self::Synth(local) => local.as_str(),
        }
    }
}

impl From<AuthoredLocal> for ActorLocal {
    fn from(local: AuthoredLocal) -> Self {
        Self::Authored(local)
    }
}

impl From<SynthBoundaryLocal> for ActorLocal {
    fn from(local: SynthBoundaryLocal) -> Self {
        Self::Synth(local)
    }
}

impl fmt::Display for ActorLocal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl PartialEq<str> for ActorLocal {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for ActorLocal {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

pub(crate) fn is_reserved_local_spelling(local: &str) -> bool {
    local.starts_with("_bn")
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PlanActorKey {
    pub scope: Vec<ScopeSegment>,
    pub local: ActorLocal,
}

impl fmt::Display for PlanActorKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(
            &circular_core::spelling::actor(&self.scope, &self.local),
            formatter,
        )
    }
}

impl circular_core::spelling::SpelledSegment for ScopeSegment {
    fn spelled(&self) -> circular_core::spelling::SegmentText<'_> {
        use circular_core::spelling::{ScalarText, SegmentText};
        fn scalar(value: &ScalarKey) -> ScalarText<'_> {
            match value {
                ScalarKey::Text(text) => ScalarText::Text(text),
                ScalarKey::Int(number) => ScalarText::Int(*number),
                ScalarKey::Bool(flag) => ScalarText::Bool(*flag),
            }
        }
        match self {
            Self::Child(name) => SegmentText::Child(name),
            Self::Instance { of, key } => SegmentText::Instance {
                of,
                key: match key {
                    InstanceKey::Scalar(value) => vec![scalar(value)],
                    InstanceKey::Tuple(values) => values.iter().map(scalar).collect(),
                },
            },
        }
    }
}

#[must_use]
pub fn spelled_scope(
    scope: &[ScopeSegment],
) -> circular_core::spelling::ScopeText<'_, ScopeSegment> {
    circular_core::spelling::ScopeText(scope)
}

pub fn decode_plan_actor_key(value: Value) -> Result<PlanActorKey, PayloadRejection> {
    let mut fields = object_fields(value, "actor")?;
    let spelling = text_of(take(&mut fields, "local")?, "local")?;
    let scope = decode_scope_identity(take(&mut fields, "scope")?)?;
    exhausted(fields)?;
    Ok(PlanActorKey {
        scope,
        local: ActorLocal::parse(spelling),
    })
}

pub fn decode_actor_address(value: Value) -> Result<AddressRef<PlanActorKey>, PayloadRejection> {
    decode_address(value, decode_plan_actor_key)
}

#[cfg(test)]
mod actor_local_tests {
    use super::*;

    fn actor_value(local: &str) -> Value {
        Value::object([
            ("local", Value::String(local.to_owned())),
            ("scope", Value::Array(Vec::new())),
        ])
        .expect("two unique keys")
    }

    #[test]
    fn authored_local_rejects_reserved_synth_spellings() {
        assert_eq!(
            AuthoredLocal::try_new("_bni1_0123456789abcdef0123456789"),
            Err(ReservedLocalSpelling)
        );
        assert_eq!(
            AuthoredLocal::try_new("_bno1_0123456789abcdef0123456789"),
            Err(ReservedLocalSpelling)
        );
        assert_eq!(
            AuthoredLocal::try_new("worker").map(|local| local.as_str().to_owned()),
            Ok("worker".to_owned())
        );
    }

    #[test]
    fn decoder_recovers_both_local_arms_without_changing_the_wire_spelling() {
        let authored = decode_plan_actor_key(actor_value("worker")).expect("authored key decodes");
        assert!(matches!(&authored.local, ActorLocal::Authored(_)));
        assert_eq!(authored.local.as_str(), "worker");
        let authored_wire = crate::boundary_port::encode_boundary_actor_key(&authored)
            .expect("authored key encodes");
        assert_eq!(
            crate::boundary_port::decode_boundary_actor_key(authored_wire),
            Ok(authored)
        );

        let spelling = "_bni1_0123456789abcdef0123456789";
        let synth = decode_plan_actor_key(actor_value(spelling)).expect("synth key decodes");
        assert!(matches!(&synth.local, ActorLocal::Synth(_)));
        assert_eq!(synth.local.as_str(), spelling);
        let synth_wire =
            crate::boundary_port::encode_boundary_actor_key(&synth).expect("synth key encodes");
        assert_eq!(
            crate::boundary_port::decode_boundary_actor_key(synth_wire),
            Ok(synth)
        );
    }
}
