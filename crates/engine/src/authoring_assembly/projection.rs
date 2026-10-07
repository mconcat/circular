
use circular_plan::{
    ActorDecl, Anchor, AnnotationPlacement, ContainerActorDecl, EdgeAttrs, EdgeDecl, Endpoint,
    Name, NamedActorId, NonContainerActorDecl, PipelineActorDecl, PortId, Presentation, ScopeId,
    ScopeRole, ScopeRoleTable, ScopeSeg, Text,
};
use circular_plan::{
    Annotation, AnnotationId, DeclaredEdgeId, DeclaredScopeSeg, Export, ExportName, Mount, Role,
    ScopeBoundary, ScopeDeclaration,
};
use circular_protocol::declaration_payload::PresentationOwner;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

const MAX_SCOPE_DEPTH: usize = circular_plan::MAX_SCOPE_DEPTH;

#[derive(Default)]
pub struct GraphLayer {
    pub(crate) declaration: ScopeDeclaration,
    pub(crate) actors: BTreeMap<NamedActorId, ActorDecl>,
    pub(crate) edges: BTreeMap<DeclaredEdgeId, EdgeDecl>,
    pub(crate) scopes: BTreeMap<DeclaredScopeSeg, AuthoredProjection>,
}

impl GraphLayer {
    #[must_use]
    pub const fn declaration(&self) -> &ScopeDeclaration {
        &self.declaration
    }

    #[must_use]
    pub const fn actors(&self) -> &BTreeMap<NamedActorId, ActorDecl> {
        &self.actors
    }

    #[must_use]
    pub const fn edges(&self) -> &BTreeMap<DeclaredEdgeId, EdgeDecl> {
        &self.edges
    }
}

pub struct AuthoredProjection {
    pub(crate) templates: BTreeMap<Name, circular_plan::Template>,
    pub(crate) graph: GraphLayer,
    pub(crate) exports: BTreeMap<ExportName, Export>,
    pub(crate) annotations: BTreeMap<AnnotationId, Annotation>,
    pub(crate) presentation: BTreeMap<PresentationOwner<NamedActorId, AnnotationId>, Presentation>,
    pub(crate) refused: BTreeMap<NamedActorId, RefusedDeclaration>,
}

#[derive(Clone, Debug)]
pub struct RefusedDeclaration {
    pub(crate) key: circular_protocol::declaration_payload::PlanActorKey,
    pub(crate) declaration: circular_protocol::declaration_payload::ActorDeclaration,
    pub(crate) generation: circular_protocol::boundary_port::BoundaryActorGeneration,
    pub(crate) rejection: super::rejection::FoldRejection,
}

impl RefusedDeclaration {
    #[must_use]
    pub fn key(&self) -> &circular_protocol::declaration_payload::PlanActorKey {
        &self.key
    }

    #[must_use]
    pub fn rejection(&self) -> &super::rejection::FoldRejection {
        &self.rejection
    }

    #[must_use]
    pub(crate) fn failure(&self) -> crate::activation_detail::RegistrationFailure {
        crate::activation_detail::RegistrationFailure::new(
            crate::activation_detail::activation::CONFIG_ADMISSION,
            self.rejection.to_string(),
        )
    }
}

impl AuthoredProjection {
    pub fn templates(&self) -> &BTreeMap<Name, circular_plan::Template> {
        &self.templates
    }

    #[must_use]
    pub fn refused(&self) -> &BTreeMap<NamedActorId, RefusedDeclaration> {
        &self.refused
    }

    #[must_use]
    pub fn empty() -> Self {
        AuthoredProjectionBuilder::new()
            .finish()
            .expect("an empty plan is valid")
    }

    #[must_use]
    pub const fn graph(&self) -> &GraphLayer {
        &self.graph
    }

    #[must_use]
    pub const fn declaration(&self) -> &ScopeDeclaration {
        self.graph.declaration()
    }

