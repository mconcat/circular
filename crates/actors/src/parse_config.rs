
use std::collections::BTreeMap;

use circular_core::Value;

use crate::actor_registry::ProductPayload;

pub const DECODERS: [&str; 3] = ["json", "kv", "regex"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Decoder {
    Json,
    KeyValue {
        pair_separator: Box<str>,
        value_separator: Box<str>,
    },
    Regex { pattern: Box<str> },
}

impl Decoder {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::KeyValue { .. } => "kv",
            Self::Regex { .. } => "regex",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseConfig {
    decoder: Decoder,
    field: Box<str>,
}

impl ParseConfig {
    pub const DECODER: &'static str = "decoder";
    pub const FIELD: &'static str = "field";
    pub const ARGUMENTS: &'static str = "arguments";

    pub const PAIR_SEPARATOR: &'static str = "pair_separator";
    pub const VALUE_SEPARATOR: &'static str = "value_separator";
    pub const PATTERN: &'static str = "pattern";

    pub fn from_value(config: &Value) -> Result<Self, ParseConfigError> {
        let Some(root) = config.as_object() else {
            return Err(ParseConfigError::NotAnObject);
        };
        let name = root
            .get(Self::DECODER)
            .and_then(Value::as_str)
            .ok_or(ParseConfigError::MissingDecoder)?;
        let field = root
            .get(Self::FIELD)
            .and_then(Value::as_str)
            .ok_or(ParseConfigError::MissingField)?;
        if field.is_empty() {
            return Err(ParseConfigError::EmptyField);
        }
        let arguments = root.get(Self::ARGUMENTS);
        let arguments = match arguments {
            None => None,
            Some(value) => Some(
                value
                    .as_object()
                    .ok_or(ParseConfigError::ArgumentsNotAnObject)?,
            ),
        };
        let argument = |name: &str| -> Result<Box<str>, ParseConfigError> {
            arguments
                .and_then(|object| object.get(name))
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(Into::into)
                .ok_or(ParseConfigError::MissingArgument {
                    name: name.to_owned(),
                })
        };

        let decoder = match name {
            "json" => Decoder::Json,
            "kv" => Decoder::KeyValue {
                pair_separator: argument(Self::PAIR_SEPARATOR)?,
                value_separator: argument(Self::VALUE_SEPARATOR)?,
            },
            "regex" => Decoder::Regex {
                pattern: argument(Self::PATTERN)?,
            },
            _ => return Err(ParseConfigError::UnknownDecoder),
        };

        Ok(Self {
            decoder,
            field: field.into(),
        })
    }

    #[must_use]
    pub const fn decoder(&self) -> &Decoder {
        &self.decoder
    }

    #[must_use]
    pub fn field(&self) -> &str {
        &self.field
    }

    #[must_use]
    pub fn merge(&self, payload: &Value, decoded: &BTreeMap<String, Value>) -> Value {
        let mut merged: BTreeMap<String, Value> = payload
            .as_object()
            .map(|object| {
                object
                    .iter()
                    .filter(|(key, _)| *key != self.field())
                    .map(|(key, value)| ((*key).to_owned(), value.clone()))
                    .collect()
            })
            .unwrap_or_default();
        for (key, value) in decoded {
            merged.insert(key.clone(), value.clone());
        }
        Value::object(merged).expect("the map keys are already unique")
    }
}

pub type Decoded = BTreeMap<String, Value>;

impl ParseConfig {
    pub fn decode_with(
        &self,
        compiled: &CompiledDecoder,
        payload: &Value,
    ) -> Result<Decoded, DecodeFailure> {
        let field = payload
            .as_object()
            .and_then(|object| object.get(self.field()))
            .ok_or(DecodeFailure::FieldAbsent)?;
        let text = match (compiled, field) {
            (_, Value::String(text)) => text.as_str(),
            (CompiledDecoder::Json, Value::Bytes(bytes)) => {
                std::str::from_utf8(bytes).map_err(|_| DecodeFailure::FieldNotText)?
            }
            _ => return Err(DecodeFailure::FieldNotText),
        };

        match compiled {
            CompiledDecoder::Json => decode_json(text),
            CompiledDecoder::KeyValue {
                pair_separator,
                value_separator,
            } => Ok(decode_kv(text, pair_separator, value_separator)),
            CompiledDecoder::Regex(pattern) => decode_regex(pattern, text),
        }
    }
}

fn decode_json(text: &str) -> Result<Decoded, DecodeFailure> {
    let parsed: serde_json::Value =
        serde_json::from_str(text).map_err(|_| DecodeFailure::Malformed)?;
    let serde_json::Value::Object(fields) = parsed else {
        return Err(DecodeFailure::Malformed);
    };
    fields
        .into_iter()
        .map(|(key, value)| json_value(value).map(|value| (key, value)))
        .collect()
}

pub fn json_value(value: serde_json::Value) -> Result<Value, DecodeFailure> {
    Ok(match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(flag) => Value::Bool(flag),
        serde_json::Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                Value::int(integer)
            } else if let Some(float) = number.as_f64() {
                Value::float(float)
            } else {
                return Err(DecodeFailure::Malformed);
            }
        }
        serde_json::Value::String(text) => Value::string(text),
        serde_json::Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(json_value)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        serde_json::Value::Object(fields) => Value::object(
            fields
                .into_iter()
                .map(|(key, value)| json_value(value).map(|value| (key, value)))
                .collect::<Result<Vec<_>, _>>()?,
        )
        .map_err(|_| DecodeFailure::Malformed)?,
    })
}

