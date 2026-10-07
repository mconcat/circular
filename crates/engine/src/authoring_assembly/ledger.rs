
use std::collections::BTreeMap;

use crate::authoring_assembly::projection::AuthoredProjection;
use crate::authoring_assembly::projection::fold_projection;
use circular_core::{Boundary, Ceilings, Value, decode, encode};
use circular_plan::{
    Anchor as PlanAnchor, AnnotationKind as PlanAnnotationKind, Config as PlanConfig,
    ConfigRecord as PlanConfigRecord, ConfigValue as PlanConfigValue, Delivery as PlanDelivery,
    Endpoint as PlanEndpoint, NamedActorId, Role as PlanRole, ScopeId, ScopeRole as PlanScopeRole,
    ScopeSeg, Shed as PlanShed,
};
#[cfg(test)]
use circular_protocol::DeclarationCommand;
use circular_protocol::authoring_snapshot::AuthoringSnapshotEncoder;
#[cfg(test)]
use circular_protocol::authoring_snapshot::{CompactedDeclarationRejection, decode_compacted};
#[cfg(test)]
use circular_protocol::declaration_payload::AddressContext;
use circular_protocol::declaration_payload::{
    AuthoringEnvironment, BeginEpoch, DeclaredEdgeKey, ExpectedRevision, PlanActorKey,
    ScopeSegment, decode_environment, decode_scope_identity,
};
use sha2::{Digest, Sha256};

use super::delta_rows::{self, RowKey};
use super::fold::EpochCandidate;
use super::rejection::{
    BracketField, Coded, Encoded, EnvironmentSide, FoldRejection, JournalVocabulary, PersistedFault,
};
use super::tables::Delta;
use super::verb::ContentVerb;
use circular_core::{CodecError, DuplicateKeyError};

#[derive(Clone, Debug, PartialEq)]
pub enum JournalEntryRejection {
    UnknownVocabulary(JournalVocabulary),
    Corrupt(FoldRejection),
}

impl From<FoldRejection> for JournalEntryRejection {
    fn from(rejection: FoldRejection) -> Self {
        Self::Corrupt(rejection)
    }
}

impl From<PersistedFault> for JournalEntryRejection {
    fn from(fault: PersistedFault) -> Self {
        Self::Corrupt(fault.into())
    }
}

