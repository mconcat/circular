
use crate::actor_support::error_payload;
use crate::config::{ConfigRejection, HarnessName, PositiveCount, Slot, Spelled};
use crate::{
    ActorType, BaseShape, ERROR_PORT_NAME, GroundShape, ProductPayload, ProductValue, Shape,
};
use circular_core::PortId;
#[cfg(test)]
use circular_core::Tick;
use circular_runtime::{
    ActorContext, ActorEffects, ActorInput, ActorTypes, AgentHarnessAuthorityBearer,
    AgentHarnessName, AgentInvokeSpec, AgentPayload, AgentProgressRecord, AgentStepNext,
    AgentStepRequest, AgentStepResult, AgentToolCall, EditableActor, Effect, EffectOutcome,
    EmittingActor, EmittingActorFactory, FoldedConfig, OutcomePayload, ProcessingCause, ToolName,
};
use std::collections::{BTreeSet, VecDeque};
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;

fn port(name: &str) -> PortId {
    PortId::try_new(name).expect("registered agent port names are canonical")
}

fn payload_shape(shape: Shape) -> GroundShape {
    GroundShape::try_new(shape).expect("agent product shapes contain no variables")
}

fn refuse(message: impl Into<String>, cause: ProcessingCause) -> ActorEffects<ProductPayload> {
    ActorEffects::reject(error_payload(message), cause)
}

fn bytes_payload(bytes: &[u8]) -> ProductPayload {
    ProductPayload::new(
        payload_shape(Shape::Base(BaseShape::Bytes)),
        ProductValue::Bytes(bytes.to_vec()),
    )
}

/// The payload kind distinguishes prompts from non-prompt events.
/// The beta event-op set is empty. Objects never become rendered prompts.
fn turn_payload(payload: &ProductPayload) -> Result<AgentPayload, &'static str> {
    match payload.value() {
        ProductValue::Bytes(bytes) => Ok(AgentPayload::new(bytes.clone())),
        ProductValue::String(text) => Ok(AgentPayload::new(text.as_bytes().to_vec())),
        ProductValue::Object(object) => match object.get("op").and_then(ProductValue::as_str) {
            Some(_) => Err("InputOutOfDomain: agent turn op is not registered"),
            None => Err("InputOutOfDomain: agent turn event requires a string op"),
        },
        _ => Err("agent turn payload is out of domain"),
    }
}

circular_core::closed_table! {
    pub enum AgentResultKind {
        Bytes => "bytes",
        Json => "json",
    }
}

pub const AGENT_RESULT_FIELD: &str = "result";

impl AgentResultKind {
    #[must_use]
    pub fn shape(self) -> Shape {
        match self {
            Self::Bytes => Shape::Base(BaseShape::Bytes),
            Self::Json => Shape::Any,
        }
    }

    fn payload(self, output: &[u8]) -> Result<ProductPayload, &'static str> {
        match self {
            Self::Bytes => Ok(bytes_payload(output)),
            Self::Json => {
                let parsed: serde_json::Value = serde_json::from_slice(output).map_err(|_| {
                    "InputOutOfDomain: agent result is declared json and the harness output is not JSON"
                })?;
                let value = crate::parse_config::json_value(parsed).map_err(|_| {
                    "InputOutOfDomain: agent result is declared json and the harness output leaves the canonical value space"
                })?;
                Ok(ProductPayload::new(payload_shape(Shape::Any), value))
            }
        }
    }
}

#[must_use]
pub fn agent_result_kind_arms() -> Vec<(&'static str, Shape)> {
    [AgentResultKind::Bytes, AgentResultKind::Json]
        .into_iter()
        .map(|kind| (kind.as_str(), kind.shape()))
        .collect()
}

pub(crate) const HARNESS: Slot<HarnessName> = Slot::new("harness", HarnessName);

pub(crate) const QUEUE_CAPACITY: Slot<PositiveCount> = Slot::new("queue_capacity", PositiveCount);

pub(crate) const RESULT: Slot<Spelled<AgentResultKind>> = Slot::new(
    AGENT_RESULT_FIELD,
    Spelled::new(&AgentResultKind::ALL, AgentResultKind::as_str),
);

#[must_use]
pub fn declared_harness(config: &FoldedConfig) -> Option<AgentHarnessName> {
    let value = config.for_type(ActorType::Agent).ok()?;
    let schema = crate::registration(ActorType::Agent).spec().config();
    let mut fields = circular_core::Fields::open(value).ok()?;
    schema.read(&mut fields, &HARNESS).ok()
}