fn decode_regex(pattern: &regex::Regex, text: &str) -> Result<Decoded, DecodeFailure> {
    let captures = pattern.captures(text).ok_or(DecodeFailure::NoMatch)?;
    Ok(pattern
        .capture_names()
        .flatten()
        .filter_map(|name| {
            captures
                .name(name)
                .map(|matched| (name.to_owned(), Value::string(matched.as_str())))
        })
        .collect())
}

fn decode_kv(text: &str, pair_separator: &str, value_separator: &str) -> Decoded {
    text.split(pair_separator)
        .filter_map(|pair| pair.split_once(value_separator))
        .filter(|(key, _)| !key.is_empty())
        .map(|(key, value)| (key.to_owned(), Value::string(value)))
        .collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseConfigError {
    NotAnObject,
    MissingDecoder,
    UnknownDecoder,
    MissingField,
    EmptyField,
    ArgumentsNotAnObject,
    MissingArgument {
        name: String,
    },
}

impl core::fmt::Display for ParseConfigError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotAnObject => formatter.write_str("parse config is not an object"),
            Self::MissingDecoder => formatter.write_str("parse config is missing decoder"),
            Self::UnknownDecoder => {
                formatter.write_str("decoder is not one of the three supported variants")
            }
            Self::MissingField => formatter.write_str("parse config is missing field"),
            Self::EmptyField => formatter.write_str("field name is empty"),
            Self::ArgumentsNotAnObject => formatter.write_str("arguments is not an object"),
            Self::MissingArgument { name } => {
                write!(formatter, "missing required decoder argument {name}")
            }
        }
    }
}

impl std::error::Error for ParseConfigError {}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DecodeFailure {
    FieldAbsent,
    FieldNotText,
    Malformed,
    NoMatch,
}

impl DecodeFailure {
    pub const ALL: [Self; 4] = [
        Self::FieldAbsent,
        Self::FieldNotText,
        Self::Malformed,
        Self::NoMatch,
    ];
}

#[derive(Clone, Debug)]
pub enum CompiledDecoder {
    Json,
    KeyValue {
        pair_separator: Box<str>,
        value_separator: Box<str>,
    },
    Regex(Box<regex::Regex>),
}

