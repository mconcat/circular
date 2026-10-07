window.PUBLISHED_ACTORS = [
  {
    "actor_type": "route",
    "label": "Route",
    "description": "Route one input to a configured case or unmatched output.",
    "in_ports": [
      {
        "id": "event",
        "primary": true
      }
    ],
    "out_ports": [
      {
        "id": "unmatched",
        "primary": false
      }
    ],
    "presentation_role": [
      1
    ]
  },
  {
    "actor_type": "pipeline_actor",
    "label": "Pipeline",
    "description": "Open a child scope and project its declared boundary.",
    "in_ports": [],
    "out_ports": [],
    "presentation_role": [
      4,
      "one"
    ]
  },
  {
    "actor_type": "debounce",
    "label": "Debounce",
    "description": "Emit the latest event after a quiet window.",
    "in_ports": [
      {
        "id": "event",
        "primary": true
      }
    ],
    "out_ports": [
      {
        "id": "event",
        "primary": true
      }
    ],
    "presentation_role": [
      2
    ]
  },
  {
    "actor_type": "alert",
    "label": "Alert",
    "description": "Pass every event and emit logical-time alert state transitions.",
    "in_ports": [
      {
        "id": "event",
        "primary": true
      }
    ],
    "out_ports": [
      {
        "id": "event",
        "primary": true
      },
      {
        "id": "transition",
        "primary": false
      }
    ],
    "presentation_role": [
      1
    ]
  },
  {
    "actor_type": "tap",
    "label": "Tap",
    "description": "Pass event payloads unchanged and expose an observation point.",
    "in_ports": [
      {
        "id": "event",
        "primary": true
      }
    ],
    "out_ports": [
      {
        "id": "event",
        "primary": true
      }
    ],
    "presentation_role": [
      2
    ]
  },
  {
    "actor_type": "input",
    "label": "Input",
    "description": "Declare an inlet on the containing pipeline.",
    "in_ports": [],
    "out_ports": [],
    "presentation_role": [
      3,
      "source"
    ]
  },
  {
    "actor_type": "output",
    "label": "Output",
    "description": "Declare an outlet on the containing pipeline.",
    "in_ports": [],
    "out_ports": [],
    "presentation_role": [
      3,
      "sink"
    ]
  },
  {
    "actor_type": "replicator",
    "label": "Replicator",
    "description": "Route events into keyed runtime child cells.",
    "in_ports": [
      {
        "id": "event",
        "primary": true
      }
    ],
    "out_ports": [],
    "presentation_role": [
      4,
      "keyed_many"
    ]
  },
  {
    "actor_type": "agent",
    "label": "Agent",
    "description": "Run one external agent harness step at a time.",
    "in_ports": [
      {
        "id": "turn",
        "primary": true
      },
      {
        "id": "tool_result",
        "primary": false
      }
    ],
    "out_ports": [
      {
        "id": "record",
        "primary": false
      },
      {
        "id": "tool_request",
        "primary": false
      },
      {
        "id": "result",
        "primary": true
      },
      {
        "id": "_error",
        "primary": false
      }
    ],
    "presentation_role": [
      1
    ]
  },
  {
    "actor_type": "counter",
    "label": "Counter",
    "description": "Count accepted input events exactly.",
    "in_ports": [
      {
        "id": "event",
        "primary": true
      }
    ],
    "out_ports": [
      {
        "id": "count",
        "primary": true
      }
    ],
    "presentation_role": [
      1
    ]
  },
  {
    "actor_type": "ema",
    "label": "EMA",
    "description": "Compute an exponential moving average by samples or recorded milliseconds.",
    "in_ports": [
      {
        "id": "sample",
        "primary": true
      }
    ],
    "out_ports": [
      {
        "id": "ema",
        "primary": true
      }
    ],
    "presentation_role": [
      2
    ]
  },
  {
    "actor_type": "windowed_reduce",
    "label": "Windowed Reduce",
    "description": "Periodically reduce numeric samples in a logical-time window.",
    "in_ports": [
      {
        "id": "sample",
        "primary": true
      }
    ],
    "out_ports": [],
    "presentation_role": [
      2
    ]
  },
  {
    "actor_type": "timer",
    "label": "Timer",
    "description": "Rearm relative delays and emit one sequenced tick per fire.",
    "in_ports": [
      {
        "id": "bang",
        "primary": true
      }
    ],
    "out_ports": [
      {
        "id": "tick",
        "primary": true
      },
      {
        "id": "_error",
        "primary": false
      }
    ],
    "presentation_role": [
      3,
      "source"
    ]
  },
  {
    "actor_type": "tool_executor",
    "label": "Tool Executor",
    "description": "Execute allowlisted filesystem and process tool effects one call at a time.",
    "in_ports": [
      {
        "id": "call",
        "primary": true
      }
    ],
    "out_ports": [
      {
        "id": "result",
        "primary": true
      },
      {
        "id": "_error",
        "primary": false
      }
    ],
    "presentation_role": [
      1
    ]
  },
  {
    "actor_type": "notify",
    "label": "Notify",
    "description": "Deliver user notifications no faster than a configured interval.",
    "in_ports": [
      {
        "id": "notification",
        "primary": true
      }
    ],
    "out_ports": [
      {
        "id": "_error",
        "primary": false
      }
    ],
    "presentation_role": [
      3,
      "sink"
    ]
  },
  {
    "actor_type": "peer",
    "label": "Peer",
    "description": "Exchange durable asynchronous messages with external agent sessions and threads.",
    "in_ports": [
      {
        "id": "send",
        "primary": true
      },
      {
        "id": "refresh",
        "primary": false
      }
    ],
    "out_ports": [
      {
        "id": "message",
        "primary": true
      },
      {
        "id": "peers",
        "primary": false
      },
      {
        "id": "delivery",
        "primary": false
      },
      {
        "id": "binding",
        "primary": false
      },
      {
        "id": "_error",
        "primary": false
      }
    ],
    "presentation_role": [
      1
    ]
  },
  {
    "actor_type": "listener",
    "label": "Listener",
    "description": "Listen to an external origin and replay its history on a control pulse.",
    "in_ports": [
      {
        "id": "control",
        "primary": true
      }
    ],
    "out_ports": [
      {
        "id": "line",
        "primary": true
      },
      {
        "id": "_error",
        "primary": false
      }
    ],
    "presentation_role": [
      3,
      "source"
    ]
  },
  {
    "actor_type": "keyed_reduce",
    "label": "Keyed Reduce",
    "description": "Accumulate a value per key and project the table, its sum, and its cardinality.",
    "in_ports": [
      {
        "id": "event",
        "primary": true
      },
      {
        "id": "remove",
        "primary": false
      }
    ],
    "out_ports": [
      {
        "id": "map",
        "primary": true
      },
      {
        "id": "total",
        "primary": false
      },
      {
        "id": "count",
        "primary": false
      }
    ],
    "presentation_role": [
      1
    ]
  },
  {
    "actor_type": "request",
    "label": "Request",
    "description": "Project each event into one configured HTTP request.",
    "in_ports": [
      {
        "id": "event",
        "primary": true
      }
    ],
    "out_ports": [
      {
        "id": "response",
        "primary": true
      },
      {
        "id": "_error",
        "primary": false
      }
    ],
    "presentation_role": [
      3,
      "sink"
    ]
  },
  {
    "actor_type": "file",
    "label": "File",
    "description": "Read or replace the complete contents of one real file.",
    "in_ports": [
      {
        "id": "write",
        "primary": true
      },
      {
        "id": "read",
        "primary": false
      }
    ],
    "out_ports": [
      {
        "id": "content",
        "primary": true
      },
      {
        "id": "written",
        "primary": false
      },
      {
        "id": "_error",
        "primary": false
      }
    ],
    "presentation_role": [
      1
    ]
  },
  {
    "actor_type": "json",
    "label": "Json",
    "description": "Emit an authored value at start and on demand.",
    "in_ports": [
      {
        "id": "set",
        "primary": false
      },
      {
        "id": "bang",
        "primary": true
      }
    ],
    "out_ports": [
      {
        "id": "value",
        "primary": true
      }
    ],
    "presentation_role": [
      3,
      "source"
    ]
  },
  {
    "actor_type": "otlp",
    "label": "OTLP",
    "description": "Receive OTLP/HTTP JSON logs and metrics on a numeric loopback address.",
    "in_ports": [],
    "out_ports": [
      {
        "id": "logs",
        "primary": false
      },
      {
        "id": "metrics",
        "primary": false
      }
    ],
    "presentation_role": [
      3,
      "source"
    ]
  },
  {
    "actor_type": "match",
    "label": "Match",
    "description": "Branch on the envelope result, emitting the payload or failure reason.",
    "in_ports": [
      {
        "id": "event",
        "primary": true
      }
    ],
    "out_ports": [
      {
        "id": "ok",
        "primary": true
      },
      {
        "id": "err",
        "primary": false
      }
    ],
    "presentation_role": [
      1
    ]
  },
  {
    "actor_type": "assemble",
    "label": "Assemble",
    "description": "Group records by key and emit one object when the window closes.",
    "in_ports": [
      {
        "id": "event",
        "primary": true
      }
    ],
    "out_ports": [
      {
        "id": "event",
        "primary": true
      },
      {
        "id": "_error",
        "primary": false
      }
    ],
    "presentation_role": [
      1
    ]
  },
  {
    "actor_type": "join",
    "label": "Join",
    "description": "Join each event with the latest reference state for its key.",
    "in_ports": [
      {
        "id": "event",
        "primary": true
      },
      {
        "id": "state",
        "primary": false
      },
      {
        "id": "remove",
        "primary": false
      }
    ],
    "out_ports": [
      {
        "id": "event",
        "primary": true
      }
    ],
    "presentation_role": [
      1
    ]
  },
  {
    "actor_type": "form",
    "label": "Form",
    "description": "Commit a typed draft into the graph.",
    "in_ports": [],
    "out_ports": [],
    "presentation_role": [
      2
    ]
  }
];
