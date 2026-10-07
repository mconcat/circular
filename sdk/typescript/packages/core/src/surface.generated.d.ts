export type PublicActorSpelling = "route" | "pipeline_actor" | "debounce" | "alert" | "tap" | "input" | "output" | "replicator" | "agent" | "counter" | "ema" | "windowed_reduce" | "timer" | "tool_executor" | "notify" | "peer" | "listener" | "keyed_reduce" | "request" | "file" | "json" | "otlp" | "match" | "assemble" | "join" | "form" | "project_input" | "project_output";
export type SourceOnlyActorSpelling = "input" | "project_input";
export type FlowActorSpelling = "debounce" | "alert" | "tap" | "agent" | "counter" | "ema" | "timer" | "tool_executor" | "peer" | "listener" | "keyed_reduce" | "request" | "file" | "json" | "match" | "assemble" | "join";
export type TerminalActorSpelling = "output" | "replicator" | "notify" | "project_output";
export type MultiOutletActorSpelling = "otlp";
export type DynamicPortActorSpelling = "route" | "windowed_reduce" | "form";