impl CompiledDecoder {
    pub fn compile(decoder: &Decoder) -> Result<Self, PatternRejection> {
        Ok(match decoder {
            Decoder::Json => Self::Json,
            Decoder::KeyValue {
                pair_separator,
                value_separator,
            } => Self::KeyValue {
                pair_separator: pair_separator.clone(),
                value_separator: value_separator.clone(),
            },
            Decoder::Regex { pattern } => {
                let compiled = regex::Regex::new(pattern).map_err(|error| {
                    PatternRejection::NotThisDialect(error.to_string())
                })?;
                if compiled.capture_names().flatten().count() == 0 {
                    return Err(PatternRejection::NoNamedCaptures);
                }
                Self::Regex(Box::new(compiled))
            }
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PatternRejection {
    NotThisDialect(String),
    NoNamedCaptures,
}

impl core::fmt::Display for PatternRejection {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotThisDialect(detail) => {
                write!(formatter, "pattern is not valid in this dialect: {detail}")
            }
            Self::NoNamedCaptures => {
                formatter.write_str("no named captures are available to produce fields")
            }
        }
    }
}

impl std::error::Error for PatternRejection {}

/// Decode and merge one payload. Failure is carried by the inlet envelope;
/// it must never become a successful pass-through payload.
pub fn parse_event(
    config: &ParseConfig,
    compiled: &CompiledDecoder,
    payload: &ProductPayload,
) -> Result<ProductPayload, DecodeFailure> {
    let decoded = config.decode_with(compiled, payload.value())?;
    let shape = crate::GroundShape::try_new(crate::Shape::Object {
        fields: crate::FieldMap::try_new(Vec::new()).expect("empty fields"),
        open: true,
    })
    .expect("open object is ground");
    Ok(ProductPayload::new(
        shape,
        config.merge(payload.value(), &decoded),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compiled_config(value: &Value) -> (ParseConfig, CompiledDecoder) {
        let config = ParseConfig::from_value(value).expect("the test config is valid");
        let compiled = CompiledDecoder::compile(config.decoder())
            .expect("the test pattern is in this dialect");
        (config, compiled)
    }

    fn transformed_value(
        config: &ParseConfig,
        compiled: &CompiledDecoder,
        payload: &Value,
    ) -> Value {
        let decoded = config
            .decode_with(compiled, payload)
            .expect("successful decode");
        config.merge(payload, &decoded)
    }

    fn config(entries: &[(&str, Value)]) -> Value {
        Value::object(
            entries
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.clone())),
        )
        .expect("the test keys are unique")
    }

    fn arguments(entries: &[(&str, &str)]) -> Value {
        config(
            &entries
                .iter()
                .map(|(key, value)| (*key, Value::string(*value)))
                .collect::<Vec<_>>(),
        )
    }

    #[test]
    fn the_three_decoders_decode_with_their_own_arguments() {
        let json = ParseConfig::from_value(&config(&[
            (ParseConfig::DECODER, Value::string("json")),
            (ParseConfig::FIELD, Value::string("body")),
        ]))
        .expect("json takes no argument");
        assert_eq!(json.decoder(), &Decoder::Json);
        assert_eq!(json.field(), "body");

        let kv = ParseConfig::from_value(&config(&[
            (ParseConfig::DECODER, Value::string("kv")),
            (ParseConfig::FIELD, Value::string("line")),
            (
                ParseConfig::ARGUMENTS,
                arguments(&[
                    (ParseConfig::PAIR_SEPARATOR, " "),
                    (ParseConfig::VALUE_SEPARATOR, "="),
                ]),
            ),
        ]))
        .expect("kv carries two delimiters");
        assert_eq!(
            kv.decoder(),
            &Decoder::KeyValue {
                pair_separator: " ".into(),
                value_separator: "=".into(),
            }
        );

        let regex = ParseConfig::from_value(&config(&[
            (ParseConfig::DECODER, Value::string("regex")),
            (ParseConfig::FIELD, Value::string("path")),
            (
                ParseConfig::ARGUMENTS,
                arguments(&[(ParseConfig::PATTERN, "(?P<repo>[^/]+)$")]),
            ),
        ]))
        .expect("regex carries a pattern");
        assert_eq!(
            regex.decoder(),
            &Decoder::Regex {
                pattern: "(?P<repo>[^/]+)$".into()
            }
        );

        for decoder in [json.decoder(), kv.decoder(), regex.decoder()] {
            assert!(DECODERS.contains(&decoder.name()), "{}", decoder.name());
        }
    }

    #[test]
    fn a_decoder_without_its_arguments_is_rejected_at_activation() {
        for (decoder, missing) in [
            ("kv", ParseConfig::PAIR_SEPARATOR),
            ("regex", ParseConfig::PATTERN),
        ] {
            let error = ParseConfig::from_value(&config(&[
                (ParseConfig::DECODER, Value::string(decoder)),
                (ParseConfig::FIELD, Value::string("body")),
            ]))
            .expect_err("an arm without its argument does not build");
            assert_eq!(
                error,
                ParseConfigError::MissingArgument {
                    name: missing.to_owned()
                }
            );
        }

        assert_eq!(
            ParseConfig::from_value(&config(&[
                (ParseConfig::DECODER, Value::string("kv")),
                (ParseConfig::FIELD, Value::string("body")),
                (
                    ParseConfig::ARGUMENTS,
                    arguments(&[
                        (ParseConfig::PAIR_SEPARATOR, ""),
                        (ParseConfig::VALUE_SEPARATOR, "="),
                    ])
                ),
            ])),
            Err(ParseConfigError::MissingArgument {
                name: ParseConfig::PAIR_SEPARATOR.to_owned()
            })
        );
    }

    #[test]
    fn every_config_rejection_is_its_own_arm() {
        assert_eq!(
            ParseConfig::from_value(&Value::int(1)),
            Err(ParseConfigError::NotAnObject)
        );
        assert_eq!(
            ParseConfig::from_value(&config(&[(ParseConfig::FIELD, Value::string("b"))])),
            Err(ParseConfigError::MissingDecoder)
        );
        assert_eq!(
            ParseConfig::from_value(&config(&[(ParseConfig::DECODER, Value::string("json"))])),
            Err(ParseConfigError::MissingField)
        );
        assert_eq!(
            ParseConfig::from_value(&config(&[
                (ParseConfig::DECODER, Value::string("json")),
                (ParseConfig::FIELD, Value::string("")),
            ])),
            Err(ParseConfigError::EmptyField)
        );
        assert_eq!(
            ParseConfig::from_value(&config(&[
                (ParseConfig::DECODER, Value::string("yaml")),
                (ParseConfig::FIELD, Value::string("b")),
            ])),
            Err(ParseConfigError::UnknownDecoder)
        );
        assert_eq!(
            ParseConfig::from_value(&config(&[
                (ParseConfig::DECODER, Value::string("json")),
                (ParseConfig::FIELD, Value::string("b")),
                (ParseConfig::ARGUMENTS, Value::int(1)),
            ])),
            Err(ParseConfigError::ArgumentsNotAnObject)
        );
    }

    #[test]
    fn a_decoded_field_named_like_the_source_field_is_a_result_not_the_source() {
        let (parse_config, compiled) = compiled_config(&config(&[
            (ParseConfig::DECODER, Value::string("json")),
            (ParseConfig::FIELD, Value::string("body")),
        ]));
        let payload = config(&[("body", Value::string("{\"body\":\"decoded\"}"))]);
        let output = parse_event(
            &parse_config,
            &compiled,
            &ProductPayload::new(open_object(), payload),
        )
        .expect("successful decoding");
        assert_eq!(
            output
                .value()
                .as_object()
                .expect("the output is an object")
                .get("body")
                .and_then(Value::as_str),
            Some("decoded")
        );
    }

    #[test]
    fn the_four_failure_arms_are_recorded_but_do_not_split_the_emission() {
        assert_eq!(DecodeFailure::ALL.len(), 4);
        let distinct = DecodeFailure::ALL
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(distinct.len(), DecodeFailure::ALL.len());
        let spec = crate::get(crate::ActorType::Parse);
        assert!(
            spec.requires().is_empty(),
            "parse does not require a term-class capability"
        );
    }

    #[test]
    fn every_failure_arm_remains_an_error() {
        let (parse_config, compiled) = compiled_config(&config(&[
            (ParseConfig::DECODER, Value::string("json")),
            (ParseConfig::FIELD, Value::string("body")),
        ]));
        for payload in [
            config(&[("other", Value::string("{}"))]),
            config(&[("body", Value::int(1))]),
            config(&[("body", Value::string("{not json"))]),
            config(&[("body", Value::string("[1,2]"))]),
            config(&[("body", Value::Bytes(b"{\"n\":\xff}".to_vec()))]),
        ] {
            let input = ProductPayload::new(open_object(), payload);
            assert!(parse_event(&parse_config, &compiled, &input).is_err());
        }
        assert_eq!(
            parse_config.decode_with(
                &compiled,
                &config(&[("body", Value::Bytes(b"{\"n\":\xff}".to_vec()))])
            ),
            Err(DecodeFailure::FieldNotText)
        );
    }

    fn open_object() -> crate::GroundShape {
        crate::GroundShape::try_new(crate::Shape::Object {
            fields: crate::FieldMap::try_new(Vec::new()).expect("empty field table"),
            open: true,
        })
        .expect("an open object is ground")
    }

    #[test]
    fn a_nonobject_input_is_an_error_and_a_merge_has_object_shape() {
        let (config, compiled) = compiled_config(&config(&[
            (ParseConfig::DECODER, Value::string("json")),
            (ParseConfig::FIELD, Value::string("line")),
        ]));
        let any = crate::GroundShape::try_new(crate::Shape::Any).unwrap();
        let raw = ProductPayload::new(any.clone(), Value::string("{}"));
        assert!(parse_event(&config, &compiled, &raw).is_err());
        let input = ProductPayload::new(
            any,
            Value::object([("line", Value::string("{\"n\":1}"))]).unwrap(),
        );
        let output = parse_event(&config, &compiled, &input).unwrap();
        assert_eq!(output.shape(), &open_object());
        assert_eq!(
            output.value().as_object().unwrap().get("n"),
            Some(&Value::int(1))
        );
    }

    #[test]
    fn the_kv_decoder_collects_well_formed_pairs_and_skips_the_rest() {
        let (parse_config, compiled) = compiled_config(&config(&[
            (ParseConfig::DECODER, Value::string("kv")),
            (ParseConfig::FIELD, Value::string("line")),
            (
                ParseConfig::ARGUMENTS,
                arguments(&[
                    (ParseConfig::PAIR_SEPARATOR, " "),
                    (ParseConfig::VALUE_SEPARATOR, "="),
                ]),
            ),
        ]));
        let produced = transformed_value(
            &parse_config,
            &compiled,
            &config(&[("line", Value::string("level=info bare msg=hello"))]),
        );
        let object = produced.as_object().unwrap();
        assert_eq!(object.get("level").and_then(Value::as_str), Some("info"));
        assert_eq!(object.get("msg").and_then(Value::as_str), Some("hello"));
        assert!(
            object.get("bare").is_none(),
            "a piece without a delimiter is not a pair"
        );
        let numeric = transformed_value(
            &parse_config,
            &compiled,
            &config(&[("line", Value::string("n=1"))]),
        );
        assert_eq!(
            numeric.as_object().unwrap().get("n"),
            Some(&Value::string("1"))
        );
    }

    #[test]
    fn a_pattern_without_named_captures_cannot_stand() {
        let config = ParseConfig::from_value(&config(&[
            (ParseConfig::DECODER, Value::string("regex")),
            (ParseConfig::FIELD, Value::string("file")),
            (
                ParseConfig::ARGUMENTS,
                arguments(&[(ParseConfig::PATTERN, r"([^/]+)$")]),
            ),
        ]))
        .unwrap();
        assert!(matches!(
            CompiledDecoder::compile(config.decoder()),
            Err(PatternRejection::NoNamedCaptures)
        ));
    }
}
