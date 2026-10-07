
use crate::actor_support::error_payload;
use crate::config::ConfigRejection;
use crate::{ActorType, ERROR_PORT_NAME, ProductPayload, ProductValue};
#[cfg(test)]
use circular_core::Tick;
use circular_core::{ObjectValue, PortId};
#[cfg(test)]
use circular_runtime::OutcomePayload;
use circular_runtime::{
    ActorContext, ActorEffects, ActorInput, ActorTypes, AgentToolCall, AgentToolResult,
    ConcreteExternalEffectTag, EditableActor, Effect, EffectOutcome, EmittingActor,
    EmittingActorFactory, FileReadSpec, FileWriteMode, FileWriteSpec, FilesystemAuthorityBearer,
    FoldedConfig, NormalizedPath, ProcessAuthorityBearer, ProcessSpec, ProcessingCause,
    ProgramName, ToolName,
};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;

fn port(name: &str) -> PortId {
    PortId::try_new(name).expect("registered tool executor port names are canonical")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolEffectTemplate {
    FileRead {
        path: NormalizedPath,
    },
    FileWrite {
        path: NormalizedPath,
        mode: FileWriteMode,
    },
    Spawn {
        program: ProgramName,
        arguments: Box<[Box<str>]>,
    },
}

impl ToolEffectTemplate {
    #[must_use]
    pub const fn effect(&self) -> ConcreteExternalEffectTag {
        match self {
            Self::FileRead { .. } => ConcreteExternalEffectTag::FileRead,
            Self::FileWrite { .. } => ConcreteExternalEffectTag::FileWrite,
            Self::Spawn { .. } => ConcreteExternalEffectTag::Spawn,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ToolExecutorConfig {
    tools: BTreeMap<ToolName, ToolEffectTemplate>,
}

impl ToolExecutorConfig {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
        }
    }

    pub fn insert(
        &mut self,
        name: ToolName,
        template: ToolEffectTemplate,
    ) -> Result<(), ToolExecutorFactoryError> {
        if self.tools.insert(name, template).is_some() {
            Err(ToolExecutorFactoryError::DuplicateTool)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolExecutorState {
    Idle,
    Pending {
        call: AgentToolCall,
        effect: ConcreteExternalEffectTag,
    },
}

pub struct ToolExecutorActor<V, I> {
    config: ToolExecutorConfig,
    state: ToolExecutorState,
    marker: PhantomData<fn() -> (V, I)>,
}

impl<V, I> ToolExecutorActor<V, I> {
    #[must_use]
    pub const fn state(&self) -> &ToolExecutorState {
        &self.state
    }
}

impl<V: Clone, I: Clone + Ord> EditableActor for ToolExecutorActor<V, I> {
    type StateVersion = V;
    type EffectId = I;

    fn on_config_change(
        &mut self,
        _config: &FoldedConfig,
    ) -> circular_runtime::ConfigChangeOutcome {
        circular_runtime::ConfigChangeOutcome::ReplaceIncarnation
    }
}

impl<T> EmittingActor<T, ProductPayload> for ToolExecutorActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Grants: FilesystemAuthorityBearer + ProcessAuthorityBearer,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        let Some(call) = crate::agent_tool_wire::decode_tool_call(input.payload::<T>()) else {
            return ActorEffects::reject(
                error_payload("invalid tool call"),
                ProcessingCause::InputOutOfDomain,
            );
        };
        let Some(template) = self.config.tools.get(call.tool()) else {
            return ActorEffects::reject(
                error_payload("tool is not allowlisted"),
                ProcessingCause::DomainRejected,
            );
        };
        if !matches!(self.state, ToolExecutorState::Idle) {
            return ActorEffects::reject(
                error_payload("tool call already active"),
                ProcessingCause::DomainRejected,
            );
        }
        let external = match template {
            ToolEffectTemplate::FileRead { path } => {
                let Some(grant) = context.grants().fs_read_authority() else {
                    return ActorEffects::emit(
                        port(ERROR_PORT_NAME),
                        error_payload("FsRead denied"),
                    );
                };
                Effect::file_read(grant, None, FileReadSpec::new(path.clone(), None))
            }
            ToolEffectTemplate::FileWrite { path, mode } => {
                let Some(grant) = context.grants().fs_write_authority() else {
                    return ActorEffects::emit(
                        port(ERROR_PORT_NAME),
                        error_payload("FsWrite denied"),
                    );
                };
                Effect::file_write(
                    grant,
                    None,
                    FileWriteSpec::new(path.clone(), call.arguments().as_bytes().to_vec(), *mode),
                )
            }
            ToolEffectTemplate::Spawn { program, arguments } => {
                let Some(grant) = context.grants().process_spawn_authority() else {
                    return ActorEffects::emit(
                        port(ERROR_PORT_NAME),
                        error_payload("ProcessSpawn denied"),
                    );
                };
                Effect::spawn(
                    grant,
                    None,
                    ProcessSpec::new(
                        program.clone(),
                        arguments.iter().map(AsRef::as_ref),
                        call.arguments().as_bytes().to_vec(),
                    ),
                )
            }
        };
        self.state = ToolExecutorState::Pending {
            call,
            effect: template.effect(),
        };
        ActorEffects::external(external)
    }

    fn accepts(&self, _inlet: &PortId) -> bool {
        matches!(self.state, ToolExecutorState::Idle)
    }

    fn on_outcome(
        &mut self,
        outcome: &EffectOutcome<T::EffectId>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        let ToolExecutorState::Pending { call, effect } = &self.state else {
            return ActorEffects::emit(port(ERROR_PORT_NAME), error_payload("outcome while idle"));
        };
        let result =
            match AgentToolResult::from_outcome(call.id().clone(), *effect, outcome.result()) {
                Ok(result) => result,
                Err(_) => {
                    self.state = ToolExecutorState::Idle;
                    return ActorEffects::reject(
                        error_payload("mismatched outcome"),
                        ProcessingCause::DomainRejected,
                    );
                }
            };
        self.state = ToolExecutorState::Idle;
        ActorEffects::emit(
            port("result"),
            crate::agent_tool_wire::tool_result_payload(&result),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolExecutorFactoryError {
    InvalidConfig,
    Config(ConfigRejection),
    DuplicateTool,
    InvalidTool,
    InvalidTemplate,
}

impl fmt::Display for ToolExecutorFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(rejection) => rejection.fmt(formatter),
            Self::InvalidConfig => formatter.write_str(
                "tool_executor config is not a tool_executor config, or its tools are not an object",
            ),
            Self::DuplicateTool => formatter.write_str("tool_executor declares a tool twice"),
            Self::InvalidTool => {
                formatter.write_str("tool_executor tool is outside the registered tool schema")
            }
            Self::InvalidTemplate => {
                formatter.write_str("tool_executor tool template is not a valid template")
            }
        }
    }
}
impl Error for ToolExecutorFactoryError {}

impl From<ConfigRejection> for ToolExecutorFactoryError {
    fn from(rejection: ConfigRejection) -> Self {
        Self::Config(rejection)
    }
}

pub struct ToolExecutorFactory<T>(PhantomData<fn() -> T>);

impl<T> EmittingActorFactory<ProductPayload> for ToolExecutorFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Grants: FilesystemAuthorityBearer + ProcessAuthorityBearer,
{
    const TYPE: circular_core::ActorType = circular_core::ActorType::ToolExecutor;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Restarts;
    const CHECKPOINTS: bool = false;
    type Grants = T::Grants;
    type Types = T;
    type Instance = ToolExecutorActor<T::StateVersion, T::EffectId>;
    type Error = ToolExecutorFactoryError;

    fn create(
        config: &FoldedConfig,
        _grants: &Self::Grants,
    ) -> Result<Self::Instance, Self::Error> {
        Ok(ToolExecutorActor {
            config: declared_tools(config)?,
            state: ToolExecutorState::Idle,
            marker: PhantomData,
        })
    }
}

fn declared_tools(config: &FoldedConfig) -> Result<ToolExecutorConfig, ToolExecutorFactoryError> {
    let value = config
        .for_type(ActorType::ToolExecutor)
        .map_err(|_| ToolExecutorFactoryError::InvalidConfig)?;
    let schema = crate::registration(ActorType::ToolExecutor).spec().config();
    let mut fields = schema.open(value)?;
    let tools = schema
        .raw(&mut fields, "tools")?
        .as_object()
        .ok_or(ToolExecutorFactoryError::InvalidConfig)?;
    let mut parsed = ToolExecutorConfig::new();
    for (name, value) in tools.iter() {
        let tool = ToolName::try_from_normalized(name.to_owned())
            .map_err(|_| ToolExecutorFactoryError::InvalidTool)?;
        let value = value
            .as_object()
            .ok_or(ToolExecutorFactoryError::InvalidTemplate)?;
        let effect = value
            .get("effect")
            .and_then(ProductValue::as_str)
            .ok_or(ToolExecutorFactoryError::InvalidTemplate)?;
        let template = match effect {
            "file_read" => ToolEffectTemplate::FileRead {
                path: template_path(value)?,
            },
            "file_write" => {
                let mode = match value.get("mode").and_then(ProductValue::as_str) {
                    Some("create") => FileWriteMode::Create,
                    Some("replace") => FileWriteMode::Replace,
                    _ => return Err(ToolExecutorFactoryError::InvalidTemplate),
                };
                ToolEffectTemplate::FileWrite {
                    path: template_path(value)?,
                    mode,
                }
            }
            "spawn" => ToolEffectTemplate::Spawn {
                program: ProgramName::from_normalized(
                    value
                        .get("program")
                        .and_then(ProductValue::as_str)
                        .ok_or(ToolExecutorFactoryError::InvalidTemplate)?,
                ),
                arguments: value
                    .get("arguments")
                    .and_then(ProductValue::as_array)
                    .ok_or(ToolExecutorFactoryError::InvalidTemplate)?
                    .iter()
                    .map(|argument| {
                        argument
                            .as_str()
                            .map(|argument| Box::<str>::from(argument.to_owned()))
                            .ok_or(ToolExecutorFactoryError::InvalidTemplate)
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .into_boxed_slice(),
            },
            _ => return Err(ToolExecutorFactoryError::InvalidTemplate),
        };
        parsed.insert(tool, template)?;
    }
    Ok(parsed)
}

pub(crate) fn judge(
    config: &FoldedConfig,
    _inlets: &crate::inlet_shapes::ResolvedInletShapes,
) -> Result<(), ToolExecutorFactoryError> {
    declared_tools(config).map(drop)
}

fn template_path(value: &ObjectValue) -> Result<NormalizedPath, ToolExecutorFactoryError> {
    value
        .get("path")
        .and_then(ProductValue::as_str)
        .ok_or(ToolExecutorFactoryError::InvalidTemplate)
        .and_then(|path| {
            NormalizedPath::new(path).map_err(|_| ToolExecutorFactoryError::InvalidTemplate)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_tool_wire::tool_call_payload;
    use circular_plan::{
        ActorId, Config, Generation, GenerationVector, Incarnation, Name as PlanName, NamedActorId,
        ScopeId,
    };
    use circular_runtime::{
        ActorEffect, Ceiling, CpuTime, FsRead, FsReadGrant, FsWrite, FsWriteGrant, Granted,
        MemoryBytes, PathScope, PathScopes, ProcessCount, ProcessSpawn, ProcessTargets,
        ResourceCeilings, WorkspaceProcessGrant,
    };
    use circular_runtime::{AgentPayload, AgentToolCallId};

    use circular_testkit::types::TestRun;

    struct TestGrants {
        read: Granted<FsRead>,
        write: Granted<FsWrite>,
        process: Granted<ProcessSpawn>,
    }
    impl FilesystemAuthorityBearer for TestGrants {
        fn fs_read_authority(&self) -> Option<Granted<FsRead>> {
            Some(self.read)
        }

        fn fs_write_authority(&self) -> Option<Granted<FsWrite>> {
            Some(self.write)
        }
    }

    impl ProcessAuthorityBearer for TestGrants {
        fn process_spawn_authority(&self) -> Option<Granted<ProcessSpawn>> {
            Some(self.process)
        }
    }

    type TestTypes = circular_testkit::types::TestTypes<ProductPayload, TestGrants>;

    fn grants(root: &NormalizedPath) -> TestGrants {
        let paths = PathScopes::new([PathScope::new(root.clone())]);
        let read = FsReadGrant::fs_read(paths.clone());
        let write = FsWriteGrant::fs_write(paths.clone());
        let process = WorkspaceProcessGrant::workspace_process(
            ProcessTargets::default(),
            paths,
            ResourceCeilings::new(
                Ceiling::<CpuTime>::Unlimited,
                Ceiling::<MemoryBytes>::Unlimited,
                Ceiling::<ProcessCount>::Unlimited,
            ),
        );
        let issuer = circular_runtime::GrantIssuer::new();
        TestGrants {
            read: issuer.issue(&read),
            write: issuer.issue(&write),
            process: issuer.issue(&process),
        }
    }

    fn context<'a>(
        actor: &'a ActorId,
        incarnation: &'a Incarnation<TestRun>,
        config: &'a Config,
        grants: &'a TestGrants,
    ) -> ActorContext<'a, TestRun, TestGrants> {
        ActorContext::new(actor, incarnation, config, grants)
    }

    #[test]
    fn parameter_denied_tool_result_preserves_the_failure_class() {
        let root = NormalizedPath::new("/tool-fixture/allowed").expect("absolute root");
        let grants = grants(&root);
        let tool = ToolName::try_from_normalized("write-note").expect("nonempty tool");
        let mut tools = ToolExecutorConfig::new();
        tools
            .insert(
                tool.clone(),
                ToolEffectTemplate::FileWrite {
                    path: NormalizedPath::new("/tool-fixture/denied.txt").expect("absolute path"),
                    mode: FileWriteMode::Replace,
                },
            )
            .expect("unique tool");
        let mut actor = ToolExecutorActor::<u16, u64> {
            config: tools,
            state: ToolExecutorState::Idle,
            marker: PhantomData,
        };
        let named = NamedActorId::new(ScopeId::root(), PlanName::from_normalized("tools"));
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .expect("root generation");
        let incarnation =
            Incarnation::new(TestRun::default(), named.as_scoped().clone(), generations);
        let actor_id = named.as_actor_id();
        let config = Config::default();
        let context = context(&actor_id, &incarnation, &config, &grants);
        let call = AgentToolCall::new(
            AgentToolCallId::try_from_bytes(b"rollback".to_vec()).expect("nonempty id"),
            tool,
            AgentPayload::new(b"denied write".to_vec()),
        );
        let input = ActorInput::new(port("call"), tool_call_payload(&call));
        let submitted =
            <ToolExecutorActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_event(
                &mut actor, &input, &context,
            );
        let [ActorEffect::External(effect)] = submitted.as_slice() else {
            panic!("one tool effect must be submitted");
        };
        let paths = PathScopes::new([PathScope::new(root)]);
        let mut interpreter = circular_runtime::LiveInterpreter::new(
            &FsReadGrant::fs_read(paths.clone()),
            &FsWriteGrant::fs_write(paths),
        );
        interpreter.submit(1, effect).expect("first effect");
        let outcome = interpreter.next_outcome().expect("denied effect settles");
        assert_eq!(
            outcome.result(),
            &Err(circular_runtime::EffectFailure::ParameterDenied {
                capability: circular_runtime::Capability::FsWrite,
            })
        );
        assert_eq!(interpreter.external_contacts(), 0);
        let completed =
            <ToolExecutorActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_outcome(
                &mut actor, &outcome, &context,
            );
        let [ActorEffect::Emit { port, payload, .. }] = completed.as_slice() else {
            panic!("the denied tool call must emit one result");
        };
        assert_eq!(port.as_str(), "result");
        let result = payload.value().as_object().expect("tool result object");
        assert_eq!(
            result.get("call"),
            Some(&ProductValue::Bytes(b"rollback".to_vec()))
        );
        assert_eq!(result.get("ok"), Some(&ProductValue::Bool(false)));
        assert_eq!(
            result.get("value").and_then(ProductValue::as_str),
            Some("parameter_denied")
        );
        assert_eq!(result.get("detail"), Some(&ProductValue::UInt(5)));
        let decoded = crate::agent_tool_wire::decode_tool_result(payload)
            .expect("the agent can read the failed tool result");
        assert_eq!(
            decoded.result(),
            &Err(circular_runtime::EffectFailure::ParameterDenied {
                capability: circular_runtime::Capability::FsWrite,
            })
        );
    }

    #[test]
    fn diverged_tool_result_preserves_both_replay_failures() {
        use circular_runtime::{
            Divergence, EffectFailure, EffectTerm, RecordedOutcome, VirtualizedInterpreter,
        };

        let root = NormalizedPath::new("/tool-fixture").expect("absolute root");
        let grants = grants(&root);
        let tool = ToolName::try_from_normalized("read-note").expect("nonempty tool");
        let mut tools = ToolExecutorConfig::new();
        tools
            .insert(
                tool.clone(),
                ToolEffectTemplate::FileRead { path: root.clone() },
            )
            .expect("unique tool");
        let mut actor = ToolExecutorActor::<u16, u64> {
            config: tools,
            state: ToolExecutorState::Idle,
            marker: PhantomData,
        };
        let named = NamedActorId::new(ScopeId::root(), PlanName::from_normalized("tools"));
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .expect("root generation");
        let incarnation =
            Incarnation::new(TestRun::default(), named.as_scoped().clone(), generations);
        let actor_id = named.as_actor_id();
        let config = Config::default();
        let context = context(&actor_id, &incarnation, &config, &grants);
        let call = AgentToolCall::new(
            AgentToolCallId::try_from_bytes(b"replayed-call".to_vec()).expect("nonempty id"),
            tool,
            AgentPayload::new(Vec::new()),
        );
        let input = ActorInput::new(port("call"), tool_call_payload(&call));

        for (records, divergence, detail) in [
            (vec![], Divergence::MissingRecord, 1),
            (
                vec![RecordedOutcome::new(
                    EffectTerm::FileWrite {
                        ticket: None,
                        spec: FileWriteSpec::new(root.clone(), vec![], FileWriteMode::Replace),
                    },
                    EffectOutcome::new(1, Ok(OutcomePayload::WrittenLength(0))),
                )],
                Divergence::EffectMismatch,
                2,
            ),
        ] {
            let submitted = <ToolExecutorActor<u16, u64> as EmittingActor<
                TestTypes,
                ProductPayload,
            >>::on_event(&mut actor, &input, &context);
            let [ActorEffect::External(effect)] = submitted.as_slice() else {
                panic!("one tool effect must be submitted");
            };
            let mut replay = VirtualizedInterpreter::try_new(records).expect("unique records");
            replay.submit(1, effect).expect("first effect");
            let outcome = replay.next_outcome().expect("replay failure settles");
            assert_eq!(outcome.result(), &Err(EffectFailure::Diverged(divergence)));
            let completed = <ToolExecutorActor<u16, u64> as EmittingActor<
                TestTypes,
                ProductPayload,
            >>::on_outcome(&mut actor, &outcome, &context);
            let [ActorEffect::Emit { port, payload, .. }] = completed.as_slice() else {
                panic!("the divergent tool call must emit one result");
            };
            assert_eq!(port.as_str(), "result");
            let result = payload.value().as_object().expect("tool result object");
            assert_eq!(result.get("ok"), Some(&ProductValue::Bool(false)));
            assert_eq!(
                result.get("value").and_then(ProductValue::as_str),
                Some("diverged")
            );
            assert_eq!(result.get("detail"), Some(&ProductValue::UInt(detail)));
            let decoded = crate::agent_tool_wire::decode_tool_result(payload)
                .expect("the agent can read the replay failure");
            assert_eq!(decoded.call(), call.id());
            assert_eq!(decoded.result(), &Err(EffectFailure::Diverged(divergence)));
        }
    }

    #[test]
    fn one_allowlisted_call_creates_one_effect_and_one_result() {
        let root = NormalizedPath::new("/tool-fixture").expect("absolute root");
        let path = NormalizedPath::new("/tool-fixture/output").expect("absolute path");
        let grants = grants(&root);
        let folded = FoldedConfig::minted(
            ActorType::ToolExecutor,
            ProductValue::object([(
                "tools",
                ProductValue::object([(
                    "write-note",
                    ProductValue::object([
                        ("effect", ProductValue::String("file_write".to_owned())),
                        (
                            "path",
                            ProductValue::String(path.as_path().to_string_lossy().into_owned()),
                        ),
                        ("mode", ProductValue::String("replace".to_owned())),
                    ])
                    .expect("template fields are unique"),
                )])
                .expect("tool names are unique"),
            )])
            .expect("config fields are unique"),
        );
        let mut actor =
            <ToolExecutorFactory<TestTypes> as EmittingActorFactory<ProductPayload>>::create(
                &folded, &grants,
            )
            .expect("file-write template parses");
        let named = NamedActorId::new(ScopeId::root(), PlanName::from_normalized("tools"));
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .expect("root generation");
        let incarnation =
            Incarnation::new(TestRun::default(), named.as_scoped().clone(), generations);
        let actor_id = named.as_actor_id();
        let raw_config = Config::default();
        let context = context(&actor_id, &incarnation, &raw_config, &grants);
        let call = AgentToolCall::new(
            AgentToolCallId::try_from_bytes(b"call-1".to_vec()).expect("nonempty id"),
            ToolName::try_from_normalized("write-note").expect("nonempty tool"),
            AgentPayload::new(b"hello".to_vec()),
        );
        let input = ActorInput::new(port("call"), tool_call_payload(&call));

        let submitted =
            <ToolExecutorActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_event(
                &mut actor, &input, &context,
            );
        assert!(matches!(
            submitted.as_slice(),
            [ActorEffect::External(Effect::FileWrite { spec, .. })]
                if spec.path() == &path && spec.body() == b"hello"
        ));
        assert!(matches!(actor.state(), ToolExecutorState::Pending { .. }));

        let overlapping = <ToolExecutorActor<u16, u64> as EmittingActor<
            TestTypes,
            ProductPayload,
        >>::on_event(&mut actor, &input, &context);
        assert!(matches!(
            overlapping.as_slice(),
            [ActorEffect::Reject {
                cause: ProcessingCause::DomainRejected,
                ..
            }]
        ));
        assert!(matches!(actor.state(), ToolExecutorState::Pending { .. }));

        let completed =
            <ToolExecutorActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_outcome(
                &mut actor,
                &EffectOutcome::new(1, Ok(OutcomePayload::WrittenLength(5))),
                &context,
            );
        assert!(matches!(actor.state(), ToolExecutorState::Idle));
        assert!(
            matches!(completed.as_slice(), [ActorEffect::Emit { port, .. }] if port.as_str() == "result")
        );

        let resubmitted = <ToolExecutorActor<u16, u64> as EmittingActor<
            TestTypes,
            ProductPayload,
        >>::on_event(&mut actor, &input, &context);
        assert!(matches!(
            resubmitted.as_slice(),
            [ActorEffect::External(Effect::FileWrite { .. })]
        ));
        let mismatched =
            <ToolExecutorActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_outcome(
                &mut actor,
                &EffectOutcome::new(2, Ok(OutcomePayload::FileBytes(Box::from(&b"wrong"[..])))),
                &context,
            );
        assert!(matches!(actor.state(), ToolExecutorState::Idle));
        assert!(matches!(
            mismatched.as_slice(),
            [ActorEffect::Reject {
                cause: ProcessingCause::DomainRejected,
                ..
            }]
        ));

        let unknown = AgentToolCall::new(
            AgentToolCallId::try_from_bytes(b"call-2".to_vec()).expect("nonempty id"),
            ToolName::try_from_normalized("not-allowed").expect("nonempty tool"),
            AgentPayload::new(Vec::new()),
        );
        let unknown = ActorInput::new(port("call"), tool_call_payload(&unknown));
        let rejected =
            <ToolExecutorActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_event(
                &mut actor, &unknown, &context,
            );
        assert!(matches!(actor.state(), ToolExecutorState::Idle));
        assert!(matches!(
            rejected.as_slice(),
            [ActorEffect::Reject {
                cause: ProcessingCause::DomainRejected,
                ..
            }]
        ));
    }

    #[test]
    fn spawn_template_preserves_exact_program_static_arguments_and_call_stdin() {
        let root = NormalizedPath::new("/tool-fixture").expect("absolute root");
        let grants = grants(&root);
        let folded = FoldedConfig::minted(
            ActorType::ToolExecutor,
            ProductValue::object([(
                "tools",
                ProductValue::object([(
                    "remediate",
                    ProductValue::object([
                        ("effect", ProductValue::String("spawn".to_owned())),
                        (
                            "program",
                            ProductValue::String("/opt/oncall/remediate".to_owned()),
                        ),
                        (
                            "arguments",
                            ProductValue::Array(
                                ["--mode", "safe"]
                                    .into_iter()
                                    .map(|value| ProductValue::String(value.to_owned()))
                                    .collect(),
                            ),
                        ),
                    ])
                    .expect("template fields are unique"),
                )])
                .expect("tool names are unique"),
            )])
            .expect("config fields are unique"),
        );
        let mut actor =
            <ToolExecutorFactory<TestTypes> as EmittingActorFactory<ProductPayload>>::create(
                &folded, &grants,
            )
            .expect("spawn template parses");
        let named = NamedActorId::new(ScopeId::root(), PlanName::from_normalized("tools"));
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .expect("root generation");
        let incarnation =
            Incarnation::new(TestRun::default(), named.as_scoped().clone(), generations);
        let actor_id = named.as_actor_id();
        let raw_config = Config::default();
        let context = context(&actor_id, &incarnation, &raw_config, &grants);
        let call = AgentToolCall::new(
            AgentToolCallId::try_from_bytes(b"call-process".to_vec()).expect("nonempty id"),
            ToolName::try_from_normalized("remediate").expect("nonempty tool"),
            AgentPayload::new(b"incident-42".to_vec()),
        );

        let submitted =
            <ToolExecutorActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_event(
                &mut actor,
                &ActorInput::new(port("call"), tool_call_payload(&call)),
                &context,
            );

        assert!(matches!(
            submitted.as_slice(),
            [ActorEffect::External(Effect::Spawn { spec, .. })]
                if spec.program().as_str() == "/opt/oncall/remediate"
                    && spec.arguments().collect::<Vec<_>>() == ["--mode", "safe"]
                    && spec.stdin() == b"incident-42"
        ));
    }
}
