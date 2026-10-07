
use std::collections::{HashMap, hash_map::Entry};
use std::error::Error;
use std::fmt;

pub use circular_core::{AuthoredPortId, PortId, PortIdError};
use circular_core::{ObjectValue, Value, ValueKind};
use circular_expr::{ConfigPath, Segment};
use circular_protocol::boundary_port::BoundaryPortDirection;

use crate::config::{ConfigSchema, Required};
use crate::spec::Label;
use crate::types::{Flow, Name, RateExpr, value_matches_shape};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Arity {
    One,
    Many,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Presence {
    Required,
    Defaulted(Value),
    Optional,
}

mod sealed {
    pub trait Sealed {}
}

pub trait Direction: sealed::Sealed {
    const SIDE: Side;

    type Extra: Clone + PartialEq + fmt::Debug;
}

pub enum In {}

impl sealed::Sealed for In {}

impl Direction for In {
    const SIDE: Side = Side::Inlet;
    type Extra = Presence;
}

pub enum Out {}

impl sealed::Sealed for Out {}

impl Direction for Out {
    const SIDE: Side = Side::Outlet;
    type Extra = ();
}

pub struct PortSpec<D: Direction> {
    id: PortId,
    ty: Flow,
    arity: Arity,
    primary: bool,
    label: Label,
    extra: D::Extra,
}

impl<D: Direction> Clone for PortSpec<D> {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            ty: self.ty.clone(),
            arity: self.arity,
            primary: self.primary,
            label: self.label.clone(),
            extra: self.extra.clone(),
        }
    }
}

impl<D: Direction> fmt::Debug for PortSpec<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match D::SIDE {
            Side::Inlet => "InletSpec",
            Side::Outlet => "OutletSpec",
        };
        let mut debug = formatter.debug_struct(name);
        debug
            .field("id", &self.id)
            .field("ty", &self.ty)
            .field("arity", &self.arity);
        if matches!(D::SIDE, Side::Inlet) {
            debug.field("presence", &self.extra);
        }
        debug
            .field("primary", &self.primary)
            .field("label", &self.label)
            .finish()
    }
}

impl<D: Direction> PartialEq for PortSpec<D> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.ty == other.ty
            && self.arity == other.arity
            && self.primary == other.primary
            && self.label == other.label
            && self.extra == other.extra
    }
}

pub type InletSpec = PortSpec<In>;

pub type OutletSpec = PortSpec<Out>;

impl InletSpec {
    pub fn try_new(
        id: AuthoredPortId,
        ty: Flow,
        arity: Arity,
        presence: Presence,
        primary: bool,
        label: Label,
    ) -> Result<Self, DefaultValueShapeMismatch> {
        validate_default(&presence, &ty)?;
        Ok(Self {
            id: id.into_port_id(),
            ty,
            arity,
            primary,
            label,
            extra: presence,
        })
    }

    /// Build a producer-derived system inlet whose reserved physical identity
    /// has already been validated by the protocol-owned codec.
    pub(crate) fn try_new_derived(
        id: PortId,
        ty: Flow,
        arity: Arity,
        presence: Presence,
        primary: bool,
        label: Label,
    ) -> Result<Self, DefaultValueShapeMismatch> {
        validate_default(&presence, &ty)?;
        Ok(Self {
            id,
            ty,
            arity,
            primary,
            label,
            extra: presence,
        })
    }

    #[must_use]
    pub const fn presence(&self) -> &Presence {
        &self.extra
    }
}

impl<D: Direction> PortSpec<D> {
    #[must_use]
    pub const fn id(&self) -> &PortId {
        &self.id
    }

    #[must_use]
    pub const fn ty(&self) -> &Flow {
        &self.ty
    }

    #[must_use]
    pub const fn arity(&self) -> Arity {
        self.arity
    }

    #[must_use]
    pub const fn primary(&self) -> bool {
        self.primary
    }

    #[must_use]
    pub const fn label(&self) -> &Label {
        &self.label
    }
}

impl OutletSpec {
    #[must_use]
    pub fn new(id: AuthoredPortId, ty: Flow, arity: Arity, primary: bool, label: Label) -> Self {
        Self {
            id: id.into_port_id(),
            ty,
            arity,
            primary,
            label,
            extra: (),
        }
    }

