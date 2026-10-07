use crate::actor_registry::ProductFactoryError;
use crate::config::ConfigRejection;
use circular_core::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FailureDetail {
    code: &'static str,
    slot: Option<&'static str>,
}

impl FailureDetail {
    #[must_use]
    pub const fn new(code: &'static str, slot: Option<&'static str>) -> Self {
        Self { code, slot }
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        self.code
    }

    #[must_use]
    pub const fn slot(self) -> Option<&'static str> {
        self.slot
    }

    #[must_use]
    pub fn to_value(self) -> Value {
        Value::object([
            ("code", Value::string(self.code)),
            ("slot", self.slot.map_or(Value::Null, Value::string)),
        ])
        .expect("code and slot are distinct fields")
    }
}

const fn at(code: &'static str, slot: &'static str) -> FailureDetail {
    FailureDetail::new(code, Some(slot))
}

const fn whole(code: &'static str) -> FailureDetail {
    FailureDetail::new(code, None)
}

impl ProductFactoryError {
    #[must_use]
    pub fn detail(&self) -> FailureDetail {
        use crate::agent_actor::AgentFactoryError as Agent;
        use crate::alert_actor::AlertFactoryError as Alert;
        use crate::debounce::DebounceFactoryError as Debounce;
        use crate::ema_state::EmaFactoryError as Ema;
        use crate::file_actor::FileFactoryError as File;
        use crate::fixture_panic::FixturePanicConfigError as FixturePanic;
        use crate::json_actor::JsonFactoryError as Json;
        use crate::keyed_reduce::KeyedReduceConfigError as KeyedReduce;
        use crate::listener::ListenerConfigError as Listener;
        use crate::notify_actor::NotifyFactoryError as Notify;
        use crate::peer_actor::PeerFactoryError as Peer;
        use crate::replicator_actor::{
            ReplicatorFactoryError as Replicator, ReplicatorRoutingError as Routing,
        };
        use crate::request_actor::RequestFactoryError as Request;
        use crate::route_config::RouteConfigError as Route;
        use crate::timer_actor::TimerFactoryError as Timer;
        use crate::tool_executor_actor::ToolExecutorFactoryError as ToolExecutor;
        use crate::windowed_reduce::WindowedReduceFactoryError as WindowedReduce;
        match self {
            Self::EmptyConfig(crate::EmptyConfigFactoryError::NonEmptyConfig) => {
                whole("empty_config.non_empty_config")
            }
            Self::Route(error) => match error {
                Route::NotAnObject => whole("route.not_an_object"),
                Route::MissingAt => at("route.missing_at", "at"),
                Route::At(_) => at("route.at", "at"),
                Route::MissingCases => at("route.missing_cases", "cases"),
                Route::CasesNotAnObject => at("route.cases_not_an_object", "cases"),
                Route::Cases(_) => at("route.cases", "cases"),
            },
            Self::Ema(error) => match error {
                Ema::InvalidConfig => whole("ema.invalid_config"),
                Ema::Config(rejection) => match rejection {
                    ConfigRejection::NotObject(_) => whole("ema.invalid_config"),
                    ConfigRejection::Unknown(_) => whole("ema.unexpected_config_key"),
                    ConfigRejection::Missing("half_life") => {
                        at("ema.missing_half_life", "half_life")
                    }
                    ConfigRejection::Slot {
                        slot: "half_life", ..
                    } => at("ema.half_life", "half_life"),
                    ConfigRejection::Missing(_) | ConfigRejection::Slot { .. } => {
                        whole("ema.invalid_config")
                    }
                },
            },
            Self::Debounce(error) => match error {
                Debounce::InvalidConfig => whole("debounce.invalid_config"),
                Debounce::Config(rejection) => match rejection {
                    ConfigRejection::NotObject(_) => whole("debounce.invalid_config"),
                    ConfigRejection::Unknown(_) => whole("debounce.unexpected_config_key"),
                    ConfigRejection::Missing("quiet_window") => {
                        at("debounce.missing_quiet_window", "quiet_window")
                    }
                    ConfigRejection::Slot {
                        slot: "quiet_window",
                        ..
                    } => at("debounce.quiet_window", "quiet_window"),
                    ConfigRejection::Missing(slot) | ConfigRejection::Slot { slot, .. } => {
                        FailureDetail::new("debounce.invalid_config", Some(slot))
                    }
                },
            },
            Self::KeyedReduce(error) => match error {
                KeyedReduce::NotAnObject => whole("keyed_reduce.not_an_object"),
                KeyedReduce::MissingAt => at("keyed_reduce.missing_at", "at"),
                KeyedReduce::At(_) => at("keyed_reduce.at", "at"),
                KeyedReduce::MissingValue => at("keyed_reduce.missing_value", "value"),
                KeyedReduce::Value(_) => at("keyed_reduce.value", "value"),
            },
            Self::WindowedReduce(error) => match error {
                WindowedReduce::InvalidConfig => whole("windowed_reduce.invalid_config"),
                WindowedReduce::Config(rejection) => match rejection {
                    ConfigRejection::NotObject(_) => whole("windowed_reduce.invalid_config"),
                    ConfigRejection::Unknown(_) => whole("windowed_reduce.unexpected_config_key"),
                    ConfigRejection::Missing(slot) => {
                        FailureDetail::new("windowed_reduce.missing_slot", Some(slot))
                    }
                    ConfigRejection::Slot { slot, .. } => {
                        FailureDetail::new("windowed_reduce.interval", Some(slot))
                    }
                },
                WindowedReduce::Reduce(_) => at("windowed_reduce.reduce", "reduce"),
                WindowedReduce::ReduceOutputShapeUnresolved => {
                    at("windowed_reduce.reduce_output_shape_unresolved", "reduce")
                }
                WindowedReduce::ReduceKindSplit(_) => {
                    at("windowed_reduce.reduce_kind_split", "reduce")
                }
            },
            Self::Join(_) => at("join.rejected", "at"),
            Self::Agent(error) => match error {
                Agent::InvalidConfig => whole("agent.invalid_config"),
                Agent::Config(rejection) => match rejection {
                    ConfigRejection::NotObject(_) | ConfigRejection::Unknown(_) => {
                        whole("agent.invalid_config")
                    }
                    ConfigRejection::Missing(slot) | ConfigRejection::Slot { slot, .. } => {
                        match *slot {
                            "harness" => at("agent.invalid_harness", "harness"),
                            "queue_capacity" => {
                                at("agent.invalid_queue_capacity", "queue_capacity")
                            }
                            crate::agent_actor::AGENT_RESULT_FIELD => at(
                                "agent.invalid_result_kind",
                                crate::agent_actor::AGENT_RESULT_FIELD,
                            ),
                            other => FailureDetail::new("agent.invalid_config", Some(other)),
                        }
                    }
                },
                Agent::InvalidQueueCapacity => at("agent.invalid_queue_capacity", "queue_capacity"),
                Agent::InvalidTools => at("agent.invalid_tools", "tools"),
                Agent::MissingGrant => whole("agent.missing_grant"),
            },
            Self::ToolExecutor(error) => match error {
                ToolExecutor::InvalidConfig | ToolExecutor::Config(_) => {
                    whole("tool_executor.invalid_config")
                }
                ToolExecutor::DuplicateTool => at("tool_executor.duplicate_tool", "tools"),
                ToolExecutor::InvalidTool => at("tool_executor.invalid_tool", "tools"),
                ToolExecutor::InvalidTemplate => at("tool_executor.invalid_template", "tools"),
            },
            Self::File(error) => match error {
                File::InvalidConfig => whole("file.invalid_config"),
                File::Config(rejection) => match rejection {
                    ConfigRejection::NotObject(_) => whole("file.invalid_config"),
                    ConfigRejection::Unknown(_) => whole("file.unexpected_key"),
                    ConfigRejection::Missing("path") => at("file.missing_path", "path"),
                    ConfigRejection::Missing(slot) | ConfigRejection::Slot { slot, .. } => {
                        FailureDetail::new("file.invalid_config", Some(slot))
                    }
                },
                File::InvalidPathType => at("file.invalid_path_type", "path"),
                File::InvalidPath(_) => at("file.invalid_path", "path"),
            },
            Self::Request(error) => match error {
                Request::InvalidConfig => whole("request.invalid_config"),
                Request::Config(rejection) => match rejection {
                    ConfigRejection::NotObject(_) => whole("request.invalid_config"),
                    ConfigRejection::Unknown(_) => whole("request.unexpected_key"),
                    ConfigRejection::Missing("method") => at("request.missing_method", "method"),
                    ConfigRejection::Missing("url") => at("request.missing_url", "url"),
                    ConfigRejection::Missing(slot) | ConfigRejection::Slot { slot, .. } => {
                        FailureDetail::new("request.invalid_config", Some(slot))
                    }
                },
                Request::InvalidUrlType => at("request.invalid_url_type", "url"),
                Request::InvalidUrl(_) => at("request.invalid_url", "url"),
                Request::InvalidHeadersType => at("request.invalid_headers_type", "headers"),
                Request::InvalidHeaderEntry { .. } => at("request.invalid_header_entry", "headers"),
                Request::UnexpectedHeaderKey { .. } => {
                    at("request.unexpected_header_key", "headers")
                }
                Request::MissingHeaderName { .. } => at("request.missing_header_name", "headers"),
                Request::InvalidHeaderNameType { .. } => {
                    at("request.invalid_header_name_type", "headers")
                }
                Request::MissingHeaderValueOrSecret { .. } => {
                    at("request.missing_header_value_or_secret", "headers")
                }
                Request::ConflictingHeaderValueAndSecret { .. } => {
                    at("request.conflicting_header_value_and_secret", "headers")
                }
                Request::InvalidHeaderValueType { .. } => {
                    at("request.invalid_header_value_type", "headers")
                }
                Request::InvalidHeaderSecretType { .. } => {
                    at("request.invalid_header_secret_type", "headers")
                }
                Request::InvalidHeader { .. } => at("request.invalid_header", "headers"),
            },
            Self::Peer(error) => match error {
                Peer::InvalidConfig => whole("peer.invalid_config"),
                Peer::Config(rejection) => match rejection {
                    ConfigRejection::NotObject(_) => whole("peer.invalid_config"),
                    ConfigRejection::Unknown(_) => whole("peer.unexpected_key"),
                    ConfigRejection::Missing(slot) | ConfigRejection::Slot { slot, .. } => {
                        match *slot {
                            "adapter" => at("peer.invalid_adapter", "adapter"),
                            "realm" => at("peer.invalid_realm", "realm"),
                            "name" => at("peer.invalid_name", "name"),
                            "inbound_policy" => at("peer.invalid_inbound_policy", "inbound_policy"),
                            "inbox_capacity" => at("peer.invalid_inbox_capacity", "inbox_capacity"),
                            other => FailureDetail::new("peer.invalid_config", Some(other)),
                        }
                    }
                },
                Peer::InvalidAdapter => at("peer.invalid_adapter", "adapter"),
                Peer::InvalidRealm => at("peer.invalid_realm", "realm"),
                Peer::InvalidName => at("peer.invalid_name", "name"),
                Peer::InvalidInboundPolicy => at("peer.invalid_inbound_policy", "inbound_policy"),
                Peer::InvalidInboxCapacity => at("peer.invalid_inbox_capacity", "inbox_capacity"),
            },
            Self::Notify(Notify::InvalidConfig | Notify::Config(_)) => {
                whole("notify.invalid_config")
            }
            Self::Timer(error) => match error {
                Timer::InvalidConfig => whole("timer.invalid_config"),
                Timer::Config(rejection) => match rejection {
                    ConfigRejection::NotObject(_) => whole("timer.invalid_config"),
                    ConfigRejection::Unknown(_) => whole("timer.unexpected_config_key"),
                    ConfigRejection::Missing("every") => at("timer.missing_every", "every"),
                    ConfigRejection::Slot { slot: "every", .. } => at("timer.every", "every"),
                    ConfigRejection::Missing(slot) | ConfigRejection::Slot { slot, .. } => {
                        FailureDetail::new("timer.invalid_config", Some(slot))
                    }
                },
            },
            Self::Json(error) => match error {
                Json::InvalidConfig => whole("json.invalid_config"),
                Json::Config(rejection) => match rejection {
                    ConfigRejection::NotObject(_) => whole("json.invalid_config"),
                    ConfigRejection::Unknown(_) => whole("json.unexpected_config_key"),
                    ConfigRejection::Missing("initial") => at("json.missing_initial", "initial"),
                    ConfigRejection::Missing(slot) | ConfigRejection::Slot { slot, .. } => {
                        FailureDetail::new("json.invalid_config", Some(slot))
                    }
                },
            },
            Self::Listener(error) => match error {
                Listener::NotAnObject => whole("listener.not_an_object"),
                Listener::MissingSource => at("listener.missing_source", "source"),
                Listener::SourceIsNotAnObject => at("listener.source_is_not_an_object", "source"),
                Listener::MissingKind => at("listener.missing_kind", "source"),
                Listener::UnknownKind => at("listener.unknown_kind", "source"),
                Listener::ArmIsNotAnObject => at("listener.arm_is_not_an_object", "source"),
                Listener::MissingGlob => at("listener.missing_glob", "source"),
                Listener::EmptyGlob => at("listener.empty_glob", "source"),
                Listener::PollIsNotAnInteger => at("listener.poll_is_not_an_integer", "source"),
                Listener::PollNegative => at("listener.poll_negative", "source"),
                Listener::PollZero => at("listener.poll_zero", "source"),
            },
            Self::Alert(error) => match error {
                Alert::InvalidConfig => whole("alert.invalid_config"),
                Alert::Predicate(_) => at("alert.predicate", "predicate"),
                Alert::Config(rejection) => match rejection {
                    ConfigRejection::NotObject(_) => whole("alert.invalid_config"),
                    ConfigRejection::Unknown(_) => whole("alert.unexpected_config_key"),
                    ConfigRejection::Missing(slot) => {
                        FailureDetail::new("alert.missing_delay", Some(slot))
                    }
                    ConfigRejection::Slot { slot, .. } => {
                        FailureDetail::new("alert.delay", Some(slot))
                    }
                },
                Alert::PredicateKindSplit(_) => at("alert.predicate_kind_split", "predicate"),
            },
            Self::Assemble(_) => whole("assemble.rejected"),
            Self::Replicator(error) => match error {
                Replicator::Routing(routing) => match routing {
                    Routing::NotAnObject => whole("replicator.routing"),
                    Routing::MissingAt | Routing::At(_) => at("replicator.routing", "at"),
                    Routing::MissingTtl
                    | Routing::TtlNotAnInteger
                    | Routing::TtlNegative { .. }
                    | Routing::TtlZero => at("replicator.routing", "ttl"),
                },
                Replicator::Capacity(
                    ConfigRejection::Missing(_) | ConfigRejection::NotObject(_),
                ) => at("replicator.missing_capacity", "capacity"),
                Replicator::Capacity(_) => at("replicator.capacity", "capacity"),
                Replicator::NoAuthority => whole("replicator.no_authority"),
            },
            Self::FoldMismatch(_) => whole("fold_mismatch.rejected"),
            Self::FixturePanic(error) => match error {
                FixturePanic::NotAnObject => whole("fixture_panic.not_an_object"),
                FixturePanic::MissingPanicAfter => {
                    at("fixture_panic.missing_panic_after", "panic_after")
                }
                FixturePanic::NotAnInteger => at("fixture_panic.not_an_integer", "panic_after"),
                FixturePanic::Negative => at("fixture_panic.negative", "panic_after"),
                FixturePanic::Zero => at("fixture_panic.zero", "panic_after"),
            },
            Self::Otlp(_) => whole("otlp.rejected"),
        }
    }
}

