
use circular_expr::{ConfigPath, EvalMode};
use circular_protocol::declaration_payload::PreprocessKind;

use crate::config::{ClosedTags, ConfigConstraint, ConfigSlot, ConfigSpace, Required, SnippetSlot};
use crate::ports::PortId;
use crate::types::{BaseShape, FieldMap, Shape};
use circular_core::{ObjectValue, Value};

#[must_use]
pub fn config_slots(kind: PreprocessKind) -> Vec<(ConfigPath, ConfigSlot)> {
    match kind {
        PreprocessKind::Map => vec![(
            key(crate::map_config::TRANSFORM),
            snippet_slot(EvalMode::Transform, crate::map_config::EVENT_BINDING),
        )],
        PreprocessKind::Filter => vec![(
            key(crate::filter_config::PREDICATE),
            snippet_slot(EvalMode::Predicate, crate::filter_config::EVENT_BINDING),
        )],
        PreprocessKind::Bang => Vec::new(),
        PreprocessKind::Parse => {
            let decoder = ConfigSlot::try_new(
                ConfigSpace::try_new(
                    Shape::Base(BaseShape::String),
                    ConfigConstraint::ClosedTags(
                        ClosedTags::try_from_members(crate::parse_config::DECODERS)
                            .expect("the three decoder names differ"),
                    ),
                )
                .expect("closed tags require the string shape"),
                Required::Mandatory,
                None,
            )
            .expect("mandatory decoder has no default to mismatch");
            vec![
                (
                    key(crate::parse_config::ParseConfig::ARGUMENTS),
                    ConfigSlot::try_new(
                        ConfigSpace::unconstrained(open_object()),
                        Required::Optional {
                            default: Value::Object(ObjectValue::new()),
                        },
                        None,
                    )
                    .expect("an empty object is inside the open object space"),
                ),
                (key(crate::parse_config::ParseConfig::DECODER), decoder),
                (
                    key(crate::parse_config::ParseConfig::FIELD),
                    mandatory(ConfigSpace::unconstrained(Shape::Base(BaseShape::String))),
                ),
            ]
        }
        PreprocessKind::Flatten => vec![(
            key(crate::flatten::AT),
            mandatory(
                ConfigSpace::try_new(
                    Shape::Array(Box::new(Shape::Any)),
                    ConfigConstraint::ExactPayloadPath,
                )
                .expect("flatten selector preserves the exact payload-path domain"),
            ),
        )],
    }
}

fn key(name: &'static str) -> ConfigPath {
    ConfigPath::root().join_key(name)
}

fn open_object() -> Shape {
    Shape::Object {
        fields: FieldMap::try_new(Vec::new()).expect("empty field map is unique"),
        open: true,
    }
}

fn mandatory(space: ConfigSpace) -> ConfigSlot {
    ConfigSlot::try_new(space, Required::Mandatory, None)
        .expect("mandatory config slots have no default to mismatch")
}

fn snippet_slot(mode: EvalMode, binding: &'static str) -> ConfigSlot {
    ConfigSlot::try_new(
        ConfigSpace::unconstrained(Shape::Base(BaseShape::String)),
        Required::Mandatory,
        Some(SnippetSlot::new(
            mode,
            vec![
                PortId::try_new(binding.to_owned())
                    .expect("a binding name is a canonical port name"),
            ]
            .into_boxed_slice(),
        )),
    )
    .expect("mandatory snippet slots have no default to mismatch")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mandatory_slots_are_exactly_what_the_acceptor_demands() {
        for kind in PreprocessKind::ALL {
            let mandatory = mandatory_keys(kind);
            let complete = sample_config(kind);
            assert!(
                accepts(kind, &complete),
                "{kind:?}: a config with every required field was refused"
            );
            for absent in &mandatory {
                let Value::Object(fields) = &complete else {
                    panic!("a preprocess config is an object")
                };
                let reduced = Value::Object(
                    ObjectValue::try_from_entries(
                        fields
                            .iter()
                            .filter(|(name, _)| name != absent)
                            .map(|(name, value)| (name.clone(), value.clone())),
                    )
                    .expect("the remaining keys stay unique"),
                );
                assert!(
                    !accepts(kind, &reduced),
                    "{kind:?}: accepted even without the required field {absent}"
                );
            }
        }
    }

    #[test]
    fn bang_publishes_no_slot_because_its_config_must_be_empty() {
        assert!(config_slots(PreprocessKind::Bang).is_empty());
        assert!(crate::bang::reject_nonempty_config(&Value::Object(ObjectValue::new())).is_ok());
        assert!(
            crate::bang::reject_nonempty_config(
                &Value::object([("at", Value::Null)]).expect("one-field object")
            )
            .is_err()
        );
    }

    fn mandatory_keys(kind: PreprocessKind) -> Vec<String> {
        config_slots(kind)
            .iter()
            .filter(|(_, slot)| matches!(slot.required(), Required::Mandatory))
            .map(|(path, _)| top_key(path))
            .collect()
    }

    fn top_key(path: &ConfigPath) -> String {
        let [segment] = path.segments() else {
            panic!("a preprocess field is one top-level key")
        };
        segment
            .as_key()
            .expect("a preprocess field is a key segment")
            .to_owned()
    }

    fn sample_config(kind: PreprocessKind) -> Value {
        Value::Object(
            ObjectValue::try_from_entries(
                config_slots(kind)
                    .iter()
                    .filter(|(_, slot)| matches!(slot.required(), Required::Mandatory))
                    .map(|(path, slot)| (top_key(path), sample_value(kind, slot))),
            )
            .expect("field names are unique"),
        )
    }

    fn sample_value(kind: PreprocessKind, slot: &ConfigSlot) -> Value {
        if slot.snippet().is_some() {
            return Value::String(
                match kind {
                    PreprocessKind::Filter => "true",
                    _ => crate::map_config::EVENT_BINDING,
                }
                .to_owned(),
            );
        }
        match slot.space().constraint() {
            Some(ConfigConstraint::ClosedTags(tags)) => Value::String(
                tags.iter()
                    .next()
                    .expect("a closed set is not empty")
                    .to_owned(),
            ),
            Some(ConfigConstraint::ExactPayloadPath) => Value::Array(Vec::new()),
            _ => Value::String("sample".to_owned()),
        }
    }

    fn accepts(kind: PreprocessKind, config: &Value) -> bool {
        match kind {
            PreprocessKind::Map => crate::map_config::accept_transform(config).is_ok(),
            PreprocessKind::Filter => crate::filter_config::accept_predicate(config).is_ok(),
            PreprocessKind::Bang => crate::bang::reject_nonempty_config(config).is_ok(),
            PreprocessKind::Parse => crate::parse_config::ParseConfig::from_value(config).is_ok(),
            PreprocessKind::Flatten => crate::flatten::FlattenConfig::from_value(config).is_ok(),
        }
    }
}
