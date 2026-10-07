
use circular_protocol::declaration_payload::PresentationOwner;
use std::collections::{BTreeMap, BTreeSet};

use crate::authoring_assembly::projection::AuthoredProjection;
use crate::authoring_assembly::projection::AuthoredProjectionBuilder;
use crate::authoring_assembly::projection::RefusedDeclaration;
use crate::authoring_assembly::projection::fold_projection;
use crate::authoring_assembly::rejection::{
    AddressUse, BoundaryFault, BuildStep, ConfigAt, ConfigFault, FoldRejection, MoveToScopeFault,
    PortAt, TargetWrite, TemplateFault,
};
use crate::authoring_assembly::tables::{self as t, DeclaredTables, Delta};
use crate::authoring_assembly::verb::ContentVerb;
use crate::resolve_declared_ports_at;
use circular_core::{CANONICAL_VALUE_TAG, Ceilings, Value, encode};
use circular_plan::{
    ActorDecl, ActorDomain, ActorFlags, ActorType, Anchor, Annotation, AnnotationId,
    AnnotationKind, AnnotationPlacement, Config, ConfigValue, ContainerActorDecl, DeclaredDelay,
    DeclaredEdgeId, Delivery, EdgeAttrs, EdgeDecl, Endpoint, Export, ExportName, Mount, Name,
    NamedActorId, NonContainerActorDecl, OperationDecl, PortId, PositiveCapacity, PreprocessKind,
    Presentation, Role, ScopeBoundary, ScopeId, ScopeSeg, Shed, Text, WirePolicy,
};
use circular_protocol::declaration_payload::{
    ActorDeclaration, ActorLocal, Anchor as WireAnchor, AnnotationDeclaration, AuthoredLocal,
    AuthoringEnvironment, DeclaredEdgeKey, EdgeDeclaration, ExportDeclaration, PlanActorKey,
    PlanAnnotationKey, PlanExportKey, Presentation as WirePresentation, ScopeBinding,
    ScopeDeclaration, ScopeSegment,
};

#[derive(Clone, Debug)]
pub struct EpochCandidate {
    pub(crate) target: Vec<ScopeSegment>,
    pub(crate) target_role: Option<circular_protocol::declaration_payload::ScopeRole>,
    pub(super) tables: DeclaredTables,
    pub(crate) replacement_environment: Option<AuthoringEnvironment>,
    pub opening_environment: Option<AuthoringEnvironment>,
    pub issued_epoch: Option<Vec<u8>>,
    /// Client-chosen identity of the whole mutation request.  It is distinct
    /// from `issued_epoch`: the former survives retries, while the latter only
    /// names this live bracket.
    pub commit_id: Option<Vec<u8>>,
    /// Baseline supplied by `BeginEpoch`.  CAS is deliberately deferred until
    /// commit, after the durable dedup lookup.
    pub(crate) expected_revision: Option<circular_protocol::declaration_payload::ExpectedRevision>,
    /// Canonical request parts used by the durable CommitId digest.  Each part
    /// is `(stable declaration verb tag, canonical payload bytes)` and retains
    /// accepted content order.
    pub request_parts: Vec<(u8, Vec<u8>)>,
    /// Accepted semantic content in the shared DeclarationCommand union.  The
    /// item repeats `kind` because the retained journal has no envelope verb;
    /// all declaration addresses are canonical absolute arms.
    pub accepted_commands: Vec<Value>,
    /// Seeded current state is not an accepted command.  The environment barrier's
    /// "otherwise empty" precondition is therefore measured separately.
    pub(crate) accepted_content_commands: usize,
    pub(crate) authored_scopes: Vec<Vec<ScopeSegment>>,
    pub(crate) retires_own_scope: bool,
}

