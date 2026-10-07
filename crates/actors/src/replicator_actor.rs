
use std::collections::{BTreeMap, BTreeSet};
use std::marker::PhantomData;

use circular_core::{Boundary, Ceilings, Value};
use circular_runtime::{
    ActorContext, ActorEffects, ActorInput, ActorRestoreError, ActorState, ActorTypes,
    ConfigChangeOutcome, EditableActor, Effect, EmittingActor, EmittingActorFactory, FoldedConfig,
    InstanceAuthority, InstanceAuthorityBearer, InstanceDisposition, InstanceGovernor,
    InstanceIntent, InstanceKey, InstanceMutationSpec, ScheduleSpec,
};

use crate::arming::ArmingCorrelation;
use crate::config::{ConfigRejection, PositiveCount, Slot};
use crate::instance_value::{
    instance_key_from_payload_scalar, instance_key_from_value, instance_key_to_value,
};
use crate::{ActorType, ProductPayload};

pub const REPLICATOR_STATE_SCHEMA: u16 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct ReplicatorRouting {
    at: crate::PayloadPath,
    ttl: circular_runtime::NonZeroMillis,
}

impl ReplicatorRouting {
    pub const AT: &'static str = "at";
    pub const TTL: &'static str = "ttl";

    pub fn from_value(value: &Value) -> Result<Self, ReplicatorRoutingError> {
        let Some(root) = value.as_object() else {
            return Err(ReplicatorRoutingError::NotAnObject);
        };
        let at = root
            .get(Self::AT)
            .ok_or(ReplicatorRoutingError::MissingAt)
            .and_then(|at| {
                crate::config::payload_path_from_value(at).map_err(ReplicatorRoutingError::At)
            })?;
        let ttl = root
            .get(Self::TTL)
            .ok_or(ReplicatorRoutingError::MissingTtl)?;
        let ttl = match crate::actor_support::read_config_unsigned(ttl) {
            crate::actor_support::ConfigUnsigned::Value(value) => value,
            crate::actor_support::ConfigUnsigned::Negative(got) => {
                return Err(ReplicatorRoutingError::TtlNegative { got });
            }
            crate::actor_support::ConfigUnsigned::NotAnInteger => {
                return Err(ReplicatorRoutingError::TtlNotAnInteger);
            }
        };
        let ttl = circular_runtime::NonZeroMillis::new(ttl)
            .map_err(|_| ReplicatorRoutingError::TtlZero)?;
        Ok(Self { at, ttl })
    }

    #[must_use]
    pub const fn at(&self) -> &crate::PayloadPath {
        &self.at
    }

    #[must_use]
    pub const fn ttl(&self) -> circular_runtime::NonZeroMillis {
        self.ttl
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplicatorRoutingError {
    NotAnObject,
    MissingAt,
    At(crate::config::PayloadPathError),
    MissingTtl,
    TtlNotAnInteger,
    TtlNegative {
        got: i64,
    },
    TtlZero,
}

impl core::fmt::Display for ReplicatorRoutingError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotAnObject => formatter.write_str("replicator config is not an object"),
            Self::MissingAt => formatter.write_str("replicator config is missing at"),
            Self::At(error) => write!(formatter, "replicator at is not an exact path: {error}"),
            Self::MissingTtl => formatter.write_str("replicator config is missing ttl"),
            Self::TtlNotAnInteger => formatter.write_str("replicator ttl is not an integer"),
            Self::TtlNegative { got } => write!(formatter, "replicator ttl is negative: {got}"),
            Self::TtlZero => formatter.write_str("replicator ttl is zero"),
        }
    }
}

impl std::error::Error for ReplicatorRoutingError {}

#[derive(Clone, Debug, PartialEq)]
pub struct ReplicatorConfig {
    routing: ReplicatorRouting,
    capacity: u64,
}

impl ReplicatorConfig {
    pub fn from_value(value: &Value) -> Result<Self, ReplicatorFactoryError> {
        let routing =
            ReplicatorRouting::from_value(value).map_err(ReplicatorFactoryError::Routing)?;
        let capacity = circular_core::Fields::open(value)
            .map_err(ConfigRejection::NotObject)
            .and_then(|mut fields| {
                crate::get(ActorType::Replicator)
                    .config()
                    .read(&mut fields, &CAPACITY)
            })
            .map_err(ReplicatorFactoryError::Capacity)?;
        Ok(Self {
            routing,
            capacity: capacity.get(),
        })
    }

