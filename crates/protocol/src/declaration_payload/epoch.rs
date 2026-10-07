
use circular_core::{Ceilings, Value};

use crate::declaration_payload::environment::{AuthoringEnvironment, decode_environment};
use crate::scope_identity::{AddressContext, ScopeAddress, decode_scope_address};
#[cfg(test)]
use crate::scope_identity::{
    AddressRef, InstanceKey, ScalarKey, ScopeSegment, decode_scope_identity, scope_identity_value,
};
use crate::wire_value::{PayloadRejection, bytes_of, decode_arm, exhausted, object, take};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExpectedRevision {
    Absent,
    At(Vec<u8>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BeginEpoch {
    pub scope: ScopeAddress,
    pub commit_id: Vec<u8>,
    pub expected_revision: ExpectedRevision,
    pub expected_environment: AuthoringEnvironment,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpochRef {
    pub epoch: Vec<u8>,
}

pub fn decode_expected_revision(value: Value) -> Result<ExpectedRevision, PayloadRejection> {
    let (tag, mut arguments) = decode_arm(value, "expected_revision")?;
    let revision = match arguments.len() {
        0 => None,
        1 => Some(bytes_of(
            arguments.pop().expect("one argument"),
            "expected_revision",
        )?),
        _ => {
            return Err(PayloadRejection::WrongCarrier {
                key: "expected_revision",
            });
        }
    };
    match (tag, revision) {
        (1, None) => Ok(ExpectedRevision::Absent),
        (2, Some(revision)) => Ok(ExpectedRevision::At(revision)),
        (other, _) => Err(PayloadRejection::UnknownArm { tag: other }),
    }
}

pub fn decode_begin_epoch(
    bytes: &[u8],
    context: AddressContext,
    ceilings: Ceilings,
) -> Result<BeginEpoch, PayloadRejection> {
    let mut fields = object(bytes, ceilings)?;
    let commit_id = bytes_of(take(&mut fields, "commit_id")?, "commit_id")?;
    let expected_environment = decode_environment(take(&mut fields, "expected_environment")?)?;
    let expected_revision = decode_expected_revision(take(&mut fields, "expected_revision")?)?;
    let scope = decode_scope_address(take(&mut fields, "scope")?)?;
    exhausted(fields)?;

    if !context.admits(&scope) {
        return Err(PayloadRejection::ArmNotAdmitted);
    }

    Ok(BeginEpoch {
        scope,
        commit_id,
        expected_revision,
        expected_environment,
    })
}

pub fn decode_epoch_ref(bytes: &[u8], ceilings: Ceilings) -> Result<EpochRef, PayloadRejection> {
    let mut fields = object(bytes, ceilings)?;
    let epoch = bytes_of(take(&mut fields, "epoch")?, "epoch")?;
    exhausted(fields)?;
    Ok(EpochRef { epoch })
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::Boundary;
    use circular_core::{ObjectValue, encode};

    const CEILINGS: Ceilings = Ceilings::for_boundary(Boundary::Wire);

    #[test]
    fn scope_identity_round_trips_through_the_wire_value() {
        let identities: [Vec<ScopeSegment>; 4] = [
            vec![],
            vec![ScopeSegment::Child("alpha".to_owned())],
            vec![
                ScopeSegment::Child("h".to_owned()),
                ScopeSegment::Instance {
                    of: "worker".to_owned(),
                    key: InstanceKey::Scalar(ScalarKey::Int(7)),
                },
                ScopeSegment::Instance {
                    of: "shard".to_owned(),
                    key: InstanceKey::Scalar(ScalarKey::Text("Ωmega".to_owned())),
                },
            ],
            vec![ScopeSegment::Instance {
                of: "flag".to_owned(),
                key: InstanceKey::Scalar(ScalarKey::Bool(true)),
            }],
        ];
        for identity in identities {
            assert_eq!(
                decode_scope_identity(scope_identity_value(&identity)),
                Ok(identity.clone()),
                "{identity:?}"
            );
        }
    }

    fn environment() -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                ("declaration_schema".to_owned(), Value::bytes(vec![1])),
                ("spec_set".to_owned(), Value::bytes(vec![3])),
            ])
            .expect("two axes"),
        )
    }

    fn begin_body(scope: Value) -> Vec<u8> {
        begin_body_with(scope, Value::Int(1), environment())
    }

    fn begin_body_with(scope: Value, revision: Value, environment: Value) -> Vec<u8> {
        let object = ObjectValue::try_from_entries([
            ("commit_id".to_owned(), Value::bytes(vec![1, 2, 3, 4])),
            ("expected_environment".to_owned(), environment),
            ("expected_revision".to_owned(), revision),
            ("scope".to_owned(), scope),
        ])
        .expect("keys do not overlap");
        encode(&Value::Object(object), CEILINGS).expect("encodes")
    }

    fn absolute(name: &str) -> Value {
        arm(1, &[name])
    }

    fn arm(tag: i64, names: &[&str]) -> Value {
        Value::Array(vec![
            Value::Int(tag),
            Value::Array(
                names
                    .iter()
                    .map(|name| {
                        Value::Array(vec![Value::Int(1), Value::String((*name).to_owned())])
                    })
                    .collect(),
            ),
        ])
    }

    #[test]
    fn a_begin_epoch_opens_with_the_published_spelling() {
        let decoded = decode_begin_epoch(
            &begin_body(absolute("root")),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("decodes");
        assert_eq!(decoded.commit_id, vec![1, 2, 3, 4]);
        assert_eq!(
            decoded.scope,
            AddressRef::Absolute(vec![ScopeSegment::Child("root".to_owned())])
        );
        assert_eq!(decoded.expected_revision, ExpectedRevision::Absent);
        assert_eq!(decoded.expected_environment.spec_set, vec![3]);
    }

    #[test]
    fn absent_is_a_value_and_not_a_missing_key() {
        let present = decode_begin_epoch(
            &begin_body_with(absolute("r"), Value::Int(1), environment()),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("Absent is a valid answer");
        assert_eq!(present.expected_revision, ExpectedRevision::Absent);

        let object = ObjectValue::try_from_entries([
            ("commit_id".to_owned(), Value::bytes(vec![1])),
            ("expected_environment".to_owned(), environment()),
            ("scope".to_owned(), absolute("r")),
        ])
        .expect("keys do not overlap");
        let bytes = encode(&Value::Object(object), CEILINGS).expect("encodes");
        assert_eq!(
            decode_begin_epoch(&bytes, AddressContext::Mutation, CEILINGS),
            Err(PayloadRejection::MissingKey("expected_revision"))
        );
    }

    #[test]
    fn an_expected_revision_at_carries_its_tag_and_digest() {
        let at = Value::Array(vec![Value::Int(2), Value::bytes(vec![0xab; 32])]);
        let decoded = decode_begin_epoch(
            &begin_body_with(absolute("r"), at, environment()),
            AddressContext::Mutation,
            CEILINGS,
        )
        .expect("decodes");
        assert_eq!(
            decoded.expected_revision,
            ExpectedRevision::At(vec![0xab; 32])
        );
    }

    #[test]
    fn the_three_address_arms_open_by_their_tags() {
        let one = vec![ScopeSegment::Child("a".to_owned())];
        for (tag, expected) in [
            (1, AddressRef::Absolute(one.clone())),
            (2, AddressRef::EpochLocal(one.clone())),
            (3, AddressRef::Relative(one.clone())),
        ] {
            assert_eq!(
                decode_scope_address(arm(tag, &["a"])).expect("opens"),
                expected
            );
        }
    }

    #[test]
    fn a_nested_scope_keeps_its_segments() {
        let decoded = decode_scope_address(arm(1, &["outer", "inner"])).expect("opens");
        assert_eq!(
            decoded,
            AddressRef::Absolute(vec![
                ScopeSegment::Child("outer".to_owned()),
                ScopeSegment::Child("inner".to_owned()),
            ])
        );
    }

    #[test]
    fn segment_order_is_the_hierarchy() {
        let forward = decode_scope_address(arm(1, &["a", "b"])).expect("opens");
        let backward = decode_scope_address(arm(1, &["b", "a"])).expect("opens");
        assert_ne!(forward, backward, "sorting merges the two scopes into one");
    }

    #[test]
    fn a_segment_name_must_be_text() {
        let value = Value::Array(vec![
            Value::Int(1),
            Value::Array(vec![Value::Array(vec![
                Value::Int(1),
                Value::bytes(b"a".to_vec()),
            ])]),
        ]);
        assert_eq!(
            decode_scope_address(value),
            Err(PayloadRejection::WrongCarrier { key: "segment" })
        );
    }

    #[test]
    fn an_instance_segment_carries_a_scalar_key() {
        let value = Value::Array(vec![
            Value::Int(1),
            Value::Array(vec![Value::Array(vec![
                Value::Int(2),
                Value::String("grid".to_owned()),
                Value::Int(7),
            ])]),
        ]);
        assert_eq!(
            decode_scope_address(value).expect("opens"),
            AddressRef::Absolute(vec![ScopeSegment::Instance {
                of: "grid".to_owned(),
                key: InstanceKey::Scalar(ScalarKey::Int(7)),
            }])
        );
    }

    #[test]
    fn an_unassigned_arm_tag_is_refused() {
        for tag in [0, 4, -1, i64::MAX] {
            assert_eq!(
                decode_scope_address(arm(tag, &["a"])),
                Err(PayloadRejection::UnknownArm { tag }),
                "tag {tag}"
            );
        }
    }

    #[test]
    fn each_context_partitions_the_arms() {
        let arms = [(1, "absolute"), (2, "epoch-local"), (3, "relative")];
        let expected = [
            (AddressContext::Mutation, [true, true, false]),
            (AddressContext::AcceptedHistory, [true, false, false]),
            (AddressContext::AuthoringSnapshot, [false, false, true]),
        ];
        for (context, admits) in expected {
            for ((tag, name), admitted) in arms.iter().zip(admits) {
                let outcome = decode_begin_epoch(&begin_body(arm(*tag, &["a"])), context, CEILINGS);
                assert_eq!(
                    outcome.is_ok(),
                    admitted,
                    "{context:?} {} {name}",
                    if admitted {
                        "must accept"
                    } else {
                        "must refuse"
                    }
                );
            }
        }
    }

    #[test]
    fn an_unknown_key_is_refused_rather_than_dropped() {
        let object = ObjectValue::try_from_entries([
            ("commit_id".to_owned(), Value::bytes(vec![1])),
            ("expected_environment".to_owned(), environment()),
            ("expected_revision".to_owned(), Value::Int(1)),
            ("scope".to_owned(), absolute("r")),
            ("surprise".to_owned(), Value::Int(1)),
        ])
        .expect("keys do not overlap");
        let bytes = encode(&Value::Object(object), CEILINGS).expect("encodes");
        assert_eq!(
            decode_begin_epoch(&bytes, AddressContext::Mutation, CEILINGS),
            Err(PayloadRejection::UnknownKey("surprise".to_owned()))
        );
    }

    #[test]
    fn a_missing_key_is_refused_rather_than_defaulted() {
        let object = ObjectValue::try_from_entries([
            ("commit_id".to_owned(), Value::bytes(vec![1])),
            ("expected_environment".to_owned(), environment()),
            ("scope".to_owned(), absolute("r")),
        ])
        .expect("keys do not overlap");
        let bytes = encode(&Value::Object(object), CEILINGS).expect("encodes");
        assert_eq!(
            decode_begin_epoch(&bytes, AddressContext::Mutation, CEILINGS),
            Err(PayloadRejection::MissingKey("expected_revision"))
        );
    }

    #[test]
    fn a_wrong_carrier_is_refused() {
        let object = ObjectValue::try_from_entries([
            ("commit_id".to_owned(), Value::String("1234".to_owned())),
            ("expected_environment".to_owned(), environment()),
            ("expected_revision".to_owned(), Value::Int(1)),
            ("scope".to_owned(), absolute("r")),
        ])
        .expect("keys do not overlap");
        let bytes = encode(&Value::Object(object), CEILINGS).expect("encodes");
        assert_eq!(
            decode_begin_epoch(&bytes, AddressContext::Mutation, CEILINGS),
            Err(PayloadRejection::WrongCarrier { key: "commit_id" })
        );
    }

    #[test]
    fn bytes_that_are_not_a_value_are_refused() {
        assert!(matches!(
            decode_begin_epoch(&[0xff, 0xff], AddressContext::Mutation, CEILINGS),
            Err(PayloadRejection::Codec(_))
        ));
    }

    #[test]
    fn an_epoch_reference_carries_exactly_one_key() {
        let object =
            ObjectValue::try_from_entries([("epoch".to_owned(), Value::bytes(vec![5, 6]))])
                .expect("one key");
        let bytes = encode(&Value::Object(object), CEILINGS).expect("encodes");
        assert_eq!(
            decode_epoch_ref(&bytes, CEILINGS).expect("decodes").epoch,
            vec![5, 6]
        );
    }
}
