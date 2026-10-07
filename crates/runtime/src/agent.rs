
use std::error::Error;
use std::fmt;

use crate::{EffectFailure, ProcessResult};

#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AgentPayload(Box<[u8]>);

impl AgentPayload {
    #[must_use]
    pub fn new(bytes: impl Into<Box<[u8]>>) -> Self {
        Self(bytes.into())
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    #[must_use]
    pub fn into_bytes(self) -> Box<[u8]> {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmptyAgentName;

impl fmt::Display for EmptyAgentName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("agent harness and tool names must not be empty")
    }
}

impl Error for EmptyAgentName {}

macro_rules! normalized_agent_name {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(Box<str>);

        impl $name {
            pub fn try_from_normalized(value: impl Into<Box<str>>) -> Result<Self, EmptyAgentName> {
                let value = value.into();
                if value.is_empty() {
                    Err(EmptyAgentName)
                } else {
                    Ok(Self(value))
                }
            }

            #[must_use]
            pub const fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }
    };
}

normalized_agent_name!(
    AgentHarnessName,
    "Canonical name that points at an agent harness adapter in the deployment environment."
);
normalized_agent_name!(
    ToolName,
    "Canonical name that a Tool declaration and a call match by exact equality."
);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AgentSessionId {
    harness: AgentHarnessName,
    opaque: Box<[u8]>,
}

impl AgentSessionId {
    #[must_use]
    pub fn new(harness: AgentHarnessName, opaque: impl Into<Box<[u8]>>) -> Self {
        Self {
            harness,
            opaque: opaque.into(),
        }
    }

    #[must_use]
    pub const fn harness(&self) -> &AgentHarnessName {
        &self.harness
    }

    #[must_use]
    pub const fn opaque(&self) -> &[u8] {
        &self.opaque
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EmptyAgentToolCallId;

impl fmt::Display for EmptyAgentToolCallId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("agent tool call id must not be empty")
    }
}

impl Error for EmptyAgentToolCallId {}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AgentToolCallId(Box<[u8]>);

impl AgentToolCallId {
    pub fn try_from_bytes(bytes: impl Into<Box<[u8]>>) -> Result<Self, EmptyAgentToolCallId> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            Err(EmptyAgentToolCallId)
        } else {
            Ok(Self(bytes))
        }
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentToolCall {
    id: AgentToolCallId,
    tool: ToolName,
    arguments: AgentPayload,
}

impl AgentToolCall {
    #[must_use]
    pub const fn new(id: AgentToolCallId, tool: ToolName, arguments: AgentPayload) -> Self {
        Self {
            id,
            tool,
            arguments,
        }
    }

    #[must_use]
    pub const fn id(&self) -> &AgentToolCallId {
        &self.id
    }

    #[must_use]
    pub const fn tool(&self) -> &ToolName {
        &self.tool
    }

    #[must_use]
    pub const fn arguments(&self) -> &AgentPayload {
        &self.arguments
    }
}

