//! Accumulator-family registration sources.

use super::support::*;

pub(crate) fn token_cost_meter_source() -> SpecSource<NoExternalEffect> {
    let fixed = PortSet::try_new(
        vec![required_primary_inlet(
            "usage",
            Flow::Stream(open_object_shape()),
            "Usage",
        )],
        vec![
            primary_outlet("totals", Flow::Stream(open_object_shape()), "Totals"),
            outlet(
                "exceeded",
                Flow::Stream(open_object_shape()),
                false,
                "Exceeded",
            ),
        ],
    )
    .expect("token meter output roles have distinct names");

    published_effect_free_source(
        actor_label("Token Cost Meter"),
        Description::from_static("Accumulate usage totals and report the first budget crossing."),
        PortRule::new(fixed, Box::new([])),
        blocked_config_schema(
            [(
                config_path("at"),
                ConfigSlot::try_new(
                    ConfigSpace::try_new(
                        Shape::Array(Box::new(Shape::Any)),
                        ConfigConstraint::ExactPayloadPath,
                    )
                    .expect("token meter group selector preserves the exact payload-path domain"),
                    Required::Mandatory,
                    None,
                )
                .expect("mandatory group path has no default to mismatch"),
            )],
            "token_cost_meter has only its group path; accounting value axes and budget-threshold policy are not registered",
        ),
    )
}

pub(crate) fn counter_source() -> SpecSource<NoExternalEffect> {
    let fixed = PortSet::try_new(
        vec![required_primary_inlet(
            "event",
            Flow::Stream(Shape::Any),
            "Event",
        )],
        vec![primary_outlet(
            "count",
            Flow::Stream(Shape::Base(BaseShape::Int)),
            "Count",
        )],
    )
    .expect("counter has one inlet and one outlet");

    published_effect_free_source(
        actor_label("Counter"),
        Description::from_static("Count the events that arrive."),
        PortRule::new(fixed, Box::new([])),
        ConfigSchema::empty(),
    )
    .with_view_config(view_defaults([
        ("heading", Value::string("Accepted arrivals")),
        ("side", Value::string("emitted")),
        ("spark", Value::string("none")),
    ]))
}

pub(crate) fn ema_source() -> SpecSource<NoExternalEffect> {
    let result = Shape::Object {
        fields: FieldMap::try_new(vec![
            (Name::from_static("value"), Shape::Base(BaseShape::Float)),
            (Name::from_static("samples"), Shape::Base(BaseShape::Int)),
        ])
        .expect("ema result fields are distinct"),
        open: false,
    };
    let fixed = PortSet::try_new(
        vec![required_primary_inlet(
            "sample",
            Flow::Stream(Shape::Base(BaseShape::Float)),
            "Sample",
        )],
        vec![primary_outlet("ema", Flow::Stream(result), "EMA")],
    )
    .expect("ema has one inlet and one outlet");

    published_effect_free_source(
        actor_label("EMA"),
        Description::from_static("Smooth incoming numbers into a moving average."),
        PortRule::new(fixed, Box::new([])),
        create_config_schema([
            described(
                mandatory_typed(&crate::ema_state::HALF_LIFE),
                "Half-life",
                "The half-life of the decay.",
            ),
            described(
                optional_typed(
                    &crate::ema_state::TIME_BASIS,
                    Value::String("samples".to_owned()),
                ),
                "Time basis",
                "Which distance the decay is measured in.",
            ),
        ]),
    )
    .with_view_config(view_defaults([
        ("heading", Value::string("Weighted mean")),
        ("count_label", Value::string("samples")),
        ("side", Value::string("emitted")),
        ("spark", Value::string("samples")),
    ]))
}

pub(crate) fn windowed_reduce_source() -> SpecSource<NoExternalEffect> {
    let reduce = config_path("reduce");
    let fixed = PortSet::try_new(
        vec![required_primary_inlet(
            "sample",
            Flow::Stream(Shape::Base(BaseShape::Float)),
            "Sample",
        )],
        Vec::new(),
    )
    .expect("windowed_reduce has one sample inlet");
    let aggregate = DynamicRule::new(
        reduce.clone(),
        ExpansionRule::Presence {
            id: authored_port_id("aggregate"),
        },
        PortTemplate::Outlet(OutletTemplate::new(
            TypeRule::FromSnippet {
                at: reduce.clone(),
                inputs: vec![port_reference("sample")].into_boxed_slice(),
                flow: FlowCtor::Stream,
            },
            Arity::Many,
            LabelRule::literal(port_label("Aggregate")),
        )),
    );

    published_effect_free_source(
        actor_label("Windowed Reduce"),
        Description::from_static("Combine the numbers from a recent time window, at a regular interval."),
        PortRule::new(fixed, vec![aggregate].into_boxed_slice()),
        create_config_schema([
            starting(
timed(
                mandatory_typed(&crate::windowed_reduce::WINDOW_LENGTH),
                "Window length",
                "The width of the window.",
            ),
Value::Int(60_000),
),
            described(
                (
                    reduce,
                    mandatory_slot(
                        Shape::Base(BaseShape::String),
                        Some(single_input_snippet(EvalMode::Reduce, "sample")),
                    ),
                ),
                "Fold step",
                "The fold step, evaluated with two bindings: acc, the value folded so far, and sample, the incoming value.",
            ),
            starting(
timed(
                mandatory_typed(&crate::windowed_reduce::EMISSION_PERIOD),
                "Emission period",
                "How often the window is folded.",
            ),
Value::Int(5_000),
),
            described(
                (
                    config_path("seed"),
                    mandatory_slot(Shape::Any, None),
                ),
                "Starting value",
                "The starting value of the fold.",
            ),
        ]),
    )
    .with_view_config(view_defaults([
        ("heading", Value::string("Window aggregate")),
        ("caption", Value::string("Current window")),
        ("side", Value::string("emitted")),
        ("spark", Value::string("samples")),
    ]))
}

