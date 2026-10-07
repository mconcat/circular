
use circular_runtime::{
    AgentPayload, AgentProgressRecord, AgentSessionId, AgentStepNext, AgentStepRequest,
    AgentStepResult, AgentToolCall, AgentToolCallId, Effect, EffectFailure, EffectOutcome,
    Interpreter, InterpreterFault, OutcomePayload, SubmitError, ToolName,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Debug, Default)]
pub struct ReferenceAgentContacts(Arc<AtomicU64>);

impl ReferenceAgentContacts {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn count(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }

    fn record(&self) -> u64 {
        self.0.fetch_add(1, Ordering::SeqCst) + 1
    }
}

pub struct ReferenceAgentExecutor {
    contacts: ReferenceAgentContacts,
    witness: Option<Box<dyn FnMut(u64) + Send>>,
    settlement: crate::direct_effect::SyncSettlement<circular_runtime::EffectId>,
}

impl ReferenceAgentExecutor {
    #[must_use]
    pub fn new(contacts: ReferenceAgentContacts) -> Self {
        Self {
            contacts,
            witness: None,
            settlement: crate::direct_effect::SyncSettlement::new(),
        }
    }

    #[must_use]
    pub fn with_witness(mut self, witness: impl FnMut(u64) + Send + 'static) -> Self {
        self.witness = Some(Box::new(witness));
        self
    }

    fn step(
        invoke: &circular_runtime::AgentInvokeSpec,
        ordinal: u64,
    ) -> Result<OutcomePayload, EffectFailure> {
        let session = invoke.session().cloned().unwrap_or_else(|| {
            AgentSessionId::new(invoke.harness().clone(), b"reference-session".to_vec())
        });
        let (progress, next) = match invoke.request() {
            AgentStepRequest::UserTurn(payload) => {
                let bytes = payload.as_bytes();
                if let Some(reason) = bytes.strip_prefix(b"fail:") {
                    let _ = reason;
                    return Err(EffectFailure::TransportTerminal);
                }
                if let Some(request) = bytes.strip_prefix(b"tool:") {
                    let mut parts = request.splitn(2, |byte| *byte == b':');
                    let name = parts.next().unwrap_or_default();
                    let arguments = parts.next().unwrap_or_default();
                    let Ok(name) = std::str::from_utf8(name) else {
                        return Err(EffectFailure::InterpreterFault(InterpreterFault::Other));
                    };
                    let Ok(tool) = ToolName::try_from_normalized(name.to_owned()) else {
                        return Err(EffectFailure::InterpreterFault(InterpreterFault::Other));
                    };
                    let call = AgentToolCallId::try_from_bytes(
                        format!("reference-call-{ordinal}").into_bytes(),
                    )
                    .expect("formatted call ids are never empty");
                    (
                        Vec::new(),
                        AgentStepNext::ToolRequest {
                            call: AgentToolCall::new(
                                call,
                                tool,
                                AgentPayload::new(arguments.to_vec()),
                            ),
                        },
                    )
                } else {
                    let record = circular_core::Value::object([(
                        "kind",
                        circular_core::Value::string("reference-step"),
                    )])
                    .expect("the record key is unique");
                    (
                        vec![AgentProgressRecord::new(AgentPayload::new(
                            circular_core::encode(
                                &record,
                                circular_core::Ceilings::for_boundary(
                                    circular_core::Boundary::Journal,
                                ),
                            )
                            .expect("reference progress is a small canonical object"),
                        ))],
                        AgentStepNext::Final {
                            output: payload.clone(),
                            metadata: AgentPayload::default(),
                        },
                    )
                }
            }
            AgentStepRequest::ToolResult { result, .. } => {
                let effect = result.effect().as_str();
                let output = match result.result() {
                    Ok(_) => format!("tool-ok:{effect}"),
                    Err(failure) => format!("tool-failed:{effect}:{failure:?}"),
                };
                (
                    Vec::new(),
                    AgentStepNext::Final {
                        output: AgentPayload::new(output.into_bytes()),
                        metadata: AgentPayload::default(),
                    },
                )
            }
        };
        let result = AgentStepResult::try_new(invoke.harness(), session, progress, next)
            .map_err(|_| EffectFailure::InterpreterFault(InterpreterFault::Other))?;
        Ok(OutcomePayload::AgentStepResult(result))
    }
}

