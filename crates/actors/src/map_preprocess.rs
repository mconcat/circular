
use crate::actor_registry::ProductPayload;
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum MapFailure {
    Evaluation(circular_expr::eval::EvalError),
    OutputShapeNotGround {
        produced: circular_core::Shape<String>,
    },
}

impl MapFailure {
    pub const TRANSFORM_FAILED: &'static str = "transform_failed";
    pub const OUTPUT_SHAPE_UNRESOLVED: &'static str = "output_shape_unresolved";
    pub const REASONS: [&'static str; 2] = [Self::TRANSFORM_FAILED, Self::OUTPUT_SHAPE_UNRESOLVED];

    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::Evaluation(_) => Self::TRANSFORM_FAILED,
            Self::OutputShapeNotGround { .. } => Self::OUTPUT_SHAPE_UNRESOLVED,
        }
    }
}

impl fmt::Display for MapFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Evaluation(error) => {
                write!(formatter, "map transform produced no value: {error:?}")
            }
            Self::OutputShapeNotGround { produced } => {
                write!(formatter, "map output shape is not ground: {produced:?}")
            }
        }
    }
}

impl Error for MapFailure {}

pub fn map_event(
    transform: &circular_expr::snippet::Snippet,
    payload: &ProductPayload,
) -> Result<ProductPayload, MapFailure> {
    let bindings = std::collections::BTreeMap::from([(
        crate::map_config::EVENT_BINDING.to_owned(),
        payload.value().clone(),
    )]);
    let produced = transform
        .evaluate(&bindings)
        .map_err(MapFailure::Evaluation)?;
    let item = payload.shape().as_shape();
    let shape = crate::map_config::output_shape_of(transform, item).ok_or_else(|| {
        MapFailure::OutputShapeNotGround {
            produced: transform.output_shape(&crate::map_config::shape_env(item)),
        }
    })?;
    Ok(ProductPayload::new(shape, produced))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor_registry::ProductValue;
    use crate::route_actor::RouteActor;
    use crate::{BaseShape, Name, Shape};
    use circular_core::GroundShape;

    fn empty_config() -> ProductValue {
        ProductValue::object(std::iter::empty::<(String, ProductValue)>())
            .expect("an empty object has no duplicate keys")
    }

    fn route_actor(at: &[ProductValue], cases: &[(&str, ProductValue)]) -> RouteActor<u16, u64> {
        let at = crate::config::payload_path_from_value(&ProductValue::array(at.to_vec()))
            .expect("the test path is valid");
        let cases = crate::route_config::RouteCases::try_new(
            cases.iter().map(|(name, value)| (*name, value.clone())),
        )
        .expect("the test case table is valid");
        RouteActor::new(at, cases)
    }

    fn transform(source: &str) -> circular_expr::snippet::Snippet {
        crate::accept_transform(
            &ProductValue::object([(crate::map_config::TRANSFORM, ProductValue::string(source))])
                .expect("only one key"),
        )
        .expect("an accepted transform")
    }

    fn object_payload(entries: &[(&str, ProductValue, BaseShape)]) -> ProductPayload {
        let fields = crate::FieldMap::try_new(
            entries
                .iter()
                .map(|(name, _, base)| {
                    (
                        Name::from_normalized((*name).to_owned()),
                        Shape::Base(*base),
                    )
                })
                .collect::<Vec<_>>(),
        )
        .expect("field names are unique");
        ProductPayload::new(
            GroundShape::try_new(Shape::Object {
                fields,
                open: false,
            })
            .expect("ground object"),
            ProductValue::object(
                entries
                    .iter()
                    .map(|(name, value, _)| ((*name).to_owned(), value.clone())),
            )
            .expect("keys are unique"),
        )
    }

    #[test]
    fn map_takes_its_output_shape_from_the_same_snippet_that_makes_the_value() {
        let snippet = transform("{'doubled': event.n * 2, 'kind': event.kind}");
        let input = object_payload(&[
            ("n", ProductValue::int(21), BaseShape::Int),
            ("kind", ProductValue::string("x"), BaseShape::String),
        ]);

        let out = map_event(&snippet, &input).expect("both evaluation and shape hold");

        assert_eq!(
            out.value(),
            &ProductValue::object([
                ("doubled".to_owned(), ProductValue::int(42)),
                ("kind".to_owned(), ProductValue::string("x")),
            ])
            .unwrap()
        );
        let Shape::Object { fields, open } = out.shape().as_shape() else {
            panic!("an object term yields an object shape");
        };
        assert!(!open, "an object term yields a closed object");
        let named: Vec<(&str, &Shape)> = fields
            .as_slice()
            .iter()
            .map(|(name, shape)| (name.as_str(), shape))
            .collect();
        assert_eq!(
            named,
            vec![
                ("doubled", &Shape::Base(BaseShape::Int)),
                ("kind", &Shape::Base(BaseShape::String)),
            ]
        );
    }

    #[test]
    fn the_same_snippet_yields_a_different_shape_under_a_different_inlet_shape() {
        let snippet = transform("event.n");

        let from_int = map_event(
            &snippet,
            &object_payload(&[("n", ProductValue::int(1), BaseShape::Int)]),
        )
        .expect("integer inlet");
        let from_text = map_event(
            &snippet,
            &object_payload(&[("n", ProductValue::string("one"), BaseShape::String)]),
        )
        .expect("string inlet");

        assert_eq!(from_int.shape().as_shape(), &Shape::Base(BaseShape::Int));
        assert_eq!(
            from_text.shape().as_shape(),
            &Shape::Base(BaseShape::String)
        );
    }

    #[test]
    fn an_evaluation_does_not_take_a_missing_field_from_another_field() {
        let snippet = transform("event.n");
        let payload = object_payload(&[("other", ProductValue::int(1), BaseShape::Int)]);
        if let Ok(out) = map_event(&snippet, &payload) {
            assert_ne!(
                out.value(),
                &ProductValue::int(1),
                "a missing field does not pick up another field's value"
            );
        }
    }

    #[test]
    fn an_any_shaped_arrival_is_not_where_the_chain_breaks() {
        let snippet = transform("event.n");
        let payload = ProductPayload::new(
            GroundShape::try_new(Shape::Any).expect("Any holds no variable"),
            ProductValue::object([("n".to_owned(), ProductValue::int(7))]).expect("only one key"),
        );

        let out = map_event(&snippet, &payload).expect("map yields a value on an Any inlet too");
        assert_eq!(out.value(), &ProductValue::int(7));
        assert_eq!(
            out.shape().as_shape(),
            &Shape::Any,
            "the output shape is ground too"
        );
    }

    #[test]
    fn a_field_access_on_the_open_object_parse_emits_is_not_where_the_chain_breaks() {
        let snippet = transform("event.n");
        let payload = ProductPayload::new(
            GroundShape::try_new(Shape::Object {
                fields: crate::FieldMap::try_new(Vec::new()).expect("empty field table"),
                open: true,
            })
            .expect("an open object is ground"),
            ProductValue::object([("n".to_owned(), ProductValue::int(7))]).expect("only one key"),
        );

        let out = map_event(&snippet, &payload)
            .expect("map yields a value over the open object that parse produced");
        assert_eq!(out.value(), &ProductValue::int(7));
    }

    #[test]
    fn the_shape_arm_never_fires_across_the_surveyed_domain() {
        let shapes = [
            Shape::Any,
            Shape::Base(BaseShape::Int),
            Shape::Base(BaseShape::String),
            Shape::Array(Box::new(Shape::Any)),
            Shape::Object {
                fields: crate::FieldMap::try_new(Vec::new()).expect("empty field table"),
                open: true,
            },
        ];

        for source in ["event", "event.n", "event.n + 1", "[event]", "size(event)"] {
            let config = ProductValue::object([(
                crate::map_config::TRANSFORM.to_owned(),
                ProductValue::string(source),
            )])
            .expect("only one key");
            let transform =
                crate::map_config::accept_transform(&config).expect("it is an accepted transform");

            for shape in &shapes {
                assert!(
                    crate::map_config::output_shape_of(&transform, shape).is_some(),
                    "the output shape did not resolve at {source} @ {shape:?}; the shape arm was hit for the first time"
                );
            }
        }
    }

    #[test]
    fn a_transform_that_cannot_evaluate_reports_the_evaluation_arm() {
        let snippet = transform("event.n + 1");
        let payload =
            object_payload(&[("n", ProductValue::string("seven"), crate::BaseShape::String)]);

        match map_event(&snippet, &payload) {
            Err(MapFailure::Evaluation(_)) => {}
            other => panic!("an evaluation failure must come out as the evaluation arm: {other:?}"),
        }
    }

    #[test]
    fn a_config_without_a_transform_is_rejected() {
        assert!(matches!(
            crate::accept_transform(&empty_config()),
            Err(crate::map_config::MapConfigError::MissingTransform)
        ));
    }

    #[test]
    fn the_cell_name_lane_stands_on_route_and_map_without_a_new_element() {
        let split = route_actor(
            &[ProductValue::string("type")],
            &[
                ("label", ProductValue::string("label")),
                ("usage", ProductValue::string("usage")),
            ],
        );
        let pick = transform("event.label");

        let label_event = object_payload(&[
            ("type", ProductValue::string("label"), BaseShape::String),
            ("label", ProductValue::string("circular"), BaseShape::String),
        ]);
        let usage_event = object_payload(&[
            ("type", ProductValue::string("usage"), BaseShape::String),
            ("label", ProductValue::string("circular"), BaseShape::String),
        ]);

        assert_eq!(split.decide(label_event.value()).as_str(), "route_label");
        assert_eq!(split.decide(usage_event.value()).as_str(), "route_usage");

        let picked =
            map_event(&pick, &label_event).expect("pull label out of the named branch's payload");
        assert_eq!(picked.value().as_str(), Some("circular"));

        let again = map_event(&pick, &label_event)
            .expect("a stateless transform yields the same output for the same input");
        assert_eq!(again.value(), picked.value());
    }
}