    #[must_use]
    pub const fn role(&self) -> ScopeRole {
        self.graph.declaration.role()
    }

    #[must_use]
    pub const fn boundary(&self) -> &ScopeBoundary {
        self.graph.declaration.boundary()
    }

    #[must_use]
    pub const fn exports(&self) -> &BTreeMap<ExportName, Export> {
        &self.exports
    }

    #[must_use]
    pub const fn annotations(&self) -> &BTreeMap<AnnotationId, Annotation> {
        &self.annotations
    }

    #[must_use]
    pub const fn presentation(
        &self,
    ) -> &BTreeMap<PresentationOwner<NamedActorId, AnnotationId>, Presentation> {
        &self.presentation
    }

    #[must_use]
    pub fn all_actors(&self) -> Vec<circular_plan::ActorId> {
        fold_projection(self, |layer| {
            let mut actors = layer
                .actors()
                .keys()
                .cloned()
                .map(circular_plan::ActorId::from)
                .collect::<Vec<_>>();
            for mut child in layer.into_scopes().into_values() {
                actors.append(&mut child);
            }
            actors
        })
    }

    fn fold_clone(&self, include_non_graph: bool) -> Self {
        let mut cloned = fold_projection(self, |layer| {
            let actors = layer.actors().clone();
            let edges = layer.edges().clone();
            let exports = if include_non_graph {
                layer.exports().clone()
            } else {
                BTreeMap::new()
            };
            let annotations = if include_non_graph {
                layer.annotations().clone()
            } else {
                BTreeMap::new()
            };
            let presentation = if include_non_graph {
                layer.presentation().clone()
            } else {
                BTreeMap::new()
            };
            let templates = layer.templates().clone();
            AuthoredProjection {
                templates,
                graph: GraphLayer {
                    declaration: layer.graph().declaration().clone(),
                    actors,
                    edges,
                    scopes: layer.into_scopes(),
                },
                exports,
                annotations,
                presentation,
                refused: BTreeMap::new(),
            }
        });
        cloned.refused = self.refused.clone();
        cloned
    }

    fn boundary_port_count(&self) -> usize {
        let boundary = self.graph.declaration.boundary();
        boundary.inlets().len() + boundary.outlets().len()
    }
}

impl Clone for AuthoredProjection {
    fn clone(&self) -> Self {
        self.fold_clone(true)
    }
}

impl PartialEq for AuthoredProjection {
    fn eq(&self, other: &Self) -> bool {
        crate::authoring_assembly::projection_diff::diff(self, other).is_empty()
    }
}

impl Eq for AuthoredProjection {}

impl fmt::Debug for AuthoredProjection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthoredProjection")
            .field("root_role", &self.graph.declaration.role())
            .field("root_boundary_ports", &self.boundary_port_count())
            .field("root_actors", &self.graph.actors.len())
            .field("root_edges", &self.graph.edges.len())
            .field("root_scopes", &self.graph.scopes.len())
            .field("root_exports", &self.exports.len())
            .field("root_annotations", &self.annotations.len())
            .field("root_presentation", &self.presentation.len())
            .finish()
    }
}

impl Drop for AuthoredProjection {
    fn drop(&mut self) {
        let mut pending = std::mem::take(&mut self.graph.scopes)
            .into_values()
            .collect::<Vec<_>>();
        while let Some(mut child) = pending.pop() {
            pending.extend(std::mem::take(&mut child.graph.scopes).into_values());
        }
    }
}

struct BuilderFrame {
    scope: ScopeId,
    graph: GraphLayer,
    exports: BTreeMap<ExportName, Export>,
    annotations: BTreeMap<AnnotationId, Annotation>,
    presentation: BTreeMap<PresentationOwner<NamedActorId, AnnotationId>, Presentation>,
    entered_as: Option<DeclaredScopeSeg>,
}

impl BuilderFrame {
    fn root() -> Self {
        Self::at_scope(ScopeId::root())
    }

