
use circular_runtime::ConfigChangeOutcome;
pub use circular_runtime::EditDisposition;

use crate::ActorType;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Editability {
    pub actor_type: ActorType,
    pub actor: &'static str,
    pub outcome: ConfigChangeOutcome,
    pub absorbs_some_changes: bool,
    pub checkpoints: bool,
    pub boarded: bool,
    pub boarded_without: Option<&'static str>,
    pub reason: &'static str,
}

impl Editability {
    pub(crate) const fn from_disposition(
        actor_type: ActorType,
        actor: &'static str,
        disposition: EditDisposition,
        checkpoints: bool,
        boarded: bool,
        boarded_without: Option<&'static str>,
        reason: &'static str,
    ) -> Self {
        let (outcome, absorbs_some_changes) = match disposition {
            EditDisposition::Absorbs => (ConfigChangeOutcome::Absorbed, false),
            EditDisposition::AbsorbsSome => (ConfigChangeOutcome::ReplaceIncarnation, true),
            EditDisposition::Restarts => (ConfigChangeOutcome::ReplaceIncarnation, false),
        };
        Self {
            actor_type,
            actor,
            outcome,
            absorbs_some_changes,
            checkpoints,
            boarded,
            boarded_without,
            reason,
        }
    }
}

const EDITABILITY_LEN: usize = crate::actor_registry::FACTORY_EDITABILITY.len() + 1;

const fn build_editability() -> [Editability; EDITABILITY_LEN] {
    let mut rows = [crate::actor_registry::FACTORY_EDITABILITY[0]; EDITABILITY_LEN];
    let mut output = 0;
    while output < crate::actor_registry::FACTORY_EDITABILITY.len() {
        rows[output] = crate::actor_registry::FACTORY_EDITABILITY[output];
        output += 1;
    }
    rows[output] = Editability::from_disposition(
        ActorType::Otlp,
        "OtlpSource",
        EditDisposition::Restarts,
        false,
        true,
        None,
        "listen changes replace only the Source listener; durable pending retains its custody identity",
    );
    rows
}

pub const EDITABILITY: [Editability; EDITABILITY_LEN] = build_editability();

#[must_use]
pub fn editability(actor_type: ActorType) -> Option<Editability> {
    EDITABILITY
        .into_iter()
        .find(|row| row.actor_type == actor_type)
}

#[cfg(test)]
mod tests {
    fn null_folded(actor_type: ActorType) -> circular_runtime::FoldedConfig {
        circular_runtime::FoldedConfig::minted(actor_type, circular_core::Value::Null)
    }

    use super::*;
    use crate::actor_registry::{ProductActor, product_actor_factory};
    use crate::ema_state::{EMA_STATE_SCHEMA, EmaActor, EmaState};
    use circular_core::{FloatValue, Value};
    use circular_runtime::{ActorState, EditableActor};

    type TestTypes = circular_testkit::types::TestTypes<crate::ProductPayload>;

    fn arm_actor_type(actor: &ProductActor<TestTypes>) -> ActorType {
        match actor {
            ProductActor::Source(_) => ActorType::Otlp,
            ProductActor::Listener(_) => ActorType::Listener,
            ProductActor::Match(_) => ActorType::Match,
            ProductActor::Join(_) => ActorType::Join,
            ProductActor::Tap(_) => ActorType::Tap,
            ProductActor::Input(_) => ActorType::Input,
            ProductActor::Output(_) => ActorType::Output,
            ProductActor::Form(_) => ActorType::Form,
            ProductActor::Counter(_) => ActorType::Counter,
            ProductActor::Route(_) => ActorType::Route,
            ProductActor::Ema(_) => ActorType::Ema,
            ProductActor::Debounce(_) => ActorType::Debounce,
            ProductActor::KeyedReduce(_) => ActorType::KeyedReduce,
            ProductActor::WindowedReduce(_) => ActorType::WindowedReduce,
            ProductActor::Replicator(_) => ActorType::Replicator,
            ProductActor::Agent(_) => ActorType::Agent,
            ProductActor::ToolExecutor(_) => ActorType::ToolExecutor,
            ProductActor::File(_) => ActorType::File,
            ProductActor::Request(_) => ActorType::Request,
            ProductActor::Peer(_) => ActorType::Peer,
            ProductActor::Notify(_) => ActorType::Notify,
            ProductActor::Timer(_) => ActorType::Timer,
            ProductActor::Json(_) => ActorType::Json,
            ProductActor::Alert(_) => ActorType::Alert,
            ProductActor::Assemble(_) => ActorType::Assemble,
            ProductActor::FixturePanic(_) => ActorType::FixturePanic,
        }
    }

