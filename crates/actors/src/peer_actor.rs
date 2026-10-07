use crate::actor_support::error_payload;
use crate::config::{ConfigRejection, PositiveCount, Slot};
use crate::{ActorType, FieldMap, GroundShape, ProductPayload, Shape};
use circular_core::{Boundary, Ceilings, Fields, ObjectValue, PortId, Value};
use circular_runtime::{
    ActorContext, ActorEffect, ActorEffects, ActorInput, ActorRestoreError, ActorState, ActorTypes,
    BindingLease, ConfigChangeOutcome, DeliveryDisposition, DiscoverRequest, EditableActor,
    EffectOutcome, EmittingActor, EmittingActorFactory, FoldedConfig, InboundPolicy,
    OpaqueProviderFields, OutcomePayload, Peer, PeerActorIncarnation, PeerAdapterName, PeerAddress,
    PeerAuthorityBearer, PeerAvailability, PeerBinding, PeerBindingCapabilities, PeerBindingId,
    PeerBody, PeerCapabilities, PeerCursor, PeerDisplayName, PeerEffect, PeerEvent,
    PeerEventEnvelope, PeerId, PeerKind, PeerMessage, PeerMessageId, PeerRealmId, PeerSnapshot,
    PeerStamp, SendRequest, SubmissionReceipt,
};
use std::{collections::BTreeSet, fmt, marker::PhantomData, num::NonZeroUsize};

const PEER_STATE_SCHEMA: u16 = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerFactoryError {
    InvalidConfig,
    Config(ConfigRejection),
    InvalidAdapter,
    InvalidRealm,
    InvalidName,
    InvalidInboundPolicy,
    InvalidInboxCapacity,
}
impl fmt::Display for PeerFactoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(rejection) => rejection.fmt(f),
            Self::InvalidConfig => f.write_str("peer config is not a peer config"),
            Self::InvalidAdapter => f.write_str("peer adapter is not a valid adapter name"),
            Self::InvalidRealm => f.write_str("peer realm is not a valid realm"),
            Self::InvalidName => f.write_str("peer name is not a valid peer name"),
            Self::InvalidInboundPolicy => {
                f.write_str("peer inbound policy is not a valid inbound policy")
            }
            Self::InvalidInboxCapacity => {
                f.write_str("peer inbox capacity is not a valid capacity")
            }
        }
    }
}
impl std::error::Error for PeerFactoryError {}

impl From<ConfigRejection> for PeerFactoryError {
    fn from(rejection: ConfigRejection) -> Self {
        Self::Config(rejection)
    }
}

pub(crate) const INBOX_CAPACITY: Slot<PositiveCount> = Slot::new("inbox_capacity", PositiveCount);

#[derive(Clone, Debug, PartialEq)]
pub struct PeerConfig {
    pub adapter: PeerAdapterName,
    pub realm: PeerRealmId,
    pub name: PeerDisplayName,
    pub inbound_policy: InboundPolicy,
    pub inbox_capacity: NonZeroUsize,
}
impl PeerConfig {
    pub fn from_value(value: &Value) -> Result<Self, PeerFactoryError> {
        use PeerFactoryError as E;
        let schema = crate::registration(ActorType::Peer).spec().config();
        let mut fields = schema.open(value)?;
        let adapter = schema
            .raw(&mut fields, "adapter")?
            .as_str()
            .ok_or(E::InvalidAdapter)?;
        let adapter = PeerAdapterName::try_new(adapter).map_err(|_| E::InvalidAdapter)?;
        let realm = schema
            .raw(&mut fields, "realm")?
            .as_str()
            .ok_or(E::InvalidRealm)?;
        PeerDisplayName::try_new(realm).map_err(|_| E::InvalidRealm)?;
        let realm = PeerRealmId::try_new(realm.as_bytes()).map_err(|_| E::InvalidRealm)?;
        let name = schema
            .raw(&mut fields, "name")?
            .as_str()
            .ok_or(E::InvalidName)?;
        let name = PeerDisplayName::try_new(name).map_err(|_| E::InvalidName)?;
        let inbound_policy = parse_inbound_policy(schema.raw(&mut fields, "inbound_policy")?)
            .ok_or(E::InvalidInboundPolicy)?;
        let inbox_capacity = usize::try_from(schema.read(&mut fields, &INBOX_CAPACITY)?.get())
            .ok()
            .and_then(NonZeroUsize::new)
            .ok_or(E::InvalidInboxCapacity)?;
        Ok(Self {
            adapter,
            realm,
            name,
            inbound_policy,
            inbox_capacity,
        })
    }
}

fn parse_inbound_policy(value: &Value) -> Option<InboundPolicy> {
    let mut policy = Fields::open(value).ok()?;
    let any_known_peer = policy.take("any_known_peer");
    let exact = policy.take("exact");
    policy.finish().ok()?;
    match (any_known_peer, exact) {
        (Some(Value::Bool(true)), None) => Some(InboundPolicy::AnyKnownPeer),
        (None, Some(Value::Array(entries))) => {
            let mut addresses = BTreeSet::new();
            for entry in entries {
                if !addresses.insert(parse_address(entry)?) {
                    return None;
                }
            }
            Some(InboundPolicy::Exact(addresses))
        }
        _ => None,
    }
}

