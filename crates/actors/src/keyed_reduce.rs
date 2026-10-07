
use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use circular_core::{FloatValue, Value};
use circular_runtime::{
    ActorContext, ActorEffects, ActorInput, ActorRestoreError, ActorState, ActorTypes,
    ConfigChangeOutcome, EditableActor, EmittingActor, EmittingActorFactory, FoldedConfig,
};

use crate::config::{PayloadPath, PayloadPathError, payload_path_from_value};
use crate::route_config::select;

pub const KEYED_REDUCE_STATE_SCHEMA: u16 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct KeyedReduceConfig {
    at: PayloadPath,
    value: PayloadPath,
}

impl KeyedReduceConfig {
    pub const AT: &'static str = "at";
    pub const VALUE: &'static str = "value";

    pub fn from_value(config: &Value) -> Result<Self, KeyedReduceConfigError> {
        let Some(root) = config.as_object() else {
            return Err(KeyedReduceConfigError::NotAnObject);
        };
        let at = root
            .get(Self::AT)
            .ok_or(KeyedReduceConfigError::MissingAt)
            .and_then(|at| payload_path_from_value(at).map_err(KeyedReduceConfigError::At))?;
        let value = root
            .get(Self::VALUE)
            .ok_or(KeyedReduceConfigError::MissingValue)
            .and_then(|value| {
                payload_path_from_value(value).map_err(KeyedReduceConfigError::Value)
            })?;
        Ok(Self { at, value })
    }

    #[must_use]
    pub const fn at(&self) -> &PayloadPath {
        &self.at
    }

    #[must_use]
    pub const fn value(&self) -> &PayloadPath {
        &self.value
    }

    #[must_use]
    pub fn key_of<'payload>(&self, payload: &'payload Value) -> Option<&'payload str> {
        select(payload, &self.at).and_then(Value::as_str)
    }

    #[must_use]
    pub fn delta_of(&self, payload: &Value) -> Option<FloatValue> {
        let selected = select(payload, &self.value)?;
        match selected {
            Value::Int(int) => Some(FloatValue::new(*int as f64)),
            Value::UInt(uint) => Some(FloatValue::new(*uint as f64)),
            _ => selected.as_float().map(FloatValue::new),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyedReduceConfigError {
    NotAnObject,
    MissingAt,
    At(PayloadPathError),
    MissingValue,
    Value(PayloadPathError),
}

impl fmt::Display for KeyedReduceConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnObject => formatter.write_str("keyed_reduce config is not an object"),
            Self::MissingAt => formatter.write_str("keyed_reduce config is missing at"),
            Self::At(error) => write!(formatter, "at is not an exact path: {error}"),
            Self::MissingValue => formatter.write_str("keyed_reduce config is missing value"),
            Self::Value(error) => write!(formatter, "value is not an exact path: {error}"),
        }
    }
}

impl std::error::Error for KeyedReduceConfigError {}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct KeyedReduceState {
    live: BTreeMap<Box<str>, FloatValue>,
}

impl KeyedReduceState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn count(&self) -> i64 {
        i64::try_from(self.live.len()).unwrap_or(i64::MAX)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<FloatValue> {
        self.live.get(key).copied()
    }

    pub fn entries(&self) -> impl Iterator<Item = (&str, FloatValue)> {
        self.live.iter().map(|(key, value)| (key.as_ref(), *value))
    }

    #[must_use]
    pub fn total(&self) -> FloatValue {
        FloatValue::new(
            self.live
                .values()
                .fold(0.0_f64, |sum, value| sum + value.get()),
        )
    }

    pub fn accumulate(&mut self, key: &str, delta: FloatValue) -> FloatValue {
        match self.live.get_mut(key) {
            Some(slot) => {
                *slot = FloatValue::new(slot.get() + delta.get());
                *slot
            }
            None => {
                self.live.insert(key.into(), delta);
                delta
            }
        }
    }

