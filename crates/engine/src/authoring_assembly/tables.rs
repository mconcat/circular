
use circular_protocol::declaration_payload::PresentationOwner;
use std::collections::BTreeMap;

use circular_plan::{Name, Template};
use circular_protocol::boundary_port::BoundaryActorGeneration;
use circular_protocol::declaration_payload::{
    ActorDeclaration, AnnotationDeclaration, AuthoringEnvironment, DeclaredEdgeKey,
    EdgeDeclaration, ExportDeclaration, PlanActorKey, PlanAnnotationKey, PlanExportKey,
    Presentation, ScopeDeclaration, ScopeSegment,
};

#[derive(Clone, Debug)]
pub struct Table<K, V>(Vec<(K, V)>);

impl<K, V> Default for Table<K, V> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<K: PartialEq, V> Table<K, V> {
    #[must_use]
    pub fn rows(&self) -> &[(K, V)] {
        &self.0
    }

    pub fn iter(&self) -> std::slice::Iter<'_, (K, V)> {
        self.0.iter()
    }

    #[must_use]
    pub fn get(&self, key: &K) -> Option<&V> {
        self.0
            .iter()
            .find(|(existing, _)| existing == key)
            .map(|(_, value)| value)
    }

    #[must_use]
    pub fn contains(&self, key: &K) -> bool {
        self.0.iter().any(|(existing, _)| existing == key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.0.iter().map(|(key, _)| key)
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.0.iter().map(|(_, value)| value)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn retained(&self, keep: impl Fn(&K, &V) -> bool) -> Self
    where
        K: Clone,
        V: Clone,
    {
        Self(
            self.0
                .iter()
                .filter(|(key, value)| keep(key, value))
                .cloned()
                .collect(),
        )
    }

    fn drain_where(&mut self, remove: impl Fn(&K, &V) -> bool) -> Vec<K>
    where
        K: Clone,
    {
        let mut removed = Vec::new();
        self.0.retain(|(key, value)| {
            let gone = remove(key, value);
            if gone {
                removed.push(key.clone());
            }
            !gone
        });
        removed
    }
}

impl<'a, K, V> IntoIterator for &'a Table<K, V> {
    type Item = &'a (K, V);
    type IntoIter = std::slice::Iter<'a, (K, V)>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

pub trait Store: Clone {
    type Key: Clone + PartialEq;
    type Value: Clone;

    fn put(&mut self, key: Self::Key, value: Self::Value);
    fn take(&mut self, key: &Self::Key) -> bool;
    fn rekey(&mut self, old: &Self::Key, new: Self::Key, value: Self::Value) -> bool;
}

impl<K: Clone + PartialEq, V: Clone> Store for Table<K, V> {
    type Key = K;
    type Value = V;

    fn put(&mut self, key: K, value: V) {
        if let Some((_, existing)) = self.0.iter_mut().find(|(existing, _)| *existing == key) {
            *existing = value;
        } else {
            self.0.push((key, value));
        }
    }

    fn take(&mut self, key: &K) -> bool {
        !self.drain_where(|existing, _| existing == key).is_empty()
    }

    fn rekey(&mut self, old: &K, new: K, value: V) -> bool {
        let Some(row) = self.0.iter_mut().find(|(existing, _)| existing == old) else {
            return false;
        };
        *row = (new, value);
        true
    }
}

impl<K: Clone + Ord, V: Clone> Store for BTreeMap<K, V> {
    type Key = K;
    type Value = V;

    fn put(&mut self, key: K, value: V) {
        self.insert(key, value);
    }

    fn take(&mut self, key: &K) -> bool {
        self.remove(key).is_some()
    }

    fn rekey(&mut self, old: &K, new: K, value: V) -> bool {
        if self.remove(old).is_none() {
            return false;
        }
        self.insert(new, value);
        true
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Change<V> {
    Put(V),
    Gone,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Changes<K, V>(Vec<(K, Change<V>)>);

impl<K, V> Default for Changes<K, V> {
    fn default() -> Self {
        Self(Vec::new())
    }
}

impl<K: PartialEq, V> Changes<K, V> {
    #[must_use]
    pub fn rows(&self) -> &[(K, Change<V>)] {
        &self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn record(&mut self, key: K, change: Change<V>) {
        if let Some((_, existing)) = self.0.iter_mut().find(|(existing, _)| *existing == key) {
            *existing = change;
        } else {
            self.0.push((key, change));
        }
    }

    fn then(mut self, later: Self) -> Self {
        for (key, change) in later.0 {
            self.record(key, change);
        }
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Delta {
    actors: Changes<PlanActorKey, ActorDeclaration>,
    presentations: Changes<PresentationOwner, Presentation<PlanActorKey>>,
    generations: Changes<PlanActorKey, BoundaryActorGeneration>,
    edges: Changes<DeclaredEdgeKey, EdgeDeclaration>,
    scopes: Changes<Vec<ScopeSegment>, ScopeDeclaration>,
    exports: Changes<PlanExportKey, ExportDeclaration>,
    annotations: Changes<PlanAnnotationKey, AnnotationDeclaration>,
    templates: Changes<Name, Template>,
    environment: Option<AuthoringEnvironment>,
}

impl Delta {
    #[must_use]
    pub fn then(self, later: Self) -> Self {
        Self {
            actors: self.actors.then(later.actors),
            presentations: self.presentations.then(later.presentations),
            generations: self.generations.then(later.generations),
            edges: self.edges.then(later.edges),
            scopes: self.scopes.then(later.scopes),
            exports: self.exports.then(later.exports),
            annotations: self.annotations.then(later.annotations),
            templates: self.templates.then(later.templates),
            environment: later.environment.or(self.environment),
        }
    }

    #[must_use]
    pub fn actors(&self) -> &Changes<PlanActorKey, ActorDeclaration> {
        &self.actors
    }

    #[must_use]
    pub fn presentations(&self) -> &Changes<PresentationOwner, Presentation<PlanActorKey>> {
        &self.presentations
    }

    #[must_use]
    pub fn generations(&self) -> &Changes<PlanActorKey, BoundaryActorGeneration> {
        &self.generations
    }

    #[must_use]
    pub fn edges(&self) -> &Changes<DeclaredEdgeKey, EdgeDeclaration> {
        &self.edges
    }

    #[must_use]
    pub fn scopes(&self) -> &Changes<Vec<ScopeSegment>, ScopeDeclaration> {
        &self.scopes
    }

    #[must_use]
    pub fn exports(&self) -> &Changes<PlanExportKey, ExportDeclaration> {
        &self.exports
    }

    #[must_use]
    pub fn annotations(&self) -> &Changes<PlanAnnotationKey, AnnotationDeclaration> {
        &self.annotations
    }

    #[must_use]
    pub fn templates(&self) -> &Changes<Name, Template> {
        &self.templates
    }

    #[must_use]
    pub fn environment(&self) -> Option<&AuthoringEnvironment> {
        self.environment.as_ref()
    }

    pub(crate) fn replace_environment(&mut self, environment: AuthoringEnvironment) {
        self.environment = Some(environment);
    }
}

mod seal {
    pub struct Seal(pub(super) ());
}

use seal::Seal;

pub trait Section {
    type Store: Store;

    fn store(tables: &mut DeclaredTables, seal: Seal) -> &mut Self::Store;
    fn changes(delta: &mut Delta, seal: Seal) -> &mut Changes<Key<Self>, Value<Self>>;
}

pub type Key<S> = <<S as Section>::Store as Store>::Key;
pub type Value<S> = <<S as Section>::Store as Store>::Value;

macro_rules! sections {
    ($($(#[$doc:meta])* $section:ident: $field:ident => $store:ty;)*) => {
        $(
            $(#[$doc])*
            pub enum $section {}

            impl Section for $section {
                type Store = $store;

                fn store(tables: &mut DeclaredTables, _: Seal) -> &mut Self::Store {
                    &mut tables.$field
                }

                fn changes(delta: &mut Delta, _: Seal) -> &mut Changes<Key<Self>, Value<Self>> {
                    &mut delta.$field
                }
            }
        )*
    };
}

sections! {
    Actors: actors => Table<PlanActorKey, ActorDeclaration>;
    Presentations: presentations => Table<PresentationOwner, Presentation<PlanActorKey>>;
    Generations: generations => Table<PlanActorKey, BoundaryActorGeneration>;
    Edges: edges => Table<DeclaredEdgeKey, EdgeDeclaration>;
    Scopes: scopes => Table<Vec<ScopeSegment>, ScopeDeclaration>;
    /// export mount.
    Exports: exports => Table<PlanExportKey, ExportDeclaration>;
    Annotations: annotations => Table<PlanAnnotationKey, AnnotationDeclaration>;
    Templates: templates => BTreeMap<Name, Template>;
}

#[derive(Clone, Debug, Default)]
pub struct DeclaredTables {
    actors: Table<PlanActorKey, ActorDeclaration>,
    presentations: Table<PresentationOwner, Presentation<PlanActorKey>>,
    generations: Table<PlanActorKey, BoundaryActorGeneration>,
    edges: Table<DeclaredEdgeKey, EdgeDeclaration>,
    scopes: Table<Vec<ScopeSegment>, ScopeDeclaration>,
    exports: Table<PlanExportKey, ExportDeclaration>,
    annotations: Table<PlanAnnotationKey, AnnotationDeclaration>,
    templates: BTreeMap<Name, Template>,
    recording: Option<Delta>,
}

impl DeclaredTables {
    #[must_use]
    pub fn actors(&self) -> &Table<PlanActorKey, ActorDeclaration> {
        &self.actors
    }

    #[must_use]
    pub fn presentations(&self) -> &Table<PresentationOwner, Presentation<PlanActorKey>> {
        &self.presentations
    }

    #[must_use]
    pub fn generations(&self) -> &Table<PlanActorKey, BoundaryActorGeneration> {
        &self.generations
    }

    #[must_use]
    pub fn edges(&self) -> &Table<DeclaredEdgeKey, EdgeDeclaration> {
        &self.edges
    }

    #[must_use]
    pub fn scopes(&self) -> &Table<Vec<ScopeSegment>, ScopeDeclaration> {
        &self.scopes
    }

    #[must_use]
    pub fn exports(&self) -> &Table<PlanExportKey, ExportDeclaration> {
        &self.exports
    }

    #[must_use]
    pub fn annotations(&self) -> &Table<PlanAnnotationKey, AnnotationDeclaration> {
        &self.annotations
    }

    #[must_use]
    pub fn templates(&self) -> &BTreeMap<Name, Template> {
        &self.templates
    }

    #[must_use]
    pub fn generation(&self, key: &PlanActorKey) -> BoundaryActorGeneration {
        self.generations
            .get(key)
            .copied()
            .unwrap_or_else(BoundaryActorGeneration::initial)
    }

    pub(crate) fn put<S: Section>(&mut self, key: Key<S>, value: Value<S>) {
        if let Some(delta) = self.recording.as_mut() {
            S::changes(delta, Seal(())).record(key.clone(), Change::Put(value.clone()));
        }
        S::store(self, Seal(())).put(key, value);
    }

    pub(crate) fn remove<S: Section>(&mut self, key: &Key<S>) {
        if S::store(self, Seal(())).take(key)
            && let Some(delta) = self.recording.as_mut()
        {
            S::changes(delta, Seal(())).record(key.clone(), Change::Gone);
        }
    }

    pub(crate) fn rekey<S: Section>(&mut self, old: &Key<S>, new: Key<S>, value: Value<S>) {
        if S::store(self, Seal(())).rekey(old, new.clone(), value.clone())
            && let Some(delta) = self.recording.as_mut()
        {
            let changes = S::changes(delta, Seal(()));
            changes.record(old.clone(), Change::Gone);
            changes.record(new, Change::Put(value));
        }
    }

    pub(crate) fn remove_below(&mut self, scope: &[ScopeSegment]) {
        let below = |candidate: &[ScopeSegment]| candidate.starts_with(scope);
        let scopes = self.scopes.drain_where(|key, _| below(key));
        let actors = self.actors.drain_where(|key, _| below(&key.scope));
        let edges = self
            .edges
            .drain_where(|_, edge| below(&edge.from.0.scope) || below(&edge.to.0.scope));
        let presentations = self.presentations.drain_where(|key, _| below(key.scope()));
        let exports = self.exports.drain_where(|key, _| below(&key.scope));
        let annotations = self.annotations.drain_where(|key, _| below(&key.scope));
        if let Some(delta) = self.recording.as_mut() {
            for key in scopes {
                delta.scopes.record(key, Change::Gone);
            }
            for key in actors {
                delta.actors.record(key, Change::Gone);
            }
            for key in edges {
                delta.edges.record(key, Change::Gone);
            }
            for key in presentations {
                delta.presentations.record(key, Change::Gone);
            }
            for key in exports {
                delta.exports.record(key, Change::Gone);
            }
            for key in annotations {
                delta.annotations.record(key, Change::Gone);
            }
        }
    }

    pub(crate) fn open_recording(&mut self) {
        self.recording = Some(Delta::default());
    }

    pub(crate) fn close_recording(&mut self) -> Delta {
        self.recording.take().unwrap_or_default()
    }

    #[must_use]
    pub(crate) fn restrict(&self, target: &[ScopeSegment]) -> Self {
        let below = |candidate: &[ScopeSegment]| candidate.starts_with(target);
        Self {
            actors: self.actors.retained(|key, _| below(&key.scope)),
            presentations: self.presentations.retained(|key, _| below(key.scope())),
            generations: self.generations.retained(|key, _| below(&key.scope)),
            edges: self
                .edges
                .retained(|_, edge| below(&edge.from.0.scope) && below(&edge.to.0.scope)),
            scopes: self
                .scopes
                .retained(|scope, _| scope.len() > target.len() && below(scope)),
            exports: self.exports.retained(|key, _| below(&key.scope)),
            annotations: self.annotations.retained(|key, _| below(&key.scope)),
            templates: self.templates.clone(),
            recording: None,
        }
    }

    pub(crate) fn splice(&mut self, target: &[ScopeSegment], candidate: Self) {
        let below = |candidate: &[ScopeSegment]| candidate.starts_with(target);
        if target.is_empty() {
            self.templates = candidate.templates;
        }
        self.actors.drain_where(|key, _| below(&key.scope));
        self.edges
            .drain_where(|_, edge| below(&edge.from.0.scope) || below(&edge.to.0.scope));
        self.scopes
            .drain_where(|scope, _| scope.len() > target.len() && below(scope));
        self.presentations.drain_where(|key, _| below(key.scope()));
        self.exports.drain_where(|key, _| below(&key.scope));
        self.annotations.drain_where(|key, _| below(&key.scope));
        self.generations.drain_where(|key, _| below(&key.scope));

        self.actors.0.extend(candidate.actors.0);
        self.edges.0.extend(candidate.edges.0);
        self.scopes.0.extend(candidate.scopes.0);
        self.presentations.0.extend(candidate.presentations.0);
        self.exports.0.extend(candidate.exports.0);
        self.annotations.0.extend(candidate.annotations.0);
        self.generations.0.extend(candidate.generations.0);
    }
}
