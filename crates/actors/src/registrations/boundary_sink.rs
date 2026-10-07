//! Boundary-sink registration sources.

use super::support::*;

pub(crate) fn request_source() -> SpecSource<ExternalEffect> {
    let response = Shape::Object {
        fields: FieldMap::try_new(vec![
            (Name::from_static("status"), Shape::Base(BaseShape::UInt)),
            (Name::from_static("body"), Shape::Base(BaseShape::Bytes)),
            (Name::from_static("truncated"), Shape::Base(BaseShape::Bool)),
            (Name::from_static("retry_after_seconds"), Shape::Any),
        ])
        .expect("request response fields differ"),
        open: false,
    };
    let ports = PortSet::try_new(
        vec![required_primary_inlet(
            "event",
            Flow::Stream(Shape::Any),
            "Event",
        )],
        vec![primary_outlet(
            "response",
            Flow::Stream(response),
            "Response",
        )],
    )
    .expect("request input and output have different directions");
    let header = Shape::Object {
        fields: FieldMap::try_new(vec![(
            Name::from_static("name"),
            Shape::Base(BaseShape::String),
        )])
        .expect("request header name is one field"),
        open: true,
    };
    published_external_source(
        actor_label("Request"),
        Description::from_static("Send one HTTP request for each event."),
        PortRule::new(ports, Box::new([])),
        RequireRules::external(
            RequireRule::new(Capability::HttpFetch, Condition::Always),
            [],
        ),
        ExternalEffect::new(Durability::Direct, failed_stand_ins(EffectCtor::Http, [])),
        create_config_schema([
            capabilities_slot(
                PolicyDemand::EveryDeclared,
                "The grant for the HTTP effect.",
            ),
            retry_slot("The waits between retries of a transient failure, in milliseconds."),
            described(
                mandatory_typed(&crate::request_actor::METHOD),
                "Method",
                "The HTTP method.",
            ),
            described(
                (
                    config_path("url"),
                    mandatory_slot(Shape::Base(BaseShape::String), None),
                ),
                "URL",
                "The request URL.",
            ),
            described(
                (
                    config_path("headers"),
                    optional_slot(
                        Shape::Array(Box::new(header)),
                        Value::Array(Vec::new()),
                        None,
                    ),
                ),
                "Headers",
                "The request headers.",
            ),
        ]),
    )
    .with_view_config(view_defaults([
        ("heading", Value::string("Response")),
        (
            "fields",
            view_defaults([
                ("status", Value::array([Value::string("status")])),
                ("value", Value::array([Value::string("body")])),
            ]),
        ),
        ("side", Value::string("emitted")),
        ("spark", Value::string("none")),
    ]))
}

pub(crate) fn tool_executor_source() -> SpecSource<ExternalEffect> {
    published_external_source(
        actor_label("Tool Executor"),
        Description::from_static(
            "Run the file and program tools you allow, one call at a time.",
        ),
        PortRule::new(
            single_request_result_ports("call", "Call", "result", "Result"),
            Box::new([]),
        ),
        RequireRules::external(
            RequireRule::new(Capability::FsRead, Condition::Always),
            [
                RequireRule::new(Capability::FsWrite, Condition::Always),
                RequireRule::new(Capability::ProcessSpawn, Condition::Always),
            ],
        ),
        ExternalEffect::new(
            Durability::Direct,
            failed_stand_ins(
                EffectCtor::FileRead,
                [EffectCtor::FileWrite, EffectCtor::Spawn],
            ),
        ),
        create_config_schema([
            capabilities_slot(
                PolicyDemand::AuthoredTools,
                "One grant per effect the tools use.",
            ),
            described(
                (
                    config_path("tools"),
                    mandatory_slot(open_object_shape(), None),
                ),
                "Tools",
                "The tools this actor may run, each named with the one effect it performs; for example, read_log with effect file_read and a path.",
            ),
        ]),
    )
    .with_view_config(view_defaults([("heading", Value::string("Tool activity"))]))
}

