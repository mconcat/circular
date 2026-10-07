
use crate::authoring_assembly::projection::AuthoredProjection;
use crate::authoring_assembly::projection::fold_projection;
use crate::authoring_assembly::projection_diff::ActorChange;
use crate::authoring_assembly::projection_diff::EdgeChange;
use crate::authoring_assembly::projection_diff::Patch;
use crate::authoring_assembly::projection_diff::ScopeChange;
use crate::authoring_assembly::projection_diff::diff;
use circular_plan::{ActorDecl, DeclaredScopeSeg, EdgeId, NamedActorId, ScopeId, ScopeSeg};
use circular_runtime::CheckpointRestore;
use std::collections::{BTreeMap, BTreeSet};

fn valid_boundary_label(
    actor_type: circular_plan::ActorType,
    config: &circular_plan::Config,
) -> bool {
    let [(name, circular_plan::ConfigValue::Scalar { .. })] = config.record().entries() else {
        return false;
    };
    if name.as_str() != "label" {
        return false;
    }
    let Ok(folded) = crate::fold_config(actor_type, config) else {
        return false;
    };
    circular_actors::registered_create_inputs(actor_type)
        .is_ok_and(|schema| schema.admit(folded.value()).is_ok())
}

/// Whether this exact edit can retain every live actor and runtime cache.
///
/// Presentation, annotations, and unchanged plans are eligible. The only config
/// exception is the registered label-only Input/Output declaration: its boundary
/// identity and fixed flow do not depend on the label. This is not the broader
/// `Patch::graph_is_empty` classification, which also admits export changes.
#[must_use]
pub fn metadata_only_plan_change(before: &AuthoredProjection, after: &AuthoredProjection) -> bool {
    let before_actors = declared_actor_map(before);
    let after_actors = declared_actor_map(after);
    let patch = diff(before, after);
    let mut pending = vec![&patch];
    while let Some(patch) = pending.pop() {
        if patch.templates_changed()
            || patch.declaration().is_some()
            || !patch.edges().is_empty()
            || !patch.exports().is_empty()
        {
            return false;
        }
        for (actor, change) in patch.actors() {
            if matches!(change, ActorChange::FlagsUpdated { .. }) {
                continue;
            }
            if !matches!(change, ActorChange::ConfigTransition { .. }) {
                return false;
            }
            let (Some(before_actor), Some(after_actor)) =
                (before_actors.get(actor), after_actors.get(actor))
            else {
                return false;
            };
            let actor_type = *before_actor.domain().actor_type();
            if !matches!(
                actor_type,
                circular_plan::ActorType::Input | circular_plan::ActorType::Output
            ) || after_actor.domain().actor_type() != &actor_type
                || before_actor.flags() != after_actor.flags()
                || before_actor.authored_generation() != after_actor.authored_generation()
                || !valid_boundary_label(actor_type, before_actor.domain().config())
                || !valid_boundary_label(actor_type, after_actor.domain().config())
            {
                return false;
            }
        }
        for (_, change) in patch.scopes() {
            match change {
                ScopeChange::Changed(child) => pending.push(child),
                ScopeChange::Added(_) | ScopeChange::Removed => return false,
            }
        }
    }
    true
}

/// A semantic wire-program edit with identical actors, endpoints, exports,
/// delays and delivery policies. It must never be classified as metadata.
#[must_use]
pub fn preprocess_only_plan_change(
    before: &AuthoredProjection,
    after: &AuthoredProjection,
) -> bool {
    let patch = diff(before, after);
    let mut pending = vec![&patch];
    let mut changed = false;
    while let Some(patch) = pending.pop() {
        if patch.templates_changed()
            || patch.declaration().is_some()
            || !patch.actors().is_empty()
            || !patch.exports().is_empty()
        {
            return false;
        }
        for (_, change) in patch.edges() {
            match change {
                EdgeChange::AttrsUpdated { before, after }
                    if before.delay() == after.delay() && before.policy() == after.policy() =>
                {
                    changed = true
                }
                _ => return false,
            }
        }
        for (_, change) in patch.scopes() {
            match change {
                ScopeChange::Changed(child) => pending.push(child),
                _ => return false,
            }
        }
    }
    changed
}

