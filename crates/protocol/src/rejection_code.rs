
use circular_core::Value;

use crate::Partition;
use crate::declaration_payload::Rejected;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RejectionReason {
    Malformed,
    Unresolved,
    MalformedPayload,
    HelloRequired,
    SessionClosed,
    UnexpectedHello,
    OutboundOnly,
    RoleInsufficient,
    ProtocolVersionMismatch,
    IncompleteFeatures,
    OwnerLocalRequired,
    EntropyUnavailable,
    GrantedUnrequestedRole,
    LiveCorrelationUnavailable,
    QueryResultEncodingFailed,
    NoStandingPipeline,
    InputNotAccepted,
    ActivationFailed,
    ArrivalRecorderStopped,
    RevisionAdoptionFailed,
    RevisionConflict,
    JournalFormatRejected,
    ClosedByClient,
    RecoveryFailed,
    EndedBeforeAnswering,
    UnknownHarness,
    InvalidProgramPath,
    ProgramNotExecutable,
    AuthoringUnavailable,
    AuthoringRevisionMismatch,
    AlreadyRunning,
    RuntimeOpenFailed,
    LifecyclePersistenceFailed,
    DesiredRunningUnavailable,
}

impl RejectionReason {
    pub const ALL: [Self; 34] = [
        Self::Malformed,
        Self::Unresolved,
        Self::MalformedPayload,
        Self::HelloRequired,
        Self::SessionClosed,
        Self::UnexpectedHello,
        Self::OutboundOnly,
        Self::RoleInsufficient,
        Self::ProtocolVersionMismatch,
        Self::IncompleteFeatures,
        Self::OwnerLocalRequired,
        Self::EntropyUnavailable,
        Self::GrantedUnrequestedRole,
        Self::LiveCorrelationUnavailable,
        Self::QueryResultEncodingFailed,
        Self::NoStandingPipeline,
        Self::InputNotAccepted,
        Self::ActivationFailed,
        Self::ArrivalRecorderStopped,
        Self::RevisionAdoptionFailed,
        Self::RevisionConflict,
        Self::JournalFormatRejected,
        Self::ClosedByClient,
        Self::RecoveryFailed,
        Self::EndedBeforeAnswering,
        Self::UnknownHarness,
        Self::InvalidProgramPath,
        Self::ProgramNotExecutable,
        Self::AuthoringUnavailable,
        Self::AuthoringRevisionMismatch,
        Self::AlreadyRunning,
        Self::RuntimeOpenFailed,
        Self::LifecyclePersistenceFailed,
        Self::DesiredRunningUnavailable,
    ];

    pub const RETIRED_OUTSIDE_LIFECYCLE: [u32; 4] = [12, 13, 14, 18];

    const fn numbers(self) -> (Option<u32>, Option<u32>) {
        match self {
            Self::Malformed => (Some(1), Some(1)),
            Self::Unresolved => (Some(2), None),
            Self::MalformedPayload => (Some(3), Some(1)),
            Self::HelloRequired => (Some(4), Some(10)),
            Self::SessionClosed => (Some(5), Some(11)),
            Self::UnexpectedHello => (Some(6), None),
            Self::OutboundOnly => (Some(7), Some(12)),
            Self::RoleInsufficient => (Some(8), Some(13)),
            Self::ProtocolVersionMismatch => (Some(9), None),
            Self::IncompleteFeatures => (Some(10), None),
            Self::OwnerLocalRequired => (Some(11), None),
            Self::EntropyUnavailable => (Some(15), None),
            Self::GrantedUnrequestedRole => (Some(16), None),
            Self::LiveCorrelationUnavailable => (Some(17), Some(14)),
            Self::QueryResultEncodingFailed => (Some(19), None),
            Self::NoStandingPipeline => (Some(20), Some(5)),
            Self::InputNotAccepted => (Some(21), None),
            Self::ActivationFailed => (Some(22), None),
            Self::ArrivalRecorderStopped => (Some(23), None),
            Self::RevisionAdoptionFailed => (Some(24), None),
            Self::RevisionConflict => (Some(26), None),
            Self::JournalFormatRejected => (Some(28), None),
            Self::ClosedByClient => (Some(29), None),
            Self::RecoveryFailed => (Some(30), None),
            Self::EndedBeforeAnswering => (Some(31), None),
            Self::UnknownHarness => (Some(6_401), None),
            Self::InvalidProgramPath => (Some(6_402), None),
            Self::ProgramNotExecutable => (Some(6_403), None),
            Self::AuthoringUnavailable => (None, Some(2)),
            Self::AuthoringRevisionMismatch => (None, Some(3)),
            Self::AlreadyRunning => (None, Some(4)),
            Self::RuntimeOpenFailed => (None, Some(7)),
            Self::LifecyclePersistenceFailed => (None, Some(8)),
            Self::DesiredRunningUnavailable => (None, Some(9)),
        }
    }

    #[must_use]
    pub const fn code_in(self, partition: Partition) -> Option<u32> {
        let (outside, lifecycle) = self.numbers();
        match partition {
            Partition::Lifecycle => lifecycle,
            _ => outside,
        }
    }

