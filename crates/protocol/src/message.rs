
use crate::{DeclarationVerb, LedgerTransitionVerb};
use std::fmt;

pub trait DeclarationDomain {
    type Scope;
    type CommitId;
    type ScopeDeclaration;
    type EpochId;
    type ExpectedRevision;
    type ActorId;
    type ActorDecl;
    type EdgeId;
    type EdgeAttrs;
    type ScopeSeg;
    type ExportName;
    type ExportMount;
    type AnnotationId;
    type Annotation;
    type PresentationOwner;
    type Presentation;
    type Flags;
    type AuthoringEnvironment;
    type TemplateName;
    type TemplateCommands;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeclarationCommand<D: DeclarationDomain> {
    BeginEpoch {
        scope: D::Scope,
        commit_id: D::CommitId,
        expected_revision: D::ExpectedRevision,
        expected_environment: D::AuthoringEnvironment,
    },
    ValidateEpoch {
        epoch: D::EpochId,
    },
    CommitEpoch {
        epoch: D::EpochId,
    },
    AbortEpoch {
        epoch: D::EpochId,
    },
    UpsertActor {
        epoch: D::EpochId,
        id: D::ActorId,
        declaration: D::ActorDecl,
    },
    RetireActor {
        epoch: D::EpochId,
        id: D::ActorId,
    },
    UpsertEdge {
        epoch: D::EpochId,
        id: D::EdgeId,
        attributes: D::EdgeAttrs,
    },
    RetireEdge {
        epoch: D::EpochId,
        id: D::EdgeId,
    },
    UpsertScope {
        epoch: D::EpochId,
        segment: D::ScopeSeg,
        declaration: D::ScopeDeclaration,
    },
    RetireScope {
        epoch: D::EpochId,
        segment: D::ScopeSeg,
    },
    MoveToScope {
        epoch: D::EpochId,
        actors: Vec<D::ActorId>,
        target: D::ScopeSeg,
    },
    UpsertExportMount {
        epoch: D::EpochId,
        name: D::ExportName,
        mount: D::ExportMount,
    },
    RetireExportMount {
        epoch: D::EpochId,
        name: D::ExportName,
    },
    UpsertAnnotation {
        epoch: D::EpochId,
        id: D::AnnotationId,
        annotation: D::Annotation,
    },
    RetireAnnotation {
        epoch: D::EpochId,
        id: D::AnnotationId,
    },
    SetPresentation {
        epoch: D::EpochId,
        owner: D::PresentationOwner,
        presentation: D::Presentation,
    },
    SetFlags {
        epoch: D::EpochId,
        actor: D::ActorId,
        flags: D::Flags,
    },
    UpsertTemplate {
        epoch: D::EpochId,
        name: D::TemplateName,
        commands: D::TemplateCommands,
    },
    RetireTemplate {
        epoch: D::EpochId,
        name: D::TemplateName,
    },
}

impl<D: DeclarationDomain> DeclarationCommand<D> {
    #[must_use]
    pub const fn verb(&self) -> DeclarationVerb {
        match self {
            Self::BeginEpoch { .. } => DeclarationVerb::BeginEpoch,
            Self::ValidateEpoch { .. } => DeclarationVerb::ValidateEpoch,
            Self::CommitEpoch { .. } => DeclarationVerb::CommitEpoch,
            Self::AbortEpoch { .. } => DeclarationVerb::AbortEpoch,
            Self::UpsertActor { .. } => DeclarationVerb::UpsertActor,
            Self::RetireActor { .. } => DeclarationVerb::RetireActor,
            Self::UpsertEdge { .. } => DeclarationVerb::UpsertEdge,
            Self::RetireEdge { .. } => DeclarationVerb::RetireEdge,
            Self::UpsertScope { .. } => DeclarationVerb::UpsertScope,
            Self::RetireScope { .. } => DeclarationVerb::RetireScope,
            Self::MoveToScope { .. } => DeclarationVerb::MoveToScope,
            Self::UpsertExportMount { .. } => DeclarationVerb::UpsertExportMount,
            Self::RetireExportMount { .. } => DeclarationVerb::RetireExportMount,
            Self::UpsertAnnotation { .. } => DeclarationVerb::UpsertAnnotation,
            Self::RetireAnnotation { .. } => DeclarationVerb::RetireAnnotation,
            Self::SetPresentation { .. } => DeclarationVerb::SetPresentation,
            Self::SetFlags { .. } => DeclarationVerb::SetFlags,
            Self::UpsertTemplate { .. } => DeclarationVerb::UpsertTemplate,
            Self::RetireTemplate { .. } => DeclarationVerb::RetireTemplate,
        }
    }

