
use circular_core::{Ceilings, CodecError, ObjectValue, Value, encode};

use crate::wire_value::{arm, unit_arm};

const PARTITION_SPELLINGS: [&str; 9] = [
    "SessionMechanics",
    "Declaration",
    "Query",
    "Subscription",
    "EventInjection",
    "LedgerTransition",
    "ReplayControl",
    "Experimental",
    "Lifecycle",
];

use crate::declaration_payload::scope_identity_value as scope_value;
pub use crate::declaration_payload::{InstanceKey, ScalarKey, ScopeSegment};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionRole {
    Reader,
    Writer { scope: Vec<ScopeSegment> },
    Operator,
}

impl SessionRole {
    #[must_use]
    pub const fn tag(&self) -> i64 {
        match self {
            Self::Reader => 1,
            Self::Writer { .. } => 2,
            Self::Operator => 4,
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Self::Reader | Self::Operator => unit_arm(self.tag()),
            Self::Writer { scope } => arm(self.tag(), scope_value(scope)),
        }
    }
}

circular_core::closed_table! {
    pub enum TransportTrust: i64 {
        LocalOwner = 1,
        LocalUser = 2,
        Remote = 3,
    }
}

fn role_array(roles: &[SessionRole], ceilings: Ceilings) -> Result<Value, CodecError> {
    let mut encoded = roles
        .iter()
        .map(|role| encode(&role.to_value(), ceilings).map(|bytes| (bytes, role.to_value())))
        .collect::<Result<Vec<_>, _>>()?;
    encoded.sort_by(|left, right| left.0.cmp(&right.0));
    encoded.dedup_by(|left, right| left.0 == right.0);
    Ok(Value::Array(
        encoded.into_iter().map(|(_, value)| value).collect(),
    ))
}

pub fn hello(
    protocol_version: u16,
    minor: u8,
    roles: &[SessionRole],
    ceilings: Ceilings,
) -> Result<Vec<u8>, CodecError> {
    let hello = ObjectValue::try_from_entries([
        ("features".to_owned(), Value::Object(feature_set(minor))),
        (
            "protocol_version".to_owned(),
            Value::Int(i64::from(protocol_version)),
        ),
        ("requested_roles".to_owned(), role_array(roles, ceilings)?),
    ])
    .expect("the three keys differ");

    encode(&Value::Object(hello), ceilings)
}

/// The published Hello fields after decoding, before policy or negotiation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedHello {
    pub protocol_version: u16,
    pub features: crate::FeatureSet<u8>,
    pub requested_roles: Vec<SessionRole>,
}

/// Decode the existing Hello carrier. This does not establish roles.
pub fn decode_hello(
    bytes: &[u8],
    ceilings: Ceilings,
) -> Result<DecodedHello, crate::wire_value::PayloadRejection> {
    use crate::wire_value::{
        PayloadRejection as Rejection, exhausted, int_of, object, object_fields, optional, take,
    };
    let mut fields = object(bytes, ceilings)?;
    let protocol_version = u16::try_from(int_of(
        take(&mut fields, "protocol_version")?,
        "protocol_version",
    )?)
    .map_err(|_| Rejection::BeyondWidth {
        key: "protocol_version",
    })?;
    let mut feature_fields = object_fields(take(&mut fields, "features")?, "features")?;
    let mut features = Vec::new();
    for (name, partition) in PARTITION_SPELLINGS.into_iter().zip(crate::Partition::ALL) {
        if let Some(value) = optional(&mut feature_fields, name) {
            let minor = u8::try_from(int_of(value, "features")?)
                .map_err(|_| Rejection::BeyondWidth { key: "features" })?;
            features.push((partition, minor));
        }
    }
    exhausted(feature_fields)?;
    let features =
        crate::FeatureSet::try_new(features).expect("each canonical partition is visited once");
    let Value::Array(role_values) = take(&mut fields, "requested_roles")? else {
        return Err(Rejection::WrongCarrier {
            key: "requested_roles",
        });
    };
    let mut requested_roles = Vec::new();
    let mut previous = None;
    for value in role_values {
        let encoded = encode(&value, ceilings).map_err(Rejection::Codec)?;
        if previous
            .as_ref()
            .is_some_and(|previous| previous >= &encoded)
        {
            return Err(Rejection::NotCanonical {
                key: "requested_roles",
            });
        }
        previous = Some(encoded);
        let (tag, mut args) = crate::wire_value::decode_arm(value, "requested_roles")?;
        let role = match (tag, args.len()) {
            (1, 0) => SessionRole::Reader,
            (4, 0) => SessionRole::Operator,
            (2, 1) => SessionRole::Writer {
                scope: crate::scope_identity::decode_scope_identity(args.remove(0))?,
            },
            (1 | 2 | 4, _) => {
                return Err(Rejection::WrongCarrier {
                    key: "requested_roles",
                });
            }
            _ => return Err(Rejection::UnknownArm { tag }),
        };
        requested_roles.push(role);
    }
    exhausted(fields)?;
    Ok(DecodedHello {
        protocol_version,
        features,
        requested_roles,
    })
}

