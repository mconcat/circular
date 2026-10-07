
use circular_core::Value;

use crate::boundary_port::{BoundaryPortIdError, encode_boundary_actor_key};
use crate::declaration_payload::{PlanActorKey, decode_plan_actor_key};
use crate::wire_value::{WireError, exhausted, object_from_value, optional, take};

pub const ACTOR_HEALTH_TRANSITION_KIND: &str = "actor_health_transition";
/// The arrival arm's own discriminator. Absence is not this arm — it is a refusal.
pub const ACTOR_ARRIVAL_KIND: &str = "actor_arrival";
/// The emission arm's own discriminator: one row per emission its producer recorded in
/// its own column, whether or not a wire leaves the outlet. `actor` is the producer, `port` the
/// outlet, `at` the emission stamp, `index` the producer's own arrival that caused it and
/// `observed_at_ms` that arrival's recorded instant.
pub const ACTOR_EMISSION_KIND: &str = "actor_emission";

circular_core::closed_table! {
    pub enum ApprovalDecisionKind {
        Approved => "approved",
        Denied => "denied",
    }
}

circular_core::closed_table! {
    /// Approved actor-health state vocabulary.
    ///
    /// Producers emit only states supported by an actual observation. In particular, the presence
    /// of `Waiting` and `Backpressure` here does not authorize a daemon to infer them from idleness.
    pub enum ActorHealthState {
        Running => "running",
        Waiting => "waiting",
        Backpressure => "backpressure",
        Failed => "failed",
        Stopped => "stopped",
    }
}

circular_core::closed_table! {
    pub enum ActorHealthReasonCode {
        OutcomeUnclaimed => "outcome_unclaimed",
        DestinationGone => "destination_gone",
        Poisoned => "poisoned",
        Declared => "declared",
        ParameterDenied => "parameter_denied",
        TransportTerminal => "transport_terminal",
        ApprovalRequired => "approval_required",
        Diverged => "diverged",
        EndpointGone => "endpoint_gone",
        InterpreterFault => "interpreter_fault",
        Peer => "peer",
        SourceFailure => "source_failure",
        Capacity => "capacity",
        /// The actor factory refused this activation; other actors keep running.
        ActivationFailed => "activation_failed",
        ActivationWitnessMissing => "activation_witness_missing",
        ActivationRegistrationFailed => "activation_registration_failed",
        RecoveryRefused => "recovery_refused",
        KernelFault => "kernel_fault",
        HarnessUnbound => "harness_unbound",
        HarnessUnusable => "harness_unusable",
    }
}

impl ActorHealthReasonCode {
    #[must_use]
    pub const fn waits(self) -> bool {
        matches!(self, Self::HarnessUnbound | Self::HarnessUnusable)
    }
}

