//! Provider-neutral, bidirectional peer contract and in-memory reference adapter.
//!
//! A peer is not an agent invocation. Submission and a later inbound message are
//! independent observations, and a provider-owned peer remains independently alive.
//! The reference adapter in this module exists to close the state machine before a
//! Claude, Codex, or Pi bridge is allowed to claim conformance.

mod adapter;
mod envelope_codec;
mod ids;
mod ingress;
mod memory;
mod peer;
mod protocol;

pub use adapter::*;
pub use envelope_codec::{decode_peer_envelope, encode_peer_envelope};
pub use ids::*;
pub use ingress::*;
pub use memory::*;
pub use peer::*;
pub use protocol::*;