fn parse_address(value: &Value) -> Option<PeerAddress> {
    let mut address = Fields::open(value).ok()?;
    let adapter: String = address.required("adapter").ok()?;
    let realm: String = address.required("realm").ok()?;
    let peer: String = address.required("peer").ok()?;
    address.finish().ok()?;
    Some(PeerAddress::new(
        PeerAdapterName::try_new(adapter).ok()?,
        PeerRealmId::try_new(realm.as_bytes()).ok()?,
        PeerId::try_new(peer.as_bytes()).ok()?,
    ))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeerState {
    Binding,
    Bound {
        binding: PeerBinding,
        peers: Option<PeerSnapshot>,
    },
    Unbinding {
        binding: PeerBinding,
    },
    Unbound,
}

pub struct PeerActor<V, I> {
    config: PeerConfig,
    state: PeerState,
    next_message: u64,
    cursor: Option<PeerCursor>,
    waiting: Waiting,
    marker: PhantomData<fn() -> (V, I)>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Waiting {
    binds: u32,
    others: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Settled {
    Bind,
    Other,
}

impl Waiting {
    fn settle(
        &mut self,
        result: &Result<OutcomePayload, circular_runtime::EffectFailure>,
    ) -> Settled {
        match result {
            Ok(OutcomePayload::PeerBinding(_)) => {
                self.binds = self.binds.saturating_sub(1);
                Settled::Bind
            }
            Ok(_) => {
                self.others = self.others.saturating_sub(1);
                Settled::Other
            }
            Err(_) if self.others > 0 => {
                self.others -= 1;
                Settled::Other
            }
            Err(_) => {
                self.binds = self.binds.saturating_sub(1);
                Settled::Bind
            }
        }
    }
}
impl<V, I> PeerActor<V, I> {
    #[must_use]
    pub const fn state(&self) -> &PeerState {
        &self.state
    }
    #[must_use]
    pub const fn config(&self) -> &PeerConfig {
        &self.config
    }
}

impl<V, I> PeerActor<V, I> {
    fn rearm(
        &mut self,
        grants: &(impl PeerAuthorityBearer + ?Sized),
    ) -> ActorEffects<ProductPayload> {
        let PeerState::Bound { binding, .. } = &self.state else {
            return ActorEffects::empty();
        };
        let Some(grant) = grants.peer_receive_authority() else {
            return error("PeerReceive denied");
        };
        let effect = ActorEffect::Peer(PeerEffect::Receive {
            grant,
            binding: binding.id().clone(),
            after: self.cursor,
        });
        self.waiting.others += 1;
        ActorEffects::singleton(effect)
    }

    fn bind<R: circular_core::StreamIdentity, G: PeerAuthorityBearer + ?Sized>(
        &mut self,
        context: &ActorContext<'_, R, G>,
    ) -> ActorEffects<ProductPayload> {
        self.state = PeerState::Binding;
        self.cursor = None;
        let grants = context.grants();
        let (Some(advertise), Some(receive)) = (
            grants.peer_advertise_authority(),
            grants.peer_receive_authority(),
        ) else {
            return error("PeerBind denied");
        };
        let Some(actor) = peer_incarnation(context) else {
            return error("InputOutOfDomain: peer incarnation");
        };
        let request = circular_runtime::BindRequest::new(
            self.config.adapter.clone(),
            actor,
            self.config.realm.clone(),
            Some(self.config.name.clone()),
            self.config.inbound_policy.clone(),
            self.config.inbox_capacity,
        );
        self.waiting.binds += 1;
        ActorEffects::singleton(ActorEffect::Peer(PeerEffect::Bind {
            advertise,
            receive,
            request,
        }))
    }

    fn unbind(
        &mut self,
        binding: PeerBinding,
        grants: &(impl PeerAuthorityBearer + ?Sized),
    ) -> ActorEffects<ProductPayload> {
        let Some(grant) = grants.peer_advertise_authority() else {
            return error("PeerUnbind denied");
        };
        let effect = ActorEffect::Peer(PeerEffect::Unbind {
            grant,
            binding: binding.id().clone(),
        });
        self.state = PeerState::Unbinding { binding };
        self.waiting.others += 1;
        ActorEffects::singleton(effect)
    }

    fn receive(
        &mut self,
        envelope: &PeerEventEnvelope,
        grants: &(impl PeerAuthorityBearer + ?Sized),
    ) -> ActorEffects<ProductPayload> {
        let (binding, bound) = match &self.state {
            PeerState::Bound { binding, .. } => (binding.clone(), true),
            PeerState::Unbinding { binding } => (binding.clone(), false),
            PeerState::Binding | PeerState::Unbound => return error("BindingStale"),
        };
        if envelope.binding() != binding.id()
            || self
                .cursor
                .is_some_and(|cursor| envelope.cursor() <= cursor)
        {
            return error("BindingStale: mismatched or stale receive outcome");
        }
        let effects = match envelope.event() {
            PeerEvent::Inbound(message) => {
                if message.to() != binding.address() {
                    return error("BindingStale: receive address mismatch");
                }
                emit("message", message_value(message))
            }
            PeerEvent::PeerSnapshotChanged(snapshot) => {
                if snapshot.realm() != binding.address().realm() {
                    return error("BindingStale: snapshot realm mismatch");
                }
                if let PeerState::Bound { peers, .. } = &mut self.state {
                    *peers = Some(snapshot.clone());
                }
                emit("peers", snapshot_value(snapshot))
            }
            PeerEvent::DeliveryChanged { message, state } => emit(
                "delivery",
                obj([
                    ("message", Value::Bytes(message.as_bytes().to_vec())),
                    ("state", Value::string(state.as_str())),
                ]),
            ),
            PeerEvent::BindingStateChanged {
                binding: changed,
                state,
            } => {
                if changed != binding.id() {
                    return error("BindingStale: binding event mismatch");
                }
                emit(
                    "binding",
                    obj([
                        ("binding", Value::Bytes(changed.as_bytes().to_vec())),
                        ("state", Value::string(state.as_str())),
                    ]),
                )
            }
            PeerEvent::AdapterDiagnostic(diagnostic) => {
                error(&format!("{}: {}", diagnostic.code(), diagnostic.detail()))
            }
        };
        self.cursor = Some(envelope.cursor());
        if bound {
            effects.concat(self.rearm(grants))
        } else {
            effects
        }
    }
}

fn message_value(message: &PeerMessage) -> Value {
    obj([
        ("id", Value::Bytes(message.id().as_bytes().to_vec())),
        (
            "provider_id",
            message
                .provider_id()
                .map_or(Value::Null, |id| Value::Bytes(id.as_bytes().to_vec())),
        ),
        ("from", address_value(message.from())),
        ("to", address_value(message.to())),
        (
            "reply_to",
            message.reply_to().map_or(Value::Null, address_value),
        ),
        (
            "correlation",
            message
                .correlation()
                .map_or(Value::Null, |id| Value::Bytes(id.as_bytes().to_vec())),
        ),
        ("body", Value::string(message.body().as_str())),
        ("provenance", Value::string(message.provenance().as_str())),
        (
            "provider_fields",
            Value::object(
                message
                    .provider_fields()
                    .iter()
                    .map(|(key, value)| (key, Value::Bytes(value.to_vec()))),
            )
            .expect("provider fields are unique"),
        ),
    ])
}

fn port(name: &str) -> PortId {
    PortId::try_new(name).expect("peer fixed port")
}
fn error(message: &str) -> ActorEffects<ProductPayload> {
    ActorEffects::emit(port(crate::ERROR_PORT_NAME), error_payload(message))
}
fn payload(value: Value) -> ProductPayload {
    ProductPayload::new(
        GroundShape::try_new(Shape::Object {
            fields: FieldMap::try_new(vec![]).expect("empty field map"),
            open: true,
        })
        .expect("peer OpenObject"),
        value,
    )
}
fn emit(name: &str, value: Value) -> ActorEffects<ProductPayload> {
    ActorEffects::emit(port(name), payload(value))
}
fn obj<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::object(fields).expect("peer runtime field names are distinct")
}
fn object(value: &Value) -> Option<&ObjectValue> {
    match value {
        Value::Object(o) => Some(o),
        _ => None,
    }
}
fn string_field<'a>(o: &'a ObjectValue, key: &str) -> Option<&'a str> {
    o.get(key)?.as_str()
}
fn bytes(value: &Value) -> Option<&[u8]> {
    match value {
        Value::Bytes(b) => Some(b),
        _ => None,
    }
}
fn uint(value: &Value) -> Option<u64> {
    match value {
        Value::UInt(n) => Some(*n),
        _ => None,
    }
}
fn boolean(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}
fn address_value(a: &PeerAddress) -> Value {
    obj([
        ("adapter", Value::string(a.adapter().as_str())),
        ("realm", Value::Bytes(a.realm().as_bytes().to_vec())),
        ("peer", Value::Bytes(a.peer().as_bytes().to_vec())),
    ])
}
fn read_address(value: &Value) -> Option<PeerAddress> {
    let o = object(value)?;
    Some(PeerAddress::new(
        PeerAdapterName::try_new(string_field(o, "adapter")?).ok()?,
        PeerRealmId::try_new(bytes(o.get("realm")?)?).ok()?,
        PeerId::try_new(bytes(o.get("peer")?)?).ok()?,
    ))
}

fn incarnation_bytes(binding: &PeerBinding) -> &[u8] {
    binding.actor().incarnation()
}
fn peer_incarnation<R: circular_core::StreamIdentity, G: ?Sized>(
    context: &ActorContext<'_, R, G>,
) -> Option<PeerActorIncarnation> {
    let ceilings = Ceilings::for_boundary(Boundary::Identity);
    let actor = circular_core::encode(
        &circular_runtime::product_identity::actor_value(context.id()).ok()?,
        ceilings,
    )
    .ok()?;
    let generations = Value::Array(
        context
            .incarnation()
            .generations()
            .iter()
            .map(|generation| Value::UInt(generation.get()))
            .collect(),
    );
    let incarnation = circular_core::encode(&generations, ceilings).ok()?;
    PeerActorIncarnation::try_new(actor, incarnation).ok()
}
fn send_request(value: &Value, binding: &PeerBinding, counter: u64) -> Option<SendRequest> {
    let o = object(value)?;
    let target = read_address(o.get("target")?)?;
    let body = PeerBody::try_new(string_field(o, "body")?).ok()?;
    let correlation = match o.get("correlation") {
        None => None,
        Some(v) => Some(PeerMessageId::try_new(bytes(v)?).ok()?),
    };
    let provider_fields = match o.get("provider_fields") {
        None => OpaqueProviderFields::empty(),
        Some(v) => OpaqueProviderFields::try_new(
            object(v)?
                .iter()
                .map(|(key, value)| Some((key, bytes(value)?)))
                .collect::<Option<Vec<_>>>()?,
        )
        .ok()?,
    };
    let message = circular_core::encode(
        &Value::Array(vec![
            Value::Bytes(incarnation_bytes(binding).to_vec()),
            Value::UInt(counter),
        ]),
        Ceilings::for_boundary(Boundary::Identity),
    )
    .ok()?;
    Some(SendRequest::new(
        binding.id().clone(),
        PeerMessageId::try_new(message).ok()?,
        target,
        body,
        correlation,
        provider_fields,
    ))
}

