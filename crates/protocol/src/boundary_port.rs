
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use circular_core::{Boundary, Ceilings, PortId, Value, encode};
use sha2::{Digest, Sha256};

use crate::authoring_snapshot::scope_identity_value;
use crate::declaration_payload::{AddressRef, PlanActorKey, decode_actor_address};
use crate::scope_identity::is_reserved_local_spelling;

/// Physical boundary-id codec version carried in the reserved port spelling.
pub const BOUNDARY_PORT_ID_VERSION: u8 = 1;
pub const SYNTH_BOUNDARY_LOCAL_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BoundaryActorGeneration(u64);

impl BoundaryActorGeneration {
    #[must_use]
    pub const fn initial() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub const fn retired(self) -> Result<Self, BoundaryGenerationError> {
        match self.0.checked_add(1) {
            Some(value) => Ok(Self(value)),
            None => Err(BoundaryGenerationError::Exhausted),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryGenerationError {
    Exhausted,
}

impl fmt::Display for BoundaryGenerationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exhausted => formatter.write_str("authored boundary generation is exhausted"),
        }
    }
}

impl Error for BoundaryGenerationError {}

circular_core::closed_table! {
    /// The two disjoint logical identity domains.
    #[derive(Ord, PartialOrd)]
    pub enum BoundaryPortDirection: i64 {
        /// A child `input` outlet projected as the parent container inlet.
        Inlet = 1,
        /// A child `output` inlet projected as the parent container outlet.
        Outlet = 2,
    }
}

impl BoundaryPortDirection {
    const fn prefix(self) -> &'static str {
        match self {
            Self::Inlet => "_bi1_",
            Self::Outlet => "_bo1_",
        }
    }

    const fn synth_local_prefix(self) -> &'static str {
        match self {
            Self::Inlet => "_bni1_",
            Self::Outlet => "_bno1_",
        }
    }
}

fn digest_hex26(identity: &Value, label: &str) -> Result<String, BoundaryPortIdError> {
    let bytes = encode(identity, Ceilings::for_boundary(Boundary::Identity))
        .map_err(|error| BoundaryPortIdError::Codec(format!("{label}: {error:?}")))?;
    let digest = Sha256::digest(bytes);
    let mut suffix = String::with_capacity(26);
    for byte in &digest[..13] {
        use std::fmt::Write as _;
        write!(&mut suffix, "{byte:02x}").expect("writing to String cannot fail");
    }
    Ok(suffix)
}

#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SynthBoundaryLocal(String);

impl fmt::Debug for SynthBoundaryLocal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, formatter)
    }
}

impl SynthBoundaryLocal {
    pub(crate) fn from_wire_spelling(spelling: String) -> Self {
        debug_assert!(is_reserved_local_spelling(&spelling));
        Self(spelling)
    }

    pub fn derive(
        direction: BoundaryPortDirection,
        inner_local: &str,
        inner_port: &PortId,
    ) -> Result<Self, BoundaryPortIdError> {
        Self::derive_from_spelling(direction, inner_local, inner_port.as_str())
    }

