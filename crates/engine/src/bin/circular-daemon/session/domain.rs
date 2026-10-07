//! The physical command codecs mapped to the existing semantic payload union.
use super::super::session_policy::{DaemonScope, SessionDecodeRejection, SessionPolicyError};
use circular_core::{Boundary, Ceilings, Value};
use circular_protocol::{self as p, declaration_payload as w};
use std::convert::Infallible;
use std::num::NonZeroU64;

pub(super) struct Domain;
impl p::DeclarationDomain for Domain {
    type Scope = w::ScopeAddress;
    type CommitId = Vec<u8>;
    type ExpectedRevision = w::ExpectedRevision;
    type EpochId = Option<Vec<u8>>;
    type ScopeDeclaration = w::ScopeDeclaration;
    type ActorId = w::AddressRef<w::PlanActorKey>;
    type ActorDecl = w::ActorDeclaration;
    type EdgeId = w::AddressRef<w::DeclaredEdgeKey>;
    type EdgeAttrs = w::EdgeDeclaration;
    type ScopeSeg = w::ScopeAddress;
    type ExportName = w::AddressRef<w::PlanExportKey>;
    type ExportMount = w::ExportDeclaration;
    type AnnotationId = w::AddressRef<w::PlanAnnotationKey>;
    type Annotation = w::AnnotationDeclaration;
    type PresentationOwner =
        w::PresentationOwner<w::AddressRef<w::PlanActorKey>, w::AddressRef<w::PlanAnnotationKey>>;
    type Presentation = w::Presentation<w::PlanActorKey>;
    type Flags = w::ActorFlags;
    type AuthoringEnvironment = w::AuthoringEnvironment;
    type TemplateName = String;
    type TemplateCommands = Vec<Value>;
}
impl p::LedgerDomain for Domain {
    type ApprovalItem = p::approval_payload::ApprovalDecisionItem;
    type ApprovalDecision = p::approval_payload::ApprovalDecisionValue;
    type ObservationControl = Value;
    type AgentHarness = p::agent_harness_payload::SetAgentHarness;
}
pub(super) struct Termination;
impl p::SubscriptionTerminationDomain for Termination {
    type ResetFloorOrCursor = Vec<u8>;
    type StructureCursor = Vec<u8>;
    type AuthoringEnvironment = w::AuthoringEnvironment;
}
use crate::daemon::subscription::FrameCredits;
pub(super) struct Replay {
    pub verb: p::ReplayControlVerb,
    pub bytes: Vec<u8>,
}
impl p::ReplayControlPayload for Replay {
    fn verb(&self) -> p::ReplayControlVerb {
        self.verb
    }
}
pub(super) struct Experimental(Infallible);
impl p::ExperimentalPayload for Experimental {
    type Verb = Infallible;
    fn verb(&self) -> Infallible {
        self.0
    }
}
impl p::SessionDomain for Domain {
    type ProtocolVersion = u16;
    type FeatureMinor = u8;
    type Scope = DaemonScope;
    type SessionToken = p::SessionToken;
    type EstablishmentRejection = p::EstablishmentRejection<SessionPolicyError>;
    type Declaration = Self;
    type DeclarationAccepted = w::Accepted;
    type DeclarationRejection = w::Rejected;
    type QueryName = String;
    type QueryArguments = Value;
    type PageLimit = Option<NonZeroU64>;
    type PageCursor = Value;
    type QuerySince = p::replay_payload::LogCut;
    type QueryAnchor = Value;
    type QueryItem = Value;
    type PageCut = Value;
    type PageReached = Value;
    type PageDiagnostic = u32;
    type QueryRejection = w::Rejected;
    type SubscriptionTarget = p::subscription_payload::Subscribe;
    type Credit = FrameCredits;
    type SubscriptionOpened = w::Accepted;
    type SubscriptionRejection = w::Rejected;
    type ConflationSlot = Value;
    type FramePayload = Value;
    type SubscriptionAnchor = Vec<u8>;
    type SubscriptionDiagnostic = u32;
    type SubscriptionTermination = Termination;
    type InjectionMount = w::PlanExportKey;
    type InjectionPayload = Value;
    type IdempotencyKey = Vec<u8>;
    type InjectionAccepted = w::Accepted;
    type InjectionRejection = w::Rejected;
    type Ledger = Self;
    type TransitionAccepted = w::Accepted;
    type TransitionRejection = w::Rejected;
    type Lifecycle = p::lifecycle_payload::WireLifecycleDomain;
    type LifecycleRejection = w::Rejected;
    type ReplayControl = Replay;
    type ExperimentalVerb = Infallible;
    type Experimental = Experimental;
}
pub(super) type Payload = p::SessionPayload<Domain>;
pub(super) type Command = p::DeclarationCommand<Domain>;
const C: Ceilings = Ceilings::for_boundary(Boundary::Wire);
const M: w::AddressContext = w::AddressContext::Mutation;
pub(super) fn declaration(
    verb: p::DeclarationVerb,
    bytes: &[u8],
    epoch: Option<Vec<u8>>,
) -> Result<Command, w::PayloadRejection> {
    use Command as D;
    use p::DeclarationVerb as V;
    Ok(match verb {
        V::BeginEpoch => {
            let b = w::decode_begin_epoch(bytes, w::AddressContext::Mutation, C)?;
            D::BeginEpoch {
                scope: b.scope,
                commit_id: b.commit_id,
                expected_revision: b.expected_revision,
                expected_environment: b.expected_environment,
            }
        }
        V::ValidateEpoch | V::CommitEpoch | V::AbortEpoch => {
            let epoch = Some(w::decode_epoch_ref(bytes, C)?.epoch);
            match verb {
                V::ValidateEpoch => D::ValidateEpoch { epoch },
                V::CommitEpoch => D::CommitEpoch { epoch },
                _ => D::AbortEpoch { epoch },
            }
        }
        V::UpsertTemplate => {
            let b = w::decode_upsert_template(bytes, C)?;
            D::UpsertTemplate {
                epoch,
                name: b.name,
                commands: b.commands,
            }
        }
        V::RetireTemplate => {
            let b = w::decode_retire_template(bytes, C)?;
            D::RetireTemplate {
                epoch,
                name: b.name,
            }
        }
        V::UpsertActor => {
            let b = w::decode_upsert_actor(bytes, M, C)?;
            D::UpsertActor {
                epoch,
                id: b.actor,
                declaration: b.declaration,
            }
        }
        V::RetireActor => {
            let b = w::decode_retire_actor(bytes, M, C)?;
            D::RetireActor { epoch, id: b.actor }
        }
        V::UpsertEdge => {
            let b = w::decode_upsert_edge(bytes, M, C)?;
            D::UpsertEdge {
                epoch,
                id: b.edge,
                attributes: b.declaration,
            }
        }
        V::RetireEdge => {
            let b = w::decode_retire_edge(bytes, M, C)?;
            D::RetireEdge { epoch, id: b.edge }
        }
        V::UpsertScope => {
            let b = w::decode_upsert_scope(bytes, M, C)?;
            D::UpsertScope {
                epoch,
                segment: b.scope,
                declaration: b.declaration,
            }
        }
        V::RetireScope => {
            let b = w::decode_retire_scope(bytes, M, C)?;
            D::RetireScope {
                epoch,
                segment: b.scope,
            }
        }
        V::MoveToScope => {
            let b = w::decode_move_to_scope(bytes, M, C)?;
            D::MoveToScope {
                epoch,
                actors: b.actors,
                target: b.target,
            }
        }
        V::UpsertExportMount => {
            let b = w::decode_upsert_export_mount(bytes, M, C)?;
            D::UpsertExportMount {
                epoch,
                name: b.mount,
                mount: b.declaration,
            }
        }
        V::RetireExportMount => {
            let b = w::decode_retire_export_mount(bytes, M, C)?;
            D::RetireExportMount {
                epoch,
                name: b.mount,
            }
        }
        V::UpsertAnnotation => {
            let b = w::decode_upsert_annotation(bytes, M, C)?;
            D::UpsertAnnotation {
                epoch,
                id: b.annotation,
                annotation: b.declaration,
            }
        }
        V::RetireAnnotation => {
            let b = w::decode_retire_annotation(bytes, M, C)?;
            D::RetireAnnotation {
                epoch,
                id: b.annotation,
            }
        }
        V::SetPresentation => {
            let b = w::decode_set_presentation(bytes, M, C)?;
            D::SetPresentation {
                epoch,
                owner: b.owner,
                presentation: b.presentation,
            }
        }
        V::SetFlags => {
            let b = w::decode_set_flags(bytes, M, C)?;
            D::SetFlags {
                epoch,
                actor: b.actor,
                flags: b.flags,
            }
        }
        V::CommandResult => return Err(w::PayloadRejection::ArmNotAdmitted),
    })
}

