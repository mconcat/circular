//! Declarative OTLP mount config. Socket ownership belongs to the engine.
use circular_core::Value;
use std::{fmt, net::SocketAddr};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OtlpConfig {
    listen: SocketAddr,
}

impl OtlpConfig {
    /// Preserve circular-catch config's numeric loopback, nonzero-port domain.
    pub fn from_value(value: &Value) -> Result<Self, OtlpConfigError> {
        let Value::Object(object) = value else {
            return Err(OtlpConfigError);
        };
        if object.iter().count() != 1 {
            return Err(OtlpConfigError);
        }
        let listen = object
            .get("listen")
            .and_then(Value::as_str)
            .ok_or(OtlpConfigError)?
            .parse::<SocketAddr>()
            .map_err(|_| OtlpConfigError)?;
        if !listen.ip().is_loopback() || listen.port() == 0 {
            return Err(OtlpConfigError);
        }
        Ok(Self { listen })
    }

    #[must_use]
    pub const fn listen(&self) -> SocketAddr {
        self.listen
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OtlpConfigError;

impl fmt::Display for OtlpConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OTLP config rejected: listen must be a numeric loopback socket address with a nonzero port, and no other fields are allowed")
    }
}

impl std::error::Error for OtlpConfigError {}

pub const OTLP_SCRUB_UNCLASSIFIABLE: &str = "otlp_scrub_unclassifiable";

/// The request did not reach this Source as a readable HTTP request.
pub const INVALID_REQUEST: &str = "invalid_request";
/// The request did not arrive within the receiver deadline.
pub const TIMEOUT: &str = "timeout";
/// The request's `Content-Type` was not exactly `application/json`.
pub const UNSUPPORTED_CONTENT_TYPE: &str = "unsupported_content_type";
/// The request carried a content or transfer encoding.
pub const UNSUPPORTED_CONTENT_ENCODING: &str = "unsupported_content_encoding";
/// The request body was not JSON.
pub const INVALID_JSON: &str = "invalid_json";
/// The JSON body was not the OTLP shape of its signal.
pub const UNSUPPORTED_SIGNAL_SHAPE: &str = "unsupported_signal_shape";
/// One signal item could not fit the forwarding cap.
pub const UNSPLITTABLE_ITEM: &str = "unsplittable_item";

/// This Source's declared dead-letter vocabulary. `DeclaredReason` has no public
/// constructor, so refusing with a name outside this set cannot be written.
pub static OTLP_DEAD_LETTER_REASONS: std::sync::LazyLock<
    circular_runtime::ReasonDecl<circular_runtime::DeadLettering>,
> = std::sync::LazyLock::new(|| {
    circular_runtime::ReasonDecl::try_from_names([
        OTLP_SCRUB_UNCLASSIFIABLE,
        INVALID_REQUEST,
        TIMEOUT,
        UNSUPPORTED_CONTENT_TYPE,
        UNSUPPORTED_CONTENT_ENCODING,
        INVALID_JSON,
        UNSUPPORTED_SIGNAL_SHAPE,
        UNSPLITTABLE_ITEM,
    ])
    .expect("the OTLP Source declares its refusal reasons once each")
});

pub struct OtlpSource<V, I> {
    marker: std::marker::PhantomData<fn() -> (V, I)>,
}
impl<V, I> OtlpSource<V, I> {
    pub fn create(config: &circular_runtime::FoldedConfig) -> Result<Self, OtlpConfigError> {
        judge(config)?;
        Ok(Self {
            marker: std::marker::PhantomData,
        })
    }
}

pub(crate) fn judge(config: &circular_runtime::FoldedConfig) -> Result<(), OtlpConfigError> {
    if config.actor_type() != crate::ActorType::Otlp {
        return Err(OtlpConfigError);
    }
    OtlpConfig::from_value(config.value()).map(drop)
}
impl<V: Clone, I: Clone + Ord> circular_runtime::EditableActor for OtlpSource<V, I> {
    type StateVersion = V;
    type EffectId = I;
}
impl<T> circular_runtime::EmittingActor<T, crate::ProductPayload>
    for OtlpSource<T::StateVersion, T::EffectId>
