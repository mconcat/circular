
use crate::GroundShape;
use circular_core::PortId;
use circular_expr::shapes::ShapeEnv;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResolvedInletShapes(BTreeMap<PortId, GroundShape>);

impl ResolvedInletShapes {
    #[must_use]
    pub fn new(shapes: impl IntoIterator<Item = (PortId, GroundShape)>) -> Self {
        Self(shapes.into_iter().collect())
    }

    #[must_use]
    pub fn get(&self, inlet: &PortId) -> Option<&GroundShape> {
        self.0.get(inlet)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn shape_env<'a>(&self, inlets: impl IntoIterator<Item = &'a PortId>) -> ShapeEnv {
        inlets
            .into_iter()
            .filter_map(|inlet| {
                self.get(inlet).map(|shape| {
                    (
                        inlet.as_str().to_owned(),
                        crate::types::to_unnamed_shape(shape.as_shape()),
                    )
                })
            })
            .collect()
    }
}
