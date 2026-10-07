
use crate::{
    AnnotationPlacement, EdgeDecl, EdgeId, Endpoint, Name, NamedActorId, PortId, ScopeSeg, Text,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ExportName(Name);

impl ExportName {
    #[must_use]
    pub const fn new(name: Name) -> Self {
        Self(name)
    }

    #[must_use]
    pub const fn name(&self) -> &Name {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AnnotationId(Name);

impl AnnotationId {
    #[must_use]
    pub const fn new(name: Name) -> Self {
        Self(name)
    }

    #[must_use]
    pub const fn name(&self) -> &Name {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Role {
    Request,
    Progress,
    Result,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Mount {
    actor: NamedActorId,
    port: PortId,
}

impl Mount {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundaryPortRef {
    actor: NamedActorId,
    port: PortId,
}

impl BoundaryPortRef {
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

    #[must_use]
    pub fn from_mount(mount: &Mount) -> Self {
        Self::new(mount.actor().clone(), mount.port().clone())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Export {
    roles: BTreeMap<Role, Mount>,
    operations: crate::OperationDecl,
    surface: Option<crate::ConfigValue>,
}

impl Export {
    #[must_use]
    pub const fn new(roles: BTreeMap<Role, Mount>, operations: crate::OperationDecl) -> Self {
        Self {
            roles,
            operations,
            surface: None,
        }
    }

    /// Preserve the canonical builder tree through the existing structural Value carrier.
    #[must_use]
    pub fn with_surface(mut self, surface: Option<crate::ConfigValue>) -> Self {
        self.surface = surface;
        self
    }

    #[must_use]
    pub const fn surface(&self) -> Option<&crate::ConfigValue> {
        self.surface.as_ref()
    }

    #[must_use]
    pub const fn roles(&self) -> &BTreeMap<Role, Mount> {
        &self.roles
    }

    #[must_use]
    pub const fn operations(&self) -> &crate::OperationDecl {
        &self.operations
    }

    #[must_use]
    pub fn request_boundary_ref(&self) -> Option<BoundaryPortRef> {
        self.roles
            .get(&Role::Request)
            .map(BoundaryPortRef::from_mount)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AnnotationKind {
    Note,
    Backdrop,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Annotation {
    kind: AnnotationKind,
    refs: BTreeSet<NamedActorId>,
    placement: AnnotationPlacement,
    body: Text,
}

impl Annotation {
    #[must_use]
    pub const fn new(
        kind: AnnotationKind,
        refs: BTreeSet<NamedActorId>,
        placement: AnnotationPlacement,
        body: Text,
    ) -> Self {
        Self {
            kind,
            refs,
            placement,
            body,
        }
    }

    #[must_use]
    pub const fn kind(&self) -> AnnotationKind {
        self.kind
    }

    #[must_use]
    pub const fn refs(&self) -> &BTreeSet<NamedActorId> {
        &self.refs
    }

    #[must_use]
    pub const fn placement(&self) -> &AnnotationPlacement {
        &self.placement
    }

    #[must_use]
    pub const fn body(&self) -> &Text {
        &self.body
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeclaredScopeSeg(Name);

impl DeclaredScopeSeg {
    #[must_use]
    pub const fn child(name: Name) -> Self {
        Self(name)
    }

    #[must_use]
    pub const fn name(&self) -> &Name {
        &self.0
    }

    #[must_use]
    pub fn as_scope_seg(&self) -> ScopeSeg {
        ScopeSeg::Child(self.0.clone())
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeclaredEdgeId {
    from: Endpoint,
    to: Endpoint,
    ordinal: u16,
}

impl DeclaredEdgeId {
    #[must_use]
    pub const fn derive(from: Endpoint, to: Endpoint, ordinal: u16) -> Self {
        Self { from, to, ordinal }
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
    pub fn as_edge_id(&self) -> EdgeId {
        EdgeId::declared(self.from.clone(), self.to.clone(), self.ordinal)
    }
}

impl From<&EdgeDecl> for DeclaredEdgeId {
    fn from(value: &EdgeDecl) -> Self {
        Self::derive(value.from().clone(), value.to().clone(), value.ordinal())
    }
}
