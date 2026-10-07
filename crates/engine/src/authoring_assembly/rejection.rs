
use std::collections::BTreeSet;
use std::fmt;

use circular_core::{CodecError, DuplicateKeyError, Value};
use circular_plan::{BoardPlacement, ConfigValueError, Name, NonContainerActorDeclError, ScopeId};
use circular_protocol::authoring_snapshot::CompactedDeclarationRejection;
use circular_protocol::boundary_port::{
    BoundaryActivationRejection, BoundaryGenerationError, BoundaryPortIdError,
    encode_boundary_activation_rejection,
};
use circular_protocol::declaration_payload::{
    ActorLocal, ExpectedRevision, ExportRoles, PayloadRejection, PlanActorKey,
    ReservedLocalSpelling, ScopeRole, ScopeSegment,
};
use circular_protocol::move_to_scope::{MoveToScopeRejection, encode_move_to_scope_rejection};
use circular_protocol::rejection_code::{Reasoned, RejectionReason};

use crate::actor_registry::PlanRegistryError;
use crate::authoring_assembly::projection::BuildError;

#[derive(Clone, Debug, PartialEq)]
pub enum FoldRejection {
    TargetIsInstance,
    TemplateOutsideRoot,
    InvalidTemplate(PayloadRejection),
    SetFlagsMissingActor { local: ActorLocal },
    EnvironmentBarrierNotFirst,
    EnvironmentBarrierOutsideRoot,
    GenerationExhausted(BoundaryGenerationError),
    MoveToScope(MoveToScopeFault),

    UnknownActorType { actor_type: String },
    DeferredActorType { actor_type: String },
    ContainerWithoutScopeDeclaration { local: ActorLocal },
    BoundaryWithoutLabel { local: ActorLocal },
    ScopeDeclaresTarget,
    AddressOutsideTarget { what: AddressUse },
    AddressAtInstance { what: AddressUse },
    TargetBoundary {
        target: Vec<ScopeSegment>,
        outside: BTreeSet<(TargetWrite, Vec<ScopeSegment>)>,
    },
    ScopeWithoutContainer { scope: Name },
    ContainerWithoutScope { container: Name },
    ScopeRoleMismatch {
        scope: Name,
        role: ScopeRole,
        container_type: String,
    },
    InvalidActorDeclaration(NonContainerActorDeclError),
    Build { step: BuildStep, error: BuildError },
    ScopeDepthLimit,
    AnnotationReferenceDepthLimit,
    PortNotCanonical { at: PortAt, port: String },
    BoardOverlap {
        left: String,
        left_board: BoardPlacement,
        right: String,
        right_board: BoardPlacement,
    },
    ExportOperationsWithoutSurface {
        export: String,
        scope: ScopeId,
        operations: Value,
        roles: ExportRoles,
    },
    ExportOperations {
        export: String,
        error: ConfigValueError,
    },
    ExportSurface(ConfigValueError),
    MountReferencesMissingActor { mount: String, actor: String },
    Config {
        actor: String,
        at: ConfigAt,
        value: Option<Value>,
        fault: ConfigFault,
    },
    Boundary(BoundaryFault),
    Registry(Box<PlanRegistryError>),
    Template(TemplateFault),

    RevisionConflict {
        expected: ExpectedRevision,
        current: ExpectedRevision,
    },
    BracketIncomplete(BracketField),
    EnvironmentChanged,
    CursorExhausted,
    NoCommitToAnswer,
    TerminalNotNewest,
    SnapshotScopeMissing,
    NoCommittedEpoch,
    InstanceScopeNotReconstructible,
    AnnotationPlacementUncarried,
    NoRevisionTransition,
    RevisionItemsExceedCounter,
    CommitCursorExceedsInt,
    Encode {
        what: Encoded,
        error: DuplicateKeyError,
    },
    Codec { what: Coded, error: CodecError },
    ConfigToWire(ConfigValueError),
    BoundaryActorKey(BoundaryPortIdError),
    SnapshotEncode(String),
    Persisted(PersistedFault),
}

