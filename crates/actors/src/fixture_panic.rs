
use crate::actor_registry::ProductPayload;
use circular_core::Value;
use circular_runtime::{
    ActorContext, ActorEffects, ActorInput, ActorTypes, ConfigChangeOutcome, EditableActor,
    EmittingActor, EmittingActorFactory, FoldedConfig,
};
use std::marker::PhantomData;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixturePanicConfig {
    panic_after: u64,
}

impl FixturePanicConfig {
    pub const PANIC_AFTER: &'static str = "panic_after";

    pub fn from_value(config: &Value) -> Result<Self, FixturePanicConfigError> {
        let Some(root) = config.as_object() else {
            return Err(FixturePanicConfigError::NotAnObject);
        };
        let after = root
            .get(Self::PANIC_AFTER)
            .ok_or(FixturePanicConfigError::MissingPanicAfter)?
            .as_int()
            .ok_or(FixturePanicConfigError::NotAnInteger)?;
        let after = u64::try_from(after).map_err(|_| FixturePanicConfigError::Negative)?;
        if after == 0 {
            return Err(FixturePanicConfigError::Zero);
        }
        Ok(Self { panic_after: after })
    }

    #[must_use]
    pub const fn panic_after(self) -> u64 {
        self.panic_after
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixturePanicConfigError {
    NotAnObject,
    MissingPanicAfter,
    NotAnInteger,
    Negative,
    Zero,
}

impl core::fmt::Display for FixturePanicConfigError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::NotAnObject => "fixture_panic config value is not an object",
            Self::MissingPanicAfter => "panic_after is missing",
            Self::NotAnInteger => "panic_after is not an integer",
            Self::Negative => "panic_after is negative",
            Self::Zero => "panic_after is 0; there is no 0th arrival",
        })
    }
}

impl std::error::Error for FixturePanicConfigError {}

#[derive(Clone, Copy, Debug)]
pub struct FixturePanicActor<V, I> {
    config: FixturePanicConfig,
    seen: u64,
    _types: PhantomData<fn() -> (V, I)>,
}

impl<V, I> FixturePanicActor<V, I> {
    #[must_use]
    pub const fn new(config: FixturePanicConfig) -> Self {
        Self {
            config,
            seen: 0,
            _types: PhantomData,
        }
    }

    #[must_use]
    pub const fn seen(&self) -> u64 {
        self.seen
    }

    pub fn observe_arrival(&mut self) {
        self.seen += 1;
        assert!(
            self.seen < self.config.panic_after,
            "fixture_panic: deliberate failure at arrival {} (panic_after = {})",
            self.seen,
            self.config.panic_after
        );
    }
}

impl<V, I> EditableActor for FixturePanicActor<V, I>
where
    V: Clone + PartialEq,
    I: Clone + Ord,
{
    type StateVersion = V;
    type EffectId = I;

    fn on_config_change(&mut self, _config: &FoldedConfig) -> ConfigChangeOutcome {
        ConfigChangeOutcome::ReplaceIncarnation
    }
}

impl<T> EmittingActor<T, ProductPayload> for FixturePanicActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: PartialEq,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        self.observe_arrival();
        ActorEffects::emit(out_port(), T::payload(input.event()).clone())
    }
}

pub struct FixturePanicFactory<T>(PhantomData<fn() -> T>);

impl<T> EmittingActorFactory<ProductPayload> for FixturePanicFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: PartialEq,
{
    const TYPE: crate::ActorType = crate::ActorType::FixturePanic;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Restarts;
    const CHECKPOINTS: bool = false;
    type Grants = T::Grants;
    type Types = T;
    type Instance = FixturePanicActor<T::StateVersion, T::EffectId>;
    type Error = FixturePanicConfigError;

    fn create(config: &FoldedConfig, _grants: &T::Grants) -> Result<Self::Instance, Self::Error> {
        FixturePanicConfig::from_value(config.value()).map(FixturePanicActor::new)
    }
}

fn out_port() -> circular_core::PortId {
    circular_core::PortId::try_new("out").expect("registered fixture_panic out port is canonical")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(after: Value) -> Value {
        Value::object([(FixturePanicConfig::PANIC_AFTER.to_owned(), after)])
            .expect("one field has no duplicate")
    }

    #[test]
    fn the_arrival_before_the_declared_one_passes() {
        let decoded = FixturePanicConfig::from_value(&config(Value::int(3))).unwrap();
        let mut actor = FixturePanicActor::<u16, u64>::new(decoded);
        actor.observe_arrival();
        actor.observe_arrival();
        assert_eq!(actor.seen(), 2, "it lives before the declared arrival");
    }

    #[test]
    fn the_declared_arrival_panics() {
        let decoded = FixturePanicConfig::from_value(&config(Value::int(2))).unwrap();
        let mut actor = FixturePanicActor::<u16, u64>::new(decoded);
        actor.observe_arrival();

        let outcome = std::panic::catch_unwind(move || {
            let mut actor = actor;
            actor.observe_arrival();
        });
        assert!(outcome.is_err(), "it must die at the declared arrival");
    }

    #[test]
    fn every_config_rejection_is_its_own_arm() {
        assert_eq!(
            FixturePanicConfig::from_value(&Value::int(1)),
            Err(FixturePanicConfigError::NotAnObject)
        );
        assert_eq!(
            FixturePanicConfig::from_value(&Value::object(Vec::<(String, Value)>::new()).unwrap()),
            Err(FixturePanicConfigError::MissingPanicAfter)
        );
        assert_eq!(
            FixturePanicConfig::from_value(&config(Value::float(2.0))),
            Err(FixturePanicConfigError::NotAnInteger)
        );
        assert_eq!(
            FixturePanicConfig::from_value(&config(Value::int(-1))),
            Err(FixturePanicConfigError::Negative)
        );
        assert_eq!(
            FixturePanicConfig::from_value(&config(Value::int(0))),
            Err(FixturePanicConfigError::Zero)
        );
    }

    #[test]
    fn the_registration_is_fixture_local() {
        assert!(
            matches!(
                crate::registration(crate::ActorType::FixturePanic).scope(),
                crate::RegistrationScope::FixtureLocal(_)
            ),
            "if a panicking element is published, the product catalog carries a deliberate failure"
        );
    }

    #[test]
    fn the_fixture_lookup_still_answers_published_names() {
        struct Types;

        #[derive(Clone, Debug, Eq, Hash, PartialEq)]
        struct Run;
        impl circular_core::StreamIdentity for Run {}

        impl circular_runtime::ActorTypes for Types {
            type Stream = Run;
            type Event = crate::actor_registry::ProductPayload;
            type Payload = crate::actor_registry::ProductPayload;
            type EffectId = u64;
            type StateVersion = u16;
            type Observation = ();
            type Grants = ();

            fn payload(event: &Self::Event) -> &Self::Payload {
                event
            }
        }

        let factory = crate::fixture_actor_factory::<Types>(crate::ActorType::Tap)
            .expect("the fixture lookup also yields the published name");
        assert_eq!(factory.actor_type(), crate::ActorType::Tap);
    }
}
