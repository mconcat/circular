
use circular_core::{Boundary, Ceilings, Value, decode};
use circular_protocol::DeclarationVerb;
use circular_protocol::declaration_payload::PresentationOwner;
use circular_protocol::declaration_payload::{
    self as wire, ActorDeclaration, ActorFlags, AddressContext, AddressRef, AnnotationDeclaration,
    AuthoringEnvironment, DeclaredEdgeKey, EdgeDeclaration, ExportDeclaration, PayloadRejection,
    PlanActorKey, PlanAnnotationKey, PlanExportKey, Presentation, ScopeDeclaration, ScopeSegment,
};

use circular_protocol::authoring_snapshot::CompactedDeclarationRejection;

use super::ledger::JournalEntryRejection;
use super::rejection::{JournalVocabulary, PersistedFault};

circular_core::closed_table! {
    pub enum ContentKind {
        UpsertActor => "UpsertActor",
        RetireActor => "RetireActor",
        UpsertEdge => "UpsertEdge",
        RetireEdge => "RetireEdge",
        UpsertScope => "UpsertScope",
        RetireScope => "RetireScope",
        MoveToScope => "MoveToScope",
        UpsertExportMount => "UpsertExportMount",
        RetireExportMount => "RetireExportMount",
        UpsertAnnotation => "UpsertAnnotation",
        RetireAnnotation => "RetireAnnotation",
        SetPresentation => "SetPresentation",
        SetFlags => "SetFlags",
        UpsertTemplate => "UpsertTemplate",
        RetireTemplate => "RetireTemplate",
        ReplaceAuthoringEnvironment => "ReplaceAuthoringEnvironment",
    }
}