    #[must_use]
    pub const fn at(&self) -> &crate::PayloadPath {
        self.routing.at()
    }
    #[must_use]
    pub const fn ttl(&self) -> circular_runtime::NonZeroMillis {
        self.routing.ttl()
    }
    #[must_use]
    pub const fn capacity(&self) -> u64 {
        self.capacity
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReplicatorFactoryError {
    Routing(ReplicatorRoutingError),
    Capacity(ConfigRejection),
    NoAuthority,
}

pub(crate) const CAPACITY: Slot<PositiveCount> = Slot::new("capacity", PositiveCount);

impl core::fmt::Display for ReplicatorFactoryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Routing(error) => write!(f, "{error}"),
            Self::Capacity(rejection) => write!(f, "{rejection}"),
            Self::NoAuthority => f.write_str("replicator has no authority to mint instances"),
        }
    }
}
impl std::error::Error for ReplicatorFactoryError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CellArm {
    minted: u64,
    current: ArmingCorrelation,
}
impl CellArm {
    pub const fn get(&self) -> u64 {
        self.current.get()
    }
}

pub struct ReplicatorActor<S, V, I> {
    ttl_arm: BTreeMap<InstanceKey, CellArm>,
    last_correlation: u64,
    config: ReplicatorConfig,
    authority: InstanceAuthority<S>,
    marker: PhantomData<fn() -> (V, I)>,
}

impl<S, V, I> ReplicatorActor<S, V, I> {
    #[must_use]
    pub fn new(config: ReplicatorConfig, authority: InstanceAuthority<S>) -> Self {
        Self {
            ttl_arm: BTreeMap::new(),
            last_correlation: 0,
            config,
            authority,
            marker: PhantomData,
        }
    }

    #[must_use]
    pub const fn config(&self) -> &ReplicatorConfig {
        &self.config
    }
    #[must_use]
    pub const fn authority(&self) -> &InstanceAuthority<S> {
        &self.authority
    }
    #[must_use]
    pub const fn ttl_arm(&self) -> &BTreeMap<InstanceKey, CellArm> {
        &self.ttl_arm
    }

    #[must_use]
    pub fn event_intents(&self, payload: &Value) -> Option<Vec<InstanceIntent>> {
        let key = crate::route_config::select(payload, self.config.at())
            .and_then(instance_key_from_payload_scalar)?;
        let mut intents = Vec::new();
        if !self.ttl_arm.contains_key(&key) {
            let mut activity: Vec<_> = self.ttl_arm.iter().collect();
            activity.sort_by(|(a_key, a), (b_key, b)| {
                a.minted.cmp(&b.minted).then_with(|| a_key.cmp(b_key))
            });
            let retire_count = (activity.len() as u64)
                .saturating_add(1)
                .saturating_sub(self.config.capacity());
            for (victim, _) in activity
                .into_iter()
                .take(usize::try_from(retire_count).expect("at most the ledger size"))
            {
                intents.push(InstanceIntent::Retire {
                    key: victim.clone(),
                });
            }
        }
        intents.push(InstanceIntent::Instantiate { key });
        Some(intents)
    }

    #[must_use]
    pub fn timer_intent(&self, payload: &Value) -> Option<InstanceIntent> {
        let Value::UInt(fired) = payload else {
            return None;
        };
        self.ttl_arm.iter().find_map(|(key, armed)| {
            (armed.get() == *fired).then(|| InstanceIntent::Retire { key: key.clone() })
        })
    }

    pub fn record_disposition(
        &mut self,
        intent: &InstanceIntent,
        disposition: InstanceDisposition,
    ) -> Option<ScheduleSpec> {
        match (intent, disposition) {
            (InstanceIntent::Instantiate { key }, InstanceDisposition::Minted) => {
                let next = self
                    .last_correlation
                    .checked_add(1)
                    .expect("replicator arming correlation exhausted its u64 space");
                let correlation = ArmingCorrelation::new(next);
                let minted = self.ttl_arm.get(key).map_or(next, |arm| arm.minted);
                self.ttl_arm.insert(
                    key.clone(),
                    CellArm {
                        minted,
                        current: correlation,
                    },
                );
                self.last_correlation = next;
                Some(ScheduleSpec::new(
                    self.config.ttl(),
                    correlation.correlation(),
                ))
            }
            (
                InstanceIntent::Retire { key },
                InstanceDisposition::Retired | InstanceDisposition::NotLive,
            ) => {
                self.ttl_arm.remove(key);
                None
            }
            _ => None,
        }
    }