impl FoldRejection {
    #[must_use]
    pub fn config_admission(&self) -> Option<&circular_actors::CreateInputAdmissionError> {
        match self {
            Self::Registry(error) => match error.as_ref() {
                PlanRegistryError::ConfigFold { admission, .. } => admission.as_deref(),
                _ => None,
            },
            _ => None,
        }
    }
}

impl Reasoned for FoldRejection {
    fn reason(&self) -> RejectionReason {
        match self {
            Self::RevisionConflict { .. } => RejectionReason::RevisionConflict,
            _ => RejectionReason::Malformed,
        }
    }

    fn at(&self) -> Option<Value> {
        match self {
            Self::Boundary(fault) => Some(encode_boundary_activation_rejection(fault.rejection())),
            Self::MoveToScope(fault) => Some(encode_move_to_scope_rejection(fault.rejection())),
            _ => None,
        }
    }
}

impl From<MoveToScopeFault> for FoldRejection {
    fn from(fault: MoveToScopeFault) -> Self {
        Self::MoveToScope(fault)
    }
}

impl From<BoundaryFault> for FoldRejection {
    fn from(fault: BoundaryFault) -> Self {
        Self::Boundary(fault)
    }
}

impl From<TemplateFault> for FoldRejection {
    fn from(fault: TemplateFault) -> Self {
        Self::Template(fault)
    }
}

impl From<PersistedFault> for FoldRejection {
    fn from(fault: PersistedFault) -> Self {
        Self::Persisted(fault)
    }
}

impl std::error::Error for FoldRejection {}

