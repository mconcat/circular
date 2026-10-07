
use crate::actor_support::error_payload;
use crate::config::ConfigRejection;
use crate::{ActorType, BaseShape, GroundShape, ProductPayload, ProductValue, Shape};
use circular_core::PortId;
use circular_runtime::{
    ActorContext, ActorEffects, ActorInput, ActorTypes, ConfigChangeOutcome, EditableActor, Effect,
    EffectOutcome, EmittingActor, EmittingActorFactory, FileReadSpec, FileWriteMode, FileWriteSpec,
    FilesystemAuthorityBearer, FoldedConfig, NormalizedPath, OutcomePayload,
    PathNormalizationError, ProcessingCause,
};
use std::{error::Error, fmt, marker::PhantomData};

fn port(name: &str) -> PortId {
    PortId::try_new(name).expect("registered file port")
}

fn payload(shape: BaseShape, value: ProductValue) -> ProductPayload {
    ProductPayload::new(
        GroundShape::try_new(Shape::Base(shape)).expect("file output is ground"),
        value,
    )
}

fn failed(message: impl Into<String>) -> ActorEffects<ProductPayload> {
    ActorEffects::emit(port("_error"), error_payload(message))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Pending {
    Read,
    Write,
}

pub struct FileActor<V, I> {
    path: NormalizedPath,
    pending: Option<Pending>,
    marker: PhantomData<fn() -> (V, I)>,
}

impl<V, I> FileActor<V, I> {
    fn submit(&mut self, kind: Pending, effect: Effect) -> ActorEffects<ProductPayload> {
        self.pending = Some(kind);
        ActorEffects::external(effect)
    }
}

impl<V: Clone, I: Clone + Ord> EditableActor for FileActor<V, I> {
    type StateVersion = V;
    type EffectId = I;

    fn on_config_change(&mut self, _config: &FoldedConfig) -> ConfigChangeOutcome {
        ConfigChangeOutcome::ReplaceIncarnation
    }
}

impl<T> EmittingActor<T, ProductPayload> for FileActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Grants: FilesystemAuthorityBearer,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        match input.inlet().as_str() {
            "read" => {
                let Some(grant) = context.grants().fs_read_authority() else {
                    return failed("FsRead denied");
                };
                self.submit(
                    Pending::Read,
                    Effect::file_read(grant, None, FileReadSpec::new(self.path.clone(), None)),
                )
            }
            "write" => {
                let Some(grant) = context.grants().fs_write_authority() else {
                    return failed("FsWrite denied");
                };
                let body = match input.payload::<T>().value() {
                    ProductValue::Bytes(bytes) => bytes.clone(),
                    ProductValue::String(text) => text.as_bytes().to_vec(),
                    _ => {
                        return ActorEffects::reject(
                            error_payload("file write must be bytes or string"),
                            ProcessingCause::InputOutOfDomain,
                        );
                    }
                };
                self.submit(
                    Pending::Write,
                    Effect::file_write(
                        grant,
                        None,
                        FileWriteSpec::new(self.path.clone(), body, FileWriteMode::Replace),
                    ),
                )
            }
            _ => failed("unknown file inlet"),
        }
    }

    fn on_outcome(
        &mut self,
        outcome: &EffectOutcome<T::EffectId>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        let pending = self.pending.take();
        debug_assert!(pending.is_some(), "engine must correlate file outcomes");
        let settled = match (pending, outcome.result()) {
            (Some(Pending::Read), Ok(OutcomePayload::FileBytes(bytes))) => ActorEffects::emit(
                port("content"),
                payload(BaseShape::Bytes, ProductValue::Bytes(bytes.to_vec())),
            ),
            (Some(Pending::Write), Ok(OutcomePayload::WrittenLength(length))) => {
                match i64::try_from(*length) {
                    Ok(length) => ActorEffects::emit(
                        port("written"),
                        payload(BaseShape::Int, ProductValue::Int(length)),
                    ),
                    Err(_) => failed("file written length exceeds Int"),
                }
            }
            (_, Err(failure)) => failed(format!("file failed: {}", failure.kind_tag())),
            (_, Ok(other)) => failed(format!(
                "file received mismatched outcome kind {}",
                other.kind_tag()
            )),
        };
        settled
    }

    fn accepts(&self, _inlet: &PortId) -> bool {
        self.pending.is_none()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileFactoryError {
    InvalidConfig,
    Config(ConfigRejection),
    InvalidPathType,
    InvalidPath(PathNormalizationError),
}

impl fmt::Display for FileFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str("file config must be an object for file"),
            Self::Config(rejection) => rejection.fmt(formatter),
            Self::InvalidPathType => formatter.write_str("file path must be a string"),
            Self::InvalidPath(error) => write!(formatter, "file path is invalid: {error}"),
        }
    }
}
impl Error for FileFactoryError {}

impl From<crate::config::ConfigRejection> for FileFactoryError {
    fn from(rejection: crate::config::ConfigRejection) -> Self {
        Self::Config(rejection)
    }
}

pub struct FileFactory<T>(PhantomData<fn() -> T>);

