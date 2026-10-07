use crate::config::{PayloadPath, payload_path_from_value};
use crate::{FieldMap, GroundShape, Name, ProductPayload, Shape};
use circular_core::{Boundary, Ceilings, Value};
use circular_runtime::{
    ActorContext, ActorEffects, ActorInput, ActorRestoreError, ActorState, ActorTypes,
    ConfigChangeOutcome, EditableActor, EmittingActor, EmittingActorFactory, EnvelopeResult,
    FoldedConfig,
};
use std::{collections::BTreeMap, fmt, marker::PhantomData};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JoinConfigError(String);
impl fmt::Display for JoinConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "join config.at: {}", self.0)
    }
}
impl std::error::Error for JoinConfigError {}

pub struct JoinActor<V, I> {
    at: PayloadPath,
    state: BTreeMap<String, ProductPayload>,
    marker: PhantomData<fn() -> (V, I)>,
}
impl<V, I> JoinActor<V, I> {
    fn new(config: &Value) -> Result<Self, JoinConfigError> {
        let path = config
            .as_object()
            .and_then(|object| object.get("at"))
            .ok_or_else(|| JoinConfigError("required exact payload path is missing".into()))?;
        let at =
            payload_path_from_value(path).map_err(|error| JoinConfigError(error.to_string()))?;
        Ok(Self {
            at,
            state: BTreeMap::new(),
            marker: PhantomData,
        })
    }
    fn key<'a>(&self, payload: &'a ProductPayload) -> Option<&'a str> {
        crate::route_config::select(payload.value(), &self.at).and_then(Value::as_str)
    }
    fn receive(&mut self, inlet: &str, payload: &ProductPayload) -> ActorEffects<ProductPayload> {
        let key = self.key(payload).map(str::to_owned);
        if inlet == "remove" {
            if let Some(key) = key {
                self.state.remove(&key);
            }
            return ActorEffects::empty();
        }
        let Some(key) = key else {
            return missing(payload);
        };
        match inlet {
            "state" => {
                self.state.insert(key, payload.clone());
                ActorEffects::empty()
            }
            "event" => {
                let Some(state) = self.state.get(&key) else {
                    return missing(payload);
                };
                ActorEffects::emit(
                    port(),
                    ProductPayload::new(
                        GroundShape::try_new(Shape::Object {
                            fields: FieldMap::try_new(vec![
                                (
                                    Name::from_static("event"),
                                    payload.shape().as_shape().clone(),
                                ),
                                (Name::from_static("state"), state.shape().as_shape().clone()),
                            ])
                            .expect("distinct join fields"),
                            open: false,
                        })
                        .expect("payload shapes are ground"),
                        Value::object([
                            ("event", payload.value().clone()),
                            ("state", state.value().clone()),
                        ])
                        .expect("distinct join fields"),
                    ),
                )
            }
            _ => ActorEffects::empty(),
        }
    }
}
fn port() -> circular_core::PortId {
    circular_core::PortId::try_new("event").unwrap()
}
fn missing(payload: &ProductPayload) -> ActorEffects<ProductPayload> {
    ActorEffects::singleton(circular_runtime::ActorEffect::emit_result(
        port(),
        payload.clone(),
        EnvelopeResult::Err {
            reason: circular_runtime::DeadLetterReason::Processing(
                circular_runtime::ProcessingCause::InputOutOfDomain,
            ),
            failure_point: None,
        },
    ))
}
impl<V: Clone + From<u16> + PartialEq, I: Clone + Ord> EditableActor for JoinActor<V, I> {
    type StateVersion = V;
    type EffectId = I;
    fn on_config_change(&mut self, _: &FoldedConfig) -> ConfigChangeOutcome {
        ConfigChangeOutcome::ReplaceIncarnation
    }
    fn checkpoint(&self) -> Option<ActorState<V>> {
        self.try_checkpoint().expect("accepted state table encodes")
    }

