use crate::tap_pilot::{TapPilotActivationError, TapPilotBoundaryError};
use circular_actors::FailureDetail;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegistrationFailure {
    detail: FailureDetail,
    message: String,
}

impl RegistrationFailure {
    #[must_use]
    pub fn new(detail: FailureDetail, message: impl Into<String>) -> Self {
        Self {
            detail,
            message: message.into(),
        }
    }

    #[must_use]
    pub const fn detail(&self) -> FailureDetail {
        self.detail
    }
}

impl fmt::Display for RegistrationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub mod agent_harness {
    use circular_actors::FailureDetail;

    pub const PROGRAM_NOT_EXECUTABLE: FailureDetail =
        FailureDetail::new("agent_harness.program_not_executable", None);
}

pub mod source {
    use circular_actors::FailureDetail;

    pub const CUSTODY_UNAVAILABLE: FailureDetail =
        FailureDetail::new("source.custody_unavailable", None);
    pub const LISTEN_REJECTED: FailureDetail =
        FailureDetail::new("source.listen_rejected", Some("listen"));
    pub const BIND_REFUSED: FailureDetail =
        FailureDetail::new("source.bind_refused", Some("listen"));
    pub const RECEIVER_UNSTARTED: FailureDetail =
        FailureDetail::new("source.receiver_unstarted", None);
    pub const ROOTS_REJECTED: FailureDetail = FailureDetail::new("source.roots_rejected", None);

    pub const RECEIVER_WAIT_FAILED: FailureDetail =
        FailureDetail::new("source.receiver_wait_failed", None);
    pub const RECEIVER_BLOCKING_UNRESTORED: FailureDetail =
        FailureDetail::new("source.receiver_blocking_unrestored", None);
    pub const RECEIVER_CONNECTION_FAILED: FailureDetail =
        FailureDetail::new("source.receiver_connection_failed", None);
    pub const RECEIVER_ACCEPT_FAILED: FailureDetail =
        FailureDetail::new("source.receiver_accept_failed", None);
    pub const FORWARDER_STATE_POISONED: FailureDetail =
        FailureDetail::new("source.forwarder_state_poisoned", None);
    pub const FORWARDER_SPLIT_OVERSIZED: FailureDetail =
        FailureDetail::new("source.forwarder_split_oversized", None);
    pub const FORWARDER_ACKNOWLEDGE_FAILED: FailureDetail =
        FailureDetail::new("source.forwarder_acknowledge_failed", None);
    pub const HEALTH_POISONED: FailureDetail = FailureDetail::new("source.health_poisoned", None);
    pub const DELIVERY_FAILED: FailureDetail = FailureDetail::new("source.delivery_failed", None);
    pub const FRAGMENT_UNPARSABLE: FailureDetail =
        FailureDetail::new("source.fragment_unparsable", None);
    pub const FRAGMENT_VALUE_REJECTED: FailureDetail =
        FailureDetail::new("source.fragment_value_rejected", None);
    pub const FRAGMENT_NOT_OBJECT: FailureDetail =
        FailureDetail::new("source.fragment_not_object", None);
    pub const SUBMIT_REJECTED: FailureDetail = FailureDetail::new("source.submit_rejected", None);
    pub const REFUSAL_DRAIN_FAILED: FailureDetail =
        FailureDetail::new("source.refusal_drain_failed", None);

    pub const GLOB_UNPUBLISHED_SYNTAX: FailureDetail =
        FailureDetail::new("source.glob_unpublished_syntax", Some("source"));
    pub const GLOB_MIXED_GLOBSTAR: FailureDetail =
        FailureDetail::new("source.glob_mixed_globstar", Some("source"));

    #[must_use]
    pub const fn glob(
        rejection: &crate::peer_bridges::transcript::GlobRejection,
    ) -> Option<FailureDetail> {
        use crate::peer_bridges::transcript::GlobRejection;
        match rejection {
            GlobRejection::NotAbsolute => None,
            GlobRejection::UnpublishedSyntax { .. } => Some(GLOB_UNPUBLISHED_SYNTAX),
            GlobRejection::MixedGlobstar => Some(GLOB_MIXED_GLOBSTAR),
        }
    }
}

pub mod journal {
    use circular_actors::FailureDetail;

    pub const ARRIVALS_BYTES_EXCEEDED: FailureDetail =
        FailureDetail::new("journal.arrivals_bytes_exceeded", None);
    pub const ARRIVALS_RECORDS_EXCEEDED: FailureDetail =
        FailureDetail::new("journal.arrivals_records_exceeded", None);
    pub const ARRIVALS_BYTES_AND_RECORDS_EXCEEDED: FailureDetail =
        FailureDetail::new("journal.arrivals_bytes_and_records_exceeded", None);
    pub const TOTAL_BYTES_EXCEEDED: FailureDetail =
        FailureDetail::new("journal.total_bytes_exceeded", None);
    pub const ARRIVALS_BYTES_AND_TOTAL_BYTES_EXCEEDED: FailureDetail =
        FailureDetail::new("journal.arrivals_bytes_and_total_bytes_exceeded", None);
    pub const ARRIVALS_RECORDS_AND_TOTAL_BYTES_EXCEEDED: FailureDetail =
        FailureDetail::new("journal.arrivals_records_and_total_bytes_exceeded", None);
    pub const ARRIVALS_BYTES_AND_RECORDS_AND_TOTAL_BYTES_EXCEEDED: FailureDetail =
        FailureDetail::new(
            "journal.arrivals_bytes_and_records_and_total_bytes_exceeded",
            None,
        );