fn binding_value(b: &PeerBinding) -> Value {
    obj([
        ("id", Value::Bytes(b.id().as_bytes().to_vec())),
        (
            "actor",
            obj([
                ("actor", Value::Bytes(b.actor().actor().to_vec())),
                (
                    "incarnation",
                    Value::Bytes(b.actor().incarnation().to_vec()),
                ),
            ]),
        ),
        ("address", address_value(b.address())),
        ("effective_name", Value::string(b.effective_name().as_str())),
        (
            "lease",
            match b.lease() {
                BindingLease::Process => Value::string("process"),
                BindingLease::Until(until) => obj([("until", Value::UInt(until.get()))]),
            },
        ),
        (
            "capabilities",
            obj([
                ("receive_text", Value::Bool(b.capabilities().receive_text())),
                ("provider_ack", Value::Bool(b.capabilities().provider_ack())),
            ]),
        ),
    ])
}
fn read_binding(value: &Value) -> Option<PeerBinding> {
    let o = object(value)?;
    let actor = object(o.get("actor")?)?;
    let caps = object(o.get("capabilities")?)?;
    let lease = match o.get("lease")? {
        Value::String(s) if s == "process" => BindingLease::Process,
        v => BindingLease::Until(PeerStamp::new(uint(object(v)?.get("until")?)?)),
    };
    Some(PeerBinding::new(
        PeerBindingId::try_new(bytes(o.get("id")?)?).ok()?,
        PeerActorIncarnation::try_new(
            bytes(actor.get("actor")?)?,
            bytes(actor.get("incarnation")?)?,
        )
        .ok()?,
        read_address(o.get("address")?)?,
        PeerDisplayName::try_new(string_field(o, "effective_name")?).ok()?,
        lease,
        PeerBindingCapabilities::new(
            boolean(caps.get("receive_text")?)?,
            boolean(caps.get("provider_ack")?)?,
        ),
    ))
}
fn peer_value(p: &Peer) -> Value {
    obj([
        ("address", address_value(p.address())),
        ("display_name", Value::string(p.display_name().as_str())),
        ("kind", Value::string(p.kind().as_str())),
        ("availability", Value::string(p.availability().as_str())),
        (
            "capabilities",
            obj([
                ("receive_text", Value::Bool(p.capabilities().receive_text())),
                ("idle_wakeup", Value::Bool(p.capabilities().idle_wakeup())),
                (
                    "active_turn_inject",
                    Value::Bool(p.capabilities().active_turn_inject()),
                ),
            ]),
        ),
    ])
}
fn read_peer(value: &Value) -> Option<Peer> {
    let o = object(value)?;
    let caps = object(o.get("capabilities")?)?;
    Some(Peer::new(
        read_address(o.get("address")?)?,
        PeerDisplayName::try_new(string_field(o, "display_name")?).ok()?,
        PeerKind::from_str(string_field(o, "kind")?)?,
        PeerAvailability::from_str(string_field(o, "availability")?)?,
        PeerCapabilities::new(
            boolean(caps.get("receive_text")?)?,
            boolean(caps.get("idle_wakeup")?)?,
            boolean(caps.get("active_turn_inject")?)?,
        ),
    ))
}
fn snapshot_value(s: &PeerSnapshot) -> Value {
    obj([
        ("realm", Value::Bytes(s.realm().as_bytes().to_vec())),
        (
            "peers",
            Value::Array(s.peers().iter().map(peer_value).collect()),
        ),
        ("observed_at", Value::UInt(s.observed_at().get())),
    ])
}
fn read_snapshot(value: &Value) -> Option<PeerSnapshot> {
    let o = object(value)?;
    let Value::Array(peers) = o.get("peers")? else {
        return None;
    };
    Some(PeerSnapshot::new(
        PeerRealmId::try_new(bytes(o.get("realm")?)?).ok()?,
        peers.iter().map(read_peer).collect::<Option<Vec<_>>>()?,
        PeerStamp::new(uint(o.get("observed_at")?)?),
    ))
}
fn receipt_value(r: &SubmissionReceipt) -> Value {
    obj([
        ("message", Value::Bytes(r.message().as_bytes().to_vec())),
        (
            "provider_id",
            r.provider_id()
                .map_or(Value::Null, |id| Value::Bytes(id.as_bytes().to_vec())),
        ),
        ("disposition", Value::string(r.disposition().as_str())),
        ("accepted_at", Value::UInt(r.accepted_at().get())),
    ])
}

impl<T> EmittingActor<T, ProductPayload> for PeerActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
    T::Grants: PeerAuthorityBearer,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        let inlet = input.inlet();
        if inlet != &port("send") && inlet != &port("refresh") {
            return error("unknown peer inlet");
        }
        let PeerState::Bound { binding, .. } = &self.state else {
            return error("BindingStale");
        };
        if inlet == &port("refresh") {
            let Some(grant) = context.grants().peer_discover_authority() else {
                return error("PeerDiscover denied");
            };
            self.waiting.others += 1;
            return ActorEffects::singleton(ActorEffect::Peer(PeerEffect::Discover {
                grant,
                request: DiscoverRequest::new(
                    self.config.adapter.clone(),
                    self.config.realm.clone(),
                    None,
                ),
            }));
        }
        let Some(grant) = context.grants().peer_send_authority() else {
            return error("PeerSend denied");
        };
        let Some(next) = self.next_message.checked_add(1) else {
            return error("InputOutOfDomain: peer message counter exhausted");
        };
        let Some(request) = send_request(input.payload::<T>().value(), binding, self.next_message)
        else {
            return error("InputOutOfDomain: invalid peer send");
        };
        self.next_message = next;
        self.waiting.others += 1;
        ActorEffects::singleton(ActorEffect::Peer(PeerEffect::Send { grant, request }))
    }
    fn on_outcome(
        &mut self,
        outcome: &EffectOutcome<T::EffectId>,
        context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        let settled = self.waiting.settle(outcome.result());
        match outcome.result() {
            Ok(OutcomePayload::PeerBinding(b)) => {
                if b.effective_name() != &self.config.name {
                    return error(circular_runtime::PeerFailureKind::NameConflict.as_str())
                        .concat(self.unbind(b.clone(), context.grants()));
                }
                if !matches!(self.state, PeerState::Binding) {
                    return self.unbind(b.clone(), context.grants());
                }
                self.state = PeerState::Bound {
                    binding: b.clone(),
                    peers: None,
                };
                self.cursor = None;
                emit("binding", binding_value(b)).concat(self.rearm(context.grants()))
            }
            Ok(OutcomePayload::PeerEnvelope(envelope)) => self.receive(envelope, context.grants()),
            Ok(OutcomePayload::PeerSnapshot(s)) => {
                let PeerState::Bound { peers, .. } = &mut self.state else {
                    return error("peer received mismatched outcome");
                };
                *peers = Some(s.clone());
                emit("peers", snapshot_value(s))
            }
            Ok(OutcomePayload::SubmissionReceipt(r))
                if matches!(self.state, PeerState::Bound { .. }) =>
            {
                match r.disposition() {
                    DeliveryDisposition::Accepted
                    | DeliveryDisposition::Queued
                    | DeliveryDisposition::Held => emit("delivery", receipt_value(r)),
                    _ => error("receipt claims terminal delivery"),
                }
            }
            Ok(OutcomePayload::UnbindReceipt(_)) => {
                if matches!(self.state, PeerState::Unbinding { .. }) {
                    self.state = PeerState::Unbound;
                    self.cursor = None;
                }
                ActorEffects::empty()
            }
            Ok(_) => error("peer received mismatched outcome"),
            Err(circular_runtime::EffectFailure::InterpreterFault(
                circular_runtime::InterpreterFault::Interrupted,
            )) => match (&self.state, settled) {
                (PeerState::Bound { .. }, _) | (PeerState::Binding, Settled::Bind) => {
                    self.bind(context)
                }
                (PeerState::Unbinding { .. }, _) => {
                    self.state = PeerState::Unbound;
                    self.cursor = None;
                    ActorEffects::empty()
                }
                (PeerState::Binding | PeerState::Unbound, _) => ActorEffects::empty(),
            },
            Err(
                failure @ circular_runtime::EffectFailure::Peer(
                    circular_runtime::PeerFailureKind::BindingNotFound
                    | circular_runtime::PeerFailureKind::BindingStale,
                ),
            ) if matches!(self.state, PeerState::Bound { .. }) => {
                error(failure.kind_tag()).concat(self.bind(context))
            }
            Err(failure) => {
                if matches!((&self.state, settled), (PeerState::Binding, Settled::Bind)) {
                    self.state = PeerState::Unbound;
                    self.cursor = None;
                }
                error(failure.kind_tag())
            }
        }
    }

    fn on_lifecycle(
        &mut self,
        life: circular_runtime::ActorLifecycle,
        context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        match life {
            circular_runtime::ActorLifecycle::Opened => self.bind(context),
            circular_runtime::ActorLifecycle::Closing => match &self.state {
                PeerState::Bound { binding, .. } => {
                    let binding = binding.clone();
                    self.unbind(binding, context.grants())
                }
                PeerState::Binding | PeerState::Unbinding { .. } | PeerState::Unbound => {
                    ActorEffects::empty()
                }
            },
        }
    }
}
fn state_value(state: &PeerState, next_message: u64, cursor: Option<PeerCursor>) -> Value {
    let (tag, binding, peers) = match state {
        PeerState::Binding => ("binding", Value::Null, Value::Null),
        PeerState::Bound { binding, peers } => (
            "bound",
            binding_value(binding),
            peers.as_ref().map_or(Value::Null, snapshot_value),
        ),
        PeerState::Unbinding { binding } => ("unbinding", binding_value(binding), Value::Null),
        PeerState::Unbound => ("unbound", Value::Null, Value::Null),
    };
    Value::Array(vec![
        Value::string(tag),
        binding,
        peers,
        Value::UInt(next_message),
        cursor.map_or(Value::Null, |cursor| Value::UInt(cursor.get())),
    ])
}
fn read_state(value: &Value) -> Option<(PeerState, u64, Option<PeerCursor>)> {
    let Value::Array(fields) = value else {
        return None;
    };
    let [
        Value::String(tag),
        binding,
        peers,
        Value::UInt(next_message),
        cursor,
    ] = fields.as_slice()
    else {
        return None;
    };
    let cursor = match cursor {
        Value::Null => None,
        Value::UInt(value) => Some(PeerCursor::new(*value)),
        _ => return None,
    };
    let state = match tag.as_str() {
        "binding" => PeerState::Binding,
        "unbound" => PeerState::Unbound,
        "unbinding" => PeerState::Unbinding {
            binding: read_binding(binding)?,
        },
        "bound" => PeerState::Bound {
            binding: read_binding(binding)?,
            peers: match peers {
                Value::Null => None,
                v => Some(read_snapshot(v)?),
            },
        },
        _ => return None,
    };
    (state_value(&state, *next_message, cursor) == *value).then_some((state, *next_message, cursor))
}
impl<V: Clone + From<u16> + PartialEq, I: Clone + Ord> EditableActor for PeerActor<V, I> {
    type StateVersion = V;
    type EffectId = I;
    fn on_config_change(&mut self, _config: &FoldedConfig) -> ConfigChangeOutcome {
        ConfigChangeOutcome::ReplaceIncarnation
    }
    fn checkpoint(&self) -> Option<ActorState<V>> {
        self.try_checkpoint().expect("peer canonical state")
    }