    #[must_use]
    pub fn from_code(partition: Partition, code: u32) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|reason| reason.code_in(partition) == Some(code))
    }

    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Malformed => "malformed command payload",
            Self::Unresolved => "the command names nothing that resolves",
            Self::MalformedPayload => "malformed session command payload",
            Self::HelloRequired => "Hello must establish the session before commands",
            Self::SessionClosed => "the session is closed",
            Self::UnexpectedHello => "Hello may occur only once per session",
            Self::OutboundOnly => "the verb is outbound-only",
            Self::RoleInsufficient => "the established roles do not admit this command",
            Self::ProtocolVersionMismatch => "the Hello protocol version does not match",
            Self::IncompleteFeatures => "Hello must declare every partition feature",
            Self::OwnerLocalRequired => "the owner-local transport identity is required",
            Self::EntropyUnavailable => "the OS session token source is unavailable",
            Self::GrantedUnrequestedRole => "the establishment policy granted an unrequested role",
            Self::LiveCorrelationUnavailable => {
                "live correlation is already in use or the session limit of 4096 is reached"
            }
            Self::QueryResultEncodingFailed => "query result encoding failed",
            Self::NoStandingPipeline => "no pipeline is standing — commit an epoch first",
            Self::InputNotAccepted => {
                "the source owner has recorded a pause and is not accepting input"
            }
            Self::ActivationFailed => {
                "the committed epoch stands, but its pipeline could not be activated"
            }
            Self::ArrivalRecorderStopped => {
                "this daemon's arrival recorder stopped; nothing further is recorded"
            }
            Self::RevisionAdoptionFailed => {
                "the pipeline stands, but it could not adopt the committed revision"
            }
            Self::RevisionConflict => {
                "the expected authoring revision differs from the current revision"
            }
            Self::JournalFormatRejected => "the stored journal format is not supported",
            Self::ClosedByClient => "the client closed the page sequence before its end",
            Self::RecoveryFailed => {
                "the retained stream could not be recovered after restart; Resume retries it"
            }
            Self::EndedBeforeAnswering => "the actor ended before it answered",
            Self::UnknownHarness => "unknown agent harness",
            Self::InvalidProgramPath => "agent program path is not an absolute normalized path",
            Self::ProgramNotExecutable => {
                "agent program does not exist or is not an executable file"
            }
            Self::AuthoringUnavailable => "Resume requires a committed authored plan",
            Self::AuthoringRevisionMismatch => "Resume expected authoring revision is stale",
            Self::AlreadyRunning => "a pipeline is already standing",
            Self::RuntimeOpenFailed => "the runtime could not be opened",
            Self::LifecyclePersistenceFailed => "the pipeline lifecycle could not be recorded",
            Self::DesiredRunningUnavailable => "the desired running state is unavailable",
        }
    }

    #[must_use]
    pub const fn number_in(self, partition: Partition) -> u32 {
        match self.code_in(partition) {
            Some(code) => code,
            None => panic!("the reason has no number in this partition"),
        }
    }

    #[must_use]
    pub const fn recorded_code(self) -> u32 {
        self.number_in(Partition::Query)
    }

    #[must_use]
    pub fn reject(self, partition: Partition, message: impl Into<String>) -> Rejected {
        Rejected {
            code: self.number_in(partition),
            message: message.into(),
            hint: None,
            at: None,
        }
    }
}

pub trait Reasoned: std::fmt::Display {
    fn reason(&self) -> RejectionReason;

    fn at(&self) -> Option<Value> {
        None
    }

    fn rejected(&self, partition: Partition) -> Rejected {
        let rejected = self.reason().reject(partition, self.to_string());
        match self.at() {
            Some(at) => rejected.at(at),
            None => rejected,
        }
    }
}