    #[must_use]
    pub const fn exceeded(
        arrivals_bytes: bool,
        arrivals_records: bool,
        total_bytes: bool,
    ) -> Option<FailureDetail> {
        match (arrivals_bytes, arrivals_records, total_bytes) {
            (false, false, false) => None,
            (true, false, false) => Some(ARRIVALS_BYTES_EXCEEDED),
            (false, true, false) => Some(ARRIVALS_RECORDS_EXCEEDED),
            (true, true, false) => Some(ARRIVALS_BYTES_AND_RECORDS_EXCEEDED),
            (false, false, true) => Some(TOTAL_BYTES_EXCEEDED),
            (true, false, true) => Some(ARRIVALS_BYTES_AND_TOTAL_BYTES_EXCEEDED),
            (false, true, true) => Some(ARRIVALS_RECORDS_AND_TOTAL_BYTES_EXCEEDED),
            (true, true, true) => Some(ARRIVALS_BYTES_AND_RECORDS_AND_TOTAL_BYTES_EXCEEDED),
        }
    }
}

pub mod preprocess {
    use circular_actors::FailureDetail;

    pub const MAP_TRANSFORM_FAILED: FailureDetail =
        FailureDetail::new("map.transform_failed", None);
    pub const MAP_OUTPUT_SHAPE_UNRESOLVED: FailureDetail =
        FailureDetail::new("map.output_shape_unresolved", None);

    pub const FILTER_NOT_ACCEPTED: FailureDetail = FailureDetail::new("filter.not_accepted", None);
    pub const FILTER_EVALUATION: FailureDetail = FailureDetail::new("filter.evaluation", None);
    pub const FILTER_LOWER: FailureDetail = FailureDetail::new("filter.lower", None);
    pub const FILTER_MODE_MISMATCH: FailureDetail =
        FailureDetail::new("filter.mode_mismatch", None);

    pub const FLATTEN_CONTEXT_NOT_OBJECT: FailureDetail =
        FailureDetail::new("flatten.context_not_object", None);
    pub const FLATTEN_PATH_NOT_ARRAY: FailureDetail =
        FailureDetail::new("flatten.path_not_array", None);
    pub const FLATTEN_ELEMENT_NOT_OBJECT: FailureDetail =
        FailureDetail::new("flatten.element_not_object", None);
    pub const FLATTEN_PATH_UNREPLACEABLE: FailureDetail =
        FailureDetail::new("flatten.path_unreplaceable", None);

    pub const PARSE_FIELD_ABSENT: FailureDetail = FailureDetail::new("parse.field_absent", None);
    pub const PARSE_FIELD_NOT_TEXT: FailureDetail =
        FailureDetail::new("parse.field_not_text", None);
    pub const PARSE_MALFORMED: FailureDetail = FailureDetail::new("parse.malformed", None);
    pub const PARSE_NO_MATCH: FailureDetail = FailureDetail::new("parse.no_match", None);

    #[must_use]
    pub fn map(failure: &circular_actors::MapFailure) -> FailureDetail {
        match failure {
            circular_actors::MapFailure::Evaluation(_) => MAP_TRANSFORM_FAILED,
            circular_actors::MapFailure::OutputShapeNotGround { .. } => MAP_OUTPUT_SHAPE_UNRESOLVED,
        }
    }

    #[must_use]
    pub fn filter(failure: &circular_actors::FilterFailure) -> FailureDetail {
        use circular_expr::eval::EvalError;
        match failure {
            EvalError::NotAccepted(_) => FILTER_NOT_ACCEPTED,
            EvalError::Evaluation(_) => FILTER_EVALUATION,
            EvalError::Lower(_) => FILTER_LOWER,
            EvalError::ModeMismatch { .. } => FILTER_MODE_MISMATCH,
        }
    }

    #[must_use]
    pub fn flatten(failure: circular_actors::flatten::FlattenFailure) -> FailureDetail {
        use circular_actors::flatten::FlattenFailure;
        match failure {
            FlattenFailure::ContextNotObject => FLATTEN_CONTEXT_NOT_OBJECT,
            FlattenFailure::PathNotArray => FLATTEN_PATH_NOT_ARRAY,
            FlattenFailure::ElementNotObject => FLATTEN_ELEMENT_NOT_OBJECT,
            FlattenFailure::PathUnreplaceable => FLATTEN_PATH_UNREPLACEABLE,
        }
    }