pub(crate) fn file_source() -> SpecSource<ExternalEffect> {
    let read = InletSpec::try_new(
        authored_port_id("read"),
        Flow::Stream(Shape::Any),
        Arity::Many,
        Presence::Optional,
        false,
        port_label("Read"),
    )
    .expect("optional bang has no default");
    let fixed = PortSet::try_new(
        vec![
            required_primary_inlet("write", Flow::Stream(Shape::Any), "Write"),
            read,
        ],
        vec![
            primary_outlet(
                "content",
                Flow::Stream(Shape::Base(BaseShape::Bytes)),
                "Content",
            ),
            outlet(
                "written",
                Flow::Stream(Shape::Base(BaseShape::Int)),
                false,
                "Written",
            ),
        ],
    )
    .expect("file port names are unique");
    published_external_source(
        actor_label("File"),
        Description::from_static("Read or replace the whole contents of one file."),
        PortRule::new(fixed, Box::new([])),
        RequireRules::external(
            RequireRule::new(Capability::FsRead, Condition::Always),
            [RequireRule::new(Capability::FsWrite, Condition::Always)],
        ),
        ExternalEffect::new(
            Durability::Direct,
            failed_stand_ins(EffectCtor::FileRead, [EffectCtor::FileWrite]),
        ),
        create_config_schema([
            capabilities_slot(
                PolicyDemand::EveryDeclared,
                "The grants for reading and for writing.",
            ),
            described(
                (
                    config_path("path"),
                    mandatory_slot(Shape::Base(BaseShape::String), None),
                ),
                "Path",
                "The file this actor is bound to, as an absolute path.",
            ),
        ]),
    )
    .with_view_config(view_defaults([("heading", Value::string("Content"))]))
}

pub(crate) fn notify_source() -> SpecSource<ExternalEffect> {
    published_external_source(
        actor_label("Notify"),
        Description::from_static("Send you notifications, no more often than an interval you set."),
        PortRule::new(
            single_sink_inlet("notification", "Notification"),
            Box::new([]),
        ),
        RequireRules::external(
            RequireRule::new(Capability::UserNotify, Condition::Always),
            [],
        ),
        ExternalEffect::new(
            Durability::Durable,
            failed_stand_ins(EffectCtor::Notify, []),
        ),
        suppressed_create_config_schema(
            [
                capabilities_slot(
                    PolicyDemand::EveryDeclared,
                    "The grant for the notification effect.",
                ),
                retry_slot("The waits between retries of a delivery, in milliseconds."),
                described(
                    (
                        config_path("channel"),
                        mandatory_slot(Shape::Base(BaseShape::String), None),
                    ),
                    "Channel",
                    "The logical channel name.",
                ),
                starting(
                    timed(
                        mandatory_typed(&crate::notify_actor::MINIMUM_INTERVAL),
                        "Minimum interval",
                        "The shortest gap between two submissions.",
                    ),
                    Value::Int(60_000),
                ),
                described(
                    mandatory_typed(&crate::notify_actor::DURING_INTERVAL),
                    "During the interval",
                    "What to do with a notification that arrives during the interval.",
                ),
            ],
            decision_suppression("cooling"),
        ),
    )
    .with_view_config(view_defaults([(
        "heading",
        Value::string("Latest notification"),
    )]))
}