/// Registry-resolved ports for one committed authored declaration.
///
/// This fact is separate from graph edges: a declared port exists even when no
/// edge currently names it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthoringActorPortFact {
    pub actor: PlanActorKey,
    pub in_ports: Vec<AuthoringPortFact>,
    pub out_ports: Vec<AuthoringPortFact>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthoringPortFact {
    pub id: String,
    pub label: Option<String>,
    pub flow: AuthoringPortFlowFact,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthoringPortFlowFact {
    Known(circular_actors::Flow),
}

enum MoveScopeRelation {
    Fold { container: PlanActorKey },
    Unfold { container: PlanActorKey },
}

struct FoldBoundary {
    key: PlanActorKey,
    port: String,
}

struct CollapsedFold {
    edge: EdgeDeclaration,
    relay: usize,
}

struct IndexedEdge {
    index: usize,
    edge: EdgeDeclaration,
}

impl EpochCandidate {
    pub fn open(target: Vec<ScopeSegment>) -> Result<Self, FoldRejection> {
        if target
            .iter()
            .any(|segment| matches!(segment, ScopeSegment::Instance { .. }))
        {
            return Err(FoldRejection::TargetIsInstance);
        }
        Ok(Self {
            target,
            target_role: Some(circular_protocol::declaration_payload::ScopeRole::Concrete),
            tables: DeclaredTables::default(),
            replacement_environment: None,
            opening_environment: None,
            issued_epoch: None,
            commit_id: None,
            expected_revision: None,
            request_parts: Vec::new(),
            accepted_commands: Vec::new(),
            accepted_content_commands: 0,
            authored_scopes: Vec::new(),
            retires_own_scope: false,
        })
    }

    #[must_use]
    pub fn tables(&self) -> &DeclaredTables {
        &self.tables
    }

    pub fn apply(&mut self, verb: &ContentVerb) -> Result<Delta, FoldRejection> {
        self.tables.open_recording();
        let applied = self.apply_verb(verb);
        let mut delta = self.tables.close_recording();
        applied?;
        if let ContentVerb::ReplaceAuthoringEnvironment { replacement } = verb {
            delta.replace_environment(replacement.clone());
        }
        Ok(delta)
    }

    fn apply_verb(&mut self, verb: &ContentVerb) -> Result<(), FoldRejection> {
        use ContentVerb as V;
        match verb {
            V::UpsertActor { actor, declaration } => {
                self.add_actor(actor.clone(), declaration.clone());
            }
            V::RetireActor { actor } => self.retire_actor(actor)?,
            V::UpsertEdge { declaration } => self.add_edge(declaration.clone()),
            V::RetireEdge { edge } => self.retire_edge(edge),
            V::UpsertScope { scope, declaration } => {
                self.add_scope(scope.clone(), declaration.clone());
            }
            V::RetireScope { scope } => self.retire_scope(scope)?,
            V::MoveToScope { actors, target } => {
                self.move_to_scope(actors.clone(), target.clone())?;
            }
            V::UpsertExportMount { mount, declaration } => {
                self.add_export(mount.clone(), declaration.clone());
            }
            V::RetireExportMount { mount } => self.retire_export(mount),
            V::UpsertAnnotation {
                annotation,
                declaration,
            } => self.add_annotation(annotation.clone(), declaration.clone()),
            V::RetireAnnotation { annotation } => self.retire_annotation(annotation),
            V::SetPresentation {
                owner,
                presentation,
            } => self.add_presentation(owner.clone(), presentation.clone()),
            V::SetFlags { actor, flags } => self.set_flags(actor, *flags)?,
            V::UpsertTemplate { name, commands } => {
                self.upsert_template(name.clone(), commands.clone())?;
            }
            V::RetireTemplate { name } => self.retire_template(name)?,
            V::ReplaceAuthoringEnvironment { replacement } => {
                self.promote_environment(replacement.clone())?;
            }
        }
        Ok(())
    }

    pub fn upsert_template(
        &mut self,
        name: String,
        commands: Vec<Value>,
    ) -> Result<(), FoldRejection> {
        if !self.target.is_empty() {
            return Err(FoldRejection::TemplateOutsideRoot);
        }
        let template = circular_plan::Template::try_new(Name::from_normalized(name), commands)
            .map_err(FoldRejection::InvalidTemplate)?;
        self.tables
            .put::<t::Templates>(template.name().clone(), template);
        self.accepted_content_commands += 1;
        mark_authored_scope(&mut self.authored_scopes, Vec::new());
        Ok(())
    }

    pub fn retire_template(&mut self, name: &str) -> Result<(), FoldRejection> {
        if !self.target.is_empty() {
            return Err(FoldRejection::TemplateOutsideRoot);
        }
        self.tables
            .remove::<t::Templates>(&Name::from_normalized(name));
        self.accepted_content_commands += 1;
        mark_authored_scope(&mut self.authored_scopes, Vec::new());
        Ok(())
    }

    pub fn add_actor(&mut self, key: PlanActorKey, declaration: ActorDeclaration) {
        mark_authored_scope(&mut self.authored_scopes, key.scope.clone());
        self.tables.put::<t::Actors>(key, declaration);
        self.accepted_content_commands += 1;
    }

    pub fn move_to_scope(
        &mut self,
        actors: Vec<PlanActorKey>,
        target: Vec<ScopeSegment>,
    ) -> Result<(), MoveToScopeFault> {
        let mut staged = self.clone();
        staged.move_to_scope_inner(actors, target)?;
        *self = staged;
        Ok(())
    }

    fn move_to_scope_inner(
        &mut self,
        actors: Vec<PlanActorKey>,
        target: Vec<ScopeSegment>,
    ) -> Result<(), MoveToScopeFault> {
        if actors.is_empty() {
            return Err(MoveToScopeFault::EmptyMoveSet);
        }

        let mut moved = Vec::with_capacity(actors.len());
        for key in actors {
            if moved.iter().any(|(existing, _)| existing == &key) {
                continue;
            }
            let Some(declaration) = self.tables.actors().get(&key) else {
                return Err(MoveToScopeFault::SourceMissing { source: key });
            };
            moved.push((key, declaration.clone()));
        }

        let target_role = if target == self.target {
            self.target_role
        } else {
            self.tables
                .scopes()
                .get(&target)
                .map(|declaration| declaration.role)
        };
        let Some(target_role) = target_role else {
            return Err(MoveToScopeFault::TargetUndeclared { target });
        };
        if target_role == circular_protocol::declaration_payload::ScopeRole::Template {
            return Err(MoveToScopeFault::TargetIsTemplate { target });
        }

        for (key, declaration) in &moved {
            if matches!(declaration.actor_type.as_str(), "input" | "output") {
                return Err(MoveToScopeFault::BoundaryActor {
                    local: key.local.clone(),
                    actor_type: declaration.actor_type.clone(),
                });
            }
        }

        for (key, declaration) in &moved {
            if !ActorType::from_str(&declaration.actor_type).is_some_and(ActorType::is_container) {
                continue;
            }
            let mut owned_scope = key.scope.clone();
            owned_scope.push(ScopeSegment::Child(key.local.as_str().to_owned()));
            let local = key.local.clone();
            return Err(if target.starts_with(&owned_scope) {
                MoveToScopeFault::ContainerIntoItself { local }
            } else {
                MoveToScopeFault::ContainerMoveUnsupported { local }
            });
        }

        let source_scope = moved[0].0.scope.clone();
        if moved.iter().any(|(key, _)| key.scope != source_scope) {
            return Err(MoveToScopeFault::SourcesSpanScopes);
        }

        let mappings = moved
            .iter()
            .map(|(old, _)| {
                (
                    old.clone(),
                    PlanActorKey {
                        scope: target.clone(),
                        local: old.local.clone(),
                    },
                )
            })
            .collect::<Vec<_>>();

        for (_, new) in &mappings {
            if self
                .tables
                .actors()
                .keys()
                .any(|existing| existing == new && !mappings.iter().any(|(old, _)| old == existing))
                || mappings
                    .iter()
                    .filter(|(_, candidate)| candidate == new)
                    .count()
                    > 1
            {
                return Err(MoveToScopeFault::LocalCollision {
                    target,
                    local: new.local.clone(),
                });
            }
        }

        if source_scope != target {
            self.rewrite_move_edges(&mappings, &source_scope, &target)?;
        }

        mark_authored_scope(&mut self.authored_scopes, source_scope.clone());
        mark_authored_scope(&mut self.authored_scopes, target.clone());

        let actors = moved_keys(&mappings, self.tables.actors().keys());
        for (old, new) in actors {
            let declaration = self.tables.actors().get(&old).cloned().expect("moved row");
            self.tables.rekey::<t::Actors>(&old, new, declaration);
        }
        let generations = moved_keys(&mappings, self.tables.generations().keys());
        for (old, new) in generations {
            let generation = self.tables.generation(&old);
            self.tables.rekey::<t::Generations>(&old, new, generation);
        }
        let presentations = moved_keys(
            &mappings,
            self.tables
                .presentations()
                .keys()
                .filter_map(|owner| match owner {
                    PresentationOwner::Actor(key) => Some(key),
                    PresentationOwner::Annotation(_) => None,
                }),
        );
        for (old, new) in presentations {
            let old = PresentationOwner::Actor(old);
            let new = PresentationOwner::Actor(new);
            let presentation = self
                .tables
                .presentations()
                .get(&old)
                .cloned()
                .expect("moved row");
            self.tables
                .rekey::<t::Presentations>(&old, new, presentation);
        }
        let exports = self
            .tables
            .exports()
            .iter()
            .filter_map(|(mount, export)| {
                let mut export = export.clone();
                let mut changed = false;
                for (actor, _) in [
                    &mut export.roles.request,
                    &mut export.roles.progress,
                    &mut export.roles.result,
                    &mut export.roles.error,
                ]
                .into_iter()
                .flatten()
                {
                    if let Some(new) = moved_key(&mappings, actor) {
                        *actor = new.clone();
                        changed = true;
                    }
                }
                changed.then(|| (mount.clone(), export))
            })
            .collect::<Vec<_>>();
        for (mount, export) in exports {
            self.tables.put::<t::Exports>(mount, export);
        }
        let annotations = self
            .tables
            .annotations()
            .iter()
            .filter_map(|(key, annotation)| {
                let mut annotation = annotation.clone();
                let mut changed = false;
                for actor in &mut annotation.refs {
                    if let Some(new) = moved_key(&mappings, actor) {
                        *actor = new.clone();
                        changed = true;
                    }
                }
                changed.then(|| (key.clone(), annotation))
            })
            .collect::<Vec<_>>();
        for (key, annotation) in annotations {
            self.tables.put::<t::Annotations>(key, annotation);
        }

        self.accepted_content_commands += 1;
        Ok(())
    }

    fn rewrite_move_edges(
        &mut self,
        mappings: &[(PlanActorKey, PlanActorKey)],
        source: &[ScopeSegment],
        target: &[ScopeSegment],
    ) -> Result<(), MoveToScopeFault> {
        use circular_protocol::boundary_port::BoundaryPortDirection;
        let forward_container = direct_child_container(target, source);
        let reverse_container = direct_child_container(source, target);
        let relation = match (forward_container, reverse_container) {
            (Some(container), _) => MoveScopeRelation::Fold { container },
            (_, Some(container)) => MoveScopeRelation::Unfold { container },
            _ => {
                return Err(MoveToScopeFault::MoreThanOneBoundary {
                    source: source.to_vec(),
                    target: target.to_vec(),
                });
            }
        };
        let container = match &relation {
            MoveScopeRelation::Fold { container } | MoveScopeRelation::Unfold { container } => {
                container
            }
        };
        if !self
            .actor_type(container)
            .and_then(ActorType::from_str)
            .is_some_and(ActorType::is_container)
        {
            return Err(MoveToScopeFault::ContainerInactive {
                local: container.local.clone(),
            });
        }

        let original = self.tables.edges().values().cloned().collect::<Vec<_>>();
        let mut rewritten = original.clone();
        let mut removed = vec![false; original.len()];
        let mut relays = Vec::new();

        for (index, edge) in original.iter().enumerate() {
            let from_moved = moved_key(mappings, &edge.from.0);
            let to_moved = moved_key(mappings, &edge.to.0);
            match (from_moved, to_moved) {
                (Some(from), Some(to)) => {
                    rewritten[index].from.0 = from.clone();
                    rewritten[index].to.0 = to.clone();
                }
                (None, None) => {}
                (from, to) => match &relation {
                    MoveScopeRelation::Fold { container } => {
                        if let Some(collapsed) = self
                            .collapse_fold_boundary(edge, from, to, container, target, &original)?
                        {
                            rewritten[index] = collapsed.edge;
                            removed[collapsed.relay] = true;
                            continue;
                        }

                        let (direction, inner, replace_from) = match (from, to) {
                            (Some(inner), None) => (
                                BoundaryPortDirection::Outlet,
                                (inner.clone(), edge.from.1.clone()),
                                true,
                            ),
                            (None, Some(inner)) => (
                                BoundaryPortDirection::Inlet,
                                (inner.clone(), edge.to.1.clone()),
                                false,
                            ),
                            _ => unreachable!("one endpoint crosses the move set"),
                        };
                        let boundary = self.ensure_fold_boundary(direction, &inner, target)?;
                        if replace_from {
                            rewritten[index].from = (container.clone(), boundary.port.clone());
                        } else {
                            rewritten[index].to = (container.clone(), boundary.port.clone());
                        }
                        push_relay_once(&original, &mut relays, direction, &boundary, &inner);
                    }
                    MoveScopeRelation::Unfold { container } => {
                        if let Some(collapsed) = self.collapse_unfold_boundary(
                            edge, from, to, container, source, &original,
                        )? {
                            removed[index] = true;
                            for outer in collapsed {
                                rewritten[outer.index] = outer.edge;
                            }
                            continue;
                        }

                        let (direction, inner, moved_out, replace_from) = match (from, to) {
                            (Some(moved_out), None) => (
                                BoundaryPortDirection::Inlet,
                                edge.to.clone(),
                                moved_out.clone(),
                                true,
                            ),
                            (None, Some(moved_out)) => (
                                BoundaryPortDirection::Outlet,
                                edge.from.clone(),
                                moved_out.clone(),
                                false,
                            ),
                            _ => unreachable!("one endpoint crosses the move set"),
                        };
                        let boundary = self.ensure_fold_boundary(direction, &inner, source)?;
                        if replace_from {
                            rewritten[index].from = (moved_out, edge.from.1.clone());
                            rewritten[index].to = (container.clone(), boundary.port.clone());
                        } else {
                            rewritten[index].from = (container.clone(), boundary.port.clone());
                            rewritten[index].to = (moved_out, edge.to.1.clone());
                        }
                        push_relay_once(&original, &mut relays, direction, &boundary, &inner);
                    }
                },
            }
        }

        for ((old, new), gone) in original.iter().zip(rewritten).zip(removed) {
            if gone {
                self.tables.remove::<t::Edges>(&edge_key(old));
            } else if *old != new {
                self.tables
                    .rekey::<t::Edges>(&edge_key(old), edge_key(&new), new);
            }
        }
        for relay in relays {
            self.tables.put::<t::Edges>(edge_key(&relay), relay);
        }
        Ok(())
    }

    fn ensure_fold_boundary(
        &mut self,
        direction: circular_protocol::boundary_port::BoundaryPortDirection,
        inner: &(PlanActorKey, String),
        scope: &[ScopeSegment],
    ) -> Result<FoldBoundary, MoveToScopeFault> {
        use circular_protocol::boundary_port::BoundaryPortId;
        let inner_port = PortId::try_new(inner.1.clone()).map_err(|_| {
            MoveToScopeFault::InnerPortNotCanonical {
                port: inner.1.clone(),
            }
        })?;
        let local = fold_boundary_local(direction, inner.0.local.as_str(), inner_port.as_str())
            .map_err(MoveToScopeFault::BoundaryLocal)?;
        let key = PlanActorKey {
            scope: scope.to_vec(),
            local: ActorLocal::Authored(local),
        };
        let actor_type = match direction {
            circular_protocol::boundary_port::BoundaryPortDirection::Inlet => "input",
            circular_protocol::boundary_port::BoundaryPortDirection::Outlet => "output",
        };
        let config = Value::object([(
            "label",
            Value::String(format!("{}.{}", inner.0.local, inner.1)),
        )])
        .expect("the synthesized boundary config has one key");
        let expected = ActorDeclaration {
            actor_type: actor_type.to_owned(),
            config,
            flags: circular_protocol::declaration_payload::ActorFlags {
                bypass: false,
                mute: false,
                pause: false,
            },
        };
        match self.tables.actors().get(&key) {
            Some(declaration) if declaration == &expected => {}
            Some(_) => {
                return Err(MoveToScopeFault::SynthesizedLocalConflicts {
                    local: key.local.clone(),
                });
            }
            None => {
                self.tables.put::<t::Actors>(key.clone(), expected);
            }
        }
        let port = BoundaryPortId::derive(direction, &key, self.actor_generation(&key))
            .map_err(MoveToScopeFault::SynthesizedPort)?
            .as_port_id()
            .as_str()
            .to_owned();
        Ok(FoldBoundary { key, port })
    }

    fn collapse_fold_boundary(
        &self,
        edge: &EdgeDeclaration,
        from: Option<&PlanActorKey>,
        to: Option<&PlanActorKey>,
        container: &PlanActorKey,
        target: &[ScopeSegment],
        edges: &[EdgeDeclaration],
    ) -> Result<Option<CollapsedFold>, MoveToScopeFault> {
        use circular_protocol::boundary_port::BoundaryPortDirection;
        let (direction, moved, port) = match (from, to) {
            (Some(moved), None) if edge.to.0 == *container => {
                (BoundaryPortDirection::Inlet, moved, &edge.to.1)
            }
            (None, Some(moved)) if edge.from.0 == *container => {
                (BoundaryPortDirection::Outlet, moved, &edge.from.1)
            }
            _ => return Ok(None),
        };
        let Some(boundary) = self.boundary_actor_for_port(direction, target, port) else {
            return Err(MoveToScopeFault::SynthesizedBoundaryMissing { port: port.clone() });
        };
        let relay = edges
            .iter()
            .enumerate()
            .find(|(_, candidate)| match direction {
                BoundaryPortDirection::Inlet => candidate.from == (boundary.clone(), port.clone()),
                BoundaryPortDirection::Outlet => candidate.to == (boundary.clone(), port.clone()),
            });
        let Some((relay_index, relay)) = relay else {
            return Err(MoveToScopeFault::NoInnerRelay { port: port.clone() });
        };
        let mut collapsed = edge.clone();
        match direction {
            BoundaryPortDirection::Inlet => {
                collapsed.from.0 = moved.clone();
                collapsed.to = relay.to.clone();
            }
            BoundaryPortDirection::Outlet => {
                collapsed.from = relay.from.clone();
                collapsed.to.0 = moved.clone();
            }
        }
        Ok(Some(CollapsedFold {
            edge: collapsed,
            relay: relay_index,
        }))
    }

    fn collapse_unfold_boundary(
        &self,
        edge: &EdgeDeclaration,
        from: Option<&PlanActorKey>,
        to: Option<&PlanActorKey>,
        container: &PlanActorKey,
        source: &[ScopeSegment],
        edges: &[EdgeDeclaration],
    ) -> Result<Option<Vec<IndexedEdge>>, MoveToScopeFault> {
        use circular_protocol::boundary_port::BoundaryPortDirection;
        let (direction, moved, boundary, inner_port) = match (from, to) {
            (None, Some(moved)) if self.actor_type(&edge.from.0) == Some("input") => (
                BoundaryPortDirection::Inlet,
                moved,
                &edge.from.0,
                &edge.to.1,
            ),
            (Some(moved), None) if self.actor_type(&edge.to.0) == Some("output") => (
                BoundaryPortDirection::Outlet,
                moved,
                &edge.to.0,
                &edge.from.1,
            ),
            _ => return Ok(None),
        };
        if boundary.scope != source {
            return Ok(None);
        }
        let port = PortId::try_new(inner_port.clone()).map_err(|_| {
            MoveToScopeFault::InnerPortNotCanonical {
                port: inner_port.clone(),
            }
        })?;
        let expected = fold_boundary_local(direction, moved.local.as_str(), port.as_str())
            .map_err(MoveToScopeFault::ReverseBoundaryLocal)?;
        if boundary.local.as_str() != expected.as_str() {
            return Ok(None);
        }
        let boundary_port = match direction {
            BoundaryPortDirection::Inlet => &edge.from.1,
            BoundaryPortDirection::Outlet => &edge.to.1,
        };
        let expected_port = circular_protocol::boundary_port::BoundaryPortId::derive(
            direction,
            boundary,
            self.actor_generation(boundary),
        )
        .map_err(MoveToScopeFault::ReversePort)?;
        if expected_port.as_port_id().as_str() != boundary_port {
            return Ok(None);
        }

        let mut outer = Vec::new();
        for (index, candidate) in edges.iter().enumerate() {
            let matches = match direction {
                BoundaryPortDirection::Inlet => {
                    candidate.to == (container.clone(), boundary_port.clone())
                }
                BoundaryPortDirection::Outlet => {
                    candidate.from == (container.clone(), boundary_port.clone())
                }
            };
            if !matches {
                continue;
            }
            let mut replacement = candidate.clone();
            match direction {
                BoundaryPortDirection::Inlet => {
                    replacement.to = (moved.clone(), inner_port.clone());
                }
                BoundaryPortDirection::Outlet => {
                    replacement.from = (moved.clone(), inner_port.clone());
                }
            }
            outer.push(IndexedEdge {
                index,
                edge: replacement,
            });
        }
        Ok(Some(outer))
    }

    fn boundary_actor_for_port(
        &self,
        direction: circular_protocol::boundary_port::BoundaryPortDirection,
        scope: &[ScopeSegment],
        port: &str,
    ) -> Option<PlanActorKey> {
        let actor_type = match direction {
            circular_protocol::boundary_port::BoundaryPortDirection::Inlet => "input",
            circular_protocol::boundary_port::BoundaryPortDirection::Outlet => "output",
        };
        self.tables
            .actors()
            .iter()
            .filter(|(key, declaration)| key.scope == scope && declaration.actor_type == actor_type)
            .find_map(|(key, _)| {
                circular_protocol::boundary_port::BoundaryPortId::derive(
                    direction,
                    key,
                    self.actor_generation(key),
                )
                .ok()
                .filter(|id| id.as_port_id().as_str() == port)
                .map(|_| key.clone())
            })
    }

    #[must_use]
    pub fn actor_generation(
        &self,
        key: &PlanActorKey,
    ) -> circular_protocol::boundary_port::BoundaryActorGeneration {
        self.tables.generation(key)
    }

    fn advance_actor_generation(&mut self, key: &PlanActorKey) -> Result<(), FoldRejection> {
        let next = self
            .actor_generation(key)
            .retired()
            .map_err(FoldRejection::GenerationExhausted)?;
        self.tables.put::<t::Generations>(key.clone(), next);
        Ok(())
    }

    /// Resolve the complete port set for every actor at or below `target`.
    ///
    /// Non-container actors go through the engine registry's canonical config
    /// fold. Pipeline actors get their outer port names from their paired child
    /// scope boundary, which is their only declaration authority.
    pub fn actor_ports(
        &self,
        target: &[ScopeSegment],
    ) -> Result<Vec<AuthoringActorPortFact>, FoldRejection> {
        self.expanded_templates()?.expanded_actor_ports(target)
    }

    fn expanded_actor_ports(
        &self,
        target: &[ScopeSegment],
    ) -> Result<Vec<AuthoringActorPortFact>, FoldRejection> {
        let scopes = self.normalized_scopes()?;
        let mut facts = Vec::new();
        for (actor, declaration) in self.tables.actors() {
            if !scope_is_at_or_below(&actor.scope, target) {
                continue;
            }
            let actor_type = ActorType::from_str(&declaration.actor_type).ok_or_else(|| {
                FoldRejection::UnknownActorType {
                    actor_type: declaration.actor_type.clone(),
                }
            })?;
            let (mut in_ports, out_ports) = if actor_type.is_container() {
                let mut child_scope = actor.scope.clone();
                child_scope.push(ScopeSegment::Child(actor.local.as_str().to_owned()));
                let scope = scopes
                    .iter()
                    .find(|(scope, _)| scope == &child_scope)
                    .map(|(_, declaration)| declaration)
                    .ok_or_else(|| FoldRejection::ContainerWithoutScopeDeclaration {
                        local: actor.local.clone(),
                    })?;
                (
                    scope
                        .boundary
                        .inlets
                        .iter()
                        .map(|binding| {
                            Ok(AuthoringPortFact {
                                id: binding.outer.clone(),
                                label: Some(self.boundary_label(&binding.inner.0)?),
                                flow: AuthoringPortFlowFact::Known(circular_actors::Flow::Stream(
                                    circular_actors::Shape::Any,
                                )),
                            })
                        })
                        .collect::<Result<_, FoldRejection>>()?,
                    scope
                        .boundary
                        .outlets
                        .iter()
                        .map(|binding| {
                            Ok(AuthoringPortFact {
                                id: binding.outer.clone(),
                                label: Some(self.boundary_label(&binding.inner.0)?),
                                flow: AuthoringPortFlowFact::Known(circular_actors::Flow::Stream(
                                    circular_actors::Shape::Any,
                                )),
                            })
                        })
                        .collect::<Result<_, FoldRejection>>()?,
                )
            } else {
                let reject = |fault| actor_config_rejection(actor, &declaration.config, fault);
                let config = config_of(&declaration.config)
                    .map_err(|error| reject(ConfigFault::Value(error)))?;
                let ports = resolve_declared_ports_at(
                    actor,
                    actor_type,
                    &config,
                    self.actor_generation(actor),
                )
                .map_err(|error| reject(ConfigFault::Ports(Box::new(error))))?;
                let label = matches!(actor_type, ActorType::Input | ActorType::Output)
                    .then(|| self.boundary_label(actor))
                    .transpose()?;
                let fact = |port: &crate::actor_registry::DeclaredPort| AuthoringPortFact {
                    id: port.id().as_str().to_owned(),
                    label: label.clone(),
                    flow: AuthoringPortFlowFact::Known(port.flow().clone()),
                };
                (
                    ports.inlets().iter().map(fact).collect(),
                    ports.outlets().iter().map(fact).collect(),
                )
            };
            if actor_type == ActorType::Replicator {
                let reject = |fault| actor_config_rejection(actor, &declaration.config, fault);
                let config = config_of(&declaration.config)
                    .map_err(|error| reject(ConfigFault::Value(error)))?;
                let ports = resolve_declared_ports_at(
                    actor,
                    actor_type,
                    &config,
                    self.actor_generation(actor),
                )
                .map_err(|error| reject(ConfigFault::Ports(Box::new(error))))?;
                in_ports = ports
                    .inlets()
                    .iter()
                    .map(|port| AuthoringPortFact {
                        id: port.id().as_str().to_owned(),
                        label: None,
                        flow: AuthoringPortFlowFact::Known(port.flow().clone()),
                    })
                    .collect();
            }
            facts.push(AuthoringActorPortFact {
                actor: actor.clone(),
                in_ports,
                out_ports,
            });
        }
        Ok(facts)
    }

    fn boundary_label(&self, boundary: &PlanActorKey) -> Result<String, FoldRejection> {
        self.tables
            .actors()
            .get(boundary)
            .and_then(
                |declaration| match declaration.config.as_object()?.get("label") {
                    Some(Value::String(label)) => Some(label.clone()),
                    _ => None,
                },
            )
            .ok_or_else(|| FoldRejection::BoundaryWithoutLabel {
                local: boundary.local.clone(),
            })
    }

    pub fn add_edge(&mut self, declaration: EdgeDeclaration) {
        mark_authored_scope(&mut self.authored_scopes, declaration.from.0.scope.clone());
        self.tables
            .put::<t::Edges>(edge_key(&declaration), declaration);
        self.accepted_content_commands += 1;
    }

    pub fn add_scope(&mut self, scope: Vec<ScopeSegment>, declaration: ScopeDeclaration) {
        mark_authored_scope(&mut self.authored_scopes, scope.clone());
        self.tables.put::<t::Scopes>(scope, declaration);
        self.accepted_content_commands += 1;
    }

    pub fn add_presentation(
        &mut self,
        owner: PresentationOwner,
        value: WirePresentation<PlanActorKey>,
    ) {
        mark_authored_scope(&mut self.authored_scopes, owner.scope().to_vec());
        self.tables.put::<t::Presentations>(owner, value);
        self.accepted_content_commands += 1;
    }

    pub fn add_export(&mut self, mount: PlanExportKey, declaration: ExportDeclaration) {
        mark_authored_scope(&mut self.authored_scopes, mount.scope.clone());
        self.tables.put::<t::Exports>(mount, declaration);
        self.accepted_content_commands += 1;
    }

    pub fn add_annotation(
        &mut self,
        annotation: PlanAnnotationKey,
        declaration: AnnotationDeclaration,
    ) {
        mark_authored_scope(&mut self.authored_scopes, annotation.scope.clone());
        self.tables.put::<t::Annotations>(annotation, declaration);
        self.accepted_content_commands += 1;
    }

    pub fn retire_actor(&mut self, key: &PlanActorKey) -> Result<(), FoldRejection> {
        mark_authored_scope(&mut self.authored_scopes, key.scope.clone());
        let existed = self.tables.actors().contains(key);
        self.tables.remove::<t::Actors>(key);
        self.tables
            .remove::<t::Presentations>(&PresentationOwner::Actor(key.clone()));
        let touching = self
            .tables
            .edges()
            .iter()
            .filter(|(_, edge)| &edge.from.0 == key || &edge.to.0 == key)
            .map(|(edge, _)| edge.clone())
            .collect::<Vec<_>>();
        for edge in touching {
            self.tables.remove::<t::Edges>(&edge);
        }
        self.accepted_content_commands += 1;
        if existed {
            self.advance_actor_generation(key)?;
        }
        Ok(())
    }

    pub fn retire_edge(&mut self, key: &DeclaredEdgeKey) {
        mark_authored_scope(&mut self.authored_scopes, key.from.0.scope.clone());
        self.tables.remove::<t::Edges>(key);
        self.accepted_content_commands += 1;
    }

    pub fn retire_scope(&mut self, scope: &[ScopeSegment]) -> Result<(), FoldRejection> {
        mark_authored_scope(&mut self.authored_scopes, scope.to_vec());
        if scope == self.target.as_slice() {
            self.retires_own_scope = true;
        }
        let descendants = self
            .tables
            .scopes()
            .keys()
            .filter(|existing| scope_is_at_or_below(existing, scope))
            .cloned()
            .collect::<Vec<_>>();
        for descendant in descendants {
            mark_authored_scope(&mut self.authored_scopes, descendant);
        }
        let retired_actors = self
            .tables
            .actors()
            .keys()
            .filter(|key| scope_is_at_or_below(&key.scope, scope))
            .cloned()
            .collect::<Vec<_>>();
        self.tables.remove_below(scope);
        self.accepted_content_commands += 1;
        for key in retired_actors {
            self.advance_actor_generation(&key)?;
        }
        Ok(())
    }

    pub fn set_flags(
        &mut self,
        key: &PlanActorKey,
        flags: circular_protocol::declaration_payload::ActorFlags,
    ) -> Result<(), FoldRejection> {
        let Some(declaration) = self.tables.actors().get(key) else {
            return Err(FoldRejection::SetFlagsMissingActor {
                local: key.local.clone(),
            });
        };
        let mut declaration = declaration.clone();
        mark_authored_scope(&mut self.authored_scopes, key.scope.clone());
        declaration.flags = flags;
        self.tables.put::<t::Actors>(key.clone(), declaration);
        self.accepted_content_commands += 1;
        Ok(())
    }

    pub fn retire_export(&mut self, key: &PlanExportKey) {
        mark_authored_scope(&mut self.authored_scopes, key.scope.clone());
        self.tables.remove::<t::Exports>(key);
        self.accepted_content_commands += 1;
    }

    pub fn retire_annotation(&mut self, key: &PlanAnnotationKey) {
        mark_authored_scope(&mut self.authored_scopes, key.scope.clone());
        self.tables.remove::<t::Annotations>(key);
        self.tables
            .remove::<t::Presentations>(&PresentationOwner::Annotation(key.clone()));
        self.accepted_content_commands += 1;
    }

    pub fn promote_environment(
        &mut self,
        replacement: AuthoringEnvironment,
    ) -> Result<(), FoldRejection> {
        if self.accepted_content_commands != 0 || self.replacement_environment.is_some() {
            return Err(FoldRejection::EnvironmentBarrierNotFirst);
        }
        if !self.target.is_empty() {
            return Err(FoldRejection::EnvironmentBarrierOutsideRoot);
        }
        self.replacement_environment = Some(replacement);
        self.accepted_content_commands = 1;
        Ok(())
    }

    fn normalized_scopes(
        &self,
    ) -> Result<Vec<(Vec<ScopeSegment>, ScopeDeclaration)>, FoldRejection> {
        self.tables
            .scopes()
            .iter()
            .map(|(scope, declaration)| {
                let derived = self.derived_boundary(scope)?;
                let is_replicator = scope.split_last().is_some_and(|(last, parent)| {
                    self.tables.actors().iter().any(|(actor, declaration)| {
                        actor.scope == parent
                            && *last == ScopeSegment::Child(actor.local.as_str().to_owned())
                            && ActorType::from_str(&declaration.actor_type)
                                == Some(ActorType::Replicator)
                    })
                });
                if is_replicator && derived.inlets.len() != 1 {
                    return Err(BoundaryFault::ReplicatorInletCount {
                        scope: scope.clone(),
                        inlets: derived.inlets.len(),
                    }
                    .into());
                }
                let authored = &declaration.boundary;
                if (!authored.inlets.is_empty() || !authored.outlets.is_empty())
                    && authored != &derived
                {
                    return Err(BoundaryFault::AuthoredDisagrees {
                        scope: scope.clone(),
                    }
                    .into());
                }
                let mut normalized = declaration.clone();
                normalized.boundary = derived;
                Ok((scope.clone(), normalized))
            })
            .collect()
    }

    fn derived_boundary(
        &self,
        scope: &[ScopeSegment],
    ) -> Result<circular_protocol::declaration_payload::ScopeBoundary, FoldRejection> {
        use circular_protocol::boundary_port::{BoundaryPortDirection, validate_boundary_port_ids};

        let boundaries = self
            .tables
            .actors()
            .iter()
            .filter(|(key, declaration)| {
                key.scope == scope && matches!(declaration.actor_type.as_str(), "input" | "output")
            })
            .map(|(key, declaration)| {
                let direction = if declaration.actor_type == "input" {
                    BoundaryPortDirection::Inlet
                } else {
                    BoundaryPortDirection::Outlet
                };
                (direction, key, self.actor_generation(key))
            })
            .collect::<Vec<_>>();
        let derived = validate_boundary_port_ids(boundaries.iter().copied()).map_err(|error| {
            BoundaryFault::PortIds {
                scope: scope.to_vec(),
                error,
            }
        })?;
        let mut inlets = Vec::new();
        let mut outlets = Vec::new();
        for (direction, actor, id) in derived {
            let mut outer = id.as_port_id().as_str().to_owned();
            if scope.split_last().is_some_and(|(last, parent)| {
                self.tables.actors().iter().any(|(key, declaration)| {
                    key.scope == parent
                        && *last == ScopeSegment::Child(key.local.as_str().to_owned())
                        && declaration
                            .config
                            .as_object()
                            .is_some_and(|config| config.contains_key("template"))
                })
            }) {
                let relative = PlanActorKey {
                    scope: vec![],
                    local: actor.local.clone(),
                };
                outer = circular_protocol::boundary_port::BoundaryPortId::derive(
                    direction,
                    &relative,
                    self.actor_generation(actor),
                )
                .map_err(TemplateFault::BoundaryPort)?
                .as_port_id()
                .as_str()
                .to_owned();
            }
            let binding = ScopeBinding {
                inner: (actor.clone(), id.as_port_id().as_str().to_owned()),
                outer,
            };
            match direction {
                BoundaryPortDirection::Inlet => inlets.push(binding),
                BoundaryPortDirection::Outlet => outlets.push(binding),
            }
        }
        inlets.sort_by(|left, right| left.outer.cmp(&right.outer));
        outlets.sort_by(|left, right| left.outer.cmp(&right.outer));
        Ok(circular_protocol::declaration_payload::ScopeBoundary { inlets, outlets })
    }

    fn actor_type(&self, actor: &PlanActorKey) -> Option<&str> {
        self.tables
            .actors()
            .get(actor)
            .map(|declaration| declaration.actor_type.as_str())
    }

    pub fn check_target_boundary(&self) -> Result<(), FoldRejection> {
        let target = &self.target;
        let mut outside = BTreeSet::new();
        for scope in &self.authored_scopes {
            if !scope_is_at_or_below(scope, target) {
                outside.insert((TargetWrite::AuthoredKey, scope.clone()));
            }
        }
        for (_, edge) in self.tables.edges() {
            for endpoint in [&edge.from.0, &edge.to.0] {
                if !scope_is_at_or_below(&endpoint.scope, target) {
                    outside.insert((TargetWrite::EdgeEndpoint, endpoint.scope.clone()));
                }
            }
        }
        for scope in self.tables.scopes().keys() {
            if scope.len() <= target.len() || !scope_is_at_or_below(scope, target) {
                outside.insert((TargetWrite::ScopeDeclaration, scope.clone()));
            }
        }
        if self.retires_own_scope {
            outside.insert((TargetWrite::ScopeRetirement, target.clone()));
        }
        if outside.is_empty() {
            return Ok(());
        }
        Err(FoldRejection::TargetBoundary {
            target: target.clone(),
            outside,
        })
    }

    pub fn assemble(&self) -> Result<AuthoredProjection, FoldRejection> {
        self.expanded_templates()?.assemble_expanded()
    }

    fn assemble_expanded(&self) -> Result<AuthoredProjection, FoldRejection> {
        let scopes = self.normalized_scopes()?;
        let mut tree = ScopeTree::default();

        for (key, declaration) in self.tables.actors().rows().to_vec() {
            let generation = self.actor_generation(&key);
            let mut refused = None;
            if let Some(actor_type) = ActorType::from_str(&declaration.actor_type) {
                if let circular_actors::RegistrationScope::Deferred(_) =
                    circular_actors::registration(actor_type).scope()
                {
                    return Err(FoldRejection::DeferredActorType {
                        actor_type: declaration.actor_type,
                    });
                }
                refused = admit_declared_config(actor_type, &key, &declaration.config, generation)
                    .err()
                    .map(|rejection| RefusedDeclaration {
                        key: key.clone(),
                        declaration: declaration.clone(),
                        generation,
                        rejection,
                    });
            }
            let path = relative(&key.scope, &self.target, AddressUse::Actor)?;
            tree.at(&path)
                .actors
                .push((key, declaration, generation.get(), refused));
        }
        for (scope, declaration) in scopes {
            let path = relative(&scope, &self.target, AddressUse::Scope)?;
            let Some((name, parent)) = path.split_last() else {
                return Err(FoldRejection::ScopeDeclaresTarget);
            };
            tree.at(parent)
                .children
                .entry(name.clone())
                .or_default()
                .declaration = Some(declaration);
        }
        for declaration in self.tables.edges().values().cloned() {
            let path = relative(&declaration.from.0.scope, &self.target, AddressUse::Edge)?;
            tree.at(&path).edges.push(declaration);
        }
        for (owner, value) in self.tables.presentations().rows().to_vec() {
            let path = relative(owner.scope(), &self.target, AddressUse::Presentation)?;
            tree.at(&path).presentations.push((
                owner.map(|key| key.local.as_str().to_owned(), |key| key.local),
                value,
            ));
        }
        for (mount, declaration) in self.tables.exports().rows().to_vec() {
            let path = relative(&mount.scope, &self.target, AddressUse::ExportMount)?;
            tree.at(&path).exports.push((mount.local, declaration));
        }
        for (annotation, declaration) in self.tables.annotations().rows().to_vec() {
            let path = relative(&annotation.scope, &self.target, AddressUse::Annotation)?;
            tree.at(&path)
                .annotations
                .push((annotation.local, declaration));
        }

        let mut builder = AuthoredProjectionBuilder::new();
        for template in self.tables.templates().values() {
            builder.set_template(template.clone());
        }
        let root_scope = ScopeId::root();
        tree.build(&mut builder, &root_scope, &self.target)?;
        let plan = builder.finish().map_err(|error| {
            if let crate::authoring_assembly::projection::BuildError::InvalidDownwardReference(
                actor,
            ) = &error
                && let Some(mut key) = authored_key_from_plan_actor(actor)
            {
                key.scope.splice(..0, self.target.iter().cloned());
                if !self.tables.actors().contains(&key) {
                    for (mount, declaration) in self.tables.exports() {
                        let roles = &declaration.roles;
                        if [&roles.request, &roles.progress, &roles.result, &roles.error]
                            .into_iter()
                            .flatten()
                            .any(|(target, _)| target == &key)
                        {
                            return FoldRejection::MountReferencesMissingActor {
                                mount: mount.local.clone(),
                                actor: key.local.as_str().to_owned(),
                            };
                        }
                    }
                }
            }
            FoldRejection::Build {
                step: BuildStep::Finish,
                error,
            }
        })?;
        validate_boundary_references(&plan)?;
        Ok(plan)
    }
}

#[derive(Default)]
struct BoundaryValidationFold {
    direct_actors: BTreeMap<NamedActorId, ActorDecl>,
    actors: BTreeMap<NamedActorId, ActorDecl>,
    edges: Vec<EdgeDecl>,
    bindings: BTreeMap<(NamedActorId, circular_actors::Side, PortId), circular_plan::Endpoint>,
    stale_bindings: BTreeSet<(NamedActorId, circular_actors::Side, PortId)>,
    boundary: ScopeBoundary,
    error: Option<FoldRejection>,
}

fn validate_boundary_references(plan: &AuthoredProjection) -> Result<(), FoldRejection> {
    let folded = fold_projection::<BoundaryValidationFold>(plan, |layer| {
        use circular_actors::Side;

        let direct_actors = layer.actors().clone();
        let layer_edges = layer.edges().clone();
        let layer_exports = layer.exports().clone();
        let mut folded = BoundaryValidationFold {
            direct_actors: direct_actors.clone(),
            actors: direct_actors,
            edges: layer_edges.values().cloned().collect(),
            boundary: layer.graph().declaration().boundary().clone(),
            ..BoundaryValidationFold::default()
        };

        for (segment, child) in layer.into_scopes() {
            let container = folded
                .direct_actors
                .keys()
                .find(|actor| actor.name() == segment.name())
                .cloned();
            if let Some(container) = container {
                for (outer, inner) in child.boundary.inlets() {
                    folded.bindings.insert(
                        (container.clone(), Side::Inlet, outer.clone()),
                        inner.clone(),
                    );
                }
                for (outer, inner) in child.boundary.outlets() {
                    folded.bindings.insert(
                        (container.clone(), Side::Outlet, outer.clone()),
                        inner.clone(),
                    );
                }
                for (actor, declaration) in &child.direct_actors {
                    let (side, direction) = match declaration.domain().actor_type() {
                        circular_plan::ActorType::Input => (
                            Side::Inlet,
                            circular_protocol::boundary_port::BoundaryPortDirection::Inlet,
                        ),
                        circular_plan::ActorType::Output => (
                            Side::Outlet,
                            circular_protocol::boundary_port::BoundaryPortDirection::Outlet,
                        ),
                        _ => continue,
                    };
                    if let Some(key) = authored_key_from_plan_actor(actor) {
                        for generation in 0..declaration.authored_generation() {
                            if let Ok(id) = circular_protocol::boundary_port::BoundaryPortId::derive(
                                direction,
                                &key,
                                circular_protocol::boundary_port::BoundaryActorGeneration::new(
                                    generation,
                                ),
                            ) {
                                folded.stale_bindings.insert((
                                    container.clone(),
                                    side,
                                    PortId::try_new(id.as_port_id().as_str())
                                        .expect("a derived boundary port name is also a plan name"),
                                ));
                            }
                        }
                    }
                }
            }

            if folded.error.is_none() {
                folded.error = child.error;
            }
            folded.actors.extend(child.actors);
            folded.edges.extend(child.edges);
            folded.bindings.extend(child.bindings);
            folded.stale_bindings.extend(child.stale_bindings);
        }

        if folded.error.is_none() {
            folded.error = validate_fold_layer(&layer_edges, &layer_exports, &folded).err();
        }
        folded
    });
    folded.error.map_or(Ok(()), Err)
}

fn validate_fold_layer(
    edges: &BTreeMap<DeclaredEdgeId, EdgeDecl>,
    exports: &BTreeMap<circular_plan::ExportName, circular_plan::Export>,
    folded: &BoundaryValidationFold,
) -> Result<(), FoldRejection> {
    use circular_actors::Side;
    use circular_protocol::boundary_port::BoundaryPortDirection;

    for edge in edges.values() {
        match folded.boundary_direction(edge.from().actor()) {
            Some(BoundaryPortDirection::Inlet) => {
                folded.validate_direct_ref(edge.from(), Side::Inlet)?;
            }
            Some(BoundaryPortDirection::Outlet) => return Err(boundary_leaf_unresolved()),
            None => {
                if folded
                    .actor_type(edge.from().actor())
                    .is_some_and(circular_plan::ActorType::is_container)
                {
                    folded.validate_container_ref(edge.from(), Side::Outlet)?;
                }
            }
        }
        match folded.boundary_direction(edge.to().actor()) {
            Some(BoundaryPortDirection::Outlet) => {
                folded.validate_direct_ref(edge.to(), Side::Outlet)?;
            }
            Some(BoundaryPortDirection::Inlet) => return Err(boundary_leaf_unresolved()),
            None => {
                if folded
                    .actor_type(edge.to().actor())
                    .is_some_and(circular_plan::ActorType::is_container)
                {
                    folded.validate_container_ref(edge.to(), Side::Inlet)?;
                }
            }
        }
    }

    for export in exports.values() {
        if let Some(mount) = export.roles().get(&Role::Request) {
            let endpoint =
                circular_plan::Endpoint::new(mount.actor().clone(), mount.port().clone());
            match folded.boundary_direction(mount.actor()) {
                Some(BoundaryPortDirection::Inlet) => {
                    folded.validate_direct_ref(&endpoint, Side::Inlet)?;
                }
                _ if folded
                    .actor_type(mount.actor())
                    .is_some_and(circular_plan::ActorType::is_container) =>
                {
                    folded.validate_container_ref(&endpoint, Side::Inlet)?;
                }
                _ => return Err(boundary_leaf_unresolved()),
            }
        }
        for role in [Role::Progress, Role::Result, Role::Error] {
            let Some(mount) = export.roles().get(&role) else {
                continue;
            };
            let endpoint =
                circular_plan::Endpoint::new(mount.actor().clone(), mount.port().clone());
            match folded.boundary_direction(mount.actor()) {
                Some(BoundaryPortDirection::Outlet) => {
                    folded.validate_direct_ref(&endpoint, Side::Outlet)?;
                }
                Some(BoundaryPortDirection::Inlet) => return Err(boundary_leaf_unresolved()),
                None => {
                    if folded
                        .actor_type(mount.actor())
                        .is_some_and(circular_plan::ActorType::is_container)
                    {
                        folded.validate_container_ref(&endpoint, Side::Outlet)?;
                    }
                }
            }
        }
    }
    Ok(())
}

impl BoundaryValidationFold {
    fn actor_type(&self, actor: &NamedActorId) -> Option<circular_plan::ActorType> {
        self.actors
            .get(actor)
            .map(|declaration| *declaration.domain().actor_type())
    }

    fn boundary_direction(
        &self,
        actor: &NamedActorId,
    ) -> Option<circular_protocol::boundary_port::BoundaryPortDirection> {
        self.actor_type(actor)
            .and_then(crate::actor_registry::boundary_direction)
    }

    fn validate_direct_ref(
        &self,
        endpoint: &circular_plan::Endpoint,
        side: circular_actors::Side,
    ) -> Result<(), FoldRejection> {
        use circular_actors::Side;
        use circular_protocol::boundary_port::BoundaryPortDirection;

        let declaration = self
            .actors
            .get(endpoint.actor())
            .ok_or_else(boundary_leaf_unresolved)?;
        let direction = match side {
            Side::Inlet => BoundaryPortDirection::Inlet,
            Side::Outlet => BoundaryPortDirection::Outlet,
        };
        let key =
            authored_key_from_plan_actor(endpoint.actor()).ok_or_else(boundary_leaf_unresolved)?;
        validate_direct_boundary_id(
            direction,
            &key,
            circular_protocol::boundary_port::BoundaryActorGeneration::new(
                declaration.authored_generation(),
            ),
            endpoint.port().as_str(),
        )?;
        if side == circular_actors::Side::Inlet
            || self.boundary_path_reaches_leaf(endpoint, side, &mut Vec::new())
        {
            Ok(())
        } else {
            Err(boundary_leaf_unresolved())
        }
    }

    fn validate_container_ref(
        &self,
        endpoint: &circular_plan::Endpoint,
        side: circular_actors::Side,
    ) -> Result<(), FoldRejection> {
        if side == circular_actors::Side::Inlet
            && self.actor_type(endpoint.actor()) == Some(ActorType::Replicator)
            && endpoint.port().as_str() == "event"
        {
            return Ok(());
        }
        let key = (endpoint.actor().clone(), side, endpoint.port().clone());
        let Some(inner) = self.bindings.get(&key) else {
            return if self.stale_bindings.contains(&key) {
                Err(stale_boundary_generation())
            } else {
                Err(boundary_leaf_unresolved())
            };
        };
        if side == circular_actors::Side::Inlet {
            return self.validate_direct_ref(inner, side);
        }
        if self.boundary_path_reaches_leaf(inner, side, &mut Vec::new()) {
            Ok(())
        } else {
            Err(boundary_leaf_unresolved())
        }
    }

    fn boundary_path_reaches_leaf(
        &self,
        endpoint: &circular_plan::Endpoint,
        side: circular_actors::Side,
        visited: &mut Vec<(circular_plan::Endpoint, circular_actors::Side)>,
    ) -> bool {
        use circular_actors::Side;

        if visited
            .iter()
            .any(|(seen, seen_side)| seen == endpoint && *seen_side == side)
        {
            return false;
        }
        visited.push((endpoint.clone(), side));
        let reaches = match side {
            Side::Inlet => self
                .edges
                .iter()
                .filter(|edge| edge.from() == endpoint)
                .any(|edge| {
                    if self.is_runtime_leaf(edge.to().actor())
                        || (self.actor_type(edge.to().actor()) == Some(ActorType::Replicator)
                            && edge.to().port().as_str() == "event")
                    {
                        return true;
                    }
                    if !self
                        .actor_type(edge.to().actor())
                        .is_some_and(ActorType::is_container)
                    {
                        return false;
                    }
                    self.bindings
                        .get(&(
                            edge.to().actor().clone(),
                            Side::Inlet,
                            edge.to().port().clone(),
                        ))
                        .is_some_and(|inner| {
                            self.boundary_path_reaches_leaf(inner, Side::Inlet, visited)
                        })
                }),
            Side::Outlet => self
                .edges
                .iter()
                .filter(|edge| edge.to() == endpoint)
                .any(|edge| {
                    if self.is_runtime_leaf(edge.from().actor()) {
                        return true;
                    }
                    if !self
                        .actor_type(edge.from().actor())
                        .is_some_and(ActorType::is_container)
                    {
                        return false;
                    }
                    self.bindings
                        .get(&(
                            edge.from().actor().clone(),
                            Side::Outlet,
                            edge.from().port().clone(),
                        ))
                        .is_some_and(|inner| {
                            self.boundary_path_reaches_leaf(inner, Side::Outlet, visited)
                        })
                }),
        };
        visited.pop();
        reaches
    }

    fn is_runtime_leaf(&self, actor: &NamedActorId) -> bool {
        self.actor_type(actor).is_some_and(|actor_type| {
            !actor_type.is_container()
                && !matches!(actor_type, ActorType::Input | ActorType::Output)
        })
    }
}

fn authored_key_from_plan_actor(actor: &NamedActorId) -> Option<PlanActorKey> {
    actor
        .scope()
        .segments()
        .iter()
        .all(|segment| matches!(segment, ScopeSeg::Child(_)))
        .then(|| circular_runtime::product_identity::wire_named_actor(actor))
}

fn moved_key<'a>(
    mappings: &'a [(PlanActorKey, PlanActorKey)],
    key: &PlanActorKey,
) -> Option<&'a PlanActorKey> {
    mappings
        .iter()
        .find(|(old, _)| old == key)
        .map(|(_, new)| new)
}