    fn try_checkpoint(&self) -> Result<Option<ActorState<V>>, circular_core::CodecError> {
        Ok(Some(ActorState::new(
            V::from(PEER_STATE_SCHEMA),
            circular_core::encode(
                &state_value(&self.state, self.next_message, self.cursor),
                Ceilings::for_boundary(Boundary::ActorState),
            )?,
        )))
    }
    fn restore(&mut self, state: ActorState<V>) -> Result<(), ActorRestoreError<V>> {
        let (schema, bytes) = state.into_parts();
        if schema != V::from(PEER_STATE_SCHEMA) {
            return Err(ActorRestoreError::SchemaBeyondLadder { schema });
        }
        let value = circular_core::decode(&bytes, Ceilings::for_boundary(Boundary::ActorState))
            .map_err(|_| ActorRestoreError::DecodeFailed {
                schema: schema.clone(),
            })?;
        let (state, next_message, cursor) =
            read_state(&value).ok_or(ActorRestoreError::StateInvariantViolated { schema })?;
        self.state = state;
        self.next_message = next_message;
        self.cursor = cursor;
        self.waiting = Waiting::default();
        Ok(())
    }
}

pub struct PeerFactory<T>(PhantomData<fn() -> T>);
impl<T> EmittingActorFactory<ProductPayload> for PeerFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
    T::Grants: PeerAuthorityBearer,
{
    const TYPE: ActorType = ActorType::Peer;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Restarts;
    const CHECKPOINTS: bool = true;
    type Grants = T::Grants;
    type Types = T;
    type Instance = PeerActor<T::StateVersion, T::EffectId>;
    type Error = PeerFactoryError;
    fn create(
        config: &FoldedConfig,
        _grants: &Self::Grants,
    ) -> Result<Self::Instance, Self::Error> {
        Ok(PeerActor {
            config: declared_config(config)?,
            state: PeerState::Binding,
            next_message: 0,
            cursor: None,
            waiting: Waiting::default(),
            marker: PhantomData,
        })
    }
}

fn declared_config(config: &FoldedConfig) -> Result<PeerConfig, PeerFactoryError> {
    PeerConfig::from_value(
        config
            .for_type(ActorType::Peer)
            .map_err(|_| PeerFactoryError::InvalidConfig)?,
    )
}