/// A reason code plus detail already carried by its runtime signal.
#[derive(Clone, Debug, PartialEq)]
pub struct ActorHealthReason {
    pub code: ActorHealthReasonCode,
    pub detail: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MailboxDepth {
    pub edge: Value,
    pub depth: u64,
}

/// The discriminated `actor.events` health arm.
#[derive(Clone, Debug, PartialEq)]
pub struct ActorHealthTransition {
    pub actor: PlanActorKey,
    pub state: ActorHealthState,
    pub since_ms: u64,
    pub reason: Option<ActorHealthReason>,
    pub mailbox_depths: Option<Vec<MailboxDepth>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActorHealthTransitionRejection {
    /// The frame's object structure — shared with every object decoder.
    Wire(WireError),
    MissingKind,
    KindIsNotText,
    UnknownKind(String),
    InvalidActor,
    InvalidSince,
    StateIsNotText,
    UnknownState(String),
    ReasonIsNotObject,
    ReasonCodeIsNotText,
    UnknownReason(String),
    MissingReason,
    UnexpectedReason,
    InvalidActorEncoding,
    InvalidMailboxDepths,
}

impl From<WireError> for ActorHealthTransitionRejection {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

impl From<BoundaryPortIdError> for ActorHealthTransitionRejection {
    fn from(_: BoundaryPortIdError) -> Self {
        Self::InvalidActorEncoding
    }
}

impl ActorHealthTransition {
    /// Encode the health arm. The arrival arm's body is the runtime's, but its discriminator is
    /// this module's [`ACTOR_ARRIVAL_KIND`] so the two arms cannot drift into two spellings.
    pub fn to_value(&self) -> Result<Value, ActorHealthTransitionRejection> {
        validate_reason_position(self.state, self.reason.as_ref())?;
        let mut fields = vec![
            (
                "kind",
                Value::String(ACTOR_HEALTH_TRANSITION_KIND.to_owned()),
            ),
            ("actor", encode_boundary_actor_key(&self.actor)?),
            (
                "since_ms",
                Value::Int(
                    i64::try_from(self.since_ms)
                        .map_err(|_| ActorHealthTransitionRejection::InvalidSince)?,
                ),
            ),
            ("state", Value::String(self.state.as_str().to_owned())),
        ];
        if let Some(reason) = &self.reason {
            fields.push((
                "reason",
                Value::object([
                    ("code", Value::String(reason.code.as_str().to_owned())),
                    ("detail", reason.detail.clone()),
                ])
                .expect("two distinct reason fields"),
            ));
        }
        if let Some(rows) = &self.mailbox_depths {
            fields.push((
                "mailbox_depths",
                Value::Array(
                    rows.iter()
                        .map(|row| {
                            Value::object([
                                ("edge", row.edge.clone()),
                                ("depth", Value::UInt(row.depth)),
                            ])
                            .expect("distinct mailbox row fields")
                        })
                        .collect(),
                ),
            ));
        }
        Ok(Value::object(fields).expect("health fields are distinct"))
    }
}

pub fn decode_actor_health_transition(
    value: Value,
) -> Result<Option<ActorHealthTransition>, ActorHealthTransitionRejection> {
    let mut fields = object_from_value(value)?;
    match optional(&mut fields, "kind") {
        None => return Err(ActorHealthTransitionRejection::MissingKind),
        Some(Value::String(kind)) if kind == ACTOR_ARRIVAL_KIND || kind == ACTOR_EMISSION_KIND => {
            return Ok(None);
        }
        Some(Value::String(kind)) if kind == ACTOR_HEALTH_TRANSITION_KIND => {}
        Some(Value::String(kind)) => {
            return Err(ActorHealthTransitionRejection::UnknownKind(kind));
        }
        Some(_) => return Err(ActorHealthTransitionRejection::KindIsNotText),
    }
    let actor = decode_plan_actor_key(take(&mut fields, "actor")?)
        .map_err(|_| ActorHealthTransitionRejection::InvalidActor)?;
    let since_ms = match take(&mut fields, "since_ms")? {
        Value::Int(value) if value >= 0 => {
            u64::try_from(value).map_err(|_| ActorHealthTransitionRejection::InvalidSince)?
        }
        _ => return Err(ActorHealthTransitionRejection::InvalidSince),
    };
    let state = match take(&mut fields, "state")? {
        Value::String(value) => ActorHealthState::from_str(&value)
            .ok_or(ActorHealthTransitionRejection::UnknownState(value))?,
        _ => return Err(ActorHealthTransitionRejection::StateIsNotText),
    };
    let reason = optional(&mut fields, "reason")
        .map(decode_reason)
        .transpose()?;
    let mailbox_depths = optional(&mut fields, "mailbox_depths")
        .map(decode_mailbox_depths)
        .transpose()?;
    exhausted(fields)?;
    validate_reason_position(state, reason.as_ref())?;
    Ok(Some(ActorHealthTransition {
        actor,
        state,
        since_ms,
        reason,
        mailbox_depths,
    }))
}

fn decode_mailbox_depths(
    value: Value,
) -> Result<Vec<MailboxDepth>, ActorHealthTransitionRejection> {
    let Value::Array(rows) = value else {
        return Err(ActorHealthTransitionRejection::InvalidMailboxDepths);
    };
    rows.into_iter()
        .map(|row| {
            if !matches!(row, Value::Object(_)) {
                return Err(ActorHealthTransitionRejection::InvalidMailboxDepths);
            }
            let mut row = object_from_value(row)?;
            let edge = take(&mut row, "edge")?;
            let Value::UInt(depth) = take(&mut row, "depth")? else {
                return Err(ActorHealthTransitionRejection::InvalidMailboxDepths);
            };
            exhausted(row)?;
            Ok(MailboxDepth { edge, depth })
        })
        .collect()
}

fn decode_reason(value: Value) -> Result<ActorHealthReason, ActorHealthTransitionRejection> {
    if !matches!(value, Value::Object(_)) {
        return Err(ActorHealthTransitionRejection::ReasonIsNotObject);
    }
    let mut fields = object_from_value(value)?;
    let code = match take(&mut fields, "code")? {
        Value::String(value) => ActorHealthReasonCode::from_str(&value)
            .ok_or(ActorHealthTransitionRejection::UnknownReason(value))?,
        _ => return Err(ActorHealthTransitionRejection::ReasonCodeIsNotText),
    };
    let detail = take(&mut fields, "detail")?;
    exhausted(fields)?;
    Ok(ActorHealthReason { code, detail })
}

fn validate_reason_position(
    state: ActorHealthState,
    reason: Option<&ActorHealthReason>,
) -> Result<(), ActorHealthTransitionRejection> {
    match (state, reason) {
        (ActorHealthState::Failed | ActorHealthState::Backpressure, None) => {
            Err(ActorHealthTransitionRejection::MissingReason)
        }
        (ActorHealthState::Waiting, Some(reason)) if reason.code.waits() => Ok(()),
        (_, Some(reason)) if reason.code.waits() => {
            Err(ActorHealthTransitionRejection::UnexpectedReason)
        }
        (
            ActorHealthState::Running | ActorHealthState::Waiting | ActorHealthState::Stopped,
            Some(_),
        ) => Err(ActorHealthTransitionRejection::UnexpectedReason),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declaration_payload::AuthoredLocal;

    fn actor() -> PlanActorKey {
        PlanActorKey {
            scope: Vec::new(),
            local: AuthoredLocal::try_new("worker")
                .expect("authored local")
                .into(),
        }
    }

    #[test]
    fn every_health_state_and_reason_has_one_published_spelling() {
        assert_eq!(
            ActorHealthState::ALL.map(ActorHealthState::as_str),
            ["running", "waiting", "backpressure", "failed", "stopped"]
        );
        assert_eq!(
            ActorHealthReasonCode::ALL.map(ActorHealthReasonCode::as_str),
            [
                "outcome_unclaimed",
                "destination_gone",
                "poisoned",
                "declared",
                "parameter_denied",
                "transport_terminal",
                "approval_required",
                "diverged",
                "endpoint_gone",
                "interpreter_fault",
                "peer",
                "source_failure",
                "capacity",
                "activation_failed",
                "activation_witness_missing",
                "activation_registration_failed",
                "recovery_refused",
                "kernel_fault",
                "harness_unbound",
                "harness_unusable",
            ]
        );
    }

    #[test]
    fn health_transition_value_round_trips_through_the_protocol_decoder() {
        let transition = ActorHealthTransition {
            mailbox_depths: None,
            actor: actor(),
            state: ActorHealthState::Failed,
            since_ms: 42,
            reason: Some(ActorHealthReason {
                code: ActorHealthReasonCode::Declared,
                detail: Value::String("chosen_by_actor".to_owned()),
            }),
        };
        let value = transition.to_value().expect("health value encodes");
        assert_eq!(decode_actor_health_transition(value), Ok(Some(transition)));
    }

    #[test]
    fn unknown_reason_and_unknown_state_fail_closed() {
        let replace = |field: &str, value: Value| {
            let transition = ActorHealthTransition {
                mailbox_depths: None,
                actor: actor(),
                state: ActorHealthState::Failed,
                since_ms: 42,
                reason: Some(ActorHealthReason {
                    code: ActorHealthReasonCode::Poisoned,
                    detail: Value::Null,
                }),
            };
            let Value::Object(object) = transition.to_value().expect("health value encodes") else {
                unreachable!()
            };
            let mut fields = object.into_map();
            fields.insert(field.to_owned(), value);
            Value::object(fields).expect("replacement keeps unique fields")
        };

        assert!(matches!(
            decode_actor_health_transition(replace(
                "state",
                Value::String("unknown".to_owned())
            )),
            Err(ActorHealthTransitionRejection::UnknownState(state)) if state == "unknown"
        ));
        let reason = Value::object([
            ("code", Value::String("invented".to_owned())),
            ("detail", Value::Null),
        ])
        .expect("reason fields");
        assert!(matches!(
            decode_actor_health_transition(replace("reason", reason)),
            Err(ActorHealthTransitionRejection::UnknownReason(reason)) if reason == "invented"
        ));
        assert_eq!(
            decode_actor_health_transition(replace("extra", Value::Null)),
            Err(ActorHealthTransitionRejection::Wire(WireError::unknown(
                "extra".to_owned()
            )))
        );
    }
}