    pub fn apply_event<G: InstanceGovernor<S>>(
        &mut self,
        payload: &Value,
        governor: &mut G,
    ) -> Option<ScheduleSpec> {
        let intents = self.event_intents(payload)?;
        let mut schedule = None;
        for intent in intents {
            let disposition = governor.apply(&self.authority, intent.clone());
            schedule = self.record_disposition(&intent, disposition);
        }
        schedule
    }

    pub fn apply_timer<G: InstanceGovernor<S>>(
        &mut self,
        payload: &Value,
        governor: &mut G,
    ) -> Option<InstanceDisposition> {
        let intent = self.timer_intent(payload)?;
        let disposition = governor.apply(&self.authority, intent.clone());
        self.record_disposition(&intent, disposition);
        Some(disposition)
    }

    fn intent_effect(&self, intent: InstanceIntent) -> ActorEffects<ProductPayload> {
        ActorEffects::external(Effect::mutate_instance(InstanceMutationSpec::requested(
            &self.authority,
            intent,
        )))
    }
}

impl<T, S> EmittingActor<T, ProductPayload> for ReplicatorActor<S, T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        let payload = input.payload::<T>().value();
        if input.inlet().as_str() == crate::TIMER_PORT_NAME {
            return self
                .timer_intent(payload)
                .map_or_else(ActorEffects::empty, |intent| self.intent_effect(intent));
        }
        self.event_intents(payload)
            .unwrap_or_default()
            .into_iter()
            .fold(ActorEffects::empty(), |effects, intent| {
                effects.concat(self.intent_effect(intent))
            })
    }
}

impl<S, V, I> EditableActor for ReplicatorActor<S, V, I>
where
    V: Clone + From<u16> + PartialEq,
    I: Clone + Ord,
{
    type StateVersion = V;
    type EffectId = I;

    fn on_instance_disposition(
        &mut self,
        intent: &InstanceIntent,
        disposition: InstanceDisposition,
    ) -> Option<ScheduleSpec> {
        self.record_disposition(intent, disposition)
    }

    fn instance_expiry(&self, intent: &InstanceIntent) -> Option<ScheduleSpec> {
        let InstanceIntent::Retire { key } = intent else {
            return None;
        };
        self.ttl_arm
            .get(key)
            .map(|arm| ScheduleSpec::new(self.config.ttl(), arm.current.correlation()))
    }

    fn instance_retirements(&self) -> Vec<InstanceIntent> {
        self.ttl_arm
            .keys()
            .map(|key| InstanceIntent::Retire { key: key.clone() })
            .collect()
    }

    fn on_config_change(&mut self, config: &FoldedConfig) -> ConfigChangeOutcome {
        let Ok(next) = ReplicatorConfig::from_value(config.value()) else {
            return ConfigChangeOutcome::ReplaceIncarnation;
        };
        if self.config.at() != next.at() {
            return ConfigChangeOutcome::ReplaceIncarnation;
        }
        self.config = next;
        ConfigChangeOutcome::Absorbed
    }

    fn checkpoint(&self) -> Option<ActorState<V>> {
        let arms = self
            .ttl_arm
            .iter()
            .map(|(key, correlation)| {
                Value::Array(vec![
                    instance_key_to_value(key),
                    Value::UInt(correlation.minted),
                    Value::UInt(correlation.get()),
                ])
            })
            .collect();
        let value = Value::Array(vec![Value::UInt(self.last_correlation), Value::Array(arms)]);
        let bytes = circular_core::encode(&value, Ceilings::for_boundary(Boundary::ActorState))
            .expect("canonical ledger encoding");
        Some(ActorState::new(V::from(REPLICATOR_STATE_SCHEMA), bytes))
    }

    fn restore(&mut self, state: ActorState<V>) -> Result<(), ActorRestoreError<V>> {
        let (schema, bytes) = state.into_parts();
        if schema != V::from(REPLICATOR_STATE_SCHEMA) {
            return Err(ActorRestoreError::SchemaBeyondLadder { schema });
        }
        let decode = || ActorRestoreError::DecodeFailed {
            schema: schema.clone(),
        };
        let invariant = || ActorRestoreError::StateInvariantViolated {
            schema: schema.clone(),
        };
        let value = circular_core::decode(&bytes, Ceilings::for_boundary(Boundary::ActorState))
            .map_err(|_| decode())?;
        let Value::Array(fields) = value else {
            return Err(decode());
        };
        let [Value::UInt(last), Value::Array(rows)] = fields.as_slice() else {
            return Err(decode());
        };
        let mut arms = BTreeMap::new();
        let mut correlations = BTreeSet::new();
        for row in rows {
            let Value::Array(fields) = row else {
                return Err(decode());
            };
            let [key, Value::UInt(minted), Value::UInt(correlation)] = fields.as_slice() else {
                return Err(decode());
            };
            let key = instance_key_from_value(key).map_err(|_| invariant())?;
            if *minted == 0
                || minted > correlation
                || *correlation == 0
                || correlation > last
                || !correlations.insert(*correlation)
                || arms
                    .last_key_value()
                    .is_some_and(|(previous, _)| previous >= &key)
            {
                return Err(invariant());
            }
            arms.insert(
                key,
                CellArm {
                    minted: *minted,
                    current: ArmingCorrelation::new(*correlation),
                },
            );
        }
        self.ttl_arm = arms;
        self.last_correlation = *last;
        Ok(())
    }
}

