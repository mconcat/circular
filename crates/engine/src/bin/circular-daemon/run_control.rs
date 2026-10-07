use crate::daemon::session::LIFECYCLE_RESULT;
use crate::daemon::{approval, ledger};
use circular_core::{Boundary, Ceilings, Value};
use circular_protocol::Partition;
use circular_protocol::approval_payload::{ApprovalDecisionItem, ApprovalDecisionValue};
use circular_protocol::declaration_payload::{Accepted, CommandResult, Rejected};
use circular_protocol::rejection_code::RejectionReason;
use circular_protocol::{EnvelopeHeader, LifecycleAccepted, LifecycleResult};
use circular_transport::{OwnerLocalChannelId, write_envelope};
use engine::execution_profile::ProductExecutionProfile;

pub(crate) fn decide_decoded_approval(
    item: ApprovalDecisionItem,
    decision: ApprovalDecisionValue,
    owner: Option<engine::RuntimeApprovalQueue>,
) -> CommandResult {
    let Some(queue) = owner else {
        return CommandResult::Rejected(RejectionReason::Unresolved.reject(
            Partition::LedgerTransition,
            "there is no approval queue to decide".to_owned(),
        ));
    };
    let ApprovalDecisionItem { item } = item;
    let decision = match decision {
        ApprovalDecisionValue::Approve => circular_runtime::ApprovalDecision::Approve,
        ApprovalDecisionValue::Deny => circular_runtime::ApprovalDecision::Deny,
    };
    let item = match approval::key_from_value(&item) {
        Ok(item) => item,
        Err(_) => {
            return CommandResult::Rejected(RejectionReason::Unresolved.reject(
                Partition::LedgerTransition,
                "approval item is not a canonical EffectId",
            ));
        }
    };
    let applied = match queue.decide(item.clone(), decision) {
        Ok(applied) => applied,
        Err(error) => {
            return CommandResult::Rejected(RejectionReason::Unresolved.reject(
                Partition::LedgerTransition,
                format!("approval decision failed: {error:?}"),
            ));
        }
    };
    match approval::decision_receipt(item, applied) {
        Ok(receipt) => CommandResult::Accepted(Accepted::Transition(receipt)),
        Err(message) => CommandResult::Rejected(
            RejectionReason::Unresolved.reject(Partition::LedgerTransition, message),
        ),
    }
}

pub(crate) fn set_agent_harness(
    world: &crate::daemon::read_world::SharedWorld,
    execution: &ProductExecutionProfile,
    request: circular_protocol::agent_harness_payload::SetAgentHarness,
) -> CommandResult {
    use crate::daemon::environment;
    let rejected = |reason: RejectionReason, message: String| {
        CommandResult::Rejected(reason.reject(Partition::LedgerTransition, message))
    };
    let binding = match environment::admit_agent_harness(request.name.clone(), request.program) {
        Ok(binding) => binding,
        Err(rejection) => {
            return rejected(
                circular_protocol::rejection_code::Reasoned::reason(&rejection),
                rejection.to_string(),
            );
        }
    };
    let Ok(harness) = circular_runtime::AgentHarnessName::try_from_normalized(request.name.clone())
    else {
        return rejected(
            RejectionReason::Unresolved,
            "an admitted harness name is not a harness name".to_owned(),
        );
    };
    let Ok(_gate) = world.commit_gate() else {
        return rejected(
            RejectionReason::Unresolved,
            "daemon commit gate poisoned".to_owned(),
        );
    };
    let prefix = world.read();
    let directory = prefix.state_directory.as_path();
    let material = match (prefix.system.as_ref(), binding.as_ref()) {
        (Some(_), Some(binding)) => {
            let material = execution
                .agent_deadline()
                .ok_or_else(|| "this daemon has no process deadline".to_owned())
                .and_then(|deadline| {
                    environment::agent_binding_material(directory, deadline, binding)
                });
            match material {
                Ok((_, program, factory)) => Some((program, factory)),
                Err(reason) => return rejected(RejectionReason::Unresolved, reason),
            }
        }
        _ => None,
    };
    if let Err(message) =
        environment::write_agent_harness(directory, &request.name, binding.as_ref())
    {
        return rejected(RejectionReason::Unresolved, message);
    }
    if let Some(system) = prefix.system.as_ref()
        && system.pipeline.bind_harness(harness, material).is_err()
    {
        return rejected(
            RejectionReason::ArrivalRecorderStopped,
            "the binding is saved, but the standing System stopped and did not take it".to_owned(),
        );
    }
    CommandResult::Accepted(Accepted::Transition(
        circular_protocol::agent_harness_payload::set_agent_harness_accepted_value(),
    ))
}

