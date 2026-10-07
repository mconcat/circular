//! `match.event` consumes the envelope result, never a payload tag.
use crate::{BaseShape, FieldMap, GroundShape, Name, ProductPayload, Shape};
use circular_runtime::{
    ActorContext, ActorEffects, ActorInput, ActorTypes, ConfigChangeOutcome, EditableActor,
    EmittingActor, EnvelopeResult, FoldedConfig,
};
use std::marker::PhantomData;

pub(crate) fn reason_shape() -> Shape {
    Shape::Object {
        fields: FieldMap::try_new(vec![
            (Name::from_static("code"), Shape::Base(BaseShape::String)),
            (Name::from_static("detail"), Shape::Any),
        ])
        .expect("existing reason fields are distinct"),
        open: false,
    }
}

pub struct MatchActor<V, I>(PhantomData<fn() -> (V, I)>);
impl<V, I> MatchActor<V, I> {
    pub(crate) const fn new() -> Self {
        Self(PhantomData)
    }
}
impl<V: Clone, I: Clone + Ord> EditableActor for MatchActor<V, I> {
    type StateVersion = V;
    type EffectId = I;
    fn on_config_change(&mut self, _: &FoldedConfig) -> ConfigChangeOutcome {
        ConfigChangeOutcome::Absorbed
    }

    fn stateless(&self) -> bool {
        true
    }
}
impl<T: ActorTypes<Payload = ProductPayload>> EmittingActor<T, ProductPayload>
    for MatchActor<T::StateVersion, T::EffectId>
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        _: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        let (port, payload) = match input.result() {
            EnvelopeResult::Ok => ("ok", input.payload::<T>().clone()),
            EnvelopeResult::Err {
                reason,
                failure_point,
            } => (
                "err",
                ProductPayload::new(
                    GroundShape::try_new(reason_shape()).expect("reason shape is ground"),
                    crate::dead_letter_reason_with_failure_point(reason, failure_point.as_ref()),
                ),
            ),
        };
        ActorEffects::emit(
            circular_core::PortId::try_new(port).expect("proposed match port"),
            payload,
        )
    }
}

/// An effect-free, stateless actor with empty configuration.
pub struct MatchFactory<T>(PhantomData<fn() -> T>);
impl<T: ActorTypes<Payload = ProductPayload>> circular_runtime::EmittingActorFactory<ProductPayload>
    for MatchFactory<T>
{
    const TYPE: crate::ActorType = crate::ActorType::Match;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Absorbs;
    const CHECKPOINTS: bool = false;
    type Grants = T::Grants;
    type Types = T;
    type Instance = MatchActor<T::StateVersion, T::EffectId>;
    type Error = crate::bang::EmptyConfigFactoryError;
    fn create(config: &FoldedConfig, _: &T::Grants) -> Result<Self::Instance, Self::Error> {
        crate::bang::reject_nonempty_config(config.value())?;
        Ok(MatchActor::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::{StreamIdentity, Tick, Value};
    use circular_plan::{
        Config, Generation, GenerationVector, Incarnation, LocalKey, ScopeId, ScopedActorId,
    };
    use circular_runtime::{ActorEffect, DeadLetterReason, ProcessingCause};
    #[derive(Clone, Debug, Eq, Hash, PartialEq)]
    struct Run;
    impl StreamIdentity for Run {}
    struct Types;
    impl ActorTypes for Types {
        type Stream = Run;
        type Event = ProductPayload;
        type Payload = ProductPayload;
        type EffectId = u64;
        type StateVersion = u16;
        type Observation = ();
        type Grants = ();
        fn payload(event: &ProductPayload) -> &ProductPayload {
            event
        }
    }
    #[test]
    fn match_actor_emits_exactly_once_and_consumes_only_the_envelope_tag() {
        let scoped = ScopedActorId::new(
            ScopeId::root(),
            LocalKey::Named(circular_plan::Name::from_normalized("match_test")),
        );
        let generations =
            GenerationVector::for_actor(&scoped, vec![Generation::new(0)].into_boxed_slice())
                .unwrap();
        let id = scoped.as_actor_id();
        let incarnation = Incarnation::new(Run, scoped, generations);
        let config = Config::default();
        let context = ActorContext::new(&id, &incarnation, &config, &());
        let payload = ProductPayload::new(
            GroundShape::try_new(Shape::Any).unwrap(),
            Value::object([("Err", Value::string("payload data"))]).unwrap(),
        );
        let mut actor = MatchActor::<u16, u64>::new();
        for (tag, port_name, expected) in [
            (EnvelopeResult::Ok, "ok", payload.value().clone()),
            (
                EnvelopeResult::Err {
                    reason: DeadLetterReason::Processing(ProcessingCause::InputOutOfDomain),
                    failure_point: None,
                },
                "err",
                Value::object([
                    ("code", Value::string("processing")),
                    ("detail", Value::array([Value::uint(3)])),
                ])
                .unwrap(),
            ),
            (
                EnvelopeResult::Err {
                    reason: DeadLetterReason::DestinationGone,
                    failure_point: None,
                },
                "err",
                Value::object([
                    ("code", Value::string("destination_gone")),
                    ("detail", Value::Null),
                ])
                .unwrap(),
            ),
            (EnvelopeResult::Ok, "ok", payload.value().clone()),
        ] {
            let input = ActorInput::new(
                circular_core::PortId::try_new("event").unwrap(),
                payload.clone(),
            )
            .with_result(tag);
            let effects = <MatchActor<u16, u64> as EmittingActor<Types, ProductPayload>>::on_event(
                &mut actor, &input, &context,
            );
            let [
                ActorEffect::Emit {
                    port,
                    payload: output,
                    key: None,
                    result: circular_runtime::EnvelopeResult::Ok,
                },
            ] = effects.as_slice()
            else {
                panic!("one ordinary emission");
            };
            assert_eq!(port.as_str(), port_name);
            assert_eq!(output.value(), &expected);
            assert!(actor.checkpoint().is_none());
        }
    }
}
