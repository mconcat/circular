//! The daemon owns the OTLP receiver and mounts its recorded arrivals.
use super::*;

pub(crate) fn otlp_source() -> SpecSource<NoExternalEffect> {
    let fixed = PortSet::try_new(
        Vec::new(),
        vec![
            outlet("logs", Flow::Stream(open_object_shape()), false, "Logs"),
            outlet(
                "metrics",
                Flow::Stream(open_object_shape()),
                false,
                "Metrics",
            ),
        ],
    )
    .expect("OTLP has two distinct named outlets");
    published_effect_free_source(
        actor_label("OTLP"),
        Description::from_static(
            "Receive OpenTelemetry logs and metrics sent over HTTP to an address on this computer.",
        ),
        PortRule::new(fixed, Box::new([])),
        create_config_schema([described(
            (
                config_path("listen"),
                mandatory_slot(Shape::Base(BaseShape::String), None),
            ),
            "Listen address",
            "The loopback address and port to receive telemetry on, in numeric form such as 127.0.0.1:4318.",
        )]),
    )
    .with_view_config(view_defaults([
        ("heading", Value::string("Telemetry")),
        ("side", Value::string("emitted")),
    ]))
}
