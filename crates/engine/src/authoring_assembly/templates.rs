//! Derive admitted scope prototypes from the retained command values.
//! This is compilation, not actor activation. The authored candidate stays unchanged.
use super::fold::EpochCandidate;
use super::rejection::{FoldRejection, TemplateFault, TemplateSide};
use super::verb::ContentVerb;
use circular_core::Value;
use circular_plan::Name;
use circular_protocol::declaration_payload::PresentationOwner;
use circular_protocol::declaration_payload::{
    PlanActorKey, ScopeBoundary, ScopeDeclaration, ScopeRole, ScopeSegment,
};

impl EpochCandidate {
    pub(super) fn expanded_templates(&self) -> Result<Self, FoldRejection> {
        let mut expanded = self.clone();
        let mut cursor = 0;
        while cursor < expanded.tables.actors().len() {
            let (container, declaration) = expanded.tables.actors().rows()[cursor].clone();
            cursor += 1;
            if !circular_plan::ActorType::from_str(&declaration.actor_type)
                .is_some_and(|kind| super::fold::binds_template(kind, &declaration.config))
            {
                continue;
            }
            let Some(config) = declaration.config.as_object() else {
                continue;
            };
            let Some(value) = config.get("template") else {
                continue;
            };
            for key in config.keys() {
                if !matches!(key, "template" | "in" | "out")
                    && !(declaration.actor_type == "replicator"
                        && matches!(key, "at" | "ttl" | "capacity"))
                {
                    return Err(TemplateFault::UnknownContainerField {
                        field: key.to_owned(),
                    }
                    .into());
                }
            }
            if declaration.actor_type == "replicator" {
                circular_actors::replicator_actor::ReplicatorConfig::from_value(
                    &declaration.config,
                )
                .map_err(TemplateFault::ReplicatorConfig)?;
            }
            let Value::String(name) = value else {
                return Err(TemplateFault::NotAName.into());
            };
            let template = self
                .tables
                .templates()
                .get(&Name::from_normalized(name.as_str()))
                .ok_or_else(|| TemplateFault::Unresolved { name: name.clone() })?;
            let mut scope = container.scope.clone();
            scope.push(ScopeSegment::Child(container.local.as_str().to_owned()));
            let segments = scope
                .iter()
                .map(|s| match s {
                    ScopeSegment::Child(name) => {
                        circular_plan::ScopeSeg::Child(Name::from_normalized(name.as_str()))
                    }
                    ScopeSegment::Instance { .. } => unreachable!("authored scope"),
                })
                .collect::<Vec<_>>();
            circular_plan::ScopeId::from_segments(segments)
                .map_err(|_| TemplateFault::ScopeDepthLimit)?;
            if expanded
                .tables
                .scopes()
                .keys()
                .any(|existing| existing.starts_with(&scope))
                || expanded
                    .tables
                    .actors()
                    .keys()
                    .any(|actor| actor.scope.starts_with(&scope))
            {
                return Err(TemplateFault::ChildrenAlsoDeclared.into());
            }
            let addresses = expanded
                .tables
                .edges()
                .values()
                .flat_map(|edge| [&edge.from.0, &edge.to.0])
                .chain(expanded.tables.exports().values().flat_map(|export| {
                    [
                        export.roles.request.as_ref(),
                        export.roles.progress.as_ref(),
                        export.roles.result.as_ref(),
                        export.roles.error.as_ref(),
                    ]
                    .into_iter()
                    .flatten()
                    .map(|(actor, _)| actor)
                }));
            if addresses
                .into_iter()
                .any(|actor| actor.scope.starts_with(&scope))
            {
                return Err(TemplateFault::ParentAddressesChildren.into());
            }
            let start = expanded.tables.actors().len();
            let verbs = template
                .commands()
                .iter()
                .map(ContentVerb::from_snapshot_item)
                .collect::<Result<Vec<_>, _>>()
                .map_err(TemplateFault::Command)?;
            let mut ports = Vec::new();
            for verb in &verbs {
                if let ContentVerb::UpsertActor { actor, declaration } = verb {
                    use circular_protocol::boundary_port::{
                        BoundaryActorGeneration, BoundaryPortDirection, BoundaryPortId,
                    };
                    let direction =
                        match circular_core::ActorType::from_str(&declaration.actor_type) {
                            Some(circular_core::ActorType::Input) => BoundaryPortDirection::Inlet,
                            Some(circular_core::ActorType::Output) => BoundaryPortDirection::Outlet,
                            _ => continue,
                        };
                    let mut actual = actor.clone();
                    actual.scope.splice(..0, scope.clone());
                    let old = BoundaryPortId::derive(
                        direction,
                        actor,
                        BoundaryActorGeneration::initial(),
                    )
                    .map_err(TemplateFault::BoundaryPort)?;
                    let new = BoundaryPortId::derive(
                        direction,
                        &actual,
                        BoundaryActorGeneration::initial(),
                    )
                    .map_err(TemplateFault::MintedBoundaryPort)?;
                    ports.push((
                        actor.clone(),
                        old.as_port_id().as_str().to_owned(),
                        new.as_port_id().as_str().to_owned(),
                    ));
                }
            }
            for verb in verbs {
                expanded.apply(&placed(verb, &scope, &ports)?).map(drop)?;
            }
            for (side, kind) in [(TemplateSide::In, "input"), (TemplateSide::Out, "output")] {
                let Some(Value::Array(topics)) = config.get(side.as_str()) else {
                    return Err(TemplateFault::SideNotArray { side }.into());
                };
                let mut expected = std::collections::BTreeSet::new();
                for topic in topics {
                    let Value::String(topic) = topic else {
                        return Err(TemplateFault::TopicNotText.into());
                    };
                    if topic.is_empty() || !expected.insert(topic.clone()) {
                        return Err(TemplateFault::TopicDuplicateOrEmpty.into());
                    }
                }
                let mut actual = std::collections::BTreeSet::new();
                for (_, boundary) in expanded.tables.actors().rows()[start..]
                    .iter()
                    .filter(|(actor, decl)| actor.scope == scope && decl.actor_type == kind)
                {
                    let Some(Value::String(topic)) =
                        boundary.config.as_object().and_then(|c| c.get("label"))
                    else {
                        return Err(TemplateFault::BoundaryWithoutTopic.into());
                    };
                    if !actual.insert(topic.clone()) {
                        return Err(TemplateFault::BoundaryTopicDuplicate.into());
                    }
                }
                if actual != expected {
                    return Err(TemplateFault::TopicsDisagree { side }.into());
                }
            }
            expanded.tables.put::<super::tables::Scopes>(
                scope,
                ScopeDeclaration {
                    role: if declaration.actor_type == "replicator" {
                        ScopeRole::Template
                    } else {
                        ScopeRole::Concrete
                    },
                    boundary: ScopeBoundary {
                        inlets: vec![],
                        outlets: vec![],
                    },
                },
            );
        }
        Ok(expanded)
    }
}

