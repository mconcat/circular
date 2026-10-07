//! Physical values for `authoring-snapshot` items.
//!
//! A query item has no declaration envelope verb to identify its arm, so it
//! repeats the existing `DeclarationCommand.kind` discriminant beside the
//! arm's ordinary payload fields.  This is still the shared declaration union:
//! no graph DTO or reconstruction-only command type is introduced.

use circular_core::{Boundary, Ceilings, Value, encode};

use crate::declaration_payload::{
    ActorDeclaration, Anchor, AnnotationDeclaration, BoardPlacement, DeclaredDelay, Delivery,
    EdgeAttrs, EdgeDeclaration, ExportDeclaration, ExportRoles, LayoutPoint, LayoutSize,
    PlanAnnotationKey, PlanExportKey, Presentation, ScopeBinding, ScopeBoundary, ScopeDeclaration,
    ViewSpec, WirePolicy, set_presentation_from_value, upsert_actor_from_value,
    upsert_annotation_from_value, upsert_edge_from_value, upsert_export_mount_from_value,
    upsert_scope_from_value,
};
use crate::declaration_payload::{DeclaredEdgeKey, EdgeKey, edge_identity_value};
use crate::scope_identity::{
    AddressContext, AddressRef, PlanActorKey, ScopeSegment, plan_actor_key_value,
    scope_identity_value as canonical_scope_identity_value,
};
use crate::wire_value::{PayloadRejection, args_arm, arm, unit_arm};

impl crate::DeclarationDomain for AddressContext {
    type Scope = Vec<ScopeSegment>;
    type CommitId = Vec<u8>;
    type ScopeDeclaration = ScopeDeclaration;
    type EpochId = Option<Vec<u8>>;
    type ExpectedRevision = crate::declaration_payload::ExpectedRevision;
    type ActorId = PlanActorKey;
    type ActorDecl = ActorDeclaration;
    type EdgeId = crate::declaration_payload::DeclaredEdgeKey;
    type EdgeAttrs = EdgeDeclaration;
    type ScopeSeg = Vec<ScopeSegment>;
    type ExportName = PlanExportKey;
    type ExportMount = ExportDeclaration;
    type AnnotationId = PlanAnnotationKey;
    type Annotation = AnnotationDeclaration;
    type PresentationOwner = PresentationOwner;
    type Presentation = Presentation<PlanActorKey>;
    type Flags = crate::declaration_payload::ActorFlags;
    type AuthoringEnvironment = crate::declaration_payload::AuthoringEnvironment;
    type TemplateName = String;
    type TemplateCommands = Vec<Value>;
}

use crate::DeclarationCommand;
use crate::declaration_payload::{PresentationOwner, presentation_owner_value};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompactedDeclarationRejection {
    Payload(PayloadRejection),
    UnknownKind(String),
}

impl From<PayloadRejection> for CompactedDeclarationRejection {
    fn from(rejection: PayloadRejection) -> Self {
        Self::Payload(rejection)
    }
}

fn relative_identity<I>(address: AddressRef<I>) -> Result<I, CompactedDeclarationRejection> {
    match address {
        AddressRef::Relative(identity) => Ok(identity),
        AddressRef::Absolute(_) | AddressRef::EpochLocal(_) => {
            Err(PayloadRejection::ArmNotAdmitted.into())
        }
    }
}

