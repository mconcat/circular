use crate::{ActorType, ProductPayload};
use circular_runtime::{
    ActorContext, ActorEffect, ActorEffects, ActorInput, ActorLifecycle, ActorTypes, EditableActor,
    Effect, EffectOutcome, EmittingActor, EmittingActorFactory, FileReadSpec,
    FilesystemAuthorityBearer, FoldedConfig, NormalizedPath,
};
use std::marker::PhantomData;

pub struct ListenerActor<V, I> {
    path: NormalizedPath,
    started: bool,
    rewind: std::sync::Arc<std::sync::atomic::AtomicBool>,
    marker: PhantomData<fn() -> (V, I)>,
}
impl<V, I> ListenerActor<V, I> {
    pub fn rewind_handle(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        self.rewind.clone()
    }
}
impl<V: Clone, I: Clone + Ord> EditableActor for ListenerActor<V, I> {
    type StateVersion = V;
    type EffectId = I;
}
impl<T> EmittingActor<T, ProductPayload> for ListenerActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Grants: FilesystemAuthorityBearer,
{
    fn on_lifecycle(
        &mut self,
        life: ActorLifecycle,
        context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        if life != ActorLifecycle::Opened {
            return ActorEffects::empty();
        }
        let Some(grant) = context.grants().fs_read_authority() else {
            return failed("Listener FsRead denied".into());
        };
        ActorEffects::external(Effect::file_read(
            grant,
            None,
            FileReadSpec::new(self.path.clone(), None),
        ))
    }

    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        if input.inlet().as_str() == LINE {
            return ActorEffects::singleton(ActorEffect::Emit {
                port: input.inlet().clone(),
                payload: input.payload::<T>().clone(),
                key: None,
                result: input.result().clone(),
            });
        }
        if !self.started {
            self.started = true;
            return ActorEffects::empty();
        }
        match crate::listener::ControlPulse::from_value(input.payload::<T>().value()) {
            Ok(_) => {
                self.rewind
                    .store(true, std::sync::atomic::Ordering::Release);
                ActorEffects::empty()
            }
            Err(error) => failed(error.to_string()),
        }
    }

    fn on_outcome(
        &mut self,
        outcome: &EffectOutcome<T::EffectId>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        match outcome.result() {
            Err(error) => ActorEffects::singleton(ActorEffect::emit_result(
                circular_core::PortId::try_new("_error").unwrap(),
                crate::actor_support::error_payload(error.kind_tag()),
                circular_runtime::EnvelopeResult::Err {
                    reason: circular_runtime::DeadLetterReason::Processing(
                        circular_runtime::classify_effect_failure(error.clone()),
                    ),
                    failure_point: None,
                },
            )),
            Ok(_) => ActorEffects::empty(),
        }
    }
}
const LINE: &str = "line";
fn failed(message: String) -> ActorEffects<ProductPayload> {
    ActorEffects::emit(
        circular_core::PortId::try_new("_error").unwrap(),
        crate::actor_support::error_payload(message),
    )
}
pub struct ListenerFactory<T>(PhantomData<fn() -> T>);
impl<T> EmittingActorFactory<ProductPayload> for ListenerFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Grants: FilesystemAuthorityBearer,
{
    const TYPE: ActorType = ActorType::Listener;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Restarts;
    const CHECKPOINTS: bool = false;
    type Grants = T::Grants;
    type Types = T;
    type Instance = ListenerActor<T::StateVersion, T::EffectId>;
    type Error = crate::listener::ListenerConfigError;
    fn create(
        config: &FoldedConfig,
        _grants: &Self::Grants,
    ) -> Result<Self::Instance, Self::Error> {
        Ok(ListenerActor {
            path: declared_path(config)?,
            started: false,
            rewind: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            marker: PhantomData,
        })
    }
}

fn declared_path(
    config: &FoldedConfig,
) -> Result<NormalizedPath, crate::listener::ListenerConfigError> {
    let value = config
        .for_type(ActorType::Listener)
        .map_err(|_| crate::listener::ListenerConfigError::NotAnObject)?;
    let config = crate::listener::ListenerConfig::from_value(value)?;
    let crate::listener::ListenerSource::FileTail { glob, .. } = config.source();
    NormalizedPath::new(glob.as_ref()).map_err(|_| crate::listener::ListenerConfigError::EmptyGlob)
}

pub(crate) fn judge(
    config: &FoldedConfig,
    _inlets: &crate::inlet_shapes::ResolvedInletShapes,
) -> Result<(), crate::listener::ListenerConfigError> {
    declared_path(config).map(drop)
}
