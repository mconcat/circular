
use crate::actor_registry::{ProductPayload, ProductValue};
use crate::bang::{EmptyConfigFactoryError, reject_nonempty_config};
use crate::{ActorType, BaseShape, Shape};
use circular_core::GroundShape;
use circular_runtime::{
    ActorContext, ActorEffects, ActorInput, ActorRestoreError, ActorState, ActorTypes,
    ConfigChangeOutcome, EditableActor, EmittingActor, EmittingActorFactory, FoldedConfig,
};
use std::marker::PhantomData;

fn count_port() -> circular_core::PortId {
    circular_core::PortId::try_new("count").expect("registered counter count port is canonical")
}

pub const COUNTER_STATE_SCHEMA: u16 = 1;

pub struct CounterActor<V, I> {
    count: i64,
    marker: PhantomData<fn() -> (V, I)>,
}

impl<V, I> CounterActor<V, I> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            count: 0,
            marker: PhantomData,
        }
    }

    #[must_use]
    pub const fn count(&self) -> i64 {
        self.count
    }

    pub fn bump(&mut self) -> ProductPayload {
        self.count = self
            .count
            .checked_add(1)
            .expect("count representation exhausted");
        self.payload()
    }

    fn payload(&self) -> ProductPayload {
        ProductPayload::new(
            GroundShape::try_new(Shape::Base(BaseShape::Int))
                .expect("Int count shape contains no variable"),
            ProductValue::int(self.count),
        )
    }

    fn encode(&self) -> Box<[u8]> {
        Box::new(self.count.to_be_bytes())
    }
}

impl<V, I> Default for CounterActor<V, I> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V, I> EditableActor for CounterActor<V, I>
where
    V: Clone + From<u16> + PartialEq,
    I: Clone + Ord,
{
    type StateVersion = V;
    type EffectId = I;

    fn on_config_change(&mut self, _config: &FoldedConfig) -> ConfigChangeOutcome {
        ConfigChangeOutcome::Absorbed
    }

    fn checkpoint(&self) -> Option<ActorState<Self::StateVersion>> {
        Some(ActorState::new(
            V::from(COUNTER_STATE_SCHEMA),
            self.encode(),
        ))
    }

    fn restore(
        &mut self,
        state: ActorState<Self::StateVersion>,
    ) -> Result<(), ActorRestoreError<Self::StateVersion>> {
        let (schema, bytes) = state.into_parts();
        if schema != V::from(COUNTER_STATE_SCHEMA) {
            return Err(ActorRestoreError::SchemaBeyondLadder { schema });
        }
        let Ok(bytes) = <[u8; 8]>::try_from(&*bytes) else {
            return Err(ActorRestoreError::DecodeFailed { schema });
        };
        let count = i64::from_be_bytes(bytes);
        if count < 0 {
            return Err(ActorRestoreError::StateInvariantViolated { schema });
        }
        self.count = count;
        Ok(())
    }
}

impl<T> EmittingActor<T, ProductPayload> for CounterActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
{
    fn on_event(
        &mut self,
        _input: &ActorInput<T::Event>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        ActorEffects::emit(count_port(), self.bump())
    }
}

pub struct CounterFactory<T>(PhantomData<fn() -> T>);

impl<T> EmittingActorFactory<ProductPayload> for CounterFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
{
    const TYPE: ActorType = ActorType::Counter;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Absorbs;
    const CHECKPOINTS: bool = true;
    type Grants = T::Grants;
    type Types = T;
    type Instance = CounterActor<T::StateVersion, T::EffectId>;
    type Error = EmptyConfigFactoryError;