fn allowed_tool_policy(
    tools: &[ProductValue],
) -> Result<Option<BTreeSet<ToolName>>, AgentFactoryError> {
    let mut allowed = BTreeSet::new();
    for element in tools {
        let declaration = element.as_object().ok_or(AgentFactoryError::InvalidTools)?;
        let Some(name) = declaration.get("name").and_then(ProductValue::as_str) else {
            return Ok(None);
        };
        let name = ToolName::try_from_normalized(name.to_owned())
            .map_err(|_| AgentFactoryError::InvalidTools)?;
        allowed.insert(name);
    }
    Ok(Some(allowed))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentActorPhase {
    Ready,
    Invoking { request: AgentStepRequest },
    AwaitingTool { call: AgentToolCall },
}

pub struct AgentActor<V, I> {
    harness: AgentHarnessName,
    grant: circular_runtime::Granted<circular_runtime::AgentHarness>,
    queue_capacity: usize,
    allowed_tools: Option<BTreeSet<ToolName>>,
    result_kind: AgentResultKind,
    session: Option<circular_runtime::AgentSessionId>,
    queue: VecDeque<ProductPayload>,
    phase: AgentActorPhase,
    marker: PhantomData<fn() -> (V, I)>,
}

impl<V, I> AgentActor<V, I> {
    #[must_use]
    pub const fn phase(&self) -> &AgentActorPhase {
        &self.phase
    }

    #[must_use]
    pub fn queued_turns(&self) -> usize {
        self.queue.len()
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        harness: AgentHarnessName,
        grant: circular_runtime::Granted<circular_runtime::AgentHarness>,
    ) -> Self {
        Self {
            harness,
            grant,
            queue_capacity: 1,
            allowed_tools: None,
            result_kind: AgentResultKind::Bytes,
            session: None,
            queue: VecDeque::new(),
            phase: AgentActorPhase::Ready,
            marker: PhantomData,
        }
    }

    fn tool_allowed(&self, tool: &ToolName) -> bool {
        self.allowed_tools
            .as_ref()
            .is_none_or(|allowed| allowed.contains(tool))
    }

    fn invoke(&mut self, request: AgentStepRequest) -> ActorEffects<ProductPayload> {
        let session = self
            .session
            .as_ref()
            .filter(|session| session.harness() == &self.harness)
            .cloned();
        let invoke = AgentInvokeSpec::try_new(self.harness.clone(), session, request.clone())
            .expect("actor keeps session and request identities aligned");
        self.phase = AgentActorPhase::Invoking { request };
        ActorEffects::external(Effect::agent_invoke(self.grant, None, invoke))
    }

    fn start_next(&mut self) -> ActorEffects<ProductPayload> {
        let mut rejected = ActorEffects::empty();
        while let Some(payload) = self.queue.pop_front() {
            match turn_payload(&payload) {
                Ok(turn) => {
                    return rejected.concat(self.invoke(AgentStepRequest::user_turn(turn)));
                }
                Err(message) => {
                    rejected = rejected.concat(refuse(message, ProcessingCause::InputOutOfDomain));
                }
            }
        }
        self.phase = AgentActorPhase::Ready;
        rejected
    }

    fn accept_turn(&mut self, payload: ProductPayload) -> ActorEffects<ProductPayload> {
        let parsed = turn_payload(&payload);
        if matches!(payload.value(), ProductValue::Object(_))
            && let Err(message) = &parsed
        {
            return refuse(*message, ProcessingCause::InputOutOfDomain);
        }
        match self.phase {
            AgentActorPhase::Ready => match parsed {
                Ok(turn) => self.invoke(AgentStepRequest::user_turn(turn)),
                Err(message) => refuse(message, ProcessingCause::InputOutOfDomain),
            },
            AgentActorPhase::Invoking { .. } | AgentActorPhase::AwaitingTool { .. }
                if self.queue.len() < self.queue_capacity =>
            {
                self.queue.push_back(payload);
                ActorEffects::empty()
            }
            AgentActorPhase::Invoking { .. } | AgentActorPhase::AwaitingTool { .. } => {
                refuse("agent turn queue is full", ProcessingCause::DomainRejected)
            }
        }
    }

    fn emit_progress(records: &[AgentProgressRecord]) -> ActorEffects<ProductPayload> {
        records
            .iter()
            .fold(ActorEffects::empty(), |effects, record| {
                let decoded = circular_core::decode(
                    record.payload().as_bytes(),
                    circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
                );
                let effect = match decoded {
                    Ok(value @ ProductValue::Object(_)) => ActorEffects::emit(
                        port("record"),
                        ProductPayload::new(
                            payload_shape(Shape::Object {
                                fields: crate::FieldMap::try_new(Vec::new())
                                    .expect("empty field map is unique"),
                                open: true,
                            }),
                            value,
                        ),
                    ),
                    _ => refuse(
                        "agent progress record is not a canonical object",
                        ProcessingCause::InputOutOfDomain,
                    ),
                };
                effects.concat(effect)
            })
    }

    fn complete_invocation(&mut self, result: &AgentStepResult) -> ActorEffects<ProductPayload> {
        self.session = Some(result.session().clone());
        let mut effects = Self::emit_progress(result.progress());
        match result.next() {
            AgentStepNext::Final { output, .. } => {
                effects = match self.result_kind.payload(output.as_bytes()) {
                    Ok(payload) => effects.concat(ActorEffects::emit(port("result"), payload)),
                    Err(reason) => {
                        effects.concat(refuse(reason, ProcessingCause::InputOutOfDomain))
                    }
                };
                effects.concat(self.start_next())
            }
            AgentStepNext::ToolRequest { call } if !self.tool_allowed(call.tool()) => {
                effects
                    .concat(refuse(
                        format!(
                            "agent tool request {:?} is outside the declared tool policy",
                            call.tool().as_str()
                        ),
                        ProcessingCause::DomainRejected,
                    ))
                    .concat(self.start_next())
            }
            AgentStepNext::ToolRequest { call } => {
                self.phase = AgentActorPhase::AwaitingTool { call: call.clone() };
                effects.concat(ActorEffects::emit(
                    port("tool_request"),
                    crate::agent_tool_wire::tool_call_payload(call),
                ))
            }
        }
    }
}

const AGENT_STATE_SCHEMA: u16 = 1;

impl<V, I> AgentActor<V, I> {
    fn state_value(&self) -> ProductValue {
        let session = self.session.as_ref().map_or(ProductValue::Null, |session| {
            ProductValue::array([
                ProductValue::string(session.harness().as_str()),
                ProductValue::Bytes(session.opaque().to_vec()),
            ])
        });
        let queue =
            ProductValue::array(self.queue.iter().map(crate::payload_value::encode_payload));
        let phase = match &self.phase {
            AgentActorPhase::Ready => ProductValue::array([ProductValue::UInt(0)]),
            AgentActorPhase::Invoking { .. } => {
                unreachable!("an invoking agent has no checkpoint")
            }
            AgentActorPhase::AwaitingTool { call } => ProductValue::array([
                ProductValue::UInt(2),
                ProductValue::Bytes(call.id().as_bytes().to_vec()),
                ProductValue::string(call.tool().as_str()),
                ProductValue::Bytes(call.arguments().as_bytes().to_vec()),
            ]),
        };
        ProductValue::array([session, queue, phase])
    }

    fn restore_value(
        &self,
        value: ProductValue,
    ) -> Option<(
        Option<circular_runtime::AgentSessionId>,
        VecDeque<ProductPayload>,
        AgentActorPhase,
    )> {
        let ProductValue::Array(fields) = value else {
            return None;
        };
        let [session, queue, phase] = <[ProductValue; 3]>::try_from(fields).ok()?;
        let session = match session {
            ProductValue::Null => None,
            ProductValue::Array(parts) => {
                let [harness, opaque] = <[ProductValue; 2]>::try_from(parts).ok()?;
                let harness =
                    AgentHarnessName::try_from_normalized(harness.as_str()?.to_owned()).ok()?;
                let ProductValue::Bytes(opaque) = opaque else {
                    return None;
                };
                Some(circular_runtime::AgentSessionId::new(harness, opaque))
            }
            _ => return None,
        };
        let ProductValue::Array(queue) = queue else {
            return None;
        };
        let queue = queue
            .into_iter()
            .map(crate::payload_value::decode_payload)
            .collect::<Option<VecDeque<_>>>()?;
        let ProductValue::Array(phase) = phase else {
            return None;
        };
        let mut phase = phase.into_iter();
        let phase = match (
            phase.next()?,
            phase.next(),
            phase.next(),
            phase.next(),
            phase.next(),
        ) {
            (ProductValue::UInt(0), None, None, None, None) => AgentActorPhase::Ready,
            (
                ProductValue::UInt(2),
                Some(ProductValue::Bytes(id)),
                Some(tool),
                Some(ProductValue::Bytes(arguments)),
                None,
            ) => AgentActorPhase::AwaitingTool {
                call: AgentToolCall::new(
                    circular_runtime::AgentToolCallId::try_from_bytes(id).ok()?,
                    ToolName::try_from_normalized(tool.as_str()?.to_owned()).ok()?,
                    AgentPayload::new(arguments),
                ),
            },
            _ => return None,
        };
        Some((session, queue, phase))
    }
}

impl<V, I> EditableActor for AgentActor<V, I>
where
    V: Clone + From<u16> + PartialEq,
    I: Clone + Ord,
{
    type StateVersion = V;
    type EffectId = I;

    fn on_config_change(
        &mut self,
        _config: &FoldedConfig,
    ) -> circular_runtime::ConfigChangeOutcome {
        circular_runtime::ConfigChangeOutcome::ReplaceIncarnation
    }

    fn checkpoint(&self) -> Option<circular_runtime::ActorState<V>> {
        self.try_checkpoint().expect("agent state encodes")
    }

    fn try_checkpoint(
        &self,
    ) -> Result<Option<circular_runtime::ActorState<V>>, circular_core::CodecError> {
        if matches!(self.phase, AgentActorPhase::Invoking { .. }) {
            return Ok(None);
        }
        Ok(Some(circular_runtime::ActorState::new(
            V::from(AGENT_STATE_SCHEMA),
            circular_core::encode(
                &self.state_value(),
                circular_core::Ceilings::for_boundary(circular_core::Boundary::ActorState),
            )?
            .into_boxed_slice(),
        )))
    }

    fn restore(
        &mut self,
        state: circular_runtime::ActorState<V>,
    ) -> Result<(), circular_runtime::ActorRestoreError<V>> {
        let (schema, bytes) = state.into_parts();
        if schema != V::from(AGENT_STATE_SCHEMA) {
            return Err(circular_runtime::ActorRestoreError::SchemaBeyondLadder { schema });
        }
        let decoded = circular_core::decode(
            &bytes,
            circular_core::Ceilings::for_boundary(circular_core::Boundary::ActorState),
        )
        .ok()
        .and_then(|value| self.restore_value(value));
        let Some((session, queue, phase)) = decoded else {
            return Err(circular_runtime::ActorRestoreError::DecodeFailed { schema });
        };
        if queue.len() > self.queue_capacity {
            return Err(circular_runtime::ActorRestoreError::StateInvariantViolated { schema });
        }
        self.session = session;
        self.queue = queue;
        self.phase = phase;
        Ok(())
    }
}

impl<T> EmittingActor<T, ProductPayload> for AgentActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Grants: AgentHarnessAuthorityBearer,
    T::StateVersion: From<u16> + PartialEq,
{
    fn accepts(&self, inlet: &PortId) -> bool {
        inlet.as_str() != "turn" || matches!(self.phase, AgentActorPhase::Ready)
    }

    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        if input.inlet() == &port("tool_result") {
            let AgentActorPhase::AwaitingTool { call } = &self.phase else {
                return refuse(
                    "agent is not awaiting a tool result",
                    ProcessingCause::InputOutOfDomain,
                );
            };
            let expected = call.id().clone();
            let Some(result) = crate::agent_tool_wire::decode_tool_result(input.payload::<T>())
            else {
                return refuse(
                    "tool result payload is out of domain",
                    ProcessingCause::InputOutOfDomain,
                );
            };
            if result.call() != &expected {
                return refuse(
                    "tool result belongs to a different call",
                    ProcessingCause::InputOutOfDomain,
                );
            }
            let request = AgentStepRequest::tool_result(expected, result)
                .expect("call identity was checked immediately above");
            return self.invoke(request);
        }
        if input.inlet() != &port("turn") {
            return refuse("unknown agent inlet", ProcessingCause::InputOutOfDomain);
        }
        self.accept_turn(input.payload::<T>().clone())
    }

    fn on_outcome(
        &mut self,
        outcome: &EffectOutcome<T::EffectId>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        if !matches!(self.phase, AgentActorPhase::Invoking { .. }) {
            return ActorEffects::emit(
                port(ERROR_PORT_NAME),
                error_payload("agent received an outcome without an active invocation"),
            );
        }
        let message = match outcome.result() {
            Ok(OutcomePayload::AgentStepResult(result)) => return self.complete_invocation(result),
            Ok(other) => format!("agent received a mismatched outcome: {}", other.kind_tag()),
            Err(failure) => format!("agent invocation failed: {}", failure.kind_tag()),
        };
        Self::emit_progress(outcome.failure_progress())
            .concat(ActorEffects::emit(
                port(ERROR_PORT_NAME),
                error_payload(message),
            ))
            .concat(self.start_next())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentFactoryError {
    InvalidConfig,
    Config(ConfigRejection),
    InvalidQueueCapacity,
    InvalidTools,
    MissingGrant,
}

impl fmt::Display for AgentFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(rejection) => rejection.fmt(formatter),
            Self::InvalidConfig => formatter
                .write_str("agent config is not an agent config, or its tools are not an array"),
            Self::InvalidQueueCapacity => {
                formatter.write_str("agent queue_capacity is larger than this platform can hold")
            }
            Self::InvalidTools => {
                formatter.write_str("agent tools are outside the registered tool schema")
            }
            Self::MissingGrant => formatter.write_str("agent has no harness grant"),
        }
    }
}