pub fn decode_compacted(
    value: &Value,
    context: AddressContext,
) -> Result<DeclarationCommand<AddressContext>, CompactedDeclarationRejection> {
    let Value::Object(object) = value.clone() else {
        return Err(PayloadRejection::NotAnObject.into());
    };
    let mut fields = object.into_map();
    let kind = fields
        .remove("kind")
        .ok_or(PayloadRejection::MissingKey("kind"))?;
    let Value::String(kind) = kind else {
        return Err(PayloadRejection::WrongCarrier { key: "kind" }.into());
    };
    let payload = Value::Object(fields.into());

    match kind.as_str() {
        "UpsertTemplate" => {
            let decoded = crate::declaration_payload::upsert_template_from_value(payload)?;
            Ok(DeclarationCommand::UpsertTemplate {
                epoch: None,
                name: decoded.name,
                commands: decoded.commands,
            })
        }
        "UpsertScope" => {
            let decoded = upsert_scope_from_value(payload, context)?;
            Ok(DeclarationCommand::UpsertScope {
                epoch: None,
                segment: relative_identity(decoded.scope)?,
                declaration: decoded.declaration,
            })
        }
        "UpsertActor" => {
            let decoded = upsert_actor_from_value(payload, context)?;
            Ok(DeclarationCommand::UpsertActor {
                epoch: None,
                id: relative_identity(decoded.actor)?,
                declaration: decoded.declaration,
            })
        }
        "UpsertEdge" => {
            let decoded = upsert_edge_from_value(payload, context)?;
            let id = relative_identity(decoded.edge)?;
            Ok(DeclarationCommand::UpsertEdge {
                epoch: None,
                id,
                attributes: decoded.declaration,
            })
        }
        "UpsertExportMount" => {
            let decoded = upsert_export_mount_from_value(payload, context)?;
            Ok(DeclarationCommand::UpsertExportMount {
                epoch: None,
                name: relative_identity(decoded.mount)?,
                mount: decoded.declaration,
            })
        }
        "UpsertAnnotation" => {
            let decoded = upsert_annotation_from_value(payload, context)?;
            Ok(DeclarationCommand::UpsertAnnotation {
                epoch: None,
                id: relative_identity(decoded.annotation)?,
                annotation: decoded.declaration,
            })
        }
        "SetPresentation" => {
            let decoded = set_presentation_from_value(payload, context)?;
            Ok(DeclarationCommand::SetPresentation {
                epoch: None,
                owner: decoded
                    .owner
                    .try_map(relative_identity, relative_identity)?,
                presentation: decoded.presentation,
            })
        }
        _ => Err(CompactedDeclarationRejection::UnknownKind(kind)),
    }
}

/// Encoder bound to one immutable snapshot root.
pub struct AuthoringSnapshotEncoder<'a> {
    root: &'a [ScopeSegment],
}

impl<'a> AuthoringSnapshotEncoder<'a> {
    #[must_use]
    pub const fn new(root: &'a [ScopeSegment]) -> Self {
        Self { root }
    }

    pub fn upsert_template(&self, name: &str, commands: &[Value]) -> Result<Value, String> {
        command([
            ("kind", Value::string("UpsertTemplate")),
            ("name", Value::string(name)),
            ("commands", Value::Array(commands.to_vec())),
        ])
    }

    pub fn upsert_scope(
        &self,
        scope: &[ScopeSegment],
        declaration: &ScopeDeclaration,
    ) -> Result<Value, String> {
        command([
            ("kind", Value::string("UpsertScope")),
            ("scope", self.scope_address(scope)?),
            ("declaration", scope_declaration(declaration, self.root)?),
        ])
    }

    pub fn upsert_actor(
        &self,
        key: &PlanActorKey,
        declaration: &ActorDeclaration,
    ) -> Result<Value, String> {
        command([
            ("kind", Value::string("UpsertActor")),
            ("actor", self.actor_address(key)?),
            ("declaration", actor_declaration(declaration)?),
        ])
    }

    pub fn upsert_edge(&self, declaration: &EdgeDeclaration) -> Result<Value, String> {
        command([
            ("kind", Value::string("UpsertEdge")),
            ("edge", self.edge_address(declaration)?),
            ("declaration", edge_declaration(declaration, self.root)?),
        ])
    }

    pub fn upsert_export(
        &self,
        key: &PlanExportKey,
        declaration: &ExportDeclaration,
    ) -> Result<Value, String> {
        command([
            ("kind", Value::string("UpsertExportMount")),
            ("mount", self.export_address(key)?),
            ("declaration", export_declaration(declaration, self.root)?),
        ])
    }

    pub fn upsert_annotation(
        &self,
        key: &PlanAnnotationKey,
        declaration: &AnnotationDeclaration,
    ) -> Result<Value, String> {
        command([
            ("kind", Value::string("UpsertAnnotation")),
            ("annotation", self.annotation_address(key)?),
            (
                "declaration",
                annotation_declaration(declaration, self.root)?,
            ),
        ])
    }