    fn try_checkpoint(&self) -> Result<Option<ActorState<V>>, circular_core::CodecError> {
        let table =
            Value::object(self.state.iter().map(|(key, payload)| {
                (key.clone(), crate::payload_value::encode_payload(payload))
            }))
            .expect("unique state keys");
        Ok(Some(ActorState::new(
            V::from(1),
            circular_core::encode(&table, Ceilings::for_boundary(Boundary::ActorState))?
                .into_boxed_slice(),
        )))
    }
    fn restore(&mut self, state: ActorState<V>) -> Result<(), ActorRestoreError<V>> {
        let (schema, bytes) = state.into_parts();
        if schema != V::from(1) {
            return Err(ActorRestoreError::SchemaBeyondLadder { schema });
        }
        let invalid = || ActorRestoreError::DecodeFailed {
            schema: schema.clone(),
        };
        let value = circular_core::decode(&bytes, Ceilings::for_boundary(Boundary::ActorState))
            .map_err(|_| invalid())?;
        let Value::Object(table) = value else {
            return Err(invalid());
        };
        let mut restored = BTreeMap::new();
        for (key, value) in table.into_map() {
            let payload = crate::payload_value::decode_payload(value).ok_or_else(invalid)?;
            if self.key(&payload) != Some(key.as_str()) {
                return Err(invalid());
            }
            restored.insert(key, payload);
        }
        self.state = restored;
        Ok(())
    }
}
impl<T> EmittingActor<T, ProductPayload> for JoinActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        _: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        self.receive(input.inlet().as_str(), input.payload::<T>())
    }
}
pub struct JoinFactory<T>(PhantomData<fn() -> T>);
impl<T> EmittingActorFactory<ProductPayload> for JoinFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
{
    const TYPE: crate::ActorType = crate::ActorType::Join;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Restarts;
    const CHECKPOINTS: bool = true;
    type Grants = T::Grants;
    type Types = T;
    type Instance = JoinActor<T::StateVersion, T::EffectId>;
    type Error = JoinConfigError;
    fn create(config: &FoldedConfig, _: &T::Grants) -> Result<Self::Instance, Self::Error> {
        JoinActor::new(config.value())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_runtime::ActorEffect;
    type Actor = JoinActor<u16, u64>;
    fn actor() -> Actor {
        Actor::new(&Value::object([("at", Value::array([Value::string("key")]))]).unwrap()).unwrap()
    }
    fn payload(key: &str, value: i64) -> ProductPayload {
        ProductPayload::new(
            GroundShape::try_new(Shape::Any).unwrap(),
            Value::object([("key", Value::string(key)), ("value", Value::Int(value))]).unwrap(),
        )
    }
    fn only(effects: &ActorEffects<ProductPayload>) -> (&ProductPayload, &EnvelopeResult) {
        let [
            ActorEffect::Emit {
                port,
                payload,
                result,
                key: None,
            },
        ] = effects.as_slice()
        else {
            panic!("exactly one unkeyed emission")
        };
        assert_eq!(port.as_str(), "event");
        (payload, result)
    }
    #[test]
    fn join_emits_trigger_and_latest_state_for_the_same_key() {
        let mut join = actor();
        for value in [payload("a", 1), payload("b", 99), payload("a", 2)] {
            assert!(join.receive("state", &value).as_slice().is_empty());
        }
        for trigger in [payload("a", 10), payload("a", 11)] {
            let effects = join.receive("event", &trigger);
            let (output, result) = only(&effects);
            assert_eq!(result, &EnvelopeResult::Ok);
            assert_eq!(
                output.value(),
                &Value::object([
                    ("event", trigger.value().clone()),
                    ("state", payload("a", 2).value().clone())
                ])
                .unwrap()
            );
        }
        assert_eq!(join.state.len(), 2);
    }
    #[test]
    fn join_missing_and_retired_reference_emit_err_without_changing_body() {
        let mut join = actor();
        let event = payload("a", 7);
        for phase in 0..2 {
            if phase == 1 {
                join.receive("state", &payload("a", 1));
                join.receive("state", &payload("b", 2));
                join.receive("remove", &event);
                assert!(join.state.contains_key("b"));
            }
            let effects = join.receive("event", &event);
            let (output, result) = only(&effects);
            assert_eq!(output, &event);
            assert_eq!(
                result,
                &EnvelopeResult::Err {
                    reason: circular_runtime::DeadLetterReason::Processing(
                        circular_runtime::ProcessingCause::InputOutOfDomain
                    ),
                    failure_point: None
                }
            );
        }
    }
    #[test]
    fn join_checkpoint_restores_latest_table_and_rejects_invalid_state_atomically() {
        let mut original = actor();
        original.receive("state", &payload("a", 1));
        original.receive("state", &payload("b", 2));
        original.receive("state", &payload("a", 3));
        original.receive("remove", &payload("b", 0));
        let checkpoint = original.checkpoint().unwrap();
        let mut restored = actor();
        restored.restore(checkpoint.clone()).unwrap();
        for key in ["a", "b"] {
            let event = payload(key, 20);
            assert_eq!(
                restored.receive("event", &event),
                original.receive("event", &event)
            );
        }
        let (_, bytes) = checkpoint.into_parts();
        assert!(restored.restore(ActorState::new(2, bytes.clone())).is_err());
        for end in 0..bytes.len() {
            assert!(
                restored
                    .restore(ActorState::new(1, bytes[..end].to_vec().into_boxed_slice()))
                    .is_err()
            );
        }
        assert_eq!(restored.state, original.state);
        let wrong_key = Value::object([(
            "wrong",
            crate::payload_value::encode_payload(&payload("a", 3)),
        )])
        .unwrap();
        assert!(
            restored
                .restore(ActorState::new(
                    1,
                    circular_core::encode(&wrong_key, Ceilings::for_boundary(Boundary::ActorState))
                        .unwrap()
                        .into_boxed_slice()
                ))
                .is_err()
        );
        assert_eq!(restored.state, original.state);
    }
    #[test]
    fn join_at_is_mandatory_and_uses_the_existing_exact_path_domain() {
        assert!(Actor::new(&Value::object([] as [(&str, Value); 0]).unwrap()).is_err());
        assert!(Actor::new(&Value::object([("at", Value::string("key"))]).unwrap()).is_err());
        let join = actor();
        assert_eq!(join.key(&payload("a", 0)), Some("a"));
        let registration = crate::get(crate::ActorType::Join);
        assert_eq!(crate::ActorType::Join.tag(), 63);
        assert!(matches!(
            crate::registration(crate::ActorType::Join).scope(),
            crate::RegistrationScope::Published
        ));
        assert_eq!(registration.ports().fixed().inlets().len(), 3);
    }
}