impl std::fmt::Display for JournalEntryRejection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownVocabulary(name) => name.fmt(formatter),
            Self::Corrupt(rejection) => rejection.fmt(formatter),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AuthoringState {
    current: EpochCandidate,
    environment: AuthoringEnvironment,
    revisions: BTreeMap<Vec<ScopeSegment>, RecordedRevision>,
    cursor: u64,
    last: Option<DurableCommit>,
    dedup: BTreeMap<Vec<u8>, DedupTerminal>,
    next_epoch: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct RecordedRevision {
    authoring: Vec<u8>,
    topology: Vec<u8>,
}

#[derive(Clone, Debug)]
struct DedupTerminal {
    request_digest: Vec<u8>,
    terminal: Value,
}

#[derive(Clone, Debug)]
pub struct DurableCommit {
    pub cursor: u64,
    pub(crate) commit_id: Vec<u8>,
    pub(crate) epoch_id: Vec<u8>,
    pub(crate) request_digest: Vec<u8>,
    pub(crate) target: Vec<ScopeSegment>,
    pub(crate) environment_before: AuthoringEnvironment,
    pub(crate) revisions: Vec<RevisionRecord>,
    pub(crate) commands: Vec<Value>,
    pub terminal: Value,
}

#[derive(Clone, Debug)]
pub(crate) struct RevisionRecord {
    pub(crate) scope: Vec<ScopeSegment>,
    pub(crate) authoring_before: Option<Vec<u8>>,
    pub(crate) authoring_after: Option<Vec<u8>>,
    pub(crate) topology_before: Option<Vec<u8>>,
    pub(crate) topology_after: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DedupLookup {
    New,
    Repeat(Value),
    Equivocation,
}

#[derive(Debug)]
pub struct Promotion {
    pub plan: AuthoredProjection,
    pub revision: Vec<u8>,
    pub cursor: u64,
    pub terminal: Option<Value>,
}

#[derive(Clone, Debug)]
pub struct AuthoringSnapshot {
    pub scope: Vec<ScopeSegment>,
    pub authoring_revision: Option<Vec<u8>>,
    pub topology_revision: Option<Vec<u8>>,
    pub cursor: u64,
    pub environment: AuthoringEnvironment,
    pub items: Vec<Value>,
}

#[must_use]
pub fn genesis_environment() -> AuthoringEnvironment {
    AuthoringEnvironment {
        declaration_schema: vec![0x01],
        spec_set: vec![0x03],
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectCreation {
    pub project: [u8; 32],
    pub environment: AuthoringEnvironment,
}

const EPOCH_ENTRY_FIELD: &str = "epoch";
const PROJECT_CREATION_ENTRY_FIELD: &str = "project_creation";

impl ProjectCreation {
    pub fn journal_entry_value(&self) -> Result<Value, FoldRejection> {
        let body = Value::object([
            (
                "environment",
                circular_protocol::authoring_snapshot::environment_value(&self.environment)
                    .map_err(FoldRejection::SnapshotEncode)?,
            ),
            ("project", Value::Bytes(self.project.to_vec())),
        ])
        .map_err(encode_error(Encoded::ProjectCreation))?;
        Value::object([
            ("entry_version", Value::Int(AUTHORING_JOURNAL_ENTRY_VERSION)),
            (PROJECT_CREATION_ENTRY_FIELD, body),
        ])
        .map_err(encode_error(Encoded::ProjectCreationEntry))
    }

    pub fn from_journal_entry(value: &Value) -> Result<Option<Self>, JournalEntryRejection> {
        let Value::Object(object) = value else {
            return Err(PersistedFault::EntryNotObject.into());
        };
        let Some(body) = object.get(PROJECT_CREATION_ENTRY_FIELD) else {
            return Ok(None);
        };
        let mut fields = object.clone().into_map();
        if take_field(&mut fields, "entry_version")? != Value::Int(AUTHORING_JOURNAL_ENTRY_VERSION)
        {
            return Err(JournalEntryRejection::UnknownVocabulary(
                JournalVocabulary::EntryVersion,
            ));
        }
        fields.remove(PROJECT_CREATION_ENTRY_FIELD);
        if let Some((unknown, _)) = fields.into_iter().next() {
            return Err(JournalEntryRejection::UnknownVocabulary(
                JournalVocabulary::EntryField(unknown),
            ));
        }
        let Value::Object(body) = body else {
            return Err(PersistedFault::ProjectCreationNotObject.into());
        };
        let mut fields = body.clone().into_map();
        let environment = decode_environment(take_field(&mut fields, "environment")?)
            .map_err(PersistedFault::ProjectCreationEnvironment)?;
        let project = match take_field(&mut fields, "project")? {
            Value::Bytes(bytes) => <[u8; 32]>::try_from(bytes.as_slice())
                .map_err(|_| PersistedFault::ProjectNot32Bytes)?,
            _ => return Err(PersistedFault::ProjectNotBytes.into()),
        };
        if let Some((unknown, _)) = fields.into_iter().next() {
            return Err(JournalEntryRejection::UnknownVocabulary(
                JournalVocabulary::ProjectCreationField(unknown),
            ));
        }
        Ok(Some(Self {
            project,
            environment,
        }))
    }
}

impl ProjectCreation {
    #[cfg(test)]
    #[must_use]
    pub fn for_test() -> Self {
        Self {
            project: [0x64; 32],
            environment: genesis_environment(),
        }
    }
}

#[cfg(test)]
impl Default for AuthoringState {
    fn default() -> Self {
        Self::created(&ProjectCreation::for_test())
    }
}

impl AuthoringState {
    #[must_use]
    pub fn created(creation: &ProjectCreation) -> Self {
        Self {
            current: EpochCandidate::open(Vec::new()).expect("project root is authorable"),
            environment: creation.environment.clone(),
            revisions: BTreeMap::new(),
            cursor: 0,
            last: None,
            dedup: BTreeMap::new(),
            next_epoch: 1,
        }
    }
}

impl AuthoringState {
    pub fn begin_candidate(
        &self,
        begin: &BeginEpoch,
        begin_payload: &[u8],
        begin_verb_tag: u8,
    ) -> Result<EpochCandidate, FoldRejection> {
        let target = address_identity(&begin.scope).clone();
        let mut candidate = EpochCandidate::open(target.clone())?;
        candidate.target_role = if target.is_empty() {
            Some(circular_protocol::declaration_payload::ScopeRole::Concrete)
        } else {
            self.current
                .tables
                .scopes()
                .get(&target)
                .map(|declaration| declaration.role)
        };
        candidate.tables = self.current.tables.restrict(&target);
        candidate.replacement_environment = None;
        candidate.opening_environment = Some(begin.expected_environment.clone());
        candidate.issued_epoch = None;
        candidate.commit_id = Some(begin.commit_id.clone());
        candidate.expected_revision = Some(begin.expected_revision.clone());
        candidate.request_parts = vec![(begin_verb_tag, begin_payload.to_vec())];
        candidate.accepted_commands.clear();
        candidate.accepted_content_commands = 0;
        candidate.authored_scopes.clear();
        Ok(candidate)
    }

    pub fn validate(&self, candidate: EpochCandidate) -> Result<AuthoredProjection, FoldRejection> {
        self.clone()
            .promote_inner(candidate)
            .map(|promotion| promotion.plan)
    }

    pub fn promote_commit(
        &mut self,
        candidate: EpochCandidate,
        request_digest: Vec<u8>,
    ) -> Result<Promotion, FoldRejection> {
        let commit_id = candidate
            .commit_id
            .clone()
            .ok_or(FoldRejection::BracketIncomplete(BracketField::CommitId))?;
        let epoch_id = candidate
            .issued_epoch
            .clone()
            .ok_or(FoldRejection::BracketIncomplete(BracketField::IssuedEpoch))?;
        let opening_environment =
            candidate
                .opening_environment
                .clone()
                .ok_or(FoldRejection::BracketIncomplete(
                    BracketField::EnvironmentBaseline,
                ))?;
        let target = candidate.target.clone();
        let authored_scopes = candidate.authored_scopes.clone();
        let replaces_environment = candidate.replacement_environment.is_some();
        let commands = candidate.accepted_commands.clone();
        self.check_baseline(&candidate, &opening_environment)?;
        let before = self.clone();
        let promotion = self.promote_inner(candidate)?;
        let environment_after = self.environment.clone();
        let revisions = revision_records(
            &before,
            self,
            &target,
            &authored_scopes,
            replaces_environment,
        )?;
        let terminal = commit_metadata_value(
            promotion.cursor,
            &target,
            &revisions,
            &opening_environment,
            &environment_after,
        )?;
        self.record_commit(DurableCommit {
            cursor: promotion.cursor,
            commit_id,
            epoch_id,
            request_digest,
            target,
            environment_before: opening_environment,
            revisions,
            commands,
            terminal: terminal.clone(),
        });
        Ok(Promotion {
            terminal: Some(terminal),
            ..promotion
        })
    }

    fn record_commit(&mut self, commit: DurableCommit) {
        for record in &commit.revisions {
            match (&record.authoring_after, &record.topology_after) {
                (Some(authoring), Some(topology)) => {
                    self.revisions.insert(
                        record.scope.clone(),
                        RecordedRevision {
                            authoring: authoring.clone(),
                            topology: topology.clone(),
                        },
                    );
                }
                _ => {
                    self.revisions.remove(&record.scope);
                }
            }
        }
        self.next_epoch = self
            .next_epoch
            .max(epoch_number(&commit.epoch_id).saturating_add(1))
            .max(1);
        self.dedup.insert(
            commit.commit_id.clone(),
            DedupTerminal {
                request_digest: commit.request_digest.clone(),
                terminal: commit.terminal.clone(),
            },
        );
        self.last = Some(commit);
    }

    fn promote_inner(&mut self, candidate: EpochCandidate) -> Result<Promotion, FoldRejection> {
        let mut staged = self.clone();
        staged.replace_target(candidate.clone());
        if let Some(replacement) = &candidate.replacement_environment {
            staged.environment = replacement.clone();
        }
        let plan = staged.current.assemble()?;
        self.refuse_touched(&staged, &plan)?;
        crate::validate_published_plan(&plan)
            .map_err(|error| FoldRejection::Registry(Box::new(error)))?;
        candidate.check_target_boundary()?;
        let revision = revision_digest(
            b"circular.authoring-revision.v1\0",
            &staged.environment,
            &staged.revision_items(&[], true)?,
        )?;
        staged.cursor = staged
            .cursor
            .checked_add(1)
            .ok_or(FoldRejection::CursorExhausted)?;
        let promotion = Promotion {
            plan,
            revision,
            cursor: staged.cursor,
            terminal: None,
        };
        *self = staged;
        Ok(promotion)
    }

    fn refuse_touched(
        &self,
        staged: &Self,
        plan: &crate::authoring_assembly::projection::AuthoredProjection,
    ) -> Result<(), FoldRejection> {
        if plan.refused().is_empty() {
            return Ok(());
        }
        let before = &self.current;
        let mut before_expanded = None;
        for refused in plan.refused().values() {
            let recorded = match before.tables().actors().get(&refused.key) {
                Some(declaration) => {
                    Some((declaration.clone(), before.actor_generation(&refused.key)))
                }
                None => before_expanded
                    .get_or_insert_with(|| before.expanded_templates().ok())
                    .as_ref()
                    .and_then(|expanded: &EpochCandidate| {
                        expanded
                            .tables()
                            .actors()
                            .get(&refused.key)
                            .map(|declaration| {
                                (declaration.clone(), expanded.actor_generation(&refused.key))
                            })
                    }),
            };
            if recorded != Some((refused.declaration.clone(), refused.generation)) {
                return Err(refused.rejection.clone());
            }
        }
        for (key, edge) in staged.current.tables().edges() {
            if before.tables().edges().get(key) == Some(edge) {
                continue;
            }
            for endpoint in [&edge.from.0, &edge.to.0] {
                if let Some(refused) = plan
                    .refused()
                    .values()
                    .find(|refused| &refused.key == endpoint)
                {
                    return Err(refused.rejection.clone());
                }
            }
        }
        Ok(())
    }

    pub fn committed_terminal(&self, commit_id: &[u8]) -> Result<Value, FoldRejection> {
        let commit = self.last.as_ref().ok_or(FoldRejection::NoCommitToAnswer)?;
        if commit.commit_id != commit_id {
            return Err(FoldRejection::TerminalNotNewest);
        }
        Ok(commit.terminal.clone())
    }

    #[must_use]
    pub fn request_digest(candidate: &EpochCandidate) -> Vec<u8> {
        let mut digest = Sha256::new();
        digest.update(b"circular.authoring-request.v1\0");
        for (verb, payload) in &candidate.request_parts {
            digest.update([*verb]);
            digest.update((payload.len() as u64).to_be_bytes());
            digest.update(payload);
        }
        digest.finalize().to_vec()
    }

    #[must_use]
    pub fn dedup_lookup(&self, commit_id: &[u8], request_digest: &[u8]) -> DedupLookup {
        let Some(entry) = self.dedup.get(commit_id) else {
            return DedupLookup::New;
        };
        if entry.request_digest == request_digest {
            DedupLookup::Repeat(entry.terminal.clone())
        } else {
            DedupLookup::Equivocation
        }
    }

    #[must_use]
    pub const fn last_commit(&self) -> Option<&DurableCommit> {
        self.last.as_ref()
    }

    #[must_use]
    pub fn commit_count(&self) -> usize {
        self.dedup.len()
    }

    #[cfg(test)]
    #[must_use]
    pub fn retained_commit_summary_bytes(&self) -> usize {
        self.dedup
            .iter()
            .map(|(commit_id, entry)| {
                commit_id.len()
                    + entry.request_digest.len()
                    + encode(&entry.terminal, Ceilings::for_boundary(Boundary::Journal))
                        .map_or(0, |bytes| bytes.len())
            })
            .sum()
    }

    #[must_use]
    pub fn scope_exists(&self, scope: &[ScopeSegment]) -> bool {
        scope.is_empty() || self.current.tables.scopes().contains(&scope.to_vec())
    }

    #[must_use]
    pub const fn next_epoch_number(&self) -> u64 {
        self.next_epoch
    }

    #[must_use]
    pub fn revision(&self) -> Option<&[u8]> {
        self.revisions
            .get::<[ScopeSegment]>(&[])
            .map(|recorded| recorded.authoring.as_slice())
    }

    #[must_use]
    pub const fn cursor(&self) -> u64 {
        self.cursor
    }

    #[must_use]
    pub fn environment(&self) -> &AuthoringEnvironment {
        &self.environment
    }

    #[must_use]
    pub fn current(&self) -> &EpochCandidate {
        &self.current
    }

    /// Rebuilds the runtime consumer from committed declarations, never a AuthoredProjection DTO.
    pub fn current_plan(&self) -> Result<AuthoredProjection, FoldRejection> {
        self.current.assemble()
    }

    /// Materializes the compacted reconstructive subset of the existing
    /// declaration union at one immutable consistency cut.
    pub fn snapshot(&self, scope: Vec<ScopeSegment>) -> Result<AuthoringSnapshot, FoldRejection> {
        let environment = self.environment.clone();
        if !scope.is_empty() && !self.current.tables.scopes().contains(&scope) {
            return Err(FoldRejection::SnapshotScopeMissing);
        }

        let items = self.materialize_items(&scope, true)?;
        let authoring_revision = self.recorded_revision(&scope, true)?.map(<[u8]>::to_vec);
        let topology_revision = self.recorded_revision(&scope, false)?.map(<[u8]>::to_vec);

        Ok(AuthoringSnapshot {
            scope,
            authoring_revision,
            topology_revision,
            cursor: self.cursor,
            environment,
            items,
        })
    }

    pub fn delta_rows(
        &self,
        delta: &Delta,
        scope: &[ScopeSegment],
    ) -> Result<Vec<Value>, FoldRejection> {
        let plan = self.current.assemble()?;
        let items = materialize_keyed_items(&plan, scope, &[], true)?;
        delta_rows::lower(delta, scope, items)
    }

    /// Encodes only the latest committed epoch for the append-only authoring
    /// journal.  There is no whole-state shape to encode: the journal is the
    /// only durable authoring fact and every entry is one accepted epoch.
    pub fn journal_epoch_value(&self) -> Result<Value, FoldRejection> {
        let commit = self.last.as_ref().ok_or(FoldRejection::NoCommittedEpoch)?;
        Value::object([
            ("entry_version", Value::Int(AUTHORING_JOURNAL_ENTRY_VERSION)),
            (EPOCH_ENTRY_FIELD, commit_value(commit)?),
        ])
        .map_err(encode_error(Encoded::JournalEpoch))
    }

    pub fn fold_journal_entry(
        previous: Option<Self>,
        entry: Value,
    ) -> Result<(Self, Delta), JournalEntryRejection> {
        match (ProjectCreation::from_journal_entry(&entry)?, previous) {
            (Some(creation), None) => Ok((Self::created(&creation), Delta::default())),
            (Some(_), Some(_)) => Err(PersistedFault::RepeatsProjectCreation.into()),
            (None, None) => Err(JournalEntryRejection::UnknownVocabulary(
                JournalVocabulary::NoProjectCreation,
            )),
            (None, Some(previous)) => replay_commit(previous, decode_journal_epoch(entry)?),
        }
    }

    fn materialize_items(
        &self,
        scope: &[ScopeSegment],
        include_presentation_and_annotation: bool,
    ) -> Result<Vec<Value>, FoldRejection> {
        let plan = self.current.assemble()?;
        materialize_plan_items(&plan, scope, include_presentation_and_annotation)
    }

    fn revision_items(
        &self,
        scope: &[ScopeSegment],
        include_presentation_and_annotation: bool,
    ) -> Result<Vec<Value>, FoldRejection> {
        let mut items = self.materialize_items(scope, include_presentation_and_annotation)?;
        items.extend(self.generation_values(scope)?);
        Ok(items)
    }

    fn generation_values(&self, scope: &[ScopeSegment]) -> Result<Vec<Value>, FoldRejection> {
        let mut values = self
            .current
            .tables
            .generations()
            .iter()
            .filter(|(key, generation)| generation.get() != 0 && is_at_or_below(&key.scope, scope))
            .map(|(key, generation)| {
                Value::object([
                    (
                        "generation",
                        Value::Bytes(generation.get().to_be_bytes().to_vec()),
                    ),
                    (
                        "actor",
                        circular_protocol::boundary_port::encode_boundary_actor_key(key)
                            .map_err(FoldRejection::BoundaryActorKey)?,
                    ),
                ])
                .map_err(encode_error(Encoded::ActorGenerationFact))
            })
            .collect::<Result<Vec<_>, _>>()?;
        sort_values(&mut values)?;
        Ok(values)
    }

    fn recorded_revision(
        &self,
        scope: &[ScopeSegment],
        authoring: bool,
    ) -> Result<Option<&[u8]>, FoldRejection> {
        match self.revisions.get(scope) {
            Some(recorded) => Ok(Some(if authoring {
                recorded.authoring.as_slice()
            } else {
                recorded.topology.as_slice()
            })),
            None if self.has_revision(scope) => Err(PersistedFault::CommitWithoutRevision {
                scope: scope.to_vec(),
            }
            .into()),
            None => Ok(None),
        }
    }

    fn has_revision(&self, scope: &[ScopeSegment]) -> bool {
        self.cursor != 0 && self.scope_exists(scope)
    }

    fn computed_revision(
        &self,
        scope: &[ScopeSegment],
        include_presentation_and_annotation: bool,
    ) -> Result<Option<Vec<u8>>, FoldRejection> {
        if !self.has_revision(scope) {
            return Ok(None);
        }
        let environment = &self.environment;
        let items = self.revision_items(scope, include_presentation_and_annotation)?;
        revision_digest(
            if include_presentation_and_annotation {
                b"circular.authoring-revision.v1\0"
            } else {
                b"circular.topology-revision.v1\0"
            },
            environment,
            &items,
        )
        .map(Some)
    }

    fn check_baseline(
        &self,
        candidate: &EpochCandidate,
        opening_environment: &AuthoringEnvironment,
    ) -> Result<(), FoldRejection> {
        let expected =
            candidate
                .expected_revision
                .as_ref()
                .ok_or(FoldRejection::BracketIncomplete(
                    BracketField::RevisionBaseline,
                ))?;
        let current = self
            .recorded_revision(&candidate.target, true)?
            .map(<[u8]>::to_vec);
        match (expected, &current) {
            (ExpectedRevision::Absent, None) => {}
            (ExpectedRevision::At(bytes), Some(current)) if bytes == current => {}
            (ExpectedRevision::Absent | ExpectedRevision::At(_), _) => {
                return Err(FoldRejection::RevisionConflict {
                    expected: expected.clone(),
                    current: current.map_or(ExpectedRevision::Absent, ExpectedRevision::At),
                });
            }
        }
        if &self.environment != opening_environment {
            return Err(FoldRejection::EnvironmentChanged);
        }
        Ok(())
    }

    fn replace_target(&mut self, candidate: EpochCandidate) {
        let target = candidate.target.clone();
        self.current.tables.splice(&target, candidate.tables);
    }
}

#[derive(Default)]
struct ReconstructionItems {
    scopes: Vec<(RowKey, Value)>,
    actors: Vec<(RowKey, Value)>,
    edges: Vec<(RowKey, Value)>,
    exports: Vec<(RowKey, Value)>,
    annotations: Vec<(RowKey, Value)>,
    presentations: Vec<(RowKey, Value)>,
}

impl ReconstructionItems {
    fn extend(&mut self, child: Self) {
        self.scopes.extend(child.scopes);
        self.actors.extend(child.actors);
        self.edges.extend(child.edges);
        self.exports.extend(child.exports);
        self.annotations.extend(child.annotations);
        self.presentations.extend(child.presentations);
    }
}

fn materialize_plan_items(
    plan: &AuthoredProjection,
    scope: &[ScopeSegment],
    include_presentation_and_annotation: bool,
) -> Result<Vec<Value>, FoldRejection> {
    Ok(
        materialize_keyed_items(plan, scope, scope, include_presentation_and_annotation)?
            .into_iter()
            .map(|(_, item)| item)
            .collect(),
    )
}

fn materialize_keyed_items(
    plan: &AuthoredProjection,
    scope: &[ScopeSegment],
    root: &[ScopeSegment],
    include_presentation_and_annotation: bool,
) -> Result<Vec<(RowKey, Value)>, FoldRejection> {
    let generated = fold_projection::<Vec<ScopeId>>(plan, |layer| {
        let mut roots = layer
            .actors()
            .iter()
            .filter(|(_, decl)| {
                decl.domain().actor_type().is_container()
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
                    .expect("admitted depth")
            })
            .collect::<Vec<_>>();
        for child in layer.into_scopes().into_values() {
            roots.extend(child);
        }
        roots
    });
    let encoder = AuthoringSnapshotEncoder::new(root);
    let folded = fold_projection::<Result<ReconstructionItems, FoldRejection>>(plan, |layer| {
        if generated
            .iter()
            .any(|root| layer.scope().segments().starts_with(root.segments()))
        {
            return Ok(ReconstructionItems::default());
        }
        let mut items = ReconstructionItems::default();
        let layer_scope = authored_scope_from_plan(layer.scope())?;
        let declaration = scope_declaration_from_plan(layer.graph().declaration())?;
        let actors = layer.actors().clone();
        let edges = layer.edges().clone();
        let exports = layer.exports().clone();
        let annotations = layer.annotations().clone();
        let presentations = layer.presentation().clone();
        for child in layer.into_scopes().into_values() {
            items.extend(child?);
        }

        let scope_row = RowKey::Scope(layer_scope.clone());
        if scope_row.within(scope) {
            let item = encoder
                .upsert_scope(&layer_scope, &declaration)
                .map_err(FoldRejection::SnapshotEncode)?;
            items.scopes.push((scope_row, item));
        }
        if delta_rows::layer_within(&layer_scope, scope) {
            for (actor, declaration) in actors {
                let key = authored_actor_from_plan(&actor)?;
                let item = encoder
                    .upsert_actor(&key, &actor_declaration_from_plan(&declaration)?)
                    .map_err(FoldRejection::SnapshotEncode)?;
                items.actors.push((RowKey::Actor(key), item));
            }
            for declaration in edges.into_values() {
                let declaration = edge_declaration_from_plan(&declaration)?;
                let item = encoder
                    .upsert_edge(&declaration)
                    .map_err(FoldRejection::SnapshotEncode)?;
                let key = DeclaredEdgeKey {
                    from: declaration.from,
                    to: declaration.to,
                    ordinal: declaration.ordinal,
                };
                items.edges.push((RowKey::Edge(key), item));
            }
            for (name, declaration) in exports {
                let key = circular_protocol::declaration_payload::PlanExportKey {
                    scope: layer_scope.clone(),
                    local: name.name().as_str().to_owned(),
                };
                let item = encoder
                    .upsert_export(&key, &export_declaration_from_plan(&declaration)?)
                    .map_err(FoldRejection::SnapshotEncode)?;
                items.exports.push((RowKey::Export(key), item));
            }
            if include_presentation_and_annotation {
                for (name, declaration) in annotations {
                    let key = circular_protocol::declaration_payload::PlanAnnotationKey {
                        scope: layer_scope.clone(),
                        local: name.name().as_str().to_owned(),
                    };
                    let item = encoder
                        .upsert_annotation(&key, &annotation_declaration_from_plan(&declaration)?)
                        .map_err(FoldRejection::SnapshotEncode)?;
                    items.annotations.push((RowKey::Annotation(key), item));
                }
                for (owner, presentation) in presentations {
                    let key = owner.try_map(
                        |actor| authored_actor_from_plan(&actor),
                        |note| {
                            Ok(circular_protocol::declaration_payload::PlanAnnotationKey {
                                scope: layer_scope.clone(),
                                local: note.name().as_str().to_owned(),
                            })
                        },
                    )?;
                    let item = encoder
                        .set_presentation(&key, &presentation_declaration_from_plan(&presentation)?)
                        .map_err(FoldRejection::SnapshotEncode)?;
                    items.presentations.push((RowKey::Presentation(key), item));
                }
            }
        }
        Ok(items)
    })?;

    let ReconstructionItems {
        mut scopes,
        mut actors,
        mut edges,
        mut exports,
        mut annotations,
        mut presentations,
    } = folded;
    let mut items = Vec::new();
    for template in plan.templates().values() {
        let key = RowKey::Template(template.name().clone());
        if key.within(scope) {
            let item = encoder
                .upsert_template(template.name().as_str(), &template.commands())
                .map_err(FoldRejection::SnapshotEncode)?;
            items.push((key, item));
        }
    }
    for phase in [
        &mut scopes,
        &mut actors,
        &mut edges,
        &mut exports,
        &mut annotations,
        &mut presentations,
    ] {
        sort_items(phase)?;
        items.append(phase);
    }
    Ok(items)
}

fn authored_scope_from_plan(scope: &ScopeId) -> Result<Vec<ScopeSegment>, FoldRejection> {
    if scope
        .segments()
        .iter()
        .any(|segment| matches!(segment, ScopeSeg::Instance { .. }))
    {
        return Err(FoldRejection::InstanceScopeNotReconstructible);
    }
    Ok(circular_runtime::product_identity::wire_scope(scope))
}

fn authored_actor_from_plan(actor: &NamedActorId) -> Result<PlanActorKey, FoldRejection> {
    authored_scope_from_plan(actor.scope())?;
    Ok(circular_runtime::product_identity::wire_named_actor(actor))
}

fn endpoint_from_plan(endpoint: &PlanEndpoint) -> Result<(PlanActorKey, String), FoldRejection> {
    Ok((
        authored_actor_from_plan(endpoint.actor())?,
        endpoint.port().as_str().to_owned(),
    ))
}

fn scope_declaration_from_plan(
    declaration: &circular_plan::ScopeDeclaration,
) -> Result<circular_protocol::declaration_payload::ScopeDeclaration, FoldRejection> {
    let role = match declaration.role() {
        PlanScopeRole::Concrete => circular_protocol::declaration_payload::ScopeRole::Concrete,
        PlanScopeRole::Template => circular_protocol::declaration_payload::ScopeRole::Template,
    };
    let boundary = circular_protocol::declaration_payload::ScopeBoundary {
        inlets: declaration
            .boundary()
            .inlets()
            .iter()
            .map(|(outer, inner)| {
                Ok(circular_protocol::declaration_payload::ScopeBinding {
                    inner: endpoint_from_plan(inner)?,
                    outer: outer.as_str().to_owned(),
                })
            })
            .collect::<Result<Vec<_>, FoldRejection>>()?,
        outlets: declaration
            .boundary()
            .outlets()
            .iter()
            .map(|(outer, inner)| {
                Ok(circular_protocol::declaration_payload::ScopeBinding {
                    inner: endpoint_from_plan(inner)?,
                    outer: outer.as_str().to_owned(),
                })
            })
            .collect::<Result<Vec<_>, FoldRejection>>()?,
    };
    Ok(circular_protocol::declaration_payload::ScopeDeclaration { role, boundary })
}

fn actor_declaration_from_plan(
    declaration: &circular_plan::ActorDecl,
) -> Result<circular_protocol::declaration_payload::ActorDeclaration, FoldRejection> {
    let flags = declaration.flags();
    Ok(circular_protocol::declaration_payload::ActorDeclaration {
        actor_type: declaration.domain().actor_type().as_str().to_owned(),
        config: config_from_plan(declaration.domain().config())?,
        flags: circular_protocol::declaration_payload::ActorFlags {
            bypass: flags.bypass(),
            mute: flags.mute(),
            pause: flags.pause(),
        },
    })
}

fn config_from_plan(config: &PlanConfig) -> Result<Value, FoldRejection> {
    if config.record().entries().is_empty() {
        return Ok(Value::Null);
    }
    config_record_from_plan(config.record())
}

fn config_record_from_plan(record: &PlanConfigRecord) -> Result<Value, FoldRejection> {
    record.to_wire_value().map_err(FoldRejection::ConfigToWire)
}

fn config_value_from_plan(value: &PlanConfigValue) -> Result<Value, FoldRejection> {
    value.to_wire_value().map_err(FoldRejection::ConfigToWire)
}

fn edge_declaration_from_plan(
    declaration: &circular_plan::EdgeDecl,
) -> Result<circular_protocol::declaration_payload::EdgeDeclaration, FoldRejection> {
    Ok(circular_protocol::declaration_payload::EdgeDeclaration {
        from: endpoint_from_plan(declaration.from())?,
        to: endpoint_from_plan(declaration.to())?,
        ordinal: declaration.ordinal(),
        attrs: declaration.attrs(),
    })
}

fn export_declaration_from_plan(
    declaration: &circular_plan::Export,
) -> Result<circular_protocol::declaration_payload::ExportDeclaration, FoldRejection> {
    let mut roles = circular_protocol::declaration_payload::ExportRoles::default();
    for (role, mount) in declaration.roles() {
        let endpoint = (
            authored_actor_from_plan(mount.actor())?,
            mount.port().as_str().to_owned(),
        );
        match role {
            PlanRole::Request => roles.request = Some(endpoint),
            PlanRole::Progress => roles.progress = Some(endpoint),
            PlanRole::Result => roles.result = Some(endpoint),
            PlanRole::Error => roles.error = Some(endpoint),
        }
    }
    let operations = if declaration
        .operations()
        .config()
        .record()
        .entries()
        .is_empty()
    {
        None
    } else {
        Some(config_from_plan(declaration.operations().config())?)
    };
    let surface = declaration
        .surface()
        .map(config_value_from_plan)
        .transpose()?;
    Ok(circular_protocol::declaration_payload::ExportDeclaration {
        roles,
        operations,
        surface,
    })
}

fn annotation_declaration_from_plan(
    declaration: &circular_plan::Annotation,
) -> Result<circular_protocol::declaration_payload::AnnotationDeclaration, FoldRejection> {
    if !declaration.placement().is_unplaced() {
        return Err(FoldRejection::AnnotationPlacementUncarried);
    }
    let kind = match declaration.kind() {
        PlanAnnotationKind::Note => circular_protocol::declaration_payload::AnnotationKind::Note,
        PlanAnnotationKind::Backdrop => {
            circular_protocol::declaration_payload::AnnotationKind::Backdrop
        }
    };
    let refs = declaration
        .refs()
        .iter()
        .map(authored_actor_from_plan)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(
        circular_protocol::declaration_payload::AnnotationDeclaration {
            kind,
            refs,
            body: declaration.body().as_str().to_owned(),
        },
    )
}

pub(super) fn presentation_declaration_from_plan(
    presentation: &circular_plan::Presentation,
) -> Result<circular_protocol::declaration_payload::Presentation<PlanActorKey>, FoldRejection> {
    let anchor = presentation
        .anchor()
        .map(
            |anchor| -> Result<
                circular_protocol::declaration_payload::Anchor<PlanActorKey>,
                FoldRejection,
            > {
                Ok(match anchor {
                    PlanAnchor::Flow => circular_protocol::declaration_payload::Anchor::Flow,
                    PlanAnchor::Relative { target, relation } => {
                        circular_protocol::declaration_payload::Anchor::Relative {
                            target: authored_actor_from_plan(target)?,
                            relation: *relation,
                        }
                    }
                    PlanAnchor::Align { target, axis } => {
                        circular_protocol::declaration_payload::Anchor::Align {
                            target: authored_actor_from_plan(target)?,
                            axis: *axis,
                        }
                    }
                })
            },
        )
        .transpose()?;
    Ok(circular_protocol::declaration_payload::Presentation {
        label: presentation.label.clone(),
        group: presentation.group.clone(),
        anchor,
        fixed: presentation.fixed,
        size: presentation.size,
        board: presentation.board,
        view: presentation.view.clone(),
        collapsed: presentation.collapsed,
    })
}

const AUTHORING_JOURNAL_ENTRY_VERSION: i64 = 2;

fn epoch_number(epoch_id: &[u8]) -> u64 {
    <[u8; 8]>::try_from(epoch_id).map_or(0, u64::from_be_bytes)
}

fn decode_journal_epoch(value: Value) -> Result<DurableCommit, JournalEntryRejection> {
    let Value::Object(object) = value else {
        return Err(PersistedFault::EntryNotObject.into());
    };
    let mut fields = object.into_map();
    if take_field(&mut fields, "entry_version")? != Value::Int(AUTHORING_JOURNAL_ENTRY_VERSION) {
        return Err(JournalEntryRejection::UnknownVocabulary(
            JournalVocabulary::EntryVersion,
        ));
    }
    let epoch = decode_commit(take_field(&mut fields, EPOCH_ENTRY_FIELD)?)?;
    if let Some((unknown, _)) = fields.into_iter().next() {
        return Err(JournalEntryRejection::UnknownVocabulary(
            JournalVocabulary::EntryField(unknown),
        ));
    }
    Ok(epoch)
}

fn replay_commit(
    mut state: AuthoringState,
    expected: DurableCommit,
) -> Result<(AuthoringState, Delta), JournalEntryRejection> {
    let next_cursor = state
        .cursor
        .checked_add(1)
        .ok_or(PersistedFault::ReplayCursorExhausted)?;
    if expected.cursor != next_cursor {
        return Err(PersistedFault::ReplayCursorMismatch {
            found: expected.cursor,
            expected: next_cursor,
        }
        .into());
    }
    if state.dedup.contains_key(&expected.commit_id) {
        return Err(PersistedFault::RepeatsCommitId.into());
    }
    if expected.root_revision().is_none() {
        return Err(PersistedFault::CommitWithoutRevision { scope: Vec::new() }.into());
    }
    let environment = expected.environment_after()?;

    let begin = BeginEpoch {
        scope: circular_protocol::declaration_payload::AddressRef::Absolute(
            expected.target.clone(),
        ),
        commit_id: expected.commit_id.clone(),
        expected_revision: expected
            .opening_revision()
            .map_or(ExpectedRevision::Absent, |revision| {
                ExpectedRevision::At(revision.to_vec())
            }),
        expected_environment: expected.environment_before.clone(),
    };
    let mut candidate = state.begin_candidate(&begin, &[], 0)?;
    let mut delta = Delta::default();
    for item in &expected.commands {
        let verb = ContentVerb::from_accepted(item)?;
        let applied = candidate
            .apply(&verb)
            .map_err(|rejection| match rejection {
                FoldRejection::MoveToScope(fault) => {
                    PersistedFault::CommandMoveToScope(fault).into()
                }
                rejection => JournalEntryRejection::from(rejection),
            })?;
        delta = delta.then(applied);
    }

    state.replace_target(candidate);
    state.environment = environment;
    state.cursor = expected.cursor;
    state.record_commit(expected);
    Ok((state, delta))
}

impl DurableCommit {
    pub fn from_journal_entry(value: Value) -> Result<Self, JournalEntryRejection> {
        decode_journal_epoch(value)
    }

    pub fn environment_after(&self) -> Result<AuthoringEnvironment, JournalEntryRejection> {
        decode_commit_metadata(&self.terminal).map(|(.., after)| after)
    }

    fn opening_revision(&self) -> Option<&[u8]> {
        self.revisions
            .iter()
            .find(|record| record.scope == self.target)
            .and_then(|record| record.authoring_before.as_deref())
    }

    fn root_revision(&self) -> Option<&[u8]> {
        self.revisions
            .iter()
            .find(|record| record.scope.is_empty())
            .and_then(|record| record.authoring_after.as_deref())
    }

    #[must_use]
    pub fn affects_scope(&self, scope: &[ScopeSegment]) -> bool {
        self.revisions.iter().any(|record| record.scope == scope) || self.replaces_environment()
    }

    fn replaces_environment(&self) -> bool {
        let Value::Object(object) = &self.terminal else {
            return false;
        };
        match (
            object.get("before_environment"),
            object.get("after_environment"),
        ) {
            (Some(before), Some(after)) => before != after,
            _ => false,
        }
    }

    #[must_use]
    pub fn retires_scope(&self, scope: &[ScopeSegment]) -> bool {
        self.revisions.iter().any(|record| {
            record.scope == scope
                && record.authoring_before.is_some()
                && record.authoring_after.is_none()
        })
    }

    pub fn frame_value(&self, delta: Vec<Value>) -> Result<Value, FoldRejection> {
        let expected_revision = self.opening_revision();
        let begin = Value::object([
            ("commit_id", Value::Bytes(self.commit_id.clone())),
            (
                "expected_environment",
                circular_protocol::authoring_snapshot::environment_value(&self.environment_before)
                    .map_err(FoldRejection::SnapshotEncode)?,
            ),
            (
                "expected_revision",
                circular_protocol::authoring_snapshot::current_revision_value(expected_revision),
            ),
            ("kind", Value::String("BeginEpoch".to_owned())),
            (
                "scope",
                Value::array([
                    Value::Int(1),
                    circular_protocol::authoring_snapshot::scope_identity_value(&self.target),
                ]),
            ),
        ])
        .map_err(encode_error(Encoded::BeginEpoch))?;
        let terminal = Value::object([
            ("epoch", Value::Bytes(self.epoch_id.clone())),
            ("kind", Value::String("CommitEpoch".to_owned())),
        ])
        .map_err(encode_error(Encoded::CommitEpoch))?;
        let epoch = Value::object([
            ("begin", begin),
            ("content", Value::Array(self.commands.clone())),
            ("terminal", terminal),
        ])
        .map_err(encode_error(Encoded::RpcEpoch))?;
        Value::object([
            ("delta", Value::Array(delta)),
            ("epoch", epoch),
            ("metadata", self.terminal.clone()),
        ])
        .map_err(encode_error(Encoded::CommitFrame))
    }
}

fn address_identity(
    address: &circular_protocol::declaration_payload::ScopeAddress,
) -> &Vec<ScopeSegment> {
    use circular_protocol::declaration_payload::AddressRef;
    match address {
        AddressRef::Absolute(identity)
        | AddressRef::EpochLocal(identity)
        | AddressRef::Relative(identity) => identity,
    }
}

fn take_field(
    fields: &mut std::collections::BTreeMap<String, Value>,
    field: &'static str,
) -> Result<Value, PersistedFault> {
    fields
        .remove(field)
        .ok_or(PersistedFault::FieldAbsent { field })
}

fn bytes_field(value: Value, field: &'static str) -> Result<Vec<u8>, PersistedFault> {
    match value {
        Value::Bytes(bytes) => Ok(bytes),
        _ => Err(PersistedFault::FieldNotBytes { field }),
    }
}

fn non_negative_u64(value: Value, field: &'static str) -> Result<u64, PersistedFault> {
    match value {
        Value::Int(value) if value >= 0 => {
            u64::try_from(value).map_err(|_| PersistedFault::ExceedsU64 { field })
        }
        _ => Err(PersistedFault::NotNonNegativeInt { field }),
    }
}

fn encode_error(what: Encoded) -> impl FnOnce(DuplicateKeyError) -> FoldRejection {
    move |error| FoldRejection::Encode { what, error }
}

fn codec_error(what: Coded) -> impl FnOnce(CodecError) -> FoldRejection {
    move |error| FoldRejection::Codec { what, error }
}

fn commit_value(commit: &DurableCommit) -> Result<Value, FoldRejection> {
    Value::object([
        ("commands", Value::Array(commit.commands.clone())),
        ("commit_id", Value::Bytes(commit.commit_id.clone())),
        ("epoch_id", Value::Bytes(commit.epoch_id.clone())),
        (
            "request_digest",
            Value::Bytes(commit.request_digest.clone()),
        ),
        ("terminal", commit.terminal.clone()),
    ])
    .map_err(encode_error(Encoded::Commit))
}

fn decode_commit_metadata(
    terminal: &Value,
) -> Result<
    (
        u64,
        Vec<ScopeSegment>,
        Vec<RevisionRecord>,
        AuthoringEnvironment,
        AuthoringEnvironment,
    ),
    JournalEntryRejection,
> {
    let Value::Object(object) = terminal else {
        return Err(PersistedFault::TerminalNotObject.into());
    };
    let mut fields = object.clone().into_map();
    let environment_after = decode_environment(take_field(&mut fields, "after_environment")?)
        .map_err(|error| PersistedFault::TerminalEnvironment {
            side: EnvironmentSide::After,
            error,
        })?;
    let environment_before = decode_environment(take_field(&mut fields, "before_environment")?)
        .map_err(|error| PersistedFault::TerminalEnvironment {
            side: EnvironmentSide::Before,
            error,
        })?;
    let cursor = non_negative_u64(take_field(&mut fields, "cursor")?, "commit cursor")?;
    let revisions = match take_field(&mut fields, "revisions")? {
        Value::Array(revisions) => revisions
            .into_iter()
            .map(decode_revision_record)
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(PersistedFault::TerminalRevisionsNotArray.into()),
    };
    if revisions.is_empty() {
        return Err(PersistedFault::CommitWithoutRevision { scope: Vec::new() }.into());
    }
    let target = decode_scope_identity(take_field(&mut fields, "target_scope")?)
        .map_err(PersistedFault::CommitTarget)?;
    if let Some((unknown, _)) = fields.into_iter().next() {
        return Err(JournalEntryRejection::UnknownVocabulary(
            JournalVocabulary::TerminalField(unknown),
        ));
    }
    Ok((
        cursor,
        target,
        revisions,
        environment_before,
        environment_after,
    ))
}

fn decode_commit(value: Value) -> Result<DurableCommit, JournalEntryRejection> {
    let Value::Object(object) = value else {
        return Err(PersistedFault::CommitNotObject.into());
    };
    let mut fields = object.into_map();
    let commands = match take_field(&mut fields, "commands")? {
        Value::Array(commands) => commands,
        _ => return Err(PersistedFault::CommitCommandsNotArray.into()),
    };
    for command in &commands {
        let Value::Object(command) = command else {
            return Err(PersistedFault::CommandNotObject.into());
        };
        if !matches!(command.get("kind"), Some(Value::String(_))) {
            return Err(PersistedFault::CommandWithoutKind.into());
        }
    }
    let commit_id = bytes_field(take_field(&mut fields, "commit_id")?, "commit_id")?;
    let epoch_id = bytes_field(take_field(&mut fields, "epoch_id")?, "epoch_id")?;
    let request_digest = bytes_field(take_field(&mut fields, "request_digest")?, "request_digest")?;
    let terminal = take_field(&mut fields, "terminal")?;
    if let Some((unknown, _)) = fields.into_iter().next() {
        return Err(JournalEntryRejection::UnknownVocabulary(
            JournalVocabulary::CommitField(unknown),
        ));
    }
    let (cursor, target, revisions, environment_before, environment_after) =
        decode_commit_metadata(&terminal)?;
    let expected_terminal = commit_metadata_value(
        cursor,
        &target,
        &revisions,
        &environment_before,
        &environment_after,
    )?;
    if terminal != expected_terminal {
        return Err(PersistedFault::CommitTerminalDisagrees.into());
    }
    Ok(DurableCommit {
        cursor,
        commit_id,
        epoch_id,
        request_digest,
        target,
        environment_before,
        revisions,
        commands,
        terminal,
    })
}

fn decode_current_revision(
    value: Value,
    field: &'static str,
) -> Result<Option<Vec<u8>>, PersistedFault> {
    match value {
        Value::Int(1) => Ok(None),
        Value::Array(mut parts) if parts.len() == 2 => {
            let revision = match parts.pop().expect("length checked") {
                Value::Bytes(bytes) if bytes.len() == 32 => bytes,
                _ => return Err(PersistedFault::RevisionNot32Bytes { field }),
            };
            if parts.pop() != Some(Value::Int(2)) {
                return Err(PersistedFault::RevisionUnknownArm { field });
            }
            Ok(Some(revision))
        }
        _ => Err(PersistedFault::RevisionNotSum { field }),
    }
}

fn decode_revision_record(value: Value) -> Result<RevisionRecord, PersistedFault> {
    let Value::Object(object) = value else {
        return Err(PersistedFault::TransitionNotObject);
    };
    let mut fields = object.into_map();
    let authoring_after = decode_current_revision(
        take_field(&mut fields, "authoring_after")?,
        "authoring_after",
    )?;
    let authoring_before = decode_current_revision(
        take_field(&mut fields, "authoring_before")?,
        "authoring_before",
    )?;
    let scope = decode_scope_identity(take_field(&mut fields, "scope")?)
        .map_err(PersistedFault::TransitionScope)?;
    let topology_after =
        decode_current_revision(take_field(&mut fields, "topology_after")?, "topology_after")?;
    let topology_before = decode_current_revision(
        take_field(&mut fields, "topology_before")?,
        "topology_before",
    )?;
    if let Some((unknown, _)) = fields.into_iter().next() {
        return Err(PersistedFault::TransitionUnknownField { field: unknown });
    }
    if authoring_after.is_some() != topology_after.is_some()
        || authoring_before.is_some() != topology_before.is_some()
    {
        return Err(PersistedFault::CommitWithoutRevision { scope });
    }
    Ok(RevisionRecord {
        scope,
        authoring_before,
        authoring_after,
        topology_before,
        topology_after,
    })
}

#[cfg(test)]
fn fold_snapshot_item(candidate: &mut EpochCandidate, item: Value) -> Result<(), String> {
    let kind = match &item {
        Value::Object(object) => match object.get("kind") {
            Some(Value::String(kind)) => kind,
            _ => return Err("persisted declaration item has no String kind".to_owned()),
        },
        _ => return Err("persisted declaration item is not an object".to_owned()),
    };
    let verb = ContentVerb::from_snapshot_item(&item).map_err(|rejection| match rejection {
        CompactedDeclarationRejection::Payload(error) => format!("persisted {kind}: {error:?}"),
        CompactedDeclarationRejection::UnknownKind(kind) => {
            format!("unknown persisted reconstructive command {kind:?}")
        }
    })?;
    candidate
        .apply(&verb)
        .map(drop)
        .map_err(|error| error.to_string())
}

fn is_at_or_below(candidate: &[ScopeSegment], root: &[ScopeSegment]) -> bool {
    candidate.starts_with(root)
}

fn sort_values(values: &mut [Value]) -> Result<(), FoldRejection> {
    sort_by_value_bytes(values, |value| value)
}

fn sort_items(items: &mut [(RowKey, Value)]) -> Result<(), FoldRejection> {
    sort_by_value_bytes(items, |(_, value)| value)
}

fn sort_by_value_bytes<T: Clone>(
    items: &mut [T],
    value: impl Fn(&T) -> &Value,
) -> Result<(), FoldRejection> {
    let mut keyed = items
        .iter()
        .map(|item| {
            let bytes = encode(value(item), Ceilings::for_boundary(Boundary::Identity))
                .map_err(codec_error(Coded::SnapshotItem))?;
            Ok((bytes, item.clone()))
        })
        .collect::<Result<Vec<_>, FoldRejection>>()?;
    keyed.sort_by(|left, right| left.0.cmp(&right.0));
    for (destination, (_, item)) in items.iter_mut().zip(keyed) {
        *destination = item;
    }
    Ok(())
}

fn revision_digest(
    domain: &[u8],
    environment: &AuthoringEnvironment,
    items: &[Value],
) -> Result<Vec<u8>, FoldRejection> {
    let mut digest = Sha256::new();
    digest.update(domain);
    hash_part(&mut digest, &environment.declaration_schema);
    hash_part(&mut digest, &environment.spec_set);
    digest.update(
        u64::try_from(items.len())
            .map_err(|_| FoldRejection::RevisionItemsExceedCounter)?
            .to_be_bytes(),
    );
    for item in items {
        let bytes = encode(item, Ceilings::for_boundary(Boundary::Identity))
            .map_err(codec_error(Coded::RevisionInput))?;
        hash_part(&mut digest, &bytes);
    }
    Ok(digest.finalize().to_vec())
}

fn scope_prefixes(scope: &[ScopeSegment]) -> impl Iterator<Item = Vec<ScopeSegment>> + '_ {
    (0..=scope.len()).map(|len| scope[..len].to_vec())
}

fn push_scope(scopes: &mut Vec<Vec<ScopeSegment>>, scope: Vec<ScopeSegment>) {
    if !scopes.contains(&scope) {
        scopes.push(scope);
    }
}

fn declared_scopes<'a>(
    before: &'a AuthoringState,
    after: &'a AuthoringState,
) -> impl Iterator<Item = &'a Vec<ScopeSegment>> + 'a {
    before
        .current
        .tables
        .scopes()
        .keys()
        .chain(after.current.tables.scopes().keys())
}

fn target_lineage(target: &[ScopeSegment]) -> impl Iterator<Item = Vec<ScopeSegment>> + '_ {
    scope_prefixes(target)
}

fn authoring_event_scopes(
    before: &AuthoringState,
    after: &AuthoringState,
    target: &[ScopeSegment],
    authored_scopes: &[Vec<ScopeSegment>],
    replaces_environment: bool,
) -> Vec<Vec<ScopeSegment>> {
    let mut affected = Vec::new();
    if replaces_environment {
        push_scope(&mut affected, Vec::new());
        for scope in declared_scopes(before, after) {
            push_scope(&mut affected, scope.clone());
        }
        return affected;
    }
    for scope in authored_scopes {
        for prefix in scope_prefixes(scope) {
            push_scope(&mut affected, prefix);
        }
    }
    for scope in target_lineage(target) {
        push_scope(&mut affected, scope);
    }
    affected
}

fn hash_part(digest: &mut Sha256, bytes: &[u8]) {
    digest.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
    digest.update(bytes);
}

fn revision_records(
    before: &AuthoringState,
    after: &AuthoringState,
    target: &[ScopeSegment],
    authored_scopes: &[Vec<ScopeSegment>],
    replaces_environment: bool,
) -> Result<Vec<RevisionRecord>, FoldRejection> {
    let mut scopes = target_lineage(target).collect::<Vec<_>>();
    for scope in declared_scopes(before, after) {
        push_scope(&mut scopes, scope.clone());
    }
    let mut encoded_scopes = scopes
        .into_iter()
        .map(|scope| {
            let encoded = encode(
                &circular_protocol::authoring_snapshot::scope_identity_value(&scope),
                Ceilings::for_boundary(Boundary::Identity),
            )
            .map_err(codec_error(Coded::RevisionScopeIdentity))?;
            Ok((encoded, scope))
        })
        .collect::<Result<Vec<_>, FoldRejection>>()?;
    encoded_scopes.sort_by(|(left, _), (right, _)| left.cmp(right));
    let scopes = encoded_scopes
        .into_iter()
        .map(|(_, scope)| scope)
        .collect::<Vec<_>>();

    let authored =
        authoring_event_scopes(before, after, target, authored_scopes, replaces_environment);
    let mut records = Vec::new();
    for scope in scopes {
        let authoring_before = before.recorded_revision(&scope, true)?.map(<[u8]>::to_vec);
        let authoring_after = after.computed_revision(&scope, true)?;
        let topology_before = before.recorded_revision(&scope, false)?.map(<[u8]>::to_vec);
        let topology_after = after.computed_revision(&scope, false)?;
        let authoring_changed = authored.contains(&scope);
        let topology_changed = topology_before != topology_after;
        if !authoring_changed && !topology_changed {
            continue;
        }
        let record = RevisionRecord {
            authoring_before,
            authoring_after,
            topology_before,
            topology_after,
            scope,
        };
        records.push(record);
    }
    if records.is_empty() {
        return Err(FoldRejection::NoRevisionTransition);
    }
    Ok(records)
}

fn revision_record_value(record: &RevisionRecord) -> Result<Value, FoldRejection> {
    Value::object([
        (
            "authoring_after",
            circular_protocol::authoring_snapshot::current_revision_value(
                record.authoring_after.as_deref(),
            ),
        ),
        (
            "authoring_before",
            circular_protocol::authoring_snapshot::current_revision_value(
                record.authoring_before.as_deref(),
            ),
        ),
        (
            "scope",
            circular_protocol::authoring_snapshot::scope_identity_value(&record.scope),
        ),
        (
            "topology_after",
            circular_protocol::authoring_snapshot::current_revision_value(
                record.topology_after.as_deref(),
            ),
        ),
        (
            "topology_before",
            circular_protocol::authoring_snapshot::current_revision_value(
                record.topology_before.as_deref(),
            ),
        ),
    ])
    .map_err(encode_error(Encoded::RevisionTransition))
}

fn commit_metadata_value(
    cursor: u64,
    target: &[ScopeSegment],
    revisions: &[RevisionRecord],
    before_environment: &AuthoringEnvironment,
    after_environment: &AuthoringEnvironment,
) -> Result<Value, FoldRejection> {
    let revisions = revisions
        .iter()
        .map(revision_record_value)
        .collect::<Result<Vec<_>, _>>()?;
    Value::object([
        (
            "after_environment",
            circular_protocol::authoring_snapshot::environment_value(after_environment)
                .map_err(FoldRejection::SnapshotEncode)?,
        ),
        (
            "before_environment",
            circular_protocol::authoring_snapshot::environment_value(before_environment)
                .map_err(FoldRejection::SnapshotEncode)?,
        ),
        (
            "cursor",
            Value::Int(i64::try_from(cursor).map_err(|_| FoldRejection::CommitCursorExceedsInt)?),
        ),
        ("revisions", Value::Array(revisions)),
        (
            "target_scope",
            circular_protocol::authoring_snapshot::scope_identity_value(target),
        ),
    ])
    .map_err(encode_error(Encoded::CommitMetadata))
}

