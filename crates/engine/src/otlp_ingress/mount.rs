//! An authored OTLP source owns this resource; the graph owner supplies recorded
//! delivery and the already-resolved custody directory. No default port, global
//! mount name, or new durable namespace is selected here.
use super::{
    edge::{OtlpDelivery, OtlpEdge},
    state::{OtlpEdgeCounters, OtlpSignal, OtlpStateStore},
};
use crate::activation_detail::{RegistrationFailure, source};
use circular_actors::otlp::OtlpConfig;
use circular_core::Value;
use std::path::Path;

pub struct OtlpMount {
    edge: OtlpEdge,
}
impl OtlpMount {
    pub(crate) fn activate(
        config: &Value,
        custody_directory: &Path,
        delivery: impl FnMut(OtlpSignal, &str, &[u8]) -> OtlpDelivery + Send + 'static,
        fallen: impl Fn(RegistrationFailure) + Send + Sync + 'static,
    ) -> Result<Self, RegistrationFailure> {
        let config = OtlpConfig::from_value(config).map_err(|error| {
            RegistrationFailure::new(source::LISTEN_REJECTED, format!("ConfigRejected: {error}"))
        })?;
        let state = OtlpStateStore::create_empty(custody_directory)
            .map_err(|message| RegistrationFailure::new(source::CUSTODY_UNAVAILABLE, message))?;
        OtlpEdge::start_with_delivery(config.listen(), state, delivery, fallen)
            .map(|edge| Self { edge })
    }
    /// Resume this same actor's custody after shutdown or a failed bind. The
    /// lifecycle owner must not resolve this path from retired catch config.
    pub(crate) fn resume(
        config: &Value,
        custody_directory: &Path,
        delivery: impl FnMut(OtlpSignal, &str, &[u8]) -> OtlpDelivery + Send + 'static,
        fallen: impl Fn(RegistrationFailure) + Send + Sync + 'static,
    ) -> Result<Self, RegistrationFailure> {
        let config = OtlpConfig::from_value(config).map_err(|error| {
            RegistrationFailure::new(source::LISTEN_REJECTED, format!("ConfigRejected: {error}"))
        })?;
        let state = OtlpStateStore::open(custody_directory)
            .map_err(|message| RegistrationFailure::new(source::CUSTODY_UNAVAILABLE, message))?;
        OtlpEdge::start_with_delivery(config.listen(), state, delivery, fallen)
            .map(|edge| Self { edge })
    }

    /// Join this mount's receiver and delivery worker and expose final health
    /// and counters to the lifecycle owner. Other mounts remain untouched.
    pub fn shutdown(self) -> Result<OtlpEdgeCounters, String> {
        self.edge.shutdown()
    }

    pub const fn edge(&self) -> &OtlpEdge {
        &self.edge
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_listen_rejects_before_opening_custody_or_binding() {
        for listen in ["localhost:4318", "0.0.0.0:4318", "127.0.0.1:0", "[::]:4318"] {
            let config = Value::object([("listen", Value::String(listen.into()))]).unwrap();
            let result = OtlpMount::activate(
                &config,
                Path::new("/unopened-otlp-test-state"),
                |_, _, _| panic!("invalid mount delivered"),
                |_| {},
            );
            match result {
                Err(failure) => {
                    assert!(failure.to_string().starts_with("ConfigRejected:"));
                    assert_eq!(failure.detail(), source::LISTEN_REJECTED);
                }
                Ok(_) => panic!("invalid listen was mounted"),
            }
        }
    }
}
