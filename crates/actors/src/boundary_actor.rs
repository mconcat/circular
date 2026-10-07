
use crate::ActorType;
use crate::actor_registry::ProductPayload;
use circular_runtime::{
    ActorContext, ActorEffect, ActorEffects, ActorInput, ActorTypes, ConfigChangeOutcome,
    EditableActor, EmittingActor, EmittingActorFactory, FoldedConfig,
};
use std::marker::PhantomData;

pub struct BoundaryActor<V, I> {
    marker: PhantomData<fn() -> (V, I)>,
}

impl<V, I> BoundaryActor<V, I> {
    const fn new() -> Self {
        Self {
            marker: PhantomData,
        }
    }
}

impl<V, I> EditableActor for BoundaryActor<V, I>
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

impl<T> EmittingActor<T, ProductPayload> for BoundaryActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        ActorEffects::singleton(ActorEffect::Emit {
            port: input.inlet().clone(),
            payload: input.payload::<T>().clone(),
            key: None,
            result: input.result().clone(),
        })
    }
}

pub trait BoundaryKind {
    const TYPE: ActorType;
}

pub struct InputBoundary;
impl BoundaryKind for InputBoundary {
    const TYPE: ActorType = ActorType::Input;
}

pub struct OutputBoundary;
impl BoundaryKind for OutputBoundary {
    const TYPE: ActorType = ActorType::Output;
}

pub struct FormBoundary;
impl BoundaryKind for FormBoundary {
    const TYPE: ActorType = ActorType::Form;
}

pub struct BoundaryFactory<T, K>(PhantomData<fn() -> (T, K)>);

pub type InputFactory<T> = BoundaryFactory<T, InputBoundary>;
pub type OutputFactory<T> = BoundaryFactory<T, OutputBoundary>;
pub type FormFactory<T> = BoundaryFactory<T, FormBoundary>;

impl<T, K> EmittingActorFactory<ProductPayload> for BoundaryFactory<T, K>
where
    T: ActorTypes<Payload = ProductPayload>,
    K: BoundaryKind,
{
    const TYPE: ActorType = K::TYPE;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Absorbs;
    const CHECKPOINTS: bool = false;
    type Grants = T::Grants;
    type Types = T;
    type Instance = BoundaryActor<T::StateVersion, T::EffectId>;
    type Error = std::convert::Infallible;

    fn create(
        _config: &circular_runtime::FoldedConfig,
        _grants: &T::Grants,
    ) -> Result<Self::Instance, Self::Error> {
        Ok(BoundaryActor::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor_registry::{ProductValue, product_actor_factory};

    use circular_testkit::types::TestRun;

    type TestTypes = circular_testkit::types::TestTypes<ProductPayload>;

    fn label(value: &str) -> circular_runtime::FoldedConfig {
        circular_runtime::FoldedConfig::minted(
            ActorType::Input,
            ProductValue::object([("label", ProductValue::String(value.to_owned()))])
                .expect("one key"),
        )
    }

    #[test]
    fn a_boundary_emits_what_it_consumed_on_the_port_it_arrived_at() {
        let factory = product_actor_factory::<TestTypes>(ActorType::Input)
            .expect("input has a product factory");
        let built = factory
            .create(&label("event"), &(), &crate::ResolvedInletShapes::default())
            .expect("input takes its label config");
        assert_eq!(built.checkpoint(), None);
        let mut actor = BoundaryActor::<u16, u64>::new();
        let port = circular_core::PortId::try_new("_bi1_x").expect("port");
        let payload = ProductPayload::new(
            crate::GroundShape::try_new(crate::Shape::Base(crate::BaseShape::Int)).unwrap(),
            ProductValue::Int(7),
        );
        let failure = circular_runtime::EnvelopeResult::Err {
            reason: circular_runtime::DeadLetterReason::Processing(
                circular_runtime::ProcessingCause::InputOutOfDomain,
            ),
            failure_point: None,
        };
        let input = ActorInput::new(port.clone(), payload.clone()).with_result(failure.clone());
        let incarnation_actor = circular_plan::NamedActorId::new(
            circular_plan::ScopeId::root(),
            circular_plan::Name::from_normalized("event"),
        );
        let generations = circular_plan::GenerationVector::for_actor(
            incarnation_actor.as_scoped(),
            vec![circular_plan::Generation::new(0)].into_boxed_slice(),
        )
        .unwrap();
        let incarnation = circular_plan::Incarnation::new(
            TestRun::default(),
            incarnation_actor.as_scoped().clone(),
            generations,
        );
        let actor_id = incarnation_actor.as_actor_id();
        let config = circular_plan::Config::default();
        let context = ActorContext::new(&actor_id, &incarnation, &config, &());
        let effects =
            <BoundaryActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_event(
                &mut actor, &input, &context,
            );
        assert_eq!(
            effects.as_slice(),
            [ActorEffect::Emit {
                port,
                payload,
                key: None,
                result: failure,
            }]
        );
    }

    #[test]
    fn every_boundary_kind_has_the_one_boundary_factory() {
        for kind in [ActorType::Input, ActorType::Output, ActorType::Form] {
            let factory = product_actor_factory::<TestTypes>(kind).expect("boundary factory");
            assert_eq!(factory.actor_type(), kind);
        }
    }
}