impl Error for AgentFactoryError {}

impl From<ConfigRejection> for AgentFactoryError {
    fn from(rejection: ConfigRejection) -> Self {
        Self::Config(rejection)
    }
}

pub struct AgentFactory<T>(PhantomData<fn() -> T>);

impl<T> EmittingActorFactory<ProductPayload> for AgentFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::Grants: AgentHarnessAuthorityBearer,
    T::StateVersion: From<u16> + PartialEq,
{
    const TYPE: circular_core::ActorType = circular_core::ActorType::Agent;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Restarts;
    const CHECKPOINTS: bool = true;
    type Grants = T::Grants;
    type Types = T;
    type Instance = AgentActor<T::StateVersion, T::EffectId>;
    type Error = AgentFactoryError;

    fn create(config: &FoldedConfig, grants: &Self::Grants) -> Result<Self::Instance, Self::Error> {
        let value = config
            .for_type(ActorType::Agent)
            .map_err(|_| AgentFactoryError::InvalidConfig)?;
        let schema = crate::registration(ActorType::Agent).spec().config();
        let mut fields = schema.open(value)?;
        let harness = schema.read(&mut fields, &HARNESS)?;
        let result_kind = schema.read(&mut fields, &RESULT)?;
        let queue_capacity = usize::try_from(schema.read(&mut fields, &QUEUE_CAPACITY)?.get())
            .map_err(|_| AgentFactoryError::InvalidQueueCapacity)?;
        let tools = schema
            .raw(&mut fields, "tools")?
            .as_array()
            .ok_or(AgentFactoryError::InvalidConfig)?;
        let allowed_tools = allowed_tool_policy(tools)?;
        let grant = grants
            .agent_harness_authority()
            .ok_or(AgentFactoryError::MissingGrant)?;
        Ok(AgentActor {
            harness,
            grant,
            queue_capacity,
            allowed_tools,
            result_kind,
            session: None,
            queue: VecDeque::new(),
            phase: AgentActorPhase::Ready,
            marker: PhantomData,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_plan::{
        ActorId, Config, Generation, GenerationVector, Incarnation, Name as PlanName, NamedActorId,
        ScopeId,
    };
    use circular_runtime::{ActorEffect, AgentHarness, AgentHarnessGrant, Granted};

    use circular_testkit::types::TestRun;

    struct TestGrants(Granted<AgentHarness>);
    impl AgentHarnessAuthorityBearer for TestGrants {
        fn agent_harness_authority(&self) -> Option<Granted<AgentHarness>> {
            Some(self.0)
        }
    }

    type TestTypes = circular_testkit::types::TestTypes<ProductPayload, TestGrants>;

    fn grants() -> TestGrants {
        let harness = AgentHarnessName::try_from_normalized("test-harness").expect("valid name");
        let grant = AgentHarnessGrant::agent_harness([harness]);
        TestGrants(circular_runtime::GrantIssuer::new().issue(&grant))
    }

    fn config_with(tools: Vec<ProductValue>) -> FoldedConfig {
        config_of(AgentResultKind::Bytes, tools)
    }

    fn config_of(result: AgentResultKind, tools: Vec<ProductValue>) -> FoldedConfig {
        FoldedConfig::minted(
            ActorType::Agent,
            ProductValue::object([
                ("harness", ProductValue::String("test-harness".to_owned())),
                ("queue_capacity", ProductValue::Int(2)),
                ("result", ProductValue::String(result.as_str().to_owned())),
                ("tools", ProductValue::Array(tools)),
            ])
            .expect("config fields are unique"),
        )
    }

    fn config() -> FoldedConfig {
        config_with(Vec::new())
    }

    fn tool_declaration(name: &str) -> ProductValue {
        ProductValue::object([("name", ProductValue::String(name.to_owned()))])
            .expect("tool declaration key is unique")
    }

    fn invocation_context<'a>(
        actor: &'a ActorId,
        incarnation: &'a Incarnation<TestRun>,
        config: &'a Config,
        grants: &'a TestGrants,
    ) -> ActorContext<'a, TestRun, TestGrants> {
        ActorContext::new(actor, incarnation, config, grants)
    }

    #[test]
    fn agent_is_single_flight_and_terminal_failure_returns_it_to_ready() {
        let grants = grants();
        let mut actor = <AgentFactory<TestTypes> as EmittingActorFactory<ProductPayload>>::create(
            &config(),
            &grants,
        )
        .expect("typed grant boards agent");
        let named = NamedActorId::new(ScopeId::root(), PlanName::from_normalized("agent"));
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .expect("root generation");
        let incarnation =
            Incarnation::new(TestRun::default(), named.as_scoped().clone(), generations);
        let actor_id = named.as_actor_id();
        let raw_config = Config::default();
        let context = invocation_context(&actor_id, &incarnation, &raw_config, &grants);
        let input = ActorInput::new(
            port("turn"),
            ProductPayload::new(
                payload_shape(Shape::Base(BaseShape::String)),
                ProductValue::String("repair this".to_owned()),
            ),
        );

        let started = <AgentActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_event(
            &mut actor, &input, &context,
        );
        assert!(matches!(
            started.as_slice(),
            [ActorEffect::External(Effect::AgentInvoke { .. })]
        ));
        assert!(matches!(actor.phase(), AgentActorPhase::Invoking { .. }));

        let settled =
            <AgentActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_outcome(
                &mut actor,
                &EffectOutcome::new(1, Err(EffectFailure::EndpointGone)),
                &context,
            );
        assert!(matches!(actor.phase(), AgentActorPhase::Ready));
        assert!(
            matches!(settled.as_slice(), [ActorEffect::Emit { port, .. }] if port.as_str() == "_error")
        );
    }

    #[test]
    fn a_structured_turn_rejects_to_error_instead_of_leaking_debug_bytes() {
        let grants = grants();
        let mut actor = <AgentFactory<TestTypes> as EmittingActorFactory<ProductPayload>>::create(
            &config(),
            &grants,
        )
        .expect("typed grant boards agent");
        let named = NamedActorId::new(ScopeId::root(), PlanName::from_normalized("agent"));
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .expect("root generation");
        let incarnation =
            Incarnation::new(TestRun::default(), named.as_scoped().clone(), generations);
        let actor_id = named.as_actor_id();
        let raw_config = Config::default();
        let context = invocation_context(&actor_id, &incarnation, &raw_config, &grants);
        let input = ActorInput::new(
            port("turn"),
            ProductPayload::new(
                payload_shape(Shape::Base(BaseShape::Int)),
                ProductValue::Int(42),
            ),
        );

        let effects = <AgentActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_event(
            &mut actor, &input, &context,
        );
        assert!(
            matches!(
                effects.as_slice(),
                [ActorEffect::Reject {
                    cause: ProcessingCause::InputOutOfDomain,
                    ..
                }]
            ),
            "a non-text turn must reject with its cause, not invoke",
        );
        assert!(matches!(actor.phase(), AgentActorPhase::Ready));
    }

    use circular_runtime::{
        AgentProgressRecord, AgentSessionId, AgentStepResult, AgentToolCallId, EffectFailure,
        InterpreterFault,
    };

    fn harness(name: &str) -> AgentHarnessName {
        AgentHarnessName::try_from_normalized(name.to_owned()).expect("nonempty harness name")
    }

    fn stand(config: &FoldedConfig) -> (AgentActor<u16, u64>, TestGrants) {
        let grants = grants();
        let actor = <AgentFactory<TestTypes> as EmittingActorFactory<ProductPayload>>::create(
            config, &grants,
        )
        .expect("typed grant boards agent");
        (actor, grants)
    }

    fn drive<Ret>(
        grants: &TestGrants,
        f: impl FnOnce(&ActorContext<'_, TestRun, TestGrants>) -> Ret,
    ) -> Ret {
        let named = NamedActorId::new(ScopeId::root(), PlanName::from_normalized("agent"));
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .expect("root generation");
        let incarnation =
            Incarnation::new(TestRun::default(), named.as_scoped().clone(), generations);
        let actor_id = named.as_actor_id();
        let raw_config = Config::default();
        let context = invocation_context(&actor_id, &incarnation, &raw_config, grants);
        f(&context)
    }

    fn event(
        actor: &mut AgentActor<u16, u64>,
        context: &ActorContext<'_, TestRun, TestGrants>,
        inlet: &str,
        payload: ProductPayload,
    ) -> ActorEffects<ProductPayload> {
        let input = ActorInput::new(port(inlet), payload);
        <AgentActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_event(
            actor, &input, context,
        )
    }

    fn settle(
        actor: &mut AgentActor<u16, u64>,
        context: &ActorContext<'_, TestRun, TestGrants>,
        result: Result<OutcomePayload, EffectFailure>,
    ) -> ActorEffects<ProductPayload> {
        <AgentActor<u16, u64> as EmittingActor<TestTypes, ProductPayload>>::on_outcome(
            actor,
            &EffectOutcome::new(1, result),
            context,
        )
    }

    fn turn(text: &str) -> ProductPayload {
        ProductPayload::new(
            payload_shape(Shape::Base(BaseShape::String)),
            ProductValue::String(text.to_owned()),
        )
    }

    fn turn_event(op: &str, text: Option<ProductValue>) -> ProductPayload {
        let mut fields = vec![("op", ProductValue::String(op.to_owned()))];
        if let Some(text) = text {
            fields.push(("text", text));
        }
        ProductPayload::new(
            payload_shape(Shape::Any),
            ProductValue::object(fields).expect("event fields are unique"),
        )
    }

    #[test]
    fn string_and_bytes_turns_preserve_literal_prompt_bytes_and_session_reuse() {
        for input in [turn("x"), bytes_payload(b"x")] {
            let (mut actor, grants) = stand(&config());
            drive(&grants, |context| {
                let effects = event(&mut actor, context, "turn", input.clone());
                let [ActorEffect::External(Effect::AgentInvoke { invoke, .. })] =
                    effects.as_slice()
                else {
                    panic!("one text turn starts exactly one harness step");
                };
                assert_eq!(
                    invoke.request(),
                    &AgentStepRequest::user_turn(AgentPayload::new(b"x".to_vec()))
                );
                let settled = settle(&mut actor, context, final_step(b"answer"));
                assert_eq!(emitted_ports(&settled), ["record", "record", "result"]);
                assert_eq!(actor.phase(), &AgentActorPhase::Ready);
                let session = actor.session.clone();
                let effects = event(&mut actor, context, "turn", bytes_payload(&[0xff, 0, 0x61]));
                let [ActorEffect::External(Effect::AgentInvoke { invoke, .. })] =
                    effects.as_slice()
                else {
                    panic!("bytes remain a prompt without UTF-8 conversion");
                };
                assert_eq!(
                    invoke.request(),
                    &AgentStepRequest::user_turn(AgentPayload::new(vec![0xff, 0, 0x61]))
                );
                assert_eq!(actor.session, session);
            });
        }
    }

    #[test]
    fn invoking_text_turn_joins_the_fifo_and_rejects_overflow() {
        let (mut actor, grants) = stand(&config());
        drive(&grants, |context| {
            event(&mut actor, context, "turn", turn("first"));
            let active = actor.phase().clone();
            assert!(
                event(&mut actor, context, "turn", turn("second"))
                    .as_slice()
                    .is_empty()
            );
            assert!(
                event(&mut actor, context, "turn", turn("third"))
                    .as_slice()
                    .is_empty()
            );
            assert_eq!(actor.phase(), &active);
            assert_eq!(actor.queued_turns(), 2);
            let overflow = event(&mut actor, context, "turn", turn("fourth"));
            assert_eq!(
                overflow,
                ActorEffects::reject(
                    error_payload("agent turn queue is full"),
                    ProcessingCause::DomainRejected
                )
            );
            assert_eq!(actor.phase(), &active);
            assert_eq!(actor.queued_turns(), 2);

            for (text, remaining) in [("second", 1), ("third", 0)] {
                let settled = settle(&mut actor, context, final_step(b"answer"));
                assert_eq!(invoke_count(&settled), 1);
                assert_eq!(
                    actor.phase(),
                    &AgentActorPhase::Invoking {
                        request: AgentStepRequest::user_turn(AgentPayload::new(text.as_bytes())),
                    }
                );
                assert_eq!(actor.queued_turns(), remaining);
            }
            let settled = settle(&mut actor, context, final_step(b"answer"));
            assert_eq!(invoke_count(&settled), 0);
            assert_eq!(actor.phase(), &AgentActorPhase::Ready);
        });
    }

    fn assert_rejected(effects: &ActorEffects<ProductPayload>, expected_message: &str) {
        let [ActorEffect::Reject { subject, cause }] = effects.as_slice() else {
            panic!("rejection is exactly one coded refusal and no external effect");
        };
        assert_eq!(cause, &ProcessingCause::InputOutOfDomain);
        assert_eq!(
            subject.value(),
            &ProductValue::String(expected_message.to_owned())
        );
        assert_eq!(
            subject.shape(),
            &payload_shape(Shape::Base(BaseShape::String))
        );
    }

    fn refusals(effects: &ActorEffects<ProductPayload>) -> Vec<&ProcessingCause> {
        effects
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                ActorEffect::Reject { cause, .. } => Some(cause),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn unregistered_turn_events_preserve_every_phase_and_empty_or_full_prompt_queue() {
        let allowed = config_with(vec![tool_declaration("write-note")]);
        for phase in 0..3 {
            for queued in [0, 2] {
                let (mut actor, grants) = stand(&allowed);
                drive(&grants, |context| {
                    if phase > 0 {
                        event(&mut actor, context, "turn", turn("first"));
                        for _ in 0..queued {
                            event(&mut actor, context, "turn", turn("queued"));
                        }
                    }
                    if phase == 2 {
                        settle(&mut actor, context, tool_step("write-note"));
                    }
                    let before_phase = actor.phase().clone();
                    let before_queue = actor.queue.clone();
                    let before_session = actor.session.clone();
                    for op in ["inject", "approve", "stop", "unknown", ""] {
                        let payload =
                            turn_event(op, Some(ProductValue::String("not a prompt".to_owned())));
                        assert_rejected(
                            &event(&mut actor, context, "turn", payload),
                            "InputOutOfDomain: agent turn op is not registered",
                        );
                        assert_eq!(actor.phase(), &before_phase);
                        assert_eq!(actor.queue, before_queue);
                        assert_eq!(actor.session, before_session);
                    }
                    for value in [
                        ProductValue::object([] as [(&str, ProductValue); 0]).unwrap(),
                        ProductValue::object([("op", ProductValue::Int(1))]).unwrap(),
                        ProductValue::object([(
                            "text",
                            ProductValue::String("not a prompt".to_owned()),
                        )])
                        .unwrap(),
                    ] {
                        let payload = ProductPayload::new(payload_shape(Shape::Any), value);
                        assert_rejected(
                            &event(&mut actor, context, "turn", payload),
                            "InputOutOfDomain: agent turn event requires a string op",
                        );
                        assert_eq!(actor.phase(), &before_phase);
                        assert_eq!(actor.queue, before_queue);
                        assert_eq!(actor.session, before_session);
                    }
                });
            }
        }
    }

    #[test]
    fn retired_control_inlet_is_not_an_alias_for_turn() {
        let (mut actor, grants) = stand(&config());
        drive(&grants, |context| {
            assert_rejected(
                &event(
                    &mut actor,
                    context,
                    "control",
                    turn_event("inject", Some(ProductValue::String("x".to_owned()))),
                ),
                "unknown agent inlet",
            );
            assert_eq!(actor.phase(), &AgentActorPhase::Ready);
            assert_eq!(actor.queued_turns(), 0);
        });
    }

    #[test]
    fn awaiting_tool_text_turn_preserves_the_call_until_its_result() {
        let allowed = config_with(vec![tool_declaration("write-note")]);
        let (mut actor, grants) = stand(&allowed);
        drive(&grants, |context| {
            event(&mut actor, context, "turn", turn("first"));
            settle(&mut actor, context, tool_step("write-note"));
            let active = actor.phase().clone();
            assert!(matches!(active, AgentActorPhase::AwaitingTool { .. }));
            assert!(
                event(&mut actor, context, "turn", turn("next"))
                    .as_slice()
                    .is_empty()
            );
            assert_eq!(actor.phase(), &active);
            assert_eq!(actor.queued_turns(), 1);
            let matched = event(
                &mut actor,
                context,
                "tool_result",
                tool_result_input(b"call-1"),
            );
            let [ActorEffect::External(Effect::AgentInvoke { invoke, .. })] = matched.as_slice()
            else {
                panic!("the matching call must still continue the tool step");
            };
            assert!(matches!(
                invoke.request(),
                AgentStepRequest::ToolResult { .. }
            ));
            assert_eq!(actor.queued_turns(), 1);
            let settled = settle(&mut actor, context, final_step(b"tool answer"));
            assert_eq!(invoke_count(&settled), 1);
            assert_eq!(actor.queued_turns(), 0);
            assert_eq!(
                actor.phase(),
                &AgentActorPhase::Invoking {
                    request: AgentStepRequest::user_turn(AgentPayload::new(b"next".to_vec())),
                }
            );
        });
    }

    #[test]
    fn rejected_turn_event_does_not_delay_the_next_text_turn() {
        let (mut actor, grants) = stand(&config());
        drive(&grants, |context| {
            event(&mut actor, context, "turn", turn("first"));
            assert_rejected(
                &event(&mut actor, context, "turn", turn_event("inject", None)),
                "InputOutOfDomain: agent turn op is not registered",
            );
            assert!(
                event(&mut actor, context, "turn", turn("next"))
                    .as_slice()
                    .is_empty()
            );
            assert_eq!(actor.queued_turns(), 1);
            let settled = settle(&mut actor, context, final_step(b"answer"));
            assert_eq!(emitted_ports(&settled), ["record", "record", "result"]);
            assert_eq!(invoke_count(&settled), 1);
            assert_eq!(actor.queued_turns(), 0);
            assert_eq!(
                actor.phase(),
                &AgentActorPhase::Invoking {
                    request: AgentStepRequest::user_turn(AgentPayload::new(b"next".to_vec())),
                }
            );
        });
    }

    fn emitted_ports(effects: &ActorEffects<ProductPayload>) -> Vec<&str> {
        effects
            .as_slice()
            .iter()
            .filter_map(|effect| match effect {
                ActorEffect::Emit { port, .. } => Some(port.as_str()),
                _ => None,
            })
            .collect()
    }

    fn invoke_count(effects: &ActorEffects<ProductPayload>) -> usize {
        effects
            .as_slice()
            .iter()
            .filter(|effect| matches!(effect, ActorEffect::External(Effect::AgentInvoke { .. })))
            .count()
    }

    fn progress_record(kind: &str) -> AgentProgressRecord {
        let value = ProductValue::object([("kind", ProductValue::string(kind))]).unwrap();
        AgentProgressRecord::new(AgentPayload::new(
            circular_core::encode(
                &value,
                circular_core::Ceilings::for_boundary(circular_core::Boundary::Journal),
            )
            .unwrap(),
        ))
    }

    fn record_kind(payload: &ProductPayload) -> ProductValue {
        let transform = crate::accept_transform(
            &ProductValue::object([("transform", ProductValue::string("event.kind"))]).unwrap(),
        )
        .unwrap();
        crate::map_event(&transform, payload)
            .expect("the downstream expression reads the record field without a parser")
            .value()
            .clone()
    }

    fn step_ok(progress: Vec<&str>, next: AgentStepNext) -> Result<OutcomePayload, EffectFailure> {
        let target = harness("test-harness");
        let session = AgentSessionId::new(target.clone(), b"session-1".to_vec());
        let progress = progress
            .into_iter()
            .map(progress_record)
            .collect::<Vec<_>>();
        Ok(OutcomePayload::AgentStepResult(
            AgentStepResult::try_new(&target, session, progress, next)
                .expect("session stays in the harness namespace"),
        ))
    }

    fn final_step(output: &[u8]) -> Result<OutcomePayload, EffectFailure> {
        step_ok(
            vec!["progress-1", "progress-2"],
            AgentStepNext::Final {
                output: AgentPayload::new(output.to_vec()),
                metadata: AgentPayload::default(),
            },
        )
    }

    fn tool_step(tool: &str) -> Result<OutcomePayload, EffectFailure> {
        step_ok(
            Vec::new(),
            AgentStepNext::ToolRequest {
                call: AgentToolCall::new(
                    AgentToolCallId::try_from_bytes(b"call-1".to_vec()).expect("nonempty call"),
                    ToolName::try_from_normalized(tool.to_owned()).expect("nonempty tool"),
                    AgentPayload::new(b"arguments".to_vec()),
                ),
            },
        )
    }

    fn tool_result_input(call: &[u8]) -> ProductPayload {
        ProductPayload::new(
            payload_shape(Shape::Any),
            ProductValue::object([
                ("call", ProductValue::Bytes(call.to_vec())),
                ("effect", ProductValue::String("file_write".to_owned())),
                ("ok", ProductValue::Bool(true)),
                ("value", ProductValue::UInt(7)),
            ])
            .expect("tool result fields are unique"),
        )
    }

    #[test]
    fn factory_narrows_tool_declarations_to_the_boarded_range() {
        let grants = grants();
        let malformed = config_with(vec![ProductValue::Int(3)]);
        assert_eq!(
            <AgentFactory<TestTypes> as EmittingActorFactory<ProductPayload>>::create(
                &malformed, &grants
            )
            .err(),
            Some(AgentFactoryError::InvalidTools),
            "a non-object tool declaration violates the registered schema"
        );
    }

    #[test]
    fn pending_turns_queue_and_overflow_is_an_explicit_error() {
        let (mut actor, grants) = stand(&config());
        drive(&grants, |context| {
            let started = event(&mut actor, context, "turn", turn("first"));
            assert_eq!(invoke_count(&started), 1);

            assert!(
                event(&mut actor, context, "turn", turn("second"))
                    .as_slice()
                    .is_empty()
            );
            assert!(
                event(&mut actor, context, "turn", turn("third"))
                    .as_slice()
                    .is_empty()
            );
            assert_eq!(actor.queued_turns(), 2);

            let overflow = event(&mut actor, context, "turn", turn("fourth"));
            assert_eq!(refusals(&overflow), [&ProcessingCause::DomainRejected]);
            assert_eq!(
                actor.queued_turns(),
                2,
                "the refused turn must not enter the queue"
            );

            let settled = settle(&mut actor, context, final_step(b"answer-1"));
            assert_eq!(invoke_count(&settled), 1);
            assert_eq!(actor.queued_turns(), 1);
            assert!(matches!(actor.phase(), AgentActorPhase::Invoking { .. }));
        });
    }

    #[test]
    fn tool_results_outside_an_awaiting_call_are_domain_errors() {
        let (mut actor, grants) = stand(&config());
        drive(&grants, |context| {
            let idle = event(
                &mut actor,
                context,
                "tool_result",
                tool_result_input(b"call-1"),
            );
            assert_eq!(refusals(&idle), [&ProcessingCause::InputOutOfDomain]);
            assert!(matches!(actor.phase(), AgentActorPhase::Ready));

            event(&mut actor, context, "turn", turn("first"));
            let invoking = event(
                &mut actor,
                context,
                "tool_result",
                tool_result_input(b"call-1"),
            );
            assert_eq!(refusals(&invoking), [&ProcessingCause::InputOutOfDomain]);
            assert!(matches!(actor.phase(), AgentActorPhase::Invoking { .. }));
        });
    }

    #[test]
    fn a_final_outcome_emits_progress_then_result_and_the_next_turn_reuses_the_session() {
        let (mut actor, grants) = stand(&config());
        drive(&grants, |context| {
            event(&mut actor, context, "turn", turn("first"));
            let settled = settle(&mut actor, context, final_step(b"answer-1"));
            assert_eq!(emitted_ports(&settled), ["record", "record", "result"]);
            let kinds = settled
                .as_slice()
                .iter()
                .filter_map(|effect| match effect {
                    ActorEffect::Emit { port, payload, .. } if port.as_str() == "record" => {
                        Some(record_kind(payload))
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                kinds,
                [
                    ProductValue::string("progress-1"),
                    ProductValue::string("progress-2")
                ]
            );
            assert!(matches!(actor.phase(), AgentActorPhase::Ready));

            let next = event(&mut actor, context, "turn", turn("second"));
            let [ActorEffect::External(Effect::AgentInvoke { invoke, .. })] = next.as_slice()
            else {
                panic!("the next turn must start exactly one harness step");
            };
            assert_eq!(
                invoke.session().map(AgentSessionId::opaque),
                Some(&b"session-1"[..]),
                "the session returned by the last successful step names the next one"
            );
        });
    }

    #[test]
    fn a_tool_request_waits_for_the_matching_call_and_a_foreign_result_keeps_it() {
        let allowed = config_with(vec![tool_declaration("write-note")]);
        let (mut actor, grants) = stand(&allowed);
        drive(&grants, |context| {
            event(&mut actor, context, "turn", turn("first"));
            let requested = settle(&mut actor, context, tool_step("write-note"));
            assert_eq!(emitted_ports(&requested), ["tool_request"]);
            assert!(matches!(
                actor.phase(),
                AgentActorPhase::AwaitingTool { .. }
            ));

            let foreign = event(
                &mut actor,
                context,
                "tool_result",
                tool_result_input(b"call-9"),
            );
            assert_eq!(refusals(&foreign), [&ProcessingCause::InputOutOfDomain]);
            assert!(
                matches!(actor.phase(), AgentActorPhase::AwaitingTool { .. }),
                "a foreign call id must not consume the active call"
            );

            let matched = event(
                &mut actor,
                context,
                "tool_result",
                tool_result_input(b"call-1"),
            );
            assert_eq!(invoke_count(&matched), 1);
            let [ActorEffect::External(Effect::AgentInvoke { invoke, .. })] = matched.as_slice()
            else {
                panic!("a matching tool result continues into exactly one harness step");
            };
            assert!(matches!(
                invoke.request(),
                AgentStepRequest::ToolResult { .. }
            ));
        });
    }

    #[test]
    fn a_tool_request_outside_the_declared_policy_frees_the_agent_explicitly() {
        let allowed = config_with(vec![tool_declaration("write-note")]);
        let (mut actor, grants) = stand(&allowed);
        drive(&grants, |context| {
            event(&mut actor, context, "turn", turn("first"));
            let refused = settle(&mut actor, context, tool_step("delete-everything"));
            assert_eq!(refusals(&refused), [&ProcessingCause::DomainRejected]);
            assert!(emitted_ports(&refused).is_empty());
            assert!(matches!(actor.phase(), AgentActorPhase::Ready));

            let next = event(&mut actor, context, "turn", turn("second"));
            assert_eq!(invoke_count(&next), 1);
        });
    }

    #[test]
    fn a_mismatched_outcome_payload_settles_the_pending_step_and_the_queue_moves_on() {
        let (mut actor, grants) = stand(&config());
        drive(&grants, |context| {
            event(&mut actor, context, "turn", turn("first"));
            event(&mut actor, context, "turn", turn("second"));
            let settled = settle(&mut actor, context, Ok(OutcomePayload::WrittenLength(9)));
            assert_eq!(emitted_ports(&settled), ["_error"]);
            assert_eq!(
                invoke_count(&settled),
                1,
                "the queued turn starts after the mismatch is reported"
            );
            assert_eq!(actor.queued_turns(), 0);
            assert!(matches!(actor.phase(), AgentActorPhase::Invoking { .. }));
        });
    }

    #[test]
    fn settlement_errors_name_the_closed_kind_not_the_debug_form() {
        let (mut actor, grants) = stand(&config());
        drive(&grants, |context| {
            for (result, expected) in [
                (
                    Err(EffectFailure::InterpreterFault(
                        InterpreterFault::Interrupted,
                    )),
                    "agent invocation failed: interpreter_fault",
                ),
                (
                    Err(EffectFailure::RetryExhausted { attempts: 3 }),
                    "agent invocation failed: retry_exhausted",
                ),
                (
                    Ok(OutcomePayload::WrittenLength(9)),
                    "agent received a mismatched outcome: written_length",
                ),
            ] {
                event(&mut actor, context, "turn", turn("once"));
                let settled = settle(&mut actor, context, result);
                let [ActorEffect::Emit { port, payload, .. }] = settled.as_slice() else {
                    panic!("one _error emission: {:?}", emitted_ports(&settled));
                };
                assert_eq!(port.as_str(), "_error");
                assert_eq!(payload.value(), &ProductValue::String(expected.to_owned()));
            }
        });
    }

    #[test]
    fn an_outcome_without_an_active_invocation_is_an_error_and_changes_nothing() {
        let (mut actor, grants) = stand(&config());
        drive(&grants, |context| {
            let orphan = settle(&mut actor, context, final_step(b"unasked"));
            assert_eq!(emitted_ports(&orphan), ["_error"]);
            assert!(matches!(actor.phase(), AgentActorPhase::Ready));
        });
    }
}
