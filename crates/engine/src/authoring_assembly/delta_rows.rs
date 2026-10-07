
use circular_core::Value;
use circular_plan::{ActorType, Name};
use circular_protocol::declaration_payload::PresentationOwner;
use circular_protocol::declaration_payload::{
    ActorLocal, DeclaredEdgeKey, EdgeKey, PlanActorKey, PlanAnnotationKey, PlanExportKey,
    ScopeSegment, edge_identity_value, plan_actor_key_value, scope_identity_value,
};

use super::rejection::FoldRejection;
use super::tables::{Change, Delta};
use super::verb::ContentKind;

#[derive(Clone, Debug, PartialEq)]
pub(super) enum RowKey {
    Template(Name),
    Scope(Vec<ScopeSegment>),
    Actor(PlanActorKey),
    Edge(DeclaredEdgeKey),
    Export(PlanExportKey),
    Annotation(PlanAnnotationKey),
    Presentation(PresentationOwner),
}

pub(super) fn layer_within(layer: &[ScopeSegment], scope: &[ScopeSegment]) -> bool {
    layer.starts_with(scope)
}

impl RowKey {
    pub(super) fn within(&self, scope: &[ScopeSegment]) -> bool {
        match self {
            Self::Template(_) => scope.is_empty(),
            Self::Scope(own) => own.len() > scope.len() && layer_within(own, scope),
            Self::Actor(key) => layer_within(&key.scope, scope),
            Self::Presentation(key) => layer_within(key.scope(), scope),
            Self::Edge(key) => layer_within(&key.from.0.scope, scope),
            Self::Export(key) => layer_within(&key.scope, scope),
            Self::Annotation(key) => layer_within(&key.scope, scope),
        }
    }

    fn place(&self) -> (&'static str, Value) {
        let absolute = |identity: Value| Value::array([Value::Int(1), identity]);
        match self {
            Self::Template(name) => ("name", Value::string(name.as_str())),
            Self::Scope(scope) => ("scope", absolute(scope_identity_value(scope))),
            Self::Actor(key) => ("actor", absolute(plan_actor_key_value(key))),
            Self::Presentation(key) => (
                "owner",
                circular_protocol::declaration_payload::presentation_owner_value(
                    key,
                    |key| Ok::<_, std::convert::Infallible>(absolute(plan_actor_key_value(key))),
                    |key| Ok(absolute(local_key_value(&key.scope, &key.local))),
                )
                .unwrap(),
            ),
            Self::Edge(key) => (
                "edge",
                absolute(edge_identity_value(&EdgeKey::Declared(key.clone()))),
            ),
            Self::Export(key) => ("mount", absolute(local_key_value(&key.scope, &key.local))),
            Self::Annotation(key) => (
                "annotation",
                absolute(local_key_value(&key.scope, &key.local)),
            ),
        }
    }

    fn retire(&self) -> Result<Value, FoldRejection> {
        let (kind, (field, value)) = match self {
            Self::Template(_) => (ContentKind::RetireTemplate, self.place()),
            Self::Scope(_) => (ContentKind::RetireScope, self.place()),
            Self::Actor(_) => (ContentKind::RetireActor, self.place()),
            Self::Presentation(owner) => {
                return match owner {
                    PresentationOwner::Actor(key) => Self::Actor(key.clone()),
                    PresentationOwner::Annotation(key) => Self::Annotation(key.clone()),
                }
                .retire();
            }
            Self::Edge(_) => (ContentKind::RetireEdge, self.place()),
            Self::Export(_) => (ContentKind::RetireExportMount, self.place()),
            Self::Annotation(_) => (ContentKind::RetireAnnotation, self.place()),
        };
        Value::object([("kind", Value::string(kind.as_str())), (field, value)])
            .map_err(|error| FoldRejection::SnapshotEncode(format!("delta row: {error:?}")))
    }

    fn put(&self, item: Value) -> Result<Value, FoldRejection> {
        let (field, value) = self.place();
        let Value::Object(object) = item else {
            return Err(FoldRejection::SnapshotEncode(
                "a snapshot item is an object".to_owned(),
            ));
        };
        let mut fields = object.into_map();
        fields.insert(field.to_owned(), value);
        Ok(Value::Object(fields.into()))
    }
}

fn local_key_value(scope: &[ScopeSegment], local: &str) -> Value {
    plan_actor_key_value(&PlanActorKey {
        scope: scope.to_vec(),
        local: ActorLocal::parse(local),
    })
}

fn reach(delta: &Delta) -> Vec<RowKey> {
    let mut keys = Vec::new();
    let mut add = |key: RowKey| {
        if !keys.contains(&key) {
            keys.push(key);
        }
    };
    let layer = |scope: &[ScopeSegment]| RowKey::Scope(scope.to_vec());
    for (scope, _) in delta.scopes().rows() {
        add(RowKey::Scope(scope.clone()));
    }
    for (actor, change) in delta.actors().rows() {
        add(RowKey::Actor(actor.clone()));
        add(layer(&actor.scope));
        if let Change::Put(declaration) = change
            && ActorType::from_str(&declaration.actor_type).is_some_and(ActorType::is_container)
        {
            let mut opened = actor.scope.clone();
            opened.push(ScopeSegment::Child(actor.local.as_str().to_owned()));
            add(RowKey::Scope(opened));
        }
    }
    for (actor, _) in delta.generations().rows() {
        add(layer(&actor.scope));
    }
    for (owner, _) in delta.presentations().rows() {
        add(RowKey::Presentation(owner.clone()));
        add(layer(owner.scope()));
    }
    for (edge, _) in delta.edges().rows() {
        add(RowKey::Edge(edge.clone()));
        add(layer(&edge.from.0.scope));
    }
    for (mount, _) in delta.exports().rows() {
        add(RowKey::Export(mount.clone()));
        add(layer(&mount.scope));
    }
    for (annotation, _) in delta.annotations().rows() {
        add(RowKey::Annotation(annotation.clone()));
        add(layer(&annotation.scope));
    }
    for (name, _) in delta.templates().rows() {
        add(RowKey::Template(name.clone()));
    }
    keys
}

pub(super) fn lower(
    delta: &Delta,
    scope: &[ScopeSegment],
    items: Vec<(RowKey, Value)>,
) -> Result<Vec<Value>, FoldRejection> {
    let mut gone: Vec<RowKey> = Vec::new();
    let mut put: Vec<RowKey> = Vec::new();
    for key in reach(delta).into_iter().filter(|key| key.within(scope)) {
        if items.iter().any(|(item, _)| item == &key) {
            put.push(key);
            continue;
        }
        let key = match key {
            RowKey::Presentation(owner) => {
                let row = match owner {
                    PresentationOwner::Actor(key) => RowKey::Actor(key),
                    PresentationOwner::Annotation(key) => RowKey::Annotation(key),
                };
                put.push(row.clone());
                row
            }
            key => key,
        };
        if !gone.contains(&key) {
            gone.push(key);
        }
    }
    let mut rows = gone
        .iter()
        .map(RowKey::retire)
        .collect::<Result<Vec<_>, _>>()?;
    for (key, item) in items {
        if put.contains(&key) {
            rows.push(key.put(item)?);
        }
    }
    Ok(rows)
}