    fn at_scope(scope: ScopeId) -> Self {
        Self {
            scope,
            graph: GraphLayer {
                declaration: ScopeDeclaration::concrete(),
                ..GraphLayer::default()
            },
            exports: BTreeMap::new(),
            annotations: BTreeMap::new(),
            presentation: BTreeMap::new(),
            entered_as: None,
        }
    }

    fn into_plan(mut self) -> AuthoredProjection {
        AuthoredProjection {
            templates: BTreeMap::new(),
            graph: std::mem::take(&mut self.graph),
            exports: std::mem::take(&mut self.exports),
            annotations: std::mem::take(&mut self.annotations),
            presentation: std::mem::take(&mut self.presentation),
            refused: BTreeMap::new(),
        }
    }
}

pub struct AuthoredProjectionBuilder {
    frames: Vec<BuilderFrame>,
    templates: BTreeMap<Name, circular_plan::Template>,
    refused: BTreeMap<NamedActorId, RefusedDeclaration>,
}

impl Default for AuthoredProjectionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthoredProjectionBuilder {
    pub fn set_template(&mut self, template: circular_plan::Template) {
        self.templates.insert(template.name().clone(), template);
    }
    #[must_use]
    pub fn new() -> Self {
        Self {
            frames: vec![BuilderFrame::root()],
            templates: BTreeMap::new(),
            refused: BTreeMap::new(),
        }
    }

    pub(crate) fn refuse(&mut self, actor: NamedActorId, refused: RefusedDeclaration) {
        self.refused.insert(actor, refused);
    }

    fn current(&self) -> &BuilderFrame {
        self.frames
            .last()
            .expect("AuthoredProjectionBuilder always has a root")
    }

    fn current_mut(&mut self) -> &mut BuilderFrame {
        self.frames
            .last_mut()
            .expect("AuthoredProjectionBuilder always has a root")
    }

    pub fn add_actor(
        &mut self,
        name: Name,
        declaration: NonContainerActorDecl,
    ) -> Result<NamedActorId, BuildError> {
        self.insert_actor(name, declaration.into_actor_decl())
    }

    fn insert_actor(
        &mut self,
        name: Name,
        declaration: ActorDecl,
    ) -> Result<NamedActorId, BuildError> {
        let id = NamedActorId::new(self.current().scope.clone(), name);
        if self.current().graph.actors.contains_key(&id) {
            return Err(BuildError::DuplicateActor(id));
        }
        self.current_mut()
            .graph
            .actors
            .insert(id.clone(), declaration);
        Ok(id)
    }

    pub fn enter_scope(
        &mut self,
        name: Name,
        container: PipelineActorDecl,
    ) -> Result<NamedActorId, BuildError> {
        self.enter_scope_with_container(name, container.into())
    }

    pub fn enter_scope_with_container(
        &mut self,
        name: Name,
        container: ContainerActorDecl,
    ) -> Result<NamedActorId, BuildError> {
        let container = container.into_actor_decl();
        let role = if *container.domain().actor_type() == circular_plan::ActorType::Replicator {
            ScopeRole::Template
        } else {
            ScopeRole::Concrete
        };
        let attempted = self.current().scope.depth() + 1;
        if attempted > MAX_SCOPE_DEPTH {
            return Err(BuildError::ScopeDepthExceeded {
                attempted,
                limit: MAX_SCOPE_DEPTH,
            });
        }
        let segment = DeclaredScopeSeg::child(name.clone());
        if self.current().graph.scopes.contains_key(&segment) {
            return Err(BuildError::DuplicateScope(segment));
        }
        let container_id = self.insert_actor(name, container)?;
        self.enter_cloned_scope(segment)?;
        self.current_mut().graph.declaration = ScopeDeclaration::new(role, ScopeBoundary::sealed());
        Ok(container_id)
    }