    pub fn evict(&mut self, key: &str) -> Eviction {
        match self.live.remove(key) {
            Some(released) => Eviction::Released { released },
            None => Eviction::NotLive,
        }
    }

    #[must_use]
    pub fn encode(&self) -> Box<[u8]> {
        let mut bytes = Vec::new();
        let count = u32::try_from(self.live.len()).unwrap_or(u32::MAX);
        bytes.extend_from_slice(&count.to_be_bytes());
        for (key, value) in &self.live {
            let len = u32::try_from(key.len()).unwrap_or(u32::MAX);
            bytes.extend_from_slice(&len.to_be_bytes());
            bytes.extend_from_slice(key.as_bytes());
            bytes.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        bytes.into_boxed_slice()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, KeyedReduceStateError> {
        let mut at = 0_usize;
        let mut take = |len: usize| -> Result<&[u8], KeyedReduceStateError> {
            let end = at.checked_add(len).ok_or(KeyedReduceStateError::Decode)?;
            let slice = bytes.get(at..end).ok_or(KeyedReduceStateError::Decode)?;
            at = end;
            Ok(slice)
        };

        let count = u32::from_be_bytes(take(4)?.try_into().expect("counted four bytes"));
        let mut live = BTreeMap::new();
        let mut previous: Option<Box<str>> = None;
        for _ in 0..count {
            let len = u32::from_be_bytes(take(4)?.try_into().expect("counted four bytes")) as usize;
            let key = core::str::from_utf8(take(len)?)
                .map_err(|_| KeyedReduceStateError::Decode)?
                .to_owned()
                .into_boxed_str();
            let raw = u64::from_be_bytes(take(8)?.try_into().expect("counted eight bytes"));
            let value = f64::from_bits(raw);
            if value.is_nan() && FloatValue::new(value).to_bits() != raw {
                return Err(KeyedReduceStateError::Invariant);
            }
            if previous
                .as_deref()
                .is_some_and(|prior| prior >= key.as_ref())
            {
                return Err(KeyedReduceStateError::Invariant);
            }
            previous = Some(key.clone());
            live.insert(key, FloatValue::new(value));
        }

        if at != bytes.len() {
            return Err(KeyedReduceStateError::Decode);
        }
        Ok(Self { live })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Eviction {
    Released { released: FloatValue },
    NotLive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyedReduceStateError {
    Decode,
    Invariant,
}

impl KeyedReduceStateError {
    fn into_restore<V>(self, schema: V) -> ActorRestoreError<V> {
        match self {
            Self::Decode => ActorRestoreError::DecodeFailed { schema },
            Self::Invariant => ActorRestoreError::StateInvariantViolated { schema },
        }
    }
}

impl fmt::Display for KeyedReduceStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode => {
                formatter.write_str("keyed_reduce state bytes do not match this schema")
            }
            Self::Invariant => {
                formatter.write_str("keyed_reduce state values are outside the declared domain")
            }
        }
    }
}

impl std::error::Error for KeyedReduceStateError {}

pub struct KeyedReduceActor<V, I> {
    state: KeyedReduceState,
    config: KeyedReduceConfig,
    marker: PhantomData<fn() -> (V, I)>,
}

impl<V, I> KeyedReduceActor<V, I> {
    #[must_use]
    pub fn new(config: KeyedReduceConfig) -> Self {
        Self {
            state: KeyedReduceState::new(),
            config,
            marker: PhantomData,
        }
    }

    #[must_use]
    pub const fn state(&self) -> &KeyedReduceState {
        &self.state
    }

    #[must_use]
    pub const fn config(&self) -> &KeyedReduceConfig {
        &self.config
    }

    pub fn set_state(&mut self, state: KeyedReduceState) {
        self.state = state;
    }

    pub fn admit_delta(&mut self, payload: &Value) -> Option<FloatValue> {
        let key = self.config.key_of(payload)?.to_owned();
        let delta = self.config.delta_of(payload)?;
        Some(self.state.accumulate(&key, delta))
    }

    #[must_use]
    pub fn project(&self) -> ActorEffects<crate::actor_registry::ProductPayload> {
        ActorEffects::emit(keyed_port("map"), map_payload(&self.state))
            .concat(ActorEffects::emit(
                keyed_port("total"),
                number_payload(self.state.total()),
            ))
            .concat(ActorEffects::emit(
                keyed_port("count"),
                count_payload(self.state.count()),
            ))
    }

    #[must_use]
    pub fn tally(&self) -> KeyedReduceTally {
        KeyedReduceTally {
            total: self.state.total(),
            keys: self.state.count(),
        }
    }

    pub fn retire(&mut self, payload: &Value) -> Eviction {
        match self.config.key_of(payload) {
            Some(key) => {
                let key = key.to_owned();
                self.state.evict(&key)
            }
            None => Eviction::NotLive,
        }
    }
}

impl<V, I> EditableActor for KeyedReduceActor<V, I>
where
    V: Clone + From<u16> + PartialEq,
    I: Clone + Ord,
{
    type StateVersion = V;
    type EffectId = I;

    fn on_config_change(&mut self, _config: &FoldedConfig) -> ConfigChangeOutcome {
        ConfigChangeOutcome::ReplaceIncarnation
    }

    fn checkpoint(&self) -> Option<ActorState<Self::StateVersion>> {
        Some(ActorState::new(
            V::from(KEYED_REDUCE_STATE_SCHEMA),
            self.state.encode(),
        ))
    }

    fn restore(
        &mut self,
        state: ActorState<Self::StateVersion>,
    ) -> Result<(), ActorRestoreError<Self::StateVersion>> {
        let (schema, bytes) = state.into_parts();
        if schema != V::from(KEYED_REDUCE_STATE_SCHEMA) {
            return Err(ActorRestoreError::SchemaBeyondLadder { schema });
        }
        let restored =
            KeyedReduceState::decode(&bytes).map_err(|error| error.into_restore(schema))?;
        self.state = restored;
        Ok(())
    }
}

fn keyed_port(name: &'static str) -> circular_core::PortId {
    circular_core::PortId::try_new(name).expect("registered keyed_reduce port is canonical")
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyedReduceTally {
    total: FloatValue,
    keys: i64,
}

impl KeyedReduceTally {
    pub const TOTAL: &'static str = "total";
    pub const KEYS: &'static str = "keys";

    pub const SLOTS: [&'static str; 2] = [Self::TOTAL, Self::KEYS];

    #[must_use]
    pub const fn total(self) -> FloatValue {
        self.total
    }

    #[must_use]
    pub const fn keys(self) -> i64 {
        self.keys
    }
}

fn number_payload(value: FloatValue) -> crate::actor_registry::ProductPayload {
    crate::actor_registry::ProductPayload::new(
        crate::GroundShape::try_new(crate::Shape::Base(crate::BaseShape::Float))
            .expect("Float is ground"),
        Value::Float(value),
    )
}

fn count_payload(count: i64) -> crate::actor_registry::ProductPayload {
    crate::actor_registry::ProductPayload::new(
        crate::GroundShape::try_new(crate::Shape::Base(crate::BaseShape::Int))
            .expect("Int is ground"),
        Value::int(count),
    )
}

fn map_payload(state: &KeyedReduceState) -> crate::actor_registry::ProductPayload {
    let entries = state
        .entries()
        .map(|(key, value)| (key.to_owned(), Value::Float(value)))
        .collect::<Vec<_>>();
    crate::actor_registry::ProductPayload::new(
        crate::GroundShape::try_new(crate::Shape::Object {
            fields: crate::FieldMap::try_new(Vec::new()).expect("an empty field table is unique"),
            open: true,
        })
        .expect("an open object is ground"),
        Value::object(entries).expect("the table keys are already unique"),
    )
}

pub struct KeyedReduceFactory<T>(PhantomData<fn() -> T>);

impl<T> EmittingActorFactory<crate::actor_registry::ProductPayload> for KeyedReduceFactory<T>
where
    T: ActorTypes<Payload = crate::actor_registry::ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
{
    const TYPE: crate::ActorType = crate::ActorType::KeyedReduce;
    const DISPOSITION: circular_runtime::EditDisposition =
        circular_runtime::EditDisposition::Restarts;
    const CHECKPOINTS: bool = true;
    type Grants = T::Grants;
    type Types = T;
    type Instance = KeyedReduceActor<T::StateVersion, T::EffectId>;
    type Error = KeyedReduceConfigError;

    fn create(config: &FoldedConfig, _grants: &T::Grants) -> Result<Self::Instance, Self::Error> {
        Ok(KeyedReduceActor::new(KeyedReduceConfig::from_value(
            config.value(),
        )?))
    }
}

impl<T> EmittingActor<T, crate::actor_registry::ProductPayload>
    for KeyedReduceActor<T::StateVersion, T::EffectId>
where
    T: ActorTypes<Payload = crate::actor_registry::ProductPayload>,
    T::StateVersion: From<u16> + PartialEq,
{
    fn on_event(
        &mut self,
        input: &ActorInput<T::Event>,
        _context: &ActorContext<'_, T::Stream, T::Grants>,
    ) -> ActorEffects<crate::actor_registry::ProductPayload> {
        let payload = input.payload::<T>();
        let changed = match input.inlet().as_str() {
            "event" => self.admit_delta(payload.value()).is_some(),
            "remove" => matches!(self.retire(payload.value()), Eviction::Released { .. }),
            _ => false,
        };
        if !changed {
            return ActorEffects::empty();
        }
        self.project()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Actor = KeyedReduceActor<u16, u64>;

    #[test]
    fn the_three_projections_come_from_one_state_at_one_time() {
        let mut actor = Actor::new(config());
        actor.admit_delta(&delta("s1", 100));
        actor.admit_delta(&delta("s2", 7));

        let effects: ActorEffects<crate::actor_registry::ProductPayload> = actor.project();
        let ports = effects
            .iter()
            .filter_map(|effect| match effect {
                circular_runtime::ActorEffect::Emit { port, .. } => Some(port.as_str().to_owned()),
                circular_runtime::ActorEffect::Suppress { .. }
                | circular_runtime::ActorEffect::DeadLetter { .. }
                | circular_runtime::ActorEffect::Reject { .. }
                | circular_runtime::ActorEffect::External(_)
                | circular_runtime::ActorEffect::Peer(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(ports, ["map", "total", "count"]);

        let values = effects
            .iter()
            .filter_map(|effect| match effect {
                circular_runtime::ActorEffect::Emit { payload, .. } => {
                    Some(payload.value().clone())
                }
                circular_runtime::ActorEffect::Suppress { .. }
                | circular_runtime::ActorEffect::DeadLetter { .. }
                | circular_runtime::ActorEffect::Reject { .. }
                | circular_runtime::ActorEffect::External(_)
                | circular_runtime::ActorEffect::Peer(_) => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(values[1], Value::Float(FloatValue::new(107.0)));
        assert_eq!(values[2], Value::int(2));
        let map = values[0].as_object().expect("map is an object");
        assert_eq!(map.len(), 2);
        assert_eq!(map.get("s1"), Some(&Value::Float(FloatValue::new(100.0))));
    }

    #[test]
    fn an_arrival_that_does_not_change_the_table_projects_nothing() {
        let mut actor = Actor::new(config());
        assert!(actor.admit_delta(&Value::int(1)).is_none());
        assert_eq!(actor.retire(&retire("ghost")), Eviction::NotLive);
        assert!(actor.state().is_empty());
    }

    #[test]
    fn a_retirement_drops_all_three_projections_together() {
        let mut actor = Actor::new(config());
        actor.admit_delta(&delta("s1", 100));
        actor.admit_delta(&delta("s2", 7));
        assert_eq!(
            actor.retire(&retire("s1")),
            Eviction::Released {
                released: FloatValue::new(100.0)
            }
        );

        let effects: ActorEffects<crate::actor_registry::ProductPayload> = actor.project();
        let values = effects
            .iter()
            .filter_map(|effect| match effect {
                circular_runtime::ActorEffect::Emit { payload, .. } => {
                    Some(payload.value().clone())
                }
                circular_runtime::ActorEffect::Suppress { .. }
                | circular_runtime::ActorEffect::DeadLetter { .. }
                | circular_runtime::ActorEffect::Reject { .. }
                | circular_runtime::ActorEffect::External(_)
                | circular_runtime::ActorEffect::Peer(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(values[1], Value::Float(FloatValue::new(7.0)));
        assert_eq!(values[2], Value::int(1));
        assert_eq!(values[0].as_object().expect("it is an object").len(), 1);
    }

    #[test]
    fn the_declared_slots_agree_with_the_three_projections_at_the_same_instant() {
        let mut actor = Actor::new(config());
        actor.admit_delta(&delta("s1", 100));
        actor.admit_delta(&delta("s2", 7));

        let tally = actor.tally();
        let effects: ActorEffects<crate::actor_registry::ProductPayload> = actor.project();
        let values = effects
            .iter()
            .filter_map(|effect| match effect {
                circular_runtime::ActorEffect::Emit { payload, .. } => {
                    Some(payload.value().clone())
                }
                circular_runtime::ActorEffect::External(_)
                | circular_runtime::ActorEffect::Peer(_)
                | circular_runtime::ActorEffect::Suppress { .. }
                | circular_runtime::ActorEffect::DeadLetter { .. }
                | circular_runtime::ActorEffect::Reject { .. } => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(
            values[1],
            Value::Float(tally.total()),
            "equals the total projection"
        );
        assert_eq!(
            values[2],
            Value::int(tally.keys()),
            "equals the count projection"
        );
    }

    #[test]
    fn the_declaration_names_exactly_two_distinct_slots() {
        assert_eq!(
            KeyedReduceTally::SLOTS,
            [KeyedReduceTally::TOTAL, KeyedReduceTally::KEYS]
        );
        assert_eq!(
            KeyedReduceTally::SLOTS
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            2,
            "two fields with the same name cannot be told apart from outside"
        );
        for name in KeyedReduceTally::SLOTS {
            assert_eq!(
                crate::Name::from_normalized(name).as_str(),
                name,
                "{name} does not normalize unchanged as a runtime name"
            );
        }
    }

    fn config() -> KeyedReduceConfig {
        let value = Value::object([
            (
                KeyedReduceConfig::AT.to_owned(),
                Value::Array(vec![Value::string("sessionId")]),
            ),
            (
                KeyedReduceConfig::VALUE.to_owned(),
                Value::Array(vec![Value::string("tokens")]),
            ),
        ])
        .expect("the two fields differ");
        KeyedReduceConfig::from_value(&value).expect("the test config is valid")
    }

    fn delta(session: &str, tokens: i64) -> Value {
        Value::object([
            ("sessionId".to_owned(), Value::string(session)),
            ("tokens".to_owned(), Value::int(tokens)),
        ])
        .expect("the two fields differ")
    }

    fn retire(session: &str) -> Value {
        Value::object([
            ("sessionId".to_owned(), Value::string(session)),
            ("removed".to_owned(), Value::Bool(true)),
        ])
        .expect("the two fields differ")
    }

    #[test]
    fn deltas_accumulate_per_key_and_the_total_is_their_sum() {
        let mut actor = Actor::new(config());

        actor.admit_delta(&delta("s1", 100));
        actor.admit_delta(&delta("s1", 50));
        actor.admit_delta(&delta("s2", 7));

        assert_eq!(actor.state().get("s1").map(FloatValue::get), Some(150.0));
        assert_eq!(actor.state().get("s2").map(FloatValue::get), Some(7.0));
        assert_eq!(actor.state().count(), 2);
        assert_eq!(actor.state().total().get(), 157.0);
    }

    #[test]
    fn retiring_a_key_drops_its_contribution_from_both_the_total_and_the_count() {
        let mut actor = Actor::new(config());
        actor.admit_delta(&delta("s1", 100));
        actor.admit_delta(&delta("s2", 7));
        assert_eq!(
            (actor.state().count(), actor.state().total().get()),
            (2, 107.0)
        );

        assert_eq!(
            actor.retire(&retire("s1")),
            Eviction::Released {
                released: FloatValue::new(100.0)
            }
        );
        assert_eq!(
            (actor.state().count(), actor.state().total().get()),
            (1, 7.0)
        );

        assert_eq!(actor.retire(&retire("s1")), Eviction::NotLive);
        assert_eq!(actor.state().count(), 1);
    }

    #[test]
    fn a_retired_key_that_returns_starts_from_zero() {
        let mut actor = Actor::new(config());
        actor.admit_delta(&delta("s1", 100));
        actor.retire(&retire("s1"));
        actor.admit_delta(&delta("s1", 5));

        assert_eq!(actor.state().get("s1").map(FloatValue::get), Some(5.0));
        assert_eq!(actor.state().total().get(), 5.0);
    }

    #[test]
    fn an_arrival_without_a_key_or_a_number_leaves_the_table_alone() {
        let mut actor = Actor::new(config());
        actor.admit_delta(&delta("s1", 10));
        let before = actor.state().clone();

        assert_eq!(
            actor.admit_delta(&Value::object([("tokens".to_owned(), Value::int(5))]).unwrap()),
            None
        );
        assert_eq!(
            actor.admit_delta(
                &Value::object([("sessionId".to_owned(), Value::string("s2"))]).unwrap()
            ),
            None
        );
        assert_eq!(
            actor.admit_delta(
                &Value::object([
                    ("sessionId".to_owned(), Value::string("s2")),
                    ("tokens".to_owned(), Value::string("many")),
                ])
                .unwrap()
            ),
            None
        );

        assert_eq!(actor.state(), &before);
    }

    #[test]
    fn integer_and_float_deltas_both_accumulate() {
        let mut actor = Actor::new(config());
        actor.admit_delta(&delta("s1", 3));
        actor.admit_delta(
            &Value::object([
                ("sessionId".to_owned(), Value::string("s1")),
                ("tokens".to_owned(), Value::float(0.5)),
            ])
            .unwrap(),
        );
        actor.admit_delta(
            &Value::object([
                ("sessionId".to_owned(), Value::string("s1")),
                ("tokens".to_owned(), Value::UInt(2)),
            ])
            .unwrap(),
        );
        assert_eq!(actor.state().get("s1").map(FloatValue::get), Some(5.5));
    }

    #[test]
    fn the_total_folds_in_a_fixed_order_so_one_state_has_one_sum() {
        let mut first = Actor::new(config());
        let mut second = Actor::new(config());

        for (session, tokens) in [("a", 1e16), ("b", 1.0), ("c", -1e16)] {
            first.admit_delta(
                &Value::object([
                    ("sessionId".to_owned(), Value::string(session)),
                    ("tokens".to_owned(), Value::float(tokens)),
                ])
                .unwrap(),
            );
        }
        for (session, tokens) in [("c", -1e16), ("b", 1.0), ("a", 1e16)] {
            second.admit_delta(
                &Value::object([
                    ("sessionId".to_owned(), Value::string(session)),
                    ("tokens".to_owned(), Value::float(tokens)),
                ])
                .unwrap(),
            );
        }

        assert_eq!(first.state(), second.state());
        assert_eq!(
            first.state().total(),
            second.state().total(),
            "if one state yields two sums, total is not a function of the state"
        );
    }

    #[test]
    fn the_map_reads_in_lexicographic_order() {
        let mut actor = Actor::new(config());
        for session in ["s3", "s1", "s2"] {
            actor.admit_delta(&delta(session, 1));
        }
        assert_eq!(
            actor
                .state()
                .entries()
                .map(|(key, _)| key)
                .collect::<Vec<_>>(),
            ["s1", "s2", "s3"]
        );
    }

    #[test]
    fn the_actor_round_trips_and_a_failed_restore_leaves_the_state_untouched() {
        let mut actor = Actor::new(config());
        actor.admit_delta(&delta("s1", 100));
        actor.admit_delta(&delta("s2", 7));

        let checkpoint = actor.checkpoint().expect("an element that carries state");
        let mut restored = Actor::new(config());
        restored
            .restore(checkpoint.clone())
            .expect("canonical bytes");
        assert_eq!(restored.state(), actor.state());
        assert_eq!(restored.checkpoint(), Some(checkpoint));

        let before = actor.state().clone();
        let error = actor
            .restore(ActorState::new(KEYED_REDUCE_STATE_SCHEMA, vec![0_u8, 0]))
            .expect_err("truncated bytes are not in this schema's shape");
        assert!(matches!(error, ActorRestoreError::DecodeFailed { .. }));
        assert_eq!(actor.state(), &before);

        let error = actor
            .restore(ActorState::new(
                KEYED_REDUCE_STATE_SCHEMA + 1,
                actor.state().encode().to_vec(),
            ))
            .expect_err("a schema outside the ladder is refused before the bytes are read");
        assert!(matches!(
            error,
            ActorRestoreError::SchemaBeyondLadder { .. }
        ));
    }

    #[test]
    fn a_non_canonical_key_order_is_out_of_domain() {
        let entry = |key: &str| {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&(key.len() as u32).to_be_bytes());
            bytes.extend_from_slice(key.as_bytes());
            bytes.extend_from_slice(&FloatValue::new(1.0).to_bits().to_be_bytes());
            bytes
        };

        let mut descending = 2_u32.to_be_bytes().to_vec();
        descending.extend(entry("b"));
        descending.extend(entry("a"));
        assert_eq!(
            KeyedReduceState::decode(&descending),
            Err(KeyedReduceStateError::Invariant)
        );

        let mut duplicated = 2_u32.to_be_bytes().to_vec();
        duplicated.extend(entry("a"));
        duplicated.extend(entry("a"));
        assert_eq!(
            KeyedReduceState::decode(&duplicated),
            Err(KeyedReduceStateError::Invariant)
        );

        let mut nan = 1_u32.to_be_bytes().to_vec();
        nan.extend_from_slice(&1_u32.to_be_bytes());
        nan.push(b'a');
        nan.extend_from_slice(&(f64::NAN.to_bits() ^ 1).to_be_bytes());
        assert_eq!(
            KeyedReduceState::decode(&nan),
            Err(KeyedReduceStateError::Invariant)
        );

        let mut trailing = KeyedReduceState::new().encode().into_vec();
        trailing.push(0);
        assert_eq!(
            KeyedReduceState::decode(&trailing),
            Err(KeyedReduceStateError::Decode)
        );
    }

    #[test]
    fn every_config_rejection_is_its_own_arm() {
        assert_eq!(
            KeyedReduceConfig::from_value(&Value::int(1)),
            Err(KeyedReduceConfigError::NotAnObject)
        );
        assert_eq!(
            KeyedReduceConfig::from_value(
                &Value::object([(
                    KeyedReduceConfig::VALUE.to_owned(),
                    Value::Array(Vec::new())
                )])
                .unwrap()
            ),
            Err(KeyedReduceConfigError::MissingAt)
        );
        assert_eq!(
            KeyedReduceConfig::from_value(
                &Value::object([(KeyedReduceConfig::AT.to_owned(), Value::Array(Vec::new()))])
                    .unwrap()
            ),
            Err(KeyedReduceConfigError::MissingValue)
        );
    }

    #[test]
    fn one_key_path_serves_both_arrivals() {
        let config = config();
        assert_eq!(config.key_of(&delta("s1", 1)), Some("s1"));
        assert_eq!(config.key_of(&retire("s1")), Some("s1"));
    }
}
