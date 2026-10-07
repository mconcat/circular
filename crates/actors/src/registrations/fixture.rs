//! Fixture-local registration sources and their bounded assumption manifests.

use super::support::*;

const INPUT_ASSUMPTIONS: &[FixtureAssumption] = &[
    FixtureAssumption::new(
        "registry discriminator",
        ActorType::FixtureInput.as_str(),
        "separates the engine source fixture from the published input boundary actor",
    ),
    FixtureAssumption::new(
        "ports",
        "out, Stream(Number), Many, primary",
        "the engine fixture fixes one i64 source output; remaining display choices are fixture-local",
    ),
];

pub(crate) static INPUT_MANIFEST: FixtureManifest = FixtureManifest::new(
    "crates/engine/src/fixture.rs synthetic input boundary",
    INPUT_ASSUMPTIONS,
);

const MAP_ASSUMPTIONS: &[FixtureAssumption] = &[
    FixtureAssumption::new(
        "display and lifecycle",
        "Fixture map; English description; Live",
        "the existing fixture defines behavior and a machine name, not SpecSource display values",
    ),
    FixtureAssumption::new(
        "registry discriminator",
        ActorType::FixtureMap.as_str(),
        "separates the engine transform fixture from the published product map registration",
    ),
    FixtureAssumption::new(
        "ports",
        "in/out, Stream(Number), Many, in Required, both primary",
        "engine fixes only the in/out names and i64 payload; remaining port declaration is fixture-local",
    ),
    FixtureAssumption::new(
        "config schema",
        "empty",
        "engine fixture config is independent of the product map transform snippet",
    ),
];

pub(crate) static MAP_MANIFEST: FixtureManifest =
    FixtureManifest::new("crates/engine/src/fixture.rs map hook", MAP_ASSUMPTIONS);

const FILTER_ASSUMPTIONS: &[FixtureAssumption] = &[
    FixtureAssumption::new(
        "display and lifecycle",
        "Fixture filter; English description; Live",
        "the existing fixture defines behavior and a machine name, not SpecSource display values",
    ),
    FixtureAssumption::new(
        "registry discriminator",
        ActorType::FixtureFilter.as_str(),
        "separates the engine selection fixture from the published product filter registration",
    ),
    FixtureAssumption::new(
        "ports",
        "in/out, Stream(Number), Many, in Required, both primary",
        "engine fixes only the in/out names and i64 payload; remaining port declaration is fixture-local",
    ),
    FixtureAssumption::new(
        "config schema",
        "empty",
        "engine fixture config is independent of the product filter predicate snippet",
    ),
];

pub(crate) static FILTER_MANIFEST: FixtureManifest = FixtureManifest::new(
    "crates/engine/src/fixture.rs filter hook",
    FILTER_ASSUMPTIONS,
);

const TAP_ASSUMPTIONS: &[FixtureAssumption] = &[
    FixtureAssumption::new(
        "registry discriminator",
        ActorType::FixtureTap.as_str(),
        "separates the engine effect fixture from the published product tap registration",
    ),
    FixtureAssumption::new(
        "ports",
        "in/out, Stream(Number), Many, in Required, both primary",
        "the fixture is an i64 pass-through between filter and its terminal",
    ),
    FixtureAssumption::new(
        "effect declaration",
        "FsRead and FsWrite; Direct; successful empty/read-zero stand-ins",
        "the fixture executes both effect constructors; durability and dry-run payloads do not affect its input-derived live path",
    ),
];

pub(crate) static TAP_MANIFEST: FixtureManifest = FixtureManifest::new(
    "crates/engine/src/fixture.rs::drive_pipeline effect fixture",
    TAP_ASSUMPTIONS,
);

const PROJECT_OUTPUT_ASSUMPTIONS: &[FixtureAssumption] = &[
    FixtureAssumption::new(
        "registry discriminator",
        ActorType::FixtureProjectOutput.as_str(),
        "keeps the retired project_output fixture spelling out of the published output registration",
    ),
    FixtureAssumption::new(
        "ports",
        "in, Stream(Number), Many, Required, primary",
        "the fixture terminal consumes one i64 stream and exposes no output",
    ),
];

pub(crate) static PROJECT_OUTPUT_MANIFEST: FixtureManifest = FixtureManifest::new(
    "crates/engine/src/fixture.rs synthetic terminal",
    PROJECT_OUTPUT_ASSUMPTIONS,
);

