
use crate::actor_support::error_payload;
use crate::config::ConfigRejection;
use crate::{ActorType, Flow, GroundShape, ProductPayload, ProductValue, Shape};
use circular_core::{Boundary, Ceilings, PortId};
use circular_protocol::port_type::{decode_port_shape, encode_port_shape};
use circular_runtime::{
    ActorContext, ActorEffects, ActorInput, ActorRestoreError, ActorState, ActorTypes,
    ConfigChangeOutcome, EditableActor, EmittingActor, EmittingActorFactory, FoldedConfig,
};
use std::marker::PhantomData;

pub const JSON_STATE_SCHEMA: u16 = 1;

fn port(name: &str) -> PortId {
    PortId::try_new(name).expect("registered json port")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JsonFactoryError {
    InvalidConfig,
    Config(ConfigRejection),
}

impl std::fmt::Display for JsonFactoryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str("json config was folded for another actor"),
            Self::Config(rejection) => rejection.fmt(formatter),
        }
    }
}

impl std::error::Error for JsonFactoryError {}

impl From<crate::config::ConfigRejection> for JsonFactoryError {
    fn from(rejection: crate::config::ConfigRejection) -> Self {
        Self::Config(rejection)
    }
}

pub struct JsonActor<V, I> {
    started: bool,
    current: ProductPayload,
    marker: PhantomData<fn() -> (V, I)>,
}

impl<V, I> JsonActor<V, I> {
    #[must_use]
    pub const fn started(&self) -> bool {
        self.started
    }

    #[must_use]
    pub const fn current(&self) -> &ProductPayload {
        &self.current
    }
}

impl<V, I> EditableActor for JsonActor<V, I>
where
    V: Clone + From<u16> + PartialEq,
    I: Clone + Ord,
{
    type StateVersion = V;
    type EffectId = I;

    fn on_config_change(&mut self, config: &FoldedConfig) -> ConfigChangeOutcome {
        match declared_initial(config) {
            Ok(initial) => {
                self.current = any_payload(initial);
                ConfigChangeOutcome::Absorbed
            }
            Err(_) => ConfigChangeOutcome::ReplaceIncarnation,
        }
    }

    fn checkpoint(&self) -> Option<ActorState<V>> {
        self.try_checkpoint()
            .expect("canonical value encoding of the json checkpoint")
    }

    fn try_checkpoint(&self) -> Result<Option<ActorState<V>>, circular_core::CodecError> {
        let flow =
            crate::port_type_from_flow(&Flow::Stream(self.current.shape().as_shape().clone()));
        let shape = encode_port_shape(flow.item()).expect("shape of the accepted payload");
        let value = ProductValue::array([
            ProductValue::Bool(self.started),
            shape,
            self.current.value().clone(),
        ]);
        let bytes = circular_core::encode(&value, Ceilings::for_boundary(Boundary::ActorState))?;
        Ok(Some(ActorState::new(V::from(JSON_STATE_SCHEMA), bytes)))
    }

    fn restore(&mut self, state: ActorState<V>) -> Result<(), ActorRestoreError<V>> {
        let (schema, bytes) = state.into_parts();
        if schema != V::from(JSON_STATE_SCHEMA) {
            return Err(ActorRestoreError::SchemaBeyondLadder { schema });
        }
        let decode_error = || ActorRestoreError::DecodeFailed {
            schema: schema.clone(),
        };
        let value = circular_core::decode(&bytes, Ceilings::for_boundary(Boundary::ActorState))
            .map_err(|_| decode_error())?;
        let ProductValue::Array(fields) = value else {
            return Err(decode_error());
        };
        let [ProductValue::Bool(started), shape, value] = fields.as_slice() else {
            return Err(decode_error());
        };
        let shape = decode_port_shape(shape.clone()).map_err(|_| decode_error())?;
        let shape = crate::types::shape_from_port_type(shape);
        let shape = GroundShape::try_new(shape)
            .map_err(|_| ActorRestoreError::StateInvariantViolated { schema })?;
        self.current = ProductPayload::new(shape, value.clone());
        self.started = *started;
        Ok(())
    }
}

impl<T> EmittingActor<T, ProductPayload> for JsonActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        match input.inlet().as_str() {
            "bang" => {
                self.started = true;
                ActorEffects::emit(port("value"), self.current.clone())
            }
            "set" => {
                self.current = input.payload::<T>().clone();
                ActorEffects::empty()
            }
            _ => ActorEffects::emit(
                port(crate::ERROR_PORT_NAME),
                error_payload("json received an unknown inlet"),
            ),
        }
    }
}

pub struct JsonFactory<T>(PhantomData<fn() -> T>);

