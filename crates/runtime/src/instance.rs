
use crate::{AdmittedTemplate, InstanceKey, Name, ScopeId};
use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InstanceAuthority<S> {
    container: ScopeId,
    template: Name,
    seal: PhantomData<fn() -> S>,
}

impl<S> InstanceAuthority<S> {
    #[must_use]
    pub fn granted(template: &AdmittedTemplate) -> Self {
        Self {
            container: template.container().clone(),
            template: template.template().clone(),
            seal: PhantomData,
        }
    }

    #[must_use]
    pub const fn container(&self) -> &ScopeId {
        &self.container
    }

    #[must_use]
    pub const fn template(&self) -> &Name {
        &self.template
    }

    #[must_use]
    pub fn instance_set_scope(&self) -> ScopeId {
        let mut segments = self.container.segments().to_vec();
        segments.push(crate::ScopeSeg::Child(self.template.clone()));
        ScopeId::from_segments(segments).expect("the container already kept the depth ceiling")
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InstanceIntent {
    Instantiate { key: InstanceKey },
    Retire { key: InstanceKey },
}

impl InstanceIntent {
    #[must_use]
    pub const fn key(&self) -> &InstanceKey {
        match self {
            Self::Instantiate { key } | Self::Retire { key } => key,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum InstanceDisposition {
    Minted,
    AlreadyLive,
    Retired,
    NotLive,
    RejectedCapacity { max: u64 },
}

impl InstanceDisposition {
    #[must_use]
    pub const fn changed_the_set(self) -> bool {
        matches!(self, Self::Minted | Self::Retired)
    }
}

pub trait InstanceAuthorityBearer {
    type Seal;

    fn instance_authority(&self) -> Option<&InstanceAuthority<Self::Seal>>;
}

impl InstanceAuthorityBearer for () {
    type Seal = ();

    fn instance_authority(&self) -> Option<&InstanceAuthority<Self::Seal>> {
        None
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct InstanceMutationSpec {
    container: ScopeId,
    template: Name,
    intent: InstanceIntent,
}

impl InstanceMutationSpec {
    #[must_use]
    pub fn requested<S>(authority: &InstanceAuthority<S>, intent: InstanceIntent) -> Self {
        Self {
            container: authority.container().clone(),
            template: authority.template().clone(),
            intent,
        }
    }

    #[must_use]
    pub const fn container(&self) -> &ScopeId {
        &self.container
    }

    #[must_use]
    pub const fn template(&self) -> &Name {
        &self.template
    }

    #[must_use]
    pub const fn intent(&self) -> &InstanceIntent {
        &self.intent
    }

    #[must_use]
    pub fn matches<S>(&self, authority: &InstanceAuthority<S>) -> bool {
        self.container == *authority.container() && self.template == *authority.template()
    }
}

pub trait InstanceGovernor<S> {
    fn apply(
        &mut self,
        authority: &InstanceAuthority<S>,
        intent: InstanceIntent,
    ) -> InstanceDisposition;

    fn live(&self, authority: &InstanceAuthority<S>) -> impl Iterator<Item = &InstanceKey>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstanceRegistry<S> {
    live: BTreeMap<(ScopeId, Name), BTreeSet<InstanceKey>>,
    capacity: BTreeMap<(ScopeId, Name), u64>,
    seal: PhantomData<fn() -> S>,
}

impl<S> Default for InstanceRegistry<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> InstanceRegistry<S> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            live: BTreeMap::new(),
            capacity: BTreeMap::new(),
            seal: PhantomData,
        }
    }

    pub fn set_capacity(&mut self, authority: &InstanceAuthority<S>, max: u64) {
        self.capacity.insert(Self::coordinate(authority), max);
    }

    #[must_use]
    pub fn count(&self, authority: &InstanceAuthority<S>) -> usize {
        self.live
            .get(&Self::coordinate(authority))
            .map_or(0, BTreeSet::len)
    }

    fn coordinate(authority: &InstanceAuthority<S>) -> (ScopeId, Name) {
        (authority.container().clone(), authority.template().clone())
    }
}

impl<S> InstanceGovernor<S> for InstanceRegistry<S> {
    fn apply(
        &mut self,
        authority: &InstanceAuthority<S>,
        intent: InstanceIntent,
    ) -> InstanceDisposition {
        let coordinate = Self::coordinate(authority);
        match intent {
            InstanceIntent::Instantiate { key } => {
                let live = self.live.entry(coordinate.clone()).or_default();
                if live.contains(&key) {
                    return InstanceDisposition::AlreadyLive;
                }
                if let Some(&max) = self.capacity.get(&coordinate)
                    && u64::try_from(live.len()).is_ok_and(|count| count >= max)
                {
                    return InstanceDisposition::RejectedCapacity { max };
                }
                live.insert(key);
                InstanceDisposition::Minted
            }
            InstanceIntent::Retire { key } => {
                if self
                    .live
                    .get_mut(&coordinate)
                    .is_some_and(|live| live.remove(&key))
                {
                    InstanceDisposition::Retired
                } else {
                    InstanceDisposition::NotLive
                }
            }
        }
    }

    fn live(&self, authority: &InstanceAuthority<S>) -> impl Iterator<Item = &InstanceKey> {
        self.live
            .get(&Self::coordinate(authority))
            .into_iter()
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_plan::{InstanceScalar, ScopeRole, ScopeSeg, admit_template};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TestSeal {}
    fn fleet_roles() -> circular_plan::ScopeRoleTable {
        let mut table = circular_plan::ScopeRoleTable::new();
        let at = |segments: &[&str]| {
            circular_plan::ScopeId::from_segments(
                segments
                    .iter()
                    .map(|s| {
                        circular_plan::ScopeSeg::Child(circular_plan::Name::from_normalized(*s))
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap()
        };
        table.declare(at(&["fleet"]), circular_plan::ScopeRole::Concrete);
        table.declare(at(&["fleet", "cell"]), circular_plan::ScopeRole::Template);
        table.declare(
            at(&["fleet", "cell", "inner"]),
            circular_plan::ScopeRole::Template,
        );
        table.declare(at(&["fleet", "shard"]), circular_plan::ScopeRole::Template);
        table
    }

    fn name(value: &str) -> Name {
        Name::from_normalized(value)
    }

    fn key(value: &str) -> InstanceKey {
        InstanceKey::Scalar(InstanceScalar::normalized_text(value))
    }

    fn fleet_scope() -> ScopeId {
        ScopeId::from_segments(vec![ScopeSeg::Child(name("fleet"))]).unwrap()
    }

    fn authority(plan: &circular_plan::ScopeRoleTable) -> InstanceAuthority<TestSeal> {
        InstanceAuthority::granted(
            &admit_template(&fleet_roles(), &fleet_scope(), &name("cell")).unwrap(),
        )
    }

    #[test]
    fn an_authority_carries_the_admitted_coordinate_and_nothing_else() {
        let plan = fleet_roles();
        let authority = authority(&plan);
        assert_eq!(authority.container(), &fleet_scope());
        assert_eq!(authority.template(), &name("cell"));
        assert_eq!(authority.clone(), authority);
    }

    #[test]
    fn a_repeated_instantiate_is_idempotent() {
        let plan = fleet_roles();
        let authority = authority(&plan);
        let mut registry = InstanceRegistry::<TestSeal>::new();

        assert_eq!(
            registry.apply(&authority, InstanceIntent::Instantiate { key: key("s1") }),
            InstanceDisposition::Minted
        );
        assert_eq!(
            registry.apply(&authority, InstanceIntent::Instantiate { key: key("s1") }),
            InstanceDisposition::AlreadyLive
        );
        assert_eq!(registry.count(&authority), 1);
        assert!(!InstanceDisposition::AlreadyLive.changed_the_set());
    }

    #[test]
    fn retiring_an_absent_key_is_a_no_op_and_retiring_a_live_one_is_not() {
        let plan = fleet_roles();
        let authority = authority(&plan);
        let mut registry = InstanceRegistry::<TestSeal>::new();

        assert_eq!(
            registry.apply(&authority, InstanceIntent::Retire { key: key("s1") }),
            InstanceDisposition::NotLive
        );
        registry.apply(&authority, InstanceIntent::Instantiate { key: key("s1") });
        assert_eq!(
            registry.apply(&authority, InstanceIntent::Retire { key: key("s1") }),
            InstanceDisposition::Retired
        );
        assert_eq!(registry.count(&authority), 0);
        assert_eq!(
            registry.apply(&authority, InstanceIntent::Instantiate { key: key("s1") }),
            InstanceDisposition::Minted
        );
    }

    #[test]
    fn two_cells_of_one_template_keep_separate_sets() {
        let plan = {
            let mut table = circular_plan::ScopeRoleTable::new();
            let at = |segments: &[&str]| {
                ScopeId::from_segments(
                    segments
                        .iter()
                        .map(|s| ScopeSeg::Child(name(s)))
                        .collect::<Vec<_>>(),
                )
                .unwrap()
            };
            table.declare(at(&["outer"]), circular_plan::ScopeRole::Template);
            table.declare(at(&["outer", "inner"]), circular_plan::ScopeRole::Template);
            table
        };

        let cell = |at: &str| {
            ScopeId::from_segments(vec![ScopeSeg::Instance {
                of: name("outer"),
                key: key(at),
            }])
            .unwrap()
        };
        let left = InstanceAuthority::<TestSeal>::granted(
            &admit_template(&plan, &cell("a"), &name("inner")).unwrap(),
        );
        let right = InstanceAuthority::<TestSeal>::granted(
            &admit_template(&plan, &cell("b"), &name("inner")).unwrap(),
        );
        assert_ne!(left, right);

        let mut registry = InstanceRegistry::<TestSeal>::new();
        registry.apply(&left, InstanceIntent::Instantiate { key: key("x") });
        assert_eq!(registry.count(&left), 1);
        assert_eq!(
            registry.count(&right),
            0,
            "the inner set of one cell does not pollute another cell's set"
        );
        assert_eq!(
            registry.apply(&right, InstanceIntent::Retire { key: key("x") }),
            InstanceDisposition::NotLive
        );
    }

    #[test]
    fn only_a_set_changing_disposition_is_a_structure_record() {
        assert!(InstanceDisposition::Minted.changed_the_set());
        assert!(InstanceDisposition::Retired.changed_the_set());
        assert!(!InstanceDisposition::AlreadyLive.changed_the_set());
        assert!(!InstanceDisposition::NotLive.changed_the_set());
        assert!(!InstanceDisposition::RejectedCapacity { max: 0 }.changed_the_set());
    }

    #[test]
    fn a_mutation_request_reads_its_coordinate_from_the_authority() {
        let plan = fleet_roles();
        let authority = authority(&plan);
        let spec = InstanceMutationSpec::requested(
            &authority,
            InstanceIntent::Instantiate { key: key("s1") },
        );

        assert_eq!(spec.container(), authority.container());
        assert_eq!(spec.template(), authority.template());
        assert_eq!(spec.intent().key(), &key("s1"));
        assert!(spec.matches(&authority));
    }

    #[test]
    fn a_request_carries_no_authority() {
        let plan = fleet_roles();
        let authority = authority(&plan);
        let spec = InstanceMutationSpec::requested(
            &authority,
            InstanceIntent::Instantiate { key: key("s1") },
        );
        drop(authority);
        assert_eq!(spec.template(), &name("cell"));
    }

    #[test]
    fn an_empty_bundle_says_it_carries_none() {
        fn bearer_authority<G: InstanceAuthorityBearer>(
            grants: &G,
        ) -> Option<&InstanceAuthority<G::Seal>> {
            grants.instance_authority()
        }
        assert!(bearer_authority(&()).is_none());
    }
}