    #[must_use]
    pub fn parse(failure: &circular_actors::parse_config::DecodeFailure) -> FailureDetail {
        use circular_actors::parse_config::DecodeFailure;
        match failure {
            DecodeFailure::FieldAbsent => PARSE_FIELD_ABSENT,
            DecodeFailure::FieldNotText => PARSE_FIELD_NOT_TEXT,
            DecodeFailure::Malformed => PARSE_MALFORMED,
            DecodeFailure::NoMatch => PARSE_NO_MATCH,
        }
    }
}

pub mod activation {
    use circular_actors::FailureDetail;

    pub const MISSING_ACTOR: FailureDetail = FailureDetail::new("activation.missing_actor", None);
    pub const UNEXPECTED_ACTOR_TYPE: FailureDetail =
        FailureDetail::new("activation.unexpected_actor_type", None);
    pub const CONFIG_FOLD: FailureDetail = FailureDetail::new("activation.config_fold", None);
    pub const UNEXPECTED_REQUIREMENTS: FailureDetail =
        FailureDetail::new("activation.unexpected_requirements", None);
    pub const CANONICAL_SPEC_MISMATCH: FailureDetail =
        FailureDetail::new("activation.canonical_spec_mismatch", None);
    pub const BUNDLE_ACTOR_MISMATCH: FailureDetail =
        FailureDetail::new("activation.bundle_actor_mismatch", None);
    pub const UNEXPECTED_SOURCE_ARM: FailureDetail =
        FailureDetail::new("activation.unexpected_source_arm", None);
    pub const STALE_INSTANCE_GRANT: FailureDetail =
        FailureDetail::new("activation.stale_instance_grant", None);
    pub const AUTHORED_PROJECTION: FailureDetail =
        FailureDetail::new("activation.authored_projection", None);
    pub const REQUIREMENT_RESOLUTION: FailureDetail =
        FailureDetail::new("activation.requirement_resolution", None);
    pub const CAPABILITY_DENIED: FailureDetail =
        FailureDetail::new("activation.capability_denied", None);
    pub const INSTANCE_ADMISSION: FailureDetail =
        FailureDetail::new("activation.instance_admission", None);
    pub const MISSING_ACTIVATION_WITNESS: FailureDetail =
        FailureDetail::new("activation.missing_activation_witness", None);

    pub const CAPABILITIES_ADMISSION: FailureDetail = FailureDetail::new(
        "activation.config_admission",
        Some(circular_actors::capability_config::FIELD),
    );
    pub const RETRY_ADMISSION: FailureDetail = FailureDetail::new(
        "activation.config_admission",
        Some(circular_actors::retry_config::RETRY_FIELD),
    );
    pub const CONFIG_ADMISSION: FailureDetail =
        FailureDetail::new("activation.config_admission", None);
    pub const EFFECT_EXECUTOR_REGISTRATION: FailureDetail =
        FailureDetail::new("activation.effect_executor_registration", None);
    pub const JOURNAL_UNAVAILABLE: FailureDetail =
        FailureDetail::new("activation.journal_unavailable", None);
}

impl TapPilotBoundaryError {
    #[must_use]
    pub fn detail(&self) -> FailureDetail {
        match self {
            Self::UnexpectedActorType(_) => activation::UNEXPECTED_ACTOR_TYPE,
            Self::ConfigFold(_) => activation::CONFIG_FOLD,
            Self::UnexpectedRequirements => activation::UNEXPECTED_REQUIREMENTS,
            Self::CanonicalSpecMismatch => activation::CANONICAL_SPEC_MISMATCH,
            Self::BundleActorMismatch => activation::BUNDLE_ACTOR_MISMATCH,
            Self::UnexpectedSourceArm => activation::UNEXPECTED_SOURCE_ARM,
            Self::StaleInstanceGrant => activation::STALE_INSTANCE_GRANT,
            Self::Factory(error, _) => error.detail(),
        }
    }
}

impl TapPilotActivationError {
    #[must_use]
    pub fn detail(&self) -> FailureDetail {
        use crate::activation::ActivationError;
        match self {
            Self::MissingActor(_) => activation::MISSING_ACTOR,
            Self::Boundary(error) => error.detail(),
            Self::AuthoredProjection(_) => activation::AUTHORED_PROJECTION,
            Self::Registered(error) => match error {
                ActivationError::RequirementResolution { .. } => activation::REQUIREMENT_RESOLUTION,
                ActivationError::CapabilityDenied { .. } => activation::CAPABILITY_DENIED,
                ActivationError::GrantIssuance { source, .. }
                | ActivationError::FactoryActivation { source, .. } => source.detail(),
            },
            Self::InstanceAdmission(_) => activation::INSTANCE_ADMISSION,
            Self::MissingActivationWitness => activation::MISSING_ACTIVATION_WITNESS,
            Self::Registration(failure) => failure.detail(),
        }
    }
}