    fn enter_cloned_scope(&mut self, segment: DeclaredScopeSeg) -> Result<(), BuildError> {
        let attempted = self.current().scope.depth() + 1;
        if attempted > MAX_SCOPE_DEPTH {
            return Err(BuildError::ScopeDepthExceeded {
                attempted,
                limit: MAX_SCOPE_DEPTH,
            });
        }
        let scope = self
            .current()
            .scope
            .append_segment(segment.as_scope_seg())
            .map_err(|error| BuildError::ScopeDepthExceeded {
                attempted: error.attempted(),
                limit: error.limit(),
            })?;
        self.frames.push(BuilderFrame {
            scope,
            graph: GraphLayer {
                declaration: ScopeDeclaration::concrete(),
                ..GraphLayer::default()
            },
            exports: BTreeMap::new(),
            annotations: BTreeMap::new(),
            presentation: BTreeMap::new(),
            entered_as: Some(segment),
        });
        Ok(())
    }

    pub fn exit_scope(&mut self) -> Result<(), BuildError> {
        if self.frames.len() == 1 {
            return Err(BuildError::CannotExitRoot);
        }
        let child = self.frames.pop().expect("length checked");
        let segment = child
            .entered_as
            .clone()
            .expect("a child frame has an entry segment");
        if self
            .current_mut()
            .graph
            .scopes
            .insert(segment.clone(), child.into_plan())
            .is_some()
        {
            return Err(BuildError::DuplicateScope(segment));
        }
        Ok(())
    }

    pub fn add_edge(
        &mut self,
        from: Endpoint,
        to: Endpoint,
        ordinal: u16,
        attrs: EdgeAttrs,
    ) -> Result<DeclaredEdgeId, BuildError> {
        let declaration = EdgeDecl::new(from, to, ordinal, attrs);
        let id = <DeclaredEdgeId as From<&EdgeDecl>>::from(&declaration);
        if self.current().graph.edges.contains_key(&id) {
            return Err(BuildError::DuplicateEdge(id));
        }
        self.current_mut()
            .graph
            .edges
            .insert(id.clone(), declaration);
        Ok(id)
    }

    pub fn set_boundary(&mut self, boundary: ScopeBoundary) -> Result<(), BuildError> {
        if self.frames.len() == 1 {
            return Err(BuildError::RootScopeBoundary);
        }
        let declaration = std::mem::take(&mut self.current_mut().graph.declaration);
        self.current_mut().graph.declaration = declaration.with_boundary(boundary);
        Ok(())
    }

    pub fn set_export(&mut self, name: ExportName, value: Export) {
        self.current_mut().exports.insert(name, value);
    }

    pub fn set_annotation(&mut self, id: AnnotationId, value: Annotation) {
        self.current_mut().annotations.insert(id, value);
    }

    pub fn set_presentation(
        &mut self,
        actor: PresentationOwner<NamedActorId, AnnotationId>,
        value: Presentation,
    ) {
        self.current_mut().presentation.insert(actor, value);
    }

    pub fn finish(mut self) -> Result<AuthoredProjection, BuildError> {
        if self.frames.len() != 1 {
            return Err(BuildError::UnclosedScopes(self.frames.len() - 1));
        }
        let root_frame = self.frames.pop().expect("root frame");
        let root_scope = root_frame.scope.clone();
        let mut root = root_frame.into_plan();
        root.templates = self.templates;
        root.refused = self.refused;
        validate_plan(&root, root_scope)?;
        Ok(root)
    }
}

