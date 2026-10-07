
#[cfg(test)]
use crate::GroundShape;
use crate::actor_support::error_payload;
use crate::arming::ArmingCorrelation;
use crate::config::{ConfigRejection, Interval, Slot, Spelled};
use crate::{ActorType, ERROR_PORT_NAME, Flow, ProductPayload, ProductValue, TIMER_PORT_NAME};
use circular_core::{Boundary, Ceilings, NonZeroMillis, PortId};
use circular_runtime::{
    ActorContext, ActorEffect, ActorEffects, ActorInput, ActorRestoreError, ActorState, ActorTypes,
    ConfigChangeOutcome, EditableActor, Effect, EffectOutcome, EmittingActor, EmittingActorFactory,
    FoldedConfig, NotificationChannel, NotificationSpec, OutcomePayload, ProcessingCause,
    ReasonDecl, ScheduleSpec, Suppression, UserNotifyAuthorityBearer,
};
use std::collections::VecDeque;
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::sync::LazyLock;

const NOTIFY_STATE_SCHEMA: u16 = 1;

static SUPPRESSION_REASONS: LazyLock<ReasonDecl<Suppression>> = LazyLock::new(|| {
    let declared = crate::get(ActorType::Notify)
        .config()
        .suppress()
        .expect("notify suppression declaration");
    ReasonDecl::try_from_names(
        declared
            .iter()
            .map(|(name, _)| name.as_str())
            .chain([crate::STALE_TIMER_FIRE]),
    )
    .expect("registered cooling and the existing stale-schedule reason")
});

fn port(name: &str) -> PortId {
    PortId::try_new(name).expect("registered notify port names are canonical")
}

fn suppress(reason: &str) -> ActorEffects<ProductPayload> {
    ActorEffects::singleton(ActorEffect::suppress(
        SUPPRESSION_REASONS
            .resolve(reason)
            .expect("declared notify suppression reason"),
    ))
}

fn suppress_stale() -> ActorEffects<ProductPayload> {
    suppress(crate::STALE_TIMER_FIRE)
}

