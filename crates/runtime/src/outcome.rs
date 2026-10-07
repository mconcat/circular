
use crate::{
    AgentProgressRecord, AgentStepResult, ApprovalRequestOutcome, Capability, ScheduleCorrelation,
};
use std::io;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessResult {
    exit_code: i32,
    stdout: Box<[u8]>,
    stderr: Box<[u8]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpResponse {
    status: u16,
    body: Box<[u8]>,
    truncated: bool,
    retry_after_seconds: Option<u64>,
}

impl HttpResponse {
    #[must_use]
    pub fn new(
        status: u16,
        body: impl Into<Box<[u8]>>,
        truncated: bool,
        retry_after_seconds: Option<u64>,
    ) -> Self {
        Self {
            status,
            body: body.into(),
            truncated,
            retry_after_seconds,
        }
    }

    #[must_use]
    pub const fn status(&self) -> u16 {
        self.status
    }

    #[must_use]
    pub const fn body(&self) -> &[u8] {
        &self.body
    }

    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    #[must_use]
    pub const fn retry_after_seconds(&self) -> Option<u64> {
        self.retry_after_seconds
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationReceipt {
    channel: crate::NotificationChannel,
}

impl NotificationReceipt {
    #[must_use]
    pub const fn delivered(channel: crate::NotificationChannel) -> Self {
        Self { channel }
    }

    #[must_use]
    pub const fn channel(&self) -> &crate::NotificationChannel {
        &self.channel
    }
}

impl ProcessResult {
    #[must_use]
    pub fn direct(
        exit_code: i32,
        stdout: impl Into<Box<[u8]>>,
        stderr: impl Into<Box<[u8]>>,
    ) -> Self {
        Self {
            exit_code,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        self.exit_code
    }

    #[must_use]
    pub const fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    #[must_use]
    pub const fn stderr(&self) -> &[u8] {
        &self.stderr
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OutcomePayload<I = crate::EffectId> {
    HttpResponse(HttpResponse),
    FileBytes(Box<[u8]>),
    WrittenLength(u64),
    ProcessResult(ProcessResult),
    NotificationDelivered(NotificationReceipt),
    AgentStepResult(AgentStepResult),
    Approval(ApprovalRequestOutcome<I>),
    ScheduleArmed(ScheduleCorrelation),
    PeerSnapshot(crate::PeerSnapshot),
    PeerBinding(crate::PeerBinding),
    SubmissionReceipt(crate::SubmissionReceipt),
    UnbindReceipt(crate::UnbindReceipt),
    PeerEnvelope(crate::PeerEventEnvelope),
}

impl<I> OutcomePayload<I> {
    #[must_use]
    pub const fn kind_tag(&self) -> &'static str {
        match self {
            Self::HttpResponse(_) => "http_response",
            Self::FileBytes(_) => "file_bytes",
            Self::WrittenLength(_) => "written_length",
            Self::ProcessResult(_) => "process_result",
            Self::NotificationDelivered(_) => "notification_delivered",
            Self::AgentStepResult(_) => "agent_step_result",
            Self::Approval(_) => "approval",
            Self::ScheduleArmed(_) => "schedule_armed",
            Self::PeerSnapshot(_) => "peer_snapshot",
            Self::PeerBinding(_) => "peer_binding",
            Self::SubmissionReceipt(_) => "submission_receipt",
            Self::UnbindReceipt(_) => "unbind_receipt",
            Self::PeerEnvelope(_) => "peer_envelope",
        }
    }
}

circular_core::closed_table! {
    pub enum Divergence: u8 {
        MissingRecord = 1,
        EffectMismatch = 2,
    }
}

circular_core::closed_table! {
    pub enum InterpreterFault: u8 {
        NotFound = 1,
        PermissionDenied = 2,
        AlreadyExists = 3,
        InvalidInput = 4,
        BrokenPipe = 5,
        ResourceExhausted = 6,
        Interrupted = 7,
        Other = 8,
    }
}

impl From<io::ErrorKind> for InterpreterFault {
    fn from(kind: io::ErrorKind) -> Self {
        match kind {
            io::ErrorKind::NotFound => Self::NotFound,
            io::ErrorKind::PermissionDenied => Self::PermissionDenied,
            io::ErrorKind::AlreadyExists => Self::AlreadyExists,
            io::ErrorKind::InvalidInput | io::ErrorKind::InvalidData => Self::InvalidInput,
            io::ErrorKind::BrokenPipe => Self::BrokenPipe,
            io::ErrorKind::OutOfMemory | io::ErrorKind::StorageFull => Self::ResourceExhausted,
            io::ErrorKind::Interrupted => Self::Interrupted,
            _ => Self::Other,
        }
    }
}

circular_core::closed_table! {
    pub enum EffectFailureKind {
        ParameterDenied => "parameter_denied",
        TransportTerminal => "transport_terminal",
        ApprovalRequired => "approval_required",
        Diverged => "diverged",
        EndpointGone => "endpoint_gone",
        InterpreterFault => "interpreter_fault",
        RetryExhausted => "retry_exhausted",
        TransportUnreached => "transport_unreached",
        RemoteDeferred => "remote_deferred",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EffectFailure {
    ParameterDenied {
        capability: Capability,
    },
    TransportTerminal,
    ApprovalRequired,
    Diverged(Divergence),
    EndpointGone,
    InterpreterFault(InterpreterFault),
    Peer(crate::PeerFailureKind),
    RetryExhausted {
        attempts: u32,
    },
    TransportUnreached,
    RemoteDeferred,
}

impl EffectFailure {
    #[must_use]
    pub const fn kind_tag(&self) -> &'static str {
        let kind = match self {
            Self::ParameterDenied { .. } => EffectFailureKind::ParameterDenied,
            Self::TransportTerminal => EffectFailureKind::TransportTerminal,
            Self::ApprovalRequired => EffectFailureKind::ApprovalRequired,
            Self::Diverged(_) => EffectFailureKind::Diverged,
            Self::EndpointGone => EffectFailureKind::EndpointGone,
            Self::InterpreterFault(_) => EffectFailureKind::InterpreterFault,
            Self::RetryExhausted { .. } => EffectFailureKind::RetryExhausted,
            Self::TransportUnreached => EffectFailureKind::TransportUnreached,
            Self::RemoteDeferred => EffectFailureKind::RemoteDeferred,
            Self::Peer(kind) => return kind.as_str(),
        };
        kind.as_str()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectOutcome<I> {
    correlation: I,
    result: Result<OutcomePayload, EffectFailure>,
    failure_progress: Vec<AgentProgressRecord>,
}

impl<I> EffectOutcome<I> {
    #[must_use]
    pub const fn new(correlation: I, result: Result<OutcomePayload, EffectFailure>) -> Self {
        Self {
            correlation,
            result,
            failure_progress: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_failure_progress(mut self, progress: Vec<AgentProgressRecord>) -> Self {
        self.failure_progress = progress;
        self
    }

    #[must_use]
    pub fn failure_progress(&self) -> &[AgentProgressRecord] {
        &self.failure_progress
    }

    #[must_use]
    pub const fn correlation(&self) -> &I {
        &self.correlation
    }

    pub const fn result(&self) -> &Result<OutcomePayload, EffectFailure> {
        &self.result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_response_has_the_published_kind_tag() {
        let response = HttpResponse::new(200, b"ok".to_vec(), false, None);
        assert_eq!(
            OutcomePayload::<u64>::HttpResponse(response).kind_tag(),
            "http_response"
        );
    }
}
