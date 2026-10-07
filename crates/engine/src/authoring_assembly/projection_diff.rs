
use super::projection::{AuthoredProjection, fold_projection};
use circular_plan::{
    ActorDecl, ActorFlags, Annotation, AnnotationId, DeclaredEdgeId, DeclaredScopeSeg, EdgeAttrs,
    EdgeDecl, Export, ExportName, NamedActorId, Presentation, ScopeDeclaration,
};
use circular_protocol::declaration_payload::PresentationOwner;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActorChange {
    Added(ActorDecl),
    Removed,
    Replaced(ActorDecl),
    ConfigTransition {
        before: circular_plan::Config,
        after: circular_plan::Config,
    },
    FlagsUpdated {
        before: ActorFlags,
        after: ActorFlags,
    },
}

use self::ActorChange::{Added, ConfigTransition, FlagsUpdated, Removed, Replaced};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EdgeChange {
    Added(EdgeDecl),
    Removed,
    AttrsUpdated { before: EdgeAttrs, after: EdgeAttrs },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ScopeChange {
    Added(crate::authoring_assembly::projection::AuthoredProjection),
    Removed,
    Changed(Patch),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExportChange {
    Set(Export),
    Remove,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnnotationChange {
    Set(Annotation),
    Remove,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PresentationChange {
    Set(Presentation),
    Remove,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Patch {
    pub(crate) templates:
        Option<std::collections::BTreeMap<circular_plan::Name, circular_plan::Template>>,
    pub(crate) declaration: Option<Box<ScopeDeclaration>>,
    pub(crate) actors: Vec<(NamedActorId, ActorChange)>,
    pub(crate) edges: Vec<(DeclaredEdgeId, EdgeChange)>,
    pub(crate) scopes: Vec<(DeclaredScopeSeg, ScopeChange)>,
    pub(crate) exports: Vec<(ExportName, ExportChange)>,
    pub(crate) annotations: Vec<(AnnotationId, AnnotationChange)>,
    pub(crate) presentation: Vec<(
        PresentationOwner<NamedActorId, circular_plan::AnnotationId>,
        PresentationChange,
    )>,
}

impl Patch {
    pub fn templates_changed(&self) -> bool {
        self.templates.is_some()
    }
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            templates: None,
            declaration: None,
            actors: Vec::new(),
            edges: Vec::new(),
            scopes: Vec::new(),
            exports: Vec::new(),
            annotations: Vec::new(),
            presentation: Vec::new(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.templates.is_none()
            && self.declaration.is_none()
            && self.actors.is_empty()
            && self.edges.is_empty()
            && self.scopes.is_empty()
            && self.exports.is_empty()
            && self.annotations.is_empty()
            && self.presentation.is_empty()
    }

    #[must_use]
    pub fn graph_is_empty(&self) -> bool {
        let mut pending = vec![self];
        while let Some(patch) = pending.pop() {
            if patch.templates.is_some()
                || patch.declaration.is_some()
                || !patch.actors.is_empty()
                || !patch.edges.is_empty()
            {
                return false;
            }
            for (_, change) in &patch.scopes {
                match change {
                    ScopeChange::Added(_) | ScopeChange::Removed => return false,
                    ScopeChange::Changed(child) => pending.push(child),
                }
            }
        }
        true
    }

    #[must_use]
    pub fn graph_change_count(&self) -> usize {
        let mut count = 0;
        let mut pending = vec![self];
        while let Some(patch) = pending.pop() {
            count += usize::from(patch.templates.is_some())
                + usize::from(patch.declaration.is_some())
                + patch.actors.len()
                + patch.edges.len();
            for (_, change) in &patch.scopes {
                match change {
                    ScopeChange::Added(_) | ScopeChange::Removed => count += 1,
                    ScopeChange::Changed(child) => pending.push(child),
                }
            }
        }
        count
    }

    #[must_use]
    pub fn declaration(&self) -> Option<&ScopeDeclaration> {
        self.declaration.as_deref()
    }

    #[must_use]
    pub fn actors(&self) -> &[(NamedActorId, ActorChange)] {
        &self.actors
    }

    #[must_use]
    pub fn edges(&self) -> &[(DeclaredEdgeId, EdgeChange)] {
        &self.edges
    }

    #[must_use]
    pub fn scopes(&self) -> &[(DeclaredScopeSeg, ScopeChange)] {
        &self.scopes
    }

    #[must_use]
    pub fn exports(&self) -> &[(ExportName, ExportChange)] {
        &self.exports
    }

    #[must_use]
    pub fn annotations(&self) -> &[(AnnotationId, AnnotationChange)] {
        &self.annotations
    }

    #[must_use]
    pub fn presentation(
        &self,
    ) -> &[(
        PresentationOwner<NamedActorId, circular_plan::AnnotationId>,
        PresentationChange,
    )] {
        &self.presentation
    }
}

struct PendingPatch {
    patch: Patch,
    parent: Option<(usize, DeclaredScopeSeg)>,
}

#[must_use]
pub fn diff(
    left: &crate::authoring_assembly::projection::AuthoredProjection,
    right: &crate::authoring_assembly::projection::AuthoredProjection,
) -> Patch {
    let mut work = vec![(left, right, None)];
    let mut pending = Vec::<PendingPatch>::new();

    while let Some((left, right, parent)) = work.pop() {
        let index = pending.len();
        let mut patch = Patch::empty();
        if left.graph.declaration != right.graph.declaration {
            patch.declaration = Some(Box::new(right.graph.declaration.clone()));
        }
        diff_actors(left, right, &mut patch);
        if left.templates != right.templates {
            patch.templates = Some(right.templates.clone());
        }
        diff_edges(left, right, &mut patch);
        diff_values(left, right, &mut patch);

        let keys = left
            .graph
            .scopes
            .keys()
            .chain(right.graph.scopes.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        for key in &keys {
            match (left.graph.scopes.get(key), right.graph.scopes.get(key)) {
                (Some(_), None) => patch.scopes.push((key.clone(), ScopeChange::Removed)),
                (None, Some(added)) => patch
                    .scopes
                    .push((key.clone(), ScopeChange::Added(added.clone()))),
                (Some(_), Some(_)) | (None, None) => {}
            }
        }

        pending.push(PendingPatch { patch, parent });
        for key in keys.into_iter().rev() {
            if let (Some(left_child), Some(right_child)) =
                (left.graph.scopes.get(&key), right.graph.scopes.get(&key))
            {
                work.push((left_child, right_child, Some((index, key))));
            }
        }
    }

    for index in (1..pending.len()).rev() {
        let parent = pending[index]
            .parent
            .clone()
            .expect("a non-root patch has a parent");
        let child = std::mem::take(&mut pending[index].patch);
        if !child.is_empty() {
            pending[parent.0]
                .patch
                .scopes
                .push((parent.1, ScopeChange::Changed(child)));
        }
    }
    for item in &mut pending {
        item.patch
            .scopes
            .sort_by(|left, right| left.0.cmp(&right.0));
    }
    pending
        .into_iter()
        .next()
        .expect("diff yields a root patch")
        .patch
}

fn diff_actors(
    left: &crate::authoring_assembly::projection::AuthoredProjection,
    right: &crate::authoring_assembly::projection::AuthoredProjection,
    patch: &mut Patch,
) {
    let keys = left
        .graph
        .actors
        .keys()
        .chain(right.graph.actors.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for key in keys {
        match (left.graph.actors.get(&key), right.graph.actors.get(&key)) {
            (None, Some(value)) => patch.actors.push((key, Added(value.clone()))),
            (Some(_), None) => patch.actors.push((key, Removed)),
            (Some(before), Some(after)) => {
                if before.domain().actor_type() != after.domain().actor_type()
                    || before.authored_generation() != after.authored_generation()
                {
                    patch.actors.push((key, Replaced(after.clone())));
                    continue;
                }
                if before.domain().config() != after.domain().config() {
                    patch.actors.push((
                        key.clone(),
                        ConfigTransition {
                            before: before.domain().config().clone(),
                            after: after.domain().config().clone(),
                        },
                    ));
                }
                if before.flags() != after.flags() {
                    patch.actors.push((
                        key,
                        FlagsUpdated {
                            before: before.flags(),
                            after: after.flags(),
                        },
                    ));
                }
            }
            (None, None) => unreachable!("keys of the union"),
        }
    }
}

fn diff_edges(
    left: &crate::authoring_assembly::projection::AuthoredProjection,
    right: &crate::authoring_assembly::projection::AuthoredProjection,
    patch: &mut Patch,
) {
    let keys = left
        .graph
        .edges
        .keys()
        .chain(right.graph.edges.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for key in keys {
        match (left.graph.edges.get(&key), right.graph.edges.get(&key)) {
            (None, Some(value)) => patch.edges.push((key, EdgeChange::Added(value.clone()))),
            (Some(_), None) => patch.edges.push((key, EdgeChange::Removed)),
            (Some(before), Some(after)) if before.attrs() != after.attrs() => patch.edges.push((
                key,
                EdgeChange::AttrsUpdated {
                    before: before.attrs(),
                    after: after.attrs(),
                },
            )),
            (Some(_), Some(_)) => {}
            (None, None) => unreachable!("keys of the union"),
        }
    }
}

fn diff_values(
    left: &crate::authoring_assembly::projection::AuthoredProjection,
    right: &crate::authoring_assembly::projection::AuthoredProjection,
    patch: &mut Patch,
) {
    patch.exports = map_changes(&left.exports, &right.exports)
        .into_iter()
        .map(|(key, value)| (key, value.map_or(ExportChange::Remove, ExportChange::Set)))
        .collect();
    patch.annotations = map_changes(&left.annotations, &right.annotations)
        .into_iter()
        .map(|(key, value)| {
            (
                key,
                value.map_or(AnnotationChange::Remove, AnnotationChange::Set),
            )
        })
        .collect();
    patch.presentation = map_changes(&left.presentation, &right.presentation)
        .into_iter()
        .map(|(key, value)| {
            (
                key,
                value.map_or(PresentationChange::Remove, PresentationChange::Set),
            )
        })
        .collect();
}

fn map_changes<K, V>(
    left: &std::collections::BTreeMap<K, V>,
    right: &std::collections::BTreeMap<K, V>,
) -> Vec<(K, Option<V>)>
where
    K: Clone + Ord,
    V: Clone + PartialEq,
{
    let mut changes = Vec::new();
    let keys = left
        .keys()
        .chain(right.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for key in keys {
        match (left.get(&key), right.get(&key)) {
            (None, Some(value)) => changes.push((key, Some(value.clone()))),
            (Some(_), None) => changes.push((key, None)),
            (Some(before), Some(after)) if before != after => {
                changes.push((key, Some(after.clone())));
            }
            (Some(_), Some(_)) => {}
            (None, None) => unreachable!("keys of the union"),
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring_assembly::projection::AuthoredProjectionBuilder;
    use circular_core::Ticks;
    use circular_plan::{
        ActorDomain, ActorFlags, ActorType, Config, ConfigValue, Delivery, Endpoint, Name,
        NonContainerActorDecl, PipelineActorDecl, PortId, ScopeBoundary, ScopeRole, Shed,
        WirePolicy,
    };

    fn name(value: &str) -> Name {
        Name::from_normalized(value)
    }

    fn config(value: u8) -> Config {
        Config::try_new(vec![(
            name("value"),
            ConfigValue::Scalar {
                tag: name("u8"),
                bytes: vec![value].into_boxed_slice(),
            },
        )])
        .unwrap()
    }

    fn actor(actor_type: ActorType, value: u8, flags: ActorFlags) -> NonContainerActorDecl {
        NonContainerActorDecl::try_new(ActorDomain::new(actor_type, config(value)), flags)
            .expect("test actor type is not a container")
    }

    fn nearby(
        value: u8,
        flags: ActorFlags,
        label: &str,
    ) -> crate::authoring_assembly::projection::AuthoredProjection {
        let mut builder = AuthoredProjectionBuilder::new();
        let id = builder
            .add_actor(name("actor"), actor(ActorType::FixtureMap, value, flags))
            .unwrap();
        builder.set_presentation(
            PresentationOwner::Actor(id),
            Presentation::new(
                Some(label.to_owned()),
                None,
                None,
                None,
                None,
                None,
                None,
                false,
            ),
        );
        builder.finish().unwrap()
    }

    fn complex_base() -> crate::authoring_assembly::projection::AuthoredProjection {
        let mut builder = AuthoredProjectionBuilder::new();
        builder
            .add_actor(
                name("keep"),
                actor(ActorType::FixtureMap, 1, ActorFlags::default()),
            )
            .unwrap();
        builder
            .add_actor(
                name("removed"),
                actor(ActorType::FixtureMap, 9, ActorFlags::default()),
            )
            .unwrap();
        builder
            .enter_scope(name("nested"), PipelineActorDecl::default())
            .unwrap();
        builder
            .add_actor(
                name("nested_keep"),
                actor(ActorType::FixtureMap, 3, ActorFlags::default()),
            )
            .unwrap();
        builder
            .add_actor(
                name("nested_removed"),
                actor(ActorType::FixtureMap, 9, ActorFlags::default()),
            )
            .unwrap();
        builder.exit_scope().unwrap();
        builder.finish().unwrap()
    }

    fn complex_candidate() -> crate::authoring_assembly::projection::AuthoredProjection {
        let mut builder = AuthoredProjectionBuilder::new();
        builder
            .add_actor(
                name("added"),
                actor(ActorType::FixtureMap, 5, ActorFlags::default()),
            )
            .unwrap();
        builder
            .add_actor(
                name("keep"),
                actor(ActorType::FixtureMap, 2, ActorFlags::default()),
            )
            .unwrap();
        builder
            .enter_scope(name("nested"), PipelineActorDecl::default())
            .unwrap();
        builder
            .add_actor(
                name("nested_added"),
                actor(ActorType::FixtureMap, 6, ActorFlags::default()),
            )
            .unwrap();
        builder
            .add_actor(
                name("nested_keep"),
                actor(ActorType::FixtureMap, 4, ActorFlags::default()),
            )
            .unwrap();
        builder.exit_scope().unwrap();
        builder.finish().unwrap()
    }

    #[test]
    fn an_unchanged_plan_diffs_empty_and_config_precedes_flags() {
        let left = nearby(1, ActorFlags::default(), "before");
        let right = nearby(2, ActorFlags::new(true, false, true), "after");
        let patch = diff(&left, &right);
        assert!(diff(&left, &left).is_empty());
        assert!(matches!(patch.actors()[0].1, ConfigTransition { .. }));
        assert!(matches!(patch.actors()[1].1, FlagsUpdated { .. }));
    }

    #[test]
    fn a_complex_edit_carries_added_removed_and_config_at_both_levels() {
        let base = complex_base();
        let candidate = complex_candidate();
        let edit = diff(&base, &candidate);

        assert!(
            edit.actors()
                .iter()
                .any(|(_, change)| matches!(change, Added(_)))
        );
        assert!(
            edit.actors()
                .iter()
                .any(|(_, change)| matches!(change, Removed))
        );
        assert!(
            edit.actors()
                .iter()
                .any(|(_, change)| matches!(change, ConfigTransition { .. }))
        );

        let nested = edit
            .scopes()
            .iter()
            .find_map(|(_, change)| match change {
                ScopeChange::Changed(child) => Some(child),
                ScopeChange::Added(_) | ScopeChange::Removed => None,
            })
            .expect("there must be a nested scope change");
        assert!(
            nested
                .actors()
                .iter()
                .any(|(_, change)| matches!(change, Added(_)))
        );
        assert!(
            nested
                .actors()
                .iter()
                .any(|(_, change)| matches!(change, Removed))
        );
        assert!(
            nested
                .actors()
                .iter()
                .any(|(_, change)| matches!(change, ConfigTransition { .. }))
        );
    }

    #[test]
    fn view_only_diff_has_no_execution_change() {
        let left = nearby(1, ActorFlags::default(), "before");
        let right = nearby(1, ActorFlags::default(), "after");
        let patch = diff(&left, &right);
        assert!(patch.graph_is_empty());
    }

    #[test]
    fn edge_attrs_preserve_id() {
        let mut left_builder = AuthoredProjectionBuilder::new();
        let from = left_builder
            .add_actor(
                name("from"),
                actor(ActorType::FixtureMap, 1, ActorFlags::default()),
            )
            .unwrap();
        let to = left_builder
            .add_actor(
                name("to"),
                actor(ActorType::FixtureMap, 1, ActorFlags::default()),
            )
            .unwrap();
        let from =
            circular_plan::Endpoint::new(from, circular_plan::PortId::try_new("out").unwrap());
        let to = circular_plan::Endpoint::new(to, circular_plan::PortId::try_new("in").unwrap());
        let policy = WirePolicy::new(
            Delivery::BestEffort {
                on_full: Shed::DropNewest,
            },
            None,
        );
        left_builder
            .add_edge(
                from.clone(),
                to.clone(),
                0,
                EdgeAttrs::new(Ticks::ZERO, policy),
            )
            .unwrap();
        let left = left_builder.finish().unwrap();

        let mut right_builder = AuthoredProjectionBuilder::new();
        let right_from = right_builder
            .add_actor(
                name("from"),
                actor(ActorType::FixtureMap, 1, ActorFlags::default()),
            )
            .unwrap();
        let right_to = right_builder
            .add_actor(
                name("to"),
                actor(ActorType::FixtureMap, 1, ActorFlags::default()),
            )
            .unwrap();
        right_builder
            .add_edge(
                circular_plan::Endpoint::new(
                    right_from,
                    circular_plan::PortId::try_new("out").unwrap(),
                ),
                circular_plan::Endpoint::new(
                    right_to,
                    circular_plan::PortId::try_new("in").unwrap(),
                ),
                0,
                EdgeAttrs::new(Ticks::new(3), policy),
            )
            .unwrap();
        let right = right_builder.finish().unwrap();

        let patch = diff(&left, &right);
        assert!(matches!(
            patch.edges()[0].1,
            EdgeChange::AttrsUpdated { .. }
        ));
    }

    #[test]
    fn preprocess_only_update_preserves_edge_identity() {
        let make = |kind| {
            let mut builder = AuthoredProjectionBuilder::new();
            let from = builder
                .add_actor(
                    name("from"),
                    actor(ActorType::FixtureMap, 1, ActorFlags::default()),
                )
                .unwrap();
            let to = builder
                .add_actor(
                    name("to"),
                    actor(ActorType::FixtureMap, 1, ActorFlags::default()),
                )
                .unwrap();
            builder
                .add_edge(
                    circular_plan::Endpoint::new(
                        from,
                        circular_plan::PortId::try_new("out").unwrap(),
                    ),
                    circular_plan::Endpoint::new(to, circular_plan::PortId::try_new("in").unwrap()),
                    0,
                    EdgeAttrs::new(Ticks::ZERO, WirePolicy::new(Delivery::Lossless, None))
                        .with_preprocess(circular_plan::PreprocessChain::new(vec![
                            circular_plan::PreprocessStep::new(kind, Config::default()),
                        ])),
                )
                .unwrap();
            builder.finish().unwrap()
        };
        let before = make(circular_plan::PreprocessKind::Bang);
        let after = make(circular_plan::PreprocessKind::Map);
        let patch = diff(&before, &after);
        assert_eq!(patch.edges().len(), 1);
        assert!(matches!(
            patch.edges()[0].1,
            EdgeChange::AttrsUpdated { .. }
        ));
        assert_eq!(
            before.graph().edges().keys().collect::<Vec<_>>(),
            after.graph().edges().keys().collect::<Vec<_>>()
        );
    }

    fn scope_plan(
        role: ScopeRole,
        outer: Option<&str>,
    ) -> crate::authoring_assembly::projection::AuthoredProjection {
        let container: circular_plan::ContainerActorDecl = match role {
            ScopeRole::Concrete => PipelineActorDecl::default().into(),
            ScopeRole::Template => circular_plan::ContainerActorDecl::replicator(
                Config::default(),
                ActorFlags::default(),
                0,
            ),
        };
        let mut builder = AuthoredProjectionBuilder::new();
        builder
            .enter_scope_with_container(name("cell"), container)
            .unwrap();
        let inside = builder
            .add_actor(
                name("inside"),
                actor(ActorType::FixtureMap, 0, ActorFlags::default()),
            )
            .unwrap();
        if let Some(outer) = outer {
            builder
                .set_boundary(ScopeBoundary::new(
                    [(
                        PortId::try_new(outer).unwrap(),
                        Endpoint::new(inside, PortId::try_new("in").unwrap()),
                    )],
                    [],
                ))
                .unwrap();
        }
        builder.exit_scope().unwrap();
        builder.finish().unwrap()
    }

    #[test]
    fn a_boundary_change_is_a_graph_change() {
        let sealed = scope_plan(ScopeRole::Template, None);
        let open = scope_plan(ScopeRole::Template, Some("event"));
        let renamed = scope_plan(ScopeRole::Template, Some("events"));

        for (before, after) in [(&sealed, &open), (&open, &renamed)] {
            let patch = diff(before, after);
            assert!(!patch.graph_is_empty());
            assert_eq!(patch.graph_change_count(), 1);
        }
        assert_eq!(diff(&open, &open), Patch::empty());
    }
}