fn placed(
    verb: ContentVerb,
    prefix: &[ScopeSegment],
    ports: &[(PlanActorKey, String, String)],
) -> Result<ContentVerb, TemplateFault> {
    let scope = |relative: &mut Vec<ScopeSegment>| {
        let mut value = prefix.to_vec();
        value.append(relative);
        *relative = value;
    };
    let actor = |key: &mut PlanActorKey| scope(&mut key.scope);
    let endpoint = |endpoint: &mut (PlanActorKey, String)| {
        if let Some((_, _, actual)) = ports
            .iter()
            .find(|(key, old, _)| key == &endpoint.0 && old == &endpoint.1)
        {
            endpoint.1 = actual.clone();
        }
        actor(&mut endpoint.0);
    };
    use ContentVerb as V;
    Ok(match verb {
        V::UpsertTemplate { .. } => return Err(TemplateFault::RegistrationOutsideRoot),
        V::UpsertActor {
            actor: mut key,
            declaration,
        } => {
            actor(&mut key);
            V::UpsertActor {
                actor: key,
                declaration,
            }
        }
        V::UpsertScope {
            scope: mut key,
            mut declaration,
        } => {
            scope(&mut key);
            for binding in declaration
                .boundary
                .inlets
                .iter_mut()
                .chain(&mut declaration.boundary.outlets)
            {
                let old = binding.inner.1.clone();
                endpoint(&mut binding.inner);
                if binding.outer == old {
                    binding.outer = binding.inner.1.clone();
                }
            }
            V::UpsertScope {
                scope: key,
                declaration,
            }
        }
        V::UpsertEdge {
            declaration: mut edge,
        } => {
            endpoint(&mut edge.from);
            endpoint(&mut edge.to);
            V::UpsertEdge { declaration: edge }
        }
        V::UpsertExportMount {
            mut mount,
            mut declaration,
        } => {
            scope(&mut mount.scope);
            for role in [
                &mut declaration.roles.request,
                &mut declaration.roles.progress,
                &mut declaration.roles.result,
                &mut declaration.roles.error,
            ]
            .into_iter()
            .flatten()
            {
                endpoint(role);
            }
            V::UpsertExportMount { mount, declaration }
        }
        V::UpsertAnnotation {
            mut annotation,
            mut declaration,
        } => {
            scope(&mut annotation.scope);
            for reference in &mut declaration.refs {
                actor(reference);
            }
            V::UpsertAnnotation {
                annotation,
                declaration,
            }
        }
        V::SetPresentation {
            mut owner,
            mut presentation,
        } => {
            use circular_protocol::declaration_payload::Anchor;
            match &mut owner {
                PresentationOwner::Actor(key) => actor(key),
                PresentationOwner::Annotation(key) => scope(&mut key.scope),
            }
            match &mut presentation.anchor {
                Some(Anchor::Relative { target, .. } | Anchor::Align { target, .. }) => {
                    actor(target)
                }
                _ => {}
            }
            V::SetPresentation {
                owner,
                presentation,
            }
        }
        V::RetireActor { .. }
        | V::RetireEdge { .. }
        | V::RetireScope { .. }
        | V::MoveToScope { .. }
        | V::RetireExportMount { .. }
        | V::RetireAnnotation { .. }
        | V::SetFlags { .. }
        | V::RetireTemplate { .. }
        | V::ReplaceAuthoringEnvironment { .. } => return Err(TemplateFault::NotAdmitted),
    })
}