    fn empty() -> Value {
        Value::object(Vec::<(String, Value)>::new()).expect("an empty config is an empty object")
    }

    fn one(key: &str, value: Value) -> Value {
        Value::object([(key.to_owned(), value)]).expect("one field has no duplicate")
    }

    fn minimal_config(actor_type: ActorType) -> Value {
        match actor_type {
            ActorType::Match | ActorType::Tap | ActorType::Counter => empty(),
            ActorType::Input | ActorType::Output => one("label", Value::string("event")),
            ActorType::Form => one("fields", Value::Array(Vec::new())),
            ActorType::Route => Value::object([
                (
                    crate::route_config::RouteConfig::AT.to_owned(),
                    Value::Array(vec![Value::string("kind")]),
                ),
                (
                    crate::route_config::RouteConfig::CASES.to_owned(),
                    one("even", Value::string("even")),
                ),
            ])
            .expect("the two fields differ"),
            ActorType::Join => one(
                crate::keyed_reduce::KeyedReduceConfig::AT,
                Value::array([Value::string("key")]),
            ),
            ActorType::Assemble => Value::object([
                ("at", Value::array([Value::string("key")])),
                ("inactivity_timeout", Value::int(3000)),
                ("max_window", Value::int(30000)),
                ("capacity", Value::int(2)),
            ])
            .unwrap(),
            ActorType::Ema => one("half_life", Value::Int(100)),
            ActorType::KeyedReduce => Value::object([
                (
                    crate::keyed_reduce::KeyedReduceConfig::AT.to_owned(),
                    Value::Array(vec![Value::string("sessionId")]),
                ),
                (
                    crate::keyed_reduce::KeyedReduceConfig::VALUE.to_owned(),
                    Value::Array(vec![Value::string("tokens")]),
                ),
            ])
            .expect("the two fields differ"),
            ActorType::WindowedReduce => Value::object([
                ("window_length", Value::int(60_000)),
                ("emission_period", Value::int(1_000)),
                ("reduce", Value::string("acc + sample")),
                ("seed", Value::float(0.0)),
            ])
            .expect("the four fields differ"),
            ActorType::Agent => Value::object([
                ("harness", Value::string("test-harness")),
                ("queue_capacity", Value::int(1)),
                ("tools", Value::Array(Vec::new())),
            ])
            .expect("the three fields differ"),
            ActorType::ToolExecutor => one("tools", empty()),
            ActorType::Listener => Value::object([
                (
                    "source",
                    Value::object([
                        ("kind", Value::string("file_tail")),
                        (
                            "value",
                            Value::object([
                                ("glob", Value::string("/listener-editability/*.jsonl")),
                                ("poll", Value::Int(500)),
                            ])
                            .unwrap(),
                        ),
                    ])
                    .unwrap(),
                ),
                (
                    "capabilities",
                    one(
                        "FsRead",
                        Value::object([
                            ("approval", Value::string("none")),
                            (
                                "roots",
                                Value::array([Value::string("/listener-editability")]),
                            ),
                        ])
                        .unwrap(),
                    ),
                ),
            ])
            .unwrap(),
            ActorType::File => one("path", Value::string("/file-editability/data")),
            ActorType::Peer => Value::object([
                ("adapter", Value::string("memory")),
                ("realm", Value::string("realm")),
                ("name", Value::string("name")),
                (
                    "inbound_policy",
                    Value::object([("any_known_peer", Value::Bool(true))]).unwrap(),
                ),
                ("inbox_capacity", Value::Int(1)),
            ])
            .expect("peer config keys are distinct"),
            ActorType::Request => Value::object([
                ("method", Value::string("post")),
                ("url", Value::string("https://example.test/events")),
                ("headers", Value::Array(Vec::new())),
            ])
            .expect("request config keys differ"),
            ActorType::Notify => Value::object([
                ("channel", Value::string("slack")),
                ("minimum_interval", Value::UInt(0)),
                ("during_interval", Value::string("suppress")),
            ])
            .expect("the three fields differ"),
            ActorType::Timer => one("every", Value::UInt(100)),
            ActorType::Debounce => one("quiet_window", Value::UInt(100)),
            ActorType::Json => one("initial", Value::Int(7)),
            ActorType::Alert => Value::object([
                ("predicate", Value::string("event.n > 0")),
                ("firing_delay", Value::Int(10)),
                ("recovery_delay", Value::Int(20)),
            ])
            .expect("the three fields differ"),
            other => panic!("{other:?} is not an onboarded arm"),
        }
    }

