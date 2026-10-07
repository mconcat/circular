
use circular_core::{
    BaseShape, Boundary, BuiltinObservationName, Ceilings, MAX_REASSEMBLED_BODY_BYTES,
    MAX_SEGMENT_BODY_BYTES,
};
use circular_expr::EvalMode;
use circular_protocol::actor_events::{
    ActorHealthReasonCode, ActorHealthState, ApprovalDecisionKind,
};
use circular_protocol::dead_letter::DeadLetterReasonKind;
use circular_protocol::declaration_payload::PreprocessKind;
use circular_protocol::lifecycle::LifecycleWord;
use circular_protocol::rejection_code::RejectionReason;
use circular_protocol::subscription_payload::{
    FrameOrigin, SubscriptionEndReasonArm, SubscriptionFrameArm,
};
use circular_protocol::timeline::TimelineMarkKind;
use circular_protocol::{
    DeclarationVerb, KindRegistration, Partition, RESERVED_CAPABILITY_PARTITION_TAG,
    RESERVED_CAPABILITY_VERB_TAG_COUNT, RESERVED_CAPABILITY_VERB_TAG_FIRST, StableVerb,
    partition_tag, verb_tag,
};
use serde_json::{Map, Value, json};

pub const MODULE: &str = "sdk/typescript/packages/protocol/src/tables.generated.js";
pub const TYPES: &str = "sdk/typescript/packages/protocol/src/tables.generated.d.ts";

pub const REGENERATE: &str = "cargo run --locked -p engine --example closed_tables";

fn named(name: impl std::fmt::Debug, tag: impl Into<Value>) -> Value {
    json!({ "name": format!("{name:?}"), "tag": tag.into() })
}

fn ceilings(boundary: Boundary) -> Value {
    let ceilings = Ceilings::for_boundary(boundary);
    json!({
        "max_bytes": ceilings.max_bytes(),
        "max_depth": ceilings.max_depth(),
        "max_container_entries": ceilings.max_container_entries(),
        "max_string_bytes": ceilings.max_string_bytes(),
    })
}

fn verb_name(verb: StableVerb) -> String {
    match verb {
        StableVerb::SessionMechanics(verb) => format!("{verb:?}"),
        StableVerb::Declaration(verb) => format!("{verb:?}"),
        StableVerb::Query(verb) => format!("{verb:?}"),
        StableVerb::Subscription(verb) => format!("{verb:?}"),
        StableVerb::EventInjection(verb) => format!("{verb:?}"),
        StableVerb::LedgerTransition(verb) => format!("{verb:?}"),
        StableVerb::ReplayControl(verb) => format!("{verb:?}"),
        StableVerb::Lifecycle(verb) => format!("{verb:?}"),
    }
}

