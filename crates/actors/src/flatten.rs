use crate::config::{PayloadPath, payload_path_from_value};
use crate::{GroundShape, ProductPayload, ProductValue, Shape};
use circular_expr::Segment;

pub const AT: &str = "at";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FlattenFailure {
    ContextNotObject,
    PathNotArray,
    ElementNotObject,
    PathUnreplaceable,
}

#[derive(Clone, Debug)]
pub struct FlattenConfig {
    at: PayloadPath,
}
impl FlattenConfig {
    pub fn from_value(value: &ProductValue) -> Result<Self, String> {
        let object = value
            .as_object()
            .ok_or("flatten config must be an object")?;
        if object.len() != 1 {
            return Err(format!("flatten requires only {AT}"));
        }
        let at = payload_path_from_value(
            object
                .get(AT)
                .ok_or_else(|| format!("flatten requires {AT}"))?,
        )
        .map_err(|e| format!("flatten {AT}: {e}"))?;
        Ok(Self { at })
    }
    pub fn expand(&self, input: &ProductPayload) -> Result<Vec<ProductPayload>, FlattenFailure> {
        if !self.at.segments().is_empty() && input.value().as_object().is_none() {
            return Err(FlattenFailure::ContextNotObject);
        }
        let values = crate::route_config::select(input.value(), &self.at)
            .and_then(ProductValue::as_array)
            .ok_or(FlattenFailure::PathNotArray)?;
        if values.iter().any(|v| v.as_object().is_none()) {
            return Err(FlattenFailure::ElementNotObject);
        }
        values
            .iter()
            .map(|value| {
                let output = replace(input.value(), self.at.segments(), value)?;
                Ok(ProductPayload::new(
                    GroundShape::try_new(Shape::Object {
                        fields: crate::FieldMap::try_new(Vec::new()).unwrap(),
                        open: true,
                    })
                    .unwrap(),
                    output,
                ))
            })
            .collect()
    }
}
fn replace(
    value: &ProductValue,
    path: &[Segment],
    item: &ProductValue,
) -> Result<ProductValue, FlattenFailure> {
    let Some((head, tail)) = path.split_first() else {
        return Ok(item.clone());
    };
    match (head, value) {
        (Segment::Key(key), ProductValue::Object(fields)) => fields
            .iter()
            .map(|(name, value)| {
                let value = if name == key {
                    replace(value, tail, item)?
                } else {
                    value.clone()
                };
                Ok((name, value))
            })
            .collect::<Result<Vec<_>, FlattenFailure>>()
            .map(|v| ProductValue::object(v).expect("an existing object's fields are distinct")),
        (Segment::Index(index), ProductValue::Array(values)) => values
            .iter()
            .enumerate()
            .map(|(i, value)| {
                if i as u64 == *index {
                    replace(value, tail, item)
                } else {
                    Ok(value.clone())
                }
            })
            .collect::<Result<Vec<_>, _>>()
            .map(ProductValue::Array),
        _ => Err(FlattenFailure::PathUnreplaceable),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FieldMap;
    fn object(fields: impl IntoIterator<Item = (&'static str, ProductValue)>) -> ProductValue {
        ProductValue::object(fields).unwrap()
    }
    fn config(at: &[ProductValue]) -> FlattenConfig {
        FlattenConfig::from_value(&object([("at", ProductValue::array(at.to_vec()))])).unwrap()
    }
    fn payload(value: ProductValue) -> ProductPayload {
        ProductPayload::new(GroundShape::try_new(Shape::Any).unwrap(), value)
    }
    #[test]
    fn spans_preserve_resource_and_scope_context_in_order() {
        let first = object([("id", ProductValue::string("first"))]);
        let second = object([("id", ProductValue::string("second"))]);
        let batch = payload(object([
            (
                "resource",
                object([("service", ProductValue::string("checkout"))]),
            ),
            ("scope", ProductValue::string("sdk")),
            (
                "spans",
                ProductValue::array([first.clone(), second.clone()]),
            ),
        ]));
        let outputs = config(&[ProductValue::string("spans")])
            .expand(&batch)
            .unwrap();
        assert_eq!(
            outputs
                .iter()
                .map(|v| v.value().clone())
                .collect::<Vec<_>>(),
            [first, second].map(|span| object([
                (
                    "resource",
                    object([("service", ProductValue::string("checkout"))])
                ),
                ("scope", ProductValue::string("sdk")),
                ("spans", span)
            ]))
        );
        assert_eq!(
            batch
                .value()
                .as_object()
                .unwrap()
                .get("spans")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }
    #[test]
    fn datapoints_and_index_paths_preserve_metric_context() {
        let point = object([("count", ProductValue::int(9))]);
        let input = payload(object([(
            "metrics",
            ProductValue::array([object([
                ("name", ProductValue::string("latency")),
                (
                    "histogram",
                    object([("dataPoints", ProductValue::array([point.clone()]))]),
                ),
            ])]),
        )]));
        let out = config(&[
            ProductValue::string("metrics"),
            ProductValue::int(0),
            ProductValue::string("histogram"),
            ProductValue::string("dataPoints"),
        ])
        .expand(&input)
        .unwrap();
        assert_eq!(
            out[0].value(),
            &object([(
                "metrics",
                ProductValue::array([object([
                    ("name", ProductValue::string("latency")),
                    ("histogram", object([("dataPoints", point)]))
                ])])
            )])
        );
    }
    #[test]
    fn a_nonempty_path_requires_the_approved_object_context() {
        let input = payload(ProductValue::array([object([(
            "spans",
            ProductValue::array([object([("id", ProductValue::string("one"))])]),
        )])]));
        let selected = config(&[ProductValue::int(0), ProductValue::string("spans")]);
        assert_eq!(
            selected.expand(&input).unwrap_err(),
            FlattenFailure::ContextNotObject
        );
    }

    #[test]
    fn config_is_exact_and_required() {
        for value in [
            object([]),
            object([("at", ProductValue::string("a.b"))]),
            object([
                ("at", ProductValue::array([])),
                ("extra", ProductValue::Null),
            ]),
        ] {
            assert!(FlattenConfig::from_value(&value).is_err());
        }
        assert_eq!(
            config(&[])
                .expand(&payload(ProductValue::array([object([])])))
                .unwrap()[0]
                .shape(),
            &GroundShape::try_new(Shape::Object {
                fields: FieldMap::try_new(Vec::new()).unwrap(),
                open: true
            })
            .unwrap()
        );
    }
}
