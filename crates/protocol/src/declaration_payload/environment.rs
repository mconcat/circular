
use circular_core::{Ceilings, Value, decode};

#[cfg(test)]
use super::epoch::decode_begin_epoch;
#[cfg(test)]
use crate::scope_identity::AddressContext;
use crate::wire_value::{PayloadRejection, bytes_of, exhausted, object_from_value, take};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthoringEnvironment {
    pub declaration_schema: Vec<u8>,
    pub spec_set: Vec<u8>,
}

pub fn decode_environment(value: Value) -> Result<AuthoringEnvironment, PayloadRejection> {
    let Value::Object(object) = value else {
        return Err(PayloadRejection::WrongCarrier {
            key: "expected_environment",
        });
    };
    let mut axes = object.into_map().into_iter().collect::<Vec<_>>();
    let declaration_schema =
        bytes_of(take(&mut axes, "declaration_schema")?, "declaration_schema")?;
    let spec_set = bytes_of(take(&mut axes, "spec_set")?, "spec_set")?;
    exhausted(axes)?;
    Ok(AuthoringEnvironment {
        declaration_schema,
        spec_set,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplaceAuthoringEnvironment {
    pub replacement: AuthoringEnvironment,
}

pub fn replace_authoring_environment_from_value(
    value: Value,
) -> Result<ReplaceAuthoringEnvironment, PayloadRejection> {
    let mut fields = object_from_value(value)?;
    let replacement = decode_environment(take(&mut fields, "replacement")?)?;
    exhausted(fields)?;
    Ok(ReplaceAuthoringEnvironment { replacement })
}

pub fn decode_replace_authoring_environment(
    bytes: &[u8],
    ceilings: Ceilings,
) -> Result<ReplaceAuthoringEnvironment, PayloadRejection> {
    replace_authoring_environment_from_value(
        decode(bytes, ceilings).map_err(PayloadRejection::Codec)?,
    )
}

#[cfg(test)]
mod replace_environment_tests {
    use super::*;
    use circular_core::{ObjectValue, encode};

    const CEILINGS: Ceilings = Ceilings::for_boundary(circular_core::Boundary::Wire);

    fn environment(schema: u8) -> Value {
        Value::Object(
            ObjectValue::try_from_entries([
                ("declaration_schema".to_owned(), Value::bytes(vec![schema])),
                ("spec_set".to_owned(), Value::bytes(vec![0xb3])),
            ])
            .expect("two axes"),
        )
    }

    fn body(entries: Vec<(&str, Value)>) -> Vec<u8> {
        encode(
            &Value::Object(
                ObjectValue::try_from_entries(
                    entries
                        .into_iter()
                        .map(|(key, value)| (key.to_owned(), value)),
                )
                .expect("keys do not overlap"),
            ),
            CEILINGS,
        )
        .expect("encodes")
    }

    #[test]
    fn a_replacement_carries_the_two_axes() {
        let decoded = decode_replace_authoring_environment(
            &body(vec![("replacement", environment(0xb1))]),
            CEILINGS,
        )
        .expect("decodes");
        assert_eq!(decoded.replacement.declaration_schema, vec![0xb1]);
        assert_eq!(decoded.replacement.spec_set, vec![0xb3]);
    }

    #[test]
    fn a_scope_address_is_not_a_place_this_command_has() {
        assert_eq!(
            decode_replace_authoring_environment(
                &body(vec![
                    ("replacement", environment(0xb1)),
                    (
                        "scope",
                        Value::Array(vec![Value::Int(1), Value::Array(Vec::new())])
                    ),
                ]),
                CEILINGS
            ),
            Err(PayloadRejection::UnknownKey("scope".to_owned()))
        );
    }

    #[test]
    fn a_partial_environment_is_rejected() {
        let one = Value::Object(
            ObjectValue::try_from_entries([(
                "declaration_schema".to_owned(),
                Value::bytes(vec![1]),
            )])
            .expect("one axis"),
        );
        assert_eq!(
            decode_replace_authoring_environment(&body(vec![("replacement", one)]), CEILINGS),
            Err(PayloadRejection::MissingKey("spec_set"))
        );
    }

    #[test]
    fn the_environment_carrier_is_the_one_begin_epoch_uses() {
        let replaced = decode_replace_authoring_environment(
            &body(vec![("replacement", environment(0xb1))]),
            CEILINGS,
        )
        .expect("decodes")
        .replacement;

        let begin = encode(
            &Value::Object(
                ObjectValue::try_from_entries([
                    ("commit_id".to_owned(), Value::bytes(vec![0xaa])),
                    ("expected_environment".to_owned(), environment(0xb1)),
                    ("expected_revision".to_owned(), Value::Int(1)),
                    (
                        "scope".to_owned(),
                        Value::Array(vec![
                            Value::Int(1),
                            Value::Array(vec![Value::Array(vec![
                                Value::Int(1),
                                Value::String("root".to_owned()),
                            ])]),
                        ]),
                    ),
                ])
                .expect("four keys"),
            ),
            CEILINGS,
        )
        .expect("encodes");
        let expected = decode_begin_epoch(&begin, AddressContext::Mutation, CEILINGS)
            .expect("decodes")
            .expected_environment;

        assert_eq!(
            replaced, expected,
            "the expectation and the replacement disagree"
        );
    }
}