impl fmt::Display for FoldRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TargetIsInstance => f.write_str(
                "epoch target is an instance scope; instances are minted by the run, not authored",
            ),
            Self::TemplateOutsideRoot => {
                f.write_str("templates can only be changed in a root epoch")
            }
            Self::InvalidTemplate(error) => write!(f, "invalid template: {error}"),
            Self::SetFlagsMissingActor { local } => {
                write!(f, "SetFlags targets missing actor `{local}`")
            }
            Self::EnvironmentBarrierNotFirst => f.write_str(
                "ReplaceAuthoringEnvironment must be the first content command in an otherwise-empty root epoch",
            ),
            Self::EnvironmentBarrierOutsideRoot => f.write_str(
                "ReplaceAuthoringEnvironment barrier is available only in a project root epoch",
            ),
            Self::GenerationExhausted(error) => write!(f, "{error}"),
            Self::MoveToScope(fault) => fault.fmt(f),
            Self::UnknownActorType { actor_type } => write!(f, "unknown actor type: {actor_type}"),
            Self::DeferredActorType { actor_type } => write!(
                f,
                "actor type `{actor_type}` is deferred and cannot be authored in this release"
            ),
            Self::ContainerWithoutScopeDeclaration { local } => write!(
                f,
                "container `{local}` has no paired child scope declaration"
            ),
            Self::BoundaryWithoutLabel { local } => write!(f, "boundary `{local}` has no label"),
            Self::ScopeDeclaresTarget => f.write_str(
                "UpsertScope declaring the target scope itself does not belong to this epoch",
            ),
            Self::AddressOutsideTarget { what } => {
                write!(f, "{what} address is outside the epoch target scope")
            }
            Self::AddressAtInstance { what } => write!(
                f,
                "{what} is authored at an instance address; instances are created by the run, not declarations"
            ),
            Self::TargetBoundary { target, outside } => {
                write!(f, "epoch target {} does not own ", spelled_scope(target))?;
                for (index, (what, scope)) in outside.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{what} in {}", spelled_scope(scope))?;
                }
                f.write_str("; a scope's own declaration belongs to its parent scope's epoch")
            }
            Self::ScopeWithoutContainer { scope } => write!(
                f,
                "scope `{scope}` has no container actor; assembly requires paired scope and container declarations"
            ),
            Self::ContainerWithoutScope { container } => write!(
                f,
                "container actor `{container}` has no child scope; containers require a paired scope"
            ),
            Self::ScopeRoleMismatch {
                scope,
                role,
                container_type,
            } => write!(
                f,
                "scope `{scope}` is declared {} but its container is `{container_type}`; a Template scope pairs only with a replicator",
                match role {
                    ScopeRole::Concrete => "concrete",
                    ScopeRole::Template => "a Template",
                }
            ),
            Self::InvalidActorDeclaration(error) => write!(f, "invalid actor declaration: {error}"),
            Self::Build { step, error } => write!(f, "{step}: {error}"),
            Self::ScopeDepthLimit => f.write_str("scope depth limit"),
            Self::AnnotationReferenceDepthLimit => {
                f.write_str("annotation reference scope depth limit")
            }
            Self::PortNotCanonical { at, port } => match at {
                PortAt::BoundaryInner => write!(
                    f,
                    "boundary binding inner port name is not canonical: {port}"
                ),
                PortAt::BoundaryOuter { scope } => write!(
                    f,
                    "scope `{scope}` boundary outer name is not canonical: {port}"
                ),
                PortAt::ExportRole { export } => {
                    write!(f, "export `{export}` port name is not canonical: {port}")
                }
                PortAt::Edge => write!(f, "port name is not canonical: {port}"),
            },
            Self::BoardOverlap {
                left,
                left_board,
                right,
                right_board,
            } => write!(
                f,
                "board placements overlap in the same scope: actor `{left}` at {left_board} \
                 and actor `{right}` at {right_board}"
            ),
            Self::ExportOperationsWithoutSurface {
                export,
                scope,
                operations,
                roles,
            } => write!(
                f,
                "ConfigRejected: export `{export}` in {}; \
                 declaration.operations = {}; declaration.surface = <missing>; \
                 roles = {}; operations requires a surface",
                scope.spelled(),
                circular_core::spelling::ValueText(operations),
                RolesText(roles)
            ),
            Self::ExportOperations { export, error } => write!(f, "export `{export}`: {error}"),
            Self::ExportSurface(error) => write!(f, "{error}"),
            Self::MountReferencesMissingActor { mount, actor } => write!(
                f,
                "mount `{mount}` references missing actor `{actor}`; unmount it before deleting the actor"
            ),
            Self::Config {
                actor,
                at,
                value,
                fault,
            } => f.write_str(&circular_actors::config::config_rejection(
                actor,
                &at.to_string(),
                value.as_ref(),
                fault,
            )),
            Self::Boundary(fault) => fault.fmt(f),
            Self::Registry(error) => write!(f, "{error}"),
            Self::Template(fault) => fault.fmt(f),
            Self::RevisionConflict { .. } => {
                f.write_str(RejectionReason::RevisionConflict.message())
            }
            Self::BracketIncomplete(field) => write!(f, "open epoch has no {field}"),
            Self::EnvironmentChanged => {
                f.write_str("authoring environment changed after the epoch opened")
            }
            Self::CursorExhausted => f.write_str("authoring cursor exhausted"),
            Self::NoCommitToAnswer => f.write_str("no commit to answer"),
            Self::TerminalNotNewest => {
                f.write_str("terminal lookup does not name the newest CommitId")
            }
            Self::SnapshotScopeMissing => {
                f.write_str("authoring-snapshot targets a scope missing from the current state")
            }
            Self::NoCommittedEpoch => {
                f.write_str("authoring state has no committed epoch to append")
            }
            Self::InstanceScopeNotReconstructible => {
                f.write_str("minted instance scopes cannot be reconstructed from authoring")
            }
            Self::AnnotationPlacementUncarried => {
                f.write_str("plan annotation placement has no declaration wire carrier")
            }
            Self::NoRevisionTransition => {
                f.write_str("accepted commit produced no authored revision transition")
            }
            Self::RevisionItemsExceedCounter => f.write_str("too many declaration items to revise"),
            Self::CommitCursorExceedsInt => f.write_str("commit cursor exceeds the Int carrier"),
            Self::Encode { what, error } => write!(f, "{what}: {error}"),
            Self::Codec { what, error } => write!(f, "{what} does not encode: {error}"),
            Self::ConfigToWire(error) => write!(f, "{error}"),
            Self::BoundaryActorKey(error) => write!(f, "{error}"),
            Self::SnapshotEncode(detail) => f.write_str(detail),
            Self::Persisted(fault) => fault.fmt(f),
        }
    }
}

