
use std::error::Error;
use std::fmt;

use circular_core::{BudgetExhausted, EvaluationBudget, Value};
use circular_expr::{ConfigPath, Segment};

use crate::capabilities::{Capability, Condition};
use crate::ports::{Arity, InletSpec, OutletSpec, PortId, Side};
use crate::spec::ActorSpec;
use crate::types::Flow;

pub const ERROR_PORT_NAME: &str = "_error";

pub const TIMER_PORT_NAME: &str = "_timer";

pub const LIFECYCLE_PORT_NAME: &str = "_lifecycle";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StaticPortRequest<'a> {
    Named { side: Side, id: &'a PortId },
    Default { side: Side },
}

#[derive(Clone, Copy, Debug)]
pub enum StaticPort<'a> {
    Inlet(&'a InletSpec),
    Outlet(&'a OutletSpec),
}

impl<'a> StaticPort<'a> {
    #[must_use]
    pub const fn side(self) -> Side {
        match self {
            Self::Inlet(_) => Side::Inlet,
            Self::Outlet(_) => Side::Outlet,
        }
    }

    #[must_use]
    pub const fn id(self) -> &'a PortId {
        match self {
            Self::Inlet(port) => port.id(),
            Self::Outlet(port) => port.id(),
        }
    }

    #[must_use]
    pub const fn ty(self) -> &'a Flow {
        match self {
            Self::Inlet(port) => port.ty(),
            Self::Outlet(port) => port.ty(),
        }
    }

    #[must_use]
    pub const fn arity(self) -> Arity {
        match self {
            Self::Inlet(port) => port.arity(),
            Self::Outlet(port) => port.arity(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StaticPortQueryError {
    DynamicRulesRequireConfigFold,
    MultiplePrimaryPorts(Side),
}

impl fmt::Display for StaticPortQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DynamicRulesRequireConfigFold => {
                formatter.write_str("dynamic port rules require the unresolved plan-config fold")
            }
            Self::MultiplePrimaryPorts(side) => {
                write!(formatter, "more than one primary {side} port")
            }
        }
    }
}

impl Error for StaticPortQueryError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EndpointAnswer {
    Exists {
        ty: Flow,
        arity: Arity,
    },
    Absent { available: Box<[PortId]> },
}

impl EndpointAnswer {
    #[must_use]
    pub const fn exists(&self) -> bool {
        matches!(self, Self::Exists { .. })
    }
}

pub fn answer_endpoint(
    spec: &ActorSpec,
    config: &circular_runtime::FoldedConfig,
    side: Side,
    port: &PortId,
) -> Result<EndpointAnswer, crate::ports::PortExpansionError> {
    let ports = spec.expand_ports(config)?;
    let found = match side {
        Side::Inlet => ports
            .inlets()
            .iter()
            .find(|spec| spec.id() == port)
            .map(|spec| (spec.ty().clone(), spec.arity())),
        Side::Outlet => ports
            .outlets()
            .iter()
            .find(|spec| spec.id() == port)
            .map(|spec| (spec.ty().clone(), spec.arity())),
    };
    Ok(match found {
        Some((ty, arity)) => EndpointAnswer::Exists { ty, arity },
        None => EndpointAnswer::Absent {
            available: available_ports(&ports, side),
        },
    })
}

#[must_use]
pub fn available_ports(ports: &crate::ports::PortSet, side: Side) -> Box<[PortId]> {
    match side {
        Side::Inlet => ports
            .inlets()
            .iter()
            .map(|spec| spec.id().clone())
            .collect(),
        Side::Outlet => ports
            .outlets()
            .iter()
            .map(|spec| spec.id().clone())
            .collect(),
    }
}