fn moved_keys<'a>(
    mappings: &[(PlanActorKey, PlanActorKey)],
    keys: impl Iterator<Item = &'a PlanActorKey>,
) -> Vec<(PlanActorKey, PlanActorKey)> {
    keys.filter_map(|key| moved_key(mappings, key).map(|new| (key.clone(), new.clone())))
        .collect()
}

fn direct_child_container(child: &[ScopeSegment], parent: &[ScopeSegment]) -> Option<PlanActorKey> {
    if child.len() != parent.len() + 1 || !child.starts_with(parent) {
        return None;
    }
    let ScopeSegment::Child(local) = child.last()? else {
        return None;
    };
    Some(PlanActorKey {
        scope: parent.to_vec(),
        local: AuthoredLocal::try_new(local.clone()).ok()?.into(),
    })
}

fn fold_boundary_local(
    direction: circular_protocol::boundary_port::BoundaryPortDirection,
    inner_local: &str,
    inner_port: &str,
) -> Result<AuthoredLocal, circular_protocol::declaration_payload::ReservedLocalSpelling> {
    use circular_protocol::boundary_port::BoundaryPortDirection;

    let side = match direction {
        BoundaryPortDirection::Inlet => "in",
        BoundaryPortDirection::Outlet => "out",
    };
    AuthoredLocal::try_new(format!(
        "b{}_{inner_local}_{inner_port}_{side}",
        inner_local.len()
    ))
}

