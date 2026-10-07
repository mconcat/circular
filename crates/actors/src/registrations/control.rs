//! Control-family registration sources.

use super::support::*;

pub(crate) fn debounce_source() -> SpecSource<NoExternalEffect> {
    published_effect_free_source(
        actor_label("Debounce"),
        Description::from_static(
            "Wait until events stop for a while, then pass on the latest one.",
        ),
        event_passthrough_rule(Shape::Var(Name::from_static("T"))),
        create_config_schema([starting(
            timed(
                mandatory_typed(&crate::debounce::QUIET_WINDOW),
                "Quiet window",
                "How long the inlet must stay quiet before the pending payload is emitted.",
            ),
            Value::Int(500),
        )]),
    )
    .with_view_config(view_defaults([(
        "count_label",
        Value::string("values released"),
    )]))
}

pub(crate) fn throttle_source() -> SpecSource<NoExternalEffect> {
    published_effect_free_source(
        actor_label("Throttle"),
        Description::from_static("Immediately pass events separated by a minimum interval."),
        event_passthrough_rule(Shape::Var(Name::from_static("T"))),
        suppressed_create_config_schema(
            [(
                config_path("minimum_interval"),
                mandatory_slot_in(interval_space(IntervalDomain::Milliseconds), None),
            )],
            decision_suppression("throttled"),
        ),
    )
}

pub(crate) fn alert_source() -> SpecSource<NoExternalEffect> {
    let polymorphic = polymorphic_event_flow();
    let fixed = PortSet::try_new(
        vec![required_primary_inlet(
            "event",
            polymorphic.clone(),
            "Event",
        )],
        vec![
            primary_outlet("event", polymorphic, "Event"),
            OutletSpec::new(
                authored_port_id("transition"),
                Flow::Stream(alert_transition_shape()),
                Arity::Many,
                false,
                port_label("Transition"),
            ),
        ],
    )
    .expect("alert's event and transition outlets have distinct names");

    published_effect_free_source(
        actor_label("Alert"),
        Description::from_static("Check a condition on each event, and report when the alert starts firing and when it recovers."),
        PortRule::new(fixed, Box::new([])),
        create_config_schema([
            described(
                (
                    config_path("predicate"),
                    mandatory_slot(Shape::Base(BaseShape::String), Some(predicate_snippet())),
                ),
                "Condition",
                "Evaluated once per arrival. A boolean true is read as a violation.",
            ),
            starting(
                timed(
                    mandatory_typed(&crate::alert_actor::FIRING_DELAY),
                    "Firing delay",
                    "How long a violation must persist before the alert moves to firing.",
                ),
                Value::Int(30_000),
            ),
            starting(
                timed(
                    mandatory_typed(&crate::alert_actor::RECOVERY_DELAY),
                    "Recovery delay",
                    "How long a recovery must persist before the alert returns to ok.",
                ),
                Value::Int(60_000),
            ),
        ]),
    )
    .with_view_config(view_defaults([("heading", Value::string("Condition"))]))
}