pub(crate) fn keyed_reduce_source() -> SpecSource<NoExternalEffect> {
    let keyed_path = || {
        ConfigSlot::try_new(
            ConfigSpace::try_new(
                Shape::Array(Box::new(Shape::Any)),
                ConfigConstraint::ExactPayloadPath,
            )
            .expect("keyed_reduce selectors preserve the exact payload-path domain"),
            Required::Mandatory,
            None,
        )
        .expect("mandatory keyed_reduce path has no default to mismatch")
    };

    let fixed = PortSet::try_new(
        vec![
            required_inlet("event", Flow::Stream(open_object_shape()), true, "Event"),
            required_inlet("remove", Flow::Stream(open_object_shape()), false, "Remove"),
        ],
        vec![
            primary_outlet("map", Flow::Stream(open_object_shape()), "Map"),
            outlet(
                "total",
                Flow::Stream(Shape::Base(BaseShape::Float)),
                false,
                "Total",
            ),
            outlet(
                "count",
                Flow::Stream(Shape::Base(BaseShape::Int)),
                false,
                "Count",
            ),
        ],
    )
    .expect("keyed_reduce port roles have distinct names");

    published_effect_free_source(
        actor_label("Keyed Reduce"),
        Description::from_static(
            "Keep a running total for each key, with the overall sum and the number of keys.",
        ),
        PortRule::new(fixed, Box::new([])),
        create_config_schema([
            described(
                (config_path("at"), keyed_path()),
                "Key path",
                "The exact payload path that selects the key.",
            ),
            described(
                (config_path("value"), keyed_path()),
                "Value path",
                "The exact payload path that selects the number to add.",
            ),
        ]),
    )
    .with_view_config(view_defaults([
        ("heading", Value::string("Values by key")),
        ("rows", Value::string("latest")),
        (
            "total",
            view_defaults([
                ("outlet", Value::string("total")),
                ("path", Value::Array(Vec::new())),
            ]),
        ),
    ]))
}

pub(crate) fn assemble_source() -> SpecSource<NoExternalEffect> {
    published_effect_free_source(
        actor_label("Assemble"),
        Description::from_static(
            "Collect records with the same key, and send them as one object when the group closes.",
        ),
        event_passthrough_rule(open_object_shape()),
        create_config_schema([
            described(
                (
                    config_path("at"),
                    ConfigSlot::try_new(
                        ConfigSpace::try_new(
                            Shape::Array(Box::new(Shape::Any)),
                            ConfigConstraint::ExactPayloadPath,
                        )
                        .unwrap(),
                        Required::Mandatory,
                        None,
                    )
                    .unwrap(),
                ),
                "Key path",
                "The exact payload path that selects the key.",
            ),
            starting(
                timed(
                    (
                        config_path("inactivity_timeout"),
                        mandatory_slot_in(
                            interval_space(IntervalDomain::NonZeroMilliseconds),
                            None,
                        ),
                    ),
                    "Inactivity timeout",
                    "How long a group may go without a record before it closes.",
                ),
                Value::Int(3_000),
            ),
            starting(
                timed(
                    (
                        config_path("max_window"),
                        mandatory_slot_in(
                            interval_space(IntervalDomain::NonZeroMilliseconds),
                            None,
                        ),
                    ),
                    "Maximum window",
                    "The maximum age of a group, measured from when it opened.",
                ),
                Value::Int(30_000),
            ),
            starting(
                described(
                    mandatory_typed(&crate::assemble::CAPACITY),
                    "Capacity",
                    "How many groups may be open at once.",
                ),
                Value::Int(1024),
            ),
        ]),
    )
    .with_view_config(view_defaults([(
        "heading",
        Value::string("Open assemblies"),
    )]))
}

pub(crate) fn join_source() -> SpecSource<NoExternalEffect> {
    let any = Flow::Stream(Shape::Any);
    let joined = Shape::Object {
        fields: FieldMap::try_new(vec![
            (Name::from_static("event"), Shape::Any),
            (Name::from_static("state"), Shape::Any),
        ])
        .unwrap(),
        open: false,
    };
    let ports = PortSet::try_new(
        vec![
            required_inlet("event", any.clone(), true, "Event"),
            required_inlet("state", any.clone(), false, "State"),
            required_inlet("remove", any, false, "Remove"),
        ],
        vec![primary_outlet("event", Flow::Stream(joined), "Event")],
    )
    .unwrap();
    published_effect_free_source(
        actor_label("Join"),
        Description::from_static("Add the latest reference value for the same key to each event."),
        PortRule::new(ports, Box::new([])),
        create_config_schema([described(
            (
                config_path(crate::keyed_reduce::KeyedReduceConfig::AT),
                ConfigSlot::try_new(
                    ConfigSpace::try_new(
                        Shape::Array(Box::new(Shape::Any)),
                        ConfigConstraint::ExactPayloadPath,
                    )
                    .unwrap(),
                    Required::Mandatory,
                    None,
                )
                .unwrap(),
            ),
            "Key path",
            "The exact payload path that selects the key.",
        )]),
    )
    .with_view_config(view_defaults([
        ("heading", Value::string("Enriched events")),
        ("rows", Value::string("latest")),
    ]))
}
