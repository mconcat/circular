
use std::error::Error;
use std::fmt;

use circular_actors::capabilities::{ConfigPath, ObjectValue, Value, ValueKind};
use circular_core::{Boundary, Ceilings, CodecError};
use circular_plan::{ActorType, Config, ConfigRecord, ConfigValue, Name};
use circular_runtime::FoldedConfig;

pub use circular_core::CANONICAL_VALUE_TAG;

fn decode_canonical_scalar(tag: &Name, bytes: &[u8]) -> Result<Value, CanonicalScalarError> {
    if tag.as_str() != CANONICAL_VALUE_TAG {
        return Err(CanonicalScalarError::UnknownTag(tag.clone()));
    }
    circular_core::decode(bytes, Ceilings::for_boundary(Boundary::Config))
        .map_err(CanonicalScalarError::Codec)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CanonicalScalarError {
    UnknownTag(Name),
    Codec(CodecError),
}

impl fmt::Display for CanonicalScalarError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTag(tag) => write!(
                formatter,
                "scalar tag {} does not identify the canonical value codec; {}",
                circular_core::spelling::Quoted(tag.as_str()),
                circular_core::spelling::allowed([CANONICAL_VALUE_TAG])
            ),
            Self::Codec(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for CanonicalScalarError {}

const MAX_VALUE_DEPTH: usize = 64;

#[derive(Debug)]
pub enum ConfigConditionError {
    ScalarDecode {
        actor_type: ActorType,
        at: ConfigPath,
        tag: Name,
        source: CanonicalScalarError,
    },
    NonScalarDecoded {
        actor_type: ActorType,
        at: ConfigPath,
        tag: Name,
        actual: ValueKind,
    },
    ListIndexUnrepresentable {
        actor_type: ActorType,
        at: ConfigPath,
        index: usize,
    },
    ValueDepthExceeded {
        actor_type: ActorType,
        at: ConfigPath,
        limit: usize,
    },
}

impl ConfigConditionError {
    #[must_use]
    pub const fn actor_type(&self) -> ActorType {
        match self {
            Self::ScalarDecode { actor_type, .. }
            | Self::NonScalarDecoded { actor_type, .. }
            | Self::ListIndexUnrepresentable { actor_type, .. }
            | Self::ValueDepthExceeded { actor_type, .. } => *actor_type,
        }
    }
}

impl fmt::Display for ConfigConditionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ScalarDecode {
                actor_type,
                at,
                tag,
                source,
            } => write!(
                formatter,
                "config scalar {} at {at} for {actor_type} could not be decoded: {source}",
                circular_core::spelling::Quoted(tag.as_str())
            ),
            Self::NonScalarDecoded {
                actor_type,
                at,
                tag,
                actual,
            } => write!(
                formatter,
                "config scalar {} at {at} for {actor_type} decoded as non-scalar {actual}",
                circular_core::spelling::Quoted(tag.as_str())
            ),
            Self::ListIndexUnrepresentable {
                actor_type,
                at,
                index,
            } => write!(
                formatter,
                "config list index {index} below {at} for {actor_type} is not representable as a canonical path"
            ),
            Self::ValueDepthExceeded {
                actor_type,
                at,
                limit,
            } => write!(
                formatter,
                "config value below {at} for {actor_type} exceeds depth limit {limit}"
            ),
        }
    }
}

impl Error for ConfigConditionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ScalarDecode { source, .. } => Some(source),
            Self::NonScalarDecoded { .. }
            | Self::ListIndexUnrepresentable { .. }
            | Self::ValueDepthExceeded { .. } => None,
        }
    }
}