circular_core::closed_table! {
    pub enum AddressUse {
        Actor => "actor",
        Scope => "scope",
        Edge => "edge",
        Presentation => "presentation",
        ExportMount => "export mount",
        Annotation => "annotation",
        BoundaryBinding => "boundary binding",
        ExportRole => "export role",
        AnnotationReference => "annotation reference",
    }
}

circular_core::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum TargetWrite {
        AuthoredKey => "authored key",
        EdgeEndpoint => "edge endpoint",
        ScopeDeclaration => "scope declaration",
        ScopeRetirement => "scope retirement",
    }
}

circular_core::closed_table! {
    pub enum BuildStep {
        PlaceActor => "cannot place actor",
        OpenScope => "cannot open scope",
        PlaceBoundary => "cannot place boundary",
        CloseScope => "cannot close scope",
        PlaceEdge => "cannot place edge",
        Finish => "failed to build plan",
    }
}

circular_core::closed_table! {
    pub enum BracketField {
        CommitId => "CommitId",
        IssuedEpoch => "issued EpochId",
        EnvironmentBaseline => "authoring environment baseline",
        RevisionBaseline => "authoring revision baseline",
    }
}

circular_core::closed_table! {
    pub enum Encoded {
        ProjectCreation => "project creation object",
        ProjectCreationEntry => "project creation entry object",
        JournalEpoch => "authoring journal epoch object",
        ActorGenerationFact => "authored actor generation fact",
        BeginEpoch => "accepted BeginEpoch object",
        CommitEpoch => "accepted CommitEpoch object",
        RpcEpoch => "accepted RPC epoch object",
        CommitFrame => "authoring commit frame object",
        Commit => "persisted authoring commit object",
        RevisionTransition => "revision transition object",
        CommitMetadata => "accepted commit metadata object",
    }
}