pub(super) fn decode(
    header: p::EnvelopeHeader,
    bytes: &[u8],
    epoch: Option<Vec<u8>>,
) -> Result<Payload, SessionDecodeRejection> {
    use p::StableVerb as V;
    Ok(match header.verb() {
        V::SessionMechanics(p::SessionMechanicsVerb::Hello) => Payload::Hello(
            super::super::session_policy::decode_owner_hello(header.protocol_version(), bytes)?,
        ),
        verb @ V::SessionMechanics(p::SessionMechanicsVerb::Goodbye) => {
            verb.open_absent_body(bytes)
                .map_err(|_| SessionDecodeRejection::Malformed)?;
            Payload::Goodbye
        }
        V::Declaration(verb) => Payload::Declaration(
            declaration(verb, bytes, epoch).map_err(|_| SessionDecodeRejection::Malformed)?,
        ),
        V::Query(p::QueryVerb::Query) => {
            let q = w::decode_query(bytes, C).map_err(|_| SessionDecodeRejection::Malformed)?;
            let page = match q.page {
                None => p::PageStep::First { limit: None },
                Some(page) => match page.cursor {
                    None => p::PageStep::First {
                        limit: Some(page.limit),
                    },
                    Some(cursor) => p::PageStep::Continue {
                        limit: Some(page.limit),
                        cursor,
                    },
                },
            };
            Payload::Query(
                p::QueryRequest::new(q.name, q.args, page, q.since)
                    .with_upto(q.upto)
                    .with_lens(q.lens),
            )
        }
        verb @ V::Query(p::QueryVerb::QueryClose) => {
            verb.open_absent_body(bytes)
                .map_err(|_| SessionDecodeRejection::Malformed)?;
            Payload::QueryClose
        }
        V::Subscription(p::SubscriptionVerb::Subscribe) => Payload::Subscribe(p::Subscribe::new(
            p::subscription_payload::decode_subscribe(bytes, C)
                .map_err(|_| SessionDecodeRejection::Malformed)?,
        )),
        V::Subscription(p::SubscriptionVerb::Credit) => Payload::Credit(
            p::PositiveCredit::try_new(FrameCredits(
                p::subscription_payload::decode_credit(bytes, C)
                    .map_err(|_| SessionDecodeRejection::Malformed)?
                    .frames,
            ))
            .map_err(|_| SessionDecodeRejection::Malformed)?,
        ),
        verb @ V::Subscription(p::SubscriptionVerb::Unsubscribe) => {
            verb.open_absent_body(bytes)
                .map_err(|_| SessionDecodeRejection::Malformed)?;
            Payload::Unsubscribe
        }
        V::EventInjection(p::EventInjectionVerb::Inject) => {
            let i = w::decode_inject(bytes, C).map_err(|_| SessionDecodeRejection::Malformed)?;
            Payload::Inject(p::Injection::new(i.mount, i.payload, i.idempotency))
        }
        V::LedgerTransition(p::LedgerTransitionVerb::ApprovalDecide) => {
            let (item, decision) = p::approval_payload::decode_approval_decision(bytes, C)
                .map_err(|_| SessionDecodeRejection::Malformed)?;
            Payload::LedgerTransition(p::LedgerTransition::ApprovalDecide { item, decision })
        }
        V::LedgerTransition(p::LedgerTransitionVerb::SetObservationControl) => {
            Payload::LedgerTransition(p::LedgerTransition::SetObservationControl {
                control: circular_core::decode(bytes, C)
                    .map_err(|_| SessionDecodeRejection::Malformed)?,
            })
        }
        V::LedgerTransition(p::LedgerTransitionVerb::SetAgentHarness) => {
            Payload::LedgerTransition(p::LedgerTransition::SetAgentHarness {
                harness: p::agent_harness_payload::decode_set_agent_harness(bytes, C)
                    .map_err(|_| SessionDecodeRejection::Malformed)?,
            })
        }
        V::Lifecycle(p::LifecycleVerb::Resume) => Payload::Lifecycle(
            p::lifecycle_payload::decode_resume(bytes, C)
                .map_err(|_| SessionDecodeRejection::Malformed)?,
        ),
        V::Lifecycle(p::LifecycleVerb::Pause) => Payload::Lifecycle(
            p::lifecycle_payload::decode_pause(bytes, C)
                .map_err(|_| SessionDecodeRejection::Malformed)?,
        ),
        V::ReplayControl(verb) if verb != p::ReplayControlVerb::ReplayResult => {
            Payload::ReplayControl(Replay {
                verb,
                bytes: bytes.to_vec(),
            })
        }
        _ => return Err(SessionDecodeRejection::Malformed),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn object(fields: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
        Value::object(fields).unwrap()
    }

    #[test]
    fn semantic_declarations_preserve_the_published_baseline_and_scope_tree() {
        let begin = object([
            (
                "scope",
                Value::Array(vec![Value::Int(1), Value::Array(vec![])]),
            ),
            ("commit_id", Value::Bytes(vec![7, 8])),
            (
                "expected_revision",
                Value::Array(vec![Value::Int(2), Value::Bytes(vec![9])]),
            ),
            (
                "expected_environment",
                object([
                    ("declaration_schema", Value::Bytes(vec![1])),
                    ("spec_set", Value::Bytes(vec![3])),
                ]),
            ),
        ]);
        let bytes = circular_core::encode(&begin, C).unwrap();
        let Command::BeginEpoch {
            scope,
            commit_id,
            expected_revision,
            expected_environment,
        } = declaration(p::DeclarationVerb::BeginEpoch, &bytes, None).unwrap()
        else {
            panic!("begin epoch arm")
        };
        assert_eq!(scope, w::AddressRef::Absolute(vec![]));
        assert_eq!(commit_id, [7, 8]);
        assert_eq!(expected_revision, w::ExpectedRevision::At(vec![9]));
        assert_eq!(expected_environment.declaration_schema, [1]);
        assert_eq!(expected_environment.spec_set, [3]);

        let scope = object([
            (
                "scope",
                Value::Array(vec![
                    Value::Int(2),
                    Value::Array(vec![Value::Array(vec![
                        Value::Int(1),
                        Value::String("child".into()),
                    ])]),
                ]),
            ),
            (
                "declaration",
                object([
                    ("role", Value::Int(2)),
                    (
                        "boundary",
                        object([
                            ("inlets", Value::Array(vec![])),
                            ("outlets", Value::Array(vec![])),
                        ]),
                    ),
                ]),
            ),
        ]);
        let bytes = circular_core::encode(&scope, C).unwrap();
        let Command::UpsertScope {
            epoch,
            segment,
            declaration,
        } = declaration(p::DeclarationVerb::UpsertScope, &bytes, Some(vec![11])).unwrap()
        else {
            panic!("scope arm")
        };
        assert_eq!(epoch, Some(vec![11]));
        assert_eq!(
            segment,
            w::AddressRef::EpochLocal(vec![w::ScopeSegment::Child("child".into())])
        );
        assert_eq!(declaration.role, w::ScopeRole::Template);
        assert!(declaration.boundary.inlets.is_empty());
        assert!(declaration.boundary.outlets.is_empty());
    }
}