    pub fn set_presentation(
        &self,
        owner: &PresentationOwner,
        value: &Presentation<PlanActorKey>,
    ) -> Result<Value, String> {
        command([
            ("kind", Value::string("SetPresentation")),
            (
                "owner",
                presentation_owner_value(
                    owner,
                    |key| self.actor_address(key),
                    |key| self.annotation_address(key),
                )?,
            ),
            ("presentation", presentation(value, self.root)?),
        ])
    }

    fn scope_address(&self, scope: &[ScopeSegment]) -> Result<Value, String> {
        Ok(relative_address(scope_identity_value(relative(
            scope, self.root,
        )?)))
    }

    fn actor_address(&self, key: &PlanActorKey) -> Result<Value, String> {
        Ok(relative_address(actor_key(key, self.root)?))
    }

    fn export_address(&self, key: &PlanExportKey) -> Result<Value, String> {
        Ok(relative_address(plan_key(
            &key.scope, &key.local, self.root,
        )?))
    }

    fn annotation_address(&self, key: &PlanAnnotationKey) -> Result<Value, String> {
        Ok(relative_address(plan_key(
            &key.scope, &key.local, self.root,
        )?))
    }

    fn edge_address(&self, declaration: &EdgeDeclaration) -> Result<Value, String> {
        Ok(relative_address(edge_key(declaration, self.root)?))
    }
}

fn command<const N: usize>(entries: [(&str, Value); N]) -> Result<Value, String> {
    Value::object(entries).map_err(|error| format!("snapshot command object: {error:?}"))
}

fn object(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Result<Value, String> {
    Value::object(entries).map_err(|error| format!("snapshot value object: {error:?}"))
}

fn relative<'a>(
    scope: &'a [ScopeSegment],
    root: &[ScopeSegment],
) -> Result<&'a [ScopeSegment], String> {
    scope
        .strip_prefix(root)
        .ok_or_else(|| "snapshot item lies outside the queried scope".to_owned())
}

fn relative_address(identity: Value) -> Value {
    arm(3, identity)
}

#[must_use]
pub fn scope_identity_value(scope: &[ScopeSegment]) -> Value {
    canonical_scope_identity_value(scope)
}

#[must_use]
pub fn current_revision_value(revision: Option<&[u8]>) -> Value {
    match revision {
        None => unit_arm(1),
        Some(bytes) => arm(2, Value::Bytes(bytes.to_vec())),
    }
}

pub fn environment_value(
    environment: &crate::declaration_payload::AuthoringEnvironment,
) -> Result<Value, String> {
    object([
        (
            "declaration_schema",
            Value::Bytes(environment.declaration_schema.clone()),
        ),
        ("spec_set", Value::Bytes(environment.spec_set.clone())),
    ])
}

fn relative_actor_key(key: &PlanActorKey, root: &[ScopeSegment]) -> Result<PlanActorKey, String> {
    Ok(PlanActorKey {
        scope: relative(&key.scope, root)?.to_vec(),
        local: key.local.clone(),
    })
}

fn plan_key(scope: &[ScopeSegment], local: &str, root: &[ScopeSegment]) -> Result<Value, String> {
    Ok(plan_actor_key_value(&PlanActorKey {
        scope: relative(scope, root)?.to_vec(),
        local: crate::scope_identity::ActorLocal::parse(local),
    }))
}

fn actor_key(key: &PlanActorKey, root: &[ScopeSegment]) -> Result<Value, String> {
    Ok(plan_actor_key_value(&relative_actor_key(key, root)?))
}

fn endpoint(endpoint: &(PlanActorKey, String), root: &[ScopeSegment]) -> Result<Value, String> {
    Ok(Value::array([
        actor_key(&endpoint.0, root)?,
        Value::String(endpoint.1.clone()),
    ]))
}

fn edge_key(declaration: &EdgeDeclaration, root: &[ScopeSegment]) -> Result<Value, String> {
    Ok(edge_identity_value(&EdgeKey::Declared(DeclaredEdgeKey {
        from: (
            relative_actor_key(&declaration.from.0, root)?,
            declaration.from.1.clone(),
        ),
        to: (
            relative_actor_key(&declaration.to.0, root)?,
            declaration.to.1.clone(),
        ),
        ordinal: declaration.ordinal,
    })))
}