    fn derive_from_spelling(
        direction: BoundaryPortDirection,
        inner_local: &str,
        inner_port: &str,
    ) -> Result<Self, BoundaryPortIdError> {
        let inner = Value::object([
            ("local", Value::String(inner_local.to_owned())),
            ("port", Value::String(inner_port.to_owned())),
        ])
        .map_err(|error| BoundaryPortIdError::Codec(format!("inner endpoint: {error:?}")))?;
        let identity = Value::object([
            ("direction", Value::Int(direction.tag())),
            ("inner", inner),
            (
                "version",
                Value::Int(i64::from(SYNTH_BOUNDARY_LOCAL_VERSION)),
            ),
        ])
        .map_err(|error| {
            BoundaryPortIdError::Codec(format!("synth boundary local identity: {error:?}"))
        })?;

        let mut spelling = String::with_capacity(32);
        spelling.push_str(direction.synth_local_prefix());
        spelling.push_str(&digest_hex26(&identity, "synth boundary local identity")?);
        Ok(Self(spelling))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A versioned, reserved [`PortId`] derived from one exact authored actor key.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BoundaryPortId(PortId);

impl BoundaryPortId {
    /// Derive the physical spelling from the complete structured actor identity.
    ///
    /// Thirteen SHA-256 bytes are rendered as 26 lowercase hex characters.  With
    /// the five-byte domain/version prefix the result is 31 bytes, below the
    /// `PortId` ceiling.  Truncation is not claimed collision-free: the producer
    /// must call [`validate_boundary_port_ids`] over the complete interface.
    pub fn derive(
        direction: BoundaryPortDirection,
        actor: &PlanActorKey,
        generation: BoundaryActorGeneration,
    ) -> Result<Self, BoundaryPortIdError> {
        validate_actor_key(actor)?;
        let actor = Value::object([
            ("local", Value::String(actor.local.as_str().to_owned())),
            ("scope", scope_identity_value(&actor.scope)),
        ])
        .map_err(|error| BoundaryPortIdError::Codec(format!("actor identity: {error:?}")))?;
        let identity = Value::object([
            ("direction", Value::Int(direction.tag())),
            (
                "generation",
                Value::Bytes(generation.get().to_be_bytes().to_vec()),
            ),
            ("actor", actor),
            ("version", Value::Int(i64::from(BOUNDARY_PORT_ID_VERSION))),
        ])
        .map_err(|error| BoundaryPortIdError::Codec(format!("boundary identity: {error:?}")))?;
        let mut spelling = String::with_capacity(31);
        spelling.push_str(direction.prefix());
        spelling.push_str(&digest_hex26(&identity, "boundary identity")?);
        let port = PortId::try_derived(spelling)
            .map_err(|error| BoundaryPortIdError::Codec(error.to_string()))?;
        Ok(Self(port))
    }

    #[must_use]
    pub const fn as_port_id(&self) -> &PortId {
        &self.0
    }

    #[must_use]
    pub fn into_port_id(self) -> PortId {
        self.0
    }
}

/// Strict physical carrier for the exact prospective authored identity used by
/// boundary admission.  It is the same tagless product used by declaration
/// payloads, without inventing a query-local string spelling.
pub fn encode_boundary_actor_key(actor: &PlanActorKey) -> Result<Value, BoundaryPortIdError> {
    validate_actor_key(actor)?;
    Value::object([
        ("local", Value::String(actor.local.as_str().to_owned())),
        ("scope", scope_identity_value(&actor.scope)),
    ])
    .map_err(|error| BoundaryPortIdError::Codec(format!("actor identity: {error:?}")))
}

/// Decode the same strict declaration identity product. Unknown fields,
/// malformed scope arms, and non-canonical address carriers fail closed in the
/// protocol decoder.
pub fn decode_boundary_actor_key(value: Value) -> Result<PlanActorKey, BoundaryPortIdError> {
    match decode_actor_address(Value::array([Value::Int(1), value]))
        .map_err(|error| BoundaryPortIdError::Codec(format!("actor identity: {error:?}")))?
    {
        AddressRef::Absolute(actor) => {
            validate_actor_key(&actor)?;
            Ok(actor)
        }
        AddressRef::EpochLocal(_) | AddressRef::Relative(_) => {
            unreachable!("the wrapper supplied the canonical absolute address arm")
        }
    }
}

/// Validate injectivity over one complete child interface.
///
/// A collision is terminal.  Suffixing or label-based fallback would change
/// identity when an unrelated boundary is added.
pub fn validate_boundary_port_ids<'a>(
    boundaries: impl IntoIterator<
        Item = (
            BoundaryPortDirection,
            &'a PlanActorKey,
            BoundaryActorGeneration,
        ),
    >,
) -> Result<Vec<(BoundaryPortDirection, &'a PlanActorKey, BoundaryPortId)>, BoundaryPortIdError> {
    let assignments = boundaries
        .into_iter()
        .map(|(direction, actor, generation)| {
            BoundaryPortId::derive(direction, actor, generation).map(|id| (direction, actor, id))
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_boundary_port_id_assignments(assignments)
}

fn validate_boundary_port_id_assignments<'a>(
    assignments: impl IntoIterator<Item = (BoundaryPortDirection, &'a PlanActorKey, BoundaryPortId)>,
) -> Result<Vec<(BoundaryPortDirection, &'a PlanActorKey, BoundaryPortId)>, BoundaryPortIdError> {
    let mut occupied: BTreeMap<BoundaryPortId, (BoundaryPortDirection, &'a PlanActorKey)> =
        BTreeMap::new();
    let mut validated = Vec::new();
    for (direction, actor, id) in assignments {
        if let Some((existing_direction, existing_actor)) = occupied.get(&id) {
            if *existing_direction != direction || *existing_actor != actor {
                return Err(BoundaryPortIdError::Collision {
                    id,
                    first: Box::new((*existing_direction, (*existing_actor).clone())),
                    second: Box::new((direction, actor.clone())),
                });
            }
        } else {
            occupied.insert(id.clone(), (direction, actor));
        }
        validated.push((direction, actor, id));
    }
    Ok(validated)
}

fn validate_actor_key(actor: &PlanActorKey) -> Result<(), BoundaryPortIdError> {
    if actor.local.as_str().is_empty() {
        return Err(BoundaryPortIdError::EmptyActorLocal);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoundaryPortIdError {
    EmptyActorLocal,
    Codec(String),
    Collision {
        id: BoundaryPortId,
        first: Box<(BoundaryPortDirection, PlanActorKey)>,
        second: Box<(BoundaryPortDirection, PlanActorKey)>,
    },
}

impl fmt::Display for BoundaryPortIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyActorLocal => formatter.write_str("boundary actor local identity is empty"),
            Self::Codec(detail) => write!(formatter, "boundary port identity codec: {detail}"),
            Self::Collision { id, first, second } => write!(
                formatter,
                "boundary port digest collision at {} between {:?} and {:?}",
                id.as_port_id(),
                first,
                second
            ),
        }
    }
}

impl Error for BoundaryPortIdError {}

circular_core::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum BoundaryActivationRejection {
        BoundaryDisagreesWithDerivation => "boundary_disagrees_with_derivation",
        StaleBoundaryGeneration => "stale_boundary_generation",
        BoundaryLeafUnresolved => "boundary_leaf_unresolved",
    }
}

#[must_use]
pub fn encode_boundary_activation_rejection(reason: BoundaryActivationRejection) -> Value {
    Value::String(reason.as_str().to_owned())
}

pub fn decode_boundary_activation_rejection(
    value: &Value,
) -> Result<BoundaryActivationRejection, BoundaryActivationRejectionCodecError> {
    let Value::String(spelling) = value else {
        return Err(BoundaryActivationRejectionCodecError::WrongCarrier);
    };
    BoundaryActivationRejection::from_str(spelling)
        .ok_or(BoundaryActivationRejectionCodecError::UnknownSpelling)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryActivationRejectionCodecError {
    WrongCarrier,
    UnknownSpelling,
}

impl fmt::Display for BoundaryActivationRejectionCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongCarrier => formatter.write_str("boundary rejection is not a String"),
            Self::UnknownSpelling => formatter.write_str("unknown boundary rejection spelling"),
        }
    }
}