fn tables() -> Vec<(&'static str, Value)> {
    vec![
        (
            "Partition",
            Partition::ALL
                .into_iter()
                .map(|partition| named(partition, partition_tag(partition)))
                .collect(),
        ),
        (
            "RESERVED_CAPABILITY_PARTITION_TAG",
            json!(RESERVED_CAPABILITY_PARTITION_TAG),
        ),
        (
            "StableVerb",
            StableVerb::all()
                .map(|verb| {
                    json!({
                        "partition": format!("{:?}", verb.partition()),
                        "name": verb_name(verb),
                        "tag": verb_tag(verb),
                        "body": format!("{:?}", verb.body()),
                    })
                })
                .collect(),
        ),
        (
            "RESERVED_CAPABILITY_VERB_TAG_FIRST",
            json!(RESERVED_CAPABILITY_VERB_TAG_FIRST),
        ),
        (
            "RESERVED_CAPABILITY_VERB_TAG_COUNT",
            json!(RESERVED_CAPABILITY_VERB_TAG_COUNT),
        ),
        (
            "DeclarationCommand",
            DeclarationVerb::ALL
                .into_iter()
                .filter(|verb| KindRegistration::<(), (), ()>::declaration(*verb, (), ()).is_ok())
                .map(|verb| json!(format!("{verb:?}")))
                .collect(),
        ),
        (
            "RejectionReason",
            RejectionReason::ALL
                .into_iter()
                .map(|reason| {
                    json!({
                        "name": format!("{reason:?}"),
                        "numbers": [
                            reason.code_in(Partition::Declaration),
                            reason.code_in(Partition::Lifecycle),
                        ],
                    })
                })
                .collect(),
        ),
        (
            "LifecycleWord",
            LifecycleWord::ALL
                .into_iter()
                .map(|word| json!({ "as_str": word.as_str(), "pipeline_stands": word.pipeline_stands() }))
                .collect(),
        ),
        (
            "BuiltinObservationName",
            BuiltinObservationName::ALL
                .into_iter()
                .map(|name| json!({ "name": format!("{name:?}"), "tag": name.tag(), "as_str": name.as_str() }))
                .collect(),
        ),
        (
            "ActorHealthState",
            json!(ActorHealthState::ALL.map(ActorHealthState::as_str).to_vec()),
        ),
        (
            "ActorHealthReasonCode",
            json!(
                ActorHealthReasonCode::ALL
                    .map(ActorHealthReasonCode::as_str)
                    .to_vec()
            ),
        ),
        (
            "ApprovalDecisionKind",
            json!(
                ApprovalDecisionKind::ALL
                    .map(ApprovalDecisionKind::as_str)
                    .to_vec()
            ),
        ),
        (
            "PreprocessKind",
            json!(PreprocessKind::ALL.map(PreprocessKind::as_str).to_vec()),
        ),
        (
            "DeadLetterReasonKind",
            json!(
                DeadLetterReasonKind::ALL
                    .map(DeadLetterReasonKind::as_str)
                    .to_vec()
            ),
        ),
        (
            "LifecyclePhase",
            crate::incarnation_transition::LifecyclePhase::ALL
                .into_iter()
                .map(|phase| named(phase, phase.tag()))
                .collect(),
        ),
        (
            "SubscriptionFrame",
            SubscriptionFrameArm::ALL
                .into_iter()
                .map(|arm| named(arm, arm.tag()))
                .collect(),
        ),
        (
            "FrameOrigin",
            FrameOrigin::ALL
                .into_iter()
                .map(|origin| named(origin, origin.tag()))
                .collect(),
        ),
        (
            "SubscriptionEndReason",
            SubscriptionEndReasonArm::ALL
                .into_iter()
                .map(|arm| named(arm, arm.tag()))
                .collect(),
        ),
        (
            "Ceilings",
            json!({
                "Wire": ceilings(Boundary::Wire),
                "Identity": ceilings(Boundary::Identity),
            }),
        ),
        ("MAX_SEGMENT_BODY_BYTES", json!(MAX_SEGMENT_BODY_BYTES)),
        (
            "MAX_REASSEMBLED_BODY_BYTES",
            json!(MAX_REASSEMBLED_BODY_BYTES),
        ),
        (
            "QueryId",
            Value::Object(
                crate::daemon::query_wire_rows()
                    .map(|(id, name, paging)| (id, json!({ "name": name, "paging": paging })))
                    .collect::<Map<_, _>>(),
            ),
        ),
        (
            "SubscriptionTarget",
            Value::Object(
                crate::daemon::subscription_wire_rows()
                    .map(|(id, name, delivery)| (id, json!({ "name": name, "delivery": delivery })))
                    .collect::<Map<_, _>>(),
            ),
        ),
        (
            "TimelineMarkKind",
            json!(TimelineMarkKind::ALL.map(TimelineMarkKind::as_str).to_vec()),
        ),
        (
            "BaseShape",
            json!(BaseShape::ALL.map(BaseShape::as_str).to_vec()),
        ),
        (
            "EvalMode",
            json!(EvalMode::ALL.map(EvalMode::as_str).to_vec()),
        ),
    ]
}

const HEADER: &str = "/*! Generated from the Rust declarations by crates/engine/src/closed_tables.rs. Do not edit.\n";

#[must_use]
pub fn module() -> String {
    let mut out = format!("{HEADER} * Regenerate: {REGENERATE} */\n");
    for (name, value) in tables() {
        let body = serde_json::to_string_pretty(&value).expect("the tables are JSON");
        out.push_str(&format!("export const {name} = {body};\n"));
    }
    out
}

#[must_use]
pub fn types() -> String {
    let mut out = format!("{HEADER} * Regenerate: {REGENERATE} */\n");
    for (name, value) in tables() {
        let mut ty = String::new();
        literal_type(&value, 0, &mut ty);
        out.push_str(&format!("export declare const {name}: {ty};\n"));
    }
    out
}

fn literal_type(value: &Value, depth: usize, out: &mut String) {
    let indent = |depth: usize| "  ".repeat(depth);
    match value {
        Value::Array(items) => {
            out.push_str("readonly [\n");
            for item in items {
                out.push_str(&indent(depth + 1));
                literal_type(item, depth + 1, out);
                out.push_str(",\n");
            }
            out.push_str(&indent(depth));
            out.push(']');
        }
        Value::Object(fields) => {
            out.push_str("{\n");
            for (key, field) in fields {
                out.push_str(&indent(depth + 1));
                out.push_str("readonly ");
                out.push_str(&Value::String(key.clone()).to_string());
                out.push_str(": ");
                literal_type(field, depth + 1, out);
                out.push_str(";\n");
            }
            out.push_str(&indent(depth));
            out.push('}');
        }
        scalar => out.push_str(&scalar.to_string()),
    }
}