impl Rejected {
    #[must_use]
    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    #[must_use]
    pub fn at(mut self, at: Value) -> Self {
        self.at = Some(at);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUTSIDE: Partition = Partition::Declaration;
    const LIFECYCLE: Partition = Partition::Lifecycle;

    const OUTSIDE_TABLE: [(RejectionReason, u32); 28] = [
        (RejectionReason::Malformed, 1),
        (RejectionReason::Unresolved, 2),
        (RejectionReason::MalformedPayload, 3),
        (RejectionReason::HelloRequired, 4),
        (RejectionReason::SessionClosed, 5),
        (RejectionReason::UnexpectedHello, 6),
        (RejectionReason::OutboundOnly, 7),
        (RejectionReason::RoleInsufficient, 8),
        (RejectionReason::ProtocolVersionMismatch, 9),
        (RejectionReason::IncompleteFeatures, 10),
        (RejectionReason::OwnerLocalRequired, 11),
        (RejectionReason::EntropyUnavailable, 15),
        (RejectionReason::GrantedUnrequestedRole, 16),
        (RejectionReason::LiveCorrelationUnavailable, 17),
        (RejectionReason::QueryResultEncodingFailed, 19),
        (RejectionReason::NoStandingPipeline, 20),
        (RejectionReason::InputNotAccepted, 21),
        (RejectionReason::ActivationFailed, 22),
        (RejectionReason::ArrivalRecorderStopped, 23),
        (RejectionReason::RevisionAdoptionFailed, 24),
        (RejectionReason::RevisionConflict, 26),
        (RejectionReason::JournalFormatRejected, 28),
        (RejectionReason::ClosedByClient, 29),
        (RejectionReason::RecoveryFailed, 30),
        (RejectionReason::EndedBeforeAnswering, 31),
        (RejectionReason::UnknownHarness, 6_401),
        (RejectionReason::InvalidProgramPath, 6_402),
        (RejectionReason::ProgramNotExecutable, 6_403),
    ];
    const LIFECYCLE_TABLE: [(RejectionReason, u32); 14] = [
        (RejectionReason::Malformed, 1),
        (RejectionReason::MalformedPayload, 1),
        (RejectionReason::AuthoringUnavailable, 2),
        (RejectionReason::AuthoringRevisionMismatch, 3),
        (RejectionReason::AlreadyRunning, 4),
        (RejectionReason::NoStandingPipeline, 5),
        (RejectionReason::RuntimeOpenFailed, 7),
        (RejectionReason::LifecyclePersistenceFailed, 8),
        (RejectionReason::DesiredRunningUnavailable, 9),
        (RejectionReason::HelloRequired, 10),
        (RejectionReason::SessionClosed, 11),
        (RejectionReason::OutboundOnly, 12),
        (RejectionReason::RoleInsufficient, 13),
        (RejectionReason::LiveCorrelationUnavailable, 14),
    ];

    fn expected(table: &[(RejectionReason, u32)], reason: RejectionReason) -> Option<u32> {
        table
            .iter()
            .find(|(arm, _)| *arm == reason)
            .map(|(_, code)| *code)
    }

    #[test]
    fn every_reason_answers_the_published_number_of_each_partition() {
        for reason in RejectionReason::ALL {
            assert_eq!(
                reason.code_in(OUTSIDE),
                expected(&OUTSIDE_TABLE, reason),
                "{reason:?} outside Lifecycle"
            );
            assert_eq!(
                reason.code_in(LIFECYCLE),
                expected(&LIFECYCLE_TABLE, reason),
                "{reason:?} in Lifecycle"
            );
        }
        for (reason, _) in OUTSIDE_TABLE.iter().chain(&LIFECYCLE_TABLE) {
            assert!(RejectionReason::ALL.contains(reason), "{reason:?}");
        }
    }

    #[test]
    fn every_partition_but_lifecycle_shares_one_line() {
        for partition in [
            Partition::SessionMechanics,
            Partition::Declaration,
            Partition::Query,
            Partition::Subscription,
            Partition::EventInjection,
            Partition::LedgerTransition,
            Partition::ReplayControl,
        ] {
            for reason in RejectionReason::ALL {
                assert_eq!(reason.code_in(partition), reason.code_in(OUTSIDE));
            }
        }
    }

    #[test]
    fn a_number_reads_back_as_its_reason_within_its_partition() {
        for (reason, code) in OUTSIDE_TABLE {
            assert_eq!(RejectionReason::from_code(OUTSIDE, code), Some(reason));
        }
        for (reason, code) in LIFECYCLE_TABLE {
            let read = RejectionReason::from_code(LIFECYCLE, code);
            if reason == RejectionReason::MalformedPayload {
                assert_eq!(read, Some(RejectionReason::Malformed));
            } else {
                assert_eq!(read, Some(reason));
            }
        }
    }

    #[test]
    fn a_number_outside_the_tables_does_not_resolve() {
        for code in [0_u32, 12, 13, 14, 18, 32, u32::MAX] {
            assert_eq!(RejectionReason::from_code(OUTSIDE, code), None, "{code}");
        }
        for code in [0_u32, 15, 26, u32::MAX] {
            assert_eq!(RejectionReason::from_code(LIFECYCLE, code), None, "{code}");
        }
        for code in RejectionReason::RETIRED_OUTSIDE_LIFECYCLE {
            assert_eq!(RejectionReason::from_code(OUTSIDE, code), None, "{code}");
        }
    }

    #[test]
    fn every_reason_carries_a_message() {
        for reason in RejectionReason::ALL {
            assert!(!reason.message().is_empty(), "{reason:?}");
        }
    }

    #[test]
    fn a_rejection_carries_the_number_of_its_partition_and_its_builders() {
        let rejected = RejectionReason::NoStandingPipeline
            .reject(LIFECYCLE, "none")
            .hint("commit first")
            .at(Value::string("lifecycle"));
        assert_eq!(rejected.code, 5);
        assert_eq!(rejected.message, "none");
        assert_eq!(rejected.hint.as_deref(), Some("commit first"));
        assert_eq!(rejected.at, Some(Value::string("lifecycle")));
        assert_eq!(
            RejectionReason::NoStandingPipeline
                .reject(OUTSIDE, "none")
                .code,
            20
        );
    }
}
