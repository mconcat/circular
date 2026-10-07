#![forbid(unsafe_code)]

mod build_identity;
mod canonical_frame;
mod envelope_integration;
mod fsm;
mod local;
#[cfg(any(target_vendor = "apple", target_os = "linux"))]
mod owner_local_establishment;
mod owner_local_invocation;
mod owner_local_permissions;
#[cfg(any(target_vendor = "apple", target_os = "linux"))]
mod owner_local_session;
mod owner_local_socket;

pub use build_identity::{BuildIdentity, build_identity};
pub use canonical_frame::{
    ChannelIdSource, OWNER_LOCAL_FRAME_HEADER_BYTES, OWNER_LOCAL_FRAMING_VERSION,
    OWNER_LOCAL_MAX_FRAME_BODY_BYTES, OWNER_LOCAL_MAX_MESSAGE_BYTES, OwnerLocalChannelId,
    OwnerLocalChannelIdSource, OwnerLocalFrame, OwnerLocalFrameDecode, OwnerLocalFrameError,
    decode_owner_local_frame_exact, decode_owner_local_frame_prefix, owner_local_transport_limits,
};
pub use envelope_integration::{
    EnvelopeIntegrationError, ReassembledEnvelope, SessionEnvelopeReassembler,
    chunk_session_envelope, decode_session_envelope,
};
pub use fsm::{
    FrameOperation, MessageLimitExceeded, OpaqueChunk, Reassembler, ReassemblyError,
    ReassemblyOutcome, ReassemblyState, Segment, TransportLimits, TransportLimitsError,
    chunk_opaque_body,
};
pub use local::{
    LocalByteStream, LocalEndpoint, LocalEndpointEvidence, LocalEvidenceError, OwnerLocalEvidence,
    UserLocalEvidence, establish_local_trust,
};
#[cfg(any(target_vendor = "apple", target_os = "linux"))]
pub use owner_local_establishment::{OwnerLocalHelloError, exchange_owner_local_hello};
pub use owner_local_invocation::{
    AcceptedStateDirectory, CANONICAL_STATE_OPTION, CIRCULAR_VERSION, DAEMON_OPTIONS,
    DaemonArguments, DaemonArgumentsError, DaemonOption, OWNER_LOCAL_HELP_OPTIONS,
    OWNER_LOCAL_SOCKET_NAME, OWNER_LOCAL_VERSION_OPTION, OwnerLocalInvocation,
    StateDirectoryRejection, current_effective_user, daemon_usage, owner_local_usage,
    owner_local_version,
};
pub use owner_local_permissions::{
    OWNER_ROOT_MODE, OwnerRootModeMismatch, validate_owner_root_mode,
};
pub use owner_local_session::{
    SessionIoError, read_envelope, read_envelope_polling, write_envelope,
};
#[cfg(any(target_vendor = "apple", target_os = "linux"))]
pub use owner_local_socket::{
    Custody, IoStep, OWNER_LOCAL_SOCKET_PATH_MAX_BYTES, OwnerLocalClaim, OwnerLocalListener,
    OwnerLocalSocketError, OwnerLocalSocketSpec, OwnerLocalStream, Violation,
};
use std::io;

#[must_use]
pub fn is_socket_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}