impl Interpreter<circular_runtime::EffectId> for ReferenceAgentExecutor {
    fn submit(
        &mut self,
        correlation: circular_runtime::EffectId,
        effect: &Effect,
    ) -> Result<(), SubmitError<circular_runtime::EffectId>> {
        let result = match effect {
            Effect::AgentInvoke { invoke, .. } => {
                let ordinal = self.contacts.record();
                if let Some(witness) = self.witness.as_mut() {
                    witness(ordinal);
                }
                Self::step(invoke, ordinal)
            }
            _ => Err(EffectFailure::InterpreterFault(InterpreterFault::Other)),
        };
        self.settlement.record(correlation, result);
        Ok(())
    }

    fn submit_peer(
        &mut self,
        correlation: circular_runtime::EffectId,
        _effect: &circular_runtime::PeerEffect,
    ) -> Result<(), SubmitError<circular_runtime::EffectId>> {
        self.settlement
            .record(correlation, Err(EffectFailure::EndpointGone));
        Ok(())
    }

    fn next_outcome(&mut self) -> Option<EffectOutcome<circular_runtime::EffectId>> {
        self.settlement.next_outcome()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_runtime::{AgentHarnessGrant, AgentHarnessName, AgentInvokeSpec, GrantIssuer};

    fn harness(name: &str) -> AgentHarnessName {
        AgentHarnessName::try_from_normalized(name.to_owned()).expect("nonempty harness")
    }

    fn invoke_effect(turn: &[u8]) -> Effect {
        let target = harness("reference");
        let grant = AgentHarnessGrant::agent_harness([target.clone()]);
        let invoke = AgentInvokeSpec::try_new(
            target,
            None,
            AgentStepRequest::user_turn(AgentPayload::new(turn.to_vec())),
        )
        .expect("user turns carry no call identity");
        Effect::agent_invoke(GrantIssuer::new().issue(&grant), None, invoke)
    }

    fn settled(
        executor: &mut ReferenceAgentExecutor,
        correlation: circular_runtime::EffectId,
        turn: &[u8],
    ) -> EffectOutcome<circular_runtime::EffectId> {
        executor
            .submit(correlation, &invoke_effect(turn))
            .expect("reference executor accepts every submission");
        executor
            .next_outcome()
            .expect("outcome is ready immediately")
    }

    #[test]
    fn the_three_request_shapes_answer_deterministically() {
        let contacts = ReferenceAgentContacts::new();
        let mut executor = ReferenceAgentExecutor::new(contacts.clone());

        let outcome = settled(&mut executor, crate::test_effect(1), b"hello");
        let Ok(OutcomePayload::AgentStepResult(result)) = outcome.result() else {
            panic!("a plain turn settles as a step result");
        };
        assert!(matches!(
            result.next(),
            AgentStepNext::Final { output, .. } if output.as_bytes() == b"hello"
        ));
        assert_eq!(result.progress().len(), 1);
        assert_eq!(
            circular_core::decode(
                result.progress()[0].payload().as_bytes(),
                circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
            )
            .expect("reference progress follows the canonical object production rule"),
            circular_core::Value::object([(
                "kind",
                circular_core::Value::string("reference-step")
            )])
            .unwrap()
        );

        let outcome = settled(
            &mut executor,
            crate::test_effect(2),
            b"tool:write-note:payload",
        );
        let Ok(OutcomePayload::AgentStepResult(result)) = outcome.result() else {
            panic!("a tool turn settles as a step result");
        };
        let AgentStepNext::ToolRequest { call } = result.next() else {
            panic!("a tool turn asks for exactly one tool call");
        };
        assert_eq!(call.tool().as_str(), "write-note");
        assert_eq!(call.arguments().as_bytes(), b"payload");

        let outcome = settled(&mut executor, crate::test_effect(3), b"fail:downstream");
        assert_eq!(outcome.result(), &Err(EffectFailure::TransportTerminal));

        assert_eq!(contacts.count(), 3, "every live submission is one contact");
    }
}