    fn create(
        config: &circular_runtime::FoldedConfig,
        _grants: &T::Grants,
    ) -> Result<Self::Instance, Self::Error> {
        let config = config.value();
        reject_nonempty_config(config)?;
        Ok(CounterActor::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor_registry::{ProductActor, product_actor_factory};

    type TestTypes = circular_testkit::types::TestTypes<ProductPayload>;

    fn empty_config() -> ProductValue {
        ProductValue::object(std::iter::empty::<(String, ProductValue)>())
            .expect("an empty object has no duplicate keys")
    }

    fn folded(actor_type: ActorType, value: ProductValue) -> circular_runtime::FoldedConfig {
        circular_runtime::FoldedConfig::minted(actor_type, value)
    }

    fn counter_actor() -> CounterActor<u16, u64> {
        product_actor_factory::<TestTypes>(ActorType::Counter)
            .expect("counter is boarded")
            .create(
                &folded(ActorType::Counter, empty_config()),
                &(),
                &crate::ResolvedInletShapes::default(),
            )
            .map(|actor| match actor {
                ProductActor::Counter(counter) => counter,
                _ => panic!("counter lookup must build the counter arm"),
            })
            .expect("counter takes an empty config")
    }

    fn state_bytes(count: i64) -> Box<[u8]> {
        Box::new(count.to_be_bytes())
    }

    fn counter_at(count: i64) -> CounterActor<u16, u64> {
        let mut actor = counter_actor();
        actor
            .restore(ActorState::new(COUNTER_STATE_SCHEMA, state_bytes(count)))
            .expect("a reachable total is restored");
        actor
    }

    #[test]
    fn counter_is_the_first_boarded_actor_whose_checkpoint_carries_a_value() {
        let stateless = [ActorType::Tap];
        for kind in stateless {
            let actor = product_actor_factory::<TestTypes>(kind)
                .unwrap()
                .create(
                    &folded(kind, empty_config()),
                    &(),
                    &crate::ResolvedInletShapes::default(),
                )
                .unwrap();
            assert_eq!(actor.checkpoint(), None, "{kind:?} carries no state");
        }

        let boarded = product_actor_factory::<TestTypes>(ActorType::Counter)
            .unwrap()
            .create(
                &folded(ActorType::Counter, empty_config()),
                &(),
                &crate::ResolvedInletShapes::default(),
            )
            .unwrap();
        assert!(boarded.checkpoint().is_some());
    }

    #[test]
    fn count_emissions_keep_literal_int_shape_before_and_after_restore() {
        let int = GroundShape::try_new(Shape::Base(BaseShape::Int)).unwrap();
        let mut actor = counter_actor();
        for expected in [1, 2, 3] {
            let payload = actor.bump();
            assert_eq!(payload.shape(), &int);
            assert_eq!(payload.value(), &ProductValue::Int(expected));
        }

        let mut restored = counter_actor();
        restored.restore(actor.checkpoint().unwrap()).unwrap();
        let payload = restored.bump();
        assert_eq!(payload.shape(), &int);
        assert_eq!(payload.value(), &ProductValue::Int(4));
    }

    #[test]
    fn a_foreign_schema_and_an_unreachable_count_take_different_branches() {
        let mut actor = counter_at(3);

        let foreign = ActorState::new(COUNTER_STATE_SCHEMA + 1, state_bytes(0));
        assert!(matches!(
            actor
                .restore(foreign)
                .expect_err("version outside the ladder"),
            ActorRestoreError::SchemaBeyondLadder { .. }
        ));

        let short = ActorState::new(COUNTER_STATE_SCHEMA, Box::new([0_u8; 4]) as Box<[u8]>);
        assert!(matches!(
            actor
                .restore(short)
                .expect_err("the length does not belong to this schema"),
            ActorRestoreError::DecodeFailed { .. }
        ));

        let negative = ActorState::new(COUNTER_STATE_SCHEMA, state_bytes(-1));
        assert!(matches!(
            actor.restore(negative).expect_err("negative total"),
            ActorRestoreError::StateInvariantViolated { .. }
        ));

        assert_eq!(actor.count(), 3);
    }

    #[test]
    fn a_config_edit_does_not_discard_the_accumulated_prefix() {
        let mut actor = counter_at(5);

        assert_eq!(
            actor.on_config_change(&folded(ActorType::Tap, empty_config())),
            ConfigChangeOutcome::Absorbed
        );
        assert_eq!(actor.count(), 5);
    }
}