const EDITABLE_COUNTER_ASSUMPTIONS: &[FixtureAssumption] = &[
    FixtureAssumption::new(
        "ports",
        "out, Stream(Number), Many, primary",
        "the edited fixture only authors the main-to-aux edge from this actor",
    ),
    FixtureAssumption::new(
        "config schema",
        "fixture_transition omitted from product schema",
        "the opaque transition byte is a fixture driver control, not a product actor config",
    ),
];

pub(crate) static EDITABLE_COUNTER_MANIFEST: FixtureManifest = FixtureManifest::new(
    "crates/engine/src/edit_fixture.rs editable main actor",
    EDITABLE_COUNTER_ASSUMPTIONS,
);

const EDITABLE_AUX_ASSUMPTIONS: &[FixtureAssumption] = &[
    FixtureAssumption::new(
        "ports",
        "in, Stream(Number), Many, Required, primary",
        "the edited fixture only authors the main-to-aux edge into this actor",
    ),
    FixtureAssumption::new(
        "config schema",
        "fixture_transition omitted from product schema",
        "the opaque transition byte is a fixture driver control, not a product actor config",
    ),
    FixtureAssumption::new(
        "config schema",
        "unit_ratio: ClosedUnitInterval optional 0.5",
        "carries a constraint kind no published registration carries, so the daemon's create-input projection branch stays exercised",
    ),
    FixtureAssumption::new(
        "config schema",
        "finite_value: FiniteNumber optional 0.0",
        "carries a constraint kind no published registration carries, so the daemon's create-input projection branch stays exercised",
    ),
];

pub(crate) static EDITABLE_AUX_MANIFEST: FixtureManifest = FixtureManifest::new(
    "crates/engine/src/edit_fixture.rs editable auxiliary actor",
    EDITABLE_AUX_ASSUMPTIONS,
);

const EDITABLE_SCOPE_PROBE_ASSUMPTIONS: &[FixtureAssumption] = &[
    FixtureAssumption::new(
        "ports",
        "empty",
        "the nested lifecycle fixture authors no edge touching these actors",
    ),
    FixtureAssumption::new(
        "config schema",
        "fixture_transition omitted from product schema",
        "the opaque transition byte is a fixture driver control, not a product actor config",
    ),
];

pub(crate) static EDITABLE_SCOPE_PROBE_MANIFEST: FixtureManifest = FixtureManifest::new(
    "crates/engine/src/edit_fixture.rs nested lifecycle actors",
    EDITABLE_SCOPE_PROBE_ASSUMPTIONS,
);

pub(crate) fn fixture_map_source() -> SpecSource<NoExternalEffect> {
    fixture_transform_source("Fixture map", "Adds one to the fixture i64 payload.")
}

pub(crate) fn fixture_filter_source() -> SpecSource<NoExternalEffect> {
    fixture_transform_source(
        "Fixture filter",
        "Passes even fixture i64 payloads and filters odd payloads.",
    )
}

fn fixture_transform_source(
    label: &'static str,
    description: &'static str,
) -> SpecSource<NoExternalEffect> {
    let number = Flow::Stream(Shape::Base(BaseShape::Float));
    let fixed = PortSet::try_new(
        vec![
            InletSpec::try_new(
                authored_port_id("in"),
                number.clone(),
                Arity::Many,
                Presence::Required,
                true,
                port_label("In"),
            )
            .expect("required fixture inlet has no default to mismatch"),
        ],
        vec![OutletSpec::new(
            authored_port_id("out"),
            number,
            Arity::Many,
            true,
            port_label("Out"),
        )],
    )
    .expect("fixture in/out names are unique in each direction");

    fixture_effect_free_source(label, description, fixed)
}

pub(crate) fn fixture_input_source() -> SpecSource<NoExternalEffect> {
    fixture_source_with_ports(
        "Fixture input",
        "Produces the synthetic fixture i64 input stream.",
        PortSet::try_new(
            Vec::new(),
            vec![OutletSpec::new(
                authored_port_id("out"),
                fixture_number_flow(),
                Arity::Many,
                true,
                port_label("Out"),
            )],
        )
        .expect("fixture input has one output"),
    )
}

pub(crate) fn fixture_tap_source() -> SpecSource<ExternalEffect> {
    let fixed = fixture_in_out_ports();
    let rules = RequireRules::external(
        RequireRule::new(Capability::FsRead, Condition::Always),
        [RequireRule::new(Capability::FsWrite, Condition::Always)],
    );
    let stand_ins = StandIns::try_new(
        (
            EffectCtor::FileRead,
            StandIn::Succeeded(OutcomePayload::FileBytes(Box::new([]))),
        ),
        [(
            EffectCtor::FileWrite,
            StandIn::Succeeded(OutcomePayload::WrittenLength(0)),
        )],
    )
    .expect("fixture tap declares each effect constructor once");
    fixture_external_source(
        "Fixture tap",
        "Passes fixture values and exercises file effects.",
        fixed,
        rules,
        ExternalEffect::new(Durability::Direct, stand_ins),
    )
}