fn validate_plan(root: &AuthoredProjection, root_scope: ScopeId) -> Result<(), BuildError> {
    if root.graph.declaration != ScopeDeclaration::concrete() {
        return Err(BuildError::RootScopeBoundary);
    }
    let mut all_actors = BTreeSet::new();
    let mut layers = Vec::new();
    let mut pending = vec![(root_scope, root)];

    while let Some((scope, plan)) = pending.pop() {
        for actor in plan.graph.actors.keys() {
            if actor.scope() != &scope {
                return Err(BuildError::ActorInWrongScope(actor.clone()));
            }
            all_actors.insert(actor.clone());
        }
        for (segment, child) in plan.graph.scopes.iter().rev() {
            let child_scope = scope
                .append_segment(segment.as_scope_seg())
                .expect("sealed construction already checked the depth");
            pending.push((child_scope, child));
        }
        layers.push((scope, plan));
    }

    for (scope, plan) in layers {
        for endpoint in plan.graph.declaration.boundary().inner_endpoints() {
            if endpoint.actor().scope() != &scope
                || !plan.graph.actors.contains_key(endpoint.actor())
            {
                return Err(BuildError::UnknownBoundaryActor(endpoint.actor().clone()));
            }
        }
        for (id, edge) in &plan.graph.edges {
            let derived = <DeclaredEdgeId as From<&EdgeDecl>>::from(edge);
            if id != &derived {
                return Err(BuildError::EdgeKeyMismatch(id.clone()));
            }
            for endpoint in [edge.from(), edge.to()] {
                if endpoint.actor().scope() != &scope {
                    return Err(BuildError::EdgeCrossesScope(id.clone()));
                }
                if !plan.graph.actors.contains_key(endpoint.actor()) {
                    return Err(BuildError::UnknownEdgeActor(endpoint.actor().clone()));
                }
            }
        }
        for export in plan.exports.values() {
            for mount in export.roles().values() {
                validate_downward_reference(&scope, &all_actors, mount.actor())?;
            }
        }
        for annotation in plan.annotations.values() {
            for actor in annotation.refs() {
                validate_downward_reference(&scope, &all_actors, actor)?;
            }
        }
        for (actor, presentation) in &plan.presentation {
            let exists = match actor {
                PresentationOwner::Actor(key) => {
                    key.scope() == &scope && plan.graph.actors.contains_key(key)
                }
                PresentationOwner::Annotation(key) => plan.annotations.contains_key(key),
            };
            if !exists {
                return Err(BuildError::UnknownPresentationOwner(actor.clone()));
            }
            let target = match presentation.anchor() {
                Some(Anchor::Relative { target, .. } | Anchor::Align { target, .. }) => {
                    Some(target)
                }
                Some(Anchor::Flow) | None => None,
            };
            if let Some(target) = target
                && (target.scope() != &scope || !plan.graph.actors.contains_key(target))
            {
                return Err(BuildError::UnknownPresentationOwner(
                    PresentationOwner::Actor(target.clone()),
                ));
            }
        }
    }
    Ok(())
}