fn actor_declaration(declaration: &ActorDeclaration) -> Result<Value, String> {
    object([
        (
            "domain",
            object([
                ("config", declaration.config.clone()),
                ("actor_type", Value::String(declaration.actor_type.clone())),
            ])?,
        ),
        (
            "flags",
            object([
                ("bypass", Value::Bool(declaration.flags.bypass)),
                ("mute", Value::Bool(declaration.flags.mute)),
                ("pause", Value::Bool(declaration.flags.pause)),
            ])?,
        ),
    ])
}

fn edge_declaration(declaration: &EdgeDeclaration, root: &[ScopeSegment]) -> Result<Value, String> {
    object([
        ("attrs", edge_attrs(&declaration.attrs)?),
        ("from", endpoint(&declaration.from, root)?),
        ("ordinal", Value::Int(i64::from(declaration.ordinal))),
        ("to", endpoint(&declaration.to, root)?),
    ])
}

fn edge_attrs(attrs: &EdgeAttrs) -> Result<Value, String> {
    let mut fields = vec![
        ("delay", delay(&attrs.delay)?),
        ("policy", wire_policy(&attrs.policy)?),
    ];
    if !attrs.preprocess.is_empty() {
        let steps = attrs
            .preprocess
            .steps()
            .iter()
            .map(|step| {
                object([
                    ("kind", Value::String(step.kind.as_str().to_owned())),
                    (
                        "config",
                        step.config
                            .to_wire_value()
                            .map_err(|error| error.to_string())?,
                    ),
                ])
            })
            .collect::<Result<Vec<_>, String>>()?;
        fields.push(("preprocess", Value::Array(steps)));
    }
    object(fields)
}

fn delay(delay: &DeclaredDelay) -> Result<Value, String> {
    object([
        ("den", bounded_int(delay.denominator(), "delay.den")?),
        ("num", bounded_int(delay.numerator(), "delay.num")?),
    ])
}

fn wire_policy(policy: &WirePolicy) -> Result<Value, String> {
    let mut entries = Vec::new();
    if let Some(capacity) = policy.capacity {
        entries.push((
            "capacity",
            bounded_int(
                u64::try_from(capacity.get()).map_err(|_| "policy.capacity".to_owned())?,
                "policy.capacity",
            )?,
        ));
    }
    entries.push((
        "delivery",
        match policy.delivery {
            Delivery::BestEffort { on_full } => arm(1, unit_arm(on_full.tag())),
            Delivery::Lossless => unit_arm(2),
            Delivery::Durable => unit_arm(3),
        },
    ));
    object(entries)
}

fn scope_declaration(
    declaration: &ScopeDeclaration,
    root: &[ScopeSegment],
) -> Result<Value, String> {
    object([
        ("boundary", scope_boundary(&declaration.boundary, root)?),
        ("role", unit_arm(declaration.role.tag())),
    ])
}

fn scope_boundary(boundary: &ScopeBoundary, root: &[ScopeSegment]) -> Result<Value, String> {
    object([
        ("inlets", scope_bindings(&boundary.inlets, root)?),
        ("outlets", scope_bindings(&boundary.outlets, root)?),
    ])
}

fn scope_bindings(bindings: &[ScopeBinding], root: &[ScopeSegment]) -> Result<Value, String> {
    let mut ordered = bindings.to_vec();
    ordered.sort_by(|left, right| left.outer.as_bytes().cmp(right.outer.as_bytes()));
    ordered.windows(2).try_for_each(|pair| {
        (pair[0].outer != pair[1].outer)
            .then_some(())
            .ok_or_else(|| "scope boundary repeats an outer name".to_owned())
    })?;
    Ok(Value::array(
        ordered
            .iter()
            .map(|binding| {
                object([
                    ("inner", endpoint(&binding.inner, root)?),
                    ("outer", Value::String(binding.outer.clone())),
                ])
            })
            .collect::<Result<Vec<_>, String>>()?,
    ))
}

fn export_declaration(
    declaration: &ExportDeclaration,
    root: &[ScopeSegment],
) -> Result<Value, String> {
    let mut entries = vec![("roles", export_roles(&declaration.roles, root)?)];
    if let Some(operations) = &declaration.operations {
        entries.push(("operations", operations.clone()));
    }
    if let Some(surface) = &declaration.surface {
        entries.push(("surface", surface.clone()));
    }
    object(entries)
}

