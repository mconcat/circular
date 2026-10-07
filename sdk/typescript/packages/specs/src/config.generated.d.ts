import type { CircularValue } from "@circular/protocol";

export interface ActorConfigTable {
  /** UniqueObjectValues{at: [["1n","cases"]]} */
  readonly "route": {
    /** ExactPayloadPath */
    readonly "at": ReadonlyArray<CircularValue>;
    readonly "cases": Record<string, CircularValue>;
  };
  readonly "pipeline_actor": null;
  readonly "debounce": {
    /** Interval(milliseconds) */
    readonly "quiet_window": bigint;
  };
  readonly "alert": {
    /** Interval(nonzero_milliseconds) */
    readonly "firing_delay": bigint;
    /** Snippet: predicate; inlets: ["event"] */
    readonly "predicate": string;
    /** Interval(nonzero_milliseconds) */
    readonly "recovery_delay": bigint;
  };
  readonly "tap": null;
  readonly "input": {
    readonly "label": string;
    /** CanonicalBaseStream */
    readonly "shape"?: ReadonlyArray<CircularValue>;
  };
  readonly "output": {
    readonly "label": string;
  };
  readonly "replicator": {
    /** ExactPayloadPath */
    readonly "at": ReadonlyArray<CircularValue>;
    /** IntegerCount{minimum: 1, maximum: null} */
    readonly "capacity": bigint;
    /** Interval(nonzero_milliseconds) */
    readonly "ttl": bigint;
  };
  readonly "agent": {
    /** ClosedTags(["none","required"]) */
    /** Default (wire): "none" */
    readonly "approval"?: "none" | "required";
    /** ClosedTags(["claude","codex","pi"]) */
    readonly "harness": "claude" | "codex" | "pi";
    /** IntegerCount{minimum: 1, maximum: null} */
    readonly "queue_capacity": bigint;
    /** ClosedTags(["bytes","json"]) */
    readonly "result": "bytes" | "json";
    /** Default (wire): [] */
    readonly "tools"?: CircularValue;
  };
  readonly "counter": null;
  /** half_life unit: samples = sample count; wallclock = nonzero milliseconds. Numeric constraints are enforced by daemon admission. */
  readonly "ema": {
    /** Interval(nonzero_milliseconds) */
    readonly "half_life": bigint;
    /** ClosedTags(["samples","wallclock"]) */
    /** Default (wire): "samples" */
    readonly "time_basis"?: "samples" | "wallclock";
  };
  readonly "windowed_reduce": {
    /** Interval(nonzero_milliseconds) */
    readonly "emission_period": bigint;
    /** Snippet: reduce; inlets: ["sample"] */
    readonly "reduce": string;
    readonly "seed": CircularValue;
    /** Interval(milliseconds) */
    readonly "window_length": bigint;
  };
  readonly "timer": {
    /** Interval(nonzero_milliseconds) */
    readonly "every": bigint;
  };
  readonly "tool_executor": {
    /** Default (wire): {} */
    readonly "capabilities"?: {
      readonly "FsRead"?: {
        /** ClosedTags(["none","required"]) */
        readonly "approval": "none" | "required";
        readonly "roots": ReadonlyArray<string>;
      };
      readonly "FsWrite"?: {
        /** ClosedTags(["none","required"]) */
        readonly "approval": "none" | "required";
        readonly "roots": ReadonlyArray<string>;
      };
      readonly "ProcessSpawn"?: {
        /** ClosedTags(["none","required"]) */
        readonly "approval": "none" | "required";
      };
    };
    readonly "tools": Record<string, CircularValue>;
  };
  readonly "notify": {
    readonly "capabilities": {
      readonly "UserNotify": {
        /** ClosedTags(["none","required"]) */
        readonly "approval": "none" | "required";
      };
    };
    readonly "channel": string;
    /** ClosedTags(["latest","queue","suppress"]) */
    readonly "during_interval": "latest" | "queue" | "suppress";
    /** Interval(milliseconds) */
    readonly "minimum_interval": bigint;
    /** IntervalList(nonzero_milliseconds) */
    /** Default (wire): ["1000n","1000n","1000n","5000n","5000n","5000n","15000n","15000n","15000n"] */
    readonly "retry_delays"?: ReadonlyArray<bigint>;
  };
  readonly "peer": {
    /** ClosedTags(["claude","codex","memory"]) */
    readonly "adapter": "claude" | "codex" | "memory";
    readonly "inbound_policy": Record<string, CircularValue>;
    /** IntegerCount{minimum: 1, maximum: null} */
    readonly "inbox_capacity": bigint;
    readonly "name": string;
    readonly "realm": string;
  };
  readonly "listener": {
    readonly "capabilities": {
      readonly "FsRead": {
        /** ClosedTags(["none","required"]) */
        readonly "approval": "none" | "required";
        readonly "roots": ReadonlyArray<string>;
      };
    };
    readonly "source": { readonly "kind": string; readonly "value": { readonly "glob": string; readonly "poll": bigint; }; };
  };
  readonly "keyed_reduce": {
    /** ExactPayloadPath */
    readonly "at": ReadonlyArray<CircularValue>;
    /** ExactPayloadPath */
    readonly "value": ReadonlyArray<CircularValue>;
  };
  readonly "request": {
    readonly "capabilities": {
      readonly "HttpFetch": {
        /** ClosedTags(["none","required"]) */
        readonly "approval": "none" | "required";
      };
    };
    /** Default (wire): [] */
    readonly "headers"?: ReadonlyArray<{ readonly "name": string; } & Record<string, CircularValue>>;
    /** ClosedTags(["get","post"]) */
    readonly "method": "get" | "post";
    /** IntervalList(nonzero_milliseconds) */
    /** Default (wire): ["1000n","1000n","1000n","5000n","5000n","5000n","15000n","15000n","15000n"] */
    readonly "retry_delays"?: ReadonlyArray<bigint>;
    readonly "url": string;
  };
  readonly "file": {
    readonly "capabilities": {
      readonly "FsRead": {
        /** ClosedTags(["none","required"]) */
        readonly "approval": "none" | "required";
        readonly "roots": ReadonlyArray<string>;
      };
      readonly "FsWrite": {
        /** ClosedTags(["none","required"]) */
        readonly "approval": "none" | "required";
        readonly "roots": ReadonlyArray<string>;
      };
    };
    readonly "path": string;
  };
  readonly "json": {
    readonly "initial": CircularValue;
  };
  readonly "otlp": {
    readonly "listen": string;
  };
  readonly "match": null;
  readonly "assemble": {
    /** ExactPayloadPath */
    readonly "at": ReadonlyArray<CircularValue>;
    /** IntegerCount{minimum: 1, maximum: null} */
    readonly "capacity": bigint;
    /** Interval(nonzero_milliseconds) */
    readonly "inactivity_timeout": bigint;
    /** Interval(nonzero_milliseconds) */
    readonly "max_window": bigint;
  };
  readonly "join": {
    /** ExactPayloadPath */
    readonly "at": ReadonlyArray<CircularValue>;
  };
  readonly "form": {
    /** CanonicalTypeExpression */
    readonly "fields": ReadonlyArray<CircularValue>;
  };
}