where
    T: circular_runtime::ActorTypes<Payload = crate::ProductPayload>,
{
    fn on_event(
        &mut self,
        input: &circular_runtime::ActorInput<T::Event>,
        _context: &circular_runtime::ActorContext<'_, T::Stream, T::Grants>,
    ) -> circular_runtime::ActorEffects<crate::ProductPayload> {
        circular_runtime::ActorEffects::singleton(circular_runtime::ActorEffect::Emit {
            port: input.inlet().clone(),
            payload: input.payload::<T>().clone(),
            key: None,
            result: input.result().clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn registration_golden_preserves_two_named_streams_and_required_listen() {
        use crate::{
            ActorType, Arity, BaseShape, ConfigPath, EffectDeclaration, Flow, Required, Shape,
        };
        let row = crate::registration(ActorType::Otlp);
        let spec = row.spec();
        assert_eq!(ActorType::from_str("otlp"), Some(ActorType::Otlp));
        assert_eq!(ActorType::Otlp.as_str(), "otlp");
        assert_eq!(ActorType::Otlp.tag(), 60);
        assert_eq!(ActorType::Peer.tag(), 51);
        assert_eq!(row.factory(), crate::FactoryArm::Source);
        assert_eq!(spec.display().label().as_str(), "OTLP");
        assert!(spec.requires().is_empty());
        assert_eq!(spec.effect(), &EffectDeclaration::None);
        assert!(spec.ports().dynamic().is_empty());
        assert!(spec.ports().fixed().inlets().is_empty());
        let outlets = spec.ports().fixed().outlets();
        assert_eq!(outlets.len(), 2);
        for (outlet, expected_name) in outlets.iter().zip(["logs", "metrics"]) {
            assert_eq!(outlet.id().as_str(), expected_name);
            assert_eq!(
                outlet.ty(),
                &Flow::Stream(Shape::Object {
                    fields: crate::FieldMap::try_new(Vec::new()).unwrap(),
                    open: true
                })
            );
            assert_eq!(outlet.arity(), Arity::Many);
            assert!(!outlet.primary());
        }
        assert_eq!(spec.config().len(), 1);
        let slot = spec
            .config()
            .get(&ConfigPath::root().join_key("listen"))
            .unwrap();
        assert_eq!(slot.space().shape(), &Shape::Base(BaseShape::String));
        assert_eq!(slot.required(), &Required::Mandatory);
    }

    #[test]
    fn create_input_seals_the_slot_while_mount_parser_checks_socket_semantics() {
        use crate::{ActorType, ConfigPath, admit_registered_create, registered_create_draft};
        let draft = registered_create_draft(ActorType::Otlp).unwrap();
        assert_eq!(
            draft.config(),
            &Value::object([] as [(&str, Value); 0]).unwrap()
        );
        assert_eq!(draft.missing(), &[ConfigPath::root().join_key("listen")]);
        let literal = Value::object([("listen", Value::string("127.0.0.1:4318"))]).unwrap();
        let admitted = admit_registered_create(ActorType::Otlp, &literal).unwrap();
        assert_eq!(admitted.config(), &literal);
        assert!(admitted.ports().inlets().is_empty());
        assert_eq!(
            admitted
                .ports()
                .outlets()
                .iter()
                .map(|p| p.id().as_str())
                .collect::<Vec<_>>(),
            ["logs", "metrics"]
        );
        assert_eq!(
            OtlpConfig::from_value(admitted.config()).unwrap().listen(),
            SocketAddr::from((Ipv4Addr::LOCALHOST, 4318))
        );
        for invalid in [
            draft.config().clone(),
            Value::object([("listen", Value::Int(4318))]).unwrap(),
        ] {
            assert!(admit_registered_create(ActorType::Otlp, &invalid).is_err());
        }
        let invalid_socket = Value::object([("listen", Value::string("0.0.0.0:4318"))]).unwrap();
        let admitted = admit_registered_create(ActorType::Otlp, &invalid_socket).unwrap();
        assert_eq!(
            OtlpConfig::from_value(admitted.config()),
            Err(OtlpConfigError)
        );
    }

    #[test]
    fn listen_preserves_catch_loopback_and_port_boundaries() {
        for (input, expected) in [
            ("127.0.0.1:1", SocketAddr::from((Ipv4Addr::LOCALHOST, 1))),
            (
                "127.2.3.4:65535",
                SocketAddr::from((Ipv4Addr::new(127, 2, 3, 4), 65535)),
            ),
            ("[::1]:4318", SocketAddr::from((Ipv6Addr::LOCALHOST, 4318))),
        ] {
            let config = Value::object([("listen", Value::string(input))]).unwrap();
            assert_eq!(OtlpConfig::from_value(&config).unwrap().listen(), expected);
        }
    }

    #[test]
    fn listen_rejects_names_remote_addresses_zero_and_invalid_ports() {
        for input in [
            "localhost:4318",
            "0.0.0.0:4318",
            "[::]:4318",
            "192.0.2.1:4318",
            "127.0.0.1:0",
            "[::1]:0",
            "127.0.0.1:65536",
            "127.0.0.1:-1",
            "127.0.0.1",
            "",
            " 127.0.0.1:4318",
        ] {
            let config = Value::object([("listen", Value::string(input))]).unwrap();
            assert_eq!(
                OtlpConfig::from_value(&config),
                Err(OtlpConfigError),
                "{input}"
            );
        }
    }

    #[test]
    fn listen_is_required_and_has_no_other_config_fields() {
        for config in [
            Value::Null,
            Value::object([] as [(&str, Value); 0]).unwrap(),
            Value::object([("listen", Value::Int(4318))]).unwrap(),
            Value::object([("otlp_bind", Value::string("127.0.0.1:4318"))]).unwrap(),
            Value::object([
                ("listen", Value::string("127.0.0.1:4318")),
                ("mount", Value::string("extra")),
            ])
            .unwrap(),
        ] {
            assert_eq!(OtlpConfig::from_value(&config), Err(OtlpConfigError));
        }
    }
}