circular_core::closed_table! {
    pub enum Coded {
        SnapshotItem => "snapshot item",
        RevisionInput => "revision input",
        RevisionScopeIdentity => "revision scope identity",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PortAt {
    BoundaryInner,
    BoundaryOuter { scope: Name },
    ExportRole { export: String },
    Edge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigAt {
    Config,
    Preprocess(usize),
}

impl fmt::Display for ConfigAt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config => f.write_str("config"),
            Self::Preprocess(index) => write!(f, "preprocess[{index}].config"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConfigFault {
    Value(ConfigValueError),
    Ports(Box<PlanRegistryError>),
    PreprocessFold(String),
    Map(circular_actors::map_config::MapConfigError),
    Filter(circular_actors::FilterConfigError),
    Parse(circular_actors::parse_config::ParseConfigError),
    Flatten(String),
    Bang(circular_actors::EmptyConfigFactoryError),
}

impl fmt::Display for ConfigFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(error) => write!(f, "{error}"),
            Self::Ports(error) => write!(f, "{error}"),
            Self::PreprocessFold(detail) | Self::Flatten(detail) => f.write_str(detail),
            Self::Map(error) => write!(f, "{error}"),
            Self::Filter(error) => write!(f, "{error}"),
            Self::Parse(error) => write!(f, "{error}"),
            Self::Bang(error) => write!(f, "{error}"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BoundaryFault {
    ReplicatorInletCount {
        scope: Vec<ScopeSegment>,
        inlets: usize,
    },
    AuthoredDisagrees { scope: Vec<ScopeSegment> },
    PortIds {
        scope: Vec<ScopeSegment>,
        error: BoundaryPortIdError,
    },
    LeafUnresolved,
    StaleGeneration,
}

impl BoundaryFault {
    #[must_use]
    pub const fn rejection(&self) -> BoundaryActivationRejection {
        match self {
            Self::ReplicatorInletCount { .. }
            | Self::AuthoredDisagrees { .. }
            | Self::PortIds { .. } => BoundaryActivationRejection::BoundaryDisagreesWithDerivation,
            Self::LeafUnresolved => BoundaryActivationRejection::BoundaryLeafUnresolved,
            Self::StaleGeneration => BoundaryActivationRejection::StaleBoundaryGeneration,
        }
    }
}

impl fmt::Display for BoundaryFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReplicatorInletCount { scope, inlets } => write!(
                f,
                "replicator template {} derives {inlets} boundary inlets from its input actors; a replicator template requires one boundary inlet",
                spelled_scope(scope)
            ),
            Self::AuthoredDisagrees { scope } => write!(
                f,
                "authored boundary of {} differs from the boundary derived from its input and output actors",
                spelled_scope(scope)
            ),
            Self::PortIds { scope, error } => write!(
                f,
                "boundary port ids of {} cannot be derived from its input and output actors: {error}",
                spelled_scope(scope)
            ),
            Self::LeafUnresolved | Self::StaleGeneration => {
                f.write_str("boundary activation rejected")
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MoveToScopeFault {
    EmptyMoveSet,
    SourceMissing {
        source: PlanActorKey,
    },
    TargetUndeclared {
        target: Vec<ScopeSegment>,
    },
    TargetIsTemplate {
        target: Vec<ScopeSegment>,
    },
    BoundaryActor {
        local: ActorLocal,
        actor_type: String,
    },
    ContainerIntoItself {
        local: ActorLocal,
    },
    ContainerMoveUnsupported {
        local: ActorLocal,
    },
    SourcesSpanScopes,
    LocalCollision {
        target: Vec<ScopeSegment>,
        local: ActorLocal,
    },
    MoreThanOneBoundary {
        source: Vec<ScopeSegment>,
        target: Vec<ScopeSegment>,
    },
    ContainerInactive {
        local: ActorLocal,
    },
    InnerPortNotCanonical {
        port: String,
    },
    BoundaryLocal(ReservedLocalSpelling),
    SynthesizedLocalConflicts {
        local: ActorLocal,
    },
    SynthesizedPort(BoundaryPortIdError),
    SynthesizedBoundaryMissing {
        port: String,
    },
    NoInnerRelay {
        port: String,
    },
    ReverseBoundaryLocal(ReservedLocalSpelling),
    ReversePort(BoundaryPortIdError),
}

impl MoveToScopeFault {
    #[must_use]
    pub const fn rejection(&self) -> MoveToScopeRejection {
        match self {
            Self::EmptyMoveSet => MoveToScopeRejection::EmptyMoveSet,
            Self::SourceMissing { .. } => MoveToScopeRejection::SourceUnresolved,
            Self::TargetUndeclared { .. } => MoveToScopeRejection::TargetScopeUnresolved,
            Self::TargetIsTemplate { .. } => MoveToScopeRejection::TargetScopeIsTemplate,
            Self::BoundaryActor { .. } => MoveToScopeRejection::BoundaryActorImmovable,
            Self::ContainerIntoItself { .. } | Self::ContainerMoveUnsupported { .. } => {
                MoveToScopeRejection::WouldNestIntoSelf
            }
            Self::LocalCollision { .. } => MoveToScopeRejection::LocalCollisionInTarget,
            Self::SourcesSpanScopes
            | Self::MoreThanOneBoundary { .. }
            | Self::ContainerInactive { .. }
            | Self::InnerPortNotCanonical { .. }
            | Self::BoundaryLocal(_)
            | Self::SynthesizedLocalConflicts { .. }
            | Self::SynthesizedPort(_)
            | Self::SynthesizedBoundaryMissing { .. }
            | Self::NoInnerRelay { .. }
            | Self::ReverseBoundaryLocal(_)
            | Self::ReversePort(_) => MoveToScopeRejection::BoundarySynthesisRefused,
        }
    }
}

impl fmt::Display for MoveToScopeFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyMoveSet => f.write_str("MoveToScope.actors is empty"),
            Self::SourceMissing { source } => write!(
                f,
                "MoveToScope source `{source}` is missing from the candidate's current cut"
            ),
            Self::TargetUndeclared { target } => write!(
                f,
                "MoveToScope target {} is not declared in the candidate",
                spelled_scope(target)
            ),
            Self::TargetIsTemplate { target } => {
                write!(
                    f,
                    "MoveToScope target {} is a Template",
                    spelled_scope(target)
                )
            }
            Self::BoundaryActor { local, actor_type } => write!(
                f,
                "MoveToScope source `{local}` is a {actor_type} boundary actor and cannot be moved directly"
            ),
            Self::ContainerIntoItself { local } => write!(
                f,
                "MoveToScope source `{local}`: cannot move into the container itself or its descendants"
            ),
            Self::ContainerMoveUnsupported { local } => write!(
                f,
                "MoveToScope source `{local}`: container moves are unsupported in the first stage because nested move semantics are unresolved"
            ),
            Self::SourcesSpanScopes => {
                f.write_str("MoveToScope actors span different source scopes")
            }
            Self::LocalCollision { target, local } => write!(
                f,
                "MoveToScope target {} already contains local `{local}`",
                spelled_scope(target)
            ),
            Self::MoreThanOneBoundary { source, target } => write!(
                f,
                "MoveToScope can synthesize only one boundary at a time: source {}, target {}",
                spelled_scope(source),
                spelled_scope(target)
            ),
            Self::ContainerInactive { local } => write!(
                f,
                "MoveToScope boundary container `{local}` is not an active container"
            ),
            Self::InnerPortNotCanonical { port } => {
                write!(
                    f,
                    "MoveToScope inner port `{port}` is not a canonical PortId"
                )
            }
            Self::BoundaryLocal(error) => {
                write!(
                    f,
                    "failed to derive the MoveToScope boundary local: {error}"
                )
            }
            Self::SynthesizedLocalConflicts { local } => write!(
                f,
                "MoveToScope synthesized local `{local}` has a conflicting declaration"
            ),
            Self::SynthesizedPort(error) => write!(
                f,
                "failed to derive MoveToScope synthesized boundary port: {error}"
            ),
            Self::SynthesizedBoundaryMissing { port } => write!(
                f,
                "cannot find synthesized boundary actor for MoveToScope target container port `{port}`"
            ),
            Self::NoInnerRelay { port } => write!(
                f,
                "MoveToScope target boundary port `{port}` has no inner relay"
            ),
            Self::ReverseBoundaryLocal(error) => write!(
                f,
                "failed to derive the MoveToScope reverse boundary local: {error}"
            ),
            Self::ReversePort(error) => write!(
                f,
                "failed to derive MoveToScope reverse-synthesized port: {error}"
            ),
        }
    }
}

circular_core::closed_table! {
    pub enum TemplateSide {
        In => "in",
        Out => "out",
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TemplateFault {
    UnknownContainerField {
        field: String,
    },
    ReplicatorConfig(circular_actors::replicator_actor::ReplicatorFactoryError),
    NotAName,
    Unresolved {
        name: String,
    },
    ScopeDepthLimit,
    ChildrenAlsoDeclared,
    ParentAddressesChildren,
    Command(CompactedDeclarationRejection),
    BoundaryPort(BoundaryPortIdError),
    MintedBoundaryPort(BoundaryPortIdError),
    SideNotArray {
        side: TemplateSide,
    },
    TopicNotText,
    TopicDuplicateOrEmpty,
    BoundaryWithoutTopic,
    BoundaryTopicDuplicate,
    TopicsDisagree {
        side: TemplateSide,
    },
    RegistrationOutsideRoot,
    NotAdmitted,
}

impl fmt::Display for TemplateFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownContainerField { field } => {
                write!(f, "unknown template container config field `{field}`")
            }
            Self::ReplicatorConfig(error) => write!(f, "invalid replicator config: {error}"),
            Self::NotAName => f.write_str("template must be a name"),
            Self::Unresolved { name } => write!(f, "unresolved template `{name}`"),
            Self::ScopeDepthLimit => f.write_str("template scope depth limit"),
            Self::ChildrenAlsoDeclared => {
                f.write_str("runtime template children cannot also be declared by their parent")
            }
            Self::ParentAddressesChildren => {
                f.write_str("parent declarations cannot address runtime template children")
            }
            Self::Command(CompactedDeclarationRejection::Payload(error)) => {
                write!(f, "invalid template command: {error}")
            }
            Self::Command(CompactedDeclarationRejection::UnknownKind(kind)) => write!(
                f,
                "invalid template command: unknown command kind {}",
                circular_core::spelling::Quoted(kind)
            ),
            Self::BoundaryPort(error) => write!(f, "invalid template boundary: {error}"),
            Self::MintedBoundaryPort(error) => write!(f, "invalid minted boundary: {error}"),
            Self::SideNotArray { side } => write!(f, "template {side} must be an array"),
            Self::TopicNotText => f.write_str("template topic must be text"),
            Self::TopicDuplicateOrEmpty => f.write_str("duplicate or empty template topic"),
            Self::BoundaryWithoutTopic => f.write_str("template boundary has no topic label"),
            Self::BoundaryTopicDuplicate => f.write_str("duplicate template boundary topic"),
            Self::TopicsDisagree { side } => {
                write!(f, "template {side} topics disagree with its boundary")
            }
            Self::RegistrationOutsideRoot => {
                f.write_str("template registration belongs to the project root")
            }
            Self::NotAdmitted => {
                f.write_str("declaration is not admitted in template reconstruction")
            }
        }
    }
}

circular_core::closed_table! {
    pub enum EnvironmentSide {
        Before => "before_environment",
        After => "after_environment",
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PersistedFault {
    EntryNotObject,
    FieldAbsent {
        field: &'static str,
    },
    FieldNotBytes {
        field: &'static str,
    },
    ExceedsU64 {
        field: &'static str,
    },
    NotNonNegativeInt {
        field: &'static str,
    },
    ProjectCreationNotObject,
    ProjectCreationEnvironment(PayloadRejection),
    ProjectNot32Bytes,
    ProjectNotBytes,
    RepeatsProjectCreation,
    ReplayCursorExhausted,
    ReplayCursorMismatch {
        found: u64,
        expected: u64,
    },
    RepeatsCommitId,
    CommandNotObject,
    CommandWithoutKind,
    CommandDecode {
        kind: String,
        error: PayloadRejection,
    },
    CommandMoveToScope(MoveToScopeFault),
    TerminalNotObject,
    TerminalEnvironment {
        side: EnvironmentSide,
        error: PayloadRejection,
    },
    TerminalRevisionsNotArray,
    CommitWithoutRevision {
        scope: Vec<ScopeSegment>,
    },
    CommitTarget(PayloadRejection),
    CommitNotObject,
    CommitCommandsNotArray,
    CommitTerminalDisagrees,
    RevisionNot32Bytes {
        field: &'static str,
    },
    RevisionUnknownArm {
        field: &'static str,
    },
    RevisionNotSum {
        field: &'static str,
    },
    TransitionNotObject,
    TransitionScope(PayloadRejection),
    TransitionUnknownField {
        field: String,
    },
}

impl fmt::Display for PersistedFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EntryNotObject => f.write_str("authoring journal entry is not an object"),
            Self::FieldAbsent { field } => {
                write!(f, "persisted authoring field {field:?} is absent")
            }
            Self::FieldNotBytes { field } => {
                write!(f, "persisted authoring field {field:?} is not Bytes")
            }
            Self::ExceedsU64 { field } => write!(f, "persisted {field} exceeds u64"),
            Self::NotNonNegativeInt { field } => {
                write!(f, "persisted {field} is not a non-negative Int")
            }
            Self::ProjectCreationNotObject => f.write_str("project creation is not an object"),
            Self::ProjectCreationEnvironment(error) => {
                write!(f, "project creation environment: {error}")
            }
            Self::ProjectNot32Bytes => f.write_str("project creation project is not 32 bytes"),
            Self::ProjectNotBytes => f.write_str("project creation project is not Bytes"),
            Self::RepeatsProjectCreation => {
                f.write_str("authoring journal repeats its project creation")
            }
            Self::ReplayCursorExhausted => {
                f.write_str("authoring cursor exhausted while replaying journal")
            }
            Self::ReplayCursorMismatch { found, expected } => write!(
                f,
                "authoring journal epoch cursor is {found}, expected {expected}"
            ),
            Self::RepeatsCommitId => f.write_str("authoring journal repeats a CommitId"),
            Self::CommandNotObject => {
                f.write_str("persisted accepted declaration command is not an object")
            }
            Self::CommandWithoutKind => {
                f.write_str("persisted accepted declaration command has no String kind")
            }
            Self::CommandDecode { kind, error } => {
                write!(f, "persisted accepted {kind}: {error}")
            }
            Self::CommandMoveToScope(fault) => write!(
                f,
                "persisted accepted MoveToScope {}: {fault}",
                fault.rejection()
            ),
            Self::TerminalNotObject => f.write_str("committed terminal is not an object"),
            Self::TerminalEnvironment { side, error } => {
                write!(f, "committed terminal {side}: {error}")
            }
            Self::TerminalRevisionsNotArray => {
                f.write_str("committed terminal revisions are not an Array")
            }
            Self::CommitWithoutRevision { scope } => write!(
                f,
                "persisted authoring commits carry no revision for {}, which stands",
                spelled_scope(scope)
            ),
            Self::CommitTarget(error) => write!(f, "persisted authoring commit target: {error}"),
            Self::CommitNotObject => f.write_str("persisted authoring commit is not an object"),
            Self::CommitCommandsNotArray => {
                f.write_str("persisted authoring commit commands are not an Array")
            }
            Self::CommitTerminalDisagrees => {
                f.write_str("persisted authoring commit terminal disagrees with its metadata")
            }
            Self::RevisionNot32Bytes { field } => {
                write!(f, "persisted {field} revision is not 32-byte Bytes")
            }
            Self::RevisionUnknownArm { field } => {
                write!(f, "persisted {field} revision has an unknown arm")
            }
            Self::RevisionNotSum { field } => {
                write!(
                    f,
                    "persisted {field} revision is not a current-revision sum"
                )
            }
            Self::TransitionNotObject => {
                f.write_str("persisted revision transition is not an object")
            }
            Self::TransitionScope(error) => {
                write!(f, "persisted revision transition scope: {error}")
            }
            Self::TransitionUnknownField { field } => {
                write!(f, "unknown persisted revision transition field {field:?}")
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JournalVocabulary {
    EntryVersion,
    EntryField(String),
    ProjectCreationField(String),
    NoProjectCreation,
    CommandKind(String),
    TerminalField(String),
    CommitField(String),
}

impl fmt::Display for JournalVocabulary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EntryVersion => f.write_str("unsupported authoring journal entry version"),
            Self::EntryField(field) => write!(f, "unknown authoring journal entry field {field:?}"),
            Self::ProjectCreationField(field) => {
                write!(f, "unknown project creation field {field:?}")
            }
            Self::NoProjectCreation => {
                f.write_str("authoring journal does not begin with a project creation record")
            }
            Self::CommandKind(kind) => write!(f, "unknown persisted accepted command {kind:?}"),
            Self::TerminalField(field) => write!(f, "unknown committed terminal field {field:?}"),
            Self::CommitField(field) => {
                write!(f, "unknown persisted authoring commit field {field:?}")
            }
        }
    }
}

fn spelled_scope(scope: &[ScopeSegment]) -> circular_core::spelling::ScopeText<'_, ScopeSegment> {
    circular_protocol::scope_identity::spelled_scope(scope)
}

struct RolesText<'a>(&'a ExportRoles);

impl fmt::Display for RolesText<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let roles = [
            ("request", &self.0.request),
            ("progress", &self.0.progress),
            ("result", &self.0.result),
            ("error", &self.0.error),
        ];
        let mut bound = roles
            .iter()
            .filter_map(|(role, binding)| binding.as_ref().map(|binding| (role, binding)))
            .peekable();
        if bound.peek().is_none() {
            return f.write_str("none");
        }
        for (index, (role, (actor, port))) in bound.enumerate() {
            if index > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{role} {actor}.{port}")?;
        }
        Ok(())
    }
}