pub fn resolve_static_port<'a>(
    spec: &'a ActorSpec,
    request: StaticPortRequest<'_>,
) -> Result<Option<StaticPort<'a>>, StaticPortQueryError> {
    if !spec.ports().dynamic().is_empty() {
        return Err(StaticPortQueryError::DynamicRulesRequireConfigFold);
    }

    let fixed = spec.ports().fixed();
    match request {
        StaticPortRequest::Named {
            side: Side::Inlet,
            id,
        } => Ok(fixed
            .inlets()
            .iter()
            .find(|port| port.id() == id)
            .map(StaticPort::Inlet)),
        StaticPortRequest::Named {
            side: Side::Outlet,
            id,
        } => Ok(fixed
            .outlets()
            .iter()
            .find(|port| port.id() == id)
            .map(StaticPort::Outlet)),
        StaticPortRequest::Default { side: Side::Inlet } => default_inlet(fixed.inlets()),
        StaticPortRequest::Default { side: Side::Outlet } => default_outlet(fixed.outlets()),
    }
}

fn default_inlet(ports: &[InletSpec]) -> Result<Option<StaticPort<'_>>, StaticPortQueryError> {
    let mut primary = ports.iter().filter(|port| port.primary());
    match (primary.next(), primary.next()) {
        (Some(port), None) => Ok(Some(StaticPort::Inlet(port))),
        (Some(_), Some(_)) => Err(StaticPortQueryError::MultiplePrimaryPorts(Side::Inlet)),
        (None, _) if ports.len() == 1 => Ok(Some(StaticPort::Inlet(&ports[0]))),
        (None, _) => Ok(None),
    }
}

fn default_outlet(ports: &[OutletSpec]) -> Result<Option<StaticPort<'_>>, StaticPortQueryError> {
    let mut primary = ports.iter().filter(|port| port.primary());
    match (primary.next(), primary.next()) {
        (Some(port), None) => Ok(Some(StaticPort::Outlet(port))),
        (Some(_), Some(_)) => Err(StaticPortQueryError::MultiplePrimaryPorts(Side::Outlet)),
        (None, _) if ports.len() == 1 => Ok(Some(StaticPort::Outlet(&ports[0]))),
        (None, _) => Ok(None),
    }
}

pub fn requires(
    spec: &ActorSpec,
    config: &Value,
    budget: &mut EvaluationBudget,
) -> Result<Vec<Capability>, BudgetExhausted> {
    let mut required = Vec::new();
    for rule in spec.requires() {
        if condition_holds(rule.condition(), config, budget)? {
            required.push(rule.capability());
        }
    }
    Ok(required)
}

pub fn derived_error_port(
    spec: &ActorSpec,
    config: &Value,
    budget: &mut EvaluationBudget,
) -> Result<Option<PortId>, BudgetExhausted> {
    Ok((!requires(spec, config, budget)?.is_empty()).then(|| {
        PortId::try_derived(ERROR_PORT_NAME.to_owned())
            .expect("the canonical derived error port name is valid")
    }))
}

pub fn derived_error_port_for_empty_config(
    spec: &ActorSpec,
    budget: &mut EvaluationBudget,
) -> Result<Option<PortId>, BudgetExhausted> {
    let config = Value::object(std::iter::empty::<(&str, Value)>())
        .expect("empty config has no duplicate keys");
    derived_error_port(spec, &config, budget)
}

fn condition_holds(
    condition: &Condition,
    config: &Value,
    budget: &mut EvaluationBudget,
) -> Result<bool, BudgetExhausted> {
    match condition {
        Condition::Always => Ok(true),
        Condition::ConfigPresent(path) => Ok(value_at(config, path).is_some()),
        Condition::ConfigEquals { at, value } => value_at(config, at).map_or(Ok(false), |actual| {
            actual.strict_eq_with_budget(value, budget)
        }),
    }
}

