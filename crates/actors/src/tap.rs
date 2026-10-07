
use crate::ActorType;
use crate::actor_registry::ProductPayload;
use crate::bang::{EmptyConfigFactoryError, reject_nonempty_config};
use circular_runtime::{
    ActorContext, ActorEffects, ActorInput, ActorTypes, ConfigChangeOutcome, EditableActor,
    EmittingActor, EmittingActorFactory, FoldedConfig,
};
use std::marker::PhantomData;

fn event_port() -> circular_core::PortId {
    circular_core::PortId::try_new("event").expect("registered tap event port is canonical")
}

pub struct TapActor<V, I> {
    marker: PhantomData<fn() -> (V, I)>,
}

impl<V, I> TapActor<V, I> {
    const fn new() -> Self {
        Self {
            marker: PhantomData,
        }
    }
}

impl<V, I> EditableActor for TapActor<V, I>
where
    V: Clone,
    I: Clone + Ord,
{
    type StateVersion = V;
    type EffectId = I;

    fn on_config_change(&mut self, _config: &FoldedConfig) -> ConfigChangeOutcome {
        ConfigChangeOutcome::Absorbed
    }

    fn stateless(&self) -> bool {
        true
    }
}

impl<T> EmittingActor<T, ProductPayload> for TapActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        ActorEffects::emit(event_port(), input.payload::<T>().clone())
    }
}

pub struct TapFactory<T>(PhantomData<fn() -> T>);

impl<T> EmittingActorFactory<ProductPayload> for TapFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
{
    const TYPE: ActorType = ActorType::Tap;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Absorbs;
    const CHECKPOINTS: bool = false;
    type Grants = T::Grants;
    type Types = T;
    type Instance = TapActor<T::StateVersion, T::EffectId>;
    type Error = EmptyConfigFactoryError;

    fn create(
        config: &circular_runtime::FoldedConfig,
        _grants: &T::Grants,
    ) -> Result<Self::Instance, Self::Error> {
        let config = config.value();
        reject_nonempty_config(config)?;
        Ok(TapActor::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor_registry::{ProductValue, product_actor_factory};

    type TestTypes = circular_testkit::types::TestTypes<ProductPayload>;

    fn empty_config() -> ProductValue {
        ProductValue::object(std::iter::empty::<(String, ProductValue)>())
            .expect("an empty object has no duplicate keys")
    }

    fn folded(actor_type: ActorType, value: ProductValue) -> circular_runtime::FoldedConfig {
        circular_runtime::FoldedConfig::minted(actor_type, value)
    }

    #[test]
    fn tap_builds_a_stateless_actor_from_an_empty_config() {
        let factory = product_actor_factory::<TestTypes>(ActorType::Tap).unwrap();
        let mut actor = factory
            .create(
                &folded(ActorType::Tap, empty_config()),
                &(),
                &crate::ResolvedInletShapes::default(),
            )
            .expect("tap takes an empty config");
        assert_eq!(actor.checkpoint(), None);
        assert_eq!(
            actor.on_config_change(&folded(ActorType::Counter, empty_config())),
            ConfigChangeOutcome::Absorbed
        );
        assert!(factory.spec().requires().is_empty());
    }
}
