
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;

pub use circular_core::{BudgetExhausted, EvaluationBudget, ObjectValue, Value, ValueKind};
pub use circular_expr::{ConfigPath, Segment};
pub use circular_runtime::{Capability, EffectCtor, EffectFailure, OutcomePayload};

mod sealed {
    pub trait Sealed {}
}

#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    Always,
    ConfigPresent(ConfigPath),
    ConfigEquals { at: ConfigPath, value: Value },
}

#[derive(Clone, Debug, PartialEq)]
pub struct RequireRule {
    cap: Capability,
    when: Condition,
}

impl RequireRule {
    #[must_use]
    pub const fn new(cap: Capability, when: Condition) -> Self {
        Self { cap, when }
    }

    #[must_use]
    pub const fn capability(&self) -> Capability {
        self.cap
    }

    #[must_use]
    pub const fn condition(&self) -> &Condition {
        &self.when
    }
}

pub trait EffectDecl: sealed::Sealed + Sized {
    #[doc(hidden)]
    fn into_declaration(self) -> EffectDeclaration;
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NoExternalEffect;

impl sealed::Sealed for NoExternalEffect {}

impl EffectDecl for NoExternalEffect {
    fn into_declaration(self) -> EffectDeclaration {
        EffectDeclaration::None
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RequireRules<E: EffectDecl> {
    rules: Box<[RequireRule]>,
    effect: PhantomData<fn() -> E>,
}

impl<E: EffectDecl> RequireRules<E> {
    #[must_use]
    pub fn as_slice(&self) -> &[RequireRule] {
        &self.rules
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = &RequireRule> {
        self.rules.iter()
    }

    #[must_use]
    pub fn required_kinds(&self) -> BTreeSet<Capability> {
        self.rules.iter().map(RequireRule::capability).collect()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.rules.len()
    }

    pub(crate) fn into_boxed(self) -> Box<[RequireRule]> {
        self.rules
    }
}

impl RequireRules<NoExternalEffect> {
    #[must_use]
    pub fn none() -> Self {
        Self {
            rules: Box::new([]),
            effect: PhantomData,
        }
    }
}

impl RequireRules<ExternalEffect> {
    #[must_use]
    pub fn external(first: RequireRule, rest: impl IntoIterator<Item = RequireRule>) -> Self {
        let rules = std::iter::once(first)
            .chain(rest)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Self {
            rules,
            effect: PhantomData,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Durability {
    Durable,
    Direct,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StandIn {
    Succeeded(OutcomePayload),
    Failed(EffectFailure),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StandIns(BTreeMap<EffectCtor, StandIn>);

impl StandIns {
    #[must_use]
    pub fn one(effect: EffectCtor, stand_in: StandIn) -> Self {
        let mut entries = BTreeMap::new();
        entries.insert(effect, stand_in);
        Self(entries)
    }

    pub fn try_new(
        first: (EffectCtor, StandIn),
        rest: impl IntoIterator<Item = (EffectCtor, StandIn)>,
    ) -> Result<Self, DuplicateEffectCtor> {
        let (effect, stand_in) = first;
        let mut entries = BTreeMap::new();
        entries.insert(effect, stand_in);

        for (effect, stand_in) in rest {
            if entries.insert(effect, stand_in).is_some() {
                return Err(DuplicateEffectCtor { effect });
            }
        }

        Ok(Self(entries))
    }

    #[must_use]
    pub fn get(&self, effect: EffectCtor) -> Option<&StandIn> {
        self.0.get(&effect)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (EffectCtor, &StandIn)> {
        self.0.iter().map(|(effect, stand_in)| (*effect, stand_in))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DuplicateEffectCtor {
    effect: EffectCtor,
}

impl DuplicateEffectCtor {
    #[must_use]
    pub const fn effect(self) -> EffectCtor {
        self.effect
    }
}

impl fmt::Display for DuplicateEffectCtor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "effect constructor {:?} has more than one stand-in",
            self.effect
        )
    }
}

impl Error for DuplicateEffectCtor {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalEffect {
    durability: Durability,
    stand_ins: StandIns,
}

impl ExternalEffect {
    #[must_use]
    pub const fn new(durability: Durability, stand_ins: StandIns) -> Self {
        Self {
            durability,
            stand_ins,
        }
    }

    #[must_use]
    pub const fn durability(&self) -> Durability {
        self.durability
    }

    #[must_use]
    pub const fn stand_ins(&self) -> &StandIns {
        &self.stand_ins
    }
}

impl sealed::Sealed for ExternalEffect {}

impl EffectDecl for ExternalEffect {
    fn into_declaration(self) -> EffectDeclaration {
        EffectDeclaration::External {
            durability: self.durability,
            stand_ins: self.stand_ins,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EffectDeclaration {
    None,
    External {
        durability: Durability,
        stand_ins: StandIns,
    },
}

impl EffectDeclaration {
    #[must_use]
    pub const fn durability(&self) -> Option<Durability> {
        match self {
            Self::None => None,
            Self::External { durability, .. } => Some(*durability),
        }
    }

    #[must_use]
    pub const fn stand_ins(&self) -> Option<&StandIns> {
        match self {
            Self::None => None,
            Self::External { stand_ins, .. } => Some(stand_ins),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(capability: Capability) -> RequireRule {
        RequireRule::new(capability, Condition::Always)
    }

    #[test]
    fn no_external_effect_is_zero_sized_and_has_only_an_empty_rule_constructor() {
        assert_eq!(std::mem::size_of::<NoExternalEffect>(), 0);

        let rules = RequireRules::<NoExternalEffect>::none();
        assert!(rules.is_empty());
        assert_eq!(rules.required_kinds(), BTreeSet::new());
    }

    #[test]
    fn external_rule_constructor_cannot_receive_an_empty_sequence() {
        let rules = RequireRules::<ExternalEffect>::external(
            rule(Capability::FsRead),
            [
                RequireRule::new(
                    Capability::FsWrite,
                    Condition::ConfigPresent(ConfigPath::root().join_key("output")),
                ),
                RequireRule::new(
                    Capability::FsRead,
                    Condition::ConfigEquals {
                        at: ConfigPath::root().join_key("mode"),
                        value: Value::string("strict"),
                    },
                ),
            ],
        );

        assert_eq!(rules.len(), 3);
        assert_eq!(
            rules.required_kinds(),
            BTreeSet::from([Capability::FsRead, Capability::FsWrite])
        );
    }

    #[test]
    fn stand_ins_are_nonempty_unique_and_canonically_ordered() {
        let stand_ins = StandIns::try_new(
            (
                EffectCtor::Spawn,
                StandIn::Failed(EffectFailure::TransportTerminal),
            ),
            [(
                EffectCtor::FileRead,
                StandIn::Succeeded(OutcomePayload::FileBytes(Box::new([]))),
            )],
        )
        .expect("distinct constructors");

        assert_eq!(stand_ins.len(), 2);
        assert_eq!(
            stand_ins
                .iter()
                .map(|(effect, _)| effect)
                .collect::<Vec<_>>(),
            [EffectCtor::FileRead, EffectCtor::Spawn]
        );

        let duplicate = StandIns::try_new(
            (
                EffectCtor::FileWrite,
                StandIn::Succeeded(OutcomePayload::WrittenLength(1)),
            ),
            [(
                EffectCtor::FileWrite,
                StandIn::Succeeded(OutcomePayload::WrittenLength(2)),
            )],
        )
        .expect_err("two bands of the same constructor must be refused");
        assert_eq!(duplicate.effect(), EffectCtor::FileWrite);
    }

    #[test]
    fn external_effect_keeps_both_mandatory_declarations() {
        let effect = ExternalEffect::new(
            Durability::Durable,
            StandIns::one(
                EffectCtor::FileWrite,
                StandIn::Succeeded(OutcomePayload::WrittenLength(0)),
            ),
        );

        assert_eq!(effect.durability(), Durability::Durable);
        assert!(effect.stand_ins().get(EffectCtor::FileWrite).is_some());
        assert_eq!(
            effect.clone().into_declaration(),
            EffectDeclaration::External {
                durability: effect.durability,
                stand_ins: effect.stand_ins,
            }
        );
    }
}