pub(crate) fn fixture_project_output_source() -> SpecSource<NoExternalEffect> {
    let fixed = PortSet::try_new(vec![fixture_number_inlet()], Vec::new())
        .expect("fixture terminal has one input");
    fixture_source_with_ports(
        "Fixture project output",
        "Consumes the synthetic fixture output stream.",
        fixed,
    )
}

pub(crate) fn editable_counter_source() -> SpecSource<NoExternalEffect> {
    let fixed = PortSet::try_new(Vec::new(), vec![fixture_number_outlet()])
        .expect("editable counter has one output");
    fixture_source_with_ports(
        "Editable counter",
        "Synthetic stateful actor used by edit record/replay.",
        fixed,
    )
}

pub(crate) fn editable_aux_source() -> SpecSource<NoExternalEffect> {
    let fixed = PortSet::try_new(vec![fixture_number_inlet()], Vec::new())
        .expect("editable auxiliary has one input");
    fixture_effect_free_source_with_config(
        "Editable auxiliary",
        "Synthetic auxiliary actor used by edit record/replay.",
        fixed,
        create_config_schema([
            (
                config_path("unit_ratio"),
                optional_slot_in(
                    constrained_space(
                        Shape::Base(BaseShape::Float),
                        ConfigConstraint::ClosedUnitInterval,
                    ),
                    Value::float(0.5),
                    None,
                ),
            ),
            (
                config_path("finite_value"),
                optional_slot_in(
                    constrained_space(
                        Shape::Base(BaseShape::Float),
                        ConfigConstraint::FiniteNumber,
                    ),
                    Value::float(0.0),
                    None,
                ),
            ),
        ]),
    )
}

pub(crate) fn editable_scope_probe_source() -> SpecSource<NoExternalEffect> {
    fixture_source_with_ports(
        "Editable scope probe",
        "Synthetic nested actor used by lifecycle record/replay.",
        PortSet::empty(),
    )
}

const FIXTURE_PANIC_ASSUMPTIONS: &[FixtureAssumption] = &[
    FixtureAssumption::new(
        "registry discriminator",
        ActorType::FixturePanic.as_str(),
        "the one deliberate failure is its own name so a gate can point at it; no published element panics",
    ),
    FixtureAssumption::new(
        "ports",
        "in/out, Stream(Number), Many, in Required, both primary",
        "the surviving arrivals pass through unchanged; the shape is the shared fixture number surface",
    ),
    FixtureAssumption::new(
        "config schema",
        "panic_after, Int, mandatory",
        "the authored ordinal decides *when* the hook panics — an actor that always panics stands a graph that observes nothing",
    ),
];

pub(crate) static FIXTURE_PANIC_MANIFEST: FixtureManifest = FixtureManifest::new(
    "crates/actors/src/fixture_panic.rs arrival-counting hook",
    FIXTURE_PANIC_ASSUMPTIONS,
);

pub(crate) fn fixture_panic_source() -> SpecSource<NoExternalEffect> {
    fixture_effect_free_source_with_config(
        "Fixture panic",
        "Synthetic actor whose hook panics at an authored arrival ordinal.",
        fixture_in_out_ports(),
        config_schema([(
            config_path(crate::fixture_panic::FixturePanicConfig::PANIC_AFTER),
            mandatory_slot(Shape::Base(BaseShape::Int), None),
        )]),
    )
}

fn fixture_source_with_ports(
    label: &'static str,
    description: &'static str,
    fixed: PortSet,
) -> SpecSource<NoExternalEffect> {
    fixture_effect_free_source(label, description, fixed)
}

fn fixture_in_out_ports() -> PortSet {
    PortSet::try_new(vec![fixture_number_inlet()], vec![fixture_number_outlet()])
        .expect("fixture in/out names are unique in each direction")
}

fn fixture_number_inlet() -> InletSpec {
    InletSpec::try_new(
        authored_port_id("in"),
        fixture_number_flow(),
        Arity::Many,
        Presence::Required,
        true,
        port_label("In"),
    )
    .expect("required fixture inlet has no default to mismatch")
}

fn fixture_number_outlet() -> OutletSpec {
    OutletSpec::new(
        authored_port_id("out"),
        fixture_number_flow(),
        Arity::Many,
        true,
        port_label("Out"),
    )
}

fn fixture_number_flow() -> Flow {
    Flow::Stream(Shape::Base(BaseShape::Float))
}