pub(crate) fn judge(
    config: &FoldedConfig,
    _inlets: &crate::inlet_shapes::ResolvedInletShapes,
) -> Result<(), PeerFactoryError> {
    declared_config(config).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::StreamIdentity;
    use circular_plan::{
        Config, Generation, GenerationVector, Incarnation, Name, NamedActorId, ScopeId,
    };
    use circular_runtime::{
        EffectFailure, GrantIssuer, Granted, PeerBindingState, PeerDiscover, PeerDiscoverGrant,
        PeerRealmScope, PeerSend, PeerSendGrant, ProviderMessageId, UnbindReceipt,
    };

    #[derive(Clone, Debug, Eq, Hash, PartialEq)]
    struct Run;
    impl StreamIdentity for Run {}
    struct Grants {
        discover: Option<Granted<PeerDiscover>>,
        send: Option<Granted<PeerSend>>,
        receive: Option<Granted<circular_runtime::PeerReceive>>,
        advertise: Option<Granted<circular_runtime::PeerAdvertise>>,
    }
    impl PeerAuthorityBearer for Grants {
        fn peer_discover_authority(&self) -> Option<Granted<PeerDiscover>> {
            self.discover
        }
        fn peer_send_authority(&self) -> Option<Granted<PeerSend>> {
            self.send
        }
        fn peer_advertise_authority(&self) -> Option<Granted<circular_runtime::PeerAdvertise>> {
            self.advertise
        }
        fn peer_receive_authority(&self) -> Option<Granted<circular_runtime::PeerReceive>> {
            self.receive
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
    type Actor = PeerActor<u16, u64>;
    fn address() -> PeerAddress {
        PeerAddress::new(
            PeerAdapterName::try_new("memory").unwrap(),
            PeerRealmId::try_new(b"realm".as_slice()).unwrap(),
            PeerId::try_new(b"peer".as_slice()).unwrap(),
        )
    }
    fn binding() -> PeerBinding {
        PeerBinding::new(
            PeerBindingId::try_new(b"binding".as_slice()).unwrap(),
            PeerActorIncarnation::try_new(b"actor".as_slice(), b"incarnation".as_slice()).unwrap(),
            address(),
            PeerDisplayName::try_new("name").unwrap(),
            BindingLease::Process,
            PeerBindingCapabilities::new(true, false),
        )
    }
    fn snapshot() -> PeerSnapshot {
        PeerSnapshot::new(
            address().realm().clone(),
            vec![Peer::new(
                address(),
                PeerDisplayName::try_new("peer name").unwrap(),
                PeerKind::Agent,
                PeerAvailability::Idle,
                PeerCapabilities::new(true, false, true),
            )],
            PeerStamp::new(19),
        )
    }
    fn config() -> Value {
        obj([
            ("adapter", Value::string("memory")),
            ("realm", Value::string("realm")),
            ("name", Value::string("name")),
            (
                "inbound_policy",
                obj([("any_known_peer", Value::Bool(true))]),
            ),
            ("inbox_capacity", Value::Int(4)),
        ])
    }
    fn changed(value: Value, key: &str, replacement: Value) -> Value {
        Value::object(object(&value).unwrap().iter().map(|(k, v)| {
            (
                k,
                if k == key {
                    replacement.clone()
                } else {
                    v.clone()
                },
            )
        }))
        .unwrap()
    }
    fn actor() -> Actor {
        PeerFactory::<Types>::create(
            &FoldedConfig::minted(ActorType::Peer, config()),
            &Grants {
                discover: None,
                send: None,
                receive: None,
                advertise: None,
            },
        )
        .unwrap()
    }
    fn drive<R>(f: impl FnOnce(&ActorContext<'_, Run, Grants>) -> R) -> R {
        let address = address();
        let issuer = GrantIssuer::new();
        let grants =
            Grants {
                discover: Some(issuer.issue(&PeerDiscoverGrant::peer_discover([
                    PeerRealmScope::new(address.adapter().clone(), address.realm().clone()),
                ]))),
                send: Some(issuer.issue(&PeerSendGrant::peer_send([address]))),
                receive: Some(issuer.issue(&circular_runtime::PeerReceiveGrant::peer_receive([]))),
                advertise: Some(
                    issuer.issue(&circular_runtime::PeerAdvertiseGrant::peer_advertise([])),
                ),
            };
        let actor = NamedActorId::new(ScopeId::root(), Name::from_normalized("peer"));
        let generations =
            GenerationVector::for_actor(actor.as_scoped(), vec![Generation::new(0)]).unwrap();
        let incarnation = Incarnation::new(Run, actor.as_scoped().clone(), generations);
        f(&ActorContext::new(
            &actor.as_actor_id(),
            &incarnation,
            &Config::default(),
            &grants,
        ))
    }
    fn event(
        actor: &mut Actor,
        ctx: &ActorContext<'_, Run, Grants>,
        inlet: &str,
        value: Value,
    ) -> ActorEffects<ProductPayload> {
        <Actor as EmittingActor<Types, ProductPayload>>::on_event(
            actor,
            &ActorInput::new(port(inlet), payload(value)),
            ctx,
        )
    }
    fn outcome(
        actor: &mut Actor,
        ctx: &ActorContext<'_, Run, Grants>,
        result: Result<OutcomePayload, EffectFailure>,
    ) -> ActorEffects<ProductPayload> {
        <Actor as EmittingActor<Types, ProductPayload>>::on_outcome(
            actor,
            &EffectOutcome::new(1, result),
            ctx,
        )
    }
    fn lifecycle(
        actor: &mut Actor,
        ctx: &ActorContext<'_, Run, Grants>,
        life: circular_runtime::ActorLifecycle,
    ) -> ActorEffects<ProductPayload> {
        <Actor as EmittingActor<Types, ProductPayload>>::on_lifecycle(actor, life, ctx)
    }
    fn emitted(effects: &ActorEffects<ProductPayload>, expected_port: &str) -> Value {
        let emissions = effects
            .iter()
            .filter(|effect| matches!(effect, ActorEffect::Emit { .. }))
            .collect::<Vec<_>>();
        let [ActorEffect::Emit { port, payload, .. }] = emissions.as_slice() else {
            panic!("expected one emit: {effects:?}");
        };
        assert_eq!(port.as_str(), expected_port);
        payload.value().clone()
    }
    fn assert_error(effects: &ActorEffects<ProductPayload>, message: &str) {
        assert!(
            emitted(effects, "_error")
                .as_str()
                .unwrap()
                .contains(message)
        );
    }
    fn send_value() -> Value {
        obj([
            (
                "target",
                obj([
                    ("adapter", Value::string("memory")),
                    ("realm", Value::Bytes(b"realm".to_vec())),
                    ("peer", Value::Bytes(b"peer".to_vec())),
                ]),
            ),
            ("body", Value::string("hello")),
            ("correlation", Value::Bytes(b"prior".to_vec())),
            (
                "provider_fields",
                obj([("opaque", Value::Bytes(vec![0, 255]))]),
            ),
        ])
    }
    fn bind(actor: &mut Actor, ctx: &ActorContext<'_, Run, Grants>) {
        outcome(actor, ctx, Ok(OutcomePayload::PeerBinding(binding())));
    }

    #[test]
    fn product_factory_boards_a_restartable_actor() {
        struct CatalogTypes;
        impl ActorTypes for CatalogTypes {
            type Stream = Run;
            type Event = ProductPayload;
            type Payload = ProductPayload;
            type EffectId = u64;
            type StateVersion = u16;
            type Observation = ();
            type Grants = ();
            fn payload(event: &Self::Event) -> &Self::Payload {
                event
            }
        }
        let factory = crate::product_actor_factory::<CatalogTypes>(ActorType::Peer).unwrap();
        assert_eq!(
            crate::registration(ActorType::Peer).factory(),
            crate::FactoryArm::Actor
        );
        let mut actor = factory
            .create(
                &FoldedConfig::minted(ActorType::Peer, config()),
                &(),
                &crate::ResolvedInletShapes::default(),
            )
            .unwrap();
        assert!(matches!(actor, crate::ProductActor::Peer(_)));
        assert!(actor.checkpoint().is_some());
        assert_eq!(
            actor.on_config_change(&FoldedConfig::minted(ActorType::Peer, config())),
            ConfigChangeOutcome::ReplaceIncarnation
        );
    }

    #[test]
    fn inbound_any_known_peer_normalizes_to_the_existing_unrestricted_policy() {
        let parsed = PeerConfig::from_value(&config()).unwrap();
        assert_eq!(parsed.inbound_policy, InboundPolicy::AnyKnownPeer);
        let undiscovered = PeerAddress::new(
            PeerAdapterName::try_new("memory").unwrap(),
            PeerRealmId::try_new(b"realm".as_slice()).unwrap(),
            PeerId::try_new(b"never-discovered".as_slice()).unwrap(),
        );
        assert!(parsed.inbound_policy.allows(&undiscovered));
    }

    #[test]
    fn inbound_exact_normalizes_three_literal_addresses_as_a_set() {
        let literal = obj([(
            "exact",
            Value::Array(vec![
                obj([
                    ("adapter", Value::string("memory")),
                    ("realm", Value::string("team")),
                    ("peer", Value::string("carol")),
                ]),
                obj([
                    ("adapter", Value::string("codex")),
                    ("realm", Value::string("team")),
                    ("peer", Value::string("alice")),
                ]),
                obj([
                    ("adapter", Value::string("pi")),
                    ("realm", Value::string("other")),
                    ("peer", Value::string("bob")),
                ]),
            ]),
        )]);
        let parsed = PeerConfig::from_value(&changed(config(), "inbound_policy", literal)).unwrap();
        let expected = InboundPolicy::Exact(BTreeSet::from([
            PeerAddress::new(
                PeerAdapterName::try_new("codex").unwrap(),
                PeerRealmId::try_new(b"team".as_slice()).unwrap(),
                PeerId::try_new(b"alice".as_slice()).unwrap(),
            ),
            PeerAddress::new(
                PeerAdapterName::try_new("pi").unwrap(),
                PeerRealmId::try_new(b"other".as_slice()).unwrap(),
                PeerId::try_new(b"bob".as_slice()).unwrap(),
            ),
            PeerAddress::new(
                PeerAdapterName::try_new("memory").unwrap(),
                PeerRealmId::try_new(b"team".as_slice()).unwrap(),
                PeerId::try_new(b"carol".as_slice()).unwrap(),
            ),
        ]));
        assert_eq!(parsed.inbound_policy, expected);
    }

    #[test]
    fn inbound_exact_empty_denies_every_address() {
        let parsed = PeerConfig::from_value(&changed(
            config(),
            "inbound_policy",
            obj([("exact", Value::Array(vec![]))]),
        ))
        .unwrap();
        assert_eq!(parsed.inbound_policy, InboundPolicy::Exact(BTreeSet::new()));
        assert!(!parsed.inbound_policy.allows(&address()));
    }

    #[test]
    fn inbound_policy_rejects_the_six_closed_shape_violations() {
        let entry = obj([
            ("adapter", Value::string("memory")),
            ("realm", Value::string("team")),
            ("peer", Value::string("alice")),
        ]);
        let invalids = [
            ("false", obj([("any_known_peer", Value::Bool(false))])),
            (
                "extra key",
                obj([
                    ("any_known_peer", Value::Bool(true)),
                    ("extra", Value::Null),
                ]),
            ),
            ("empty object", obj([])),
            (
                "both forms",
                obj([
                    ("any_known_peer", Value::Bool(true)),
                    ("exact", Value::Array(vec![])),
                ]),
            ),
            (
                "extra address field",
                obj([(
                    "exact",
                    Value::Array(vec![obj([
                        ("adapter", Value::string("memory")),
                        ("realm", Value::string("team")),
                        ("peer", Value::string("alice")),
                        ("extra", Value::Null),
                    ])]),
                )]),
            ),
            (
                "duplicate address",
                obj([("exact", Value::Array(vec![entry.clone(), entry]))]),
            ),
        ];
        for (case, literal) in invalids {
            assert_eq!(
                PeerConfig::from_value(&changed(config(), "inbound_policy", literal)).unwrap_err(),
                PeerFactoryError::InvalidInboundPolicy,
                "{case}"
            );
        }
    }

    #[test]
    fn inbound_exact_rejects_malformed_address_values() {
        for literal in [
            Value::Null,
            obj([("exact", Value::string("alice"))]),
            obj([("exact", Value::Array(vec![Value::string("alice")]))]),
            obj([(
                "exact",
                Value::Array(vec![obj([
                    ("adapter", Value::string("memory")),
                    ("realm", Value::string("team")),
                ])]),
            )]),
            obj([(
                "exact",
                Value::Array(vec![obj([
                    ("adapter", Value::string("memory")),
                    ("realm", Value::string("team")),
                    ("peer", Value::string("")),
                ])]),
            )]),
            obj([(
                "exact",
                Value::Array(vec![obj([
                    ("adapter", Value::string("memory")),
                    ("realm", Value::Bool(true)),
                    ("peer", Value::string("alice")),
                ])]),
            )]),
        ] {
            assert_eq!(
                PeerConfig::from_value(&changed(config(), "inbound_policy", literal)).unwrap_err(),
                PeerFactoryError::InvalidInboundPolicy
            );
        }
    }

    #[test]
    fn non_bound_events_and_non_bind_failures_preserve_state() {
        drive(|ctx| {
            let mut a = actor();
            for state in [
                PeerState::Binding,
                PeerState::Unbinding { binding: binding() },
                PeerState::Unbound,
            ] {
                a.state = state.clone();
                for inlet in ["send", "refresh"] {
                    assert_error(&event(&mut a, ctx, inlet, Value::Null), "BindingStale");
                }
                for inlet in ["_timer", "unknown"] {
                    assert_error(
                        &event(&mut a, ctx, inlet, Value::Null),
                        "unknown peer inlet",
                    );
                }
                assert_eq!(a.state, state);
            }
            lifecycle(&mut a, ctx, circular_runtime::ActorLifecycle::Opened);
            bind(&mut a, ctx);
            let bound_state = a.state.clone();
            assert!(matches!(bound_state, PeerState::Bound { .. }));
            for failure in [
                EffectFailure::EndpointGone,
                EffectFailure::Peer(circular_runtime::PeerFailureKind::InboxFull),
            ] {
                let sent = event(&mut a, ctx, "send", send_value());
                assert!(matches!(
                    sent.as_slice(),
                    [ActorEffect::Peer(PeerEffect::Send { .. })]
                ));
                assert_error(
                    &outcome(&mut a, ctx, Err(failure.clone())),
                    failure.kind_tag(),
                );
                assert_eq!(a.state, bound_state);
            }
            assert_error(
                &outcome(&mut a, ctx, Ok(OutcomePayload::WrittenLength(0))),
                "mismatched outcome",
            );
            assert_eq!(a.state, bound_state);
            a.state = PeerState::Bound {
                binding: binding(),
                peers: None,
            };
            let stale = outcome(
                &mut a,
                ctx,
                Err(EffectFailure::Peer(
                    circular_runtime::PeerFailureKind::BindingStale,
                )),
            );
            assert!(matches!(
                stale.as_slice(),
                [ActorEffect::Emit { port, .. }, ActorEffect::Peer(PeerEffect::Bind { .. })]
                    if port.as_str() == "_error"
            ));
            assert_eq!(a.state, PeerState::Binding);
        });
    }

    #[test]
    fn prior_send_failure_does_not_close_a_pending_bind() {
        drive(|ctx| {
            let mut a = actor();
            lifecycle(&mut a, ctx, circular_runtime::ActorLifecycle::Opened);
            bind(&mut a, ctx);
            event(&mut a, ctx, "send", send_value());
            let rebind = outcome(
                &mut a,
                ctx,
                Err(EffectFailure::Peer(
                    circular_runtime::PeerFailureKind::BindingStale,
                )),
            );
            assert!(matches!(
                rebind.as_slice(),
                [
                    ActorEffect::Emit { .. },
                    ActorEffect::Peer(PeerEffect::Bind { .. })
                ]
            ));
            assert_error(
                &outcome(&mut a, ctx, Err(EffectFailure::EndpointGone)),
                "endpoint_gone",
            );
            assert_eq!(a.state(), &PeerState::Binding);
            outcome(
                &mut a,
                ctx,
                Err(EffectFailure::Peer(
                    circular_runtime::PeerFailureKind::AdapterUnavailable,
                )),
            );
            assert_eq!(a.state(), &PeerState::Unbound);
        });
    }

    #[test]
    fn terminal_bind_failure_is_unbound_and_can_bind_again() {
        for failure in [
            EffectFailure::Peer(circular_runtime::PeerFailureKind::AdapterUnavailable),
            EffectFailure::Peer(circular_runtime::PeerFailureKind::UnsupportedCapability),
            EffectFailure::EndpointGone,
        ] {
            drive(|ctx| {
                let mut a = actor();
                let opened = lifecycle(&mut a, ctx, circular_runtime::ActorLifecycle::Opened);
                assert!(matches!(
                    opened.as_slice(),
                    [ActorEffect::Peer(PeerEffect::Bind { .. })]
                ));
                let failed = outcome(&mut a, ctx, Err(failure.clone()));
                assert_error(&failed, failure.kind_tag());
                assert_eq!(failed.as_slice().len(), 1, "failure does not retry Bind");
                assert_eq!(a.state(), &PeerState::Unbound);
                let (_, bytes) = a.checkpoint().unwrap().into_parts();
                let state =
                    circular_core::decode(&bytes, Ceilings::for_boundary(Boundary::ActorState))
                        .unwrap();
                assert_eq!(state.as_array().unwrap()[0], Value::string("unbound"));

                assert!(
                    lifecycle(&mut a, ctx, circular_runtime::ActorLifecycle::Closing).is_empty()
                );
                let reopened = lifecycle(&mut a, ctx, circular_runtime::ActorLifecycle::Opened);
                assert!(matches!(
                    reopened.as_slice(),
                    [ActorEffect::Peer(PeerEffect::Bind { .. })]
                ));
                let bound = outcome(&mut a, ctx, Ok(OutcomePayload::PeerBinding(binding())));
                assert_eq!(
                    object(&emitted(&bound, "binding"))
                        .unwrap()
                        .get("effective_name"),
                    Some(&Value::string("name"))
                );
                assert!(matches!(a.state(), PeerState::Bound { .. }));
            });
        }
    }

    #[test]
    fn binding_projection_matches_every_literal_runtime_field() {
        drive(|ctx| {
            let mut a = actor();
            let effects = outcome(&mut a, ctx, Ok(OutcomePayload::PeerBinding(binding())));
            assert_eq!(
                emitted(&effects, "binding"),
                obj([
                    ("id", Value::Bytes(b"binding".to_vec())),
                    (
                        "actor",
                        obj([
                            ("actor", Value::Bytes(b"actor".to_vec())),
                            ("incarnation", Value::Bytes(b"incarnation".to_vec()))
                        ])
                    ),
                    (
                        "address",
                        obj([
                            ("adapter", Value::string("memory")),
                            ("realm", Value::Bytes(b"realm".to_vec())),
                            ("peer", Value::Bytes(b"peer".to_vec()))
                        ])
                    ),
                    ("effective_name", Value::string("name")),
                    ("lease", Value::string("process")),
                    (
                        "capabilities",
                        obj([
                            ("receive_text", Value::Bool(true)),
                            ("provider_ack", Value::Bool(false))
                        ])
                    ),
                ])
            );
            assert_eq!(
                a.state(),
                &PeerState::Bound {
                    binding: binding(),
                    peers: None
                }
            );
        });
    }

    #[test]
    fn send_preserves_fields_and_mints_monotone_canonical_ids() {
        drive(|ctx| {
            let mut a = actor();
            bind(&mut a, ctx);
            for counter in 0..2 {
                let effects = event(&mut a, ctx, "send", send_value());
                let [ActorEffect::Peer(PeerEffect::Send { request, .. })] = effects.as_slice()
                else {
                    panic!("send");
                };
                assert_eq!(request.binding(), binding().id());
                assert_eq!(request.target(), &address());
                assert_eq!(request.body().as_str(), "hello");
                assert_eq!(request.correlation().unwrap().as_bytes(), b"prior");
                assert_eq!(
                    request.provider_fields().iter().collect::<Vec<_>>(),
                    vec![("opaque", [0, 255].as_slice())]
                );
                assert_eq!(
                    circular_core::decode(
                        request.message().as_bytes(),
                        Ceilings::for_boundary(Boundary::Identity)
                    )
                    .unwrap(),
                    Value::Array(vec![
                        Value::Bytes(b"incarnation".to_vec()),
                        Value::UInt(counter)
                    ])
                );
            }
            let value = obj([
                ("target", address_value(&address())),
                ("body", Value::string("minimal")),
            ]);
            let effects = event(&mut a, ctx, "send", value);
            let [ActorEffect::Peer(PeerEffect::Send { request, .. })] = effects.as_slice() else {
                panic!("send");
            };
            assert!(request.correlation().is_none());
            assert_eq!(request.provider_fields(), &OpaqueProviderFields::empty());
        });
    }

    #[test]
    fn memory_peers_and_binding_addresses_deliver_between_two_peer_actors() {
        use circular_runtime::{BindRequest, MemoryPeerAdapter, PeerAdapter};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use std::task::{Wake, Waker};

        #[derive(Default)]
        struct ReceiveWake(AtomicUsize);
        impl Wake for ReceiveWake {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        for from_peers in [true, false] {
            drive(|ctx| {
                let mut adapter = MemoryPeerAdapter::new(address().adapter().clone());
                let bind_peer = |adapter: &mut MemoryPeerAdapter, id, name: &str| {
                    adapter
                        .bind(BindRequest::new(
                            address().adapter().clone(),
                            PeerActorIncarnation::try_new([id], [id]).unwrap(),
                            address().realm().clone(),
                            Some(PeerDisplayName::try_new(name).unwrap()),
                            InboundPolicy::AnyKnownPeer,
                            NonZeroUsize::new(4).unwrap(),
                        ))
                        .unwrap()
                };
                let named = |name: &str| -> Actor {
                    PeerFactory::<Types>::create(
                        &FoldedConfig::minted(
                            ActorType::Peer,
                            changed(config(), "name", Value::string(name)),
                        ),
                        &Grants {
                            discover: None,
                            send: None,
                            receive: None,
                            advertise: None,
                        },
                    )
                    .unwrap()
                };
                let a_binding = bind_peer(&mut adapter, 1, "a");
                let b_binding = bind_peer(&mut adapter, 2, "b");
                let mut a = named("a");
                let mut b = named("b");
                outcome(
                    &mut a,
                    ctx,
                    Ok(OutcomePayload::PeerBinding(a_binding.clone())),
                );
                let binding_output = emitted(
                    &outcome(
                        &mut b,
                        ctx,
                        Ok(OutcomePayload::PeerBinding(b_binding.clone())),
                    ),
                    "binding",
                );
                let refresh = event(&mut a, ctx, "refresh", Value::Null);
                let [ActorEffect::Peer(PeerEffect::Discover { request, .. })] = refresh.as_slice()
                else {
                    panic!("refresh must discover peers");
                };
                let peers = emitted(
                    &outcome(
                        &mut a,
                        ctx,
                        Ok(OutcomePayload::PeerSnapshot(
                            adapter.discover(request.clone()).unwrap(),
                        )),
                    ),
                    "peers",
                );
                let binding_address = object(&binding_output).unwrap().get("address").unwrap();
                let Value::Array(peers) = object(&peers).unwrap().get("peers").unwrap() else {
                    panic!("peers must be an array");
                };
                assert_eq!(peers.len(), 2);
                let advertised_address = peers
                    .iter()
                    .map(|p| object(p).unwrap().get("address").unwrap())
                    .find(|p| *p == binding_address)
                    .expect("b is advertised by a.peers");
                let target = if from_peers {
                    advertised_address
                } else {
                    binding_address
                };
                let effects = event(
                    &mut a,
                    ctx,
                    "send",
                    obj([
                        ("target", target.clone()),
                        ("body", Value::string("hello peer")),
                    ]),
                );
                let [ActorEffect::Peer(PeerEffect::Send { request, .. })] = effects.as_slice()
                else {
                    panic!("send must produce one request and no error");
                };
                let wake = Arc::new(ReceiveWake::default());
                adapter.set_receive_waker(Waker::from(wake.clone()));
                let receipt = adapter.send(request.clone()).unwrap();
                let delivery = emitted(
                    &outcome(&mut a, ctx, Ok(OutcomePayload::SubmissionReceipt(receipt))),
                    "delivery",
                );
                assert_eq!(
                    object(&delivery).unwrap().get("disposition"),
                    Some(&Value::string("accepted"))
                );
                assert_eq!(wake.0.load(Ordering::SeqCst), 1);
                let inbound = adapter.receive(b_binding.id(), None).unwrap();
                assert_eq!(inbound.events().len(), 1);
                let message = emitted(
                    &outcome(
                        &mut b,
                        ctx,
                        Ok(OutcomePayload::PeerEnvelope(inbound.events()[0].clone())),
                    ),
                    "message",
                );
                assert_eq!(
                    object(&message).unwrap().get("body"),
                    Some(&Value::string("hello peer"))
                );
                assert_eq!(
                    object(&message).unwrap().get("provenance"),
                    Some(&Value::string("circular_actor"))
                );
                let delivered = adapter.receive(a_binding.id(), None).unwrap();
                assert_eq!(delivered.events().len(), 1);
                assert!(
                    matches!(delivered.events()[0].event(), PeerEvent::DeliveryChanged { message, state }
                    if message == request.message() && *state == DeliveryDisposition::Delivered)
                );
                let delivery = emitted(
                    &outcome(
                        &mut a,
                        ctx,
                        Ok(OutcomePayload::PeerEnvelope(delivered.events()[0].clone())),
                    ),
                    "delivery",
                );
                assert_eq!(
                    object(&delivery).unwrap().get("state"),
                    Some(&Value::string("delivered"))
                );
            });
        }
    }

    #[test]
    fn send_rejects_bad_projection_and_missing_authority_without_advancing_counter() {
        drive(|ctx| {
            let mut a = actor();
            bind(&mut a, ctx);
            for invalid in [
                Value::Null,
                obj([("body", Value::string("hello"))]),
                changed(send_value(), "body", Value::string("")),
                changed(send_value(), "body", Value::string("a\0b")),
                changed(
                    send_value(),
                    "target",
                    changed(
                        address_value(&address()),
                        "adapter",
                        Value::string(" memory"),
                    ),
                ),
                changed(send_value(), "correlation", Value::Bytes(vec![])),
                changed(
                    send_value(),
                    "provider_fields",
                    obj([("bad", Value::string("bytes required"))]),
                ),
            ] {
                assert_error(&event(&mut a, ctx, "send", invalid), "InputOutOfDomain");
                assert_eq!(a.next_message, 0);
            }
            let grants = Grants {
                send: None,
                receive: None,
                discover: None,
                advertise: None,
            };
            let denied = ActorContext::new(ctx.id(), ctx.incarnation(), ctx.config(), &grants);
            assert_error(
                &event(&mut a, &denied, "send", send_value()),
                "PeerSend denied",
            );
            assert_error(
                &event(&mut a, &denied, "refresh", Value::Null),
                "PeerDiscover denied",
            );
            assert_eq!(a.next_message, 0);
            a.next_message = u64::MAX;
            assert_error(
                &event(&mut a, ctx, "send", send_value()),
                "InputOutOfDomain",
            );
            assert_eq!(a.next_message, u64::MAX);
        });
    }

    #[test]
    fn receipts_only_emit_nonterminal_delivery_and_do_not_change_state() {
        drive(|ctx| {
            let mut a = actor();
            bind(&mut a, ctx);
            let state = a.state.clone();
            for (disposition, name) in [
                (DeliveryDisposition::Accepted, "accepted"),
                (DeliveryDisposition::Queued, "queued"),
                (DeliveryDisposition::Held, "held"),
            ] {
                let receipt = SubmissionReceipt::try_new(
                    PeerMessageId::try_new(b"message".as_slice()).unwrap(),
                    Some(ProviderMessageId::try_new(b"provider".as_slice()).unwrap()),
                    disposition,
                    PeerStamp::new(23),
                )
                .unwrap();
                let effects = outcome(&mut a, ctx, Ok(OutcomePayload::SubmissionReceipt(receipt)));
                assert_eq!(
                    emitted(&effects, "delivery"),
                    obj([
                        ("message", Value::Bytes(b"message".to_vec())),
                        ("provider_id", Value::Bytes(b"provider".to_vec())),
                        ("disposition", Value::string(name)),
                        ("accepted_at", Value::UInt(23))
                    ])
                );
                assert_eq!(a.state, state);
            }
            for disposition in [
                DeliveryDisposition::Delivered,
                DeliveryDisposition::Refused,
                DeliveryDisposition::Expired,
                DeliveryDisposition::Unreachable,
            ] {
                assert!(
                    SubmissionReceipt::try_new(
                        PeerMessageId::try_new(vec![1]).unwrap(),
                        None,
                        disposition,
                        PeerStamp::new(0)
                    )
                    .is_err()
                );
            }
        });
    }

    #[test]
    fn snapshots_replace_state_and_unbind_closes_without_publication() {
        drive(|ctx| {
            let mut a = actor();
            assert_error(
                &outcome(&mut a, ctx, Ok(OutcomePayload::PeerSnapshot(snapshot()))),
                "mismatched outcome",
            );
            bind(&mut a, ctx);
            let effects = outcome(&mut a, ctx, Ok(OutcomePayload::PeerSnapshot(snapshot())));
            assert_eq!(
                emitted(&effects, "peers"),
                obj([
                    ("realm", Value::Bytes(b"realm".to_vec())),
                    (
                        "peers",
                        Value::Array(vec![obj([
                            (
                                "address",
                                obj([
                                    ("adapter", Value::string("memory")),
                                    ("realm", Value::Bytes(b"realm".to_vec())),
                                    ("peer", Value::Bytes(b"peer".to_vec()))
                                ])
                            ),
                            ("display_name", Value::string("peer name")),
                            ("kind", Value::string("agent")),
                            ("availability", Value::string("idle")),
                            (
                                "capabilities",
                                obj([
                                    ("receive_text", Value::Bool(true)),
                                    ("idle_wakeup", Value::Bool(false)),
                                    ("active_turn_inject", Value::Bool(true))
                                ])
                            )
                        ])])
                    ),
                    ("observed_at", Value::UInt(19))
                ])
            );
            assert_eq!(
                a.state,
                PeerState::Bound {
                    binding: binding(),
                    peers: Some(snapshot())
                }
            );
            let replacement =
                PeerSnapshot::new(address().realm().clone(), vec![], PeerStamp::new(20));
            outcome(
                &mut a,
                ctx,
                Ok(OutcomePayload::PeerSnapshot(replacement.clone())),
            );
            assert_eq!(
                a.state,
                PeerState::Bound {
                    binding: binding(),
                    peers: Some(replacement)
                }
            );
            let closing = lifecycle(&mut a, ctx, circular_runtime::ActorLifecycle::Closing);
            assert!(matches!(
                closing.as_slice(),
                [ActorEffect::Peer(PeerEffect::Unbind { binding: id, .. })] if id == binding().id()
            ));
            assert!(
                outcome(
                    &mut a,
                    ctx,
                    Ok(OutcomePayload::UnbindReceipt(UnbindReceipt::new(
                        binding().id().clone(),
                        address(),
                        PeerStamp::new(24)
                    )))
                )
                .is_empty()
            );
            assert_eq!(a.state, PeerState::Unbound);
            let opened = lifecycle(&mut a, ctx, circular_runtime::ActorLifecycle::Opened);
            assert!(matches!(
                opened.as_slice(),
                [ActorEffect::Peer(PeerEffect::Bind { request, .. })]
                    if request.requested_name().map(PeerDisplayName::as_str) == Some("name")
            ));
            assert_eq!(a.state, PeerState::Binding);
            bind(&mut a, ctx);
            assert!(matches!(a.state, PeerState::Bound { .. }));
        });
    }

    #[test]
    fn receive_emits_then_rearms_and_checkpoint_restores_exact_cursor() {
        drive(|ctx| {
            let mut a = actor();
            let armed = outcome(&mut a, ctx, Ok(OutcomePayload::PeerBinding(binding())));
            assert!(matches!(
                armed.as_slice(),
                [
                    ActorEffect::Emit { .. },
                    ActorEffect::Peer(PeerEffect::Receive { after: None, .. })
                ]
            ));
            let message = PeerMessage::external(
                PeerMessageId::try_new(b"incoming".as_slice()).unwrap(),
                None,
                address(),
                binding().address().clone(),
                Some(address()),
                None,
                PeerBody::try_new("outside text").unwrap(),
                OpaqueProviderFields::empty(),
            );
            let event = |cursor| {
                PeerEventEnvelope::new(
                    binding().id().clone(),
                    PeerCursor::new(cursor),
                    PeerEvent::Inbound(message.clone()),
                )
            };
            let effects = outcome(&mut a, ctx, Ok(OutcomePayload::PeerEnvelope(event(41))));
            let [
                ActorEffect::Emit { port, payload, .. },
                ActorEffect::Peer(PeerEffect::Receive {
                    binding: next_binding,
                    after,
                    ..
                }),
            ] = effects.as_slice()
            else {
                panic!("emit then rearm: {effects:?}");
            };
            assert_eq!(port.as_str(), "message");
            assert_eq!(
                object(payload.value()).unwrap().get("body"),
                Some(&Value::string("outside text"))
            );
            assert_eq!(
                object(payload.value()).unwrap().get("provenance"),
                Some(&Value::string("external_agent"))
            );
            assert_eq!(next_binding, binding().id());
            assert_eq!(*after, Some(PeerCursor::new(41)));
            let checkpoint = a.checkpoint().unwrap();
            let (schema, bytes) = checkpoint.clone().into_parts();
            assert_eq!(schema, 2);
            let Value::Array(fields) =
                circular_core::decode(&bytes, Ceilings::for_boundary(Boundary::ActorState))
                    .unwrap()
            else {
                panic!("state array");
            };
            assert_eq!(fields.len(), 5);
            assert_eq!(fields[4], Value::UInt(41));
            let mut restored = actor();
            restored.restore(checkpoint.clone()).unwrap();
            let stale = outcome(
                &mut restored,
                ctx,
                Ok(OutcomePayload::PeerEnvelope(event(41))),
            );
            assert_error(&stale, "stale");
            assert_eq!(stale.as_slice().len(), 1);
            assert_eq!(restored.checkpoint().unwrap(), checkpoint);
            let next = outcome(
                &mut restored,
                ctx,
                Ok(OutcomePayload::PeerEnvelope(event(42))),
            );
            assert!(
                matches!(next.as_slice(), [ActorEffect::Emit { .. }, ActorEffect::Peer(PeerEffect::Receive { after: Some(cursor), .. })] if cursor.get() == 42)
            );
            let old = ActorState::new(
                1,
                circular_core::encode(
                    &Value::Array(fields[..4].to_vec()),
                    Ceilings::for_boundary(Boundary::ActorState),
                )
                .unwrap(),
            );
            assert!(restored.restore(old).is_err());
        });
    }

    #[test]
    fn a_restart_interrupts_the_binding_and_the_actor_binds_once_again() {
        drive(|ctx| {
            let interrupted = || {
                Err(EffectFailure::InterpreterFault(
                    circular_runtime::InterpreterFault::Interrupted,
                ))
            };
            let is_bind = |effects: &ActorEffects<ProductPayload>| {
                matches!(
                    effects.as_slice(),
                    [ActorEffect::Peer(PeerEffect::Bind { .. })]
                )
            };
            let mut a = actor();
            assert!(is_bind(&lifecycle(
                &mut a,
                ctx,
                circular_runtime::ActorLifecycle::Opened
            )));
            bind(&mut a, ctx);
            assert_eq!(event(&mut a, ctx, "send", send_value()).as_slice().len(), 1);
            assert!(is_bind(&outcome(&mut a, ctx, interrupted())));
            assert_eq!(a.state, PeerState::Binding);
            assert!(outcome(&mut a, ctx, interrupted()).is_empty());
            assert!(is_bind(&outcome(&mut a, ctx, interrupted())));
            bind(&mut a, ctx);
            assert!(matches!(a.state, PeerState::Bound { .. }));
            let closing = lifecycle(&mut a, ctx, circular_runtime::ActorLifecycle::Closing);
            assert_eq!(closing.as_slice().len(), 1);
            assert!(outcome(&mut a, ctx, interrupted()).is_empty());
            assert_eq!(a.state, PeerState::Unbound);
        });
    }

    #[test]
    fn receive_event_arms_preserve_ports_and_rearm_until_unbind() {
        drive(|ctx| {
            let mut a = actor();
            bind(&mut a, ctx);
            let events = [
                (PeerEvent::PeerSnapshotChanged(snapshot()), "peers", true),
                (
                    PeerEvent::DeliveryChanged {
                        message: PeerMessageId::try_new(b"m".as_slice()).unwrap(),
                        state: DeliveryDisposition::Delivered,
                    },
                    "delivery",
                    true,
                ),
                (
                    PeerEvent::AdapterDiagnostic(circular_runtime::PeerDiagnostic::new(
                        "provider", "detail",
                    )),
                    "_error",
                    true,
                ),
                (
                    PeerEvent::BindingStateChanged {
                        binding: binding().id().clone(),
                        state: PeerBindingState::Advertised,
                    },
                    "binding",
                    true,
                ),
                (
                    PeerEvent::BindingStateChanged {
                        binding: binding().id().clone(),
                        state: PeerBindingState::Closed,
                    },
                    "binding",
                    true,
                ),
            ];
            for (index, (event, outlet, rearms)) in events.into_iter().enumerate() {
                let effects = outcome(
                    &mut a,
                    ctx,
                    Ok(OutcomePayload::PeerEnvelope(PeerEventEnvelope::new(
                        binding().id().clone(),
                        PeerCursor::new(index as u64 + 10),
                        event,
                    ))),
                );
                let _ = emitted(&effects, outlet);
                assert_eq!(effects.as_slice().len(), if rearms { 2 } else { 1 });
            }
            assert!(matches!(a.state(), PeerState::Bound { .. }));
            assert_eq!(a.cursor, Some(PeerCursor::new(14)));
        });
    }

    #[test]
    fn checkpoint_round_trips_states_counter_and_restored_send() {
        drive(|ctx| {
            let mut a = actor();
            bind(&mut a, ctx);
            outcome(&mut a, ctx, Ok(OutcomePayload::PeerSnapshot(snapshot())));
            event(&mut a, ctx, "send", send_value());
            let mut restored = actor();
            restored.restore(a.checkpoint().unwrap()).unwrap();
            assert_eq!(restored.state, a.state);
            assert_eq!(restored.next_message, 1);
            let effects = event(&mut restored, ctx, "send", send_value());
            let [ActorEffect::Peer(PeerEffect::Send { request, .. })] = effects.as_slice() else {
                panic!("restored send");
            };
            assert_eq!(
                circular_core::decode(
                    request.message().as_bytes(),
                    Ceilings::for_boundary(Boundary::Identity)
                )
                .unwrap(),
                Value::Array(vec![Value::Bytes(b"incarnation".to_vec()), Value::UInt(1)])
            );
            for state in [
                PeerState::Binding,
                PeerState::Unbinding { binding: binding() },
                PeerState::Unbound,
            ] {
                a.state = state.clone();
                restored.restore(a.checkpoint().unwrap()).unwrap();
                assert_eq!(restored.state, state);
            }
            let b = binding();
            a.state = PeerState::Bound {
                binding: PeerBinding::new(
                    b.id().clone(),
                    b.actor().clone(),
                    b.address().clone(),
                    b.effective_name().clone(),
                    BindingLease::Until(PeerStamp::new(99)),
                    b.capabilities(),
                ),
                peers: None,
            };
            restored.restore(a.checkpoint().unwrap()).unwrap();
            assert_eq!(restored.state, a.state);
            assert_eq!(
                a.on_config_change(&FoldedConfig::minted(ActorType::Peer, config())),
                ConfigChangeOutcome::ReplaceIncarnation
            );
        });
    }

    #[test]
    fn restore_rejects_old_broken_and_partial_state_atomically() {
        drive(|ctx| {
            let mut a = actor();
            bind(&mut a, ctx);
            let before = a.checkpoint().unwrap();
            for state in [
                ActorState::new(0, vec![]),
                ActorState::new(2, vec![]),
                ActorState::new(1, vec![255]),
                ActorState::new(
                    1,
                    circular_core::encode(
                        &Value::Null,
                        Ceilings::for_boundary(Boundary::ActorState),
                    )
                    .unwrap(),
                ),
                ActorState::new(
                    1,
                    circular_core::encode(
                        &Value::Array(vec![
                            Value::string("bound"),
                            Value::Null,
                            Value::Null,
                            Value::UInt(0),
                        ]),
                        Ceilings::for_boundary(Boundary::ActorState),
                    )
                    .unwrap(),
                ),
                ActorState::new(
                    1,
                    circular_core::encode(
                        &Value::Array(vec![
                            Value::string("binding"),
                            binding_value(&binding()),
                            Value::Null,
                            Value::UInt(0),
                        ]),
                        Ceilings::for_boundary(Boundary::ActorState),
                    )
                    .unwrap(),
                ),
            ] {
                assert!(a.restore(state).is_err());
                assert_eq!(a.checkpoint().unwrap(), before);
            }
        });
    }
}
