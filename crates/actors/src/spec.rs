
use std::borrow::Cow;
use std::error::Error;
use std::fmt;

use circular_core::Value;

use crate::capabilities::{EffectDecl, EffectDeclaration, RequireRule, RequireRules};
use crate::config::ConfigSchema;
use crate::ports::{BoundaryPortRule, PortRule};
use crate::registrations::RegistrationAuthority;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Label(Cow<'static, str>);

impl Label {
    pub fn try_from_static(value: &'static str) -> Result<Self, DisplayTextError> {
        validate_label(value)?;
        Ok(Self(Cow::Borrowed(value)))
    }

    pub fn try_from_owned(value: String) -> Result<Self, DisplayTextError> {
        validate_label(&value)?;
        Ok(Self(Cow::Owned(value)))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Description(Cow<'static, str>);

impl Description {
    #[must_use]
    pub const fn from_static(value: &'static str) -> Self {
        Self(Cow::Borrowed(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DisplayTextError {
    EmptyLabel,
    UntrimmedLabel,
    MultilineLabel,
}

impl fmt::Display for DisplayTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::EmptyLabel => "actor label must not be empty",
            Self::UntrimmedLabel => "actor label must not have surrounding whitespace",
            Self::MultilineLabel => "actor label must be one line",
        };
        formatter.write_str(message)
    }
}

impl Error for DisplayTextError {}

fn validate_label(value: &str) -> Result<(), DisplayTextError> {
    if value.is_empty() {
        return Err(DisplayTextError::EmptyLabel);
    }
    if value.trim() != value {
        return Err(DisplayTextError::UntrimmedLabel);
    }
    if value.contains('\n') || value.contains('\r') {
        return Err(DisplayTextError::MultilineLabel);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Display {
    label: Label,
    description: Description,
}

impl Display {
    #[must_use]
    pub const fn label(&self) -> &Label {
        &self.label
    }

    #[must_use]
    pub const fn description(&self) -> &Description {
        &self.description
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FactoryArm {
    Actor,
    Source,
}

impl FactoryArm {
    #[must_use]
    pub const fn is_source(self) -> bool {
        matches!(self, Self::Source)
    }
}

#[derive(Debug)]
pub struct SpecSource<E: EffectDecl> {
    label: Label,
    description: Description,
    ports: PortRule,
    requires: RequireRules<E>,
    effect: E,
    config: ConfigSchema,
    boundary: Option<BoundaryPortRule>,
    /// Opaque type defaults; interpreted only by the SDK view-config interpreter.
    view_config: Option<Value>,
}

impl<E: EffectDecl> SpecSource<E> {
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "the seven data arguments are the exhaustive SpecSource fields; the eighth is the sealed registration authority"
    )]
    pub(crate) const fn new(
        _authority: &RegistrationAuthority,
        label: Label,
        description: Description,
        ports: PortRule,
        requires: RequireRules<E>,
        effect: E,
        config: ConfigSchema,
    ) -> Self {
        Self {
            label,
            description,
            ports,
            requires,
            effect,
            config,
            boundary: None,
            view_config: None,
        }
    }

    /// Attach the one registration-authored pipeline-boundary contract.
    pub(crate) fn with_boundary(mut self, boundary: BoundaryPortRule) -> Self {
        assert!(
            self.boundary.is_none(),
            "boundary contract is assigned once"
        );
        self.boundary = Some(boundary);
        self
    }

    pub(crate) fn with_view_config(mut self, view_config: Value) -> Self {
        assert!(
            self.view_config.is_none(),
            "view defaults are assigned once"
        );
        self.view_config = Some(view_config);
        self
    }

    pub(crate) fn derive(
        self,
        factory: FactoryArm,
        _authority: &RegistrationAuthority,
    ) -> ActorSpec {
        ActorSpec {
            display: Display {
                label: self.label,
                description: self.description,
            },
            ports: self.ports,
            requires: self.requires.into_boxed(),
            effect: self.effect.into_declaration(),
            is_source: factory.is_source(),
            config: self.config,
            boundary: self.boundary,
            view_config: self.view_config,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct ActorSpec {
    display: Display,
    ports: PortRule,
    requires: Box<[RequireRule]>,
    effect: EffectDeclaration,
    is_source: bool,
    config: ConfigSchema,
    boundary: Option<BoundaryPortRule>,
    view_config: Option<Value>,
}

impl ActorSpec {
    #[must_use]
    pub const fn view_config(&self) -> Option<&Value> {
        self.view_config.as_ref()
    }

    #[must_use]
    pub const fn display(&self) -> &Display {
        &self.display
    }

    #[must_use]
    pub const fn ports(&self) -> &PortRule {
        &self.ports
    }

    #[must_use]
    pub const fn requires(&self) -> &[RequireRule] {
        &self.requires
    }

    #[must_use]
    pub const fn effect(&self) -> &EffectDeclaration {
        &self.effect
    }

    #[must_use]
    pub const fn is_source(&self) -> bool {
        self.is_source
    }

    #[must_use]
    pub const fn config(&self) -> &ConfigSchema {
        &self.config
    }

    /// Exact pipeline-boundary semantics, when this registration declares one.
    #[must_use]
    pub const fn boundary(&self) -> Option<&BoundaryPortRule> {
        self.boundary.as_ref()
    }

    pub fn expand_ports(
        &self,
        config: &circular_runtime::FoldedConfig,
    ) -> Result<crate::ports::PortSet, crate::ports::PortExpansionError> {
        self.ports.expand(config.value(), &self.config)
    }

    /// Expand ports with the exact protocol-derived identity required by a
    /// pipeline-boundary registration. Ordinary registrations reject a
    /// supplied id only by ignoring it; the id has no semantic role there.
    pub fn expand_ports_at(
        &self,
        config: &circular_runtime::FoldedConfig,
        boundary_id: Option<crate::ports::PortId>,
    ) -> Result<crate::ports::PortSet, crate::ports::PortExpansionError> {
        match &self.boundary {
            Some(boundary) => boundary.expand(
                boundary_id.ok_or(crate::ports::PortExpansionError::BoundaryIdentityRequired)?,
                config.value(),
                &self.config,
            ),
            None => self.expand_ports(config),
        }
    }

    #[must_use]
    pub fn boundary_undeclared_any(&self, config: &circular_runtime::FoldedConfig) -> bool {
        self.boundary
            .as_ref()
            .is_some_and(|boundary| boundary.undeclared_any(config.value(), &self.config))
    }
}
