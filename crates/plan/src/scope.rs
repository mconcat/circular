
use crate::{Endpoint, PortId};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ScopeRole {
    #[default]
    Concrete,
    Template,
}

impl ScopeRole {
    #[must_use]
    pub const fn is_concrete(self) -> bool {
        matches!(self, Self::Concrete)
    }

    #[must_use]
    pub const fn is_template(self) -> bool {
        matches!(self, Self::Template)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScopeRoleTable(BTreeMap<crate::ScopeId, ScopeRole>);

impl ScopeRoleTable {
    #[must_use]
    pub fn new() -> Self {
        Self(BTreeMap::from([(
            crate::ScopeId::root(),
            ScopeRole::Concrete,
        )]))
    }

    pub fn declare(&mut self, declared: crate::ScopeId, role: ScopeRole) {
        self.0.insert(declared, role);
    }

    pub fn absorb(&mut self, other: &Self) {
        for (scope, role) in &other.0 {
            self.0.insert(scope.clone(), *role);
        }
    }

    #[must_use]
    pub fn role_at(&self, declared: &crate::ScopeId) -> Option<ScopeRole> {
        self.0.get(declared).copied()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScopeBoundary {
    inlets: BTreeMap<PortId, Endpoint>,
    outlets: BTreeMap<PortId, Endpoint>,
}

impl ScopeBoundary {
    #[must_use]
    pub fn sealed() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn new(
        inlets: impl IntoIterator<Item = (PortId, Endpoint)>,
        outlets: impl IntoIterator<Item = (Endpoint, PortId)>,
    ) -> Self {
        Self {
            inlets: inlets.into_iter().collect(),
            outlets: outlets
                .into_iter()
                .map(|(inner, outer)| (outer, inner))
                .collect(),
        }
    }

    #[must_use]
    pub const fn inlets(&self) -> &BTreeMap<PortId, Endpoint> {
        &self.inlets
    }

    #[must_use]
    pub const fn outlets(&self) -> &BTreeMap<PortId, Endpoint> {
        &self.outlets
    }

    #[must_use]
    pub fn is_sealed(&self) -> bool {
        self.inlets.is_empty() && self.outlets.is_empty()
    }

    pub fn inner_endpoints(&self) -> impl Iterator<Item = &Endpoint> {
        self.inlets.values().chain(self.outlets.values())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScopeDeclaration {
    role: ScopeRole,
    boundary: ScopeBoundary,
}

impl ScopeDeclaration {
    #[must_use]
    pub const fn new(role: ScopeRole, boundary: ScopeBoundary) -> Self {
        Self { role, boundary }
    }

    #[must_use]
    pub fn concrete() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn template() -> Self {
        Self::new(ScopeRole::Template, ScopeBoundary::sealed())
    }

    #[must_use]
    pub const fn role(&self) -> ScopeRole {
        self.role
    }

    #[must_use]
    pub const fn boundary(&self) -> &ScopeBoundary {
        &self.boundary
    }

    #[must_use]
    pub fn with_boundary(self, boundary: ScopeBoundary) -> Self {
        Self {
            role: self.role,
            boundary,
        }
    }
}