#[derive(Clone, Copy)]
enum AddressField {
    One(&'static str),
    Each(&'static str),
}

impl ContentKind {
    #[must_use]
    pub const fn of(verb: DeclarationVerb) -> Option<Self> {
        use DeclarationVerb as V;
        Some(match verb {
            V::UpsertActor => Self::UpsertActor,
            V::RetireActor => Self::RetireActor,
            V::UpsertEdge => Self::UpsertEdge,
            V::RetireEdge => Self::RetireEdge,
            V::UpsertScope => Self::UpsertScope,
            V::RetireScope => Self::RetireScope,
            V::MoveToScope => Self::MoveToScope,
            V::UpsertExportMount => Self::UpsertExportMount,
            V::RetireExportMount => Self::RetireExportMount,
            V::UpsertAnnotation => Self::UpsertAnnotation,
            V::RetireAnnotation => Self::RetireAnnotation,
            V::SetPresentation => Self::SetPresentation,
            V::SetFlags => Self::SetFlags,
            V::UpsertTemplate => Self::UpsertTemplate,
            V::RetireTemplate => Self::RetireTemplate,
            V::BeginEpoch
            | V::ValidateEpoch
            | V::CommitEpoch
            | V::AbortEpoch
            | V::CommandResult => return None,
        })
    }

    const fn address_fields(self) -> &'static [AddressField] {
        use AddressField::{Each, One};
        match self {
            Self::UpsertActor | Self::RetireActor | Self::SetFlags => &[One("actor")],
            Self::UpsertEdge | Self::RetireEdge => &[One("edge")],
            Self::UpsertScope | Self::RetireScope => &[One("scope")],
            Self::MoveToScope => &[One("target"), Each("actors")],
            Self::UpsertExportMount | Self::RetireExportMount => &[One("mount")],
            Self::UpsertAnnotation | Self::RetireAnnotation => &[One("annotation")],
            Self::SetPresentation => &[One("owner")],
            Self::UpsertTemplate | Self::RetireTemplate | Self::ReplaceAuthoringEnvironment => &[],
        }
    }
}

pub fn accepted_item(verb: DeclarationVerb, payload: &[u8]) -> Result<Value, PayloadRejection> {
    let kind = ContentKind::of(verb).ok_or(PayloadRejection::ArmNotAdmitted)?;
    let Value::Object(object) =
        decode(payload, Ceilings::for_boundary(Boundary::Wire)).map_err(PayloadRejection::Codec)?
    else {
        return Err(PayloadRejection::NotAnObject);
    };
    let mut fields = object.into_map();
    for field in kind.address_fields() {
        match *field {
            AddressField::One(key) => {
                if key == "owner" {
                    let owner = wire::presentation_owner_from_value(
                        fields
                            .remove(key)
                            .ok_or(PayloadRejection::MissingKey(key))?,
                        AddressContext::Mutation,
                    )?;
                    let actor = |address: &AddressRef<PlanActorKey>| {
                        Ok::<_, PayloadRejection>(Value::array([
                            Value::Int(1),
                            wire::plan_actor_key_value(&identity(address.clone())),
                        ]))
                    };
                    let annotation = |address: &AddressRef<PlanAnnotationKey>| {
                        let key = identity(address.clone());
                        Ok::<_, PayloadRejection>(Value::array([
                            Value::Int(1),
                            Value::object([
                                ("local", Value::string(key.local)),
                                ("scope", wire::scope_identity_value(&key.scope)),
                            ])
                            .expect("annotation identity"),
                        ]))
                    };
                    fields.insert(
                        key.to_owned(),
                        wire::presentation_owner_value(&owner, actor, annotation)?,
                    );
                    continue;
                }
                absolute(
                    fields
                        .get_mut(key)
                        .ok_or(PayloadRejection::MissingKey(key))?,
                    key,
                )?;
            }
            AddressField::Each(key) => {
                let Value::Array(addresses) = fields
                    .get_mut(key)
                    .ok_or(PayloadRejection::MissingKey(key))?
                else {
                    return Err(PayloadRejection::WrongCarrier { key });
                };
                for address in addresses {
                    absolute(address, key)?;
                }
            }
        }
    }
    fields.insert("kind".to_owned(), Value::String(kind.as_str().to_owned()));
    Ok(Value::Object(fields.into()))
}

fn absolute(address: &mut Value, key: &'static str) -> Result<(), PayloadRejection> {
    let Value::Array(parts) = address else {
        return Err(PayloadRejection::WrongCarrier { key });
    };
    if parts.len() != 2 || !matches!(parts[0], Value::Int(1 | 2)) {
        return Err(PayloadRejection::ArmNotAdmitted);
    }
    parts[0] = Value::Int(1);
    Ok(())
}

#[derive(Clone, Debug, PartialEq)]
pub enum ContentVerb {
    UpsertActor {
        actor: PlanActorKey,
        declaration: ActorDeclaration,
    },
    RetireActor {
        actor: PlanActorKey,
    },
    UpsertEdge {
        declaration: EdgeDeclaration,
    },
    RetireEdge {
        edge: DeclaredEdgeKey,
    },
    UpsertScope {
        scope: Vec<ScopeSegment>,
        declaration: ScopeDeclaration,
    },
    RetireScope {
        scope: Vec<ScopeSegment>,
    },
    MoveToScope {
        actors: Vec<PlanActorKey>,
        target: Vec<ScopeSegment>,
    },
    UpsertExportMount {
        mount: PlanExportKey,
        declaration: ExportDeclaration,
    },
    RetireExportMount {
        mount: PlanExportKey,
    },
    UpsertAnnotation {
        annotation: PlanAnnotationKey,
        declaration: AnnotationDeclaration,
    },
    RetireAnnotation {
        annotation: PlanAnnotationKey,
    },
    SetPresentation {
        owner: PresentationOwner,
        presentation: Presentation<PlanActorKey>,
    },
    SetFlags {
        actor: PlanActorKey,
        flags: ActorFlags,
    },
    UpsertTemplate {
        name: String,
        commands: Vec<Value>,
    },
    RetireTemplate {
        name: String,
    },
    ReplaceAuthoringEnvironment {
        replacement: AuthoringEnvironment,
    },
}

impl ContentVerb {
    pub fn from_accepted(item: &Value) -> Result<Self, JournalEntryRejection> {
        let Value::Object(object) = item else {
            return Err(PersistedFault::CommandNotObject.into());
        };
        let mut fields = object.clone().into_map();
        let Some(Value::String(spelling)) = fields.remove("kind") else {
            return Err(PersistedFault::CommandWithoutKind.into());
        };
        let Some(kind) = ContentKind::from_str(&spelling) else {
            return Err(JournalEntryRejection::UnknownVocabulary(
                JournalVocabulary::CommandKind(spelling),
            ));
        };
        let body = Value::Object(fields.into());
        let corrupt = |error| PersistedFault::CommandDecode {
            kind: spelling.clone(),
            error,
        };
        let history = AddressContext::AcceptedHistory;
        Ok(match kind {
            ContentKind::UpsertActor => {
                let command = wire::upsert_actor_from_value(body, history).map_err(corrupt)?;
                Self::UpsertActor {
                    actor: identity(command.actor),
                    declaration: command.declaration,
                }
            }
            ContentKind::RetireActor => Self::RetireActor {
                actor: identity(
                    wire::retire_actor_from_value(body, history)
                        .map_err(corrupt)?
                        .actor,
                ),
            },
            ContentKind::UpsertEdge => Self::UpsertEdge {
                declaration: wire::upsert_edge_from_value(body, history)
                    .map_err(corrupt)?
                    .declaration,
            },
            ContentKind::RetireEdge => Self::RetireEdge {
                edge: identity(
                    wire::retire_edge_from_value(body, history)
                        .map_err(corrupt)?
                        .edge,
                ),
            },
            ContentKind::UpsertScope => {
                let command = wire::upsert_scope_from_value(body, history).map_err(corrupt)?;
                Self::UpsertScope {
                    scope: identity(command.scope),
                    declaration: command.declaration,
                }
            }
            ContentKind::RetireScope => Self::RetireScope {
                scope: identity(
                    wire::retire_scope_from_value(body, history)
                        .map_err(corrupt)?
                        .scope,
                ),
            },
            ContentKind::MoveToScope => {
                let command = wire::move_to_scope_from_value(body, history).map_err(corrupt)?;
                Self::MoveToScope {
                    actors: command.actors.into_iter().map(identity).collect(),
                    target: identity(command.target),
                }
            }
            ContentKind::UpsertExportMount => {
                let command =
                    wire::upsert_export_mount_from_value(body, history).map_err(corrupt)?;
                Self::UpsertExportMount {
                    mount: identity(command.mount),
                    declaration: command.declaration,
                }
            }
            ContentKind::RetireExportMount => Self::RetireExportMount {
                mount: identity(
                    wire::retire_export_mount_from_value(body, history)
                        .map_err(corrupt)?
                        .mount,
                ),
            },
            ContentKind::UpsertAnnotation => {
                let command = wire::upsert_annotation_from_value(body, history).map_err(corrupt)?;
                Self::UpsertAnnotation {
                    annotation: identity(command.annotation),
                    declaration: command.declaration,
                }
            }
            ContentKind::RetireAnnotation => Self::RetireAnnotation {
                annotation: identity(
                    wire::retire_annotation_from_value(body, history)
                        .map_err(corrupt)?
                        .annotation,
                ),
            },
            ContentKind::SetPresentation => {
                let command = wire::set_presentation_from_value(body, history).map_err(corrupt)?;
                Self::SetPresentation {
                    owner: command.owner.map(identity, identity),
                    presentation: command.presentation,
                }
            }
            ContentKind::SetFlags => {
                let command = wire::set_flags_from_value(body, history).map_err(corrupt)?;
                Self::SetFlags {
                    actor: identity(command.actor),
                    flags: command.flags,
                }
            }
            ContentKind::UpsertTemplate => {
                let command = wire::upsert_template_from_value(body).map_err(corrupt)?;
                Self::UpsertTemplate {
                    name: command.name,
                    commands: command.commands,
                }
            }
            ContentKind::RetireTemplate => Self::RetireTemplate {
                name: wire::retire_template_from_value(body)
                    .map_err(corrupt)?
                    .name,
            },
            ContentKind::ReplaceAuthoringEnvironment => Self::ReplaceAuthoringEnvironment {
                replacement: wire::replace_authoring_environment_from_value(body)
                    .map_err(corrupt)?
                    .replacement,
            },
        })
    }
}

impl ContentVerb {
    pub fn from_snapshot_item(item: &Value) -> Result<Self, CompactedDeclarationRejection> {
        let Value::Object(object) = item else {
            return Err(PayloadRejection::NotAnObject.into());
        };
        let mut fields = object.clone().into_map();
        let spelling = match fields.remove("kind") {
            Some(Value::String(spelling)) => spelling,
            Some(_) => return Err(PayloadRejection::WrongCarrier { key: "kind" }.into()),
            None => return Err(PayloadRejection::MissingKey("kind").into()),
        };
        let Some(kind) = ContentKind::from_str(&spelling) else {
            return Err(CompactedDeclarationRejection::UnknownKind(spelling));
        };
        let body = Value::Object(fields.into());
        let snapshot = AddressContext::AuthoringSnapshot;
        Ok(match kind {
            ContentKind::UpsertTemplate => {
                let command = wire::upsert_template_from_value(body)?;
                Self::UpsertTemplate {
                    name: command.name,
                    commands: command.commands,
                }
            }
            ContentKind::UpsertScope => {
                let command = wire::upsert_scope_from_value(body, snapshot)?;
                Self::UpsertScope {
                    scope: identity(command.scope),
                    declaration: command.declaration,
                }
            }
            ContentKind::UpsertActor => {
                let command = wire::upsert_actor_from_value(body, snapshot)?;
                Self::UpsertActor {
                    actor: identity(command.actor),
                    declaration: command.declaration,
                }
            }
            ContentKind::UpsertEdge => Self::UpsertEdge {
                declaration: wire::upsert_edge_from_value(body, snapshot)?.declaration,
            },
            ContentKind::UpsertExportMount => {
                let command = wire::upsert_export_mount_from_value(body, snapshot)?;
                Self::UpsertExportMount {
                    mount: identity(command.mount),
                    declaration: command.declaration,
                }
            }
            ContentKind::UpsertAnnotation => {
                let command = wire::upsert_annotation_from_value(body, snapshot)?;
                Self::UpsertAnnotation {
                    annotation: identity(command.annotation),
                    declaration: command.declaration,
                }
            }
            ContentKind::SetPresentation => {
                let command = wire::set_presentation_from_value(body, snapshot)?;
                Self::SetPresentation {
                    owner: command.owner.map(identity, identity),
                    presentation: command.presentation,
                }
            }
            ContentKind::RetireActor
            | ContentKind::RetireEdge
            | ContentKind::RetireScope
            | ContentKind::MoveToScope
            | ContentKind::RetireExportMount
            | ContentKind::RetireAnnotation
            | ContentKind::SetFlags
            | ContentKind::RetireTemplate
            | ContentKind::ReplaceAuthoringEnvironment => {
                return Err(CompactedDeclarationRejection::UnknownKind(spelling));
            }
        })
    }
}

fn identity<I>(address: AddressRef<I>) -> I {
    match address {
        AddressRef::Absolute(identity)
        | AddressRef::EpochLocal(identity)
        | AddressRef::Relative(identity) => identity,
    }
}