circular_core::closed_table! {
    #[derive(Ord, PartialOrd)]
    pub enum ConcreteExternalEffectTag: u8 {
        FileRead = 1 => "file_read",
        FileWrite = 2 => "file_write",
        Spawn = 3 => "spawn",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConcreteOutcomePayload {
    FileBytes(Box<[u8]>),
    WrittenLength(u64),
    ProcessResult(ProcessResult),
}

impl ConcreteOutcomePayload {
    #[must_use]
    pub const fn effect(&self) -> ConcreteExternalEffectTag {
        match self {
            Self::FileBytes(_) => ConcreteExternalEffectTag::FileRead,
            Self::WrittenLength(_) => ConcreteExternalEffectTag::FileWrite,
            Self::ProcessResult(_) => ConcreteExternalEffectTag::Spawn,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConcreteOutcomeMismatch;

impl fmt::Display for ConcreteOutcomeMismatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("concrete outcome payload does not match its effect tag")
    }
}

impl Error for ConcreteOutcomeMismatch {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentToolResult {
    call: AgentToolCallId,
    effect: ConcreteExternalEffectTag,
    result: Result<ConcreteOutcomePayload, EffectFailure>,
}

impl AgentToolResult {
    /// Narrow a recorded interpreter result at the tool boundary. Failures keep
    /// their original effect identity; a successful payload must match that
    /// effect. The caller decides how a mismatch changes its actor state.
    pub fn from_outcome<S>(
        call: AgentToolCallId,
        effect: ConcreteExternalEffectTag,
        result: &Result<crate::OutcomePayload<S>, EffectFailure>,
    ) -> Result<Self, ConcreteOutcomeMismatch> {
        let payload = match result {
            Ok(crate::OutcomePayload::FileBytes(bytes)) => {
                ConcreteOutcomePayload::FileBytes(bytes.clone())
            }
            Ok(crate::OutcomePayload::WrittenLength(length)) => {
                ConcreteOutcomePayload::WrittenLength(*length)
            }
            Ok(crate::OutcomePayload::ProcessResult(process)) => {
                ConcreteOutcomePayload::ProcessResult(process.clone())
            }
            Err(failure) => return Ok(Self::failed(call, effect, failure.clone())),
            _ => return Err(ConcreteOutcomeMismatch),
        };
        Self::succeeded(call, effect, payload)
    }

    pub fn succeeded(
        call: AgentToolCallId,
        effect: ConcreteExternalEffectTag,
        payload: ConcreteOutcomePayload,
    ) -> Result<Self, ConcreteOutcomeMismatch> {
        if payload.effect() != effect {
            return Err(ConcreteOutcomeMismatch);
        }
        Ok(Self {
            call,
            effect,
            result: Ok(payload),
        })
    }

    #[must_use]
    pub const fn failed(
        call: AgentToolCallId,
        effect: ConcreteExternalEffectTag,
        failure: EffectFailure,
    ) -> Self {
        Self {
            call,
            effect,
            result: Err(failure),
        }
    }

    #[must_use]
    pub const fn call(&self) -> &AgentToolCallId {
        &self.call
    }

    #[must_use]
    pub const fn effect(&self) -> ConcreteExternalEffectTag {
        self.effect
    }

    pub const fn result(&self) -> &Result<ConcreteOutcomePayload, EffectFailure> {
        &self.result
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentStepRequest {
    UserTurn(AgentPayload),
    ToolResult {
        call: AgentToolCallId,
        result: AgentToolResult,
    },
}

impl AgentStepRequest {
    #[must_use]
    pub const fn user_turn(payload: AgentPayload) -> Self {
        Self::UserTurn(payload)
    }

    pub fn tool_result(
        call: AgentToolCallId,
        result: AgentToolResult,
    ) -> Result<Self, AgentStepRequestError> {
        let request = Self::ToolResult { call, result };
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), AgentStepRequestError> {
        match self {
            Self::UserTurn(_) => Ok(()),
            Self::ToolResult { call, result } if call == result.call() => Ok(()),
            Self::ToolResult { .. } => Err(AgentStepRequestError::ToolCallMismatch),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentStepRequestError {
    ToolCallMismatch,
}

impl fmt::Display for AgentStepRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("agent tool result must carry the requested call id")
    }
}

impl Error for AgentStepRequestError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentInvokeSpec {
    harness: AgentHarnessName,
    session: Option<AgentSessionId>,
    request: AgentStepRequest,
}

impl AgentInvokeSpec {
    pub fn try_new(
        harness: AgentHarnessName,
        session: Option<AgentSessionId>,
        request: AgentStepRequest,
    ) -> Result<Self, AgentInvokeSpecError> {
        request
            .validate()
            .map_err(|_| AgentInvokeSpecError::ToolCallMismatch)?;
        if session
            .as_ref()
            .is_some_and(|session| session.harness() != &harness)
        {
            return Err(AgentInvokeSpecError::SessionHarnessMismatch);
        }
        Ok(Self {
            harness,
            session,
            request,
        })
    }

    #[must_use]
    pub const fn harness(&self) -> &AgentHarnessName {
        &self.harness
    }

    #[must_use]
    pub const fn session(&self) -> Option<&AgentSessionId> {
        self.session.as_ref()
    }

    #[must_use]
    pub const fn request(&self) -> &AgentStepRequest {
        &self.request
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentInvokeSpecError {
    SessionHarnessMismatch,
    ToolCallMismatch,
}

impl fmt::Display for AgentInvokeSpecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SessionHarnessMismatch => {
                formatter.write_str("agent session belongs to a different harness")
            }
            Self::ToolCallMismatch => {
                formatter.write_str("agent tool result must carry the requested call id")
            }
        }
    }
}

impl Error for AgentInvokeSpecError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentProgressRecord(AgentPayload);

impl AgentProgressRecord {
    #[must_use]
    pub const fn new(payload: AgentPayload) -> Self {
        Self(payload)
    }

    #[must_use]
    pub const fn payload(&self) -> &AgentPayload {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentStepNext {
    Final {
        output: AgentPayload,
        metadata: AgentPayload,
    },
    ToolRequest {
        call: AgentToolCall,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentStepResult {
    session: AgentSessionId,
    progress: Box<[AgentProgressRecord]>,
    next: AgentStepNext,
}

impl AgentStepResult {
    pub fn try_new(
        expected_harness: &AgentHarnessName,
        session: AgentSessionId,
        progress: impl Into<Box<[AgentProgressRecord]>>,
        next: AgentStepNext,
    ) -> Result<Self, AgentStepResultError> {
        if session.harness() != expected_harness {
            return Err(AgentStepResultError::SessionHarnessMismatch);
        }
        Ok(Self {
            session,
            progress: progress.into(),
            next,
        })
    }

    #[must_use]
    pub const fn session(&self) -> &AgentSessionId {
        &self.session
    }

    #[must_use]
    pub const fn progress(&self) -> &[AgentProgressRecord] {
        &self.progress
    }

    #[must_use]
    pub const fn next(&self) -> &AgentStepNext {
        &self.next
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentStepResultError {
    SessionHarnessMismatch,
}

impl fmt::Display for AgentStepResultError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("agent result session belongs to a different harness")
    }
}

impl Error for AgentStepResultError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AgentHarnessGrant;

    fn harness(value: &str) -> AgentHarnessName {
        AgentHarnessName::try_from_normalized(value).expect("nonempty harness")
    }

    fn call(value: u8) -> AgentToolCallId {
        AgentToolCallId::try_from_bytes([value]).expect("nonempty call id")
    }

    #[test]
    fn names_and_tool_call_ids_reject_their_empty_forms() {
        assert_eq!(
            AgentHarnessName::try_from_normalized(""),
            Err(EmptyAgentName)
        );
        assert_eq!(ToolName::try_from_normalized(""), Err(EmptyAgentName));
        assert_eq!(
            AgentToolCallId::try_from_bytes(Box::<[u8]>::default()),
            Err(EmptyAgentToolCallId)
        );
    }

    #[test]
    fn harness_grant_uses_a_finite_exact_name_set() {
        let allowed = harness("pi");
        let similarly_named = harness("pi-preview");
        let grant = AgentHarnessGrant::agent_harness([allowed.clone()]);

        assert!(grant.parameters().allows(&allowed));
        assert!(!grant.parameters().allows(&similarly_named));
        assert_eq!(grant.parameters().iter().count(), 1);
    }

    #[test]
    fn invoke_and_outcome_keep_the_harness_namespace() {
        let primary = harness("primary");
        let fallback = harness("fallback");
        let foreign = AgentSessionId::new(fallback.clone(), [1]);
        assert_eq!(
            AgentInvokeSpec::try_new(
                primary.clone(),
                Some(foreign.clone()),
                AgentStepRequest::user_turn(AgentPayload::new([2])),
            ),
            Err(AgentInvokeSpecError::SessionHarnessMismatch)
        );
        assert_eq!(
            AgentStepResult::try_new(
                &primary,
                foreign,
                Box::<[AgentProgressRecord]>::default(),
                AgentStepNext::Final {
                    output: AgentPayload::new([3]),
                    metadata: AgentPayload::default(),
                },
            ),
            Err(AgentStepResultError::SessionHarnessMismatch)
        );
    }

    #[test]
    fn tool_result_keeps_call_identity_and_effect_payload_pair() {
        let first = call(1);
        let second = call(2);
        assert_eq!(
            AgentToolResult::succeeded(
                first.clone(),
                ConcreteExternalEffectTag::FileRead,
                ConcreteOutcomePayload::WrittenLength(4),
            ),
            Err(ConcreteOutcomeMismatch)
        );

        let result = AgentToolResult::succeeded(
            first,
            ConcreteExternalEffectTag::FileWrite,
            ConcreteOutcomePayload::WrittenLength(4),
        )
        .expect("matching effect and payload");
        assert_eq!(
            AgentStepRequest::tool_result(second, result),
            Err(AgentStepRequestError::ToolCallMismatch)
        );
    }

    #[test]
    fn recorded_tool_outcomes_narrow_only_the_matching_concrete_payload() {
        use crate::OutcomePayload;
        let cases: [(
            ConcreteExternalEffectTag,
            OutcomePayload,
            ConcreteOutcomePayload,
        ); 3] = [
            (
                ConcreteExternalEffectTag::FileRead,
                OutcomePayload::FileBytes(Box::from(&b"read"[..])),
                ConcreteOutcomePayload::FileBytes(Box::from(&b"read"[..])),
            ),
            (
                ConcreteExternalEffectTag::FileWrite,
                OutcomePayload::WrittenLength(17),
                ConcreteOutcomePayload::WrittenLength(17),
            ),
            (
                ConcreteExternalEffectTag::Spawn,
                OutcomePayload::ProcessResult(ProcessResult::direct(2, [3], [4])),
                ConcreteOutcomePayload::ProcessResult(ProcessResult::direct(2, [3], [4])),
            ),
        ];
        for (effect, _, _) in &cases {
            for (payload_effect, payload, expected) in &cases {
                let result = AgentToolResult::from_outcome(call(9), *effect, &Ok(payload.clone()));
                if effect == payload_effect {
                    let result = result.unwrap();
                    assert_eq!(result.call(), &call(9));
                    assert_eq!(result.effect(), *effect);
                    assert_eq!(result.result(), &Ok(expected.clone()));
                } else {
                    assert_eq!(result, Err(ConcreteOutcomeMismatch));
                }
            }
            assert_eq!(
                AgentToolResult::from_outcome::<()>(
                    call(9),
                    *effect,
                    &Ok(OutcomePayload::ScheduleArmed(
                        crate::ScheduleCorrelation::new(1)
                    )),
                ),
                Err(ConcreteOutcomeMismatch),
            );
            let failed = AgentToolResult::from_outcome::<()>(
                call(9),
                *effect,
                &Err(EffectFailure::EndpointGone),
            )
            .unwrap();
            assert_eq!(failed.call(), &call(9));
            assert_eq!(failed.effect(), *effect);
            assert_eq!(failed.result(), &Err(EffectFailure::EndpointGone));
        }
    }
}