fn push_relay_once(
    existing: &[EdgeDeclaration],
    pending: &mut Vec<EdgeDeclaration>,
    direction: circular_protocol::boundary_port::BoundaryPortDirection,
    boundary: &FoldBoundary,
    inner: &(PlanActorKey, String),
) {
    use circular_protocol::boundary_port::BoundaryPortDirection;

    let (from, to) = match direction {
        BoundaryPortDirection::Inlet => {
            ((boundary.key.clone(), boundary.port.clone()), inner.clone())
        }
        BoundaryPortDirection::Outlet => {
            (inner.clone(), (boundary.key.clone(), boundary.port.clone()))
        }
    };
    if existing
        .iter()
        .chain(pending.iter())
        .any(|edge| edge.from == from && edge.to == to)
    {
        return;
    }
    pending.push(EdgeDeclaration {
        from,
        to,
        ordinal: 0,
        attrs: circular_protocol::declaration_payload::EdgeAttrs {
            preprocess: circular_plan::PreprocessChain::default(),
            delay: circular_protocol::declaration_payload::DeclaredDelay::ZERO,
            policy: circular_protocol::declaration_payload::WirePolicy::DEFAULT_EDGE,
        },
    });
}

fn matches_retired_generation(
    direction: circular_protocol::boundary_port::BoundaryPortDirection,
    actor: &PlanActorKey,
    current: circular_protocol::boundary_port::BoundaryActorGeneration,
    port: &str,
) -> bool {
    (0..current.get()).any(|generation| {
        circular_protocol::boundary_port::BoundaryPortId::derive(
            direction,
            actor,
            circular_protocol::boundary_port::BoundaryActorGeneration::new(generation),
        )
        .is_ok_and(|id| id.as_port_id().as_str() == port)
    })
}