pub(crate) fn cli_source() -> SpecSource<ExternalEffect> {
    let fixed = PortSet::try_new(
        vec![required_primary_inlet(
            "request",
            Flow::Stream(Shape::Any),
            "Request",
        )],
        vec![
            primary_outlet(
                "stdout",
                Flow::Stream(Shape::Base(BaseShape::String)),
                "Stdout",
            ),
            outlet(
                "stderr",
                Flow::Stream(Shape::Base(BaseShape::String)),
                false,
                "Stderr",
            ),
            outlet(
                "exit",
                Flow::Stream(Shape::Base(BaseShape::Int)),
                false,
                "Exit",
            ),
        ],
    )
    .expect("cli output role names are distinct");

    published_external_source(
        actor_label("CLI"),
        Description::from_static(
            "Run configured processes for events or feed a configured coprocess.",
        ),
        PortRule::new(fixed, Box::new([])),
        RequireRules::external(
            RequireRule::new(Capability::ProcessSpawn, Condition::Always),
            [],
        ),
        ExternalEffect::new(Durability::Direct, failed_stand_ins(EffectCtor::Spawn, [])),
        blocked_config_schema(
            [
                capabilities_slot(
                    PolicyDemand::EveryDeclared,
                    "The grant for spawning processes.",
                ),
                (
                    config_path("mode"),
                    mandatory_slot_in(
                        constrained_space(
                            Shape::Base(BaseShape::String),
                            ConfigConstraint::ClosedTags(
                                ClosedTags::try_from_members(["coprocess", "request"])
                                    .expect("CLI modes are distinct"),
                            ),
                        ),
                        None,
                    ),
                ),
                (
                    config_path("program"),
                    mandatory_slot(Shape::Base(BaseShape::String), None),
                ),
                (
                    config_path("arguments"),
                    mandatory_slot(Shape::Array(Box::new(Shape::Base(BaseShape::String))), None),
                ),
            ],
            "cli requires mode-tagged coprocess/request argument and isolation contracts; only request spawning is boarded",
        ),
    )
}

pub(crate) fn agent_source() -> SpecSource<ExternalEffect> {
    let fixed = PortSet::try_new(
        vec![
            required_inlet("turn", Flow::Stream(Shape::Any), true, "Turn"),
            required_inlet(
                "tool_result",
                Flow::Stream(open_object_shape()),
                false,
                "Tool result",
            ),
        ],
        vec![
            outlet("record", Flow::Stream(open_object_shape()), false, "Record"),
            outlet(
                "tool_request",
                Flow::Stream(open_object_shape()),
                false,
                "Tool request",
            ),
            outlet(
                "result",
                Flow::Stream(Shape::Var(Name::from_static("Result"))),
                true,
                "Result",
            ),
        ],
    )
    .expect("agent role names are distinct in each direction");
    let result_choice = FlowChoice::new(
        Side::Outlet,
        authored_port_id("result"),
        crate::agent_actor::RESULT.path(),
        crate::agent_actor::agent_result_kind_arms()
            .into_iter()
            .map(|(tag, shape)| (tag, Flow::Stream(shape)))
            .collect::<Vec<_>>(),
    );
    let requires = RequireRules::external(
        RequireRule::new(Capability::AgentHarness, Condition::Always),
        [],
    );
    let effect = ExternalEffect::new(
        Durability::Direct,
        StandIns::one(
            EffectCtor::AgentInvoke,
            StandIn::Failed(EffectFailure::EndpointGone),
        ),
    );

    published_external_source(
        actor_label("Agent"),
        Description::from_static(
            "Hand each turn to an external agent program, one turn at a time.",
        ),
        PortRule::new(fixed, Box::new([])).with_flow_choices(Box::new([result_choice])),
        requires,
        effect,
        create_config_schema([
            approval_slot("Whether the harness step needs approval."),
            described(
                mandatory_typed(&crate::agent_actor::RESULT),
                "Result type",
                "Selects the type of the result outlet.",
            ),
            described(
                mandatory_typed(&crate::agent_actor::HARNESS),
                "Harness",
                "Which external harness to invoke.",
            ),
            starting(
                described(
                    mandatory_typed(&crate::agent_actor::QUEUE_CAPACITY),
                    "Queue capacity",
                    "The most turns that can wait inside the agent while it works on one.",
                ),
                Value::Int(8),
            ),
            described(
                (
                    config_path("tools"),
                    optional_slot(Shape::Any, Value::Array(Vec::new()), None),
                ),
                "Tools",
                "The tool policy.",
            ),
        ]),
    )
    .with_view_config(view_defaults([("heading", Value::string("Current work"))]))
}