    /// Build a producer-derived system outlet whose reserved physical identity
    /// has already been validated by the protocol-owned codec.
    pub(crate) const fn new_derived(
        id: PortId,
        ty: Flow,
        arity: Arity,
        primary: bool,
        label: Label,
    ) -> Self {
        Self {
            id,
            ty,
            arity,
            primary,
            label,
            extra: (),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PortSet {
    inlets: Box<[InletSpec]>,
    outlets: Box<[OutletSpec]>,
}

impl PortSet {
    pub fn try_new(
        inlets: impl Into<Box<[InletSpec]>>,
        outlets: impl Into<Box<[OutletSpec]>>,
    ) -> Result<Self, DuplicatePortId> {
        let inlets = inlets.into();
        let outlets = outlets.into();
        reject_duplicates(&inlets)?;
        reject_duplicates(&outlets)?;
        Ok(Self { inlets, outlets })
    }

    #[must_use]
    pub fn empty() -> Self {
        Self {
            inlets: Box::new([]),
            outlets: Box::new([]),
        }
    }

    #[must_use]
    pub const fn inlets(&self) -> &[InletSpec] {
        &self.inlets
    }

    #[must_use]
    pub const fn outlets(&self) -> &[OutletSpec] {
        &self.outlets
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DuplicatePortId {
    side: Side,
    id: PortId,
}

impl DuplicatePortId {
    #[must_use]
    pub const fn side(&self) -> Side {
        self.side
    }

    #[must_use]
    pub const fn id(&self) -> &PortId {
        &self.id
    }
}

impl fmt::Display for DuplicatePortId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "duplicate {} port id {}", self.side, self.id)
    }
}

impl Error for DuplicatePortId {}

fn reject_duplicates<D: Direction>(ports: &[PortSpec<D>]) -> Result<(), DuplicatePortId> {
    for (index, port) in ports.iter().enumerate() {
        if ports[..index].iter().any(|previous| previous.id == port.id) {
            return Err(DuplicatePortId {
                side: D::SIDE,
                id: port.id.clone(),
            });
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub struct FlowChoice {
    side: Side,
    port: PortId,
    source: ConfigPath,
    arms: Box<[(Box<str>, Flow)]>,
}

impl FlowChoice {
    #[must_use]
    pub fn new(
        side: Side,
        port: AuthoredPortId,
        source: ConfigPath,
        arms: impl IntoIterator<Item = (&'static str, Flow)>,
    ) -> Self {
        let arms: Box<[(Box<str>, Flow)]> = arms
            .into_iter()
            .map(|(tag, flow)| (Box::from(tag), flow))
            .collect();
        assert!(!arms.is_empty(), "flow choice declares at least one arm");
        for (index, (tag, _)) in arms.iter().enumerate() {
            assert!(
                !arms[..index].iter().any(|(earlier, _)| earlier == tag),
                "flow choice tags are distinct"
            );
        }
        Self {
            side,
            port: port.into_port_id(),
            source,
            arms,
        }
    }

    #[must_use]
    pub const fn side(&self) -> Side {
        self.side
    }

    #[must_use]
    pub const fn port(&self) -> &PortId {
        &self.port
    }

    #[must_use]
    pub const fn source(&self) -> &ConfigPath {
        &self.source
    }

    pub fn arms(&self) -> impl ExactSizeIterator<Item = (&str, &Flow)> {
        self.arms.iter().map(|(tag, flow)| (&**tag, flow))
    }

    fn resolve(&self, config: &Value) -> Result<Flow, PortExpansionError> {
        let missing = || PortExpansionError::MissingFlowChoice {
            side: self.side,
            port: self.port.clone(),
            source: self.source.clone(),
        };
        let value = value_at_exact(config, &self.source)
            .map_err(|_| missing())?
            .ok_or_else(missing)?;
        let Some(tag) = value.as_str() else {
            return Err(PortExpansionError::UnknownFlowChoice {
                side: self.side,
                port: self.port.clone(),
                source: self.source.clone(),
                value: value.clone(),
            });
        };
        self.arms
            .iter()
            .find(|(candidate, _)| &**candidate == tag)
            .map(|(_, flow)| flow.clone())
            .ok_or_else(|| PortExpansionError::UnknownFlowChoice {
                side: self.side,
                port: self.port.clone(),
                source: self.source.clone(),
                value: value.clone(),
            })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PortRule {
    fixed: PortSet,
    dynamic: Box<[DynamicRule]>,
    choices: Box<[FlowChoice]>,
}

impl PortRule {
    #[must_use]
    pub fn new(fixed: PortSet, dynamic: Box<[DynamicRule]>) -> Self {
        Self {
            fixed,
            dynamic,
            choices: Box::new([]),
        }
    }

    #[must_use]
    pub fn with_flow_choices(mut self, choices: Box<[FlowChoice]>) -> Self {
        for choice in &choices {
            let declared = match choice.side {
                Side::Inlet => self
                    .fixed
                    .inlets()
                    .iter()
                    .any(|port| port.id == choice.port),
                Side::Outlet => self
                    .fixed
                    .outlets()
                    .iter()
                    .any(|port| port.id == choice.port),
            };
            assert!(
                declared,
                "flow choice names a fixed {:?} port: {:?}",
                choice.side, choice.port
            );
        }
        self.choices = choices;
        self
    }

    #[must_use]
    pub const fn fixed(&self) -> &PortSet {
        &self.fixed
    }

    #[must_use]
    pub const fn dynamic(&self) -> &[DynamicRule] {
        &self.dynamic
    }

    #[must_use]
    pub const fn choices(&self) -> &[FlowChoice] {
        &self.choices
    }

    pub fn expand(
        &self,
        config: &Value,
        schema: &crate::config::ConfigSchema,
    ) -> Result<PortSet, PortExpansionError> {
        let additions = preflight_expansion(self, config)?;
        let inlet_count =
            total_port_count(Side::Inlet, self.fixed.inlets().len(), additions.inlets)?;
        let outlet_count =
            total_port_count(Side::Outlet, self.fixed.outlets().len(), additions.outlets)?;

        let mut inlets = Vec::new();
        reserve_ports(&mut inlets, Side::Inlet, inlet_count)?;
        inlets.extend_from_slice(self.fixed.inlets());
        let mut outlets = Vec::new();
        reserve_ports(&mut outlets, Side::Outlet, outlet_count)?;
        outlets.extend_from_slice(self.fixed.outlets());
        let mut inlet_origins = fixed_origins(self.fixed.inlets(), inlet_count)?;
        let mut outlet_origins = fixed_origins(self.fixed.outlets(), outlet_count)?;

        for choice in &self.choices {
            let flow = choice.resolve(config)?;
            let port = match choice.side {
                Side::Inlet => inlets
                    .iter_mut()
                    .find(|port| port.id == choice.port)
                    .map(|port| &mut port.ty),
                Side::Outlet => outlets
                    .iter_mut()
                    .find(|port| port.id == choice.port)
                    .map(|port| &mut port.ty),
            };
            *port.expect("flow choice names a declared fixed port") = flow;
        }

        for (rule_index, rule) in self.dynamic.iter().enumerate() {
            let elements = elements(rule_index, rule, config)?;
            if elements.len() == 0 {
                continue;
            }

            let flow = resolve_template_type(rule_index, rule, config, schema, &self.fixed)?;
            match elements {
                ExpansionElements::Empty => {}
                ExpansionElements::Presence { id } => {
                    push_expanded_port(
                        rule_index,
                        rule,
                        &flow,
                        id.into_port_id(),
                        &mut inlets,
                        &mut outlets,
                        &mut inlet_origins,
                        &mut outlet_origins,
                    )?;
                }
                ExpansionElements::Count { prefix, count } => {
                    for index in 0..count {
                        let id = generated_id(
                            rule_index,
                            rule.source(),
                            ExpansionElement::Index(index),
                            indexed_port_id(&prefix, index),
                        )?;
                        push_expanded_port(
                            rule_index,
                            rule,
                            &flow,
                            id,
                            &mut inlets,
                            &mut outlets,
                            &mut inlet_origins,
                            &mut outlet_origins,
                        )?;
                    }
                }
                ExpansionElements::Keys { prefix, object } => {
                    for key in object.keys() {
                        let id = generated_id(
                            rule_index,
                            rule.source(),
                            ExpansionElement::Key(key.to_owned()),
                            from_key_port_id(&prefix, key),
                        )?;
                        push_expanded_port(
                            rule_index,
                            rule,
                            &flow,
                            id,
                            &mut inlets,
                            &mut outlets,
                            &mut inlet_origins,
                            &mut outlet_origins,
                        )?;
                    }
                }
            }
        }

        Ok(PortSet {
            inlets: inlets.into_boxed_slice(),
            outlets: outlets.into_boxed_slice(),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DynamicRule {
    source: ConfigPath,
    rule: ExpansionRule,
    template: PortTemplate,
}

impl DynamicRule {
    #[must_use]
    pub const fn new(source: ConfigPath, rule: ExpansionRule, template: PortTemplate) -> Self {
        Self {
            source,
            rule,
            template,
        }
    }

    #[must_use]
    pub const fn source(&self) -> &ConfigPath {
        &self.source
    }

    #[must_use]
    pub const fn expansion(&self) -> Expansion {
        self.rule.kind()
    }

    #[must_use]
    pub const fn rule(&self) -> &ExpansionRule {
        &self.rule
    }

    #[must_use]
    pub const fn template(&self) -> &PortTemplate {
        &self.template
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExpansionRule {
    Presence { id: AuthoredPortId },
    Count { prefix: AuthoredPortId },
    Keys { prefix: AuthoredPortId },
}

impl ExpansionRule {
    #[must_use]
    pub const fn kind(&self) -> Expansion {
        match self {
            Self::Presence { .. } => Expansion::Presence,
            Self::Count { .. } => Expansion::Count,
            Self::Keys { .. } => Expansion::Keys,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Expansion {
    Presence,
    Count,
    Keys,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PortTemplate {
    Inlet(InletTemplate),
    Outlet(OutletTemplate),
}

impl PortTemplate {
    #[must_use]
    pub const fn ty(&self) -> &TypeRule {
        match self {
            Self::Inlet(template) => template.ty(),
            Self::Outlet(template) => template.ty(),
        }
    }
}

pub struct PortTemplateOf<D: Direction> {
    ty: TypeRule,
    arity: Arity,
    label: LabelRule,
    extra: D::Extra,
}

impl<D: Direction> Clone for PortTemplateOf<D> {
    fn clone(&self) -> Self {
        Self {
            ty: self.ty.clone(),
            arity: self.arity,
            label: self.label.clone(),
            extra: self.extra.clone(),
        }
    }
}

impl<D: Direction> fmt::Debug for PortTemplateOf<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match D::SIDE {
            Side::Inlet => "InletTemplate",
            Side::Outlet => "OutletTemplate",
        };
        let mut debug = formatter.debug_struct(name);
        debug.field("ty", &self.ty).field("arity", &self.arity);
        if matches!(D::SIDE, Side::Inlet) {
            debug.field("presence", &self.extra);
        }
        debug.field("label", &self.label).finish()
    }
}

impl<D: Direction> PartialEq for PortTemplateOf<D> {
    fn eq(&self, other: &Self) -> bool {
        self.ty == other.ty
            && self.arity == other.arity
            && self.label == other.label
            && self.extra == other.extra
    }
}

pub type InletTemplate = PortTemplateOf<In>;

pub type OutletTemplate = PortTemplateOf<Out>;

impl InletTemplate {
    pub fn try_new(
        ty: TypeRule,
        arity: Arity,
        presence: Presence,
        label: LabelRule,
    ) -> Result<Self, DefaultValueShapeMismatch> {
        match (&presence, &ty) {
            (Presence::Defaulted(_), TypeRule::Fixed(flow)) => {
                validate_default(&presence, flow)?;
            }
            (Presence::Defaulted(_), _) => return Err(DefaultValueShapeMismatch),
            (Presence::Required | Presence::Optional, _) => {}
        }
        Ok(Self {
            ty,
            arity,
            label,
            extra: presence,
        })
    }
}

impl PortTemplateOf<In> {
    #[must_use]
    pub const fn presence(&self) -> &Presence {
        &self.extra
    }
}

impl<D: Direction> PortTemplateOf<D> {
    #[must_use]
    pub const fn ty(&self) -> &TypeRule {
        &self.ty
    }

    #[must_use]
    pub const fn arity(&self) -> Arity {
        self.arity
    }

    #[must_use]
    pub const fn label(&self) -> &LabelRule {
        &self.label
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DefaultValueShapeMismatch;

impl fmt::Display for DefaultValueShapeMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("default inlet value does not belong to the inlet item shape")
    }
}

impl Error for DefaultValueShapeMismatch {}

fn validate_default(presence: &Presence, flow: &Flow) -> Result<(), DefaultValueShapeMismatch> {
    if let Presence::Defaulted(value) = presence
        && !value_matches_shape(value, flow.item())
    {
        return Err(DefaultValueShapeMismatch);
    }
    Ok(())
}

impl OutletTemplate {
    #[must_use]
    pub const fn new(ty: TypeRule, arity: Arity, label: LabelRule) -> Self {
        Self {
            ty,
            arity,
            label,
            extra: (),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypeRule {
    Fixed(Flow),
    FromConfigType(ConfigPath),
    FromSnippet {
        at: ConfigPath,
        inputs: Box<[PortId]>,
        flow: FlowCtor,
    },
    Var(Name),
}

impl TypeRule {
    #[must_use]
    pub const fn kind(&self) -> TypeRuleKind {
        match self {
            Self::Fixed(_) => TypeRuleKind::Fixed,
            Self::FromConfigType(_) => TypeRuleKind::FromConfigType,
            Self::FromSnippet { .. } => TypeRuleKind::FromSnippet,
            Self::Var(_) => TypeRuleKind::Var,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum TypeRuleKind {
    Fixed,
    FromConfigType,
    FromSnippet,
    Var,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FlowCtor {
    Stream,
    Signal { rate: RateExpr },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LabelRule(Label);

impl LabelRule {
    #[must_use]
    pub const fn literal(label: Label) -> Self {
        Self(label)
    }

    #[must_use]
    pub const fn label(&self) -> &Label {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Side {
    Inlet,
    Outlet,
}

impl fmt::Display for Side {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Inlet => "input",
            Self::Outlet => "output",
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryPortRule {
    direction: BoundaryPortDirection,
    side: Side,
    ty: TypeRule,
    label: Label,
}

impl BoundaryPortRule {
    #[must_use]
    pub(crate) const fn new(
        direction: BoundaryPortDirection,
        side: Side,
        ty: TypeRule,
        label: Label,
    ) -> Self {
        Self {
            direction,
            side,
            ty,
            label,
        }
    }

    #[must_use]
    pub const fn direction(&self) -> BoundaryPortDirection {
        self.direction
    }

    #[must_use]
    pub const fn side(&self) -> Side {
        self.side
    }

    #[must_use]
    pub const fn ty(&self) -> &TypeRule {
        &self.ty
    }

    pub(crate) fn undeclared_any(&self, config: &Value, schema: &ConfigSchema) -> bool {
        let supplied = match &self.ty {
            TypeRule::Fixed(flow) => Some(flow),
            TypeRule::FromConfigType(at) => match Self::declared_type(at, config) {
                Ok(None) => Self::absent_flow(at, schema),
                Ok(Some(_)) | Err(_) => None,
            },
            TypeRule::FromSnippet { .. } | TypeRule::Var(_) => None,
        };
        supplied.is_some_and(|flow| matches!(flow.item(), crate::types::Shape::Any))
    }

    fn absent_flow<'schema>(
        at: &ConfigPath,
        schema: &'schema ConfigSchema,
    ) -> Option<&'schema Flow> {
        match schema.get(at)?.required() {
            Required::Omittable { absent } => Some(absent),
            Required::Mandatory | Required::Optional { .. } => None,
        }
    }

    fn declared_type<'value>(
        at: &ConfigPath,
        config: &'value Value,
    ) -> Result<Option<&'value Value>, PortExpansionError> {
        value_at_exact(config, at).map_err(|mismatch| {
            PortExpansionError::BoundaryTypePathMismatch {
                source: at.clone(),
                segment_index: mismatch.segment_index,
                expected: mismatch.expected,
                actual: mismatch.actual,
            }
        })
    }

    pub(crate) fn flow(
        &self,
        config: &Value,
        schema: &ConfigSchema,
    ) -> Result<Flow, PortExpansionError> {
        match &self.ty {
            TypeRule::Fixed(flow) => Ok(flow.clone()),
            TypeRule::FromConfigType(at) => {
                let Some(source) = Self::declared_type(at, config)? else {
                    return Self::absent_flow(at, schema).cloned().ok_or_else(|| {
                        PortExpansionError::BoundaryTypeMissing { source: at.clone() }
                    });
                };
                circular_protocol::port_type::decode_port_flow(source.clone())
                    .map(crate::types::flow_from_port_type)
                    .map_err(|error| PortExpansionError::BoundaryTypeInvalid {
                        source: at.clone(),
                        detail: error.to_string(),
                    })
            }
            TypeRule::FromSnippet { .. } | TypeRule::Var(_) => {
                Err(PortExpansionError::BoundaryTypeUnsupported {
                    kind: self.ty.kind(),
                })
            }
        }
    }

    pub(crate) fn expand(
        &self,
        id: PortId,
        config: &Value,
        schema: &ConfigSchema,
    ) -> Result<PortSet, PortExpansionError> {
        let flow = self.flow(config, schema)?;
        Ok(match self.side {
            Side::Inlet => PortSet::try_new(
                vec![
                    InletSpec::try_new_derived(
                        id,
                        flow,
                        Arity::Many,
                        Presence::Required,
                        true,
                        self.label.clone(),
                    )
                    .expect("a required boundary inlet has no default"),
                ],
                Vec::new(),
            )
            .expect("a singleton boundary inlet is unique"),
            Side::Outlet => PortSet::try_new(
                Vec::new(),
                vec![OutletSpec::new_derived(
                    id,
                    flow,
                    Arity::Many,
                    true,
                    self.label.clone(),
                )],
            )
            .expect("a singleton boundary outlet is unique"),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExpansionElement {
    Presence,
    Index(usize),
    Key(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortOrigin {
    Fixed {
        side: Side,
        index: usize,
    },
    Dynamic {
        side: Side,
        rule_index: usize,
        source: ConfigPath,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExpansionStorage {
    Ports,
    CollisionIndex,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidCountReason {
    Negative,
    OutOfRange,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GeneratedPortIdError {
    EmptyKey,
    InvalidKeyByte { offset: usize, byte: u8 },
    InvalidPortId(PortIdError),
}

impl fmt::Display for GeneratedPortIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyKey => formatter.write_str("dynamic port key must not be empty"),
            Self::InvalidKeyByte { offset, byte } => write!(
                formatter,
                "dynamic port key has non-canonical byte {byte:#04x} at offset {offset}"
            ),
            Self::InvalidPortId(reason) => reason.fmt(formatter),
        }
    }
}

impl Error for GeneratedPortIdError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidPortId(reason) => Some(reason),
            Self::EmptyKey | Self::InvalidKeyByte { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PortExpansionError {
    BoundaryIdentityRequired,
    BoundaryTypeMissing {
        source: ConfigPath,
    },
    BoundaryTypePathMismatch {
        source: ConfigPath,
        segment_index: usize,
        expected: ValueKind,
        actual: ValueKind,
    },
    BoundaryTypeInvalid {
        source: ConfigPath,
        detail: String,
    },
    BoundaryTypeUnsupported {
        kind: TypeRuleKind,
    },
    MissingSource {
        rule_index: usize,
        source: ConfigPath,
        expansion: Expansion,
    },
    PathTypeMismatch {
        rule_index: usize,
        source: ConfigPath,
        segment_index: usize,
        expected: ValueKind,
        actual: ValueKind,
    },
    SourceTypeMismatch {
        rule_index: usize,
        source: ConfigPath,
        expansion: Expansion,
        expected: ValueKind,
        actual: ValueKind,
    },
    InvalidCount {
        rule_index: usize,
        source: ConfigPath,
        value: Value,
        reason: InvalidCountReason,
    },
    InvalidGeneratedName {
        rule_index: usize,
        source: ConfigPath,
        element: ExpansionElement,
        attempted: String,
        reason: GeneratedPortIdError,
    },
    Collision {
        side: Side,
        id: PortId,
        first: PortOrigin,
        second: PortOrigin,
    },
    UnsupportedTypeRule {
        rule_index: usize,
        source: ConfigPath,
        kind: TypeRuleKind,
    },
    InvalidConfigType {
        rule_index: usize,
        source: ConfigPath,
        detail: String,
    },
    SizeOverflow {
        side: Side,
    },
    CapacityUnavailable {
        side: Side,
        storage: ExpansionStorage,
        requested: usize,
    },
    MissingFlowChoice {
        side: Side,
        port: PortId,
        source: ConfigPath,
    },
    UnknownFlowChoice {
        side: Side,
        port: PortId,
        source: ConfigPath,
        value: Value,
    },
}

impl PortExpansionError {
    #[must_use]
    pub fn invalid_count_value(&self) -> Option<&Value> {
        match self {
            Self::InvalidCount { value, .. } => Some(value),
            _ => None,
        }
    }
}

impl fmt::Display for PortExpansionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BoundaryIdentityRequired => formatter.write_str(
                "registered boundary port expansion requires its protocol-derived physical identity",
            ),
            Self::BoundaryTypeMissing { source } => write!(
                formatter,
                "boundary port type is missing config source {source}"
            ),
            Self::BoundaryTypePathMismatch {
                source,
                segment_index,
                expected,
                actual,
            } => write!(
                formatter,
                "boundary port type path {source} segment {segment_index} expected {expected} but found {actual}"
            ),
            Self::BoundaryTypeInvalid { source, detail } => write!(
                formatter,
                "boundary port type at {source} is not the canonical type-expression carrier: {detail}"
            ),
            Self::BoundaryTypeUnsupported { kind } => write!(
                formatter,
                "boundary port type rule {kind:?} has no boundary resolver"
            ),
            Self::MissingSource {
                rule_index,
                source,
                expansion,
            } => write!(
                formatter,
                "dynamic port rule {rule_index} by {} is missing config source {source}",
                expansion_text(*expansion)
            ),
            Self::PathTypeMismatch {
                rule_index,
                source,
                segment_index,
                expected,
                actual,
            } => write!(
                formatter,
                "dynamic port rule {rule_index} cannot traverse segment {segment_index} of {source}: expected {}, got {}",
                expected.as_str(),
                actual.as_str()
            ),
            Self::SourceTypeMismatch {
                rule_index,
                source,
                expansion,
                expected,
                actual,
            } => write!(
                formatter,
                "dynamic port rule {rule_index} by {} expected {} at {source}, got {}",
                expansion_text(*expansion),
                expected.as_str(),
                actual.as_str()
            ),
            Self::InvalidCount {
                rule_index,
                source,
                value,
                reason,
            } => write!(
                formatter,
                "dynamic port rule {rule_index} has invalid count {} at {source}: {}",
                circular_core::spelling::ValueText(value),
                match reason {
                    InvalidCountReason::Negative => "a count cannot be negative",
                    InvalidCountReason::OutOfRange => "the count is out of range",
                }
            ),
            Self::InvalidGeneratedName {
                rule_index,
                source,
                element,
                attempted,
                reason,
            } => write!(
                formatter,
                "dynamic port rule {rule_index} cannot name {} from {source} as {}: {reason}",
                match element {
                    ExpansionElement::Presence => "the present value".to_owned(),
                    ExpansionElement::Index(index) => format!("element {index}"),
                    ExpansionElement::Key(key) =>
                        format!("key {}", circular_core::spelling::Quoted(key)),
                },
                circular_core::spelling::Quoted(attempted)
            ),
            Self::Collision {
                side,
                id,
                first,
                second,
            } => write!(
                formatter,
                "dynamic port expansion collides on {side} port {id}: {} and {} both name it",
                OriginText(first),
                OriginText(second)
            ),
            Self::UnsupportedTypeRule {
                rule_index,
                source,
                kind,
            } => write!(
                formatter,
                "dynamic port rule {rule_index} at {source} requires unsupported type rule {kind:?}"
            ),
            Self::InvalidConfigType {
                rule_index,
                source,
                detail,
            } => write!(
                formatter,
                "dynamic port rule {rule_index} has invalid canonical Flow at {source}: {detail}"
            ),
            Self::SizeOverflow { side } => {
                write!(formatter, "expanded {side} port count exceeds usize")
            }
            Self::CapacityUnavailable {
                side,
                storage,
                requested,
            } => write!(
                formatter,
                "cannot reserve {storage:?} storage for {requested} expanded {side} ports"
            ),
            Self::MissingFlowChoice { side, port, source } => write!(
                formatter,
                "{side} port {port} requires a declared kind at config source {source}"
            ),
            Self::UnknownFlowChoice {
                side,
                port,
                source,
                value,
            } => write!(
                formatter,
                "{side} port {port} has no registered kind for {} at config source {source}",
                circular_core::spelling::ValueText(value)
            ),
        }
    }
}

const fn expansion_text(expansion: Expansion) -> &'static str {
    match expansion {
        Expansion::Presence => "presence",
        Expansion::Count => "count",
        Expansion::Keys => "keys",
    }
}

struct OriginText<'a>(&'a PortOrigin);

impl fmt::Display for OriginText<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            PortOrigin::Fixed { side, index } => write!(formatter, "fixed {side} port {index}"),
            PortOrigin::Dynamic {
                side,
                rule_index,
                source,
            } => write!(
                formatter,
                "dynamic {side} port rule {rule_index} at {source}"
            ),
        }
    }
}

impl Error for PortExpansionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidGeneratedName { reason, .. } => Some(reason),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PathTypeMismatch {
    segment_index: usize,
    expected: ValueKind,
    actual: ValueKind,
}

fn value_at_exact<'value>(
    root: &'value Value,
    path: &ConfigPath,
) -> Result<Option<&'value Value>, PathTypeMismatch> {
    let mut current = root;
    for (segment_index, segment) in path.segments().iter().enumerate() {
        current = match segment {
            Segment::Key(key) => {
                let Value::Object(object) = current else {
                    return Err(PathTypeMismatch {
                        segment_index,
                        expected: ValueKind::Object,
                        actual: current.kind(),
                    });
                };
                let Some(next) = object.get(key) else {
                    return Ok(None);
                };
                next
            }
            Segment::Index(index) => {
                let Value::Array(array) = current else {
                    return Err(PathTypeMismatch {
                        segment_index,
                        expected: ValueKind::Array,
                        actual: current.kind(),
                    });
                };
                let Ok(index) = usize::try_from(*index) else {
                    return Ok(None);
                };
                let Some(next) = array.get(index) else {
                    return Ok(None);
                };
                next
            }
        };
    }
    Ok(Some(current))
}

fn count_of(value: &Value) -> Option<Result<usize, InvalidCountReason>> {
    match value {
        Value::Int(count) => Some(checked_count(*count)),
        _ => None,
    }
}

fn checked_count(value: i64) -> Result<usize, InvalidCountReason> {
    if value < 0 {
        return Err(InvalidCountReason::Negative);
    }
    usize::try_from(value).map_err(|_| InvalidCountReason::OutOfRange)
}

const fn can_resolve_template_type(template: &PortTemplate) -> bool {
    matches!(
        template.ty(),
        TypeRule::Fixed(_) | TypeRule::FromConfigType(_) | TypeRule::FromSnippet { .. }
    )
}

fn resolve_template_type(
    rule_index: usize,
    rule: &DynamicRule,
    config: &Value,
    schema: &crate::config::ConfigSchema,
    fixed: &PortSet,
) -> Result<Flow, PortExpansionError> {
    match rule.template().ty() {
        TypeRule::Fixed(flow) => Ok(flow.clone()),
        TypeRule::FromSnippet { at, inputs, flow } => {
            let unsupported = || PortExpansionError::UnsupportedTypeRule {
                rule_index,
                source: rule.source().clone(),
                kind: TypeRuleKind::FromSnippet,
            };
            let mode = schema
                .get(at)
                .and_then(crate::config::ConfigSlot::snippet)
                .ok_or_else(unsupported)?
                .mode();
            let source = value_at_exact(config, at)
                .map_err(|mismatch| PortExpansionError::PathTypeMismatch {
                    rule_index,
                    source: at.clone(),
                    segment_index: mismatch.segment_index,
                    expected: mismatch.expected,
                    actual: mismatch.actual,
                })?
                .ok_or_else(|| PortExpansionError::MissingSource {
                    rule_index,
                    source: at.clone(),
                    expansion: rule.expansion(),
                })?;

            let declared = inputs
                .iter()
                .map(|port| port.as_str().to_owned())
                .chain(
                    mode.implicit_bindings()
                        .iter()
                        .map(|name| (*name).to_owned()),
                )
                .collect::<std::collections::BTreeSet<_>>();
            let snippet = circular_expr::snippet::from_config_value(source, &declared, mode)
                .map_err(|error| PortExpansionError::InvalidConfigType {
                    rule_index,
                    source: at.clone(),
                    detail: error.to_string(),
                })?;

            let mut environment = circular_expr::shapes::ShapeEnv::new();
            for port in inputs {
                let inlet = fixed
                    .inlets()
                    .iter()
                    .find(|inlet| inlet.id() == port)
                    .ok_or_else(unsupported)?;
                environment.insert(
                    port.as_str().to_owned(),
                    crate::types::to_unnamed_shape(inlet.ty().item()),
                );
            }

            if let Some(split) = snippet.kind_split(&environment) {
                return Err(PortExpansionError::InvalidConfigType {
                    rule_index,
                    source: at.clone(),
                    detail: format!("ConfigRejected: {split}"),
                });
            }
            let item = crate::types::from_unnamed_shape(&snippet.output_shape(&environment));
            Ok(match flow {
                FlowCtor::Stream => Flow::Stream(item),
                FlowCtor::Signal { rate } => Flow::Signal {
                    item,
                    rate: rate.clone(),
                },
            })
        }
        TypeRule::FromConfigType(at) => {
            let source = value_at_exact(config, at)
                .map_err(|mismatch| PortExpansionError::PathTypeMismatch {
                    rule_index,
                    source: at.clone(),
                    segment_index: mismatch.segment_index,
                    expected: mismatch.expected,
                    actual: mismatch.actual,
                })?
                .ok_or_else(|| PortExpansionError::MissingSource {
                    rule_index,
                    source: at.clone(),
                    expansion: rule.expansion(),
                })?;
            circular_protocol::port_type::decode_port_flow(source.clone())
                .map(crate::types::flow_from_port_type)
                .map_err(|error| PortExpansionError::InvalidConfigType {
                    rule_index,
                    source: at.clone(),
                    detail: error.to_string(),
                })
        }
        TypeRule::Var(_) => Err(PortExpansionError::UnsupportedTypeRule {
            rule_index,
            source: rule.source().clone(),
            kind: rule.template().ty().kind(),
        }),
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ExpansionCardinality {
    inlets: usize,
    outlets: usize,
}

#[derive(Clone, Debug)]
enum ExpansionElements<'value> {
    Empty,
    Presence {
        id: AuthoredPortId,
    },
    Count {
        prefix: AuthoredPortId,
        count: usize,
    },
    Keys {
        prefix: AuthoredPortId,
        object: &'value ObjectValue,
    },
}

impl ExpansionElements<'_> {
    fn len(&self) -> usize {
        match self {
            Self::Empty => 0,
            Self::Presence { .. } => 1,
            Self::Count { count, .. } => *count,
            Self::Keys { object, .. } => object.len(),
        }
    }
}

fn preflight_expansion(
    port_rule: &PortRule,
    config: &Value,
) -> Result<ExpansionCardinality, PortExpansionError> {
    let mut cardinality = ExpansionCardinality::default();
    for (rule_index, rule) in port_rule.dynamic().iter().enumerate() {
        if !can_resolve_template_type(rule.template()) {
            return Err(PortExpansionError::UnsupportedTypeRule {
                rule_index,
                source: rule.source().clone(),
                kind: rule.template().ty().kind(),
            });
        }
        let count = elements(rule_index, rule, config)?.len();
        let target = match rule.template() {
            PortTemplate::Inlet(_) => &mut cardinality.inlets,
            PortTemplate::Outlet(_) => &mut cardinality.outlets,
        };
        *target = target
            .checked_add(count)
            .ok_or(PortExpansionError::SizeOverflow {
                side: template_side(rule.template()),
            })?;
    }
    Ok(cardinality)
}

fn elements<'value>(
    rule_index: usize,
    rule: &DynamicRule,
    config: &'value Value,
) -> Result<ExpansionElements<'value>, PortExpansionError> {
    let value = value_at_exact(config, rule.source()).map_err(|mismatch| {
        PortExpansionError::PathTypeMismatch {
            rule_index,
            source: rule.source().clone(),
            segment_index: mismatch.segment_index,
            expected: mismatch.expected,
            actual: mismatch.actual,
        }
    })?;
    match rule.rule() {
        ExpansionRule::Presence { id } => {
            if value.is_some() {
                Ok(ExpansionElements::Presence { id: id.clone() })
            } else {
                Ok(ExpansionElements::Empty)
            }
        }
        ExpansionRule::Count { prefix } => {
            let value = value.ok_or_else(|| PortExpansionError::MissingSource {
                rule_index,
                source: rule.source().clone(),
                expansion: rule.expansion(),
            })?;
            let Some(count) = count_of(value) else {
                return Err(PortExpansionError::SourceTypeMismatch {
                    rule_index,
                    source: rule.source().clone(),
                    expansion: rule.expansion(),
                    expected: ValueKind::Int,
                    actual: value.kind(),
                });
            };
            let count = count.map_err(|reason| PortExpansionError::InvalidCount {
                rule_index,
                source: rule.source().clone(),
                value: value.clone(),
                reason,
            })?;
            Ok(ExpansionElements::Count {
                prefix: prefix.clone(),
                count,
            })
        }
        ExpansionRule::Keys { prefix } => {
            let value = value.ok_or_else(|| PortExpansionError::MissingSource {
                rule_index,
                source: rule.source().clone(),
                expansion: rule.expansion(),
            })?;
            let Value::Object(object) = value else {
                return Err(PortExpansionError::SourceTypeMismatch {
                    rule_index,
                    source: rule.source().clone(),
                    expansion: rule.expansion(),
                    expected: ValueKind::Object,
                    actual: value.kind(),
                });
            };
            Ok(ExpansionElements::Keys {
                prefix: prefix.clone(),
                object,
            })
        }
    }
}

fn generated_id(
    rule_index: usize,
    source: &ConfigPath,
    element: ExpansionElement,
    derived: Result<PortId, (String, GeneratedPortIdError)>,
) -> Result<PortId, PortExpansionError> {
    derived.map_err(
        |(attempted, reason)| PortExpansionError::InvalidGeneratedName {
            rule_index,
            source: source.clone(),
            element,
            attempted,
            reason,
        },
    )
}

const fn template_side(template: &PortTemplate) -> Side {
    match template {
        PortTemplate::Inlet(_) => Side::Inlet,
        PortTemplate::Outlet(_) => Side::Outlet,
    }
}

fn total_port_count(side: Side, fixed: usize, dynamic: usize) -> Result<usize, PortExpansionError> {
    fixed
        .checked_add(dynamic)
        .ok_or(PortExpansionError::SizeOverflow { side })
}

fn reserve_ports<T>(
    ports: &mut Vec<T>,
    side: Side,
    requested: usize,
) -> Result<(), PortExpansionError> {
    ports
        .try_reserve_exact(requested)
        .map_err(|_| PortExpansionError::CapacityUnavailable {
            side,
            storage: ExpansionStorage::Ports,
            requested,
        })
}

fn fixed_origins<D: Direction>(
    fixed: &[PortSpec<D>],
    requested: usize,
) -> Result<HashMap<PortId, PortOrigin>, PortExpansionError> {
    let mut origins = HashMap::new();
    origins
        .try_reserve(requested)
        .map_err(|_| PortExpansionError::CapacityUnavailable {
            side: D::SIDE,
            storage: ExpansionStorage::CollisionIndex,
            requested,
        })?;
    origins.extend(fixed.iter().enumerate().map(|(index, port)| {
        (
            port.id().clone(),
            PortOrigin::Fixed {
                side: D::SIDE,
                index,
            },
        )
    }));
    Ok(origins)
}

#[allow(clippy::too_many_arguments)]
fn push_expanded_port(
    rule_index: usize,
    rule: &DynamicRule,
    flow: &Flow,
    id: PortId,
    inlets: &mut Vec<InletSpec>,
    outlets: &mut Vec<OutletSpec>,
    inlet_origins: &mut HashMap<PortId, PortOrigin>,
    outlet_origins: &mut HashMap<PortId, PortOrigin>,
) -> Result<(), PortExpansionError> {
    match rule.template() {
        PortTemplate::Inlet(template) => {
            push_expanded_port_of(rule_index, rule, template, flow, id, inlets, inlet_origins)
        }
        PortTemplate::Outlet(template) => push_expanded_port_of(
            rule_index,
            rule,
            template,
            flow,
            id,
            outlets,
            outlet_origins,
        ),
    }
}

fn push_expanded_port_of<D: Direction>(
    rule_index: usize,
    rule: &DynamicRule,
    template: &PortTemplateOf<D>,
    flow: &Flow,
    id: PortId,
    ports: &mut Vec<PortSpec<D>>,
    origins: &mut HashMap<PortId, PortOrigin>,
) -> Result<(), PortExpansionError> {
    let origin = PortOrigin::Dynamic {
        side: D::SIDE,
        rule_index,
        source: rule.source().clone(),
    };
    insert_origin(origins, D::SIDE, &id, &origin)?;
    ports.push(PortSpec {
        id,
        ty: flow.clone(),
        arity: template.arity(),
        primary: false,
        label: template.label().label().clone(),
        extra: template.extra.clone(),
    });
    Ok(())
}

fn insert_origin(
    origins: &mut HashMap<PortId, PortOrigin>,
    side: Side,
    id: &PortId,
    origin: &PortOrigin,
) -> Result<(), PortExpansionError> {
    match origins.entry(id.clone()) {
        Entry::Vacant(entry) => {
            entry.insert(origin.clone());
            Ok(())
        }
        Entry::Occupied(entry) => Err(PortExpansionError::Collision {
            side,
            id: id.clone(),
            first: entry.get().clone(),
            second: origin.clone(),
        }),
    }
}

fn indexed_port_id(
    prefix: &AuthoredPortId,
    index: usize,
) -> Result<PortId, (String, GeneratedPortIdError)> {
    let attempted = format!("{}_{}", prefix.as_port_id().as_str(), index);
    PortId::try_authored(attempted.clone())
        .map(AuthoredPortId::into_port_id)
        .map_err(|reason| (attempted, GeneratedPortIdError::InvalidPortId(reason)))
}

fn from_key_port_id(
    prefix: &AuthoredPortId,
    key: &str,
) -> Result<PortId, (String, GeneratedPortIdError)> {
    let attempted = format!("{}_{}", prefix.as_port_id().as_str(), key);
    if key.is_empty() {
        return Err((attempted, GeneratedPortIdError::EmptyKey));
    }
    if let Some((offset, byte)) = key
        .as_bytes()
        .iter()
        .copied()
        .enumerate()
        .find(|(_, byte)| !byte.is_ascii_lowercase() && !byte.is_ascii_digit() && *byte != b'_')
    {
        return Err((
            attempted,
            GeneratedPortIdError::InvalidKeyByte { offset, byte },
        ));
    }
    PortId::try_authored(attempted.clone())
        .map(AuthoredPortId::into_port_id)
        .map_err(|reason| (attempted, GeneratedPortIdError::InvalidPortId(reason)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{BaseShape, Shape};

    fn label(value: &'static str) -> Label {
        Label::try_from_static(value).expect("test label is canonical")
    }

    fn inlet(id: &'static str) -> InletSpec {
        InletSpec::try_new(
            PortId::try_authored_static(id).expect("test id is canonical"),
            Flow::Stream(Shape::Base(BaseShape::Float)),
            Arity::Many,
            Presence::Required,
            true,
            label(id),
        )
        .expect("required inlet has no default to mismatch")
    }

    fn authored(id: &'static str) -> AuthoredPortId {
        PortId::try_authored_static(id).expect("test id is canonical")
    }

    fn number_flow() -> Flow {
        Flow::Stream(Shape::Base(BaseShape::Float))
    }

    fn outlet(id: &'static str) -> OutletSpec {
        OutletSpec::new(authored(id), number_flow(), Arity::Many, true, label(id))
    }

    fn outlet_rule(source: ConfigPath, rule: ExpansionRule, ty: TypeRule) -> DynamicRule {
        DynamicRule::new(
            source,
            rule,
            PortTemplate::Outlet(OutletTemplate::new(
                ty,
                Arity::One,
                LabelRule::literal(label("Dynamic")),
            )),
        )
    }

    fn inlet_rule(source: ConfigPath, rule: ExpansionRule) -> DynamicRule {
        DynamicRule::new(
            source,
            rule,
            PortTemplate::Inlet(
                InletTemplate::try_new(
                    TypeRule::Fixed(number_flow()),
                    Arity::Many,
                    Presence::Optional,
                    LabelRule::literal(label("Dynamic")),
                )
                .expect("optional inlet has no default to mismatch"),
            ),
        )
    }

    fn object<const N: usize>(entries: [(&'static str, Value); N]) -> Value {
        Value::object(entries).expect("test object keys are unique")
    }

    fn ids(ports: &[OutletSpec]) -> Vec<&str> {
        ports.iter().map(|port| port.id().as_str()).collect()
    }

    #[test]
    fn authored_and_derived_names_have_distinct_reserved_boundaries() {
        let reference = PortId::try_new("_error").expect("reserved plan reference is canonical");
        assert_eq!(
            PortId::try_authored_static("_error"),
            Err(PortIdError::Reserved)
        );
        assert_eq!(
            reference,
            PortId::try_derived("_error".to_owned()).expect("derived error port is canonical")
        );
    }

    #[test]
    fn port_set_rejects_duplicate_ids_per_direction() {
        let error = PortSet::try_new(vec![inlet("event"), inlet("event")], Vec::new())
            .expect_err("duplicate inlet must be rejected");
        assert_eq!(error.side(), Side::Inlet);
        assert_eq!(error.id().as_str(), "event");
    }

    #[test]
    fn inlet_and_outlet_types_make_outlet_presence_unrepresentable() {
        let outlet = OutletSpec::new(
            PortId::try_authored_static("event").expect("test id is canonical"),
            Flow::Stream(Shape::Base(BaseShape::Float)),
            Arity::Many,
            true,
            label("event"),
        );
        let set = PortSet::try_new(vec![inlet("event")], vec![outlet])
            .expect("same id may occur on opposite sides");
        assert_eq!(set.inlets().len(), 1);
        assert_eq!(set.outlets().len(), 1);
    }

    #[test]
    fn default_value_must_match_the_inlet_item_shape() {
        let number = Flow::Stream(Shape::Base(BaseShape::Float));
        let id = || PortId::try_authored_static("value").unwrap();
        assert!(
            InletSpec::try_new(
                id(),
                number.clone(),
                Arity::One,
                Presence::Defaulted(Value::float(1.0)),
                true,
                label("Value"),
            )
            .is_ok()
        );
        assert_eq!(
            InletSpec::try_new(
                id(),
                number,
                Arity::One,
                Presence::Defaulted(Value::Bool(true)),
                true,
                label("Value"),
            ),
            Err(DefaultValueShapeMismatch)
        );
        assert_eq!(
            InletTemplate::try_new(
                TypeRule::Var(Name::from_static("T")),
                Arity::One,
                Presence::Defaulted(Value::float(0.0)),
                LabelRule::literal(label("Value")),
            ),
            Err(DefaultValueShapeMismatch)
        );
    }

    #[test]
    fn presence_reads_existence_only_and_fixed_ports_always_remain() {
        let source = ConfigPath::root().join_key("enabled");
        let dynamic = outlet_rule(
            source,
            ExpansionRule::Presence {
                id: authored("present"),
            },
            TypeRule::Fixed(number_flow()),
        );
        let rule = PortRule::new(
            PortSet::try_new(vec![inlet("input")], Vec::new()).unwrap(),
            vec![dynamic].into_boxed_slice(),
        );

        let absent = rule
            .expand(&object([]), &crate::config::ConfigSchema::empty())
            .expect("absence is an empty Presence expansion");
        assert_eq!(
            absent
                .inlets()
                .iter()
                .map(|port| port.id().as_str())
                .collect::<Vec<_>>(),
            ["input"]
        );
        assert!(absent.outlets().is_empty());

        let false_value = rule
            .expand(
                &object([("enabled", Value::Bool(false))]),
                &crate::config::ConfigSchema::empty(),
            )
            .expect("false is still present");
        assert_eq!(ids(false_value.outlets()), ["present"]);
        assert_eq!(false_value.outlets()[0].ty(), &number_flow());
        assert_eq!(false_value.outlets()[0].arity(), Arity::One);
        assert!(!false_value.outlets()[0].primary());
        assert_eq!(false_value.outlets()[0].label(), &label("Dynamic"));
    }

    #[test]
    fn count_expands_zero_or_consecutive_indices_in_rule_order() {
        let first_source = ConfigPath::root().join_key("first");
        let second_source = ConfigPath::root().join_key("second");
        let rule = PortRule::new(
            PortSet::try_new(Vec::new(), vec![outlet("fixed")]).unwrap(),
            vec![
                outlet_rule(
                    first_source,
                    ExpansionRule::Count {
                        prefix: authored("first"),
                    },
                    TypeRule::Fixed(number_flow()),
                ),
                outlet_rule(
                    second_source,
                    ExpansionRule::Count {
                        prefix: authored("second"),
                    },
                    TypeRule::Fixed(number_flow()),
                ),
            ]
            .into_boxed_slice(),
        );
        let config = object([("second", Value::int(2)), ("first", Value::int(3))]);

        let expanded = rule
            .expand(&config, &crate::config::ConfigSchema::empty())
            .expect("non-negative integer counts");
        assert_eq!(
            ids(expanded.outlets()),
            [
                "fixed", "first_0", "first_1", "first_2", "second_0", "second_1",
            ]
        );

        let empty = rule
            .expand(
                &object([("first", Value::int(0)), ("second", Value::int(0))]),
                &crate::config::ConfigSchema::empty(),
            )
            .expect("zero counts are empty Count expansions");
        assert_eq!(ids(empty.outlets()), ["fixed"]);
    }

    #[test]
    fn missing_wrong_type_and_invalid_count_are_distinct_failures() {
        let source = ConfigPath::root().join_key("count");
        let rule = PortRule::new(
            PortSet::empty(),
            vec![outlet_rule(
                source.clone(),
                ExpansionRule::Count {
                    prefix: authored("item"),
                },
                TypeRule::Fixed(number_flow()),
            )]
            .into_boxed_slice(),
        );

        assert!(matches!(
            rule.expand(&object([]), &crate::config::ConfigSchema::empty()),
            Err(PortExpansionError::MissingSource {
                rule_index: 0,
                source: missing,
                expansion: Expansion::Count,
            }) if missing == source
        ));
        assert!(matches!(
            rule.expand(&object([("count", Value::Bool(true))]), &crate::config::ConfigSchema::empty()),
            Err(PortExpansionError::SourceTypeMismatch {
                rule_index: 0,
                source: wrong,
                expansion: Expansion::Count,
                expected: ValueKind::Int,
                actual: ValueKind::Bool,
            }) if wrong == source
        ));

        for wrong_kind in [Value::float(1.5), Value::float(f64::NAN)] {
            assert!(matches!(
                rule.expand(
                    &object([("count", wrong_kind)]),
                    &crate::config::ConfigSchema::empty()
                ),
                Err(PortExpansionError::SourceTypeMismatch {
                    expected: ValueKind::Int,
                    actual: ValueKind::Float,
                    ..
                })
            ));
        }

        let cases = [(-1_i64, InvalidCountReason::Negative)];
        for (value, expected_reason) in cases {
            let error = rule
                .expand(
                    &object([("count", Value::int(value))]),
                    &crate::config::ConfigSchema::empty(),
                )
                .expect_err("invalid count must reject the whole expansion");
            assert!(matches!(
                error,
                PortExpansionError::InvalidCount {
                    rule_index: 0,
                    source: ref invalid,
                    reason,
                    ..
                } if invalid == &source && reason == expected_reason
            ));
            assert_eq!(error.invalid_count_value(), Some(&Value::int(value)));
        }
    }

    #[test]
    fn huge_valid_count_fails_reservation_before_materialization() {
        let source = ConfigPath::root().join_key("count");
        let rule = PortRule::new(
            PortSet::empty(),
            vec![outlet_rule(
                source,
                ExpansionRule::Count {
                    prefix: authored("item"),
                },
                TypeRule::Fixed(number_flow()),
            )]
            .into_boxed_slice(),
        );
        let impossible_count = ((isize::MAX as usize / size_of::<OutletSpec>()) + 1_024) as i64;
        let requested = checked_count(impossible_count).expect("fixture is a valid usize count");

        assert_eq!(
            rule.expand(
                &object([("count", Value::int(impossible_count))]),
                &crate::config::ConfigSchema::empty()
            ),
            Err(PortExpansionError::CapacityUnavailable {
                side: Side::Outlet,
                storage: ExpansionStorage::Ports,
                requested,
            })
        );
    }

    #[test]
    fn keys_follow_canonical_utf8_byte_order_not_insertion_order() {
        let source = ConfigPath::root().join_key("cases");
        let rule = PortRule::new(
            PortSet::empty(),
            vec![outlet_rule(
                source,
                ExpansionRule::Keys {
                    prefix: authored("case"),
                },
                TypeRule::Fixed(number_flow()),
            )]
            .into_boxed_slice(),
        );
        let left = object([(
            "cases",
            object([
                ("z", Value::Null),
                ("aa", Value::Null),
                ("a_", Value::Null),
                ("a0", Value::Null),
            ]),
        )]);
        let right = object([(
            "cases",
            object([
                ("a0", Value::Null),
                ("a_", Value::Null),
                ("aa", Value::Null),
                ("z", Value::Null),
            ]),
        )]);

        let left = rule
            .expand(&left, &crate::config::ConfigSchema::empty())
            .expect("all keys are canonical");
        let right = rule
            .expand(&right, &crate::config::ConfigSchema::empty())
            .expect("all keys are canonical");
        assert_eq!(left, right);
        assert_eq!(
            ids(left.outlets()),
            ["case_a0", "case_a_", "case_aa", "case_z"]
        );

        let empty = rule
            .expand(
                &object([("cases", object([]))]),
                &crate::config::ConfigSchema::empty(),
            )
            .expect("empty object is a successful empty Keys expansion");
        assert!(empty.outlets().is_empty());
    }

    #[test]
    fn config_path_keeps_keys_indices_missing_and_container_errors_distinct() {
        let source = ConfigPath::root()
            .join_key("groups")
            .join_index(1)
            .join_key("count");
        let rule = PortRule::new(
            PortSet::empty(),
            vec![outlet_rule(
                source.clone(),
                ExpansionRule::Count {
                    prefix: authored("nested"),
                },
                TypeRule::Fixed(number_flow()),
            )]
            .into_boxed_slice(),
        );
        let config = object([(
            "groups",
            Value::Array(vec![Value::Null, object([("count", Value::int(2))])]),
        )]);
        assert_eq!(
            ids(rule
                .expand(&config, &crate::config::ConfigSchema::empty())
                .unwrap()
                .outlets()),
            ["nested_0", "nested_1"]
        );

        assert!(matches!(
            rule.expand(&object([]), &crate::config::ConfigSchema::empty()),
            Err(PortExpansionError::MissingSource { source: missing, .. }) if missing == source
        ));
        assert!(matches!(
            rule.expand(&object([("groups", Value::Bool(false))]), &crate::config::ConfigSchema::empty()),
            Err(PortExpansionError::PathTypeMismatch {
                rule_index: 0,
                source: wrong,
                segment_index: 1,
                expected: ValueKind::Array,
                actual: ValueKind::Bool,
            }) if wrong == source
        ));
    }

    #[test]
    fn fixed_and_prior_dynamic_collisions_report_both_origins() {
        let count_source = ConfigPath::root().join_key("count");
        let fixed_collision = PortRule::new(
            PortSet::try_new(Vec::new(), vec![outlet("item_0")]).unwrap(),
            vec![outlet_rule(
                count_source.clone(),
                ExpansionRule::Count {
                    prefix: authored("item"),
                },
                TypeRule::Fixed(number_flow()),
            )]
            .into_boxed_slice(),
        );
        assert_eq!(
            fixed_collision.expand(
                &object([("count", Value::int(1))]),
                &crate::config::ConfigSchema::empty()
            ),
            Err(PortExpansionError::Collision {
                side: Side::Outlet,
                id: PortId::try_new("item_0").unwrap(),
                first: PortOrigin::Fixed {
                    side: Side::Outlet,
                    index: 0,
                },
                second: PortOrigin::Dynamic {
                    side: Side::Outlet,
                    rule_index: 0,
                    source: count_source,
                },
            })
        );

        let first_source = ConfigPath::root().join_key("first");
        let second_source = ConfigPath::root().join_key("second");
        let dynamic_collision = PortRule::new(
            PortSet::empty(),
            vec![
                outlet_rule(
                    first_source.clone(),
                    ExpansionRule::Presence {
                        id: authored("same"),
                    },
                    TypeRule::Fixed(number_flow()),
                ),
                outlet_rule(
                    second_source.clone(),
                    ExpansionRule::Presence {
                        id: authored("same"),
                    },
                    TypeRule::Fixed(number_flow()),
                ),
            ]
            .into_boxed_slice(),
        );
        let before = dynamic_collision.clone();
        assert_eq!(
            dynamic_collision.expand(
                &object([("second", Value::Null), ("first", Value::Null),]),
                &crate::config::ConfigSchema::empty()
            ),
            Err(PortExpansionError::Collision {
                side: Side::Outlet,
                id: PortId::try_new("same").unwrap(),
                first: PortOrigin::Dynamic {
                    side: Side::Outlet,
                    rule_index: 0,
                    source: first_source,
                },
                second: PortOrigin::Dynamic {
                    side: Side::Outlet,
                    rule_index: 1,
                    source: second_source,
                },
            })
        );
        assert_eq!(
            dynamic_collision, before,
            "failed expansion mutates no rule state"
        );
    }

    #[test]
    fn direction_domains_allow_the_same_dynamic_name() {
        let inlet_source = ConfigPath::root().join_key("inlet");
        let outlet_source = ConfigPath::root().join_key("outlet");
        let rule = PortRule::new(
            PortSet::empty(),
            vec![
                inlet_rule(
                    inlet_source,
                    ExpansionRule::Presence {
                        id: authored("same"),
                    },
                ),
                outlet_rule(
                    outlet_source,
                    ExpansionRule::Presence {
                        id: authored("same"),
                    },
                    TypeRule::Fixed(number_flow()),
                ),
            ]
            .into_boxed_slice(),
        );
        let expanded = rule
            .expand(
                &object([("outlet", Value::Null), ("inlet", Value::Null)]),
                &crate::config::ConfigSchema::empty(),
            )
            .expect("opposite directions have separate name spaces");

        assert_eq!(expanded.inlets()[0].id().as_str(), "same");
        assert_eq!(expanded.inlets()[0].presence(), &Presence::Optional);
        assert!(!expanded.inlets()[0].primary());
        assert_eq!(expanded.outlets()[0].id().as_str(), "same");
    }

    #[test]
    fn invalid_key_or_overlong_generated_name_rejects_the_whole_result() {
        let source = ConfigPath::root().join_key("keys");
        let rule = PortRule::new(
            PortSet::try_new(Vec::new(), vec![outlet("fixed")]).unwrap(),
            vec![outlet_rule(
                source.clone(),
                ExpansionRule::Keys {
                    prefix: authored("key"),
                },
                TypeRule::Fixed(number_flow()),
            )]
            .into_boxed_slice(),
        );
        let error = rule
            .expand(
                &object([("keys", object([("ok", Value::Null), ("é", Value::Null)]))]),
                &crate::config::ConfigSchema::empty(),
            )
            .expect_err("one invalid key rejects fixed and prior expanded ports together");
        assert!(matches!(
            error,
            PortExpansionError::InvalidGeneratedName {
                rule_index: 0,
                source: invalid_source,
                element: ExpansionElement::Key(key),
                attempted,
                reason: GeneratedPortIdError::InvalidKeyByte { offset: 0, byte: 0xc3 },
            } if invalid_source == source && key == "é" && attempted == "key_é"
        ));

        let empty_key = rule
            .expand(
                &object([("keys", object([("", Value::Null)]))]),
                &crate::config::ConfigSchema::empty(),
            )
            .expect_err("empty key is not normalized into a trailing underscore");
        assert!(matches!(
            empty_key,
            PortExpansionError::InvalidGeneratedName {
                reason: GeneratedPortIdError::EmptyKey,
                ..
            }
        ));

        let long_prefix = PortId::try_authored("p".repeat(31)).unwrap();
        let indexed = PortRule::new(
            PortSet::empty(),
            vec![outlet_rule(
                ConfigPath::root().join_key("count"),
                ExpansionRule::Count {
                    prefix: long_prefix,
                },
                TypeRule::Fixed(number_flow()),
            )]
            .into_boxed_slice(),
        );
        assert!(matches!(
            indexed.expand(
                &object([("count", Value::int(1))]),
                &crate::config::ConfigSchema::empty()
            ),
            Err(PortExpansionError::InvalidGeneratedName {
                element: ExpansionElement::Index(0),
                reason: GeneratedPortIdError::InvalidPortId(PortIdError::TooLong),
                ..
            })
        ));
    }

    #[test]
    fn a_config_type_rule_decodes_the_exact_published_flow() {
        let source = ConfigPath::root().join_key("present");
        let type_at = ConfigPath::root().join_key("type");
        let rule = PortRule::new(
            PortSet::empty(),
            vec![outlet_rule(
                source,
                ExpansionRule::Presence {
                    id: authored("dynamic"),
                },
                TypeRule::FromConfigType(type_at),
            )]
            .into_boxed_slice(),
        );
        let string_flow = circular_protocol::port_type::encode_port_flow(
            &circular_protocol::port_type::PortFlow::Stream(
                circular_protocol::port_type::PortShape::Base(circular_core::BaseShape::String),
            ),
        )
        .expect("canonical String Flow encodes");
        let expanded = rule
            .expand(
                &object([("present", Value::Null), ("type", string_flow)]),
                &crate::config::ConfigSchema::empty(),
            )
            .expect("published Flow resolves without client inference");
        assert_eq!(expanded.outlets().len(), 1);
        assert_eq!(
            expanded.outlets()[0].ty(),
            &Flow::Stream(Shape::Base(BaseShape::String))
        );
    }

    #[test]
    fn a_snippet_rule_with_nothing_to_expand_yields_no_port_instead_of_an_error() {
        let rule = PortRule::new(
            PortSet::empty(),
            vec![outlet_rule(
                ConfigPath::root().join_key("missing"),
                ExpansionRule::Presence {
                    id: authored("dynamic"),
                },
                TypeRule::FromSnippet {
                    at: ConfigPath::root().join_key("missing"),
                    inputs: Box::new([]),
                    flow: FlowCtor::Stream,
                },
            )]
            .into_boxed_slice(),
        );

        let expanded = rule
            .expand(&object([]), &crate::config::ConfigSchema::empty())
            .expect("with nothing to expand there is no port");

        assert!(expanded.outlets().is_empty());
        assert!(expanded.inlets().is_empty());
    }
}