fn validate_direct_boundary_id(
    direction: circular_protocol::boundary_port::BoundaryPortDirection,
    actor: &PlanActorKey,
    current_generation: circular_protocol::boundary_port::BoundaryActorGeneration,
    port: &str,
) -> Result<(), FoldRejection> {
    let current = circular_protocol::boundary_port::BoundaryPortId::derive(
        direction,
        actor,
        current_generation,
    )
    .map_err(|_| boundary_leaf_unresolved())?;
    if current.as_port_id().as_str() == port {
        return Ok(());
    }
    if matches_retired_generation(direction, actor, current_generation, port) {
        Err(stale_boundary_generation())
    } else {
        Err(boundary_leaf_unresolved())
    }
}

fn stale_boundary_generation() -> FoldRejection {
    FoldRejection::Boundary(BoundaryFault::StaleGeneration)
}

fn boundary_leaf_unresolved() -> FoldRejection {
    FoldRejection::Boundary(BoundaryFault::LeafUnresolved)
}

fn mark_authored_scope(scopes: &mut Vec<Vec<ScopeSegment>>, scope: Vec<ScopeSegment>) {
    if !scopes.contains(&scope) {
        scopes.push(scope);
    }
}

fn edge_key(declaration: &EdgeDeclaration) -> DeclaredEdgeKey {
    DeclaredEdgeKey {
        from: declaration.from.clone(),
        to: declaration.to.clone(),
        ordinal: declaration.ordinal,
    }
}

fn scope_is_at_or_below(candidate: &[ScopeSegment], root: &[ScopeSegment]) -> bool {
    candidate.starts_with(root)
}

fn relative(
    scope: &[ScopeSegment],
    target: &[ScopeSegment],
    what: AddressUse,
) -> Result<Vec<Name>, FoldRejection> {
    if !scope.starts_with(target) {
        return Err(FoldRejection::AddressOutsideTarget { what });
    }
    scope[target.len()..]
        .iter()
        .map(|segment| match segment {
            ScopeSegment::Child(name) => Ok(Name::from_normalized(name.clone())),
            ScopeSegment::Instance { .. } => Err(FoldRejection::AddressAtInstance { what }),
        })
        .collect()
}

#[derive(Debug, Default)]
struct ScopeTree {
    actors: Vec<(
        PlanActorKey,
        ActorDeclaration,
        u64,
        Option<RefusedDeclaration>,
    )>,
    edges: Vec<EdgeDeclaration>,
    presentations: Vec<(
        PresentationOwner<String, String>,
        WirePresentation<PlanActorKey>,
    )>,
    exports: Vec<(String, ExportDeclaration)>,
    annotations: Vec<(String, AnnotationDeclaration)>,
    declaration: Option<ScopeDeclaration>,
    children: BTreeMap<Name, ScopeTree>,
}

impl ScopeTree {
    fn at(&mut self, path: &[Name]) -> &mut Self {
        let mut cursor = self;
        for segment in path {
            cursor = cursor.children.entry(segment.clone()).or_default();
        }
        cursor
    }

    fn build(
        mut self,
        builder: &mut AuthoredProjectionBuilder,
        scope: &ScopeId,
        target: &[ScopeSegment],
    ) -> Result<(), FoldRejection> {
        self.actors
            .sort_by(|(left, ..), (right, ..)| left.local.cmp(&right.local));
        self.edges
            .sort_by(|left, right| edge_order(left).cmp(&edge_order(right)));
        self.presentations
            .sort_by(|(left, _), (right, _)| left.cmp(right));
        self.exports
            .sort_by(|(left, _), (right, _)| left.cmp(right));
        self.annotations
            .sort_by(|(left, _), (right, _)| left.cmp(right));

        let mut containers = BTreeMap::new();
        let mut plain = Vec::new();
        for (key, declaration, generation, refused) in self.actors {
            let name = Name::from_normalized(key.local.as_str());
            if ActorType::from_str(&declaration.actor_type).is_some_and(ActorType::is_container) {
                containers.insert(name, (key, declaration, generation, refused));
            } else {
                plain.push((name, key, declaration, generation, refused));
            }
        }

        for (name, key, declaration, generation, refused) in plain {
            let domain = domain_of(&key, &declaration)?;
            let decl = NonContainerActorDecl::try_new_at_generation(
                domain,
                flags_of(&declaration),
                generation,
            )
            .map_err(FoldRejection::InvalidActorDeclaration)?;
            let actor = builder
                .add_actor(name, decl)
                .map_err(|error| FoldRejection::Build {
                    step: BuildStep::PlaceActor,
                    error,
                })?;
            if let Some(refused) = refused {
                builder.refuse(actor, refused);
            }
        }

        for (name, child) in self.children {
            let Some((key, container, generation, refused)) = containers.remove(&name) else {
                return Err(FoldRejection::ScopeWithoutContainer { scope: name });
            };
            let reject_config = |fault| actor_config_rejection(&key, &container.config, fault);
            let config = config_of(&container.config)
                .map_err(|error| reject_config(ConfigFault::Value(error)))?;
            let replicator =
                ActorType::from_str(&container.actor_type) == Some(ActorType::Replicator);
            if let Some(payload) = child.declaration.as_ref() {
                let template =
                    payload.role == circular_protocol::declaration_payload::ScopeRole::Template;
                if template != replicator {
                    return Err(FoldRejection::ScopeRoleMismatch {
                        scope: name,
                        role: payload.role,
                        container_type: container.actor_type,
                    });
                }
            }
            let container_actor = builder
                .enter_scope_with_container(
                    name.clone(),
                    if replicator {
                        ContainerActorDecl::replicator(config, flags_of(&container), generation)
                    } else {
                        ContainerActorDecl::pipeline_template(
                            config,
                            flags_of(&container),
                            generation,
                        )
                    },
                )
                .map_err(|error| FoldRejection::Build {
                    step: BuildStep::OpenScope,
                    error,
                })?;
            if let Some(refused) = refused {
                builder.refuse(container_actor, refused);
            }
            let child_scope = scope
                .append_segment(ScopeSeg::Child(name.clone()))
                .map_err(|_| FoldRejection::ScopeDepthLimit)?;
            let boundary = match child.declaration.as_ref() {
                Some(payload) => boundary_of(&name, &payload.boundary, target)?,
                None => ScopeBoundary::sealed(),
            };
            child.build(builder, &child_scope, target)?;
            builder
                .set_boundary(boundary)
                .map_err(|error| FoldRejection::Build {
                    step: BuildStep::PlaceBoundary,
                    error,
                })?;
            builder.exit_scope().map_err(|error| FoldRejection::Build {
                step: BuildStep::CloseScope,
                error,
            })?;
        }

        if let Some((name, _)) = containers.into_iter().next() {
            return Err(FoldRejection::ContainerWithoutScope { container: name });
        }

        let presentations = self
            .presentations
            .into_iter()
            .map(|(local, value)| {
                let presentation = presentation_of(&value, scope);
                (local, presentation)
            })
            .collect::<Vec<_>>();

        for (left_index, (left_local, left)) in presentations.iter().enumerate() {
            let Some(left_board) = left.board() else {
                continue;
            };
            for (right_local, right) in presentations.iter().skip(left_index + 1) {
                let Some(right_board) = right.board() else {
                    continue;
                };
                if left_board.overlaps(right_board) {
                    return Err(FoldRejection::BoardOverlap {
                        left: match left_local {
                            PresentationOwner::Actor(local)
                            | PresentationOwner::Annotation(local) => local.clone(),
                        },
                        left_board,
                        right: match right_local {
                            PresentationOwner::Actor(local)
                            | PresentationOwner::Annotation(local) => local.clone(),
                        },
                        right_board,
                    });
                }
            }
        }

        for (local, presentation) in presentations {
            let owner = local.map(
                |name| NamedActorId::new(scope.clone(), Name::from_normalized(name)),
                |name| AnnotationId::new(Name::from_normalized(name)),
            );
            builder.set_presentation(owner, presentation);
        }

        for (local, declaration) in self.exports {
            let name = Name::from_normalized(local.clone());
            builder.set_export(
                ExportName::new(name),
                export_of(&local, &declaration, scope, target)?,
            );
        }

        for (local, declaration) in self.annotations {
            builder.set_annotation(
                AnnotationId::new(Name::from_normalized(local.clone())),
                annotation_of(&declaration, target)?,
            );
        }

        for declaration in self.edges {
            let from = endpoint_of(&declaration.from, scope)?;
            let to = endpoint_of(&declaration.to, scope)?;
            builder
                .add_edge(
                    from,
                    to,
                    declaration.ordinal,
                    attrs_of(&declaration.attrs, declaration.to.0.local.as_str())?,
                )
                .map_err(|error| FoldRejection::Build {
                    step: BuildStep::PlaceEdge,
                    error,
                })?;
        }
        Ok(())
    }
}

fn edge_order(edge: &EdgeDeclaration) -> (&str, &str, &str, &str, u16) {
    (
        edge.from.0.local.as_str(),
        edge.from.1.as_str(),
        edge.to.0.local.as_str(),
        edge.to.1.as_str(),
        edge.ordinal,
    )
}

fn presentation_of(value: &WirePresentation<PlanActorKey>, scope: &ScopeId) -> Presentation {
    let target_of = |target: &PlanActorKey| {
        NamedActorId::new(scope.clone(), Name::from_normalized(target.local.as_str()))
    };
    let anchor = value.anchor.as_ref().map(|anchor| match anchor {
        WireAnchor::Flow => Anchor::Flow,
        WireAnchor::Relative { target, relation } => Anchor::Relative {
            target: target_of(target),
            relation: *relation,
        },
        WireAnchor::Align { target, axis } => Anchor::Align {
            target: target_of(target),
            axis: *axis,
        },
    });

    Presentation {
        label: value.label.clone(),
        group: value.group.clone(),
        anchor,
        fixed: value.fixed,
        size: value.size,
        board: value.board,
        view: value.view.clone(),
        collapsed: value.collapsed,
    }
}

fn boundary_of(
    name: &Name,
    boundary: &circular_protocol::declaration_payload::ScopeBoundary,
    target: &[ScopeSegment],
) -> Result<ScopeBoundary, FoldRejection> {
    let bind = |binding: &circular_protocol::declaration_payload::ScopeBinding| {
        let path = relative(&binding.inner.0.scope, target, AddressUse::BoundaryBinding)?;
        let mut inner = ScopeId::root();
        for segment in path {
            inner = inner
                .append_segment(ScopeSeg::Child(segment))
                .map_err(|_| FoldRejection::ScopeDepthLimit)?;
        }
        let local = Name::from_normalized(binding.inner.0.local.as_str());
        let port = PortId::try_new(binding.inner.1.clone()).map_err(|_| {
            FoldRejection::PortNotCanonical {
                at: PortAt::BoundaryInner,
                port: binding.inner.1.clone(),
            }
        })?;
        let outer = PortId::try_new(binding.outer.clone()).map_err(|_| {
            FoldRejection::PortNotCanonical {
                at: PortAt::BoundaryOuter {
                    scope: name.clone(),
                },
                port: binding.outer.clone(),
            }
        })?;
        Ok::<_, FoldRejection>((Endpoint::new(NamedActorId::new(inner, local), port), outer))
    };

    let mut inlets = Vec::new();
    for binding in &boundary.inlets {
        let (endpoint, outer) = bind(binding)?;
        inlets.push((outer, endpoint));
    }
    let mut outlets = Vec::new();
    for binding in &boundary.outlets {
        let (endpoint, outer) = bind(binding)?;
        outlets.push((endpoint, outer));
    }
    Ok(ScopeBoundary::new(inlets, outlets))
}

fn export_of(
    local: &str,
    declaration: &ExportDeclaration,
    scope: &ScopeId,
    target: &[ScopeSegment],
) -> Result<Export, FoldRejection> {
    if let Some(operations) = &declaration.operations
        && declaration.surface.is_none()
    {
        return Err(FoldRejection::ExportOperationsWithoutSurface {
            export: local.to_owned(),
            scope: scope.clone(),
            operations: operations.clone(),
            roles: declaration.roles.clone(),
        });
    }
    let mut roles = BTreeMap::new();
    let mut bind = |role: Role,
                    slot: &Option<(PlanActorKey, String)>|
     -> Result<(), FoldRejection> {
        let Some((key, port)) = slot else {
            return Ok(());
        };
        let path = relative(&key.scope, target, AddressUse::ExportRole)?;
        let mut at = ScopeId::root();
        for segment in path {
            at = at
                .append_segment(ScopeSeg::Child(segment))
                .map_err(|_| FoldRejection::ScopeDepthLimit)?;
        }
        let port = PortId::try_new(port.clone()).map_err(|_| FoldRejection::PortNotCanonical {
            at: PortAt::ExportRole {
                export: local.to_owned(),
            },
            port: port.clone(),
        })?;
        roles.insert(
            role,
            Mount::new(
                NamedActorId::new(at, Name::from_normalized(key.local.as_str())),
                port,
            ),
        );
        Ok(())
    };
    bind(Role::Request, &declaration.roles.request)?;
    bind(Role::Progress, &declaration.roles.progress)?;
    bind(Role::Result, &declaration.roles.result)?;
    bind(Role::Error, &declaration.roles.error)?;
    let _ = scope;

    let operations = OperationDecl::new(match &declaration.operations {
        None => Config::default(),
        Some(value) => config_of(value).map_err(|error| FoldRejection::ExportOperations {
            export: local.to_owned(),
            error,
        })?,
    });
    let surface = declaration
        .surface
        .as_ref()
        .map(|value| ConfigValue::from_wire_value(value).map_err(FoldRejection::ExportSurface))
        .transpose()?;
    Ok(Export::new(roles, operations).with_surface(surface))
}