pub fn established(
    protocol_version: u16,
    features: &crate::FeatureSet<u8>,
    roles: &[SessionRole],
    trust: TransportTrust,
    token: &crate::SessionToken,
    ceilings: Ceilings,
) -> Result<Vec<u8>, CodecError> {
    let established = ObjectValue::try_from_entries([
        (
            "features".to_owned(),
            Value::Object(
                ObjectValue::try_from_entries(
                    crate::Partition::ALL
                        .into_iter()
                        .zip(PARTITION_SPELLINGS)
                        .filter_map(|(partition, name)| {
                            features
                                .get(partition)
                                .map(|minor| (name.to_owned(), Value::Int(i64::from(*minor))))
                        }),
                )
                .expect("unique feature partitions"),
            ),
        ),
        (
            "protocol_version".to_owned(),
            Value::Int(i64::from(protocol_version)),
        ),
        ("roles".to_owned(), role_array(roles, ceilings)?),
        ("token".to_owned(), Value::bytes(token.as_bytes().to_vec())),
        ("trust".to_owned(), unit_arm(trust.tag())),
    ])
    .expect("the five keys differ");

    encode(&Value::Object(established), ceilings)
}

fn feature_set(minor: u8) -> ObjectValue {
    ObjectValue::try_from_entries(
        PARTITION_SPELLINGS
            .iter()
            .map(|spelling| ((*spelling).to_owned(), Value::Int(i64::from(minor)))),
    )
    .expect("the column spellings differ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::Boundary;
    use circular_core::decode;

    const CEILINGS: Ceilings = Ceilings::for_boundary(Boundary::Wire);

    fn decoded(bytes: &[u8]) -> ObjectValue {
        match decode(bytes, CEILINGS).expect("the body is a value") {
            Value::Object(object) => object,
            other => panic!("the establishment body is an object: {other:?}"),
        }
    }

    #[test]
    fn the_body_carries_exactly_the_published_keys() {
        let body = hello(1, 0, &[], CEILINGS).expect("encodes");
        let keys = decoded(&body).into_map().into_keys().collect::<Vec<_>>();
        assert_eq!(
            keys,
            ["features", "protocol_version", "requested_roles"],
            "differs from the published key set and order"
        );
    }

    #[test]
    fn the_feature_set_names_all_nine_partitions() {
        let body = hello(1, 3, &[], CEILINGS).expect("encodes");
        let Some(Value::Object(features)) = decoded(&body).into_map().remove("features") else {
            panic!("features is not an object");
        };
        let map = features.into_map();
        assert_eq!(map.len(), 9, "all nine partitions");
        for spelling in PARTITION_SPELLINGS {
            assert_eq!(
                map.get(spelling),
                Some(&Value::Int(3)),
                "the minor of {spelling} was not carried"
            );
        }
    }

    #[test]
    fn the_body_has_no_freedom_left() {
        let first = hello(1, 0, &[], CEILINGS).expect("encodes");
        let second = hello(1, 0, &[], CEILINGS).expect("encodes");
        assert_eq!(first, second);
    }
}

#[cfg(test)]
mod arm_with_argument_tests {
    use super::*;
    use circular_core::decode;

    const CEILINGS: Ceilings = Ceilings::for_boundary(circular_core::Boundary::Wire);

    fn roles(bytes: &[u8]) -> Vec<Value> {
        let Value::Object(object) = decode(bytes, CEILINGS).expect("it is a value") else {
            panic!("it is an object");
        };
        let Some(Value::Array(items)) = object.into_map().remove("requested_roles") else {
            panic!("not a sequence");
        };
        items
    }

    fn writer(name: &str) -> SessionRole {
        SessionRole::Writer {
            scope: vec![ScopeSegment::Child(name.to_owned())],
        }
    }

    #[test]
    fn an_arm_with_an_argument_is_an_array_led_by_its_tag() {
        let body = hello(1, 0, &[writer("root")], CEILINGS).expect("encodes");
        let items = roles(&body);
        assert_eq!(items.len(), 1);
        let Value::Array(arm) = &items[0] else {
            panic!(
                "an arm that carries an argument is a sequence: {:?}",
                items[0]
            );
        };
        assert_eq!(arm[0], Value::Int(2), "the head is the Writer tag");
        assert_eq!(
            arm[1],
            Value::Array(vec![Value::Array(vec![
                Value::Int(1),
                Value::String("root".to_owned())
            ])])
        );
    }

    #[test]
    fn scope_segments_keep_their_order() {
        let deep = SessionRole::Writer {
            scope: vec![
                ScopeSegment::Child("zeta".to_owned()),
                ScopeSegment::Child("alpha".to_owned()),
            ],
        };
        let body = hello(1, 0, &[deep], CEILINGS).expect("encodes");
        let Value::Array(arm) = &roles(&body)[0] else {
            panic!("it is a sequence");
        };
        let Value::Array(segments) = &arm[1] else {
            panic!("a scope is a sequence");
        };
        let names = segments
            .iter()
            .map(|segment| match segment {
                Value::Array(parts) => parts[1].clone(),
                other => panic!("a segment is a sequence: {other:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                Value::String("zeta".to_owned()),
                Value::String("alpha".to_owned())
            ],
            "sorting the segments would erase the hierarchy"
        );
    }

    #[test]
    fn an_instance_segment_carries_its_tag_name_and_key() {
        let role = SessionRole::Writer {
            scope: vec![ScopeSegment::Instance {
                of: "grid".to_owned(),
                key: InstanceKey::Scalar(ScalarKey::Int(7)),
            }],
        };
        let body = hello(1, 0, &[role], CEILINGS).expect("encodes");
        let Value::Array(arm) = &roles(&body)[0] else {
            panic!("it is a sequence");
        };
        assert_eq!(arm[0], Value::Int(2), "Writer tag");
        assert_eq!(
            arm[1],
            Value::Array(vec![Value::Array(vec![
                Value::Int(2),
                Value::String("grid".to_owned()),
                Value::Int(7)
            ])])
        );
    }

    #[test]
    fn all_three_roles_encode() {
        let all = [SessionRole::Reader, writer("w"), SessionRole::Operator];
        let body = hello(1, 0, &all, CEILINGS).expect("encodes");
        assert_eq!(roles(&body).len(), 3, "all three are carried");
    }
}

#[cfg(test)]
mod hello_decode_tests {
    use super::*;
    use crate::wire_value::PayloadRejection;

    fn body(roles: Vec<Value>) -> Vec<u8> {
        body_with(roles, Vec::new())
    }

    fn body_with(roles: Vec<Value>, extra: Vec<(&'static str, Value)>) -> Vec<u8> {
        let mut fields = vec![
            ("protocol_version", Value::Int(1)),
            (
                "features",
                Value::object([
                    ("SessionMechanics", Value::Int(0)),
                    ("Declaration", Value::Int(0)),
                    ("Query", Value::Int(0)),
                    ("Subscription", Value::Int(0)),
                    ("EventInjection", Value::Int(0)),
                    ("LedgerTransition", Value::Int(0)),
                    ("ReplayControl", Value::Int(0)),
                    ("Experimental", Value::Int(0)),
                    ("Lifecycle", Value::Int(0)),
                ])
                .unwrap(),
            ),
            ("requested_roles", Value::Array(roles)),
        ];
        fields.extend(extra);
        encode(
            &Value::object(fields).unwrap(),
            Ceilings::for_boundary(circular_core::Boundary::Wire),
        )
        .unwrap()
    }

    #[test]
    fn hello_decodes_the_three_published_role_literals() {
        let bytes = body(vec![
            Value::Int(1),
            Value::Int(4),
            Value::Array(vec![Value::Int(2), Value::Array(vec![])]),
        ]);
        let hello = decode_hello(
            &bytes,
            Ceilings::for_boundary(circular_core::Boundary::Wire),
        )
        .unwrap();
        assert_eq!(hello.protocol_version, 1);
        assert_eq!(hello.features.iter().count(), 9);
        assert_eq!(
            hello.requested_roles,
            [
                SessionRole::Reader,
                SessionRole::Operator,
                SessionRole::Writer { scope: vec![] },
            ]
        );
    }

    #[test]
    fn hello_refuses_duplicate_unsorted_unknown_and_malformed_roles() {
        for roles in [
            vec![Value::Int(1), Value::Int(1)],
            vec![Value::Int(4), Value::Int(1)],
            vec![Value::Int(6)],
            vec![Value::Int(2)],
            vec![Value::Array(vec![Value::Int(1), Value::Null])],
        ] {
            assert!(
                decode_hello(
                    &body(roles),
                    Ceilings::for_boundary(circular_core::Boundary::Wire)
                )
                .is_err()
            );
        }
    }
}
