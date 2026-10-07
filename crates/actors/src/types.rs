
use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fmt;

use circular_core::Value;
pub use circular_core::{BaseShape, NonZeroTicks};
use circular_protocol::port_type::{PortFlow, PortRate, PortShape, PortShapeField};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Name(Cow<'static, str>);

impl Name {
    #[must_use]
    pub const fn from_static(value: &'static str) -> Self {
        Self(Cow::Borrowed(value))
    }

    #[must_use]
    pub fn from_normalized(value: impl Into<Cow<'static, str>>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Name {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub type Shape = circular_core::Shape<Name>;

pub type FieldMap = circular_core::FieldMap<Name>;

pub type GroundShape = circular_core::GroundShape<Name>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RateExpr {
    Period(NonZeroTicks),
    RateVar(Name),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Flow {
    Stream(Shape),
    Signal { item: Shape, rate: RateExpr },
}

/// Decode the one protocol-owned physical Flow carrier into actor semantics.
///
/// The protocol decoder has already enforced its closed arms, canonical object
/// fields, depth/width ceilings, and nonzero signal periods.  This conversion
/// changes only the owned name wrapper.
#[must_use]
pub fn flow_from_port_type(flow: PortFlow) -> Flow {
    let shape = |shape| shape_from_port_type(shape);
    match flow {
        PortFlow::Stream(item) => Flow::Stream(shape(item)),
        PortFlow::Signal { item, rate } => Flow::Signal {
            item: shape(item),
            rate: match rate {
                PortRate::Period(period) => RateExpr::Period(period),
                PortRate::Variable(name) => RateExpr::RateVar(Name::from_normalized(name)),
            },
        },
    }
}

pub(crate) fn shape_from_port_type(shape: PortShape) -> Shape {
    match shape {
        PortShape::Any => Shape::Any,
        PortShape::Base(base) => Shape::Base(base),
        PortShape::Array(item) => Shape::Array(Box::new(shape_from_port_type(*item))),
        PortShape::Object { fields, open } => Shape::Object {
            fields: FieldMap::try_new(
                fields
                    .into_iter()
                    .map(|PortShapeField { name, shape }| {
                        (Name::from_normalized(name), shape_from_port_type(shape))
                    })
                    .collect::<Vec<_>>(),
            )
            .expect("the protocol codec rejects duplicate object fields"),
            open,
        },
        PortShape::Variable(name) => Shape::Var(Name::from_normalized(name)),
    }
}

/// Project actor semantics through the same protocol-owned closed carrier.
#[must_use]
pub fn port_type_from_flow(flow: &Flow) -> PortFlow {
    let shape = |shape: &Shape| PortShape::from_core(&to_unnamed_shape(shape));
    match flow {
        Flow::Stream(item) => PortFlow::Stream(shape(item)),
        Flow::Signal { item, rate } => PortFlow::Signal {
            item: shape(item),
            rate: match rate {
                RateExpr::Period(period) => PortRate::Period(*period),
                RateExpr::RateVar(name) => PortRate::Variable(name.as_str().to_owned()),
            },
        },
    }
}

impl Flow {
    #[must_use]
    pub const fn item(&self) -> &Shape {
        match self {
            Self::Stream(item) | Self::Signal { item, .. } => item,
        }
    }

    pub fn variables(&self) -> impl Iterator<Item = &Name> {
        let mut found: Vec<&Name> = Vec::new();
        let mut pending = vec![self.item()];
        while let Some(current) = pending.pop() {
            match current {
                Shape::Var(name) => {
                    if !found.contains(&name) {
                        found.push(name);
                    }
                }
                Shape::Array(item) => pending.push(item),
                Shape::Object { fields, .. } => {
                    pending.extend(fields.as_slice().iter().rev().map(|(_, shape)| shape));
                }
                Shape::Any | Shape::Base(_) => {}
            }
        }
        found.into_iter()
    }

    #[must_use]
    pub fn substitute(&self, environment: &Substitution) -> Self {
        match self {
            Self::Stream(item) => Self::Stream(substitute_shape(item, environment)),
            Self::Signal { item, rate } => Self::Signal {
                item: substitute_shape(item, environment),
                rate: rate.clone(),
            },
        }
    }
}

fn substitute_shape(shape: &Shape, environment: &Substitution) -> Shape {
    match shape {
        Shape::Var(name) => environment
            .get(name)
            .map_or_else(|| shape.clone(), |ground| ground.as_shape().clone()),
        Shape::Array(item) => Shape::Array(Box::new(substitute_shape(item, environment))),
        Shape::Object { fields, open } => Shape::Object {
            fields: FieldMap::try_new(
                fields
                    .as_slice()
                    .iter()
                    .map(|(name, shape)| (name.clone(), substitute_shape(shape, environment)))
                    .collect::<Vec<_>>(),
            )
            .expect("substitution does not rename fields, so uniqueness is preserved"),
            open: *open,
        },
        Shape::Any | Shape::Base(_) => shape.clone(),
    }
}

#[must_use]
pub fn to_unnamed_shape(shape: &Shape) -> circular_core::Shape<String> {
    use circular_core::Shape as S;
    match shape {
        S::Any => S::Any,
        S::Base(base) => S::Base(*base),
        S::Var(name) => S::Var(name.as_str().to_owned()),
        S::Array(item) => S::Array(Box::new(to_unnamed_shape(item))),
        S::Object { fields, open } => S::Object {
            fields: circular_core::FieldMap::try_new(
                fields
                    .as_slice()
                    .iter()
                    .map(|(name, shape)| (name.as_str().to_owned(), to_unnamed_shape(shape)))
                    .collect::<Vec<_>>(),
            )
            .expect("the name mapping is injective, so field uniqueness is preserved"),
            open: *open,
        },
    }
}

#[must_use]
pub fn from_unnamed_shape(shape: &circular_core::Shape<String>) -> Shape {
    use circular_core::Shape as S;
    match shape {
        S::Any => S::Any,
        S::Base(base) => S::Base(*base),
        S::Var(name) => S::Var(Name::from_normalized(name.clone())),
        S::Array(item) => S::Array(Box::new(from_unnamed_shape(item))),
        S::Object { fields, open } => S::Object {
            fields: FieldMap::try_new(
                fields
                    .as_slice()
                    .iter()
                    .map(|(name, shape)| {
                        (
                            Name::from_normalized(name.clone()),
                            from_unnamed_shape(shape),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
            .expect("the name mapping is injective, so field uniqueness is preserved"),
            open: *open,
        },
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Substitution(BTreeMap<Name, GroundShape>);

impl Substitution {
    #[must_use]
    pub fn new() -> Self {
        Self(BTreeMap::new())
    }

    #[must_use]
    pub fn get(&self, variable: &Name) -> Option<&GroundShape> {
        self.0.get(variable)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Name, &GroundShape)> {
        self.0.iter()
    }

    pub fn insert(
        &mut self,
        variable: Name,
        shape: GroundShape,
    ) -> Result<Assigned, VariableConflict> {
        match self.0.get(&variable) {
            Some(existing) if *existing == shape => Ok(Assigned::Confirmed),
            Some(existing) => Err(VariableConflict {
                variable,
                existing: existing.clone(),
                incoming: shape,
            }),
            None => {
                self.0.insert(variable, shape);
                Ok(Assigned::Fresh)
            }
        }
    }

    #[must_use]
    pub fn ground(&self, flow: &Flow) -> Option<GroundFlow> {
        GroundFlow::try_new(flow.substitute(self)).ok()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Assigned {
    Fresh,
    Confirmed,
}

impl Assigned {
    #[must_use]
    pub const fn is_fresh(self) -> bool {
        matches!(self, Self::Fresh)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VariableConflict {
    variable: Name,
    existing: GroundShape,
    incoming: GroundShape,
}

impl VariableConflict {
    #[must_use]
    pub const fn variable(&self) -> &Name {
        &self.variable
    }

    #[must_use]
    pub const fn existing(&self) -> &GroundShape {
        &self.existing
    }

    #[must_use]
    pub const fn incoming(&self) -> &GroundShape {
        &self.incoming
    }
}

impl fmt::Display for VariableConflict {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "shape variable {} received two different ground shapes",
            self.variable
        )
    }
}

impl std::error::Error for VariableConflict {}

#[must_use]
pub fn value_inhabits(value: &Value, shape: &GroundShape) -> bool {
    value_matches_shape(value, shape.as_shape())
}

#[must_use]
pub(crate) fn value_matches_shape(value: &Value, shape: &Shape) -> bool {
    match (value, shape) {
        (_, Shape::Any) => true,
        (_, Shape::Var(_)) => false,
        (value, Shape::Base(base)) => value.kind().base_shape() == Some(*base),
        (Value::Array(values), Shape::Array(item)) => {
            values.iter().all(|value| value_matches_shape(value, item))
        }
        (Value::Object(value), Shape::Object { fields, open }) => {
            if !open && value.len() != fields.as_slice().len() {
                return false;
            }
            fields.as_slice().iter().all(|(name, shape)| {
                value
                    .get(name.as_str())
                    .is_some_and(|value| value_matches_shape(value, shape))
            })
        }
        _ => false,
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroundFlow(Flow);

impl GroundFlow {
    pub fn try_new(flow: Flow) -> Result<Self, ContainsVariable> {
        let rate_has_variable = matches!(
            &flow,
            Flow::Signal {
                rate: RateExpr::RateVar(_),
                ..
            }
        );
        if rate_has_variable || shape_contains_variable(flow.item()) {
            Err(ContainsVariable)
        } else {
            Ok(Self(flow))
        }
    }

    #[must_use]
    pub const fn as_flow(&self) -> &Flow {
        &self.0
    }
}

impl TryFrom<Flow> for GroundFlow {
    type Error = ContainsVariable;

    fn try_from(flow: Flow) -> Result<Self, Self::Error> {
        Self::try_new(flow)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContainsVariable;

impl fmt::Display for ContainsVariable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .write_str("shape and rate variables must be substituted before checking connectivity")
    }
}

impl std::error::Error for ContainsVariable {}

#[must_use]
pub fn connectable(source: &GroundFlow, destination: &GroundFlow) -> bool {
    let (source_item, destination_item) = match (source.as_flow(), destination.as_flow()) {
        (Flow::Stream(source), Flow::Stream(destination))
        | (Flow::Signal { item: source, .. }, Flow::Stream(destination)) => (source, destination),
        (
            Flow::Signal {
                item: source,
                rate: source_rate,
            },
            Flow::Signal {
                item: destination,
                rate: destination_rate,
            },
        ) if source_rate == destination_rate => (source, destination),
        (Flow::Signal { .. }, Flow::Signal { .. }) | (Flow::Stream(_), Flow::Signal { .. }) => {
            return false;
        }
    };

    shapes_connectable(source_item, destination_item)
}

fn shape_contains_variable(shape: &Shape) -> bool {
    let mut pending = vec![shape];
    while let Some(current) = pending.pop() {
        match current {
            Shape::Var(_) => return true,
            Shape::Array(item) => pending.push(item),
            Shape::Object { fields, .. } => {
                pending.extend(fields.as_slice().iter().map(|(_, shape)| shape));
            }
            Shape::Any | Shape::Base(_) => {}
        }
    }
    false
}

fn shapes_connectable(source: &Shape, destination: &Shape) -> bool {
    let mut pending = vec![(source, destination)];

    while let Some((source, destination)) = pending.pop() {
        match (source, destination) {
            (_, Shape::Any) => {}
            (Shape::Any, _) => return false,
            (Shape::Base(source), Shape::Base(destination)) if source == destination => {}
            (Shape::Array(source), Shape::Array(destination)) => {
                pending.push((source, destination));
            }
            (
                Shape::Object {
                    fields: source_fields,
                    open: source_open,
                },
                Shape::Object {
                    fields: destination_fields,
                    open: destination_open,
                },
            ) => {
                if !destination_open
                    && (*source_open
                        || source_fields.as_slice().len() != destination_fields.as_slice().len())
                {
                    return false;
                }

                let source_by_name: HashMap<_, _> = source_fields
                    .as_slice()
                    .iter()
                    .map(|(name, shape)| (name, shape))
                    .collect();
                for (name, destination_shape) in destination_fields.as_slice() {
                    let Some(source_shape) = source_by_name.get(name) else {
                        return false;
                    };
                    pending.push((*source_shape, destination_shape));
                }
            }
            (Shape::Var(_), _) | (_, Shape::Var(_)) => {
                unreachable!("building a GroundFlow refuses every shape variable")
            }
            _ => return false,
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(value: &'static str) -> Name {
        Name::from_static(value)
    }

    fn object(fields: &[(&'static str, BaseShape)], open: bool) -> Shape {
        let fields = fields
            .iter()
            .map(|(field, shape)| (name(field), Shape::Base(*shape)))
            .collect();
        Shape::Object {
            fields: FieldMap::try_new(fields).expect("test field names are unique"),
            open,
        }
    }

    fn stream(shape: Shape) -> GroundFlow {
        GroundFlow::try_new(Flow::Stream(shape)).expect("the test flow is ground")
    }

    #[test]
    fn static_and_owned_names_are_the_same_key() {
        let static_name = Name::from_static("field");
        let owned_name = Name::from_normalized(String::from("field"));

        assert_eq!(static_name, owned_name);
    }

    #[test]
    fn inv_actor_80_closed_destination_requires_closed_source_and_same_field_set() {
        let same_fields_closed = stream(object(
            &[("right", BaseShape::Float), ("left", BaseShape::Bool)],
            false,
        ));
        let same_fields_different_order = stream(object(
            &[("left", BaseShape::Bool), ("right", BaseShape::Float)],
            false,
        ));
        let same_fields_open = stream(object(
            &[("left", BaseShape::Bool), ("right", BaseShape::Float)],
            true,
        ));
        let different_fields_closed = stream(object(
            &[("left", BaseShape::Bool), ("other", BaseShape::Float)],
            false,
        ));
        let extra_field_closed = stream(object(
            &[
                ("left", BaseShape::Bool),
                ("right", BaseShape::Float),
                ("extra", BaseShape::Null),
            ],
            false,
        ));

        assert!(connectable(
            &same_fields_closed,
            &same_fields_different_order
        ));
        assert!(!connectable(&same_fields_open, &same_fields_closed));
        assert!(!connectable(&different_fields_closed, &same_fields_closed));
        assert!(!connectable(&extra_field_closed, &same_fields_closed));

        let open_subset = stream(object(&[("left", BaseShape::Bool)], true));
        assert!(connectable(&same_fields_closed, &open_subset));
        assert!(connectable(&same_fields_open, &open_subset));
    }

    #[test]
    fn variables_cannot_enter_connectable_domain() {
        assert_eq!(
            GroundFlow::try_new(Flow::Stream(Shape::Var(name("item")))),
            Err(ContainsVariable)
        );
        assert_eq!(
            GroundFlow::try_new(Flow::Signal {
                item: Shape::Base(BaseShape::Bool),
                rate: RateExpr::RateVar(name("rate")),
            }),
            Err(ContainsVariable)
        );
    }

    #[test]
    fn flow_rules_do_not_add_coercion_or_resampling() {
        let period_one = NonZeroTicks::new(1).expect("positive period");
        let period_two = NonZeroTicks::new(2).expect("positive period");
        let signal = |shape, period| {
            GroundFlow::try_new(Flow::Signal {
                item: shape,
                rate: RateExpr::Period(period),
            })
            .expect("the periodic signal is ground")
        };

        let bool_stream = stream(Shape::Base(BaseShape::Bool));
        let number_stream = stream(Shape::Base(BaseShape::Float));
        let bool_signal_one = signal(Shape::Base(BaseShape::Bool), period_one);
        let bool_signal_two = signal(Shape::Base(BaseShape::Bool), period_two);

        assert!(connectable(&bool_signal_one, &bool_stream));
        assert!(!connectable(&bool_stream, &bool_signal_one));
        assert!(!connectable(&bool_signal_one, &bool_signal_two));
        assert!(!connectable(&bool_stream, &number_stream));
    }

    fn ground(shape: Shape) -> GroundShape {
        GroundShape::try_new(shape).expect("the test shape is ground")
    }

    fn var(value: &'static str) -> Shape {
        Shape::Var(name(value))
    }

    fn nested_variables() -> Flow {
        Flow::Stream(Shape::Object {
            fields: FieldMap::try_new(vec![
                (name("outer"), Shape::Array(Box::new(var("T")))),
                (
                    name("inner"),
                    Shape::Object {
                        fields: FieldMap::try_new(vec![
                            (name("leaf"), var("U")),
                            (name("again"), var("T")),
                        ])
                        .expect("nested field names are unique"),
                        open: false,
                    },
                ),
            ])
            .expect("outer field names are unique"),
            open: false,
        })
    }

    #[test]
    fn variables_come_out_in_source_order_without_repeats() {
        let found: Vec<_> = nested_variables().variables().cloned().collect();

        assert_eq!(found, vec![name("T"), name("U")]);
    }

    #[test]
    fn a_flow_without_variables_enumerates_nothing() {
        let ground_flow = Flow::Stream(object(&[("field", BaseShape::Int)], false));

        assert_eq!(ground_flow.variables().count(), 0);
    }

    #[test]
    fn substitution_leaves_the_variables_it_does_not_carry() {
        let mut environment = Substitution::new();
        environment
            .insert(name("T"), ground(Shape::Base(BaseShape::Int)))
            .expect("the first substitution does not conflict");

        let substituted = nested_variables().substitute(&environment);

        let remaining: Vec<_> = substituted.variables().cloned().collect();
        assert_eq!(remaining, vec![name("U")]);
        assert!(GroundFlow::try_new(substituted).is_err());
    }

    #[test]
    fn a_flow_becomes_ground_only_once_every_variable_is_carried() {
        let mut environment = Substitution::new();
        let flow = nested_variables();
        assert!(environment.ground(&flow).is_none());

        environment
            .insert(name("T"), ground(Shape::Base(BaseShape::Int)))
            .unwrap();
        assert!(environment.ground(&flow).is_none());

        environment
            .insert(name("U"), ground(Shape::Base(BaseShape::Bool)))
            .unwrap();
        assert!(environment.ground(&flow).is_some());
    }

    #[test]
    fn the_same_shape_arriving_again_confirms_without_growing_the_environment() {
        let mut environment = Substitution::new();
        let int = || ground(Shape::Base(BaseShape::Int));

        assert_eq!(environment.insert(name("T"), int()), Ok(Assigned::Fresh));
        assert_eq!(
            environment.insert(name("T"), int()),
            Ok(Assigned::Confirmed)
        );
        assert!(!Assigned::Confirmed.is_fresh());
        assert_eq!(environment.len(), 1);
    }

    #[test]
    fn a_second_ground_shape_is_refused_instead_of_merged() {
        let mut environment = Substitution::new();
        environment
            .insert(name("T"), ground(Shape::Base(BaseShape::Int)))
            .unwrap();

        let conflict = environment
            .insert(name("T"), ground(Shape::Base(BaseShape::Bool)))
            .expect_err("two syntactically different ground shapes conflict");

        assert_eq!(conflict.variable(), &name("T"));
        assert_eq!(conflict.existing().as_shape(), &Shape::Base(BaseShape::Int));
        assert_eq!(
            conflict.incoming().as_shape(),
            &Shape::Base(BaseShape::Bool)
        );
        assert_eq!(
            environment.get(&name("T")).map(GroundShape::as_shape),
            Some(&Shape::Base(BaseShape::Int))
        );
        assert_eq!(environment.len(), 1);
    }

    #[test]
    fn the_environment_does_not_remember_the_order_it_was_built_in() {
        let entries = [
            (name("T"), ground(Shape::Base(BaseShape::Int))),
            (name("U"), ground(Shape::Base(BaseShape::Bool))),
            (name("S"), ground(Shape::Array(Box::new(Shape::Any)))),
        ];
        let build = |order: [usize; 3]| {
            let mut environment = Substitution::new();
            for index in order {
                let (variable, shape) = entries[index].clone();
                environment.insert(variable, shape).unwrap();
            }
            environment
        };

        assert_eq!(build([0, 1, 2]), build([2, 0, 1]));
        let forward: Vec<_> = build([0, 1, 2]).iter().map(|(n, _)| n.clone()).collect();
        let shuffled: Vec<_> = build([2, 1, 0]).iter().map(|(n, _)| n.clone()).collect();
        assert_eq!(forward, shuffled);
        assert_eq!(forward, vec![name("S"), name("T"), name("U")]);
    }

    #[test]
    fn a_rate_variable_stays_outside_the_substitution_domain() {
        let flow = Flow::Signal {
            item: var("T"),
            rate: RateExpr::RateVar(name("R")),
        };
        let mut environment = Substitution::new();
        environment
            .insert(name("T"), ground(Shape::Base(BaseShape::Int)))
            .unwrap();
        environment
            .insert(name("R"), ground(Shape::Base(BaseShape::Int)))
            .unwrap();

        let substituted = flow.substitute(&environment);

        assert_eq!(substituted.item(), &Shape::Base(BaseShape::Int));
        assert!(matches!(
            substituted,
            Flow::Signal {
                rate: RateExpr::RateVar(_),
                ..
            }
        ));
        assert!(environment.ground(&flow).is_none());
    }
}