impl<T> EmittingActorFactory<ProductPayload> for JsonFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
{
    const TYPE: ActorType = ActorType::Json;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::AbsorbsSome;
    const CHECKPOINTS: bool = true;
    type Grants = T::Grants;
    type Types = T;
    type Instance = JsonActor<T::StateVersion, T::EffectId>;
    type Error = JsonFactoryError;

    fn create(
        config: &FoldedConfig,
        _grants: &Self::Grants,
    ) -> Result<Self::Instance, Self::Error> {
        Ok(JsonActor {
            started: false,
            current: any_payload(declared_initial(config)?),
            marker: PhantomData,
        })
    }
}

fn declared_initial(config: &FoldedConfig) -> Result<ProductValue, JsonFactoryError> {
    let value = config
        .for_type(ActorType::Json)
        .map_err(|_| JsonFactoryError::InvalidConfig)?;
    let schema = crate::registration(ActorType::Json).spec().config();
    let mut fields = schema.open(value)?;
    Ok(schema.raw(&mut fields, "initial")?.clone())
}

fn any_payload(value: ProductValue) -> ProductPayload {
    ProductPayload::new(
        GroundShape::try_new(Shape::Any).expect("Any is a ground shape"),
        value,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::Tick;
    use circular_plan::{
        ActorId, Config, Generation, GenerationVector, Incarnation, Name as PlanName, NamedActorId,
        ScopeId,
    };
    use circular_runtime::ActorEffect;

    use circular_testkit::types::TestRun;

    type TestTypes = circular_testkit::types::TestTypes<ProductPayload>;

    fn config(value: ProductValue) -> FoldedConfig {
        FoldedConfig::minted(ActorType::Json, value)
    }

    fn actor() -> JsonActor<u16, u64> {
        JsonFactory::<TestTypes>::create(
            &config(ProductValue::object([("initial", ProductValue::Int(7))]).unwrap()),
            &(),
        )
        .unwrap()
    }

    fn payload(value: ProductValue) -> ProductPayload {
        ProductPayload::new(
            GroundShape::try_new(Shape::Any).expect("Any is a ground shape"),
            value,
        )
    }

    fn context<'a>(
        actor: &'a ActorId,
        incarnation: &'a Incarnation<TestRun>,
        config: &'a Config,
    ) -> ActorContext<'a, TestRun, ()> {
        ActorContext::new(actor, incarnation, config, &())
    }

    fn drive<Ret>(f: impl FnOnce(&ActorContext<'_, TestRun, ()>) -> Ret) -> Ret {
        let named = NamedActorId::new(ScopeId::root(), PlanName::from_normalized("json"));
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .expect("root generation");
        let incarnation =
            Incarnation::new(TestRun::default(), named.as_scoped().clone(), generations);
        let actor = named.as_actor_id();
        let config = Config::default();
        let context = context(&actor, &incarnation, &config);
        f(&context)
    }

    fn event(
        actor: &mut JsonActor<u16, u64>,
        context: &ActorContext<'_, TestRun, ()>,
        inlet: &str,
        value: ProductValue,
    ) -> ActorEffects<ProductPayload> {
        <JsonActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_event(
            actor,
            &ActorInput::new(port(inlet), payload(value)),
            context,
        )
    }

    fn emitted(effects: &ActorEffects<ProductPayload>) -> &ProductPayload {
        match effects.as_slice() {
            [
                ActorEffect::Emit {
                    port,
                    payload,
                    key,
                    result: circular_runtime::EnvelopeResult::Ok,
                },
            ] => {
                assert_eq!(port.as_str(), "value");
                assert_eq!(*key, None);
                payload
            }
            other => panic!("expected one value emission: {other:?}"),
        }
    }

    #[test]
    fn first_and_second_bang_emit_initial_then_set_changes_the_next_value() {
        drive(|ctx| {
            let mut actor = actor();
            assert!(!actor.started());
            for _ in 0..2 {
                let effects = event(&mut actor, ctx, "bang", ProductValue::Bool(true));
                assert_eq!(emitted(&effects).value(), &ProductValue::Int(7));
                assert!(actor.started());
            }
            assert!(event(&mut actor, ctx, "set", ProductValue::Int(19)).is_empty());
            assert_eq!(
                emitted(&event(&mut actor, ctx, "bang", ProductValue::Null)).value(),
                &ProductValue::Int(19)
            );
        });
    }

    #[test]
    fn set_before_first_bang_keeps_the_start_tag_and_supplies_current() {
        drive(|ctx| {
            let mut actor = actor();
            assert!(event(&mut actor, ctx, "set", ProductValue::Null).is_empty());
            assert!(!actor.started());
            assert_eq!(
                emitted(&event(&mut actor, ctx, "bang", ProductValue::Int(99))).value(),
                &ProductValue::Null
            );
            assert!(actor.started());
        });
    }

    #[test]
    fn checkpoint_round_trip_preserves_started_shape_and_lossless_values() {
        drive(|ctx| {
            for started in [false, true] {
                let mut source = actor();
                let value = ProductValue::array([
                    ProductValue::UInt(u64::MAX),
                    ProductValue::Float(circular_core::FloatValue::new(-0.0)),
                    ProductValue::Float(circular_core::FloatValue::new(f64::NAN)),
                    ProductValue::Bytes(vec![0, 255]),
                    ProductValue::object([("nested", ProductValue::Null)]).unwrap(),
                ]);
                let current = ProductPayload::new(
                    GroundShape::try_new(Shape::Array(Box::new(Shape::Any))).unwrap(),
                    value,
                );
                <JsonActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_event(
                    &mut source,
                    &ActorInput::new(port("set"), current.clone()),
                    ctx,
                );
                if started {
                    event(&mut source, ctx, "bang", ProductValue::Null);
                }
                let checkpoint = source.checkpoint().unwrap();
                let mut restored = actor();
                restored.restore(checkpoint.clone()).unwrap();
                assert_eq!(restored.started(), started);
                assert_eq!(restored.current(), &current);
                assert_eq!(restored.checkpoint(), Some(checkpoint));
                assert_eq!(
                    emitted(&event(&mut restored, ctx, "bang", ProductValue::Null)),
                    &current
                );
            }
        });
    }

    #[test]
    fn invalid_checkpoint_does_not_partially_restore() {
        let mut actor = actor();
        let original = actor.checkpoint().unwrap();
        assert!(matches!(
            actor.restore(ActorState::new(2, vec![])),
            Err(ActorRestoreError::SchemaBeyondLadder { schema: 2 })
        ));
        assert!(matches!(
            actor.restore(ActorState::new(1, vec![255])),
            Err(ActorRestoreError::DecodeFailed { schema: 1 })
        ));
        assert_eq!(actor.checkpoint(), Some(original));
    }

    #[test]
    fn factory_rejects_absent_config_and_initial_but_accepts_any_initial() {
        let create = |value| JsonFactory::<TestTypes>::create(&config(value), &()).map(|_| ());
        assert_eq!(
            create(ProductValue::Null),
            Err(JsonFactoryError::Config(
                crate::config::ConfigRejection::NotObject(circular_core::NotObject {
                    at: circular_core::FieldPath::root(),
                    actual: circular_core::ValueKind::Null,
                })
            ))
        );
        assert_eq!(
            create(ProductValue::object([] as [(&str, ProductValue); 0]).unwrap()),
            Err(JsonFactoryError::Config(
                crate::config::ConfigRejection::Missing("initial")
            ))
        );
        assert_eq!(
            create(ProductValue::object([("initial", ProductValue::Null)]).unwrap()),
            Ok(())
        );
        assert_eq!(
            create(
                ProductValue::object([
                    ("initial", ProductValue::Int(1)),
                    ("extra", ProductValue::Null)
                ])
                .unwrap()
            ),
            Err(JsonFactoryError::Config(
                crate::config::ConfigRejection::Unknown(circular_core::UnknownField {
                    at: circular_core::FieldPath::root(),
                    key: "extra".to_owned(),
                })
            ))
        );
    }

    #[test]
    fn factory_recreation_starts_false_with_authored_initial() {
        drive(|ctx| {
            let mut old = actor();
            event(&mut old, ctx, "set", ProductValue::Int(99));
            event(&mut old, ctx, "bang", ProductValue::Null);
            let mut recreated = actor();
            assert!(!recreated.started());
            assert_eq!(
                emitted(&event(&mut recreated, ctx, "bang", ProductValue::Null)).value(),
                &ProductValue::Int(7)
            );
        });
    }

    #[test]
    fn config_change_restarts_and_unknown_inlet_emits_error_without_mutation() {
        drive(|ctx| {
            let mut actor = actor();
            let checkpoint = actor.checkpoint();
            assert_eq!(
                actor.on_config_change(&config(ProductValue::Null)),
                ConfigChangeOutcome::ReplaceIncarnation
            );
            assert!(
                matches!(event(&mut actor, ctx, "unknown", ProductValue::Null).as_slice(), [ActorEffect::Emit { port, .. }] if port.as_str() == "_error")
            );
            assert_eq!(actor.checkpoint(), checkpoint);
        });
    }
}