pub struct ReplicatorFactory<T>(PhantomData<fn() -> T>);
impl<T> EmittingActorFactory<ProductPayload> for ReplicatorFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
    T::Grants: InstanceAuthorityBearer,
    InstanceAuthority<<T::Grants as InstanceAuthorityBearer>::Seal>: Clone,
{
    const TYPE: ActorType = ActorType::Replicator;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::AbsorbsSome;
    const CHECKPOINTS: bool = true;
    type Grants = T::Grants;
    type Types = T;
    type Instance =
        ReplicatorActor<<T::Grants as InstanceAuthorityBearer>::Seal, T::StateVersion, T::EffectId>;
    type Error = ReplicatorFactoryError;

    fn create(config: &FoldedConfig, grants: &Self::Grants) -> Result<Self::Instance, Self::Error> {
        let config = ReplicatorConfig::from_value(config.value())?;
        let authority = grants
            .instance_authority()
            .ok_or(ReplicatorFactoryError::NoAuthority)?
            .clone();
        Ok(ReplicatorActor::new(config, authority))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use circular_core::{StreamIdentity, Tick};
    use circular_plan::{
        ActorFlags, Config, ConfigValue, Generation, GenerationVector, Incarnation, Name,
        NamedActorId, ScopeId, admit_template,
    };
    use circular_runtime::ActorEffect;
    use circular_runtime::{InstanceRegistry, InstanceScalar};

    type Actor = ReplicatorActor<(), u16, u64>;
    struct Grants(Option<InstanceAuthority<()>>);
    impl InstanceAuthorityBearer for Grants {
        type Seal = ();
        fn instance_authority(&self) -> Option<&InstanceAuthority<()>> {
            self.0.as_ref()
        }
    }
    impl circular_runtime::AgentHarnessAuthorityBearer for Grants {
        fn agent_harness_authority(
            &self,
        ) -> Option<circular_runtime::Granted<circular_runtime::AgentHarness>> {
            None
        }
    }
    impl circular_runtime::ProcessAuthorityBearer for Grants {
        fn process_spawn_authority(
            &self,
        ) -> Option<circular_runtime::Granted<circular_runtime::ProcessSpawn>> {
            None
        }
    }
    impl circular_runtime::UserNotifyAuthorityBearer for Grants {
        fn user_notify_authority(
            &self,
        ) -> Option<circular_runtime::Granted<circular_runtime::UserNotify>> {
            None
        }
    }
    impl circular_runtime::PeerAuthorityBearer for Grants {
        fn peer_discover_authority(
            &self,
        ) -> Option<circular_runtime::Granted<circular_runtime::PeerDiscover>> {
            None
        }
        fn peer_send_authority(
            &self,
        ) -> Option<circular_runtime::Granted<circular_runtime::PeerSend>> {
            None
        }
        fn peer_advertise_authority(
            &self,
        ) -> Option<circular_runtime::Granted<circular_runtime::PeerAdvertise>> {
            None
        }
        fn peer_receive_authority(
            &self,
        ) -> Option<circular_runtime::Granted<circular_runtime::PeerReceive>> {
            None
        }
    }
    impl circular_runtime::HttpFetchAuthorityBearer for Grants {
        fn http_fetch_authority(
            &self,
        ) -> Option<circular_runtime::Granted<circular_runtime::HttpFetch>> {
            None
        }
    }
    impl circular_runtime::FilesystemAuthorityBearer for Grants {
        fn fs_read_authority(&self) -> Option<circular_runtime::Granted<circular_runtime::FsRead>> {
            None
        }
        fn fs_write_authority(
            &self,
        ) -> Option<circular_runtime::Granted<circular_runtime::FsWrite>> {
            None
        }
    }
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
        type Grants = Grants;
        fn payload(event: &ProductPayload) -> &ProductPayload {
            event
        }
    }

    fn authority() -> InstanceAuthority<()> {
        let scalar = |value: Value| ConfigValue::Scalar {
            tag: Name::from_normalized(circular_core::CANONICAL_VALUE_TAG),
            bytes: circular_core::encode(
                &value,
                circular_core::Ceilings::for_boundary(circular_core::Boundary::Config),
            )
            .unwrap()
            .into_boxed_slice(),
        };
        let roles = {
            let mut table = circular_plan::ScopeRoleTable::new();
            table.declare(
                circular_plan::ScopeId::from_segments(vec![circular_plan::ScopeSeg::Child(
                    Name::from_normalized("cell"),
                )])
                .unwrap(),
                circular_plan::ScopeRole::Template,
            );
            table
        };
        InstanceAuthority::granted(
            &admit_template(&roles, &ScopeId::root(), &Name::from_normalized("cell")).unwrap(),
        )
    }
    fn config(at: &[&str], ttl: i64, capacity: i64) -> FoldedConfig {
        FoldedConfig::minted(
            ActorType::Replicator,
            Value::object([
                (
                    "at",
                    Value::Array(at.iter().map(|s| Value::string(*s)).collect()),
                ),
                ("ttl", Value::Int(ttl)),
                ("capacity", Value::Int(capacity)),
            ])
            .unwrap(),
        )
    }
    fn actor(capacity: i64) -> Actor {
        ReplicatorFactory::<Types>::create(&config(&[], 100, capacity), &Grants(Some(authority())))
            .unwrap()
    }
    fn key(s: &str) -> InstanceKey {
        InstanceKey::Scalar(InstanceScalar::normalized_text(s))
    }
    fn mint(s: &str) -> InstanceIntent {
        InstanceIntent::Instantiate { key: key(s) }
    }
    fn retire(s: &str) -> InstanceIntent {
        InstanceIntent::Retire { key: key(s) }
    }

    #[derive(Default)]
    struct Governor {
        registry: InstanceRegistry<()>,
        calls: Vec<(InstanceIntent, InstanceDisposition)>,
    }
    impl InstanceGovernor<()> for Governor {
        fn apply(
            &mut self,
            authority: &InstanceAuthority<()>,
            intent: InstanceIntent,
        ) -> InstanceDisposition {
            let disposition = self.registry.apply(authority, intent.clone());
            self.calls.push((intent, disposition));
            disposition
        }
        fn live(&self, authority: &InstanceAuthority<()>) -> impl Iterator<Item = &InstanceKey> {
            self.registry.live(authority)
        }
    }
    fn event(actor: &mut Actor, governor: &mut Governor, s: &str) -> ScheduleSpec {
        actor.apply_event(&Value::string(s), governor).unwrap()
    }
    fn hook(actor: &mut Actor, inlet: &str, value: Value) -> ActorEffects<ProductPayload> {
        let named = NamedActorId::new(ScopeId::root(), Name::from_normalized("cell"));
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .unwrap();
        let incarnation = Incarnation::new(Run, named.as_scoped().clone(), generations);
        let actor_id = named.as_actor_id();
        let config = Config::default();
        let grants = Grants(Some(authority()));
        let context = ActorContext::new(&actor_id, &incarnation, &config, &grants);
        let payload = ProductPayload::new(
            crate::GroundShape::try_new(crate::Shape::Any).unwrap(),
            value,
        );
        <Actor as EmittingActor<Types, ProductPayload>>::on_event(
            actor,
            &ActorInput::new(circular_core::PortId::try_new(inlet).unwrap(), payload),
            &context,
        )
    }

    #[test]
    fn exact_path_reads_text_integer_and_bool_and_rejects_other_values_without_intents() {
        let mut actor = ReplicatorFactory::<Types>::create(
            &config(&["session", "id"], 100, 8),
            &Grants(Some(authority())),
        )
        .unwrap();
        let mut governor = Governor::default();
        for value in [Value::string("s"), Value::Int(-7), Value::Bool(true)] {
            let expected = instance_key_from_payload_scalar(&value).unwrap();
            let payload =
                Value::object([("session", Value::object([("id", value)]).unwrap())]).unwrap();
            assert!(actor.apply_event(&payload, &mut governor).is_some());
            assert_eq!(
                governor.calls.last(),
                Some(&(
                    InstanceIntent::Instantiate { key: expected },
                    InstanceDisposition::Minted
                ))
            );
        }
        let before = actor.checkpoint();
        for payload in [
            Value::Null,
            Value::object([(
                "session",
                Value::object([("id", Value::Array(vec![]))]).unwrap(),
            )])
            .unwrap(),
        ] {
            assert!(actor.apply_event(&payload, &mut governor).is_none());
        }
        assert_eq!(governor.calls.len(), 3);
        assert_eq!(actor.checkpoint(), before);
    }

    #[test]
    fn already_live_keeps_the_original_expiry_without_rearming() {
        let mut actor = actor(1);
        let mut governor = Governor::default();
        let first = event(&mut actor, &mut governor, "a");
        let before = actor.checkpoint();
        let second = actor.apply_event(&Value::string("a"), &mut governor);
        assert_eq!(
            governor.calls,
            vec![
                (mint("a"), InstanceDisposition::Minted),
                (mint("a"), InstanceDisposition::AlreadyLive)
            ]
        );
        assert_eq!(governor.registry.count(actor.authority()), 1);
        assert!(second.is_none());
        assert_eq!(actor.checkpoint(), before);
        assert_eq!(actor.instance_expiry(&retire("a")).unwrap(), first);
    }

    #[test]
    fn capacity_retires_the_oldest_minted_key_before_minting() {
        let mut actor = actor(2);
        let mut governor = Governor::default();
        let stale = event(&mut actor, &mut governor, "a");
        event(&mut actor, &mut governor, "b");
        assert!(
            actor
                .apply_event(&Value::string("a"), &mut governor)
                .is_none()
        );
        event(&mut actor, &mut governor, "c");
        assert_eq!(
            &governor.calls[3..],
            &[
                (retire("a"), InstanceDisposition::Retired),
                (mint("c"), InstanceDisposition::Minted)
            ]
        );
        assert_eq!(governor.registry.count(actor.authority()), 2);
        assert!(
            actor
                .apply_timer(&Value::UInt(stale.correlation().get()), &mut governor)
                .is_none()
        );
    }

    #[test]
    fn equal_activity_uses_the_smallest_instance_key() {
        let mut actor = actor(2);
        let mut governor = Governor::default();
        event(&mut actor, &mut governor, "z");
        event(&mut actor, &mut governor, "a");
        actor.ttl_arm.get_mut(&key("z")).unwrap().minted = 2;
        event(&mut actor, &mut governor, "next");
        assert_eq!(governor.calls[2].0, retire("a"));
    }

    #[test]
    fn ttl_retires_current_arm_and_suppresses_retired_and_reminted_fires() {
        let mut actor = actor(2);
        let mut governor = Governor::default();
        let old = event(&mut actor, &mut governor, "a");
        assert_eq!(
            actor.apply_timer(&Value::UInt(old.correlation().get()), &mut governor),
            Some(InstanceDisposition::Retired)
        );
        let current = event(&mut actor, &mut governor, "a");
        let old = Value::UInt(old.correlation().get());
        let current = Value::UInt(current.correlation().get());
        assert!(actor.apply_timer(&old, &mut governor).is_none());
        assert_eq!(
            actor.apply_timer(&current, &mut governor),
            Some(InstanceDisposition::Retired)
        );
        assert_eq!(governor.registry.count(actor.authority()), 0);
        assert!(actor.ttl_arm().is_empty());
        let fresh = event(&mut actor, &mut governor, "a");
        assert!(
            fresh.correlation().get()
                > match current {
                    Value::UInt(value) => value,
                    _ => unreachable!(),
                }
        );
        assert!(actor.apply_timer(&current, &mut governor).is_none());
        assert!(actor.apply_timer(&Value::Int(3), &mut governor).is_none());
        let effects = hook(&mut actor, crate::TIMER_PORT_NAME, current);
        assert!(effects.iter().next().is_none());
    }

    #[test]
    fn rejected_capacity_never_arms_or_changes_the_checkpoint() {
        let mut actor = actor(2);
        let mut governor = Governor::default();
        governor.registry.set_capacity(actor.authority(), 0);
        let before = actor.checkpoint();
        assert!(
            actor
                .apply_event(&Value::string("a"), &mut governor)
                .is_none()
        );
        assert_eq!(
            governor.calls,
            vec![(mint("a"), InstanceDisposition::RejectedCapacity { max: 0 })]
        );
        assert_eq!(actor.checkpoint(), before);
    }

    #[test]
    fn config_absorbs_ttl_for_expiry_and_capacity_for_minting_and_restarts_for_at() {
        let mut actor = actor(3);
        let mut governor = Governor::default();
        for s in ["a", "b", "c"] {
            event(&mut actor, &mut governor, s);
        }
        let before = actor.checkpoint();
        assert_eq!(
            actor.on_config_change(&config(&[], 500, 1)),
            ConfigChangeOutcome::Absorbed
        );
        assert_eq!(actor.checkpoint(), before);
        assert_eq!(governor.calls.len(), 3);
        assert!(
            actor
                .apply_event(&Value::string("c"), &mut governor)
                .is_none()
        );
        assert_eq!(
            actor
                .instance_expiry(&retire("c"))
                .unwrap()
                .after()
                .get()
                .get(),
            500
        );
        assert_eq!(
            governor.calls.len(),
            4,
            "an existing key does not enforce a capacity reduction"
        );
        event(&mut actor, &mut governor, "d");
        assert_eq!(
            &governor.calls[4..],
            &[
                (retire("a"), InstanceDisposition::Retired),
                (retire("b"), InstanceDisposition::Retired),
                (retire("c"), InstanceDisposition::Retired),
                (mint("d"), InstanceDisposition::Minted)
            ]
        );
        let before = actor.checkpoint();
        let old_config = actor.config().clone();
        assert_eq!(
            actor.on_config_change(&config(&["other"], 900, 5)),
            ConfigChangeOutcome::ReplaceIncarnation
        );
        assert_eq!(actor.config(), &old_config);
        assert_eq!(actor.checkpoint(), before);
        assert_eq!(
            actor.on_config_change(&config(&[], 0, 1)),
            ConfigChangeOutcome::ReplaceIncarnation
        );
    }

    #[test]
    fn checkpoint_restores_only_ledgers_and_keeps_stale_fires_stale() {
        let mut actor = actor(2);
        let mut governor = Governor::default();
        let old = event(&mut actor, &mut governor, "a");
        event(&mut actor, &mut governor, "b");
        assert!(
            actor
                .apply_event(&Value::string("a"), &mut governor)
                .is_none()
        );
        assert_eq!(
            actor.apply_timer(&Value::UInt(old.correlation().get()), &mut governor),
            Some(InstanceDisposition::Retired)
        );
        let current = event(&mut actor, &mut governor, "a");
        let checkpoint = actor.checkpoint().unwrap();
        let mut restored = self::actor(2);
        restored.restore(checkpoint.clone()).unwrap();
        assert_eq!(restored.checkpoint(), Some(checkpoint));
        let mut fresh_governor = Governor::default();
        assert_eq!(
            fresh_governor.registry.count(restored.authority()),
            0,
            "a checkpoint does not create a cell"
        );
        assert!(
            restored
                .apply_timer(&Value::UInt(old.correlation().get()), &mut fresh_governor)
                .is_none()
        );
        assert_eq!(
            restored.apply_timer(&Value::UInt(current.correlation().get()), &mut governor),
            Some(InstanceDisposition::Retired)
        );
        let next = event(&mut restored, &mut governor, "c");
        assert!(next.correlation().get() > current.correlation().get());
    }

    #[test]
    fn restore_rejects_unknown_schema_malformed_and_noncanonical_ledger_atomically() {
        let mut actor = actor(2);
        event(&mut actor, &mut Governor::default(), "keep");
        let before = actor.checkpoint();
        assert!(matches!(
            actor.restore(ActorState::new(99, vec![])),
            Err(ActorRestoreError::SchemaBeyondLadder { .. })
        ));
        assert!(matches!(
            actor.restore(ActorState::new(REPLICATOR_STATE_SCHEMA, vec![255])),
            Err(ActorRestoreError::DecodeFailed { .. })
        ));
        for rows in [
            vec![("b", 1), ("a", 2)],
            vec![("a", 1), ("b", 1)],
            vec![("a", 0)],
            vec![("a", 3)],
            vec![("a", 1), ("a", 2)],
        ] {
            let value = Value::Array(vec![
                Value::UInt(2),
                Value::Array(
                    rows.into_iter()
                        .map(|(s, c)| {
                            Value::Array(vec![
                                instance_key_to_value(&key(s)),
                                Value::UInt(c),
                                Value::UInt(c),
                            ])
                        })
                        .collect(),
                ),
            ]);
            let bytes = circular_core::encode(&value, Ceilings::for_boundary(Boundary::ActorState))
                .unwrap();
            assert!(matches!(
                actor.restore(ActorState::new(REPLICATOR_STATE_SCHEMA, bytes)),
                Err(ActorRestoreError::StateInvariantViolated { .. })
            ));
            assert_eq!(actor.checkpoint(), before);
        }
    }

    #[test]
    fn factory_requires_all_three_valid_slots_and_instance_authority() {
        let good = config(&[], 100, 2);
        assert!(matches!(
            ReplicatorFactory::<Types>::create(&good, &Grants(None)),
            Err(ReplicatorFactoryError::NoAuthority)
        ));
        for slot in ["at", "ttl", "capacity"] {
            let value = Value::object(
                [
                    ("at", Value::Array(vec![])),
                    ("ttl", Value::Int(100)),
                    ("capacity", Value::Int(2)),
                ]
                .into_iter()
                .filter(|(name, _)| *name != slot),
            )
            .unwrap();
            assert!(ReplicatorConfig::from_value(&value).is_err());
        }
        for (ttl, capacity) in [(0, 1), (1, 0), (-1, 1), (1, -1)] {
            assert!(
                ReplicatorFactory::<Types>::create(
                    &config(&[], ttl, capacity),
                    &Grants(Some(authority()))
                )
                .is_err()
            );
        }
        for invalid in [Value::float(1.0), Value::string("1"), Value::Null] {
            let value = Value::object([
                ("at", Value::Array(vec![])),
                ("ttl", Value::Int(100)),
                ("capacity", invalid),
            ])
            .unwrap();
            assert!(ReplicatorConfig::from_value(&value).is_err());
        }
    }

    #[test]
    fn catalog_factory_and_editability_publish_the_actor_after_engine_boarding() {
        let row = crate::editability::editability(ActorType::Replicator).unwrap();
        assert!(row.boarded && row.checkpoints);
        assert_eq!(row.boarded_without, None);
        assert_eq!(
            crate::registration(ActorType::Replicator).factory(),
            crate::FactoryArm::Actor
        );
        let mut actor = crate::product_actor_factory::<Types>(ActorType::Replicator)
            .unwrap()
            .create(
                &config(&[], 100, 2),
                &Grants(Some(authority())),
                &crate::ResolvedInletShapes::default(),
            )
            .unwrap();
        assert!(matches!(actor, crate::ProductActor::Replicator(_)));
        assert_eq!(
            actor.on_config_change(&config(&[], 200, 1)),
            ConfigChangeOutcome::Absorbed
        );
        assert_eq!(
            actor.on_config_change(&config(&["id"], 200, 1)),
            ConfigChangeOutcome::ReplaceIncarnation
        );
        assert!(actor.checkpoint().is_some());
    }

    #[test]
    fn product_hook_emits_only_intents_and_does_not_claim_cells_or_arm_without_dispositions() {
        let mut actor = actor(2);
        let before = actor.checkpoint();
        let governor = Governor::default();
        let effects = hook(&mut actor, "event", Value::string("a"));
        assert_eq!(effects.iter().count(), 1);
        assert!(
            matches!(effects.iter().next(), Some(ActorEffect::External(Effect::MutateInstance { spec })) if spec.intent() == &mint("a"))
        );
        assert_eq!(actor.checkpoint(), before);
        assert_eq!(governor.registry.count(actor.authority()), 0);
        assert!(
            hook(&mut actor, "event", Value::Null)
                .iter()
                .next()
                .is_none()
        );
        actor
            .record_disposition(&mint("a"), InstanceDisposition::Minted)
            .unwrap();
        let current = actor.ttl_arm()[&key("a")].get();
        let effects = hook(&mut actor, crate::TIMER_PORT_NAME, Value::UInt(current));
        assert!(
            matches!(effects.iter().next(), Some(ActorEffect::External(Effect::MutateInstance { spec })) if spec.intent() == &retire("a"))
        );
    }
}
