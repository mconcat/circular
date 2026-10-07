//! Transform-family registration sources.

use super::support::*;

pub(crate) fn bang_source() -> SpecSource<NoExternalEffect> {
    let fixed = PortSet::try_new(
        vec![required_primary_inlet(
            "event",
            Flow::Stream(Shape::Any),
            "Event",
        )],
        vec![primary_outlet(
            "pulse",
            Flow::Stream(Shape::Base(BaseShape::Null)),
            "Pulse",
        )],
    )
    .expect("bang has one inlet and one outlet");

    published_effect_free_source(
        actor_label("Bang"),
        Description::from_static("Replace each input payload with the canonical null pulse."),
        PortRule::new(fixed, Box::new([])),
        ConfigSchema::empty(),
    )
}

pub(crate) fn map_source() -> SpecSource<NoExternalEffect> {
    let transform = config_path(crate::map_config::TRANSFORM);
    let fixed = PortSet::try_new(
        vec![required_primary_inlet(
            "event",
            polymorphic_event_flow(),
            "Event",
        )],
        Vec::new(),
    )
    .expect("map has one fixed inlet and no fixed outlet");
    let dynamic = DynamicRule::new(
        transform.clone(),
        ExpansionRule::Presence {
            id: authored_port_id("event"),
        },
        PortTemplate::Outlet(OutletTemplate::new(
            TypeRule::FromSnippet {
                at: transform.clone(),
                inputs: vec![port_reference("event")].into_boxed_slice(),
                flow: FlowCtor::Stream,
            },
            Arity::Many,
            LabelRule::literal(port_label("Event")),
        )),
    );

    published_effect_free_source(
        actor_label("Map"),
        Description::from_static("Map each event payload with an authored transform expression."),
        PortRule::new(fixed, vec![dynamic].into_boxed_slice()),
        preprocess_create_config_schema(PreprocessKind::Map, None),
    )
}

pub(crate) fn parse_source() -> SpecSource<NoExternalEffect> {
    published_effect_free_source(
        actor_label("Parse"),
        Description::from_static("Decode a configured text field and merge parsed attributes."),
        event_passthrough_rule(open_object_shape()),
        preprocess_create_config_schema(PreprocessKind::Parse, None),
    )
}

pub(crate) fn tap_source() -> SpecSource<NoExternalEffect> {
    published_effect_free_source(
        actor_label("Tap"),
        Description::from_static("Pass events through unchanged, so you can watch them."),
        event_passthrough_rule(Shape::Var(Name::from_static("T"))),
        ConfigSchema::empty(),
    )
    .with_view_config(view_defaults([
        ("heading", Value::string("Record stream")),
        ("side", Value::string("arrivals")),
    ]))
}