    fn boarded_actor(actor_type: ActorType) -> ProductActor<TestTypes> {
        if actor_type == ActorType::Otlp {
            let folded = circular_runtime::FoldedConfig::minted(
                actor_type,
                Value::object([("listen", Value::string("127.0.0.1:4318"))]).unwrap(),
            );
            return ProductActor::Source(crate::otlp::OtlpSource::create(&folded).unwrap());
        }
        if actor_type == ActorType::Agent {
            let harness = circular_runtime::AgentHarnessName::try_from_normalized("test-harness")
                .expect("nonempty harness");
            let grant = circular_runtime::AgentHarnessGrant::agent_harness([harness.clone()]);
            let evidence = circular_runtime::GrantIssuer::new().issue(&grant);
            return ProductActor::Agent(crate::AgentActor::for_test(harness, evidence));
        }
        let folded = circular_runtime::FoldedConfig::minted(actor_type, minimal_config(actor_type));
        product_actor_factory::<TestTypes>(actor_type)
            .expect("a name the table lists as onboarded")
            .create(&folded, &(), &crate::ResolvedInletShapes::default())
            .expect("the minimal config is accepted")
    }

    #[test]
    fn every_boarded_row_declares_the_outcome_its_actor_returns() {
        for row in EDITABILITY
            .into_iter()
            .filter(|row| row.boarded && row.actor_type != ActorType::Replicator)
        {
            let mut actor = boarded_actor(row.actor_type);
            assert_eq!(
                arm_actor_type(&actor),
                row.actor_type,
                "{}: the factory produced a different arm",
                row.actor
            );
            assert_eq!(
                actor.on_config_change(&null_folded(ActorType::FixtureMap)),
                row.outcome,
                "{}: the table's disposition differs from the actor",
                row.actor
            );
            assert_eq!(
                actor.checkpoint().is_some(),
                row.checkpoints,
                "{}: the table's checkpoint flag differs from the actor",
                row.actor
            );
        }
    }

    fn assert_round_trip<N>(prepared: &mut N, fresh: impl Fn() -> N)
    where
        N: EditableActor<StateVersion = u16>,
        N::EffectId: Clone + Ord,
    {
        let checkpoint = prepared.checkpoint().expect("a row that checkpoints");
        let bytes = checkpoint.clone();
        let mut restored = fresh();
        restored
            .restore(bytes)
            .expect("canonical bytes are restored");
        assert_eq!(
            restored.checkpoint(),
            Some(checkpoint),
            "restore(checkpoint(s)) does not produce the same bytes"
        );
    }

    #[test]
    fn a_partial_boarding_is_declared_in_the_table() {
        for row in EDITABILITY {
            let Some(missing) = row.boarded_without else {
                continue;
            };
            assert!(
                row.boarded,
                "{}: a row that is not onboarded carries a partial marker",
                row.actor
            );
            assert!(
                !missing.is_empty(),
                "{}: the partial marker is empty",
                row.actor
            );
        }
    }

    fn keyed_reduce_config() -> crate::keyed_reduce::KeyedReduceConfig {
        let value = Value::object([
            (
                crate::keyed_reduce::KeyedReduceConfig::AT.to_owned(),
                Value::Array(vec![Value::string("sessionId")]),
            ),
            (
                crate::keyed_reduce::KeyedReduceConfig::VALUE.to_owned(),
                Value::Array(vec![Value::string("tokens")]),
            ),
        ])
        .expect("the two fields differ");
        crate::keyed_reduce::KeyedReduceConfig::from_value(&value)
            .expect("the test config is valid")
    }
}