fn validate_downward_reference(
    owner: &ScopeId,
    all_actors: &BTreeSet<NamedActorId>,
    target: &NamedActorId,
) -> Result<(), BuildError> {
    if !owner.is_ancestor_of(target.scope()) || !all_actors.contains(target) {
        Err(BuildError::InvalidDownwardReference(target.clone()))
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BuildError {
    ScopeDepthExceeded { attempted: usize, limit: usize },
    DuplicateActor(NamedActorId),
    DuplicateEdge(DeclaredEdgeId),
    DuplicateScope(DeclaredScopeSeg),
    CannotExitRoot,
    UnclosedScopes(usize),
    ActorInWrongScope(NamedActorId),
    EdgeKeyMismatch(DeclaredEdgeId),
    EdgeCrossesScope(DeclaredEdgeId),
    UnknownEdgeActor(NamedActorId),
    InvalidDownwardReference(NamedActorId),
    UnknownPresentationOwner(PresentationOwner<NamedActorId, AnnotationId>),
    UnknownBoundaryActor(NamedActorId),
    RootScopeBoundary,
}

impl BuildError {
    #[must_use]
    pub const fn diagnostic_code(&self) -> Option<&'static str> {
        match self {
            Self::ScopeDepthExceeded { .. } => Some("CIR-E0500"),
            _ => None,
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ScopeDepthExceeded { attempted, limit } => {
                write!(formatter, "scope depth {attempted} exceeds limit {limit}")
            }
            Self::DuplicateActor(_) => formatter.write_str("duplicate actor key within a scope"),
            Self::DuplicateEdge(_) => formatter.write_str("duplicate authored edge topology key"),
            Self::DuplicateScope(_) => {
                formatter.write_str("duplicate child scope key within a parent")
            }
            Self::CannotExitRoot => formatter.write_str("cannot leave the root scope"),
            Self::UnclosedScopes(count) => write!(formatter, "{count} scopes remain unclosed"),
            Self::ActorInWrongScope(_) => {
                formatter.write_str("actor key scope differs from the declaration location")
            }
            Self::EdgeKeyMismatch(_) => {
                formatter.write_str("edge key was not derived from the value's topology")
            }
            Self::EdgeCrossesScope(_) => formatter.write_str("edge crosses a scope boundary"),
            Self::UnknownEdgeActor(_) => {
                formatter.write_str("edge endpoint actor is missing from the same scope")
            }
            Self::InvalidDownwardReference(_) => {
                formatter.write_str("unresolved export or annotation reference")
            }
            Self::UnknownPresentationOwner(_) => {
                formatter.write_str("unresolved presentation owner or anchor reference")
            }
            Self::UnknownBoundaryActor(_) => {
                formatter.write_str("scope boundary inner endpoint is missing from the same scope")
            }
            Self::RootScopeBoundary => {
                formatter.write_str("root scope cannot have a role or boundary")
            }
        }
    }
}

impl std::error::Error for BuildError {}

pub struct ProjectionFoldLayer<'a, A> {
    templates: &'a BTreeMap<Name, circular_plan::Template>,
    scope: ScopeId,
    graph: GraphFoldLayer<'a, A>,
    exports: &'a BTreeMap<ExportName, Export>,
    annotations: &'a BTreeMap<AnnotationId, Annotation>,
    presentation: &'a BTreeMap<PresentationOwner<NamedActorId, AnnotationId>, Presentation>,
}

pub struct GraphFoldLayer<'a, A> {
    declaration: &'a ScopeDeclaration,
    actors: &'a BTreeMap<NamedActorId, ActorDecl>,
    edges: &'a BTreeMap<DeclaredEdgeId, EdgeDecl>,
    scopes: BTreeMap<DeclaredScopeSeg, A>,
}

impl<'a, A> ProjectionFoldLayer<'a, A> {
    pub fn templates(&self) -> &'a BTreeMap<Name, circular_plan::Template> {
        self.templates
    }
    #[must_use]
    pub const fn scope(&self) -> &ScopeId {
        &self.scope
    }

    #[must_use]
    pub const fn graph(&self) -> &GraphFoldLayer<'a, A> {
        &self.graph
    }

    #[must_use]
    pub const fn actors(&self) -> &'a BTreeMap<NamedActorId, ActorDecl> {
        self.graph.actors
    }

    #[must_use]
    pub const fn edges(&self) -> &'a BTreeMap<DeclaredEdgeId, EdgeDecl> {
        self.graph.edges
    }

    #[must_use]
    pub const fn scopes(&self) -> &BTreeMap<DeclaredScopeSeg, A> {
        &self.graph.scopes
    }

    #[must_use]
    pub fn into_scopes(self) -> BTreeMap<DeclaredScopeSeg, A> {
        self.graph.scopes
    }

    #[must_use]
    pub const fn exports(&self) -> &'a BTreeMap<ExportName, Export> {
        self.exports
    }

    #[must_use]
    pub const fn annotations(&self) -> &'a BTreeMap<AnnotationId, Annotation> {
        self.annotations
    }

    #[must_use]
    pub const fn presentation(
        &self,
    ) -> &'a BTreeMap<PresentationOwner<NamedActorId, AnnotationId>, Presentation> {
        self.presentation
    }
}