impl<T> EmittingActorFactory<ProductPayload> for FileFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Grants: FilesystemAuthorityBearer,
{
    const TYPE: ActorType = ActorType::File;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Restarts;
    const CHECKPOINTS: bool = false;
    type Grants = T::Grants;
    type Types = T;
    type Instance = FileActor<T::StateVersion, T::EffectId>;
    type Error = FileFactoryError;

    fn create(
        config: &FoldedConfig,
        _grants: &Self::Grants,
    ) -> Result<Self::Instance, Self::Error> {
        Ok(FileActor {
            path: declared_path(config)?,
            pending: None,
            marker: PhantomData,
        })
    }
}

fn declared_path(config: &FoldedConfig) -> Result<NormalizedPath, FileFactoryError> {
    let value = config
        .for_type(ActorType::File)
        .map_err(|_| FileFactoryError::InvalidConfig)?;
    let schema = crate::registration(ActorType::File).spec().config();
    let mut fields = schema.open(value)?;
    let path = schema
        .raw(&mut fields, "path")?
        .as_str()
        .ok_or(FileFactoryError::InvalidPathType)?;
    NormalizedPath::new(path).map_err(FileFactoryError::InvalidPath)
}

pub(crate) fn judge(
    config: &FoldedConfig,
    _inlets: &crate::inlet_shapes::ResolvedInletShapes,
) -> Result<(), FileFactoryError> {
    declared_path(config).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::{StreamIdentity, Tick};
    use circular_plan::{
        Config, Generation, GenerationVector, Incarnation, Name, NamedActorId, ScopeId,
    };
    use circular_runtime::{
        ActorEffect, EffectFailure, FsRead, FsReadGrant, FsWrite, FsWriteGrant, GrantIssuer,
        Granted, PathScope, PathScopes,
    };

    #[derive(Clone, Debug, Eq, Hash, PartialEq)]
    struct Run;
    impl StreamIdentity for Run {}
    struct Grants {
        read: Option<Granted<FsRead>>,
        write: Option<Granted<FsWrite>>,
    }
    impl FilesystemAuthorityBearer for Grants {
        fn fs_read_authority(&self) -> Option<Granted<FsRead>> {
            self.read
        }
        fn fs_write_authority(&self) -> Option<Granted<FsWrite>> {
            self.write
        }
    }
    struct Types;
    impl ActorTypes for Types {
        type Stream = Run;
        type Event = ProductPayload;
        type Payload = ProductPayload;
        type EffectId = u64;
        type StateVersion = u16;
        type Observation = ();
        type Grants = Grants;
        fn payload(event: &Self::Event) -> &Self::Payload {
            event
        }
    }
    struct UnreadTypes;
    impl ActorTypes for UnreadTypes {
        type Stream = Run;
        type Event = ();
        type Payload = ProductPayload;
        type EffectId = u64;
        type StateVersion = u16;
        type Observation = ();
        type Grants = Grants;
        fn payload(_: &()) -> &ProductPayload {
            panic!("read bang inspected payload")
        }
    }
    fn grants() -> Grants {
        let issuer = GrantIssuer::new();
        let scope = PathScopes::new([PathScope::new(NormalizedPath::new("/file-test").unwrap())]);
        Grants {
            read: Some(issuer.issue(&FsReadGrant::fs_read(scope.clone()))),
            write: Some(issuer.issue(&FsWriteGrant::fs_write(scope))),
        }
    }
    fn folded(value: ProductValue) -> FoldedConfig {
        FoldedConfig::minted(ActorType::File, value)
    }
    fn config(path: &str) -> FoldedConfig {
        folded(ProductValue::object([("path", ProductValue::String(path.into()))]).unwrap())
    }
    fn actor(grants: &Grants) -> FileActor<u16, u64> {
        FileFactory::<Types>::create(&config("/file-test/data"), grants).unwrap()
    }
    fn drive<R>(grants: &Grants, f: impl FnOnce(&ActorContext<'_, Run, Grants>) -> R) -> R {
        let named = NamedActorId::new(ScopeId::root(), Name::from_normalized("file"));
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .unwrap();
        let incarnation = Incarnation::new(Run, named.as_scoped().clone(), generations);
        let actor = named.as_actor_id();
        let config = Config::default();
        f(&ActorContext::new(&actor, &incarnation, &config, grants))
    }
    fn event(
        actor: &mut FileActor<u16, u64>,
        ctx: &ActorContext<'_, Run, Grants>,
        inlet: &str,
        value: ProductValue,
    ) -> ActorEffects<ProductPayload> {
        <FileActor<u16, u64> as EmittingActor<Types, ProductPayload>>::on_event(
            actor,
            &ActorInput::new(port(inlet), payload(BaseShape::Null, value)),
            ctx,
        )
    }
    fn settle(
        actor: &mut FileActor<u16, u64>,
        ctx: &ActorContext<'_, Run, Grants>,
        result: Result<OutcomePayload, EffectFailure>,
    ) -> ActorEffects<ProductPayload> {
        <FileActor<u16, u64> as EmittingActor<Types, ProductPayload>>::on_outcome(
            actor,
            &EffectOutcome::new(1, result),
            ctx,
        )
    }
    fn assert_error(effects: &ActorEffects<ProductPayload>) {
        assert!(
            matches!(effects.as_slice(), [ActorEffect::Emit { port, .. }] if port.as_str() == "_error")
        );
    }
    fn assert_out_of_domain(effects: &ActorEffects<ProductPayload>) {
        assert!(matches!(
            effects.as_slice(),
            [ActorEffect::Reject {
                cause: ProcessingCause::InputOutOfDomain,
                ..
            }]
        ));
    }
    #[test]
    fn factory_rejects_missing_nonstring_relative_escape_and_old_sink_config() {
        let grants = grants();
        for value in [
            ProductValue::Null,
            ProductValue::object([] as [(&str, ProductValue); 0]).unwrap(),
            ProductValue::object([("path", ProductValue::Int(1))]).unwrap(),
            ProductValue::object([("path", ProductValue::String("relative".into()))]).unwrap(),
            ProductValue::object([("path", ProductValue::String("/../escape".into()))]).unwrap(),
        ] {
            assert!(FileFactory::<Types>::create(&folded(value), &grants).is_err());
        }
        for key in ["encoding", "mode", "max_bytes", "flush_period"] {
            let value = ProductValue::object([
                ("path", ProductValue::String("/file-test/data".into())),
                (key, ProductValue::String("ignored".into())),
            ])
            .unwrap();
            assert_eq!(
                FileFactory::<Types>::create(&folded(value), &grants).err(),
                Some(FileFactoryError::Config(
                    crate::config::ConfigRejection::Unknown(circular_core::UnknownField {
                        at: circular_core::FieldPath::root(),
                        key: key.to_owned(),
                    })
                ))
            );
        }
        assert!(
            FileFactory::<Types>::create(
                &FoldedConfig::minted(ActorType::Request, ProductValue::Null),
                &grants
            )
            .is_err()
        );
        assert_eq!(
            FileFactory::<Types>::create(&config("/file-test/a/../data"), &grants)
                .unwrap()
                .path
                .as_path(),
            std::path::Path::new("/file-test/data")
        );
    }
    #[test]
    fn bytes_and_utf8_strings_stage_replace_without_touching_the_filesystem() {
        let grants = grants();
        drive(&grants, |ctx| {
            for (value, expected) in [
                (ProductValue::Bytes(vec![0, 255, 1]), vec![0, 255, 1]),
                (
                    ProductValue::String("café".into()),
                    "café".as_bytes().to_vec(),
                ),
                (ProductValue::Bytes(vec![]), vec![]),
            ] {
                let mut actor = actor(&grants);
                let effects = event(&mut actor, ctx, "write", value);
                let [ActorEffect::External(Effect::FileWrite { ticket, spec, .. })] =
                    effects.as_slice()
                else {
                    panic!("one write effect")
                };
                assert_eq!(*ticket, None);
                assert_eq!(spec.mode(), FileWriteMode::Replace);
                assert_eq!(
                    spec.path().as_path(),
                    std::path::Path::new("/file-test/data")
                );
                assert_eq!(spec.body(), expected);
            }
        });
    }
    #[test]
    fn invalid_payload_inlet_and_absent_grants_never_submit_effects() {
        let grants = grants();
        let mut actor = actor(&grants);
        drive(&grants, |ctx| {
            for value in [
                ProductValue::Null,
                ProductValue::Int(1),
                ProductValue::Bool(true),
                ProductValue::Array(vec![]),
            ] {
                assert_out_of_domain(&event(&mut actor, ctx, "write", value));
            }
            assert_error(&event(&mut actor, ctx, "event", ProductValue::Null));
        });
        drive(
            &Grants {
                read: None,
                write: None,
            },
            |ctx| {
                assert_error(&event(&mut actor, ctx, "read", ProductValue::Null));
                assert_error(&event(
                    &mut actor,
                    ctx,
                    "write",
                    ProductValue::Bytes(vec![]),
                ));
            },
        );
        assert_eq!(actor.pending, None);
    }
    #[test]
    fn oversized_written_length_is_not_silently_truncated() {
        let grants = grants();
        let mut actor = actor(&grants);
        drive(&grants, |ctx| {
            event(&mut actor, ctx, "write", ProductValue::Bytes(vec![]));
            assert_error(&settle(
                &mut actor,
                ctx,
                Ok(OutcomePayload::WrittenLength(u64::MAX)),
            ));
            assert_eq!(actor.pending, None);
        });
    }
    #[test]
    fn file_contents_are_not_checkpointed_and_path_edits_restart() {
        let grants = grants();
        let mut actor = actor(&grants);
        assert!(actor.checkpoint().is_none());
        assert_eq!(
            actor.on_config_change(&config("/file-test/other")),
            ConfigChangeOutcome::ReplaceIncarnation
        );
        const { assert!(!<FileFactory<Types> as EmittingActorFactory<ProductPayload>>::CHECKPOINTS) };
    }
}
