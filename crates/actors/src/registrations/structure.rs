//! Structure-family registration sources.

use super::support::*;

pub(crate) fn pipeline_actor_source() -> SpecSource<NoExternalEffect> {
    published_effect_free_source(
        actor_label("Pipeline"),
        Description::from_static(
            "Hold a group of actors in one card, with its own inputs and outputs.",
        ),
        PortRule::new(PortSet::empty(), Box::new([])),
        ConfigSchema::empty(),
    )
    .with_view_config(view_defaults([("count_label", Value::string("actors"))]))
}

pub(crate) fn input_source() -> SpecSource<NoExternalEffect> {
    let shape = config_path("shape");
    published_effect_free_source(
        actor_label("Input"),
        Description::from_static("Add an input to the pipeline this actor is in."),
        PortRule::new(PortSet::empty(), Box::new([])),
        create_config_schema([
            boundary_label_slot(),
            (
                shape.clone(),
                ConfigSlot::try_new(
                    crate::config::SlotKind::space(&crate::config::BaseStreamTypeExpr),
                    Required::Omittable {
                        absent: Flow::Stream(Shape::Any),
                    },
                    None,
                )
                .expect("omittable shape has no default to mismatch")
                .with_text(
                    "Value type",
                    "The type of the values this boundary injects.",
                ),
            ),
        ]),
    )
    .with_boundary(BoundaryPortRule::new(
        circular_protocol::boundary_port::BoundaryPortDirection::Inlet,
        Side::Outlet,
        TypeRule::FromConfigType(shape),
        port_label("Input"),
    ))
}

pub(crate) fn output_source() -> SpecSource<NoExternalEffect> {
    published_effect_free_source(
        actor_label("Output"),
        Description::from_static("Add an output to the pipeline this actor is in."),
        PortRule::new(PortSet::empty(), Box::new([])),
        create_config_schema([boundary_label_slot()]),
    )
    .with_boundary(BoundaryPortRule::new(
        circular_protocol::boundary_port::BoundaryPortDirection::Outlet,
        Side::Inlet,
        TypeRule::Fixed(Flow::Stream(Shape::Any)),
        port_label("Output"),
    ))
    .with_view_config(view_defaults([
        ("side", Value::string("arrivals")),
        ("spark", Value::string("none")),
    ]))
}

pub(crate) fn replicator_source() -> SpecSource<NoExternalEffect> {
    let fixed = PortSet::try_new(
        vec![required_primary_inlet(
            "event",
            Flow::Stream(Shape::Any),
            "Event",
        )],
        Vec::new(),
    )
    .expect("replicator has one event inlet");

    published_effect_free_source(
        actor_label("Replicator"),
        Description::from_static(
            "Run one copy of a group of actors for each key the events carry.",
        ),
        PortRule::new(fixed, Box::new([])),
        create_config_schema([
            described(
                (
                    config_path("at"),
                    ConfigSlot::try_new(
                        ConfigSpace::try_new(
                            Shape::Array(Box::new(Shape::Any)),
                            ConfigConstraint::ExactPayloadPath,
                        )
                        .expect("replicator selector preserves the exact payload-path domain"),
                        Required::Mandatory,
                        None,
                    )
                    .expect("mandatory replicator path has no default to mismatch"),
                ),
                "Key path",
                "The exact payload path whose value becomes the cell key.",
            ),
            starting(
                timed(
                    (
                        config_path("ttl"),
                        mandatory_slot_in(
                            interval_space(IntervalDomain::NonZeroMilliseconds),
                            None,
                        ),
                    ),
                    "Idle lifetime",
                    "A cell that sees nothing for this long retires.",
                ),
                Value::Int(60_000),
            ),
            starting(
                described(
                    mandatory_typed(&crate::replicator_actor::CAPACITY),
                    "Capacity",
                    "The maximum number of live cells.",
                ),
                Value::Int(16),
            ),
        ]),
    )
    .with_view_config(view_defaults([
        ("heading", Value::string("Dynamic scope")),
        ("count_label", Value::string("current instances")),
    ]))
}

/// The display name `input` and `output` share.
fn boundary_label_slot() -> (ConfigPath, ConfigSlot) {
    described(
        (
            config_path("label"),
            mandatory_slot(Shape::Base(BaseShape::String), None),
        ),
        "Label",
        "The display name of this boundary.",
    )
}
