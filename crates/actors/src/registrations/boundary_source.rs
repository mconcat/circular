//! Boundary-source registration sources.

use super::support::*;

pub(crate) fn json_source() -> SpecSource<NoExternalEffect> {
    let bang = InletSpec::try_new(
        authored_port_id("bang"),
        Flow::Stream(Shape::Any),
        Arity::Many,
        Presence::Optional,
        true,
        port_label("Bang"),
    )
    .expect("optional json bang has no default to mismatch");
    let fixed = PortSet::try_new(
        vec![
            required_inlet("set", Flow::Stream(Shape::Any), false, "Set"),
            bang,
        ],
        vec![primary_outlet("value", Flow::Stream(Shape::Any), "Value")],
    )
    .expect("json roles are unique in each direction");
    published_effect_free_source(
        actor_label("JSON"),
        Description::from_static("Send a value you write, at start and whenever asked."),
        PortRule::new(fixed, Box::new([])),
        create_config_schema([described(
            (config_path("initial"), mandatory_slot(Shape::Any, None)),
            "Initial value",
            "The authored starting value.",
        )]),
    )
    .with_view_config(view_defaults([
        ("heading", Value::string("Stored value")),
        ("side", Value::string("emitted")),
        ("spark", Value::string("none")),
    ]))
}

pub(crate) fn timer_source() -> SpecSource<NoExternalEffect> {
    let bang = InletSpec::try_new(
        authored_port_id("bang"),
        Flow::Stream(Shape::Any),
        Arity::Many,
        Presence::Optional,
        true,
        port_label("Bang"),
    )
    .expect("optional timer inlet has no default to mismatch");
    let fixed = PortSet::try_new(
        vec![bang],
        vec![primary_outlet(
            "tick",
            Flow::Stream(Shape::Object {
                fields: FieldMap::try_new(vec![(
                    Name::from_static("sequence"),
                    Shape::Base(BaseShape::UInt),
                )])
                .expect("timer tick is one sequence field"),
                open: false,
            }),
            "Tick",
        )],
    )
    .expect("timer has one inlet and one outlet");

    published_effect_free_source(
        actor_label("Timer"),
        Description::from_static("Send a numbered tick at a fixed interval."),
        PortRule::new(fixed, Box::new([])),
        suppressed_create_config_schema(
            [starting(
                timed(
                    mandatory_typed(&crate::timer_actor::EVERY),
                    "Interval",
                    "The interval between ticks.",
                ),
                Value::Int(1000),
            )],
            decision_suppression(crate::STALE_TIMER_FIRE),
        ),
    )
    .with_view_config(view_defaults([
        ("heading", Value::string("Periodic ticks")),
        ("count_label", Value::string("ticks emitted")),
    ]))
}

pub(crate) fn listener_source() -> SpecSource<ExternalEffect> {
    let control = InletSpec::try_new(
        authored_port_id("control"),
        Flow::Stream(Shape::Object {
            fields: FieldMap::try_new(vec![(
                Name::from_static("op"),
                Shape::Base(BaseShape::String),
            )])
            .expect("control pulse has one field"),
            open: false,
        }),
        Arity::Many,
        Presence::Optional,
        true,
        port_label("Control"),
    )
    .expect("optional listener inlet has no default to mismatch");

    let fixed = PortSet::try_new(
        vec![control],
        vec![primary_outlet(
            "line",
            Flow::Stream(open_object_shape()),
            "Line",
        )],
    )
    .expect("listener has one inlet and one outlet");

    let file_tail = Shape::Object {
        fields: FieldMap::try_new(vec![
            (Name::from_static("glob"), Shape::Base(BaseShape::String)),
            (Name::from_static("poll"), Shape::Base(BaseShape::Int)),
        ])
        .expect("file tail fields are distinct"),
        open: false,
    };

    published_external_source(
        actor_label("Listener"),
        Description::from_static(
            "Follow new lines in files on this computer, and send them again from the start on request.",
        ),
        PortRule::new(fixed, Box::new([])),
        RequireRules::external(RequireRule::new(Capability::FsRead, Condition::Always), []),
        ExternalEffect::new(
            Durability::Direct,
            failed_stand_ins(EffectCtor::FileRead, []),
        ),
        create_config_schema([
            capabilities_slot(
                PolicyDemand::EveryDeclared,
                "The grant for reading the origin files.",
            ),
            described(
                (
                    config_path("source"),
                    mandatory_slot(
                        Shape::Object {
                            fields: FieldMap::try_new(vec![
                                (Name::from_static("kind"), Shape::Base(BaseShape::String)),
                                (Name::from_static("value"), file_tail),
                            ])
                            .expect("source fields are distinct"),
                            open: false,
                        },
                        None,
                    ),
                ),
                "Source",
                "The origin to listen to: kind file_tail, whose value names the files to follow (glob) and the milliseconds between reads (poll).",
            ),
        ]),
    )
    .with_view_config(view_defaults([
        ("heading", Value::string("Incoming lines")),
        ("side", Value::string("emitted")),
    ]))
}

pub(crate) fn form_source() -> SpecSource<NoExternalEffect> {
    let fields = config_path("fields");
    published_effect_free_source(
        actor_label("Form"),
        Description::from_static("A form you fill in and submit; its fields arrive as one event."),
        PortRule::new(PortSet::empty(), Box::new([])),
        create_config_schema([(
            fields.clone(),
            ConfigSlot::try_new(
                ConfigSpace::try_new(
                    Shape::Array(Box::new(Shape::Any)),
                    ConfigConstraint::CanonicalTypeExpr,
                )
                .expect("form fields preserve the canonical type-expression domain"),
                Required::Mandatory,
                None,
            )
            .expect("mandatory form fields have no default to mismatch")
            .with_text(
                "Fields",
                "The fields a person fills in, each with its type; a committed form carries exactly these fields.",
            ),
        )]),
    )
    .with_boundary(BoundaryPortRule::new(
        circular_protocol::boundary_port::BoundaryPortDirection::Inlet,
        Side::Outlet,
        TypeRule::FromConfigType(fields),
        port_label("Commit"),
    ))
    .with_view_config(view_defaults([("heading", Value::string("Input form"))]))
}