fn export_roles(roles: &ExportRoles, root: &[ScopeSegment]) -> Result<Value, String> {
    let mut entries = Vec::new();
    if let Some(value) = &roles.error {
        entries.push(("error", endpoint(value, root)?));
    }
    if let Some(value) = &roles.progress {
        entries.push(("progress", endpoint(value, root)?));
    }
    if let Some(value) = &roles.request {
        entries.push(("request", endpoint(value, root)?));
    }
    if let Some(value) = &roles.result {
        entries.push(("result", endpoint(value, root)?));
    }
    object(entries)
}

fn annotation_declaration(
    declaration: &AnnotationDeclaration,
    root: &[ScopeSegment],
) -> Result<Value, String> {
    let mut refs = declaration
        .refs
        .iter()
        .map(|key| actor_key(key, root))
        .collect::<Result<Vec<_>, _>>()?;
    let mut encoded_refs = refs
        .drain(..)
        .map(|value| {
            encode(&value, Ceilings::for_boundary(Boundary::Wire))
                .map(|encoded| (encoded, value))
                .map_err(|error| format!("snapshot annotation ref does not encode: {error:?}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    encoded_refs.sort_by(|left, right| left.0.cmp(&right.0));
    refs.extend(encoded_refs.into_iter().map(|(_, value)| value));
    object([
        ("body", Value::String(declaration.body.clone())),
        ("kind", unit_arm(declaration.kind.tag())),
        ("refs", Value::Array(refs)),
    ])
}

fn presentation(
    value: &Presentation<PlanActorKey>,
    root: &[ScopeSegment],
) -> Result<Value, String> {
    let mut entries = Vec::new();
    if let Some(anchor) = &value.anchor {
        entries.push(("anchor", anchor_value(anchor, root)?));
    }
    if let Some(board) = value.board {
        entries.push(("board", board_cell(board)?));
    }
    entries.push(("collapsed", Value::Bool(value.collapsed)));
    if let Some(fixed) = value.fixed {
        entries.push(("fixed", layout_point(fixed)?));
    }
    if let Some(group) = &value.group {
        entries.push(("group", Value::String(group.as_str().to_owned())));
    }
    if let Some(label) = &value.label {
        entries.push(("label", Value::String(label.clone())));
    }
    if let Some(size) = value.size {
        entries.push(("size", layout_size(size)?));
    }
    if let Some(view) = &value.view {
        entries.push(("view", view_spec(view)?));
    }
    object(entries)
}

fn anchor_value(anchor: &Anchor<PlanActorKey>, root: &[ScopeSegment]) -> Result<Value, String> {
    Ok(match anchor {
        Anchor::Flow => unit_arm(1),
        Anchor::Relative { target, relation } => {
            args_arm(2, [actor_key(target, root)?, unit_arm(relation.tag())])
        }
        Anchor::Align { target, axis } => {
            args_arm(3, [actor_key(target, root)?, unit_arm(axis.tag())])
        }
    })
}

fn board_cell(board: BoardPlacement) -> Result<Value, String> {
    object([
        ("col", Value::Int(i64::from(board.col()))),
        ("h", Value::Int(i64::from(board.h()))),
        ("row", Value::Int(i64::from(board.row()))),
        ("w", Value::Int(i64::from(board.w()))),
    ])
}

fn layout_point(point: LayoutPoint) -> Result<Value, String> {
    object([
        ("x", Value::Int(i64::from(point.x.get()))),
        ("y", Value::Int(i64::from(point.y.get()))),
    ])
}

fn layout_size(size: LayoutSize) -> Result<Value, String> {
    object([
        ("h", bounded_int(size.h, "size.h")?),
        ("w", bounded_int(size.w, "size.w")?),
    ])
}

fn view_spec(view: &ViewSpec) -> Result<Value, String> {
    object([
        ("config", view.config.clone()),
        ("kind", Value::String(view.kind.clone())),
    ])
}

fn bounded_int(value: u64, label: &str) -> Result<Value, String> {
    i64::try_from(value)
        .map(Value::Int)
        .map_err(|_| format!("{label} exceeds the published Int carrier"))
}

