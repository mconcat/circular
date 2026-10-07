//! Abstract owner-local and user-local transport surfaces.
//!
//! This module intentionally does not choose a socket API, frame layout, or
//! buffer size.  The deployment owner supplies endpoint values and peer
//! evidence; the framing owner supplies opaque buffers.

mod byte_stream;
mod evidence;

pub use byte_stream::LocalByteStream;
pub use evidence::{
    LocalEndpoint, LocalEndpointEvidence, LocalEvidenceError, OwnerLocalEvidence,
    UserLocalEvidence, establish_local_trust,
};