#[derive(Clone, Copy)]
enum ConfigAt<'a> {
    Root(&'a ConfigRecord),
    Value(&'a ConfigValue),
}

pub fn fold_config(
    actor_type: ActorType,
    config: &Config,
) -> Result<FoldedConfig, ConfigConditionError> {
    decode_at(
        actor_type,
        &ConfigPath::root(),
        ConfigAt::Root(config.record()),
    )
    .map(|value| FoldedConfig::minted(actor_type, value))
}

#[derive(Debug)]
enum ConfigValueError {
    ScalarDecode {
        at: ConfigPath,
        tag: Name,
        source: CanonicalScalarError,
    },
    NonScalarDecoded {
        at: ConfigPath,
        tag: Name,
        actual: ValueKind,
    },
    ListIndexUnrepresentable { at: ConfigPath, index: usize },
    ValueDepthExceeded { at: ConfigPath, limit: usize },
}

fn decode_at(
    actor_type: ActorType,
    at: &ConfigPath,
    value: ConfigAt<'_>,
) -> Result<Value, ConfigConditionError> {
    decode_value_at(at, value).map_err(|error| match error {
        ConfigValueError::ScalarDecode { at, tag, source } => ConfigConditionError::ScalarDecode {
            actor_type,
            at,
            tag,
            source,
        },
        ConfigValueError::NonScalarDecoded { at, tag, actual } => {
            ConfigConditionError::NonScalarDecoded {
                actor_type,
                at,
                tag,
                actual,
            }
        }
        ConfigValueError::ListIndexUnrepresentable { at, index } => {
            ConfigConditionError::ListIndexUnrepresentable {
                actor_type,
                at,
                index,
            }
        }
        ConfigValueError::ValueDepthExceeded { at, limit } => {
            ConfigConditionError::ValueDepthExceeded {
                actor_type,
                at,
                limit,
            }
        }
    })
}

/// Inlet config uses the same bounds and canonical scalar carrier as actor config,
/// without manufacturing an actor registration or a FoldedConfig witness.
pub(crate) fn fold_preprocess_config(config: &Config) -> Result<Value, String> {
    decode_value_at(&ConfigPath::root(), ConfigAt::Root(config.record()))
        .map_err(|error| format!("{error:?}"))
}

fn decode_value_at(at: &ConfigPath, value: ConfigAt<'_>) -> Result<Value, ConfigValueError> {
    enum DecodeFrame<'config> {
        Enter {
            at: ConfigPath,
            value: ConfigAt<'config>,
            depth: usize,
            child_visit: bool,
        },
        ListNext {
            at: ConfigPath,
            values: &'config [ConfigValue],
            index: usize,
            child_depth: usize,
            decoded_start: usize,
        },
        RecordNext {
            at: ConfigPath,
            record: &'config ConfigRecord,
            index: usize,
            child_depth: usize,
            decoded_start: usize,
        },
    }

    let mut work = vec![DecodeFrame::Enter {
        at: at.clone(),
        value,
        depth: 0,
        child_visit: false,
    }];
    let mut decoded = Vec::new();

    while let Some(frame) = work.pop() {
        match frame {
            DecodeFrame::Enter {
                at,
                value,
                depth,
                child_visit,
            } => {
                if child_visit && depth > MAX_VALUE_DEPTH {
                    return Err(ConfigValueError::ValueDepthExceeded {
                        at,
                        limit: MAX_VALUE_DEPTH,
                    });
                }

                match value {
                    ConfigAt::Root(record) | ConfigAt::Value(ConfigValue::Record(record)) => {
                        work.push(DecodeFrame::RecordNext {
                            at,
                            record,
                            index: 0,
                            child_depth: depth + 1,
                            decoded_start: decoded.len(),
                        });
                    }
                    ConfigAt::Value(ConfigValue::List(values)) => {
                        work.push(DecodeFrame::ListNext {
                            at,
                            values,
                            index: 0,
                            child_depth: depth + 1,
                            decoded_start: decoded.len(),
                        });
                    }
                    ConfigAt::Value(ConfigValue::Scalar { tag, bytes }) => {
                        let value = decode_canonical_scalar(tag, bytes).map_err(|source| {
                            ConfigValueError::ScalarDecode {
                                at: at.clone(),
                                tag: tag.clone(),
                                source,
                            }
                        })?;
                        match value.kind() {
                            ValueKind::Array | ValueKind::Object => {
                                return Err(ConfigValueError::NonScalarDecoded {
                                    at,
                                    tag: tag.clone(),
                                    actual: value.kind(),
                                });
                            }
                            ValueKind::Null
                            | ValueKind::Bool
                            | ValueKind::Int
                            | ValueKind::UInt
                            | ValueKind::Float
                            | ValueKind::String
                            | ValueKind::Bytes => decoded.push(value),
                        }
                    }
                }
            }
            DecodeFrame::ListNext {
                at,
                values,
                index,
                child_depth,
                decoded_start,
            } => {
                if let Some(value) = values.get(index) {
                    let natural = u64::try_from(index).map_err(|_| {
                        ConfigValueError::ListIndexUnrepresentable {
                            at: at.clone(),
                            index,
                        }
                    })?;
                    work.push(DecodeFrame::ListNext {
                        at: at.clone(),
                        values,
                        index: index + 1,
                        child_depth,
                        decoded_start,
                    });
                    work.push(DecodeFrame::Enter {
                        at: at.join_index(natural),
                        value: ConfigAt::Value(value),
                        depth: child_depth,
                        child_visit: true,
                    });
                } else {
                    let values = decoded.split_off(decoded_start);
                    decoded.push(Value::Array(values));
                }
            }
            DecodeFrame::RecordNext {
                at,
                record,
                index,
                child_depth,
                decoded_start,
            } => {
                if let Some((name, value)) = record.entries().get(index) {
                    work.push(DecodeFrame::RecordNext {
                        at: at.clone(),
                        record,
                        index: index + 1,
                        child_depth,
                        decoded_start,
                    });
                    work.push(DecodeFrame::Enter {
                        at: at.join_key(name.as_str()),
                        value: ConfigAt::Value(value),
                        depth: child_depth,
                        child_visit: true,
                    });
                } else {
                    let values = decoded.split_off(decoded_start);
                    let entries = record
                        .entries()
                        .iter()
                        .map(|(name, _)| name.as_str().to_owned())
                        .zip(values)
                        .collect();
                    decoded.push(Value::Object(ObjectValue::from_map(entries)));
                }
            }
        }
    }

    debug_assert_eq!(decoded.len(), 1);
    Ok(decoded.pop().expect("one root config value is decoded"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(value: &str) -> Name {
        Name::from_normalized(value)
    }

    fn scalar(tag: &str, bytes: impl Into<Box<[u8]>>) -> ConfigValue {
        ConfigValue::Scalar {
            tag: name(tag),
            bytes: bytes.into(),
        }
    }

    fn config(entries: Vec<(&str, ConfigValue)>) -> Config {
        Config::try_new(
            entries
                .into_iter()
                .map(|(key, value)| (name(key), value))
                .collect(),
        )
        .expect("test config keys are unique")
    }

    fn canonical(value: &Value) -> ConfigValue {
        scalar(
            CANONICAL_VALUE_TAG,
            circular_core::encode(value, Ceilings::for_boundary(Boundary::Config)).expect("encode"),
        )
    }

    #[test]
    fn broken_bytes_surface_as_a_codec_error() {
        let error = decode_canonical_scalar(&name(CANONICAL_VALUE_TAG), &[0xff, 0xff, 0xff])
            .expect_err("does not become a value");

        assert!(matches!(error, CanonicalScalarError::Codec(_)));
    }

    #[test]
    fn a_scalar_carrying_structure_is_rejected() {
        let structured = canonical(&Value::array([Value::string("kind")]));

        let error = fold_config(ActorType::Agent, &config(vec![("at", structured)]))
            .expect_err("a structure arrived in a scalar field");

        assert!(matches!(
            error,
            ConfigConditionError::NonScalarDecoded { .. }
        ));
    }

    #[test]
    fn config_fold_rejects_a_child_below_the_depth_limit() {
        let nested_config = |wraps: usize| {
            let mut nested = canonical(&Value::Bool(true));
            for _ in 0..wraps {
                nested = ConfigValue::List(vec![nested].into_boxed_slice());
            }
            config(vec![("deep", nested)])
        };
        fold_config(ActorType::Agent, &nested_config(MAX_VALUE_DEPTH - 1))
            .expect("a leaf exactly at the depth limit folds");
        let error = fold_config(ActorType::Agent, &nested_config(MAX_VALUE_DEPTH))
            .expect_err("a leaf one step below the depth limit is rejected");

        assert!(matches!(
            error,
            ConfigConditionError::ValueDepthExceeded {
                actor_type: ActorType::Agent,
                limit: MAX_VALUE_DEPTH,
                ..
            }
        ));
    }

    #[test]
    fn preprocess_config_preserves_canonical_scalars_and_structural_limits() {
        let canonical = |value: &Value| {
            scalar(
                CANONICAL_VALUE_TAG,
                circular_core::encode(value, Ceilings::for_boundary(Boundary::Config)).unwrap(),
            )
        };
        let valid = config(vec![(
            "items",
            ConfigValue::List(
                vec![canonical(&Value::Bool(true)), canonical(&Value::Null)].into_boxed_slice(),
            ),
        )]);
        assert_eq!(
            fold_preprocess_config(&valid).unwrap(),
            Value::object([("items", Value::Array(vec![Value::Bool(true), Value::Null]))]).unwrap()
        );
        let composite = config(vec![("bad", canonical(&Value::Array(Vec::new())))]);
        assert!(
            fold_preprocess_config(&composite)
                .unwrap_err()
                .contains("NonScalarDecoded")
        );
        let unknown = config(vec![("bad", scalar("unknown", [0]))]);
        assert!(
            fold_preprocess_config(&unknown)
                .unwrap_err()
                .contains("UnknownTag")
        );
        let mut nested = canonical(&Value::Null);
        for _ in 0..=MAX_VALUE_DEPTH {
            nested = ConfigValue::List(vec![nested].into_boxed_slice());
        }
        let deep = config(vec![("deep", nested)]);
        assert!(
            fold_preprocess_config(&deep)
                .unwrap_err()
                .contains("ValueDepthExceeded")
        );
    }
}

#[cfg(test)]
mod folded_root_tests {
    use super::*;
    use circular_plan::{Config, ConfigValue, Name};

    fn fold(config: &Config) -> Value {
        folded_config(config).into_value()
    }

    fn folded_config(config: &Config) -> circular_runtime::FoldedConfig {
        fold_config(ActorType::Counter, config)
            .expect("the test config fits the canonical encoding")
    }

    #[test]
    fn a_populated_config_keeps_the_object_root() {
        let value = circular_core::encode(
            &Value::int(7),
            circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
        )
        .expect("a small integer fits the canonical encoding");
        let config = Config::try_new(vec![(
            Name::from_normalized("k"),
            ConfigValue::Scalar {
                tag: Name::from_normalized(CANONICAL_VALUE_TAG),
                bytes: value.into_boxed_slice(),
            },
        )])
        .expect("one key");

        let folded = fold(&config);
        let Value::Object(fields) = &folded else {
            panic!("the root is an object even with the key present");
        };
        assert_eq!(fields.len(), 1);
    }
}

pub(crate) fn factory_rejection_reason(
    actor: impl fmt::Display,
    folded: &FoldedConfig,
    error: &circular_actors::ProductFactoryError,
) -> String {
    circular_actors::config::config_rejection(actor, "config", Some(folded.value()), error)
}

pub(crate) fn admit_activation_config(
    actor: &circular_plan::NamedActorId,
    declaration: &circular_plan::ActorDecl,
    inlets: &circular_actors::ResolvedInletShapes,
) -> Result<(), crate::authoring_assembly::rejection::FoldRejection> {
    let actor_type = *declaration.domain().actor_type();
    let Ok(folded) = fold_config(actor_type, declaration.domain().config()) else {
        return Ok(());
    };
    circular_actors::judge_activation_config(actor_type, &folded, inlets).map_err(|error| {
        crate::authoring_assembly::rejection::FoldRejection::Registry(Box::new(
            crate::actor_registry::PlanRegistryError::ConfigFold {
                actor_type,
                detail: factory_rejection_reason(actor, &folded, &error),
                admission: error.detail().slot().map(|slot| {
                    Box::new(circular_actors::CreateInputAdmissionError::outside_space(
                        ConfigPath::root().join_key(slot),
                    ))
                }),
            },
        ))
    })
}

pub(crate) fn config_rejection(
    actor: impl fmt::Display,
    path: &str,
    config: &Config,
    reason: impl fmt::Display,
) -> String {
    match fold_preprocess_config(config) {
        Ok(value) => circular_actors::config::config_rejection(actor, path, Some(&value), reason),
        Err(_) => format!("ConfigRejected: actor `{actor}`; {path} = <unreadable>; {reason}"),
    }
}