#[derive(Clone, Debug)]
pub struct RunReconciliation<V, I> {
    pub(crate) added: Vec<(NamedActorId, u64)>,
    pub(crate) absorbed: Vec<NamedActorId>,
    pub(crate) restarted: Vec<(NamedActorId, u64, CheckpointRestore<V, I>)>,
    pub(crate) retired: Vec<NamedActorId>,
    pub(crate) rewired: Vec<EdgeId>,
}

impl<V, I> Default for RunReconciliation<V, I> {
    fn default() -> Self {
        Self::empty()
    }
}

impl<V, I> RunReconciliation<V, I> {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            added: Vec::new(),
            absorbed: Vec::new(),
            restarted: Vec::new(),
            retired: Vec::new(),
            rewired: Vec::new(),
        }
    }

    #[must_use]
    pub fn added(&self) -> &[(NamedActorId, u64)] {
        &self.added
    }

    #[must_use]
    pub fn absorbed(&self) -> &[NamedActorId] {
        &self.absorbed
    }

    #[must_use]
    pub fn restarted(&self) -> &[(NamedActorId, u64, CheckpointRestore<V, I>)] {
        &self.restarted
    }

    #[must_use]
    pub fn retired(&self) -> &[NamedActorId] {
        &self.retired
    }

    #[must_use]
    pub fn rewired(&self) -> &[EdgeId] {
        &self.rewired
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.absorbed.is_empty()
            && self.restarted.is_empty()
            && self.retired.is_empty()
            && self.rewired.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct RevisionReconciliation {
    config_restarts: BTreeSet<NamedActorId>,
    lifecycle_activations: Vec<NamedActorId>,
    metadata_only: bool,
    preprocess_only: bool,
}

impl RevisionReconciliation {
    #[must_use]
    pub fn between(before: &AuthoredProjection, after: &AuthoredProjection) -> Self {
        let (actors, _) = plan_changes(before, after);
        let config_restarts = actors
            .iter()
            .filter(|(actor, change)| {
                matches!(change, FlatActorChange::Config) && config_always_restarts(after, actor)
            })
            .map(|(actor, _)| actor.clone())
            .collect();
        Self {
            lifecycle_activations: lifecycle_activation_actors(before, after),
            metadata_only: metadata_only_plan_change(before, after),
            preprocess_only: preprocess_only_plan_change(before, after),
            config_restarts,
        }
    }

    #[must_use]
    pub fn none() -> Self {
        Self {
            config_restarts: BTreeSet::new(),
            lifecycle_activations: Vec::new(),
            metadata_only: false,
            preprocess_only: false,
        }
    }

    pub(crate) fn config_restart_is_available(&self, actor: &NamedActorId) -> bool {
        self.config_restarts.contains(actor)
    }

    #[must_use]
    pub fn lifecycle_activations(&self) -> &[NamedActorId] {
        &self.lifecycle_activations
    }

    #[must_use]
    pub const fn is_metadata_only(&self) -> bool {
        self.metadata_only
    }

    #[must_use]
    pub const fn is_preprocess_only(&self) -> bool {
        self.preprocess_only
    }
}

pub(crate) fn lifecycle_activation_actors(
    before: &AuthoredProjection,
    after: &AuthoredProjection,
) -> Vec<NamedActorId> {
    plan_changes(before, after)
        .0
        .into_iter()
        .filter_map(|(actor, change)| {
            let needs_activation = (matches!(change, FlatActorChange::Config)
                && config_always_restarts(after, &actor))
                || matches!(
                    &change,
                    FlatActorChange::Added(declaration) if declaration.authored_generation() > 0
                )
                || after_declaration(after, &actor).is_some_and(|decl| {
                    match decl.domain().actor_type() {
                        circular_plan::ActorType::Peer => matches!(
                            change, FlatActorChange::Added(_) | FlatActorChange::Config
                        ),
                        circular_plan::ActorType::Timer
                        | circular_plan::ActorType::Json
                        | circular_plan::ActorType::Listener
                        | circular_plan::ActorType::Otlp => matches!(
                            change,
                            FlatActorChange::Added(_)
                                | FlatActorChange::Replaced(_)
                                | FlatActorChange::Config
                        ),
                        _ => false,
                    }
                });
            needs_activation.then_some(actor)
        })
        .collect()
}

/// A constant factory disposition is sufficient to prepare a fresh actor without
/// invoking or borrowing the old actor. Conditional absorption is not inferred here.
pub(crate) fn config_always_restarts(after: &AuthoredProjection, actor: &NamedActorId) -> bool {
    after_declaration(after, actor)
        .and_then(|declaration| {
            circular_actors::editability::editability(*declaration.domain().actor_type())
        })
        .is_some_and(|row| {
            !row.absorbs_some_changes
                && row.outcome == circular_runtime::ConfigChangeOutcome::ReplaceIncarnation
        })
}

fn after_declaration(after: &AuthoredProjection, actor: &NamedActorId) -> Option<ActorDecl> {
    fold_projection(after, |layer| {
        layer.actors().get(actor).cloned().or_else(|| {
            layer
                .into_scopes()
                .into_values()
                .find(Option::is_some)
                .flatten()
        })
    })
}

#[derive(Clone, Debug)]
pub(crate) enum FlatActorChange {
    Added(ActorDecl),
    Removed,
    Replaced(ActorDecl),
    Config,
    Absorbed,
}

/// Shared change classification for live reconciliation and recorded replay.
pub(crate) fn plan_changes(
    before: &AuthoredProjection,
    after: &AuthoredProjection,
) -> (Vec<(NamedActorId, FlatActorChange)>, Vec<EdgeId>) {
    let mut actors = Vec::new();
    let mut edges = Vec::new();
    collect_changes(
        &diff(before, after),
        before,
        &ScopeId::root(),
        &mut actors,
        &mut edges,
    );
    let prototypes = |plan: &AuthoredProjection| {
        fold_projection::<Vec<ScopeId>>(plan, |layer| {
            let mut scopes = if layer.graph().declaration().role().is_template() {
                vec![layer.scope().clone()]
            } else {
                vec![]
            };
            for child in layer.into_scopes().into_values() {
                scopes.extend(child);
            }
            scopes
        })
    };
    let roots = prototypes(before)
        .into_iter()
        .chain(prototypes(after))
        .collect::<Vec<_>>();
    actors.retain(|(actor, _)| {
        !roots
            .iter()
            .any(|root| actor.scope().segments().starts_with(root.segments()))
    });
    let pipelines = fold_projection::<Vec<ScopeId>>(after, |layer| {
        let mut roots = layer
            .actors()
            .iter()
            .filter(|(_, decl)| {
                *decl.domain().actor_type() == circular_plan::ActorType::PipelineActor
                    && decl
                        .domain()
                        .config()
                        .record()
                        .entries()
                        .iter()
                        .any(|(key, _)| key.as_str() == "template")
            })
            .map(|(actor, _)| {
                actor
                    .scope()
                    .append_segment(ScopeSeg::Child(actor.name().clone()))
                    .expect("admitted scope")
            })
            .collect::<Vec<_>>();
        for child in layer.into_scopes().into_values() {
            roots.extend(child);
        }
        roots
    });
    for (actor, change) in &mut actors {
        if let Some(declaration) = after_declaration(after, actor) {
            if declaration.domain().actor_type().is_container() {
                if *declaration.domain().actor_type() == circular_plan::ActorType::PipelineActor {
                    *change = FlatActorChange::Absorbed;
                }
            } else if matches!(change, FlatActorChange::Config)
                && pipelines
                    .iter()
                    .any(|root| actor.scope().segments().starts_with(root.segments()))
            {
                *change = FlatActorChange::Replaced(declaration.clone());
            }
        }
    }
    (actors, edges)
}

fn collect_changes(
    patch: &Patch,
    before: &AuthoredProjection,
    scope: &ScopeId,
    actors: &mut Vec<(NamedActorId, FlatActorChange)>,
    edges: &mut Vec<EdgeId>,
) {
    for (actor, change) in patch.actors() {
        let flat = match change {
            ActorChange::Added(declaration) => FlatActorChange::Added(declaration.clone()),
            ActorChange::Removed => FlatActorChange::Removed,
            ActorChange::Replaced(declaration) => FlatActorChange::Replaced(declaration.clone()),
            ActorChange::ConfigTransition {
                before: old,
                after: new,
            } if after_declaration(before, actor).is_some_and(|decl| {
                let kind = *decl.domain().actor_type();
                matches!(
                    kind,
                    circular_plan::ActorType::Input | circular_plan::ActorType::Output
                ) && valid_boundary_label(kind, old)
                    && valid_boundary_label(kind, new)
            }) =>
            {
                FlatActorChange::Absorbed
            }
            ActorChange::ConfigTransition { .. }
                if after_declaration(before, actor).is_some_and(|decl| {
                    circular_actors::editability::editability(*decl.domain().actor_type())
                        .is_some_and(|row| {
                            row.outcome == circular_runtime::ConfigChangeOutcome::Absorbed
                        })
                }) =>
            {
                FlatActorChange::Absorbed
            }
            ActorChange::ConfigTransition { .. } => FlatActorChange::Config,
            ActorChange::FlagsUpdated { .. } => FlatActorChange::Absorbed,
        };
        actors.push((actor.clone(), flat));
    }
    for (edge, change) in patch.edges() {
        match change {
            EdgeChange::Added(_) | EdgeChange::Removed | EdgeChange::AttrsUpdated { .. } => {
                edges.push(edge.as_edge_id());
            }
        }
    }
    for (segment, change) in patch.scopes() {
        let child = child_scope(scope, segment);
        match change {
            ScopeChange::Added(plan) => {
                for (actor, declaration) in declared_actors(plan) {
                    actors.push((actor, FlatActorChange::Added(declaration)));
                }
            }
            ScopeChange::Removed => {
                for (actor, _) in declared_actors_under(before, &child) {
                    actors.push((actor, FlatActorChange::Removed));
                }
            }
            ScopeChange::Changed(child_patch) => {
                collect_changes(child_patch, before, &child, actors, edges);
            }
        }
    }
}

fn child_scope(scope: &ScopeId, segment: &DeclaredScopeSeg) -> ScopeId {
    scope
        .append_segment(ScopeSeg::Child(segment.name().clone()))
        .expect("the sealed plan already checked the depth")
}

fn declared_actors(plan: &AuthoredProjection) -> Vec<(NamedActorId, ActorDecl)> {
    fold_projection(plan, |layer| {
        let mut actors = layer
            .actors()
            .iter()
            .map(|(actor, declaration)| (actor.clone(), declaration.clone()))
            .collect::<Vec<_>>();
        for mut child in layer.into_scopes().into_values() {
            actors.append(&mut child);
        }
        actors
    })
}

fn declared_actor_map(plan: &AuthoredProjection) -> BTreeMap<NamedActorId, ActorDecl> {
    declared_actors(plan).into_iter().collect()
}

fn declared_actors_under(
    plan: &AuthoredProjection,
    scope: &ScopeId,
) -> Vec<(NamedActorId, ActorDecl)> {
    declared_actors(plan)
        .into_iter()
        .filter(|(actor, _)| scope.is_ancestor_of(actor.scope()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authoring_assembly::projection::AuthoredProjectionBuilder;
    use circular_actors::ActorType;
    use circular_plan::{
        ActorDomain, ActorFlags, Config, ConfigValue, Delivery, EdgeAttrs, Endpoint, Name,
        NonContainerActorDecl, PortId, PositiveCapacity, Shed, WirePolicy,
    };

    fn boundary_label_config(value: circular_core::Value) -> Config {
        Config::try_new(vec![(
            name("label"),
            ConfigValue::Scalar {
                tag: name(crate::CANONICAL_VALUE_TAG),
                bytes: circular_core::encode(
                    &value,
                    circular_core::Ceilings::for_boundary(circular_core::Boundary::Config),
                )
                .unwrap()
                .into(),
            },
        )])
        .unwrap()
    }

    fn boundary_metadata_plan(
        actor_type: ActorType,
        config: Config,
        flags: ActorFlags,
        generation: u64,
        nested: bool,
    ) -> AuthoredProjection {
        let mut builder = AuthoredProjectionBuilder::new();
        if nested {
            builder
                .enter_scope(name("child"), circular_plan::PipelineActorDecl::default())
                .unwrap();
        }
        builder
            .add_actor(
                name("boundary"),
                NonContainerActorDecl::try_new_at_generation(
                    ActorDomain::new(actor_type, config),
                    flags,
                    generation,
                )
                .unwrap(),
            )
            .unwrap();
        if nested {
            builder.exit_scope().unwrap();
        }
        builder.finish().unwrap()
    }

    #[test]
    fn metadata_classification_preserves_both_boundary_labels_in_nested_scopes() {
        for actor_type in [ActorType::Input, ActorType::Output] {
            for nested in [false, true] {
                let plan = |label: &str| {
                    boundary_metadata_plan(
                        actor_type,
                        boundary_label_config(circular_core::Value::string(label)),
                        ActorFlags::default(),
                        7,
                        nested,
                    )
                };
                let before = plan("before");
                let after = plan("after");
                assert!(metadata_only_plan_change(&before, &before));
                assert!(metadata_only_plan_change(&before, &after));
                assert!(metadata_only_plan_change(&after, &before));
                let (changes, edges) = plan_changes(&before, &after);
                assert_eq!(changes.len(), 1);
                assert!(matches!(changes[0].1, FlatActorChange::Absorbed));
                assert!(edges.is_empty());
            }
        }
    }

    #[test]
    fn metadata_classification_allows_presentation_and_annotation_changes() {
        use circular_plan::{
            Annotation, AnnotationId, AnnotationKind, AnnotationPlacement, PipelineActorDecl,
            Presentation, Text,
        };

        let plan = |decorated| {
            let mut builder = AuthoredProjectionBuilder::new();
            builder
                .enter_scope(name("child"), PipelineActorDecl::default())
                .unwrap();
            let actor = builder
                .add_actor(name("actor"), declaration(ActorType::FixtureMap, 1))
                .unwrap();
            if decorated {
                builder.set_presentation(
                    circular_protocol::declaration_payload::PresentationOwner::Actor(actor.clone()),
                    Presentation::new(
                        Some("visible".to_owned()),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        false,
                    ),
                );
                builder.set_annotation(
                    AnnotationId::new(name("note")),
                    Annotation::new(
                        AnnotationKind::Note,
                        std::collections::BTreeSet::from([actor]),
                        AnnotationPlacement::unplaced(),
                        Text::new("annotation"),
                    ),
                );
            }
            builder.exit_scope().unwrap();
            builder.finish().unwrap()
        };
        let before = plan(false);
        let after = plan(true);
        assert!(metadata_only_plan_change(&before, &after));
        assert!(metadata_only_plan_change(&after, &before));
        assert!(metadata_only_plan_change(
            &AuthoredProjection::empty(),
            &AuthoredProjection::empty()
        ));
    }

    #[test]
    fn metadata_classification_rejects_noncanonical_boundary_configs() {
        let valid = boundary_label_config(circular_core::Value::string("valid"));
        let mut extra = valid.record().entries().to_vec();
        extra.push((name("extra"), ConfigValue::List(Box::new([]))));
        let invalid = [
            Config::default(),
            Config::try_new(extra).unwrap(),
            boundary_label_config(circular_core::Value::Int(1)),
            Config::try_new(vec![(name("label"), ConfigValue::List(Box::new([])))]).unwrap(),
            Config::try_new(vec![(
                name("label"),
                ConfigValue::Scalar {
                    tag: name("unknown"),
                    bytes: Box::new([]),
                },
            )])
            .unwrap(),
            Config::try_new(vec![(
                name("label"),
                ConfigValue::Scalar {
                    tag: name(crate::CANONICAL_VALUE_TAG),
                    bytes: Box::new([]),
                },
            )])
            .unwrap(),
        ];
        for actor_type in [ActorType::Input, ActorType::Output] {
            let plan =
                |config| boundary_metadata_plan(actor_type, config, ActorFlags::default(), 0, true);
            let before = plan(valid.clone());
            for config in &invalid {
                let after = plan(config.clone());
                assert!(!metadata_only_plan_change(&before, &after));
                assert!(!metadata_only_plan_change(&after, &before));
            }
        }
    }

    #[test]
    fn metadata_classification_absorbs_flags_but_rejects_identity_and_actor_config_changes() {
        let config = boundary_label_config(circular_core::Value::string("label"));
        let plan = |actor_type, flags, generation| {
            boundary_metadata_plan(actor_type, config.clone(), flags, generation, true)
        };
        let before = plan(ActorType::Input, ActorFlags::default(), 0);
        let flagged = plan(ActorType::Input, ActorFlags::new(false, false, true), 0);
        assert!(metadata_only_plan_change(&before, &flagged));
        assert!(metadata_only_plan_change(&flagged, &before));
        for after in [
            plan(ActorType::Output, ActorFlags::default(), 0),
            plan(ActorType::Input, ActorFlags::default(), 1),
            AuthoredProjection::empty(),
        ] {
            assert!(!metadata_only_plan_change(&before, &after));
            assert!(!metadata_only_plan_change(&after, &before));
        }
        let before = plan_of(1, 2, 3, false);
        assert!(!metadata_only_plan_change(
            &before,
            &plan_of(1, 4, 3, false)
        ));
        assert!(!metadata_only_plan_change(&before, &plan_of(1, 2, 3, true)));
        let actor = |label| {
            boundary_metadata_plan(
                ActorType::FixtureMap,
                boundary_label_config(circular_core::Value::string(label)),
                ActorFlags::default(),
                0,
                false,
            )
        };
        assert!(!metadata_only_plan_change(&actor("old"), &actor("new")));
    }

    fn name(value: &str) -> Name {
        Name::from_normalized(value)
    }

    fn config(parameter: i64) -> Config {
        Config::try_new(vec![(
            name("parameter"),
            ConfigValue::Scalar {
                tag: name("i64"),
                bytes: parameter.to_be_bytes().into(),
            },
        )])
        .unwrap()
    }

    fn declaration(actor_type: ActorType, parameter: i64) -> NonContainerActorDecl {
        NonContainerActorDecl::try_new(
            ActorDomain::new(
                actor_type,
                crate::actor_capability::tests::fixture_config(actor_type, config(parameter)),
            ),
            ActorFlags::default(),
        )
        .unwrap()
    }

    fn plan_of(gate: i64, tally: i64, echo: i64, wired: bool) -> AuthoredProjection {
        let mut builder = AuthoredProjectionBuilder::new();
        let gate_id = builder
            .add_actor(name("gate"), declaration(ActorType::FixtureFilter, gate))
            .unwrap();
        let echo_id = builder
            .add_actor(name("echo"), declaration(ActorType::FixtureTap, echo))
            .unwrap();
        builder
            .add_actor(name("tally"), declaration(ActorType::FixtureMap, tally))
            .unwrap();
        if wired {
            builder
                .add_edge(
                    Endpoint::new(gate_id, PortId::try_new("event").unwrap()),
                    Endpoint::new(echo_id, PortId::try_new("event").unwrap()),
                    0,
                    EdgeAttrs::new(
                        circular_core::Ticks::default(),
                        WirePolicy::new(
                            Delivery::BestEffort {
                                on_full: Shed::DropOldest,
                            },
                            Some(PositiveCapacity::new(1).unwrap()),
                        ),
                    ),
                )
                .unwrap();
        }
        builder.finish().unwrap()
    }
}