fn annotation_of(
    declaration: &AnnotationDeclaration,
    target: &[ScopeSegment],
) -> Result<Annotation, FoldRejection> {
    let kind = match declaration.kind {
        circular_protocol::declaration_payload::AnnotationKind::Note => AnnotationKind::Note,
        circular_protocol::declaration_payload::AnnotationKind::Backdrop => {
            AnnotationKind::Backdrop
        }
    };
    let refs = declaration
        .refs
        .iter()
        .map(|actor| {
            let path = relative(&actor.scope, target, AddressUse::AnnotationReference)?;
            let scope =
                ScopeId::from_segments(path.into_iter().map(ScopeSeg::Child).collect::<Vec<_>>())
                    .map_err(|_| FoldRejection::AnnotationReferenceDepthLimit)?;
            Ok(NamedActorId::new(
                scope,
                Name::from_normalized(actor.local.as_str()),
            ))
        })
        .collect::<Result<BTreeSet<_>, FoldRejection>>()?;
    Ok(Annotation::new(
        kind,
        refs,
        AnnotationPlacement::unplaced(),
        Text::new(declaration.body.clone()),
    ))
}

fn endpoint_of(
    (key, port): &(PlanActorKey, String),
    scope: &ScopeId,
) -> Result<Endpoint, FoldRejection> {
    let name = Name::from_normalized(key.local.as_str());
    let port = PortId::try_new(port.clone()).map_err(|_| FoldRejection::PortNotCanonical {
        at: PortAt::Edge,
        port: port.clone(),
    })?;
    Ok(Endpoint::new(NamedActorId::new(scope.clone(), name), port))
}

fn domain_of(
    actor: &PlanActorKey,
    declaration: &ActorDeclaration,
) -> Result<ActorDomain, FoldRejection> {
    let actor_type = ActorType::from_str(&declaration.actor_type).ok_or_else(|| {
        FoldRejection::UnknownActorType {
            actor_type: declaration.actor_type.clone(),
        }
    })?;
    let config = config_of(&declaration.config).map_err(|error| {
        actor_config_rejection(actor, &declaration.config, ConfigFault::Value(error))
    })?;
    Ok(ActorDomain::new(actor_type, config))
}

fn actor_config_rejection(
    actor: &PlanActorKey,
    config: &Value,
    fault: ConfigFault,
) -> FoldRejection {
    FoldRejection::Config {
        actor: actor.to_string(),
        at: ConfigAt::Config,
        value: Some(config.clone()),
        fault,
    }
}

#[must_use]
pub fn binds_template(actor_type: ActorType, config: &Value) -> bool {
    actor_type.is_container()
        && config
            .as_object()
            .is_some_and(|config| config.contains_key("template"))
}

fn admit_declared_config(
    actor_type: ActorType,
    key: &PlanActorKey,
    config: &Value,
    generation: circular_protocol::boundary_port::BoundaryActorGeneration,
) -> Result<(), FoldRejection> {
    if binds_template(actor_type, config) {
        return Ok(());
    }
    let actor = key.to_string();
    circular_actors::admit_registered_create_at(actor_type, config, key, generation)
        .map(|_| ())
        .map_err(|error| {
            let (detail, admission) = match error {
                circular_actors::RegisteredCreateAdmissionError::Config(error) => (
                    error.rejection_message(&actor, config),
                    Some(Box::new(error)),
                ),
                other => (
                    circular_actors::config::config_rejection(
                        &actor,
                        "config",
                        Some(config),
                        other,
                    ),
                    None,
                ),
            };
            FoldRejection::Registry(Box::new(
                crate::actor_registry::PlanRegistryError::ConfigFold {
                    actor_type,
                    detail,
                    admission,
                },
            ))
        })
}

fn config_of(value: &Value) -> Result<Config, circular_plan::ConfigValueError> {
    Config::from_wire_value(value)
}

const fn flags_of(declaration: &ActorDeclaration) -> ActorFlags {
    ActorFlags::new(
        declaration.flags.bypass,
        declaration.flags.mute,
        declaration.flags.pause,
    )
}