impl Error for BoundaryActivationRejectionCodecError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration_payload::{InstanceKey, ScalarKey, ScopeSegment};

    fn actor(scope: &[&str], local: &str) -> PlanActorKey {
        PlanActorKey {
            scope: scope
                .iter()
                .map(|name| ScopeSegment::Child((*name).to_owned()))
                .collect(),
            local: crate::scope_identity::AuthoredLocal::try_new(local)
                .expect("test actor local is authored")
                .into(),
        }
    }

    #[test]
    fn physical_spellings_match_the_typescript_known_answer_vectors() {
        let instance = PlanActorKey {
            scope: vec![
                ScopeSegment::Child("fleet".to_owned()),
                ScopeSegment::Instance {
                    of: "session-cell".to_owned(),
                    key: InstanceKey::Scalar(ScalarKey::Text("session-17".to_owned())),
                },
            ],
            local: crate::scope_identity::AuthoredLocal::try_new("output")
                .expect("test actor local is authored")
                .into(),
        };
        let vectors = [
            (
                BoundaryPortDirection::Inlet,
                actor(&[], "source"),
                BoundaryActorGeneration::initial(),
                "_bi1_33577e6b2e50666123d8270f2a",
            ),
            (
                BoundaryPortDirection::Inlet,
                actor(&["fleet", "session-cell"], "input"),
                BoundaryActorGeneration::initial(),
                "_bi1_ac4647791b31cb74eef38fdbde",
            ),
            (
                BoundaryPortDirection::Outlet,
                actor(&["fleet", "session-cell"], "output"),
                BoundaryActorGeneration::initial(),
                "_bo1_150474c65a280d64734cea3258",
            ),
            (
                BoundaryPortDirection::Outlet,
                instance,
                BoundaryActorGeneration::new(9),
                "_bo1_252ae83bffa5809a613ccc033a",
            ),
        ];

        for (direction, actor, generation, expected) in vectors {
            assert_eq!(
                BoundaryPortId::derive(direction, &actor, generation)
                    .unwrap()
                    .as_port_id()
                    .as_str(),
                expected,
            );
        }
    }

    /// The third vector uses non-ASCII actor and port names on purpose. The derived local
    /// is a hash of the spelling, so it pins how a name outside ASCII is derived.
    #[test]
    fn synth_boundary_locals_match_the_cross_language_known_answer_vectors() {
        let vectors = [
            (
                BoundaryPortDirection::Inlet,
                "source",
                "event",
                "_bni1_49acde745761d9598d1c726ec4",
            ),
            (
                BoundaryPortDirection::Outlet,
                "meter",
                "out",
                "_bno1_2a622b39ae28447eea187b5652",
            ),
            (
                BoundaryPortDirection::Inlet,
                "액터",
                "포트",
                "_bni1_6064a41f6a5b185491bbb2377f",
            ),
            (
                BoundaryPortDirection::Outlet,
                "_bni1_49acde745761d9598d1c726ec4",
                "relay",
                "_bno1_0a2689f81da7f117fed84f126d",
            ),
        ];

        for (direction, local, port, expected) in vectors {
            let actual = match PortId::try_new(port) {
                Ok(port) => SynthBoundaryLocal::derive(direction, local, &port).unwrap(),
                Err(_) => {
                    assert_eq!(port, "포트");
                    SynthBoundaryLocal::derive_from_spelling(direction, local, port).unwrap()
                }
            };
            assert_eq!(actual.as_str(), expected);
            assert_eq!(actual.as_str().len(), 32);
        }
    }

    #[test]
    fn only_the_bn_prefix_is_reserved_for_authored_actor_locals() {
        for reserved in ["_bn", "_bni1_digest", "_bno1_digest", "_bnanything"] {
            assert!(is_reserved_local_spelling(reserved));
        }
        for available in ["", "bn", "_b", "_bi1_digest", "actor_bn"] {
            assert!(!is_reserved_local_spelling(available));
        }
    }

    #[test]
    fn physical_spelling_is_versioned_directional_stable_and_canonical() {
        let source = actor(&["pipeline"], "request");
        let generation = BoundaryActorGeneration::initial();
        let inlet =
            BoundaryPortId::derive(BoundaryPortDirection::Inlet, &source, generation).unwrap();
        let again =
            BoundaryPortId::derive(BoundaryPortDirection::Inlet, &source, generation).unwrap();
        let outlet =
            BoundaryPortId::derive(BoundaryPortDirection::Outlet, &source, generation).unwrap();
        assert_eq!(inlet, again);
        assert_ne!(inlet, outlet);
        assert!(inlet.as_port_id().as_str().starts_with("_bi1_"));
        assert!(outlet.as_port_id().as_str().starts_with("_bo1_"));
        assert_eq!(inlet.as_port_id().as_str().len(), 31);
        assert_eq!(outlet.as_port_id().as_str().len(), 31);
    }

    #[test]
    fn structured_scope_and_local_both_participate_and_restart_rederivation_is_exact() {
        let first = actor(&["ab", "c"], "request");
        let ambiguous_text = actor(&["a", "bc"], "request");
        let other_local = actor(&["ab", "c"], "request-2");
        let generation = BoundaryActorGeneration::initial();
        let id = BoundaryPortId::derive(BoundaryPortDirection::Inlet, &first, generation).unwrap();
        assert_ne!(
            id,
            BoundaryPortId::derive(BoundaryPortDirection::Inlet, &ambiguous_text, generation)
                .unwrap()
        );
        assert_ne!(
            id,
            BoundaryPortId::derive(BoundaryPortDirection::Inlet, &other_local, generation).unwrap()
        );
        assert_eq!(
            id,
            BoundaryPortId::derive(BoundaryPortDirection::Inlet, &first.clone(), generation)
                .unwrap()
        );
        assert_ne!(
            id,
            BoundaryPortId::derive(
                BoundaryPortDirection::Inlet,
                &first,
                generation.retired().unwrap(),
            )
            .unwrap(),
            "delete/recreate must not silently rebind the old port identity"
        );
        assert_eq!(
            decode_boundary_actor_key(encode_boundary_actor_key(&first).unwrap()).unwrap(),
            first
        );
    }

    #[test]
    fn interface_validation_preserves_duplicate_identity_but_never_renames() {
        let first = actor(&[], "request");
        let values = validate_boundary_port_ids([
            (
                BoundaryPortDirection::Inlet,
                &first,
                BoundaryActorGeneration::initial(),
            ),
            (
                BoundaryPortDirection::Inlet,
                &first,
                BoundaryActorGeneration::initial(),
            ),
        ])
        .unwrap();
        assert_eq!(values[0].2, values[1].2);
        assert!(
            BoundaryPortId::derive(
                BoundaryPortDirection::Inlet,
                &actor(&[], ""),
                BoundaryActorGeneration::initial(),
            )
            .is_err()
        );

        let second = actor(&[], "other");
        let collision = BoundaryPortId::derive(
            BoundaryPortDirection::Inlet,
            &first,
            BoundaryActorGeneration::initial(),
        )
        .unwrap();
        assert!(matches!(
            validate_boundary_port_id_assignments([
                (BoundaryPortDirection::Inlet, &first, collision.clone()),
                (BoundaryPortDirection::Inlet, &second, collision),
            ]),
            Err(BoundaryPortIdError::Collision { first: left, second: right, .. })
                if left.1 == first && right.1 == second
        ));
    }

    #[test]
    fn boundary_activation_rejection_spellings_round_trip_only_in_protocol() {
        for reason in [
            BoundaryActivationRejection::BoundaryDisagreesWithDerivation,
            BoundaryActivationRejection::StaleBoundaryGeneration,
            BoundaryActivationRejection::BoundaryLeafUnresolved,
        ] {
            let encoded = encode_boundary_activation_rejection(reason);
            assert_eq!(decode_boundary_activation_rejection(&encoded), Ok(reason));
        }
        assert_eq!(
            decode_boundary_activation_rejection(&Value::String("other".to_owned())),
            Err(BoundaryActivationRejectionCodecError::UnknownSpelling)
        );
    }
}