impl<'a, A> GraphFoldLayer<'a, A> {
    #[must_use]
    pub const fn declaration(&self) -> &'a ScopeDeclaration {
        self.declaration
    }

    #[must_use]
    pub const fn actors(&self) -> &'a BTreeMap<NamedActorId, ActorDecl> {
        self.actors
    }

    #[must_use]
    pub const fn edges(&self) -> &'a BTreeMap<DeclaredEdgeId, EdgeDecl> {
        self.edges
    }

    #[must_use]
    pub const fn scopes(&self) -> &BTreeMap<DeclaredScopeSeg, A> {
        &self.scopes
    }
}

#[must_use]
pub fn scope_roles(root: &AuthoredProjection) -> circular_plan::ScopeRoleTable {
    fold_projection::<circular_plan::ScopeRoleTable>(root, |layer| {
        let role = layer.graph().declaration().role();
        let scope = layer.scope().clone();
        let mut table = circular_plan::ScopeRoleTable::new();
        for child in layer.into_scopes().into_values() {
            table.absorb(&child);
        }
        table.declare(scope, role);
        table
    })
}

pub fn fold_projection<A>(
    root: &AuthoredProjection,
    mut algebra: impl FnMut(ProjectionFoldLayer<'_, A>) -> A,
) -> A {
    enum Task<'a> {
        Visit(&'a AuthoredProjection, ScopeId),
        Fold(&'a AuthoredProjection, ScopeId),
    }

    let mut tasks = vec![Task::Visit(root, ScopeId::root())];
    let mut results = Vec::new();
    while let Some(task) = tasks.pop() {
        match task {
            Task::Visit(plan, scope) => {
                tasks.push(Task::Fold(plan, scope.clone()));
                for (segment, child) in plan.graph.scopes.iter().rev() {
                    let child_scope = scope
                        .append_segment(segment.as_scope_seg())
                        .expect("a sealed plan keeps the depth ceiling");
                    tasks.push(Task::Visit(child, child_scope));
                }
            }
            Task::Fold(plan, scope) => {
                let child_count = plan.graph.scopes.len();
                let child_results = results.split_off(results.len() - child_count);
                let scopes = plan
                    .graph
                    .scopes
                    .keys()
                    .cloned()
                    .zip(child_results)
                    .collect();
                results.push(algebra(ProjectionFoldLayer {
                    templates: &plan.templates,
                    scope,
                    graph: GraphFoldLayer {
                        declaration: &plan.graph.declaration,
                        actors: &plan.graph.actors,
                        edges: &plan.graph.edges,
                        scopes,
                    },
                    exports: &plan.exports,
                    annotations: &plan.annotations,
                    presentation: &plan.presentation,
                }));
            }
        }
    }
    results.pop().expect("fold result of the root")
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_plan::{ActorDomain, ActorFlags, ActorType, Config, NonContainerActorDecl};

    fn name(value: &str) -> Name {
        Name::from_normalized(value)
    }

    fn actor(actor_type: ActorType) -> NonContainerActorDecl {
        NonContainerActorDecl::try_new(
            ActorDomain::new(actor_type, Config::default()),
            ActorFlags::default(),
        )
        .expect("test actor type is not a container")
    }

    #[test]
    fn depth_is_checked_before_entering_child() {
        let mut builder = AuthoredProjectionBuilder::new();
        for index in 0..MAX_SCOPE_DEPTH {
            builder
                .enter_scope(name(&format!("s{index}")), PipelineActorDecl::default())
                .unwrap();
        }
        let error = builder
            .enter_scope(name("too_deep"), PipelineActorDecl::default())
            .unwrap_err();
        assert_eq!(
            error,
            BuildError::ScopeDepthExceeded {
                attempted: 65,
                limit: 64
            }
        );
        assert_eq!(error.diagnostic_code(), Some("CIR-E0500"));
    }

    fn port(value: &str) -> PortId {
        PortId::try_new(value).expect("test port name")
    }

    fn replicator() -> ContainerActorDecl {
        ContainerActorDecl::replicator(Config::default(), ActorFlags::default(), 0)
    }

    #[test]
    fn the_root_scope_has_no_boundary() {
        let mut builder = AuthoredProjectionBuilder::new();
        assert_eq!(
            builder.set_boundary(ScopeBoundary::sealed()).unwrap_err(),
            BuildError::RootScopeBoundary
        );
    }

    #[test]
    fn a_boundary_endpoint_resolves_in_its_own_scope() {
        let mut builder = AuthoredProjectionBuilder::new();
        builder
            .enter_scope(name("child"), PipelineActorDecl::default())
            .unwrap();
        let inside = builder
            .add_actor(name("inside"), actor(ActorType::FixtureMap))
            .unwrap();
        builder
            .set_boundary(ScopeBoundary::new(
                [(port("event"), Endpoint::new(inside.clone(), port("in")))],
                [(Endpoint::new(inside.clone(), port("out")), port("done"))],
            ))
            .unwrap();
        builder.exit_scope().unwrap();
        let plan = builder.finish().unwrap();

        let child = plan
            .graph
            .scopes
            .get(&DeclaredScopeSeg::child(name("child")))
            .unwrap();
        assert_eq!(child.boundary().inlets().len(), 1);
        assert_eq!(child.boundary().outlets().len(), 1);
        assert!(!child.boundary().is_sealed());
        assert_eq!(
            child
                .boundary()
                .inlets()
                .get(&port("event"))
                .unwrap()
                .actor(),
            &inside
        );
    }

    #[test]
    fn a_boundary_endpoint_outside_the_scope_is_not_built() {
        let mut builder = AuthoredProjectionBuilder::new();
        let outside = builder
            .add_actor(name("outside"), actor(ActorType::FixtureMap))
            .unwrap();
        builder
            .enter_scope(name("child"), PipelineActorDecl::default())
            .unwrap();
        builder
            .set_boundary(ScopeBoundary::new(
                [(port("event"), Endpoint::new(outside.clone(), port("in")))],
                [],
            ))
            .unwrap();
        builder.exit_scope().unwrap();

        assert_eq!(
            builder.finish().unwrap_err(),
            BuildError::UnknownBoundaryActor(outside)
        );
    }

    #[test]
    fn a_boundary_endpoint_naming_an_absent_actor_is_not_built() {
        let mut builder = AuthoredProjectionBuilder::new();
        builder
            .enter_scope(name("child"), PipelineActorDecl::default())
            .unwrap();
        let ghost = NamedActorId::new(
            ScopeId::from_segments(vec![ScopeSeg::Child(name("child"))]).unwrap(),
            name("ghost"),
        );
        builder
            .set_boundary(ScopeBoundary::new(
                [],
                [(Endpoint::new(ghost.clone(), port("out")), port("done"))],
            ))
            .unwrap();
        builder.exit_scope().unwrap();

        assert_eq!(
            builder.finish().unwrap_err(),
            BuildError::UnknownBoundaryActor(ghost)
        );
    }

    #[test]
    fn fold_visits_each_layer_once_without_recursion() {
        let mut builder = AuthoredProjectionBuilder::new();
        builder
            .add_actor(name("root"), actor(ActorType::FixtureMap))
            .unwrap();
        builder
            .enter_scope(name("child"), PipelineActorDecl::default())
            .unwrap();
        builder
            .add_actor(name("inside"), actor(ActorType::FixtureFilter))
            .unwrap();
        builder.exit_scope().unwrap();
        let plan = builder.finish().unwrap();
        let calls = std::cell::Cell::new(0);
        let count = fold_projection(&plan, |layer| {
            calls.set(calls.get() + 1);
            layer.actors().len() + layer.into_scopes().into_values().sum::<usize>()
        });
        assert_eq!(calls.get(), 2);
        assert_eq!(count, 3);
        assert!(plan.all_actors().windows(2).all(|pair| pair[0] < pair[1]));
    }
}

