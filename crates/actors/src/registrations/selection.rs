//! Selection-family registration sources.

use super::support::*;

pub(crate) fn route_source() -> SpecSource<NoExternalEffect> {
    let event = authored_port_id("event");
    let unmatched = authored_port_id("unmatched");
    let polymorphic = Flow::Stream(Shape::Var(Name::from_static("T")));
    let fixed = PortSet::try_new(
        vec![
            InletSpec::try_new(
                event,
                polymorphic.clone(),
                Arity::Many,
                Presence::Required,
                true,
                port_label("Event"),
            )
            .expect("required route inlet has no default to mismatch"),
        ],
        vec![OutletSpec::new(
            unmatched,
            polymorphic.clone(),
            Arity::Many,
            false,
            port_label("Unmatched"),
        )],
    )
    .expect("route's two fixed ports are in different direction domains");

    let cases_path = ConfigPath::root().join_key("cases");
    let dynamic = DynamicRule::new(
        cases_path.clone(),
        ExpansionRule::Keys {
            prefix: authored_port_id("route"),
        },
        PortTemplate::Outlet(OutletTemplate::new(
            TypeRule::Fixed(polymorphic),
            Arity::Many,
            LabelRule::literal(port_label("Route case")),
        )),
    );

    let cases_shape = Shape::Object {
        fields: FieldMap::try_new(Vec::new()).expect("empty field map is unique"),
        open: true,
    };
    let config = ConfigSchema::try_from_create_parts(
        [
            (
                ConfigPath::root().join_key("at"),
                ConfigSlot::try_new(
                    ConfigSpace::try_new(
                        Shape::Array(Box::new(Shape::Any)),
                        ConfigConstraint::ExactPayloadPath,
                    )
                    .expect("route selector preserves the exact payload-path domain"),
                    Required::Mandatory,
                    None,
                )
                .expect("mandatory route path has no default to mismatch")
                .with_text(
                    "Compared path",
                    "The exact payload path whose value is compared.",
                ),
            ),
            (
                cases_path.clone(),
                ConfigSlot::try_new(
                    ConfigSpace::unconstrained(cases_shape),
                    Required::Mandatory,
                    None,
                )
                .expect("mandatory route cases have no default to mismatch")
                .with_text(
                    "Cases",
                    "Each case pairs a name with a value; an event whose value matches leaves on that case's outlet.",
                ),
            ),
        ],
        [CreateInputRelation::UniqueObjectValues {
            at: cases_path.clone(),
        }],
        None,
    )
    .expect("route's slots, case uniqueness, and snippet-free admission are complete");

    published_effect_free_source(
        actor_label("Route"),
        Description::from_static("Send each event to the case its value matches, or to unmatched."),
        PortRule::new(fixed, vec![dynamic].into_boxed_slice()),
        config,
    )
    .with_view_config(view_defaults([
        ("heading", Value::string("Destinations")),
        ("rows", Value::string("outlets")),
    ]))
}

pub(crate) fn filter_source() -> SpecSource<NoExternalEffect> {
    published_effect_free_source(
        actor_label("Filter"),
        Description::from_static("Pass events whose predicate evaluates true."),
        event_passthrough_rule(Shape::Var(Name::from_static("T"))),
        preprocess_create_config_schema(
            PreprocessKind::Filter,
            Some(decision_suppression("filtered_out")),
        ),
    )
}

pub(crate) fn dedup_source() -> SpecSource<NoExternalEffect> {
    let at = config_path("at");
    published_effect_free_source(
        actor_label("Dedup"),
        Description::from_static("Pass the first event for a key within a logical-time window."),
        event_passthrough_rule(Shape::Var(Name::from_static("T"))),
        suppressed_create_config_schema(
            [
                (
                    at.clone(),
                    optional_slot_in(
                        ConfigSpace::try_new(
                            Shape::Array(Box::new(Shape::Any)),
                            ConfigConstraint::ExactPayloadPath,
                        )
                        .expect("dedup selector preserves the exact payload-path domain"),
                        Value::Array(Vec::new()),
                        None,
                    ),
                ),
                (
                    config_path("window"),
                    mandatory_slot_in(interval_space(IntervalDomain::Milliseconds), None),
                ),
            ],
            SuppressDecl::try_from_entries([(
                Name::from_static("duplicate"),
                SuppressRule::Equal(CompareTarget::FromConfig(at)),
            )])
            .expect("dedup declares one unique suppression reason"),
        ),
    )
}

pub(crate) fn match_source() -> SpecSource<NoExternalEffect> {
    let flow = Flow::Stream(Shape::Var(Name::from_static("T")));
    let ports = PortSet::try_new(
        vec![required_primary_inlet("event", flow.clone(), "Event")],
        vec![
            primary_outlet("ok", flow, "Ok"),
            OutletSpec::new(
                authored_port_id("err"),
                Flow::Stream(crate::match_actor::reason_shape()),
                Arity::Many,
                false,
                port_label("Err"),
            ),
        ],
    )
    .expect("match has one inlet and two distinct outlets");
    published_effect_free_source(
        actor_label("Match"),
        Description::from_static("Split results into successes and failures."),
        PortRule::new(ports, Box::new([])),
        ConfigSchema::empty(),
    )
    .with_view_config(view_defaults([
        ("heading", Value::string("Outcome")),
        ("rows", Value::string("outlets")),
    ]))
}
