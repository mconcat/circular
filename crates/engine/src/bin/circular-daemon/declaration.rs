use crate::daemon::session::{BEGIN_EPOCH, COMMAND_RESULT, answer_with};
use crate::daemon::{authoring_store, ledger, run_control_store};
use circular_core::Value;
use circular_protocol::Partition;
use circular_protocol::declaration_payload;
use circular_protocol::declaration_payload::{Accepted, CommandResult, Rejected};
use circular_protocol::rejection_code::{Reasoned, RejectionReason};
use circular_protocol::{
    BeginEpochError, DeclarationVerb, EnvelopeHeader, OpenEpoch, ScopeCoverage, StableVerb,
    encode_begin_epoch_rejection, verb_tag,
};
use engine::authoring_assembly::rejection::FoldRejection;
use engine::authoring_assembly::verb::{ContentVerb, accepted_item};
use engine::authoring_assembly::{fold as assembly, ledger as authoring};
use engine::execution_profile::ProductExecutionProfile;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DaemonEpochScope(Vec<declaration_payload::ScopeSegment>);

impl ScopeCoverage for DaemonEpochScope {
    fn covers(&self, target: &Self) -> bool {
        target.0.starts_with(&self.0)
    }
}

pub(crate) enum SessionEpoch {
    Idle,
    Open(OpenEpoch<DaemonEpochScope, assembly::EpochCandidate>),
}

impl SessionEpoch {
    pub(crate) fn active_id(&self) -> Option<Vec<u8>> {
        match self {
            Self::Open(epoch) => epoch.candidate().issued_epoch.clone(),
            Self::Idle => None,
        }
    }

    pub(crate) fn active_scope(&self) -> Option<Vec<declaration_payload::ScopeSegment>> {
        match self {
            Self::Open(epoch) => Some(epoch.scope().0.clone()),
            Self::Idle => None,
        }
    }

    pub(crate) fn take_active(&mut self) -> Option<assembly::EpochCandidate> {
        match std::mem::replace(self, Self::Idle) {
            Self::Open(epoch) => Some(epoch.into_candidate()),
            Self::Idle => None,
        }
    }
}

pub(crate) fn answer_declaration(
    stream: &mut impl circular_transport::LocalByteStream<Error = std::io::Error>,
    header: EnvelopeHeader,
    result: CommandResult,
) {
    answer_with(stream, header, COMMAND_RESULT, result);
}

pub(crate) fn prepare_content_declaration(
    verb: DeclarationVerb,
    payload: &[u8],
    epochs: &mut SessionEpoch,
) -> CommandResult {
    let SessionEpoch::Open(epoch) = epochs else {
        return no_open_epoch();
    };
    epoch
        .update(|candidate| {
            let mut next = candidate.clone();
            let item = apply_content(verb, payload, &mut next)?;
            next.request_parts
                .push((verb_tag(StableVerb::Declaration(verb)), payload.to_vec()));
            next.accepted_commands.push(item);
            Ok((next, CommandResult::Accepted(Accepted::Nothing)))
        })
        .unwrap_or_else(|rejected| rejected)
}

fn apply_content(
    verb: DeclarationVerb,
    payload: &[u8],
    candidate: &mut assembly::EpochCandidate,
) -> Result<Value, CommandResult> {
    let canonicalized = |reason: String| {
        malformed(format!(
            "accepted declaration command could not be canonicalized: {reason}"
        ))
    };
    let item = accepted_item(verb, payload).map_err(|error| canonicalized(format!("{error:?}")))?;
    let content =
        ContentVerb::from_accepted(&item).map_err(|error| canonicalized(error.to_string()))?;
    candidate
        .apply(&content)
        .map(drop)
        .map_err(|rejection| rejected(ContentRejection::from(rejection)))?;
    Ok(item)
}

fn no_open_epoch() -> CommandResult {
    CommandResult::Rejected(
        RejectionReason::Malformed.reject(
            Partition::Declaration,
            "no epoch is open — content commands are valid only between BeginEpoch and CommitEpoch"
                .to_owned(),
        ),
    )
}