    #[must_use]
    pub const fn epoch(&self) -> Option<&D::EpochId> {
        match self {
            Self::BeginEpoch { .. } => None,
            Self::ValidateEpoch { epoch }
            | Self::CommitEpoch { epoch, .. }
            | Self::AbortEpoch { epoch }
            | Self::UpsertActor { epoch, .. }
            | Self::RetireActor { epoch, .. }
            | Self::UpsertEdge { epoch, .. }
            | Self::RetireEdge { epoch, .. }
            | Self::UpsertScope { epoch, .. }
            | Self::RetireScope { epoch, .. }
            | Self::MoveToScope { epoch, .. }
            | Self::UpsertExportMount { epoch, .. }
            | Self::RetireExportMount { epoch, .. }
            | Self::UpsertAnnotation { epoch, .. }
            | Self::RetireAnnotation { epoch, .. }
            | Self::SetPresentation { epoch, .. }
            | Self::SetFlags { epoch, .. }
            | Self::UpsertTemplate { epoch, .. }
            | Self::RetireTemplate { epoch, .. } => Some(epoch),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandResult<A, R> {
    Accepted(A),
    Rejected(R),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Injection<M, V, K> {
    mount: M,
    payload: V,
    idempotency: K,
}

impl<M, V, K> Injection<M, V, K> {
    #[must_use]
    pub const fn new(mount: M, payload: V, idempotency: K) -> Self {
        Self {
            mount,
            payload,
            idempotency,
        }
    }

    #[must_use]
    pub const fn mount(&self) -> &M {
        &self.mount
    }

    #[must_use]
    pub const fn payload(&self) -> &V {
        &self.payload
    }

    #[must_use]
    pub const fn idempotency(&self) -> &K {
        &self.idempotency
    }
}

pub trait LedgerDomain {
    type ApprovalItem;
    type ApprovalDecision;
    type ObservationControl;
    type AgentHarness;
}

/// Domain-owned identities carried by stable run lifecycle requests.
pub trait LifecycleDomain {
    type AuthoringRevision;
}

/// Manual interruption mode. Wire ordinals follow declaration order.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PauseMode {
    #[default]
    Pause,
    ForcePause,
}

/// A lifecycle request for the single standing product run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Lifecycle<D: LifecycleDomain> {
    /// Start exactly the currently observed authored cut.
    Resume {
        expected_authoring_revision: D::AuthoringRevision,
    },
    /// Pause the pipeline served by this state directory.
    Pause { mode: Option<PauseMode> },
}

impl<D: LifecycleDomain> Lifecycle<D> {
    /// The payload determines its own stable verb.
    #[must_use]
    pub const fn verb(&self) -> crate::LifecycleVerb {
        match self {
            Self::Resume { .. } => crate::LifecycleVerb::Resume,
            Self::Pause { .. } => crate::LifecycleVerb::Pause,
        }
    }
}

/// Accepted lifecycle transition of the standing pipeline.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LifecycleAccepted {
    Resumed,
    Paused,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LedgerTransition<D: LedgerDomain> {
    ApprovalDecide {
        item: D::ApprovalItem,
        decision: D::ApprovalDecision,
    },
    SetObservationControl {
        control: D::ObservationControl,
    },
    SetAgentHarness {
        harness: D::AgentHarness,
    },
}

impl<D: LedgerDomain> LedgerTransition<D> {
    #[must_use]
    pub const fn verb(&self) -> LedgerTransitionVerb {
        match self {
            Self::ApprovalDecide { .. } => LedgerTransitionVerb::ApprovalDecide,
            Self::SetObservationControl { .. } => LedgerTransitionVerb::SetObservationControl,
            Self::SetAgentHarness { .. } => LedgerTransitionVerb::SetAgentHarness,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Anchor<T, L> {
    Topology(T),
    LogUpperBound(L),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cursor<A, D, P> {
    anchor: A,
    domain: D,
    position: P,
}

impl<A, D, P> Cursor<A, D, P> {
    #[must_use]
    pub const fn new(anchor: A, domain: D, position: P) -> Self {
        Self {
            anchor,
            domain,
            position,
        }
    }

    #[must_use]
    pub const fn anchor(&self) -> &A {
        &self.anchor
    }

    #[must_use]
    pub const fn domain(&self) -> &D {
        &self.domain
    }

    #[must_use]
    pub const fn position(&self) -> &P {
        &self.position
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PageStep<L, C> {
    First { limit: L },
    Continue { limit: L, cursor: C },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PageEnd<C, D> {
    More { next: C },
    Complete,
    Diagnostic { code: D },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryRequest<N, A, L, C, K> {
    name: N,
    arguments: A,
    page: PageStep<L, C>,
    since: Option<K>,
    upto: Option<K>,
    lens: Option<u32>,
}

impl<N, A, L, C, K> QueryRequest<N, A, L, C, K> {
    #[must_use]
    pub const fn new(name: N, arguments: A, page: PageStep<L, C>, since: Option<K>) -> Self {
        Self {
            name,
            arguments,
            page,
            since,
            upto: None,
            lens: None,
        }
    }

    #[must_use]
    pub fn with_upto(mut self, upto: Option<K>) -> Self {
        self.upto = upto;
        self
    }

    #[must_use]
    pub fn with_lens(mut self, lens: Option<u32>) -> Self {
        self.lens = lens;
        self
    }

    #[must_use]
    pub const fn name(&self) -> &N {
        &self.name
    }

    #[must_use]
    pub const fn arguments(&self) -> &A {
        &self.arguments
    }

    #[must_use]
    pub const fn page(&self) -> &PageStep<L, C> {
        &self.page
    }

    #[must_use]
    pub const fn since(&self) -> Option<&K> {
        self.since.as_ref()
    }

    #[must_use]
    pub const fn upto(&self) -> Option<&K> {
        self.upto.as_ref()
    }

    #[must_use]
    pub const fn lens(&self) -> Option<u32> {
        self.lens
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueryPage<A, I, C, D, K, R> {
    anchor: A,
    items: Box<[I]>,
    end: PageEnd<C, D>,
    cut: Option<K>,
    folded_from: Option<K>,
    reached: Option<R>,
}

impl<A, I, C, D, K, R> QueryPage<A, I, C, D, K, R> {
    #[must_use]
    pub fn new(
        anchor: A,
        items: impl Into<Box<[I]>>,
        end: PageEnd<C, D>,
        cut: Option<K>,
        folded_from: Option<K>,
        reached: Option<R>,
    ) -> Self {
        Self {
            anchor,
            items: items.into(),
            end,
            cut,
            folded_from,
            reached,
        }
    }

    #[must_use]
    pub const fn anchor(&self) -> &A {
        &self.anchor
    }

    #[must_use]
    pub const fn items(&self) -> &[I] {
        &self.items
    }

    #[must_use]
    pub const fn end(&self) -> &PageEnd<C, D> {
        &self.end
    }

    #[must_use]
    pub const fn cut(&self) -> Option<&K> {
        self.cut.as_ref()
    }

    #[must_use]
    pub const fn folded_from(&self) -> Option<&K> {
        self.folded_from.as_ref()
    }

    #[must_use]
    pub const fn reached(&self) -> Option<&R> {
        self.reached.as_ref()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QueryResult<P, R> {
    Accepted(P),
    Rejected(R),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Target<N, A> {
    name: N,
    arguments: A,
}

impl<N, A> Target<N, A> {
    #[must_use]
    pub const fn new(name: N, arguments: A) -> Self {
        Self { name, arguments }
    }

    #[must_use]
    pub const fn name(&self) -> &N {
        &self.name
    }

    #[must_use]
    pub const fn arguments(&self) -> &A {
        &self.arguments
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryDiscipline {
    Lossless,
    Conflated,
    Credit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Subscribe<T> {
    target: T,
}

impl<T> Subscribe<T> {
    #[must_use]
    pub const fn new(target: T) -> Self {
        Self { target }
    }

    #[must_use]
    pub const fn target(&self) -> &T {
        &self.target
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CreditError;

impl fmt::Display for CreditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("credit must be a positive frame count")
    }
}

impl std::error::Error for CreditError {}

pub trait PositiveFrameCount {
    fn is_positive(&self) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PositiveCredit<N>(N);

impl<N: PositiveFrameCount> PositiveCredit<N> {
    pub fn try_new(frames: N) -> Result<Self, CreditError> {
        if frames.is_positive() {
            Ok(Self(frames))
        } else {
            Err(CreditError)
        }
    }
}

impl<N> PositiveCredit<N> {
    #[must_use]
    pub const fn frames(&self) -> &N {
        &self.0
    }

    #[must_use]
    pub fn into_frames(self) -> N {
        self.0
    }
}

pub use crate::subscription_payload::FrameOrigin;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubscriptionFrame<S, P> {
    Lossless { origin: FrameOrigin, payload: P },
    Conflated {
        origin: FrameOrigin,
        slot: Option<S>,
        folded: u32,
        payload: P,
    },
    Credit {
        origin: FrameOrigin,
        payload: P,
        pending_after: u64,
    },
    RetentionComplete { anchor: P, delivered: u64 },
}

pub trait SubscriptionTerminationDomain {
    type ResetFloorOrCursor: Clone + fmt::Debug + Eq;
    type StructureCursor: Clone + fmt::Debug + Eq;
    type AuthoringEnvironment: Clone + fmt::Debug + Eq;
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NoTerminationPayload;

impl SubscriptionTerminationDomain for NoTerminationPayload {
    type ResetFloorOrCursor = std::convert::Infallible;
    type StructureCursor = std::convert::Infallible;
    type AuthoringEnvironment = std::convert::Infallible;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubscriptionEndReason<T: SubscriptionTerminationDomain> {
    ByClient,
    ConsumerBehind,
    TargetGone,
    Withdrawn,
    SessionClosed,
    ResetRequired {
        floor_or_cursor: T::ResetFloorOrCursor,
    },
    ScopeGone {
        cursor: T::StructureCursor,
    },
    IncompatibleClient {
        required_environment: T::AuthoringEnvironment,
    },
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubscriptionEnded<T: SubscriptionTerminationDomain, A, D> {
    reason: SubscriptionEndReason<T>,
    diagnostic: D,
    anchor: A,
}

impl<T: SubscriptionTerminationDomain, A, D> SubscriptionEnded<T, A, D> {
    #[must_use]
    pub const fn new(reason: SubscriptionEndReason<T>, diagnostic: D, anchor: A) -> Self {
        Self {
            reason,
            diagnostic,
            anchor,
        }
    }

    #[must_use]
    pub const fn reason(&self) -> &SubscriptionEndReason<T> {
        &self.reason
    }

    #[must_use]
    pub const fn diagnostic(&self) -> &D {
        &self.diagnostic
    }

    #[must_use]
    pub const fn anchor(&self) -> &A {
        &self.anchor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestDeclarationDomain;

    impl DeclarationDomain for TestDeclarationDomain {
        type CommitId = &'static str;
        type ScopeDeclaration = &'static str;
        type Scope = &'static str;
        type EpochId = &'static str;
        type ExpectedRevision = &'static str;
        type ActorId = &'static str;
        type ActorDecl = &'static str;
        type EdgeId = &'static str;
        type EdgeAttrs = &'static str;
        type ScopeSeg = &'static str;
        type ExportName = &'static str;
        type ExportMount = &'static str;
        type AnnotationId = &'static str;
        type Annotation = &'static str;
        type PresentationOwner = &'static str;
        type Presentation = &'static str;
        type Flags = &'static str;
        type AuthoringEnvironment = &'static str;
        type TemplateName = String;
        type TemplateCommands = Vec<circular_core::Value>;
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestFrameCount(u16);

    impl PositiveFrameCount for TestFrameCount {
        fn is_positive(&self) -> bool {
            self.0 > 0
        }
    }

    #[test]
    fn begin_carries_the_published_baseline_and_later_declarations_name_the_epoch() {
        let begin: DeclarationCommand<TestDeclarationDomain> = DeclarationCommand::BeginEpoch {
            scope: "root",
            commit_id: "commit",
            expected_revision: "revision",
            expected_environment: "environment",
        };
        assert_eq!(begin.epoch(), None);

        let commands: [DeclarationCommand<TestDeclarationDomain>; 16] = [
            DeclarationCommand::ValidateEpoch { epoch: "epoch" },
            DeclarationCommand::CommitEpoch { epoch: "epoch" },
            DeclarationCommand::AbortEpoch { epoch: "epoch" },
            DeclarationCommand::UpsertActor {
                epoch: "epoch",
                id: "actor",
                declaration: "declaration",
            },
            DeclarationCommand::RetireActor {
                epoch: "epoch",
                id: "actor",
            },
            DeclarationCommand::UpsertEdge {
                epoch: "epoch",
                id: "edge",
                attributes: "attributes",
            },
            DeclarationCommand::RetireEdge {
                epoch: "epoch",
                id: "edge",
            },
            DeclarationCommand::UpsertScope {
                epoch: "epoch",
                segment: "scope",
                declaration: "scope-declaration",
            },
            DeclarationCommand::RetireScope {
                epoch: "epoch",
                segment: "scope",
            },
            DeclarationCommand::MoveToScope {
                epoch: "epoch",
                actors: vec!["actor"],
                target: "scope",
            },
            DeclarationCommand::UpsertExportMount {
                epoch: "epoch",
                name: "export",
                mount: "mount",
            },
            DeclarationCommand::RetireExportMount {
                epoch: "epoch",
                name: "export",
            },
            DeclarationCommand::UpsertAnnotation {
                epoch: "epoch",
                id: "annotation",
                annotation: "value",
            },
            DeclarationCommand::RetireAnnotation {
                epoch: "epoch",
                id: "annotation",
            },
            DeclarationCommand::SetPresentation {
                epoch: "epoch",
                owner: "actor",
                presentation: "presentation",
            },
            DeclarationCommand::SetFlags {
                epoch: "epoch",
                actor: "actor",
                flags: "flags",
            },
        ];

        assert!(
            commands
                .iter()
                .all(|command| command.epoch() == Some(&"epoch"))
        );
    }

    #[test]
    fn credit_rejects_zero() {
        assert_eq!(PositiveCredit::try_new(TestFrameCount(0)), Err(CreditError));
        assert_eq!(
            PositiveCredit::try_new(TestFrameCount(3)).map(PositiveCredit::into_frames),
            Ok(TestFrameCount(3))
        );
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestTermination;

    impl SubscriptionTerminationDomain for TestTermination {
        type ResetFloorOrCursor = &'static str;
        type StructureCursor = &'static str;
        type AuthoringEnvironment = &'static str;
    }

    #[test]
    fn subscription_termination_always_has_reason_diagnostic_and_anchor() {
        let ended = SubscriptionEnded::<TestTermination, _, _>::new(
            SubscriptionEndReason::TargetGone,
            "CIR-WIRE",
            7_u64,
        );
        assert_eq!(ended.reason(), &SubscriptionEndReason::TargetGone);
        assert_eq!(ended.diagnostic(), &"CIR-WIRE");
        assert_eq!(ended.anchor(), &7);
    }

    #[test]
    fn the_three_payload_carrying_reasons_hold_their_value() {
        let reset = SubscriptionEndReason::<TestTermination>::ResetRequired {
            floor_or_cursor: "floor-3",
        };
        let gone = SubscriptionEndReason::<TestTermination>::ScopeGone { cursor: "cursor-9" };
        let incompatible = SubscriptionEndReason::<TestTermination>::IncompatibleClient {
            required_environment: "env-2",
        };

        assert!(matches!(
            reset,
            SubscriptionEndReason::ResetRequired {
                floor_or_cursor: "floor-3"
            }
        ));
        assert!(matches!(
            gone,
            SubscriptionEndReason::ScopeGone { cursor: "cursor-9" }
        ));
        assert!(matches!(
            incompatible,
            SubscriptionEndReason::IncompatibleClient {
                required_environment: "env-2"
            }
        ));
    }
}
