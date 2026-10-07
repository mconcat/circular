
use crate::{ActorType, NamedActorId, PortId};
pub use circular_protocol::authored_value::{
    Config, ConfigError, ConfigRecord, ConfigValue, ConfigValueError,
};
pub use circular_protocol::declaration_payload::{
    ActorFlags, Axis, BoardPlacement, BoardPlacementError, DeclaredDelay,
    DeclaredDelayDurationError, DeclaredDelayError, Delivery, EdgeAttrs, GroupName, GroupNameError,
    LayoutCoord, LayoutPoint, LayoutSize, PositiveCapacity, PositiveCapacityError, PreprocessChain,
    PreprocessKind, PreprocessStep, Relation, Shed, ViewSpec, WirePolicy,
};
use std::fmt;

pub type Anchor = circular_protocol::declaration_payload::Anchor<NamedActorId>;

pub type Presentation = circular_protocol::declaration_payload::Presentation<NamedActorId>;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Text(Box<str>);

impl Text {
    #[must_use]
    pub fn new(value: impl Into<Box<str>>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActorDomain {
    actor_type: ActorType,
    config: Config,
}

impl ActorDomain {
    #[must_use]
    pub const fn new(actor_type: ActorType, config: Config) -> Self {
        Self { actor_type, config }
    }

    #[must_use]
    pub const fn actor_type(&self) -> &ActorType {
        &self.actor_type
    }

    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.config
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActorDecl {
    domain: ActorDomain,
    flags: ActorFlags,
    authored_generation: u64,
}

impl ActorDecl {
    #[must_use]
    pub const fn new(domain: ActorDomain, flags: ActorFlags) -> Self {
        Self::new_at_generation(domain, flags, 0)
    }

    #[must_use]
    pub const fn new_at_generation(
        domain: ActorDomain,
        flags: ActorFlags,
        authored_generation: u64,
    ) -> Self {
        Self {
            domain,
            flags,
            authored_generation,
        }
    }

    #[must_use]
    pub const fn domain(&self) -> &ActorDomain {
        &self.domain
    }

    #[must_use]
    pub const fn flags(&self) -> ActorFlags {
        self.flags
    }

    #[must_use]
    pub const fn authored_generation(&self) -> u64 {
        self.authored_generation
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NonContainerActorDecl(ActorDecl);

impl NonContainerActorDecl {
    pub fn try_new(
        domain: ActorDomain,
        flags: ActorFlags,
    ) -> Result<Self, NonContainerActorDeclError> {
        Self::try_new_at_generation(domain, flags, 0)
    }

    pub fn try_new_at_generation(
        domain: ActorDomain,
        flags: ActorFlags,
        authored_generation: u64,
    ) -> Result<Self, NonContainerActorDeclError> {
        if domain.actor_type().is_container() {
            Err(match domain.actor_type() {
                ActorType::Replicator => NonContainerActorDeclError::ReplicatorRequiresScope,
                _ => NonContainerActorDeclError::PipelineActorRequiresScope,
            })
        } else {
            Ok(Self(ActorDecl::new_at_generation(
                domain,
                flags,
                authored_generation,
            )))
        }
    }

    #[must_use]
    pub const fn domain(&self) -> &ActorDomain {
        self.0.domain()
    }

    #[must_use]
    pub const fn flags(&self) -> ActorFlags {
        self.0.flags()
    }

    #[must_use]
    pub const fn authored_generation(&self) -> u64 {
        self.0.authored_generation()
    }

    #[must_use]
    pub fn into_actor_decl(self) -> ActorDecl {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NonContainerActorDeclError {
    PipelineActorRequiresScope,
    ReplicatorRequiresScope,
}

impl fmt::Display for NonContainerActorDeclError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReplicatorRequiresScope => {
                formatter.write_str("replicator must be built with a child scope")
            }
            Self::PipelineActorRequiresScope => {
                formatter.write_str("pipeline_actor must be built with a child scope")
            }
        }
    }
}

impl std::error::Error for NonContainerActorDeclError {}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PipelineActorDecl {
    flags: ActorFlags,
    authored_generation: u64,
}

impl PipelineActorDecl {
    #[must_use]
    pub const fn new(flags: ActorFlags) -> Self {
        Self::new_at_generation(flags, 0)
    }

    #[must_use]
    pub const fn new_at_generation(flags: ActorFlags, authored_generation: u64) -> Self {
        Self {
            flags,
            authored_generation,
        }
    }

    #[must_use]
    pub const fn flags(self) -> ActorFlags {
        self.flags
    }

    #[must_use]
    pub const fn authored_generation(self) -> u64 {
        self.authored_generation
    }

    #[must_use]
    pub fn into_actor_decl(self) -> ActorDecl {
        ActorDecl::new_at_generation(
            ActorDomain::new(ActorType::PipelineActor, Config::default()),
            self.flags,
            self.authored_generation,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContainerActorDecl(ActorDecl);

impl ContainerActorDecl {
    #[must_use]
    pub fn pipeline_actor(flags: ActorFlags, generation: u64) -> Self {
        PipelineActorDecl::new_at_generation(flags, generation).into()
    }

    /// A pipeline carrying the retained template reference and topic interface.
    #[must_use]
    pub fn pipeline_template(config: Config, flags: ActorFlags, generation: u64) -> Self {
        Self(ActorDecl::new_at_generation(
            ActorDomain::new(ActorType::PipelineActor, config),
            flags,
            generation,
        ))
    }

    #[must_use]
    pub fn replicator(config: Config, flags: ActorFlags, generation: u64) -> Self {
        Self(ActorDecl::new_at_generation(
            ActorDomain::new(ActorType::Replicator, config),
            flags,
            generation,
        ))
    }

    #[must_use]
    pub fn into_actor_decl(self) -> ActorDecl {
        self.0
    }
}

impl From<PipelineActorDecl> for ContainerActorDecl {
    fn from(declaration: PipelineActorDecl) -> Self {
        Self(declaration.into_actor_decl())
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Endpoint {
    actor: NamedActorId,
    port: PortId,
}

impl Endpoint {
    #[must_use]
    pub const fn new(actor: NamedActorId, port: PortId) -> Self {
        Self { actor, port }
    }

    #[must_use]
    pub const fn actor(&self) -> &NamedActorId {
        &self.actor
    }

    #[must_use]
    pub const fn port(&self) -> &PortId {
        &self.port
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum EdgeId {
    Declared {
        from: Endpoint,
        to: Endpoint,
        ordinal: u16,
    },
    Outcome {
        target: NamedActorId,
    },
}

impl EdgeId {
    #[must_use]
    pub fn declared(from: Endpoint, to: Endpoint, ordinal: u16) -> Self {
        Self::Declared { from, to, ordinal }
    }

    #[must_use]
    pub fn outcome(target: NamedActorId) -> Self {
        Self::Outcome { target }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EdgeDecl {
    from: Endpoint,
    to: Endpoint,
    ordinal: u16,
    attrs: EdgeAttrs,
}

impl EdgeDecl {
    #[must_use]
    pub const fn new(from: Endpoint, to: Endpoint, ordinal: u16, attrs: EdgeAttrs) -> Self {
        Self {
            from,
            to,
            ordinal,
            attrs,
        }
    }

    #[must_use]
    pub const fn from(&self) -> &Endpoint {
        &self.from
    }

    #[must_use]
    pub const fn to(&self) -> &Endpoint {
        &self.to
    }

    #[must_use]
    pub const fn ordinal(&self) -> u16 {
        self.ordinal
    }

    #[must_use]
    pub fn attrs(&self) -> EdgeAttrs {
        self.attrs.clone()
    }

    #[must_use]
    pub fn id(&self) -> EdgeId {
        EdgeId::declared(self.from.clone(), self.to.clone(), self.ordinal)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnnotationPlacement(Box<[u8]>);

impl AnnotationPlacement {
    #[must_use]
    pub fn unplaced() -> Self {
        Self(Box::new([]))
    }

    #[must_use]
    pub fn is_unplaced(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct OperationDecl(Config);

impl OperationDecl {
    #[must_use]
    pub const fn new(value: Config) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn config(&self) -> &Config {
        &self.0
    }
}

#[cfg(test)]
mod declared_delay_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn reduced_rational_seconds_convert_exactly_or_fail_closed() {
        let delay = DeclaredDelay::try_new(3, 5).expect("three fifths is reduced");
        assert_eq!(delay.numerator(), 3);
        assert_eq!(delay.denominator(), 5);
        assert_eq!(delay.try_duration().unwrap(), Duration::from_millis(600));
        assert_eq!(
            DeclaredDelay::try_new(2, 4),
            Err(DeclaredDelayError::NotReduced { divisor: 2 })
        );
        assert_eq!(
            DeclaredDelay::try_new(1, 0),
            Err(DeclaredDelayError::ZeroDenominator)
        );
        assert_eq!(
            DeclaredDelay::try_new(1, 3)
                .expect("one third is reduced")
                .try_duration(),
            Err(DeclaredDelayDurationError::BelowNanosecondResolution)
        );
    }
}

#[cfg(test)]
mod board_placement_tests {
    use super::*;

    #[test]
    fn zero_extent_is_rejected() {
        assert_eq!(
            BoardPlacement::try_new(3, 4, 0, 2),
            Err(BoardPlacementError::ZeroExtent { w: 0, h: 2 })
        );
    }

    #[test]
    fn a_shared_edge_is_not_an_overlap_but_a_shared_cell_is() {
        let cell = |col, row, w, h| BoardPlacement::try_new(col, row, w, h).expect("valid cell");
        assert!(
            !cell(0, 0, 2, 1).overlaps(cell(2, 0, 2, 1)),
            "the columns touch"
        );
        assert!(
            !cell(0, 0, 2, 1).overlaps(cell(0, 1, 2, 1)),
            "the rows touch"
        );
        assert!(
            cell(0, 0, 2, 2).overlaps(cell(1, 1, 2, 2)),
            "shares one cell"
        );
        assert!(
            !cell(0, 0, 2, 1).overlaps(cell(1, 5, 2, 1)),
            "only the columns overlap"
        );
        assert!(
            cell(0, 0, 4, 4).overlaps(cell(1, 1, 1, 1)),
            "contains it fully"
        );
        assert_eq!(
            cell(0, 0, 2, 2).overlaps(cell(1, 1, 2, 2)),
            cell(1, 1, 2, 2).overlaps(cell(0, 0, 2, 2))
        );
    }

    #[test]
    fn overflowing_extent_is_rejected() {
        assert_eq!(
            BoardPlacement::try_new(u32::MAX, 4, 1, 2),
            Err(BoardPlacementError::ExtentOverflow {
                start: u32::MAX,
                span: 1,
            })
        );
    }
}

#[cfg(test)]
mod group_name_tests {
    use super::*;

    #[test]
    fn authored_group_names_are_nfc_normalized_before_storage() {
        let composed = GroupName::from_authored_syntax("café".as_bytes()).expect("composed name");
        let decomposed =
            GroupName::from_authored_syntax("cafe\u{0301}".as_bytes()).expect("decomposed name");

        assert_eq!(composed, decomposed);
        assert_eq!(decomposed.as_str(), "café");

        let pre_normalization_66_bytes = "e\u{0301}".repeat(22);
        let normalized = GroupName::from_authored_syntax(pre_normalization_66_bytes.as_bytes())
            .expect("length is measured after NFC normalization");
        assert_eq!(normalized.as_str(), "é".repeat(22));
        assert!(
            GroupName::from_authored_syntax("a".repeat(64).as_bytes()).is_ok(),
            "64 bytes is inclusive",
        );
    }

    #[test]
    fn authored_group_name_rejections_preserve_the_violated_rule() {
        assert_eq!(
            GroupName::from_authored_syntax(b""),
            Err(GroupNameError::Empty)
        );
        assert_eq!(
            GroupName::from_authored_syntax("a".repeat(65).as_bytes()),
            Err(GroupNameError::TooLong { bytes: 65 })
        );
        assert_eq!(
            GroupName::from_authored_syntax("row\u{001f}one".as_bytes()),
            Err(GroupNameError::ControlCharacter)
        );
        assert_eq!(
            GroupName::from_authored_syntax("row\u{0085}one".as_bytes()),
            Err(GroupNameError::ControlCharacter)
        );
        assert_eq!(
            GroupName::from_authored_syntax(" leading".as_bytes()),
            Err(GroupNameError::SurroundingWhitespace)
        );
        assert_eq!(
            GroupName::from_authored_syntax("trailing ".as_bytes()),
            Err(GroupNameError::SurroundingWhitespace)
        );
    }
}

#[cfg(test)]
mod preprocess_tests {
    use super::*;
    use crate::Name;
    use circular_core::Ticks;
    use std::collections::HashSet;

    #[test]
    fn constructors_are_empty_and_with_preprocess_preserves_delay_and_policy() {
        let delay = DeclaredDelay::try_new(3, 5).unwrap();
        let policy = WirePolicy::new(Delivery::Lossless, PositiveCapacity::new(4).ok());
        let empty = EdgeAttrs::new_rational(delay, policy);
        assert!(empty.preprocess().steps().is_empty());
        assert!(
            EdgeAttrs::new(Ticks::ZERO, policy)
                .preprocess()
                .steps()
                .is_empty()
        );
        let chain = PreprocessChain::new(vec![
            PreprocessStep::new(PreprocessKind::Map, Config::default()),
            PreprocessStep::new(PreprocessKind::Bang, Config::default()),
        ]);
        let attrs = empty.clone().with_preprocess(chain.clone());
        assert_eq!(attrs.delay(), delay);
        assert_eq!(attrs.policy(), policy);
        assert_eq!(attrs.preprocess(), &chain);
        assert_ne!(attrs, empty);
        assert_eq!(attrs.with_preprocess(PreprocessChain::default()), empty);
    }

    #[test]
    fn equality_and_hash_include_step_kind_config_and_order() {
        let empty = EdgeAttrs::new(Ticks::ZERO, WirePolicy::new(Delivery::Lossless, None));
        let bang = PreprocessStep::new(PreprocessKind::Bang, Config::default());
        let map = PreprocessStep::new(PreprocessKind::Map, Config::default());
        let configured_map = PreprocessStep::new(
            PreprocessKind::Map,
            Config::try_new(vec![(
                Name::from_normalized("transform"),
                ConfigValue::Scalar {
                    tag: Name::from_normalized("value.v1"),
                    bytes: vec![1].into_boxed_slice(),
                },
            )])
            .unwrap(),
        );
        let variants = [
            vec![],
            vec![bang.clone()],
            vec![map.clone()],
            vec![configured_map],
            vec![bang.clone(), map.clone()],
            vec![map, bang],
        ]
        .map(|steps| empty.clone().with_preprocess(PreprocessChain::new(steps)));
        for (index, attrs) in variants.iter().enumerate() {
            assert_eq!(*attrs, attrs.clone());
            for other in &variants[index + 1..] {
                assert_ne!(attrs, other);
            }
        }
        assert_eq!(variants.into_iter().collect::<HashSet<_>>().len(), 6);
    }
}