fn value_at<'a>(root: &'a Value, path: &ConfigPath) -> Option<&'a Value> {
    let mut current = root;
    for segment in path.segments() {
        current = match segment {
            Segment::Key(key) => current.as_object()?.get(key)?,
            Segment::Index(index) => {
                let index = usize::try_from(*index).ok()?;
                current.as_array()?.get(index)?
            }
        };
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActorType, get};

    fn empty_config() -> Value {
        Value::object(std::iter::empty::<(&str, Value)>()).expect("empty object is unique")
    }

    #[test]
    fn fixture_defaults_come_from_registered_fixed_ports() {
        let map = get(ActorType::FixtureMap);
        let missing = PortId::try_new("missing").expect("test port reference is canonical");
        let inlet = resolve_static_port(map, StaticPortRequest::Default { side: Side::Inlet })
            .unwrap()
            .unwrap();
        let outlet = resolve_static_port(map, StaticPortRequest::Default { side: Side::Outlet })
            .unwrap()
            .unwrap();

        assert_eq!(inlet.id().as_str(), "in");
        assert_eq!(outlet.id().as_str(), "out");
        assert_eq!(inlet.side(), Side::Inlet);
        assert_eq!(outlet.side(), Side::Outlet);
        assert!(
            resolve_static_port(
                map,
                StaticPortRequest::Named {
                    side: Side::Outlet,
                    id: &missing,
                },
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn dynamic_rules_are_rejected_instead_of_returning_a_fixed_partial_answer() {
        let event = PortId::try_new("event").expect("test port reference is canonical");
        assert!(matches!(
            resolve_static_port(
                get(ActorType::Route),
                StaticPortRequest::Named {
                    side: Side::Inlet,
                    id: &event,
                },
            ),
            Err(StaticPortQueryError::DynamicRulesRequireConfigFold)
        ));
    }

    #[test]
    fn error_port_is_derived_from_active_requirements_only() {
        let config = empty_config();
        let mut budget = EvaluationBudget::new(std::num::NonZeroUsize::new(1).unwrap());
        assert_eq!(
            derived_error_port(get(ActorType::FixtureTap), &config, &mut budget)
                .expect("the fixture profile does not evaluate a composite equality")
                .expect("fixture tap has active external requirements")
                .as_str(),
            "_error"
        );
        assert!(
            derived_error_port(get(ActorType::FixtureMap), &config, &mut budget)
                .expect("the fixture profile does not evaluate a composite equality")
                .is_none()
        );
    }

    #[test]
    fn config_equality_cannot_bypass_the_shared_budget() {
        let config = Value::Array(vec![Value::Null, Value::Bool(true)]);
        let condition = Condition::ConfigEquals {
            at: ConfigPath::root(),
            value: config.clone(),
        };
        let mut short = EvaluationBudget::new(std::num::NonZeroUsize::new(1).unwrap());
        assert!(condition_holds(&condition, &config, &mut short).is_err());

        let mut exact = EvaluationBudget::new(std::num::NonZeroUsize::new(2).unwrap());
        assert_eq!(condition_holds(&condition, &config, &mut exact), Ok(true));
        assert_eq!(exact.remaining(), 0);
    }

    #[test]
    fn config_shaped_ports_are_answerable_where_the_static_query_refuses() {
        let spec = crate::get(crate::ActorType::Route);
        let config = circular_runtime::FoldedConfig::minted(
            crate::ActorType::Route,
            circular_core::Value::object([
                (
                    "at".to_owned(),
                    circular_core::Value::Array(vec![circular_core::Value::string("kind")]),
                ),
                (
                    "cases".to_owned(),
                    circular_core::Value::object([(
                        "even".to_owned(),
                        circular_core::Value::string("even"),
                    )])
                    .unwrap(),
                ),
            ])
            .unwrap(),
        );

        assert!(matches!(
            resolve_static_port(
                spec,
                StaticPortRequest::Named {
                    side: Side::Outlet,
                    id: &port("route_even"),
                },
            ),
            Err(StaticPortQueryError::DynamicRulesRequireConfigFold)
        ));

        assert!(
            answer_endpoint(spec, &config, Side::Outlet, &port("route_even"))
                .unwrap()
                .exists()
        );
        let EndpointAnswer::Absent { available } =
            answer_endpoint(spec, &config, Side::Outlet, &port("route_odd")).unwrap()
        else {
            panic!("an undeclared case is not an outlet");
        };
        assert!(available.iter().any(|id| id.as_str() == "route_even"));
        assert!(available.iter().any(|id| id.as_str() == "unmatched"));
    }

    fn port(name: &str) -> PortId {
        PortId::try_new(name).expect("the test port name is canonical")
    }
}