fn attrs_of(attrs: &EdgeAttrs, actor: &str) -> Result<EdgeAttrs, FoldRejection> {
    for (index, step) in attrs.preprocess().steps().iter().enumerate() {
        let reject = |fault| FoldRejection::Config {
            actor: actor.to_owned(),
            at: ConfigAt::Preprocess(index),
            value: step.config().to_wire_value().ok(),
            fault,
        };
        let value = crate::activation_config::fold_preprocess_config(step.config())
            .map_err(|error| reject(ConfigFault::PreprocessFold(error)))?;
        match step.kind() {
            PreprocessKind::Map => circular_actors::map_config::accept_transform(&value)
                .map(|_| ())
                .map_err(ConfigFault::Map),
            PreprocessKind::Filter => circular_actors::accept_predicate(&value)
                .map(|_| ())
                .map_err(ConfigFault::Filter),
            PreprocessKind::Parse => circular_actors::parse_config::ParseConfig::from_value(&value)
                .map(|_| ())
                .map_err(ConfigFault::Parse),
            PreprocessKind::Flatten => circular_actors::flatten::FlattenConfig::from_value(&value)
                .map(|_| ())
                .map_err(ConfigFault::Flatten),
            PreprocessKind::Bang => {
                circular_actors::reject_nonempty_config(&value).map_err(ConfigFault::Bang)
            }
        }
        .map_err(reject)?;
    }
    Ok(attrs.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_protocol::boundary_port::{
        BoundaryActorGeneration, BoundaryPortDirection, BoundaryPortId,
    };
    use circular_protocol::declaration_payload as wire;
    use circular_protocol::rejection_code::Reasoned;

    fn child(name: &str) -> ScopeSegment {
        ScopeSegment::Child(name.to_owned())
    }

    fn actor(actor_type: &str) -> ActorDeclaration {
        ActorDeclaration {
            actor_type: actor_type.to_owned(),
            config: Value::Null,
            flags: wire::ActorFlags {
                bypass: false,
                mute: false,
                pause: false,
            },
        }
    }

    fn actor_with_config(actor_type: &str, config: Value) -> ActorDeclaration {
        ActorDeclaration {
            actor_type: actor_type.to_owned(),
            config,
            flags: wire::ActorFlags {
                bypass: false,
                mute: false,
                pause: false,
            },
        }
    }

    fn key(scope: Vec<ScopeSegment>, local: &str) -> PlanActorKey {
        PlanActorKey {
            scope,
            local: AuthoredLocal::try_new(local)
                .expect("test actor local is authored")
                .into(),
        }
    }

    fn edge(scope: &[ScopeSegment], from: &str, to: &str) -> EdgeDeclaration {
        EdgeDeclaration {
            from: (key(scope.to_vec(), from), "out".to_owned()),
            to: (key(scope.to_vec(), to), "in".to_owned()),
            ordinal: 0,
            attrs: wire::EdgeAttrs {
                preprocess: wire::PreprocessChain::default(),
                delay: wire::DeclaredDelay::ZERO,
                policy: wire::WirePolicy {
                    delivery: wire::Delivery::Lossless,
                    capacity: None,
                },
            },
        }
    }

    fn preprocess_config(fields: &[(&str, &str)]) -> Value {
        Value::object(
            fields
                .iter()
                .map(|(key, value)| (*key, Value::string(*value))),
        )
        .unwrap()
    }

    fn preprocess_record(fields: &[(&str, &str)]) -> Config {
        Config::from_wire_value(&preprocess_config(fields)).expect("an object becomes a record")
    }

    fn preprocess_candidate(steps: Vec<wire::PreprocessStep>) -> EpochCandidate {
        let mut candidate = EpochCandidate::open(Vec::new()).unwrap();
        candidate.add_actor(key(Vec::new(), "source"), actor("tap"));
        candidate.add_actor(key(Vec::new(), "sink"), actor("tap"));
        let mut declaration = edge(&[], "source", "sink");
        declaration.attrs.preprocess = wire::PreprocessChain::new(steps);
        candidate.add_edge(declaration);
        candidate
    }

    #[test]
    fn preprocess_admits_all_four_kinds_and_preserves_order_and_config() {
        use wire::{PreprocessKind as Kind, PreprocessStep as Step};
        let steps = vec![
            Step {
                kind: Kind::Map,
                config: preprocess_record(&[("transform", "event")]),
            },
            Step {
                kind: Kind::Filter,
                config: preprocess_record(&[("predicate", "true")]),
            },
            Step {
                kind: Kind::Bang,
                config: Config::default(),
            },
            Step {
                kind: Kind::Parse,
                config: preprocess_record(&[("decoder", "json"), ("field", "body")]),
            },
        ];
        let plan = preprocess_candidate(steps.clone())
            .assemble()
            .expect("four admissions");
        let attrs = plan.graph().edges().values().next().unwrap().attrs();
        assert_eq!(
            attrs
                .preprocess()
                .steps()
                .iter()
                .map(|step| step.kind())
                .collect::<Vec<_>>(),
            vec![
                circular_plan::PreprocessKind::Map,
                circular_plan::PreprocessKind::Filter,
                circular_plan::PreprocessKind::Bang,
                circular_plan::PreprocessKind::Parse
            ]
        );
        for (actual, authored) in attrs.preprocess().steps().iter().zip(&steps) {
            assert_eq!(actual.config(), &authored.config);
        }
        preprocess_candidate(vec![Step {
            kind: Kind::Bang,
            config: Config::from_wire_value(&Value::object([] as [(&str, Value); 0]).unwrap())
                .expect("an empty object is an empty record"),
        }])
        .assemble()
        .expect("empty object also admits bang");
    }

    #[test]
    fn preprocess_refuses_each_invalid_config_with_the_actors_reason() {
        use wire::{PreprocessKind as Kind, PreprocessStep as Step};
        let map = preprocess_config(&[("transform", "event +")]);
        let filter = preprocess_config(&[("predicate", "event +")]);
        let parse = preprocess_config(&[("decoder", "unregistered"), ("field", "body")]);
        let bang = preprocess_config(&[("unexpected", "value")]);
        let cases = [
            (
                Kind::Map,
                map.clone(),
                ConfigFault::Map(circular_actors::accept_transform(&map).unwrap_err()),
            ),
            (
                Kind::Filter,
                filter.clone(),
                ConfigFault::Filter(circular_actors::accept_predicate(&filter).unwrap_err()),
            ),
            (
                Kind::Parse,
                parse.clone(),
                ConfigFault::Parse(circular_actors::ParseConfig::from_value(&parse).unwrap_err()),
            ),
            (
                Kind::Bang,
                bang.clone(),
                ConfigFault::Bang(circular_actors::reject_nonempty_config(&bang).unwrap_err()),
            ),
        ];
        for (kind, config, expected) in cases {
            let candidate = preprocess_candidate(vec![
                Step {
                    kind: Kind::Bang,
                    config: Config::default(),
                },
                Step {
                    kind,
                    config: Config::from_wire_value(&config).expect("an object becomes a record"),
                },
            ]);
            let error = candidate.assemble().expect_err(kind.as_str());
            let FoldRejection::Config {
                actor, at, fault, ..
            } = error
            else {
                panic!("{kind:?}: not a config rejection: {error:?}");
            };
            assert_eq!(
                (actor.as_str(), at, fault),
                ("sink", ConfigAt::Preprocess(1), expected)
            );
        }
    }

    fn projected(plan: &AuthoredProjection) -> String {
        let actors: Vec<String> = plan
            .graph()
            .actors()
            .keys()
            .map(|id| format!("{id:?}"))
            .collect();
        let edges: Vec<String> = plan
            .graph()
            .edges()
            .keys()
            .map(|id| format!("{id:?}"))
            .collect();
        format!("{}\n{}", actors.join("\n"), edges.join("\n"))
    }

    #[test]
    fn authored_input_projects_its_identity_derived_any_outlet() {
        let mut candidate = EpochCandidate::open(Vec::new()).expect("root");
        let input = key(Vec::new(), "request-input");
        candidate.add_actor(
            input.clone(),
            actor_with_config(
                "input",
                Value::object([("label", Value::String("request".to_owned()))])
                    .expect("boundary config"),
            ),
        );

        let facts = candidate.actor_ports(&[]).expect("boundary ports");
        let [fact] = facts.as_slice() else {
            panic!("one boundary actor")
        };
        assert_eq!(fact.actor, input);
        assert!(fact.in_ports.is_empty());
        let [outlet] = fact.out_ports.as_slice() else {
            panic!("one boundary outlet")
        };
        let expected = circular_protocol::boundary_port::BoundaryPortId::derive(
            circular_protocol::boundary_port::BoundaryPortDirection::Inlet,
            &input,
            circular_protocol::boundary_port::BoundaryActorGeneration::initial(),
        )
        .unwrap();
        assert_eq!(outlet.id, expected.as_port_id().as_str());
        assert_eq!(outlet.label.as_deref(), Some("request"));
        assert_eq!(
            outlet.flow,
            AuthoringPortFlowFact::Known(circular_actors::Flow::Stream(
                circular_actors::Shape::Any
            ))
        );
        candidate
            .assemble()
            .expect("identity-aware plan registry admits the boundary declaration");
    }

    #[test]
    fn pipeline_ports_are_the_derived_names_of_the_child_boundary() {
        let mut candidate = EpochCandidate::open(Vec::new()).expect("root");
        let container = key(Vec::new(), "cell");
        candidate.add_actor(container.clone(), actor("pipeline_actor"));
        let child_scope = vec![child("cell")];
        let input = key(child_scope.clone(), "input");
        let output = key(child_scope.clone(), "output");
        candidate.add_actor(
            input.clone(),
            actor_with_config(
                "input",
                Value::object([("label", Value::String("request".to_owned()))])
                    .expect("input config"),
            ),
        );
        candidate.add_actor(
            output.clone(),
            actor_with_config(
                "output",
                Value::object([("label", Value::String("result".to_owned()))])
                    .expect("output config"),
            ),
        );
        candidate.add_scope(
            child_scope.clone(),
            wire::ScopeDeclaration {
                role: wire::ScopeRole::Concrete,
                boundary: wire::ScopeBoundary {
                    inlets: Vec::new(),
                    outlets: Vec::new(),
                },
            },
        );

        let facts = candidate.actor_ports(&[]).expect("boundary ports");
        let fact = facts
            .iter()
            .find(|fact| fact.actor == container)
            .expect("container fact");
        let inlet = BoundaryPortId::derive(
            BoundaryPortDirection::Inlet,
            &input,
            BoundaryActorGeneration::initial(),
        )
        .expect("inlet")
        .into_port_id();
        let outlet = BoundaryPortId::derive(
            BoundaryPortDirection::Outlet,
            &output,
            BoundaryActorGeneration::initial(),
        )
        .expect("outlet")
        .into_port_id();
        assert_eq!(fact.in_ports[0].id, inlet.as_str());
        assert_eq!(fact.out_ports[0].id, outlet.as_str());
        assert_eq!(fact.in_ports[0].label.as_deref(), Some("request"));
        assert_eq!(fact.out_ports[0].label.as_deref(), Some("result"));
        for port in fact.in_ports.iter().chain(fact.out_ports.iter()) {
            assert_eq!(
                port.flow,
                AuthoringPortFlowFact::Known(circular_actors::Flow::Stream(
                    circular_actors::Shape::Any
                ))
            );
        }
    }

    #[test]
    fn config_edit_preserves_generation_and_retire_advances_it() {
        let mut candidate = EpochCandidate::open(Vec::new()).expect("root");
        let input = key(Vec::new(), "input");
        candidate.add_actor(
            input.clone(),
            actor_with_config(
                "input",
                Value::object([("label", Value::String("first".to_owned()))]).unwrap(),
            ),
        );
        candidate.add_actor(
            input.clone(),
            actor_with_config(
                "input",
                Value::object([("label", Value::String("renamed".to_owned()))]).unwrap(),
            ),
        );
        assert_eq!(candidate.actor_generation(&input).get(), 0);
        candidate.retire_actor(&input).expect("retire");
        assert_eq!(candidate.actor_generation(&input).get(), 1);
    }

    #[test]
    fn recreated_boundary_rejects_an_old_export_generation() {
        let mut candidate = EpochCandidate::open(Vec::new()).expect("root");
        let container = key(Vec::new(), "cell");
        let child_scope = vec![child("cell")];
        let input = key(child_scope.clone(), "input");
        let consumer = key(child_scope.clone(), "consumer");
        candidate.add_actor(container.clone(), actor("pipeline_actor"));
        candidate.add_scope(
            child_scope.clone(),
            wire::ScopeDeclaration {
                role: wire::ScopeRole::Concrete,
                boundary: wire::ScopeBoundary {
                    inlets: Vec::new(),
                    outlets: Vec::new(),
                },
            },
        );
        candidate.add_actor(
            input.clone(),
            actor_with_config(
                "input",
                Value::object([("label", Value::String("in".to_owned()))]).unwrap(),
            ),
        );
        candidate.add_actor(consumer.clone(), actor("tap"));
        let old = BoundaryPortId::derive(
            BoundaryPortDirection::Inlet,
            &input,
            BoundaryActorGeneration::initial(),
        )
        .unwrap()
        .into_port_id();
        candidate.retire_actor(&input).expect("retire");
        candidate.add_actor(
            input.clone(),
            actor_with_config(
                "input",
                Value::object([("label", Value::String("in".to_owned()))]).unwrap(),
            ),
        );
        candidate.retire_actor(&input).expect("retire again");
        candidate.add_actor(
            input.clone(),
            actor_with_config(
                "input",
                Value::object([("label", Value::String("in".to_owned()))]).unwrap(),
            ),
        );
        let current = BoundaryPortId::derive(
            BoundaryPortDirection::Inlet,
            &input,
            candidate.actor_generation(&input),
        )
        .unwrap()
        .into_port_id();
        candidate.add_edge(wire::EdgeDeclaration {
            from: (input, current.as_str().to_owned()),
            to: (consumer, "event".to_owned()),
            ordinal: 0,
            attrs: wire::EdgeAttrs {
                preprocess: wire::PreprocessChain::default(),
                delay: wire::DeclaredDelay::ZERO,
                policy: wire::WirePolicy {
                    delivery: wire::Delivery::Lossless,
                    capacity: None,
                },
            },
        });
        candidate.add_export(
            wire::PlanExportKey {
                scope: Vec::new(),
                local: "front".to_owned(),
            },
            wire::ExportDeclaration {
                surface: None,
                roles: wire::ExportRoles {
                    request: Some((container, old.as_str().to_owned())),
                    progress: None,
                    result: None,
                    error: None,
                },
                operations: None,
            },
        );

        assert_eq!(
            candidate.assemble(),
            Err(FoldRejection::Boundary(BoundaryFault::StaleGeneration))
        );
    }

    #[test]
    fn request_boundary_can_be_declared_before_its_child_wires() {
        let mut candidate = EpochCandidate::open(Vec::new()).expect("root");
        let container = key(Vec::new(), "cell");
        let child_scope = vec![child("cell")];
        let input = key(child_scope.clone(), "input");
        candidate.add_actor(container.clone(), actor("pipeline_actor"));
        candidate.add_scope(
            child_scope,
            wire::ScopeDeclaration {
                role: wire::ScopeRole::Concrete,
                boundary: wire::ScopeBoundary {
                    inlets: Vec::new(),
                    outlets: Vec::new(),
                },
            },
        );
        candidate.add_actor(
            input.clone(),
            actor_with_config(
                "input",
                Value::object([("label", Value::String("in".to_owned()))]).unwrap(),
            ),
        );
        let port = BoundaryPortId::derive(
            BoundaryPortDirection::Inlet,
            &input,
            BoundaryActorGeneration::initial(),
        )
        .unwrap()
        .into_port_id();
        candidate.add_export(
            wire::PlanExportKey {
                scope: Vec::new(),
                local: "front".to_owned(),
            },
            wire::ExportDeclaration {
                surface: None,
                roles: wire::ExportRoles {
                    request: Some((container, port.as_str().to_owned())),
                    progress: None,
                    result: None,
                    error: None,
                },
                operations: None,
            },
        );
        let plan = candidate
            .assemble()
            .expect("an existing boundary needs no downstream actor");
        assert_eq!(plan.exports().len(), 1);
        candidate.retire_actor(&input).unwrap();
        assert_eq!(
            candidate.assemble(),
            Err(boundary_leaf_unresolved()),
            "a missing boundary still rejects instead of binding by label"
        );
    }

    #[test]
    fn an_instance_target_dies_where_the_bracket_opens() {
        let target = vec![
            child("fleet"),
            ScopeSegment::Instance {
                of: "cell".to_owned(),
                key: wire::InstanceKey::Scalar(wire::ScalarKey::Int(7)),
            },
        ];
        let reason = EpochCandidate::open(target).expect_err("an instance target does not open");
        assert_eq!(reason, FoldRejection::TargetIsInstance);
        assert!(
            EpochCandidate::open(vec![child("fleet")]).is_ok(),
            "a plain child scope target opens; a refusal that blocks everything is a defect too"
        );
    }

    #[test]
    fn arrival_order_does_not_change_the_plan() {
        let target = vec![child("fleet")];
        let names = ["sink", "source", "relay", "other"];

        let mut forward = EpochCandidate::open(target.clone()).expect("opens");
        for name in names {
            forward.add_actor(key(target.clone(), name), actor("tap"));
        }
        forward.add_edge(edge(&target, "source", "relay"));
        forward.add_edge(edge(&target, "relay", "sink"));

        let mut backward = EpochCandidate::open(target.clone()).expect("opens");
        for name in names.iter().rev() {
            backward.add_actor(key(target.clone(), name), actor("tap"));
        }
        backward.add_edge(edge(&target, "relay", "sink"));
        backward.add_edge(edge(&target, "source", "relay"));

        let first = forward.assemble().expect("the plan builds");
        let second = backward.assemble().expect("the plan builds");
        assert_eq!(
            projected(&first),
            projected(&second),
            "arrival order changed the plan; assembly is not a function"
        );
    }

    #[test]
    fn a_container_that_carries_config_is_refused() {
        let target = vec![child("fleet")];
        let inner = vec![child("fleet"), child("cell")];

        let mut container = actor("pipeline_actor");
        container.config = Value::Object(
            circular_core::ObjectValue::try_from_entries([("size".to_owned(), Value::Int(3))])
                .expect("one key"),
        );

        let mut candidate = EpochCandidate::open(target.clone()).expect("opens");
        candidate.add_actor(key(target.clone(), "cell"), container);
        candidate.add_actor(key(inner.clone(), "relay"), actor("tap"));
        candidate.add_scope(
            inner,
            ScopeDeclaration {
                role: wire::ScopeRole::Concrete,
                boundary: wire::ScopeBoundary {
                    inlets: Vec::new(),
                    outlets: Vec::new(),
                },
            },
        );

        let plan = candidate.assemble().expect("the projection stands");
        let refused = plan
            .refused()
            .values()
            .find(|refused| refused.key() == &key(target.clone(), "cell"))
            .expect("a container that carries config does not pass silently");
        let reason = refused.rejection();
        assert!(
            reason.to_string().contains("config"),
            "the reason must name config: {reason}"
        );
    }

    fn presentation() -> wire::Presentation<PlanActorKey> {
        wire::Presentation {
            label: None,
            group: None,
            anchor: None,
            fixed: None,
            size: None,
            board: None,
            view: None,
            collapsed: false,
        }
    }

    fn board(col: u32, row: u32, w: u32, h: u32) -> wire::BoardPlacement {
        wire::BoardPlacement::try_new(col, row, w, h).expect("a board cell has no zero span")
    }

    fn scope_declaration(role: wire::ScopeRole) -> wire::ScopeDeclaration {
        wire::ScopeDeclaration {
            role,
            boundary: wire::ScopeBoundary {
                inlets: Vec::new(),
                outlets: Vec::new(),
            },
        }
    }

    fn fold_candidate() -> (EpochCandidate, PlanActorKey, Vec<ScopeSegment>) {
        let target = vec![child("group")];
        let moving = key(Vec::new(), "moving");
        let mut candidate = EpochCandidate::open(Vec::new()).expect("root");
        candidate.add_actor(key(Vec::new(), "group"), actor("pipeline_actor"));
        candidate.add_scope(target.clone(), scope_declaration(wire::ScopeRole::Concrete));
        candidate.add_actor(key(Vec::new(), "outside"), actor("tap"));
        candidate.add_actor(moving.clone(), actor("tap"));
        candidate.add_edge(EdgeDeclaration {
            from: (key(Vec::new(), "outside"), "event".to_owned()),
            to: (moving.clone(), "event".to_owned()),
            ordinal: 7,
            attrs: wire::EdgeAttrs {
                preprocess: wire::PreprocessChain::default(),
                delay: wire::DeclaredDelay::ZERO,
                policy: wire::WirePolicy {
                    delivery: wire::Delivery::Lossless,
                    capacity: PositiveCapacity::new(4).ok(),
                },
            },
        });
        (candidate, moving, target)
    }

    fn rejection(
        result: Result<(), MoveToScopeFault>,
    ) -> circular_protocol::move_to_scope::MoveToScopeRejection {
        result.expect_err("move is rejected").rejection()
    }

    #[test]
    fn move_rewrites_all_six_reference_tables_and_preserves_generation_value() {
        let (mut candidate, moving, target) = fold_candidate();
        let second = key(Vec::new(), "second");
        candidate.add_actor(second.clone(), actor("tap"));
        candidate.add_edge(EdgeDeclaration {
            from: (moving.clone(), "event".to_owned()),
            to: (second.clone(), "event".to_owned()),
            ordinal: 3,
            attrs: wire::EdgeAttrs {
                preprocess: wire::PreprocessChain::default(),
                delay: wire::DeclaredDelay::ZERO,
                policy: wire::WirePolicy {
                    delivery: wire::Delivery::Lossless,
                    capacity: None,
                },
            },
        });
        candidate
            .tables
            .put::<t::Generations>(moving.clone(), BoundaryActorGeneration::new(11));
        candidate.add_presentation(PresentationOwner::Actor(moving.clone()), presentation());
        candidate.add_export(
            wire::PlanExportKey {
                scope: Vec::new(),
                local: "observe".to_owned(),
            },
            wire::ExportDeclaration {
                surface: None,
                roles: wire::ExportRoles {
                    request: None,
                    progress: Some((moving.clone(), "event".to_owned())),
                    result: Some((moving.clone(), "event".to_owned())),
                    error: Some((moving.clone(), "event".to_owned())),
                },
                operations: None,
            },
        );
        candidate.add_annotation(
            wire::PlanAnnotationKey {
                scope: Vec::new(),
                local: "note".to_owned(),
            },
            wire::AnnotationDeclaration {
                kind: wire::AnnotationKind::Note,
                refs: vec![moving.clone()],
                body: "moves with identity".to_owned(),
            },
        );

        candidate
            .move_to_scope(vec![moving.clone(), second.clone()], target.clone())
            .expect("folds");
        let moved = key(target.clone(), "moving");
        let moved_second = key(target, "second");

        assert!(
            candidate
                .tables()
                .actors()
                .rows()
                .iter()
                .any(|(key, _)| key == &moved)
        );
        assert!(
            !candidate
                .tables()
                .actors()
                .rows()
                .iter()
                .any(|(key, _)| key == &moving)
        );
        assert_eq!(candidate.actor_generation(&moved).get(), 11);
        assert!(
            !candidate
                .tables()
                .generations()
                .rows()
                .iter()
                .any(|(key, _)| key == &moving)
        );
        assert!(candidate.tables().edges().values().any(|edge| {
            edge.from.0 == moved && edge.to.0 == moved_second && edge.ordinal == 3
        }));
        assert!(
            candidate
                .tables()
                .presentations()
                .rows()
                .iter()
                .any(|(owner, _)| owner == &PresentationOwner::Actor(moved.clone()))
        );
        let roles = &candidate.tables().exports().rows()[0].1.roles;
        for role in [&roles.progress, &roles.result, &roles.error] {
            assert_eq!(role.as_ref().map(|(actor, _)| actor), Some(&moved));
        }
        assert_eq!(
            candidate.tables().annotations().rows()[0].1.refs,
            vec![moved]
        );
    }

    #[test]
    fn presentation_before_or_after_move_converges_on_the_same_candidate_state() {
        let (before, moving, target) = fold_candidate();
        let moved = key(target.clone(), "moving");
        let mut value = presentation();
        value.label = Some("same".to_owned());

        let mut presentation_first = before.clone();
        presentation_first
            .add_presentation(PresentationOwner::Actor(moving.clone()), value.clone());
        presentation_first
            .move_to_scope(vec![moving.clone()], target.clone())
            .expect("moves after presentation");

        let mut move_first = before;
        move_first
            .move_to_scope(vec![moving], target)
            .expect("moves before presentation");
        move_first.add_presentation(PresentationOwner::Actor(moved), value);

        assert_eq!(
            presentation_first.tables().actors().rows(),
            move_first.tables().actors().rows()
        );
        assert_eq!(
            presentation_first
                .tables()
                .edges()
                .values()
                .cloned()
                .collect::<Vec<_>>(),
            move_first
                .tables()
                .edges()
                .values()
                .cloned()
                .collect::<Vec<_>>()
        );
        assert_eq!(
            presentation_first.tables().presentations().rows(),
            move_first.tables().presentations().rows()
        );
        assert_eq!(
            presentation_first.tables().generations().rows(),
            move_first.tables().generations().rows()
        );
    }

    #[test]
    fn target_existing_boundary_port_is_byte_stable_across_an_unrelated_fold() {
        let (mut candidate, moving, target) = fold_candidate();
        let input = key(target.clone(), "existing-input");
        candidate.add_actor(
            input.clone(),
            actor_with_config(
                "input",
                Value::object([("label", Value::String("existing".to_owned()))]).unwrap(),
            ),
        );
        let before = BoundaryPortId::derive(
            BoundaryPortDirection::Inlet,
            &input,
            candidate.actor_generation(&input),
        )
        .unwrap();

        candidate
            .move_to_scope(vec![moving], target)
            .expect("unrelated actor folds");
        let after = BoundaryPortId::derive(
            BoundaryPortDirection::Inlet,
            &input,
            candidate.actor_generation(&input),
        )
        .unwrap();
        assert_eq!(before.as_port_id().as_str(), after.as_port_id().as_str());
    }

    #[test]
    fn observation_export_roles_follow_a_moved_plain_actor_into_runtime_resolution() {
        let (mut candidate, moving, target) = fold_candidate();
        candidate.add_export(
            wire::PlanExportKey {
                scope: Vec::new(),
                local: "progress".to_owned(),
            },
            wire::ExportDeclaration {
                surface: None,
                roles: wire::ExportRoles {
                    request: None,
                    progress: Some((moving.clone(), "event".to_owned())),
                    result: Some((moving.clone(), "event".to_owned())),
                    error: Some((moving.clone(), "event".to_owned())),
                },
                operations: None,
            },
        );
        candidate
            .move_to_scope(vec![moving], target)
            .expect("plain observation target folds");
        let plan = candidate.assemble().expect("moved observation plan stands");
        let export = plan
            .exports()
            .get(&ExportName::new(Name::from_normalized("progress")))
            .expect("export is assembled");
        for role in [Role::Progress, Role::Result, Role::Error] {
            let bound = export.roles().get(&role).expect("observation role");
            let resolved = crate::observation_mount_actor(&plan, bound.actor(), bound.port())
                .expect("plain actors resolve to themselves");
            assert_eq!(resolved, *bound.actor());
            assert_eq!(
                resolved.scope().segments(),
                &[circular_plan::ScopeSeg::Child(Name::from_normalized(
                    "group"
                ))]
            );
        }
    }

    fn slow_wire() -> EdgeDeclaration {
        EdgeDeclaration {
            from: (key(Vec::new(), "slow"), "event".to_owned()),
            to: (key(Vec::new(), "moving"), "event".to_owned()),
            ordinal: 1,
            attrs: wire::EdgeAttrs {
                preprocess: wire::PreprocessChain::default(),
                delay: wire::DeclaredDelay::try_new(2, 1).unwrap(),
                policy: wire::WirePolicy {
                    delivery: wire::Delivery::BestEffort {
                        on_full: wire::Shed::DropNewest,
                    },
                    capacity: PositiveCapacity::new(4).ok(),
                },
            },
        }
    }

    fn mapped_wire() -> EdgeDeclaration {
        EdgeDeclaration {
            from: (key(Vec::new(), "mapped"), "event".to_owned()),
            to: (key(Vec::new(), "moving"), "event".to_owned()),
            ordinal: 2,
            attrs: wire::EdgeAttrs {
                preprocess: wire::PreprocessChain::new(vec![wire::PreprocessStep {
                    kind: wire::PreprocessKind::Map,
                    config: preprocess_record(&[("transform", "event")]),
                }]),
                delay: wire::DeclaredDelay::ZERO,
                policy: wire::WirePolicy {
                    delivery: wire::Delivery::Lossless,
                    capacity: PositiveCapacity::new(64).ok(),
                },
            },
        }
    }

    fn fold_wire_pair(wires: [EdgeDeclaration; 2]) -> (Vec<EdgeDeclaration>, Vec<EdgeDeclaration>) {
        let target = vec![child("group")];
        let moving = key(Vec::new(), "moving");
        let mut candidate = EpochCandidate::open(Vec::new()).expect("root");
        candidate.add_actor(key(Vec::new(), "group"), actor("pipeline_actor"));
        candidate.add_scope(target.clone(), scope_declaration(wire::ScopeRole::Concrete));
        candidate.add_actor(key(Vec::new(), "slow"), actor("tap"));
        candidate.add_actor(key(Vec::new(), "mapped"), actor("tap"));
        candidate.add_actor(moving.clone(), actor("tap"));
        for wire in wires {
            candidate.add_edge(wire);
        }
        candidate
            .move_to_scope(vec![moving], target.clone())
            .expect("two wires fold into one boundary");
        candidate
            .tables()
            .edges()
            .values()
            .cloned()
            .partition(|edge| edge.to.0.scope.is_empty())
    }

    #[test]
    fn a_second_fold_synthesizes_the_next_nested_boundary_without_special_naming() {
        let (mut candidate, moving, group) = fold_candidate();
        let nested = vec![child("group"), child("nested")];
        candidate.add_actor(key(group.clone(), "nested"), actor("pipeline_actor"));
        candidate.add_scope(nested.clone(), scope_declaration(wire::ScopeRole::Concrete));
        candidate
            .move_to_scope(vec![moving], group.clone())
            .expect("first fold");
        candidate
            .move_to_scope(vec![key(group.clone(), "moving")], nested.clone())
            .expect("nested fold");

        assert!(
            candidate
                .tables()
                .actors()
                .rows()
                .iter()
                .any(|(key, declaration)| {
                    key.scope == nested
                        && key.local.as_str() == "b6_moving_event_in"
                        && declaration.actor_type == "input"
                })
        );
        candidate
            .assemble()
            .expect("nested synthesized plan stands");
    }

    #[test]
    fn move_to_scope_emits_each_closed_rejection_arm_owned_by_the_fold() {
        use circular_protocol::move_to_scope::MoveToScopeRejection as R;

        let (base, moving, target) = fold_candidate();
        let mut empty = base.clone();
        assert_eq!(
            rejection(empty.move_to_scope(Vec::new(), target.clone())),
            R::EmptyMoveSet
        );

        let mut missing = base.clone();
        assert_eq!(
            rejection(missing.move_to_scope(vec![key(Vec::new(), "missing")], target.clone())),
            R::SourceUnresolved
        );

        let mut missing_target = base.clone();
        assert_eq!(
            rejection(missing_target.move_to_scope(vec![moving.clone()], vec![child("missing")])),
            R::TargetScopeUnresolved
        );

        let mut template = base.clone();
        let (scope, mut declaration) = template.tables().scopes().rows()[0].clone();
        declaration.role = wire::ScopeRole::Template;
        template.add_scope(scope, declaration);
        assert_eq!(
            rejection(template.move_to_scope(vec![moving.clone()], target.clone())),
            R::TargetScopeIsTemplate
        );

        let mut collision = base.clone();
        collision.add_actor(key(target.clone(), "moving"), actor("tap"));
        assert_eq!(
            rejection(collision.move_to_scope(vec![moving.clone()], target.clone())),
            R::LocalCollisionInTarget
        );

        let mut boundary = base.clone();
        let input = key(Vec::new(), "input");
        boundary.add_actor(input.clone(), actor("input"));
        assert_eq!(
            rejection(boundary.move_to_scope(vec![input], target.clone())),
            R::BoundaryActorImmovable
        );

        let mut container = base.clone();
        assert_eq!(
            rejection(container.move_to_scope(vec![key(Vec::new(), "group")], target.clone())),
            R::WouldNestIntoSelf
        );

        let mut non_adjacent = base;
        let deep = vec![child("group"), child("deep")];
        non_adjacent.add_actor(key(target.clone(), "deep"), actor("pipeline_actor"));
        non_adjacent.add_scope(deep.clone(), scope_declaration(wire::ScopeRole::Concrete));
        assert_eq!(
            rejection(non_adjacent.move_to_scope(vec![moving], deep)),
            R::BoundarySynthesisRefused
        );
    }

    #[test]
    fn what_the_plan_can_hold_is_actually_held() {
        let target = vec![child("fleet")];
        let mut candidate = EpochCandidate::open(target.clone()).expect("opens");
        candidate.add_actor(key(target.clone(), "meter"), actor("tap"));

        let mut value = presentation();
        value.label = Some("✓ tokens per second".to_owned());
        value.collapsed = true;
        value.fixed = Some(wire::LayoutPoint::new(
            wire::LayoutCoord::new(12),
            wire::LayoutCoord::new(-3),
        ));
        value.size = Some(wire::LayoutSize { w: 0, h: 24 });
        value.board = Some(board(4, 5, 2, 3));
        value.view = Some(wire::ViewSpec {
            kind: "meter".to_owned(),
            config: Value::Null,
        });
        candidate.add_presentation(PresentationOwner::Actor(key(target, "meter")), value);

        let plan = candidate.assemble().expect("the plan builds");
        let (_, held) = plan
            .presentation()
            .iter()
            .next()
            .expect("one presentation field is carried");
        assert_eq!(held.label(), Some("✓ tokens per second"));
        assert!(held.collapsed(), "collapsed was not carried");
        assert!(held.fixed().is_some(), "fixed was not carried");
        assert_eq!(
            held.size(),
            Some(wire::LayoutSize::new(0, 24)),
            "size was not carried"
        );
        assert_eq!(
            held.board(),
            Some(wire::BoardPlacement::try_new(4, 5, 2, 3).expect("valid cell")),
            "board was not carried"
        );
        assert!(held.view().is_some(), "view kind was not carried");
    }

    #[test]
    fn board_set_presentation_round_trips_through_the_plan_losslessly() {
        let target = vec![child("fleet")];
        let mut candidate = EpochCandidate::open(target.clone()).expect("opens");
        candidate.add_actor(key(target.clone(), "meter"), actor("tap"));
        let expected = board(7, 11, 13, 17);
        let mut value = presentation();
        value.size = Some(wire::LayoutSize { w: 320, h: 90 });
        value.board = Some(expected);
        value.view = Some(wire::ViewSpec {
            kind: "meter".to_owned(),
            config: Value::Null,
        });
        candidate.add_presentation(PresentationOwner::Actor(key(target, "meter")), value);

        let plan = candidate.assemble().expect("board presentation assembles");
        let (_, held) = plan
            .presentation()
            .iter()
            .next()
            .expect("presentation lands in plan");
        let projected = crate::authoring_assembly::ledger::presentation_declaration_from_plan(held)
            .expect("plan presentation projects back to the declaration carrier");
        assert_eq!(projected.board, Some(expected));
        assert_eq!(projected.size, Some(wire::LayoutSize { w: 320, h: 90 }));
        assert_eq!(
            projected.view.expect("view survives").config,
            Value::Null,
            "view.config remains the name-only null carrier"
        );
    }

    #[test]
    fn overlapping_board_cells_are_refused_by_assembly() {
        let target = vec![child("fleet")];
        let mut candidate = EpochCandidate::open(target.clone()).expect("opens");
        for (local, cell) in [("alpha", board(0, 1, 3, 2)), ("beta", board(2, 2, 4, 5))] {
            candidate.add_actor(key(target.clone(), local), actor("tap"));
            let mut value = presentation();
            value.board = Some(cell);
            candidate.add_presentation(PresentationOwner::Actor(key(target.clone(), local)), value);
        }

        let rejection = candidate
            .assemble()
            .expect_err("same-scope overlapping board cells must fail assembly");
        assert_eq!(
            rejection,
            FoldRejection::BoardOverlap {
                left: "alpha".to_owned(),
                left_board: board(0, 1, 3, 2),
                right: "beta".to_owned(),
                right_board: board(2, 2, 4, 5),
            }
        );
        assert_eq!(
            rejection.to_string(),
            "board placements overlap in the same scope: actor `alpha` at (col 0, row 1, 3×2) and actor `beta` at (col 2, row 2, 4×5)"
        );
    }

    #[test]
    fn board_cells_that_only_share_an_edge_pass_assembly() {
        let target = vec![child("fleet")];
        let mut candidate = EpochCandidate::open(target.clone()).expect("opens");
        for (local, cell) in [("left", board(0, 0, 2, 3)), ("right", board(2, 0, 2, 3))] {
            candidate.add_actor(key(target.clone(), local), actor("tap"));
            let mut value = presentation();
            value.board = Some(cell);
            candidate.add_presentation(PresentationOwner::Actor(key(target.clone(), local)), value);
        }

        let plan = candidate
            .assemble()
            .expect("half-open board cells may share an edge");
        assert_eq!(plan.presentation().len(), 2);
    }

    fn binding(
        scope: Vec<ScopeSegment>,
        local: &str,
        inner: &str,
        outer: &str,
    ) -> wire::ScopeBinding {
        wire::ScopeBinding {
            inner: (key(scope, local), inner.to_owned()),
            outer: outer.to_owned(),
        }
    }

    #[test]
    fn boundary_port_id_failure_names_its_own_cause() {
        let target = vec![child("fleet")];
        let inner = vec![child("fleet"), child("cell")];

        let mut candidate = EpochCandidate::open(target.clone()).expect("opens");
        candidate.add_actor(key(target, "cell"), actor("pipeline_actor"));
        candidate.add_actor(
            key(inner.clone(), ""),
            actor_with_config(
                "input",
                Value::object([("label", Value::string("request"))]).unwrap(),
            ),
        );
        candidate.add_scope(
            inner.clone(),
            ScopeDeclaration {
                role: wire::ScopeRole::Concrete,
                boundary: wire::ScopeBoundary {
                    inlets: Vec::new(),
                    outlets: Vec::new(),
                },
            },
        );

        let reason = candidate
            .assemble()
            .expect_err("no boundary port name is derived for an empty local");
        assert_eq!(
            reason,
            FoldRejection::Boundary(BoundaryFault::PortIds {
                scope: inner,
                error: circular_protocol::boundary_port::BoundaryPortIdError::EmptyActorLocal,
            })
        );
        assert_eq!(
            reason.to_string(),
            "boundary port ids of scope fleet/cell cannot be derived from its input and output actors: boundary actor local identity is empty"
        );
        assert_eq!(
            reason.at(),
            Some(Value::string("boundary_disagrees_with_derivation"))
        );
        assert_eq!(
            reason.reason(),
            circular_protocol::rejection_code::RejectionReason::Malformed
        );
    }
}