circular_core::closed_table! {
    pub enum DuringInterval {
        Suppress => "suppress",
        Latest => "latest",
        Queue => "queue",
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotifyConfig {
    channel: NotificationChannel,
    minimum_interval: Option<NonZeroMillis>,
    during_interval: DuringInterval,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Deferred {
    None,
    Latest(ProductPayload),
    Queue(VecDeque<ProductPayload>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum NotifyState {
    Ready,
    Cooling {
        policy: DuringInterval,
        deferred: Deferred,
    },
}

enum IntervalState {
    Ready,
    Cooling {
        arming: ArmingCorrelation,
        pending: PendingNotifications,
    },
}

enum PendingNotifications {
    Suppress,
    Latest(Option<ProductPayload>),
    Queue(VecDeque<ProductPayload>),
}

impl PendingNotifications {
    fn empty(policy: DuringInterval) -> Self {
        match policy {
            DuringInterval::Suppress => Self::Suppress,
            DuringInterval::Latest => Self::Latest(None),
            DuringInterval::Queue => Self::Queue(VecDeque::new()),
        }
    }

    fn policy(&self) -> DuringInterval {
        match self {
            Self::Suppress => DuringInterval::Suppress,
            Self::Latest(_) => DuringInterval::Latest,
            Self::Queue(_) => DuringInterval::Queue,
        }
    }

    fn observe(&self) -> Deferred {
        match self {
            Self::Suppress | Self::Latest(None) => Deferred::None,
            Self::Latest(Some(payload)) => Deferred::Latest(payload.clone()),
            Self::Queue(queue) => Deferred::Queue(queue.clone()),
        }
    }

    fn front(&self) -> Option<&ProductPayload> {
        match self {
            Self::Suppress => None,
            Self::Latest(payload) => payload.as_ref(),
            Self::Queue(queue) => queue.front(),
        }
    }

    fn pop_front(&mut self) {
        match self {
            Self::Suppress => {}
            Self::Latest(payload) => *payload = None,
            Self::Queue(queue) => {
                queue.pop_front();
            }
        }
    }

    fn checkpoint(&self) -> ProductValue {
        let deferred = match self {
            Self::Suppress | Self::Latest(None) => ProductValue::Null,
            Self::Latest(Some(payload)) => encode_payload(payload),
            Self::Queue(queue) => ProductValue::array(queue.iter().map(encode_payload)),
        };
        ProductValue::array([ProductValue::string(self.policy().as_str()), deferred])
    }
}

pub struct NotifyActor<V, I> {
    config: NotifyConfig,
    state: IntervalState,
    last_correlation: u64,
    marker: PhantomData<fn() -> (V, I)>,
}

impl<V, I> NotifyActor<V, I> {
    /// A derived observation. Deferred payloads are cloned only for this caller;
    /// normal transitions keep ownership of the pending queue.
    #[must_use]
    pub fn state(&self) -> NotifyState {
        match &self.state {
            IntervalState::Ready => NotifyState::Ready,
            IntervalState::Cooling { pending, .. } => NotifyState::Cooling {
                policy: pending.policy(),
                deferred: pending.observe(),
            },
        }
    }

    fn arming(&self) -> Option<ArmingCorrelation> {
        match &self.state {
            IntervalState::Ready => None,
            IntervalState::Cooling { arming, .. } => Some(*arming),
        }
    }

    fn timer<G: UserNotifyAuthorityBearer + ?Sized>(
        &mut self,
        value: &ProductValue,
        grants: &G,
    ) -> ActorEffects<ProductPayload> {
        let ProductValue::UInt(correlation) = value else {
            return suppress_stale();
        };
        let IntervalState::Cooling { arming, pending } = &mut self.state else {
            return suppress_stale();
        };
        if arming.get() != *correlation {
            return suppress_stale();
        }
        let Some(payload) = pending.front() else {
            self.state = IntervalState::Ready;
            return ActorEffects::empty();
        };
        let effects = match self.config.notification(payload, grants) {
            Ok(effects) => effects,
            Err(error) => {
                return ActorEffects::emit(port(ERROR_PORT_NAME), error_payload(error));
            }
        };
        pending.pop_front();
        let (correlation, schedule) = arm(
            &mut self.last_correlation,
            self.config
                .minimum_interval
                .expect("cooling only takes a positive interval"),
        );
        *arming = correlation;
        effects.concat(schedule)
    }
}

fn arm(
    last_correlation: &mut u64,
    after: NonZeroMillis,
) -> (ArmingCorrelation, ActorEffects<ProductPayload>) {
    let next = last_correlation
        .checked_add(1)
        .expect("notify arming correlation exhausted its u64 space");
    let correlation = ArmingCorrelation::new(next);
    *last_correlation = next;
    (
        correlation,
        ActorEffects::external(Effect::schedule(ScheduleSpec::new(
            after,
            correlation.correlation(),
        ))),
    )
}

impl NotifyConfig {
    fn projection(&self, payload: &ProductPayload) -> Result<NotificationSpec, &'static str> {
        let object = payload.value().as_object().ok_or("invalid notification")?;
        let title = object
            .get("title")
            .and_then(ProductValue::as_str)
            .ok_or("notification title missing")?;
        let body = object
            .get("body")
            .and_then(ProductValue::as_str)
            .ok_or("notification body missing")?;
        Ok(NotificationSpec::new(self.channel.clone(), title, body))
    }

    fn notification<G: UserNotifyAuthorityBearer + ?Sized>(
        &self,
        payload: &ProductPayload,
        grants: &G,
    ) -> Result<ActorEffects<ProductPayload>, &'static str> {
        self.delivery(self.projection(payload)?, grants)
    }

    fn delivery<G: UserNotifyAuthorityBearer + ?Sized>(
        &self,
        spec: NotificationSpec,
        grants: &G,
    ) -> Result<ActorEffects<ProductPayload>, &'static str> {
        let grant = grants.user_notify_authority().ok_or("UserNotify denied")?;
        Ok(ActorEffects::external(Effect::notify(grant, None, spec)))
    }
}

use crate::payload_value::{decode_payload, encode_payload};

impl<V, I> EditableActor for NotifyActor<V, I>
where
    V: Clone + From<u16> + PartialEq,
    I: Clone + Ord,
{
    type StateVersion = V;
    type EffectId = I;

    fn on_config_change(&mut self, _config: &FoldedConfig) -> ConfigChangeOutcome {
        ConfigChangeOutcome::ReplaceIncarnation
    }

    fn checkpoint(&self) -> Option<ActorState<V>> {
        self.try_checkpoint()
            .expect("canonical encoding of the notify state")
    }

    fn try_checkpoint(&self) -> Result<Option<ActorState<V>>, circular_core::CodecError> {
        let state = match &self.state {
            IntervalState::Ready => ProductValue::Null,
            IntervalState::Cooling { pending, .. } => pending.checkpoint(),
        };
        let value = ProductValue::array([
            state,
            self.arming().map_or(ProductValue::Null, |arming| {
                ProductValue::UInt(arming.get())
            }),
            ProductValue::UInt(self.last_correlation),
        ]);
        let bytes = circular_core::encode(&value, Ceilings::for_boundary(Boundary::ActorState))?;
        Ok(Some(ActorState::new(V::from(NOTIFY_STATE_SCHEMA), bytes)))
    }

    fn restore(&mut self, state: ActorState<V>) -> Result<(), ActorRestoreError<V>> {
        let (schema, bytes) = state.into_parts();
        if schema != V::from(NOTIFY_STATE_SCHEMA) {
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
        let ProductValue::Array(fields) = value else {
            return Err(decode());
        };
        let Ok([state, current, last]) = <[ProductValue; 3]>::try_from(fields) else {
            return Err(decode());
        };
        let ProductValue::UInt(last) = last else {
            return Err(decode());
        };
        let arming = match current {
            ProductValue::Null => None,
            ProductValue::UInt(current) if current != 0 && current == last => {
                Some(ArmingCorrelation::new(current))
            }
            _ => return Err(invariant()),
        };
        let state = match (state, arming) {
            (ProductValue::Null, None) => IntervalState::Ready,
            (ProductValue::Array(fields), Some(arming))
                if self.config.minimum_interval.is_some() =>
            {
                let Ok([policy, deferred]) = <[ProductValue; 2]>::try_from(fields) else {
                    return Err(decode());
                };
                let ProductValue::String(policy) = policy else {
                    return Err(decode());
                };
                let policy = DuringInterval::from_str(&policy).ok_or_else(invariant)?;
                if policy != self.config.during_interval {
                    return Err(invariant());
                }
                let pending = match (policy, deferred) {
                    (DuringInterval::Suppress, ProductValue::Null) => {
                        PendingNotifications::Suppress
                    }
                    (DuringInterval::Latest, ProductValue::Null) => {
                        PendingNotifications::Latest(None)
                    }
                    (DuringInterval::Latest, value) => PendingNotifications::Latest(Some(
                        decode_payload(value).ok_or_else(decode)?,
                    )),
                    (DuringInterval::Queue, ProductValue::Array(values)) => {
                        PendingNotifications::Queue(
                            values
                                .into_iter()
                                .map(decode_payload)
                                .collect::<Option<_>>()
                                .ok_or_else(decode)?,
                        )
                    }
                    _ => return Err(invariant()),
                };
                IntervalState::Cooling { arming, pending }
            }
            _ => return Err(invariant()),
        };
        self.state = state;
        self.last_correlation = last;
        Ok(())
    }
}

impl<T> EmittingActor<T, ProductPayload> for NotifyActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
    T::Grants: UserNotifyAuthorityBearer,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        let payload = input.payload::<T>();
        if input.inlet().as_str() == TIMER_PORT_NAME {
            return self.timer(payload.value(), context.grants());
        }
        if let IntervalState::Cooling { pending, .. } = &mut self.state {
            match pending {
                PendingNotifications::Suppress => return suppress("cooling"),
                PendingNotifications::Latest(latest) => *latest = Some(payload.clone()),
                PendingNotifications::Queue(queue) => {
                    queue.push_back(payload.clone());
                }
            }
            return ActorEffects::empty();
        }
        let spec = match self.config.projection(payload) {
            Ok(spec) => spec,
            Err(error) => {
                return ActorEffects::reject(
                    error_payload(error),
                    ProcessingCause::InputOutOfDomain,
                );
            }
        };
        let effects = match self.config.delivery(spec, context.grants()) {
            Ok(effects) => effects,
            Err(error) => return ActorEffects::emit(port(ERROR_PORT_NAME), error_payload(error)),
        };
        if let Some(after) = self.config.minimum_interval {
            let (arming, schedule) = arm(&mut self.last_correlation, after);
            self.state = IntervalState::Cooling {
                arming,
                pending: PendingNotifications::empty(self.config.during_interval),
            };
            effects.concat(schedule)
        } else {
            effects
        }
    }

    fn on_outcome(
        &mut self,
        outcome: &EffectOutcome<T::EffectId>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<ProductPayload> {
        match outcome.result() {
            Ok(OutcomePayload::NotificationDelivered(receipt))
                if receipt.channel() == &self.config.channel =>
            {
                ActorEffects::empty()
            }
            Err(failure) => ActorEffects::emit(
                port(ERROR_PORT_NAME),
                error_payload(format!("notify failed: {}", failure.kind_tag())),
            ),
            _ => ActorEffects::emit(
                port(ERROR_PORT_NAME),
                error_payload("mismatched notify outcome"),
            ),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NotifyFactoryError {
    InvalidConfig,
    Config(ConfigRejection),
}

impl fmt::Display for NotifyFactoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig => formatter.write_str(
                "notify config is not a notify config, or a slot value is outside its domain",
            ),
            Self::Config(rejection) => rejection.fmt(formatter),
        }
    }
}
impl Error for NotifyFactoryError {}

impl From<ConfigRejection> for NotifyFactoryError {
    fn from(rejection: ConfigRejection) -> Self {
        Self::Config(rejection)
    }
}

pub(crate) const MINIMUM_INTERVAL: Slot<Interval> = Slot::new("minimum_interval", Interval);

pub(crate) const DURING_INTERVAL: Slot<Spelled<DuringInterval>> = Slot::new(
    "during_interval",
    Spelled::new(&DuringInterval::ALL, DuringInterval::as_str),
);

pub struct NotifyFactory<T>(PhantomData<fn() -> T>);

impl<T> EmittingActorFactory<ProductPayload> for NotifyFactory<T>
where
    T: ActorTypes<Payload = ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
    T::Grants: UserNotifyAuthorityBearer,
{
    const TYPE: ActorType = ActorType::Notify;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Restarts;
    const CHECKPOINTS: bool = true;
    type Grants = T::Grants;
    type Types = T;
    type Instance = NotifyActor<T::StateVersion, T::EffectId>;
    type Error = NotifyFactoryError;

    fn create(
        config: &FoldedConfig,
        _grants: &Self::Grants,
    ) -> Result<Self::Instance, Self::Error> {
        Ok(NotifyActor {
            config: declared_config(config)?,
            state: IntervalState::Ready,
            last_correlation: 0,
            marker: PhantomData,
        })
    }
}

fn declared_config(config: &FoldedConfig) -> Result<NotifyConfig, NotifyFactoryError> {
    crate::retry_config::declared(config.value()).map_err(|_| NotifyFactoryError::InvalidConfig)?;
    let value = config
        .for_type(ActorType::Notify)
        .map_err(|_| NotifyFactoryError::InvalidConfig)?;
    let schema = crate::registration(ActorType::Notify).spec().config();
    let mut fields = schema.open(value)?;
    let channel = schema
        .raw(&mut fields, "channel")?
        .as_str()
        .filter(|channel| !channel.is_empty())
        .ok_or(NotifyFactoryError::InvalidConfig)?;
    let interval = schema.read(&mut fields, &MINIMUM_INTERVAL)?.get();
    let during_interval = schema.read(&mut fields, &DURING_INTERVAL)?;
    Ok(NotifyConfig {
        channel: NotificationChannel::from_normalized(channel.to_owned()),
        minimum_interval: NonZeroMillis::new(interval).ok(),
        during_interval,
    })
}

pub(crate) fn judge(
    config: &FoldedConfig,
    _inlets: &crate::inlet_shapes::ResolvedInletShapes,
) -> Result<(), NotifyFactoryError> {
    declared_config(config).map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Shape;
    use circular_plan::{
        Config, Generation, GenerationVector, Incarnation, Name as PlanName, NamedActorId, ScopeId,
    };
    use circular_runtime::{
        EffectFailure, Granted, InterpreterFault, NotificationChannels, NotificationReceipt,
        UserNotify, UserNotifyGrant,
    };

    use circular_testkit::types::TestRun;
    struct TestGrants(Option<Granted<UserNotify>>);
    impl UserNotifyAuthorityBearer for TestGrants {
        fn user_notify_authority(&self) -> Option<Granted<UserNotify>> {
            self.0
        }
    }
    type TestTypes = circular_testkit::types::TestTypes<ProductPayload, TestGrants>;
    type Actor = NotifyActor<u16, u64>;

    fn config(interval: ProductValue, during: &str, channel: &str) -> ProductValue {
        ProductValue::object([
            (
                "capabilities",
                ProductValue::object([(
                    "UserNotify",
                    ProductValue::object([("approval", ProductValue::string("none"))]).unwrap(),
                )])
                .unwrap(),
            ),
            ("channel", ProductValue::string(channel)),
            ("minimum_interval", interval),
            ("during_interval", ProductValue::string(during)),
        ])
        .unwrap()
    }

    fn folded(interval: i64, during: &str) -> FoldedConfig {
        FoldedConfig::minted(
            ActorType::Notify,
            config(ProductValue::Int(interval), during, "slack"),
        )
    }

    fn actor(interval: i64, during: &str) -> Actor {
        NotifyFactory::<TestTypes>::create(&folded(interval, during), &TestGrants(None)).unwrap()
    }

    fn drive<R>(stamp: u64, f: impl FnOnce(&ActorContext<'_, TestRun, TestGrants>) -> R) -> R {
        let channel = NotificationChannel::from_normalized("slack");
        let raw = UserNotifyGrant::user_notify(NotificationChannels::exact([channel]));
        let grants = TestGrants(Some(circular_runtime::GrantIssuer::new().issue(&raw)));
        let named = NamedActorId::new(ScopeId::root(), PlanName::from_normalized("notify"));
        let generations = GenerationVector::for_actor(
            named.as_scoped(),
            vec![Generation::new(0)].into_boxed_slice(),
        )
        .unwrap();
        let incarnation =
            Incarnation::new(TestRun::default(), named.as_scoped().clone(), generations);
        let actor = named.as_actor_id();
        let config = Config::default();
        f(&ActorContext::new(&actor, &incarnation, &config, &grants))
    }

    fn payload(value: ProductValue) -> ProductPayload {
        ProductPayload::new(GroundShape::try_new(Shape::Any).unwrap(), value)
    }

    fn notification(title: &str) -> ProductPayload {
        payload(
            ProductValue::object([
                ("title", ProductValue::string(title)),
                ("body", ProductValue::string("disk full")),
                (
                    "extra",
                    ProductValue::array([
                        ProductValue::UInt(u64::MAX),
                        ProductValue::bytes([0, 255]),
                        ProductValue::float(-0.0),
                    ]),
                ),
            ])
            .unwrap(),
        )
    }

    fn input(
        actor: &mut Actor,
        inlet: &str,
        value: ProductPayload,
        stamp: u64,
    ) -> ActorEffects<ProductPayload> {
        drive(stamp, |cx| {
            <Actor as EmittingActor<TestTypes, ProductPayload>>::on_event(
                actor,
                &ActorInput::new(port(inlet), value),
                cx,
            )
        })
    }
    fn event(actor: &mut Actor, title: &str, stamp: u64) -> ActorEffects<ProductPayload> {
        input(actor, "notification", notification(title), stamp)
    }
    fn timer(actor: &mut Actor, correlation: u64, stamp: u64) -> ActorEffects<ProductPayload> {
        input(
            actor,
            TIMER_PORT_NAME,
            payload(ProductValue::UInt(correlation)),
            stamp,
        )
    }
    fn scheduled(effects: &ActorEffects<ProductPayload>) -> u64 {
        let schedules: Vec<_> = effects
            .iter()
            .filter_map(|effect| match effect {
                ActorEffect::External(Effect::Schedule { spec }) => Some(spec),
                _ => None,
            })
            .collect();
        assert_eq!(schedules.len(), 1);
        assert_eq!(schedules[0].after().get().get(), 100);
        schedules[0].correlation().get()
    }
    fn titles(effects: &ActorEffects<ProductPayload>) -> Vec<&str> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                ActorEffect::External(Effect::Notify { spec, .. }) => {
                    assert_eq!(
                        spec.channel(),
                        &NotificationChannel::from_normalized("slack")
                    );
                    assert_eq!(spec.body(), "disk full");
                    Some(spec.title())
                }
                _ => None,
            })
            .collect()
    }
    fn assert_suppressed(effects: &ActorEffects<ProductPayload>, expected: &str) {
        assert!(
            matches!(effects.as_slice(), [ActorEffect::Suppress { reason }] if reason.name() == expected)
        );
    }
    fn assert_error(effects: &ActorEffects<ProductPayload>) {
        assert!(
            matches!(effects.as_slice(), [ActorEffect::Emit { port, .. }] if port.as_str() == ERROR_PORT_NAME)
        );
    }

    #[test]
    fn zero_interval_submits_three_notifications_without_schedules_for_every_policy() {
        for policy in ["suppress", "latest", "queue"] {
            let mut actor = actor(0, policy);
            for title in ["first", "second", "third"] {
                let effects = event(&mut actor, title, 7);
                assert_eq!(titles(&effects), [title]);
                assert_eq!(effects.as_slice().len(), 1);
                assert_eq!(actor.state(), NotifyState::Ready);
                assert_eq!(actor.arming(), None);
            }
        }
    }

    #[test]
    fn suppress_waits_for_the_matching_arrival_regardless_of_stamp() {
        let mut actor = actor(100, "suppress");
        let first = event(&mut actor, "first", 10);
        assert_eq!(titles(&first), ["first"]);
        let correlation = scheduled(&first);
        let before = actor.checkpoint();
        assert_suppressed(&event(&mut actor, "second", u64::MAX), "cooling");
        assert_eq!(actor.checkpoint(), before);
        assert!(timer(&mut actor, correlation, 0).is_empty());
        assert_eq!(actor.state(), NotifyState::Ready);
        assert_ne!(actor.checkpoint(), before);
        let third = event(&mut actor, "third", 0);
        assert_eq!(titles(&third), ["third"]);
        assert!(scheduled(&third) > correlation);
    }

    #[test]
    fn latest_replaces_the_deferred_payload_and_rearms_from_submission() {
        let mut actor = actor(100, "latest");
        let first = scheduled(&event(&mut actor, "first", 5));
        assert!(event(&mut actor, "second", 6).is_empty());
        let second = actor.checkpoint();
        assert!(event(&mut actor, "third", 1_000).is_empty());
        assert_ne!(actor.checkpoint(), second);
        assert!(
            matches!(actor.state(), NotifyState::Cooling { policy: DuringInterval::Latest, deferred: Deferred::Latest(value) } if value == notification("third"))
        );
        let delivered = timer(&mut actor, first, 1);
        assert_eq!(titles(&delivered), ["third"]);
        let next = scheduled(&delivered);
        assert!(next > first);
        assert!(timer(&mut actor, next, 1).is_empty());
        assert_eq!(actor.state(), NotifyState::Ready);
    }

    #[test]
    fn queue_submits_one_fifo_payload_per_matching_arrival() {
        let mut actor = actor(100, "queue");
        let mut correlation = scheduled(&event(&mut actor, "first", 99));
        assert!(event(&mut actor, "second", 99).is_empty());
        assert!(event(&mut actor, "third", 100_000).is_empty());
        for title in ["second", "third"] {
            let effects = timer(&mut actor, correlation, 0);
            assert_eq!(titles(&effects), [title]);
            let next = scheduled(&effects);
            assert!(next > correlation);
            correlation = next;
        }
        assert!(
            matches!(actor.state(), NotifyState::Cooling { deferred: Deferred::Queue(queue), .. } if queue.is_empty())
        );
        assert!(timer(&mut actor, correlation, 0).is_empty());
        assert_eq!(actor.state(), NotifyState::Ready);
    }

    #[test]
    fn stale_malformed_and_duplicate_timers_preserve_state_and_raw_ticks_do_not_release() {
        for policy in ["suppress", "latest", "queue"] {
            let mut actor = actor(100, policy);
            assert_suppressed(&timer(&mut actor, 1, 0), crate::STALE_TIMER_FIRE);
            let correlation = scheduled(&event(&mut actor, "first", 10));
            let before = actor.checkpoint();
            for wrong in [0, correlation + 1] {
                assert_suppressed(&timer(&mut actor, wrong, u64::MAX), crate::STALE_TIMER_FIRE);
                assert_eq!(actor.checkpoint(), before);
            }
            assert_suppressed(
                &input(
                    &mut actor,
                    TIMER_PORT_NAME,
                    payload(ProductValue::Int(correlation as i64)),
                    100,
                ),
                crate::STALE_TIMER_FIRE,
            );
            assert_eq!(actor.checkpoint(), before);
            timer(&mut actor, correlation, 0);
            let ready = actor.checkpoint();
            assert_suppressed(&timer(&mut actor, correlation, 0), crate::STALE_TIMER_FIRE);
            assert_eq!(actor.checkpoint(), ready);
            let next = scheduled(&event(&mut actor, "second", 0));
            assert!(next > correlation);
            let armed = actor.checkpoint();
            assert_suppressed(&timer(&mut actor, correlation, 0), crate::STALE_TIMER_FIRE);
            assert_eq!(actor.checkpoint(), armed);
        }
    }

    #[test]
    fn outcomes_never_clear_or_extend_cooling_and_success_is_silent() {
        for interval in [0, 100] {
            let mut actor = actor(interval, "queue");
            event(&mut actor, "first", 2);
            event(&mut actor, "second", 3);
            let before = actor.checkpoint();
            for result in [
                Ok(OutcomePayload::NotificationDelivered(
                    NotificationReceipt::delivered(NotificationChannel::from_normalized("slack")),
                )),
                Err(EffectFailure::EndpointGone),
                Ok(OutcomePayload::NotificationDelivered(
                    NotificationReceipt::delivered(NotificationChannel::from_normalized("other")),
                )),
            ] {
                let success = matches!(&result, Ok(OutcomePayload::NotificationDelivered(receipt)) if receipt.channel() == &NotificationChannel::from_normalized("slack"));
                let effects = drive(u64::MAX, |cx| {
                    <Actor as EmittingActor<TestTypes, ProductPayload>>::on_outcome(
                        &mut actor,
                        &EffectOutcome::new(1, result),
                        cx,
                    )
                });
                if success {
                    assert!(effects.is_empty());
                } else {
                    assert_error(&effects);
                }
                assert_eq!(actor.checkpoint(), before);
            }
        }
    }

    #[test]
    fn notify_failure_error_names_the_closed_kind_not_the_debug_form() {
        for (failure, expected) in [
            (
                EffectFailure::InterpreterFault(InterpreterFault::Interrupted),
                "notify failed: interpreter_fault",
            ),
            (EffectFailure::EndpointGone, "notify failed: endpoint_gone"),
            (
                EffectFailure::RetryExhausted { attempts: 2 },
                "notify failed: retry_exhausted",
            ),
        ] {
            let mut actor = actor(0, "queue");
            let effects = drive(u64::MAX, |cx| {
                <Actor as EmittingActor<TestTypes, ProductPayload>>::on_outcome(
                    &mut actor,
                    &EffectOutcome::new(1, Err(failure)),
                    cx,
                )
            });
            let [ActorEffect::Emit { port, payload, .. }] = effects.as_slice() else {
                panic!("one _error emission");
            };
            assert_eq!(port.as_str(), ERROR_PORT_NAME);
            assert_eq!(payload.value(), &ProductValue::String(expected.to_owned()));
        }
    }

    #[test]
    fn factory_keeps_the_nonnegative_integer_and_nonempty_channel_boundary() {
        for interval in [
            ProductValue::Int(-1),
            ProductValue::float(1.0),
            ProductValue::string("100"),
            ProductValue::Null,
        ] {
            let folded =
                FoldedConfig::minted(ActorType::Notify, config(interval, "queue", "slack"));
            assert!(NotifyFactory::<TestTypes>::create(&folded, &TestGrants(None)).is_err());
        }
        for interval in [
            ProductValue::Int(0),
            ProductValue::Int(100),
            ProductValue::UInt(0),
            ProductValue::UInt(u64::MAX),
        ] {
            let valid = FoldedConfig::minted(ActorType::Notify, config(interval, "queue", "slack"));
            assert!(NotifyFactory::<TestTypes>::create(&valid, &TestGrants(None)).is_ok());
        }
        let empty = FoldedConfig::minted(
            ActorType::Notify,
            config(ProductValue::Int(0), "suppress", ""),
        );
        assert!(NotifyFactory::<TestTypes>::create(&empty, &TestGrants(None)).is_err());
    }

    #[test]
    fn checkpoints_round_trip_ready_and_every_cooling_policy_with_full_payloads() {
        for policy in ["suppress", "latest", "queue"] {
            let mut original = actor(100, policy);
            for step in 0..4 {
                if step > 0 {
                    event(
                        &mut original,
                        ["", "first", "second", "third"][step],
                        step as u64,
                    );
                }
                let checkpoint = original.checkpoint().unwrap();
                let mut restored = actor(100, policy);
                restored.restore(checkpoint.clone()).unwrap();
                assert_eq!(restored.checkpoint(), Some(checkpoint));
                assert_eq!(restored.state(), original.state());
                assert_eq!(restored.arming(), original.arming());
                assert_eq!(restored.last_correlation, original.last_correlation);
                if let Some(correlation) = restored.arming() {
                    let effects = timer(&mut restored, correlation.get(), 0);
                    if step >= 2 && policy != "suppress" {
                        assert_eq!(
                            titles(&effects),
                            [if policy == "latest" && step == 3 {
                                "third"
                            } else {
                                "second"
                            }]
                        );
                        assert!(scheduled(&effects) > correlation.get());
                    } else {
                        assert!(effects.is_empty());
                    }
                }
            }
        }
    }

    #[test]
    fn checkpoint_bytes_preserve_schema_one_ready_and_empty_cooling() {
        const READY: &[u8] = b"\x07\0\0\0\x03\x01\x01\x09\0\0\0\0\0\0\0\0";
        for (policy, cooling) in [
            ("suppress", &b"\x07\0\0\0\x03\x07\0\0\0\x02\x05\0\0\0\x08suppress\x01\x09\0\0\0\0\0\0\0\x01\x09\0\0\0\0\0\0\0\x01"[..]),
            ("latest", &b"\x07\0\0\0\x03\x07\0\0\0\x02\x05\0\0\0\x06latest\x01\x09\0\0\0\0\0\0\0\x01\x09\0\0\0\0\0\0\0\x01"[..]),
            ("queue", &b"\x07\0\0\0\x03\x07\0\0\0\x02\x05\0\0\0\x05queue\x07\0\0\0\0\x09\0\0\0\0\0\0\0\x01\x09\0\0\0\0\0\0\0\x01"[..]),
        ] {
            let mut original = actor(100, policy);
            assert_eq!(original.checkpoint().unwrap().bytes(), READY);
            event(&mut original, "first", 0);
            assert_eq!(original.checkpoint().unwrap().bytes(), cooling);

            let mut restored = actor(100, policy);
            restored.restore(ActorState::new(1, cooling.to_vec())).unwrap();
            assert_eq!(restored.checkpoint().unwrap().bytes(), cooling);
            assert!(timer(&mut restored, 1, 0).is_empty());
            assert_eq!(restored.state(), NotifyState::Ready);
            assert_eq!(scheduled(&event(&mut restored, "second", 0)), 2);
        }
    }

    #[test]
    fn literal_nonempty_checkpoints_preserve_payloads_and_deferred_order() {
        for (policy, hex) in [
            (
                "latest",
                "0700000003070000000205000000066c617465737407000000020700000001030000000000000001080000000300000004626f647905000000096469736b2066756c6c000000056578747261070000000309ffffffffffffffff060000000200ff048000000000000000000000057469746c6505000000057468697264090000000000000001090000000000000001",
            ),
            (
                "queue",
                "0700000003070000000205000000057175657565070000000207000000020700000001030000000000000001080000000300000004626f647905000000096469736b2066756c6c000000056578747261070000000309ffffffffffffffff060000000200ff048000000000000000000000057469746c6505000000067365636f6e6407000000020700000001030000000000000001080000000300000004626f647905000000096469736b2066756c6c000000056578747261070000000309ffffffffffffffff060000000200ff048000000000000000000000057469746c6505000000057468697264090000000000000001090000000000000001",
            ),
        ] {
            let bytes = hex
                .as_bytes()
                .chunks_exact(2)
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect::<Vec<_>>();
            let mut original = actor(100, policy);
            event(&mut original, "first", 0);
            event(&mut original, "second", 0);
            event(&mut original, "third", 0);
            assert_eq!(original.checkpoint().unwrap().bytes(), bytes);

            let mut restored = actor(100, policy);
            restored.restore(ActorState::new(1, bytes.clone())).unwrap();
            assert_eq!(restored.checkpoint().unwrap().bytes(), bytes);
            let expected = if policy == "latest" {
                Deferred::Latest(notification("third"))
            } else {
                Deferred::Queue(VecDeque::from([
                    notification("second"),
                    notification("third"),
                ]))
            };
            assert_eq!(
                restored.state(),
                NotifyState::Cooling {
                    policy: DuringInterval::from_str(policy).unwrap(),
                    deferred: expected,
                }
            );
            let first = timer(&mut restored, 1, 0);
            assert_eq!(
                titles(&first),
                [if policy == "latest" {
                    "third"
                } else {
                    "second"
                }]
            );
            assert_eq!(scheduled(&first), 2);
            let second = timer(&mut restored, 2, 0);
            if policy == "queue" {
                assert_eq!(titles(&second), ["third"]);
                assert_eq!(scheduled(&second), 3);
                assert!(timer(&mut restored, 3, 0).is_empty());
            } else {
                assert!(second.is_empty());
            }
            assert_eq!(restored.state(), NotifyState::Ready);
        }
    }

    #[test]
    fn restore_rejects_invalid_bytes_schema_and_state_without_partial_mutation() {
        let mut actor = actor(100, "queue");
        event(&mut actor, "first", 0);
        event(&mut actor, "second", 0);
        let before = actor.checkpoint();
        assert!(matches!(
            actor.restore(ActorState::new(2, vec![])),
            Err(ActorRestoreError::SchemaBeyondLadder { .. })
        ));
        assert!(matches!(
            actor.restore(ActorState::new(1, vec![255])),
            Err(ActorRestoreError::DecodeFailed { .. })
        ));
        for value in [
            ProductValue::array([
                ProductValue::Null,
                ProductValue::UInt(1),
                ProductValue::UInt(1),
            ]),
            ProductValue::array([
                ProductValue::array([ProductValue::string("queue"), ProductValue::array([])]),
                ProductValue::UInt(0),
                ProductValue::UInt(1),
            ]),
            ProductValue::array([
                ProductValue::array([ProductValue::string("suppress"), ProductValue::Null]),
                ProductValue::UInt(1),
                ProductValue::UInt(1),
            ]),
            ProductValue::array([
                ProductValue::array([ProductValue::string("queue"), ProductValue::array([])]),
                ProductValue::UInt(2),
                ProductValue::UInt(1),
            ]),
        ] {
            let bytes = circular_core::encode(&value, Ceilings::for_boundary(Boundary::ActorState))
                .unwrap();
            assert!(matches!(
                actor.restore(ActorState::new(1, bytes)),
                Err(ActorRestoreError::StateInvariantViolated { .. })
            ));
            assert_eq!(actor.checkpoint(), before);
        }
    }

    #[test]
    fn fresh_factory_is_ready_before_c2_and_config_changes_require_restart() {
        for policy in ["suppress", "latest", "queue"] {
            let mut original = actor(100, policy);
            event(&mut original, "first", 0);
            event(&mut original, "second", 0);
            let before = original.checkpoint();
            assert_eq!(
                original.on_config_change(&folded(0, "suppress")),
                ConfigChangeOutcome::ReplaceIncarnation
            );
            assert_eq!(original.checkpoint(), before);
            let mut fresh = actor(100, policy);
            assert_eq!(fresh.state(), NotifyState::Ready);
            assert_eq!(fresh.arming(), None);
            assert_eq!(fresh.last_correlation, 0);
            assert_eq!(
                fresh.on_config_change(&folded(200, "latest")),
                ConfigChangeOutcome::ReplaceIncarnation
            );
        }
    }

    #[test]
    fn invalid_projection_and_missing_authority_do_not_start_cooling() {
        let mut actor = actor(100, "queue");
        let before = actor.checkpoint();
        for value in [
            ProductValue::Null,
            ProductValue::object([] as [(&str, ProductValue); 0]).unwrap(),
            ProductValue::object([("title", ProductValue::string("first"))]).unwrap(),
        ] {
            assert!(matches!(
                input(&mut actor, "notification", payload(value), 0).as_slice(),
                [ActorEffect::Reject {
                    cause: ProcessingCause::InputOutOfDomain,
                    ..
                }]
            ));
            assert_eq!(actor.checkpoint(), before);
        }
        assert!(
            actor
                .config
                .notification(&notification("first"), &TestGrants(None))
                .is_err()
        );
        assert_eq!(actor.checkpoint(), before);
    }
}