pub(crate) fn answer_run_control(
    stream: &mut impl circular_transport::LocalByteStream<Error = std::io::Error>,
    header: EnvelopeHeader,
    result: LifecycleResult,
) {
    let body = match result.encode(Ceilings::for_boundary(Boundary::Wire)) {
        Ok(body) => body,
        Err(error) => {
            eprintln!("circular-daemon: could not encode LifecycleResult: {error:?}");
            return;
        }
    };
    if let Err(error) = write_envelope(
        stream,
        OwnerLocalChannelId::new(1),
        header.protocol_version(),
        LIFECYCLE_RESULT,
        header.correlation(),
        &body,
    ) {
        eprintln!("circular-daemon: could not answer Lifecycle: {error}");
    }
}

/// Resume is explicit. Recovery success alone does not release a recorded Pause.
pub(crate) fn start(
    world: &crate::daemon::read_world::SharedWorld,
    execution: &ProductExecutionProfile,
    expected_authoring_revision: Vec<u8>,
) -> LifecycleResult {
    let _gate = world.commit_gate().expect("daemon commit gate poisoned");
    let mut owner = world.run_write().expect("run projection owner poisoned");
    let authoring = world.read().authoring.clone();
    let directory = owner.state_directory().to_path_buf();
    let (server, system) = owner.parts();
    let state = match system
        .as_ref()
        .map(|system| system.pipeline.system_state())
        .transpose()
    {
        Ok(state) => state,
        Err(_) => return open_failed("System stopped".into()),
    };
    if server.is_some() && state.as_ref().is_none_or(|state| state.pause.is_none()) {
        return run_control_rejected(
            RejectionReason::AlreadyRunning,
            "the pipeline is already running".into(),
            Some("Pause the pipeline before another Resume".into()),
            None,
        );
    }
    let Some(revision) = authoring.revision() else {
        return run_control_rejected(
            RejectionReason::AuthoringUnavailable,
            "Resume requires committed authoring".into(),
            None,
            None,
        );
    };
    if revision != expected_authoring_revision.as_slice() {
        return run_control_rejected(
            RejectionReason::AuthoringRevisionMismatch,
            "Resume expected authoring revision is stale".into(),
            Some("refresh the authoritative authoring cut and retry".into()),
            Value::object([
                ("expected", Value::Bytes(expected_authoring_revision)),
                ("current", Value::Bytes(revision.to_vec())),
            ])
            .ok(),
        );
    }
    if let Some(system) = system.as_ref() {
        if server.is_none() {
            match ledger::recover_execution(
                &directory,
                execution,
                system,
                ledger::RecoveryInterrupt::process(),
            ) {
                Ok(run) => *server = Some(run),
                Err(reason) => return open_failed(reason),
            }
        }
    } else {
        match crate::daemon::declaration::stand_first_activation(
            &authoring, execution, &directory, system, None,
        ) {
            Ok(Some(run)) => *server = Some(run),
            Ok(None) => return open_failed("activation failed; read the System outcome".into()),
            Err(reason) => return open_failed(reason),
        }
    }
    let standing = server.as_ref().expect("Resume stands a server run");
    match standing.resume() {
        Ok(_) => LifecycleResult::Accepted(LifecycleAccepted::Resumed),
        Err(reason) => run_control_rejected(
            RejectionReason::LifecyclePersistenceFailed,
            format!("System refused Resume: {reason:?}"),
            None,
            None,
        ),
    }
}

/// Pause goes directly to System. The receipt acknowledges its durable acceptance,
/// not a barrier over member application, a second history append, or a world lock.
pub(crate) fn stop(
    world: &crate::daemon::read_world::SharedWorld,
    mode: Option<circular_protocol::PauseMode>,
) -> LifecycleResult {
    let prefix = world.read();
    let Some(system) = prefix.system.as_ref() else {
        return run_control_rejected(
            RejectionReason::NoStandingPipeline,
            "Pause found no pipeline".into(),
            None,
            None,
        );
    };
    let force = mode == Some(circular_protocol::PauseMode::ForcePause);
    let accepted = match prefix
        .server
        .as_deref()
        .and_then(|standing| standing.pause(force))
    {
        Some(accepted) => accepted,
        None => system.pipeline.pause(
            engine::RevisionEpochId::new(prefix.authoring.cursor()).expect("committed revision"),
            force,
        ),
    };
    match accepted {
        Ok(_) => LifecycleResult::Accepted(LifecycleAccepted::Paused),
        Err(reason) => run_control_rejected(
            RejectionReason::LifecyclePersistenceFailed,
            format!("System refused Pause: {reason:?}"),
            None,
            None,
        ),
    }
}

fn open_failed(reason: String) -> LifecycleResult {
    run_control_rejected(RejectionReason::RuntimeOpenFailed, reason, None, None)
}

fn run_control_rejected(
    reason: RejectionReason,
    message: String,
    hint: Option<String>,
    at: Option<Value>,
) -> LifecycleResult {
    LifecycleResult::Rejected(Rejected {
        hint,
        at,
        ..reason.reject(Partition::Lifecycle, message)
    })
}

