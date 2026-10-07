
use crate::ActorType;
use crate::actor_registry::{ProductPayload, ProductValue};
use circular_runtime::{
    ActorContext, ActorEffects, ActorInput, ActorTypes, ConfigChangeOutcome, EditableActor,
    EmittingActor, EmittingActorFactory, FoldedConfig,
};
use std::marker::PhantomData;

fn unmatched_port() -> circular_core::PortId {
    circular_core::PortId::try_new(crate::route_config::UNMATCHED_PORT)
        .expect("registered route unmatched port is canonical")
}

pub struct RouteActor<V, I> {
    at: crate::config::PayloadPath,
    cases: crate::route_config::RouteCases,
    marker: PhantomData<fn() -> (V, I)>,
}

impl<V, I> RouteActor<V, I> {
    #[must_use]
    pub fn new(at: crate::config::PayloadPath, cases: crate::route_config::RouteCases) -> Self {
        Self {
            at,
            cases,
            marker: PhantomData,
        }
    }

    #[must_use]
    pub fn decide(&self, payload: &ProductValue) -> circular_core::PortId {
        crate::route_config::select(payload, &self.at)
            .and_then(|selected| self.cases.match_case(selected))
            .map_or_else(unmatched_port, |case| case.port().clone())
    }
}

impl<V, I> EditableActor for RouteActor<V, I>
where
    V: Clone,
    I: Clone + Ord,
{
    type StateVersion = V;
    type EffectId = I;

    fn on_config_change(&mut self, _config: &FoldedConfig) -> ConfigChangeOutcome {
        ConfigChangeOutcome::ReplaceIncarnation
    }

    fn stateless(&self) -> bool {
        true
    }
}

impl<T> EmittingActor<T, ProductPayload> for RouteActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        let payload = input.payload::<T>();
        ActorEffects::emit(self.decide(payload.value()), payload.clone())
    }
}

pub struct RouteFactory<T>(PhantomData<fn() -> T>);

impl<T> EmittingActorFactory<ProductPayload> for RouteFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
{
    const TYPE: ActorType = ActorType::Route;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Restarts;
    const CHECKPOINTS: bool = false;
    type Grants = T::Grants;
    type Types = T;
    type Instance = RouteActor<T::StateVersion, T::EffectId>;
    type Error = crate::route_config::RouteConfigError;

    fn create(
        config: &circular_runtime::FoldedConfig,
        _grants: &T::Grants,
    ) -> Result<Self::Instance, Self::Error> {
        let config = config.value();
        let (at, cases) = crate::route_config::RouteConfig::from_value(config)?.into_parts();
        Ok(RouteActor::new(at, cases))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor_registry::{ProductActor, ProductFactoryError, product_actor_factory};
    use crate::bang::EmptyConfigFactoryError;

    type TestTypes = circular_testkit::types::TestTypes<ProductPayload>;

    fn empty_config() -> ProductValue {
        ProductValue::object(std::iter::empty::<(String, ProductValue)>())
            .expect("an empty object has no duplicate keys")
    }

    fn folded(actor_type: ActorType, value: ProductValue) -> circular_runtime::FoldedConfig {
        circular_runtime::FoldedConfig::minted(actor_type, value)
    }

    fn route_actor(at: &[ProductValue], cases: &[(&str, ProductValue)]) -> RouteActor<u16, u64> {
        let at = crate::config::payload_path_from_value(&ProductValue::array(at.to_vec()))
            .expect("the test path is valid");
        let cases = crate::route_config::RouteCases::try_new(
            cases.iter().map(|(name, value)| (*name, value.clone())),
        )
        .expect("the test case table is valid");
        RouteActor::new(at, cases)
    }

    fn object(entries: &[(&str, ProductValue)]) -> ProductValue {
        ProductValue::object(entries.iter().map(|(k, v)| ((*k).to_owned(), v.clone())))
            .expect("test keys are unique")
    }

    #[test]
    fn route_sends_a_match_to_its_case_outlet_and_everything_else_to_unmatched() {
        let actor = route_actor(
            &[ProductValue::string("kind")],
            &[
                ("even", ProductValue::string("even")),
                ("odd", ProductValue::string("odd")),
            ],
        );

        for (kind, expected) in [("even", "route_even"), ("odd", "route_odd")] {
            let payload = object(&[("kind", ProductValue::string(kind))]);
            assert_eq!(actor.decide(&payload).as_str(), expected);
        }

        for miss in [
            object(&[("kind", ProductValue::string("other"))]),
            object(&[("other", ProductValue::string("even"))]),
            ProductValue::int(3),
        ] {
            assert_eq!(actor.decide(&miss).as_str(), "unmatched");
        }
    }

    #[test]
    fn an_empty_case_table_routes_everything_to_unmatched() {
        let actor = RouteActor::<u16, u64>::new(
            crate::config::PayloadRoot::path(),
            crate::route_config::RouteCases::empty(),
        );

        for value in [ProductValue::Null, ProductValue::int(0), object(&[])] {
            assert_eq!(actor.decide(&value).as_str(), "unmatched");
        }
    }

    #[test]
    fn a_case_edit_restarts_because_the_port_set_can_change_with_it() {
        let mut actor = route_actor(&[], &[("one", ProductValue::int(1))]);

        assert_eq!(
            actor.on_config_change(&folded(ActorType::Tap, empty_config())),
            ConfigChangeOutcome::ReplaceIncarnation
        );
        assert_eq!(actor.checkpoint(), None);
    }

    #[test]
    fn a_config_that_is_not_a_route_config_rejects_the_activation() {
        let factory = product_actor_factory::<TestTypes>(ActorType::Route).unwrap();

        let Err(error) = factory.create(
            &folded(ActorType::Route, empty_config()),
            &(),
            &crate::ResolvedInletShapes::default(),
        ) else {
            panic!("a config without at is refused");
        };
        assert!(matches!(
            error,
            ProductFactoryError::Route(crate::route_config::RouteConfigError::MissingAt)
        ));

        let bang = product_actor_factory::<TestTypes>(ActorType::Tap).unwrap();
        let Err(error) = bang.create(
            &folded(ActorType::Tap, ProductValue::int(1)),
            &(),
            &crate::ResolvedInletShapes::default(),
        ) else {
            panic!("a non-empty config is refused");
        };
        assert!(matches!(
            error,
            ProductFactoryError::EmptyConfig(EmptyConfigFactoryError::NonEmptyConfig)
        ));
    }
}