fn into_address<I>(address: circular_protocol::declaration_payload::AddressRef<I>) -> I {
    use circular_protocol::declaration_payload::AddressRef;
    match address {
        AddressRef::Absolute(identity)
        | AddressRef::EpochLocal(identity)
        | AddressRef::Relative(identity) => identity,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CommitBase {
    pub(crate) standing: bool,
    pub(crate) stream: Option<circular_store::StreamId>,
}

impl CommitBase {
    pub(crate) fn of(prefix: &crate::daemon::read_world::WorldRead) -> Self {
        Self {
            standing: prefix
                .server
                .as_ref()
                .is_some_and(|read| read.ingress().is_some()),
            stream: prefix.system.as_ref().map(|system| system.stream),
        }
    }
}

pub(crate) enum PreparedCommit {
    Answer(CommandResult),
    Install(Box<CommitInstallation>),
}

enum PreparedRuntime {
    Unavailable { reason: String },
    Activate {
        plan: Box<crate::authoring_assembly::projection::AuthoredProjection>,
        cut: Box<authoring::AuthoringSnapshot>,
        execution: ProductExecutionProfile,
    },
    Adopt {
        plan: Box<crate::authoring_assembly::projection::AuthoredProjection>,
        cut: Box<authoring::AuthoringSnapshot>,
    },
}

pub(crate) struct CommitInstallation {
    draft: authoring::AuthoringState,
    commit_id: Vec<u8>,
    cursor: u64,
    state_directory: Option<std::path::PathBuf>,
    runtime: PreparedRuntime,
}

#[allow(clippy::too_many_arguments)]
fn prepare_decoded_commit_epoch(
    epoch: &[u8],
    candidate: Option<assembly::EpochCandidate>,
    authority: &authoring::AuthoringState,
    base: CommitBase,
    execution: &ProductExecutionProfile,
    state_directory: Option<&std::path::Path>,
) -> PreparedCommit {
    let Some(candidate) = candidate else {
        return PreparedCommit::Answer(no_open_epoch());
    };
    if candidate.issued_epoch.as_deref() != Some(epoch) {
        return PreparedCommit::Answer(CommandResult::Rejected(RejectionReason::Malformed.reject(
            Partition::Declaration,
            "CommitEpoch references an identifier other than the currently open epoch".to_owned(),
        )));
    }
    let Some(commit_id) = candidate.commit_id.clone() else {
        return PreparedCommit::Answer(malformed("open epoch has no CommitId".to_owned()));
    };
    let request_digest = authoring::AuthoringState::request_digest(&candidate);
    match authority.dedup_lookup(&commit_id, &request_digest) {
        authoring::DedupLookup::Repeat(terminal) => {
            eprintln!(
                "circular-daemon: CommitEpoch — durable CommitId retry returned its first terminal"
            );
            return PreparedCommit::Answer(CommandResult::Accepted(Accepted::Transition(terminal)));
        }
        authoring::DedupLookup::Equivocation => {
            return PreparedCommit::Answer(CommandResult::Rejected(RejectionReason::Malformed.reject(Partition::Declaration, "CommitIdEquivocation: the same CommitId references a different authoring request"
                    .to_owned())));
        }
        authoring::DedupLookup::New => {}
    }
    let mut draft = authority.clone();
    let promotion = match draft.promote_commit(candidate, request_digest) {
        Ok(promotion) => promotion,
        Err(reason) => {
            eprintln!("circular-daemon: CommitEpoch — plan failed to stand: {reason}");
            return PreparedCommit::Answer(assembly_rejected(reason));
        }
    };
    if let Err(rejection) = admit_live_plan(authority, &promotion.plan, execution) {
        return PreparedCommit::Answer(assembly_rejected(rejection));
    }
    let cursor = promotion.cursor;
    let plan = promotion.plan;
    let settle = |draft: authoring::AuthoringState, runtime: PreparedRuntime| {
        PreparedCommit::Install(Box::new(CommitInstallation {
            draft,
            commit_id: commit_id.clone(),
            cursor,
            state_directory: state_directory.map(std::path::Path::to_path_buf),
            runtime,
        }))
    };
    let authoring_cut = match draft.snapshot(Vec::new()) {
        Ok(snapshot) => snapshot,
        Err(reason) => {
            return settle(
                draft,
                PreparedRuntime::Unavailable {
                    reason: format!("GraphRevision cut is unavailable: {reason}"),
                },
            );
        }
    };
    if base.standing {
        return settle(
            draft,
            PreparedRuntime::Adopt {
                plan: Box::new(plan),
                cut: Box::new(authoring_cut),
            },
        );
    }
    if let Some(stream) = base.stream {
        return settle(
            draft,
            PreparedRuntime::Unavailable {
                reason: format!(
                    "stream {} does not stand for revision adoption",
                    stream.get()
                ),
            },
        );
    }
    settle(
        draft,
        PreparedRuntime::Activate {
            plan: Box::new(plan),
            cut: Box::new(authoring_cut),
            execution: execution.clone(),
        },
    )
}

enum CommitActivation {
    Unavailable {
        reason: String,
    },
    Activate {
        plan: Box<crate::authoring_assembly::projection::AuthoredProjection>,
        cut: Box<authoring::AuthoringSnapshot>,
        execution: ProductExecutionProfile,
    },
    AdoptRevision {
        prepared: ledger::PreparedAdoption,
    },
}

/// First boot activation and committed activation keep System before user assembly.
pub(crate) fn stand_first_activation(
    authoring: &authoring::AuthoringState,
    execution: &ProductExecutionProfile,
    state_directory: &std::path::Path,
    system: &mut Option<std::sync::Arc<ledger::SystemRuntime>>,
    restart: Option<circular_store::ProductRestartBody>,
) -> Result<Option<ledger::ServerRun>, String> {
    if authoring.revision().is_none() {
        return Ok(None);
    }
    let cut = authoring
        .snapshot(Vec::new())
        .map_err(|error| error.to_string())?;
    let plan = authoring
        .current_plan()
        .map_err(|error| error.to_string())?;
    activate_first(
        plan,
        &cut,
        execution,
        Some(state_directory),
        system,
        restart,
    )
}

fn activate_first(
    plan: crate::authoring_assembly::projection::AuthoredProjection,
    cut: &authoring::AuthoringSnapshot,
    execution: &ProductExecutionProfile,
    directory: Option<&std::path::Path>,
    system: &mut Option<std::sync::Arc<ledger::SystemRuntime>>,
    restart: Option<circular_store::ProductRestartBody>,
) -> Result<Option<ledger::ServerRun>, String> {
    let stream = run_control_store::state_stream(directory)?;
    let revision =
        engine::RevisionEpochId::new(cut.cursor).ok_or("activation revision is absent")?;
    let mut pending = match directory {
        Some(directory) => ledger::ServerRun::prepare_at_authoring_cut(
            plan,
            revision,
            stream,
            execution.clone(),
            directory,
            cut,
        ),
        None => {
            ledger::ServerRun::prepare_authoring_cut(plan, revision, stream, execution.clone(), cut)
        }
    }?;
    let owner = pending.start_system()?;
    *system = Some(owner.clone());
    if let Some(body) = restart {
        owner
            .pipeline
            .record_facts(
                revision,
                vec![crate::kernel::system::DaemonFact::Restart {
                    origin: circular_core::Tick::ZERO,
                    body,
                }],
            )
            .map_err(|reason| format!("restart after record owner failure: {reason}"))?;
    }
    match pending.activate_on(&owner) {
        Ok(run) => Ok(Some(run)),
        Err(reason) => {
            eprintln!("circular-daemon: recorded activation failure: {reason}");
            Ok(None)
        }
    }
}

pub(crate) fn install_commit_epoch(
    installation: Box<CommitInstallation>,
    authoring_store: &authoring_store::AuthoringStore,
    server: &mut Option<ledger::ServerRun>,
    system: &mut Option<std::sync::Arc<ledger::SystemRuntime>>,
) -> Result<CommittedEpoch, Rejected> {
    let CommitInstallation {
        draft,
        commit_id,
        cursor,
        state_directory,
        runtime,
    } = *installation;
    let target = engine::RevisionEpochId::new(cursor).expect("authoring cursors start at one");
    let activation = match runtime {
        PreparedRuntime::Unavailable { reason } => CommitActivation::Unavailable { reason },
        PreparedRuntime::Activate {
            plan,
            cut,
            execution,
        } => CommitActivation::Activate {
            plan,
            cut,
            execution,
        },
        PreparedRuntime::Adopt { plan, cut } => {
            let standing = server
                .as_mut()
                .expect("a standing run was measured in the commit base");
            let prepared = standing.prepare_authoring_cut_adoption(target, *plan, &cut);
            CommitActivation::AdoptRevision { prepared }
        }
    };
    let terminal = draft
        .committed_terminal(&commit_id)
        .map_err(|rejection| rejection.rejected(Partition::Declaration))?;
    if let Err(message) = authoring_store.save(&draft) {
        return Err(RejectionReason::Malformed.reject(
            Partition::Declaration,
            format!("authoring commit was not published to durable state: {message}"),
        ));
    }
    let committed = CommittedEpoch {
        terminal,
        authoring: draft,
    };
    Ok(activate_committed_epoch(
        committed,
        activation,
        state_directory,
        server,
        system,
    ))
}

/// The durable terminal and its fold travel together until publication.
/// A refusal cannot carry a new fold, and an accepted append cannot lose it.
pub(crate) struct CommittedEpoch {
    terminal: Value,
    authoring: authoring::AuthoringState,
}

impl CommittedEpoch {
    fn publish(self, publish: impl FnOnce(authoring::AuthoringState)) -> CommandResult {
        publish(self.authoring);
        CommandResult::Accepted(Accepted::Transition(self.terminal))
    }
}

#[allow(clippy::too_many_arguments)]
fn activate_committed_epoch(
    committed: CommittedEpoch,
    activation: CommitActivation,
    state_directory: Option<std::path::PathBuf>,
    server: &mut Option<ledger::ServerRun>,
    system: &mut Option<std::sync::Arc<ledger::SystemRuntime>>,
) -> CommittedEpoch {
    match activation {
        CommitActivation::Unavailable { reason } => {
            let Some(owner) = system.as_ref() else {
                crate::daemon::platform::record_owner_failed(&reason);
            };
            let revision = engine::RevisionEpochId::new(committed.authoring.cursor())
                .expect("committed revision");
            if let Err(reason) = owner.pipeline.adopt(revision, move || Err(reason)) {
                eprintln!("circular-daemon: revision preparation failed: {reason}");
            }
        }
        CommitActivation::Activate {
            plan,
            cut,
            execution,
        } => match activate_first(
            *plan,
            &cut,
            &execution,
            state_directory.as_deref(),
            system,
            None,
        ) {
            Ok(standing) => *server = standing,
            Err(reason) => crate::daemon::platform::record_owner_failed(&reason),
        },
        CommitActivation::AdoptRevision { prepared } => {
            let standing = server
                .as_mut()
                .expect("a standing run was measured in the commit base");
            let outcome = standing.commit_prepared_adoption(prepared);
            match outcome {
                ledger::PlanAdoptionOutcome::Adopted => {}
                ledger::PlanAdoptionOutcome::Behind { reason } => {
                    eprintln!("circular-daemon: failed to adopt edit as revision: {reason}");
                }
            }
        }
    }
    committed
}

pub(crate) fn commit_decoded_epoch_outside_world(
    epoch: &[u8],
    candidate: Option<assembly::EpochCandidate>,
    world: &crate::daemon::read_world::SharedWorld,
    authoring_store: &authoring_store::AuthoringStore,
    execution: &ProductExecutionProfile,
) -> CommandResult {
    let Ok(_gate) = world.commit_gate() else {
        return malformed("daemon commit gate poisoned".to_owned());
    };
    let prefix = world.read();
    let base = CommitBase::of(&prefix);
    let state_directory = prefix.state_directory.clone();

    let prepared = prepare_decoded_commit_epoch(
        epoch,
        candidate,
        prefix.authoring.as_ref(),
        base,
        execution,
        Some(state_directory.as_path()),
    );
    let installation = match prepared {
        PreparedCommit::Answer(result) => return result,
        PreparedCommit::Install(installation) => installation,
    };

    let Ok(mut run_owner) = world.run_write() else {
        return malformed("run projection owner poisoned".to_owned());
    };
    let (server, system) = run_owner.parts();
    let result = match install_commit_epoch(installation, authoring_store, server, system) {
        Err(rejected) => CommandResult::Rejected(rejected),
        Ok(committed) => committed.publish(|fold| run_owner.set_authoring(Some(fold))),
    };
    run_owner.finish();
    result
}

fn assembly_rejected(error: FoldRejection) -> CommandResult {
    let rejected = error.rejected(Partition::Declaration);
    let at = match error.config_admission() {
        Some(admission) => crate::daemon::actor_catalog::config_admission_at(admission),
        None => rejected.at,
    };
    CommandResult::Rejected(Rejected { at, ..rejected })
}

pub(crate) fn malformed(detail: String) -> CommandResult {
    CommandResult::Rejected(RejectionReason::Malformed.reject(Partition::Declaration, detail))
}

fn epoch_barrier_rejected(error: BeginEpochError, message: impl Into<String>) -> CommandResult {
    CommandResult::Rejected(
        RejectionReason::Malformed
            .reject(Partition::Declaration, message)
            .at(encode_begin_epoch_rejection(error)),
    )
}

pub(crate) static NEXT_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn issue_epoch() -> Vec<u8> {
    NEXT_EPOCH
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        .to_be_bytes()
        .to_vec()
}

pub(crate) fn begin_epoch(
    begin: declaration_payload::BeginEpoch,
    payload: &[u8],
    epochs: &mut SessionEpoch,
    authoring: &authoring::AuthoringState,
) -> CommandResult {
    let SessionEpoch::Idle = epochs else {
        return epoch_barrier_rejected(
            BeginEpochError::DuplicateEpoch,
            "this session already has an open epoch",
        );
    };
    let issued = issue_epoch();
    let target = DaemonEpochScope(into_address(begin.scope.clone()));
    match authoring.begin_candidate(&begin, payload, verb_tag(BEGIN_EPOCH)) {
        Ok(mut opened) => {
            opened.issued_epoch = Some(issued.clone());
            *epochs = SessionEpoch::Open(OpenEpoch::new(target, opened));
        }
        Err(reason) => return CommandResult::Rejected(reason.rejected(Partition::Declaration)),
    }
    CommandResult::Accepted(Accepted::Epoch(issued))
}

pub(crate) struct ContentRejection {
    reason: RejectionReason,
    message: String,
    at: Option<Value>,
}

impl From<FoldRejection> for ContentRejection {
    fn from(rejection: FoldRejection) -> Self {
        Self {
            reason: rejection.reason(),
            message: rejection.to_string(),
            at: rejection.at(),
        }
    }
}

fn rejected(rejection: ContentRejection) -> CommandResult {
    CommandResult::Rejected(Rejected {
        at: rejection.at,
        ..rejection
            .reason
            .reject(Partition::Declaration, rejection.message)
    })
}

fn admit_live_plan(
    authority: &authoring::AuthoringState,
    plan: &crate::authoring_assembly::projection::AuthoredProjection,
    execution: &ProductExecutionProfile,
) -> Result<(), FoldRejection> {
    execution.admit_standing_actors(plan, || authority.current_plan())
}

pub(crate) fn validate_epoch(
    epoch: &[u8],
    epochs: &SessionEpoch,
    authority: &authoring::AuthoringState,
    execution: &ProductExecutionProfile,
) -> CommandResult {
    let SessionEpoch::Open(open) = epochs else {
        return no_open_epoch();
    };
    if let Err(result) = epoch_matches(epoch, open.candidate()) {
        return result;
    }
    match open.validate(|candidate| {
        authority
            .validate(candidate.clone())
            .and_then(|plan| admit_live_plan(authority, &plan, execution))
    }) {
        Ok(()) => CommandResult::Accepted(Accepted::Nothing),
        Err(error) => assembly_rejected(error),
    }
}

pub(crate) fn abort_epoch(epoch: &[u8], epochs: &mut SessionEpoch) -> CommandResult {
    let SessionEpoch::Open(open) = epochs else {
        return no_open_epoch();
    };
    if let Err(result) = epoch_matches(epoch, open.candidate()) {
        return result;
    }
    let _ = epochs.take_active();
    CommandResult::Accepted(Accepted::Nothing)
}

fn epoch_matches(epoch: &[u8], candidate: &assembly::EpochCandidate) -> Result<(), CommandResult> {
    if candidate.issued_epoch.as_deref() != Some(epoch) {
        return Err(CommandResult::Rejected(RejectionReason::Malformed.reject(
            Partition::Declaration,
            "epoch reference does not match the currently open epoch".to_owned(),
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{accepted_item, apply_content};
    use circular_core::{Boundary, Ceilings, Value, decode, encode};
    use circular_protocol::DeclarationVerb;
    use circular_protocol::authoring_snapshot::AuthoringSnapshotEncoder;
    use circular_protocol::declaration_payload::{
        ActorDeclaration, ActorFlags, ActorLocal, AddressContext, AddressRef,
        AnnotationDeclaration, AnnotationKind, AuthoredLocal, CommandResult, MoveToScope,
        PayloadRejection, PlanActorKey, PlanAnnotationKey, ScopeSegment, decode_upsert_actor,
    };
    use engine::authoring_assembly::fold::EpochCandidate;
    use engine::authoring_assembly::verb::ContentVerb;

    fn actor(scope: Vec<ScopeSegment>, local: &str) -> PlanActorKey {
        PlanActorKey {
            scope,
            local: AuthoredLocal::try_new(local)
                .expect("test actor local is authored")
                .into(),
        }
    }

    #[test]
    fn annotation_mutations_round_trip_through_daemon_accepted_history_canonicalization() {
        let actor = actor(vec![ScopeSegment::Child("stage".to_owned())], "source");
        let annotation = PlanAnnotationKey {
            scope: actor.scope.clone(),
            local: "note".to_owned(),
        };
        let declaration = AnnotationDeclaration {
            kind: AnnotationKind::Note,
            refs: vec![actor.clone()],
            body: "check this".to_owned(),
        };
        let Value::Object(snapshot) = AuthoringSnapshotEncoder::new(&[])
            .upsert_annotation(&annotation, &declaration)
            .expect("snapshot command encodes")
        else {
            panic!("snapshot command object")
        };
        let mut mutation = snapshot.into_map();
        mutation.remove("kind");
        let Value::Array(address) = mutation.get_mut("annotation").expect("address") else {
            panic!("annotation address array")
        };
        address[0] = Value::Int(2);
        let upsert_body = encode(
            &Value::object(mutation).expect("upsert body"),
            Ceilings::for_boundary(Boundary::Wire),
        )
        .expect("upsert body encodes");

        let accepted =
            accepted_item(DeclarationVerb::UpsertAnnotation, &upsert_body).expect("canonicalizes");
        let fields = accepted.as_object().expect("accepted command object");
        assert_eq!(fields.get("kind"), Some(&Value::string("UpsertAnnotation")));
        assert_eq!(
            fields
                .get("annotation")
                .and_then(Value::as_array)
                .map(|arm| &arm[0]),
            Some(&Value::Int(1)),
            "the epoch-local arm becomes the absolute arm"
        );
        assert_eq!(
            ContentVerb::from_accepted(&accepted).expect("accepted history reopens"),
            ContentVerb::UpsertAnnotation {
                annotation: annotation.clone(),
                declaration,
            }
        );

        let Value::Object(upsert_value) =
            decode(&upsert_body, Ceilings::for_boundary(Boundary::Wire))
                .expect("mutation upsert decodes")
        else {
            panic!("mutation upsert object")
        };
        let retire_body = Value::object([(
            "annotation",
            upsert_value.get("annotation").expect("identity").clone(),
        )])
        .expect("retire body");
        let retire_body =
            encode(&retire_body, Ceilings::for_boundary(Boundary::Wire)).expect("retire encodes");
        let accepted =
            accepted_item(DeclarationVerb::RetireAnnotation, &retire_body).expect("canonicalizes");
        assert_eq!(
            accepted.as_object().and_then(|fields| fields.get("kind")),
            Some(&Value::string("RetireAnnotation"))
        );
        assert_eq!(
            ContentVerb::from_accepted(&accepted).expect("accepted history reopens"),
            ContentVerb::RetireAnnotation { annotation }
        );
    }

    #[test]
    fn move_to_scope_accepted_history_makes_every_address_absolute() {
        let command = MoveToScope {
            actors: vec![AddressRef::EpochLocal(actor(Vec::new(), "moving"))],
            target: AddressRef::EpochLocal(vec![ScopeSegment::Child("group".to_owned())]),
        };
        let body = command
            .encode(Ceilings::for_boundary(Boundary::Wire))
            .expect("encodes");
        let accepted = accepted_item(DeclarationVerb::MoveToScope, &body).expect("canonicalizes");
        let fields = accepted.as_object().expect("accepted command object");
        assert_eq!(fields.get("kind"), Some(&Value::string("MoveToScope")));
        let arm = |address: &Value| address.as_array().map(|parts| parts[0].clone());
        assert_eq!(fields.get("target").and_then(arm), Some(Value::Int(1)));
        assert_eq!(
            fields
                .get("actors")
                .and_then(Value::as_array)
                .map(|actors| actors.iter().filter_map(arm).collect::<Vec<_>>()),
            Some(vec![Value::Int(1)])
        );
        assert_eq!(
            ContentVerb::from_accepted(&accepted).expect("absolute accepted command decodes"),
            ContentVerb::MoveToScope {
                actors: vec![actor(Vec::new(), "moving")],
                target: vec![ScopeSegment::Child("group".to_owned())],
            }
        );
    }
}

#[cfg(test)]
mod template_tests {
    use super::*;

    #[test]
    fn template_content_uses_the_existing_epoch_receipt_and_retains_the_command_value() {
        use circular_core::{Boundary, Ceilings};
        let body = Value::object([
            ("name", Value::string("worker")),
            ("commands", Value::array([])),
        ])
        .unwrap();
        let bytes = circular_core::encode(&body, Ceilings::for_boundary(Boundary::Wire)).unwrap();
        let mut candidate = assembly::EpochCandidate::open(vec![]).unwrap();
        let item = apply_content(DeclarationVerb::UpsertTemplate, &bytes, &mut candidate)
            .expect("the root epoch accepts a template");
        assert_eq!(candidate.assemble().unwrap().templates().len(), 1);
        assert_eq!(
            item,
            Value::object([
                ("name", Value::string("worker")),
                ("commands", Value::array([])),
                ("kind", Value::string("UpsertTemplate"))
            ])
            .unwrap()
        );
        assert!(matches!(
            prepare_content_declaration(
                DeclarationVerb::UpsertTemplate,
                &bytes,
                &mut SessionEpoch::Idle
            ),
            CommandResult::Rejected(_)
        ));
        let mut child =
            assembly::EpochCandidate::open(vec![declaration_payload::ScopeSegment::Child(
                "child".into(),
            )])
            .unwrap();
        assert!(matches!(
            apply_content(DeclarationVerb::UpsertTemplate, &bytes, &mut child),
            Err(CommandResult::Rejected(_))
        ));
        let bytes = circular_core::encode(
            &Value::object([("name", Value::string("worker"))]).unwrap(),
            Ceilings::for_boundary(Boundary::Wire),
        )
        .unwrap();
        let item = apply_content(DeclarationVerb::RetireTemplate, &bytes, &mut candidate)
            .expect("the root epoch retires the template");
        assert!(candidate.assemble().unwrap().templates().is_empty());
        assert_eq!(
            item,
            Value::object([
                ("kind", Value::string("RetireTemplate")),
                ("name", Value::string("worker"))
            ])
            .unwrap()
        );
    }
}
