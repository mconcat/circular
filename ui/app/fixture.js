window.STUDY = {
  root: {
    name: "Research desk",
    nodes: [
      {
        id: "prompt",
        title: "A place to begin",
        type: "input",
        icon: "MessageSquare",
        x: 40,
        y: 88,
        width: 218,
        health: "running",
        activity: "Idle",
        view: "prompt",
        in: [],
        out: [["event", "Any", 148]],
        config: { label: "Prompt" },
      },
      {
        id: "research",
        title: "Research agent",
        type: "agent",
        icon: "Sparkles",
        x: 366,
        y: 101,
        width: 246,
        health: "running",
        activity: "Awaiting tool",
        view: "agent",
        in: [
          ["turn", "Any", 115],
          ["tool_result", "Object", 150],
        ],
        out: [
          ["tool_request", "Object", 115],
          ["result", "Bytes", 150],
          ["record", "Object", 185],
        ],
        config: {
          harness: "local-research",
          result: "bytes",
          queue_capacity: 16,
          tools: [{ name: "read_file" }, { name: "write_file" }],
          approval: "none",
        },
      },
      {
        id: "tools",
        title: "Workspace tools",
        type: "tool_executor",
        icon: "Terminal",
        x: 739,
        y: 101,
        width: 231,
        health: "running",
        activity: "Reading file",
        view: "tools",
        in: [["call", "Object", 115]],
        out: [["result", "Object", 150]],
        config: {
          tools: {
            read_file: { effect: "read" },
            write_file: { effect: "write" },
          },
        },
      },
      {
        id: "sources",
        title: "Sources",
        type: "pipeline_actor",
        icon: "Layers",
        x: 40,
        y: 406,
        width: 218,
        health: "running",
        activity: "3 actors alive",
        view: "scope",
        scope: "sources",
        in: [["query", "String", 98]],
        out: [["context", "Bytes", 98]],
        config: null,
      },
      {
        id: "transcript",
        title: "Conversation",
        type: "tap",
        icon: "AudioLines",
        x: 366,
        y: 477,
        width: 246,
        health: "running",
        activity: "Listening",
        view: "transcript",
        in: [["event", "Object", 75]],
        out: [["event", "Object", 75]],
        config: null,
      },
      {
        id: "notebook",
        title: "Research notes",
        type: "file",
        icon: "FileText",
        x: 739,
        y: 401,
        width: 231,
        health: "running",
        activity: "Idle",
        view: "notebook",
        in: [
          ["write", "Any", 90],
          ["read", "Any", 125],
        ],
        out: [
          ["content", "Bytes", 90],
          ["written", "Int", 125],
        ],
        config: { path: "./notes/research.md" },
      },
    ],
    edges: [
      {
        id: "prompt-agent",
        from: "prompt",
        out: "event",
        to: "research",
        in: "turn",
        label: "",
      },
      {
        id: "agent-tools",
        from: "research",
        out: "tool_request",
        to: "tools",
        in: "call",
        label: "",
      },
      {
        id: "tools-agent",
        from: "tools",
        out: "result",
        to: "research",
        in: "tool_result",
        label: "",
      },
      {
        id: "sources-agent",
        from: "sources",
        out: "context",
        to: "research",
        in: "turn",
        label: "map",
        map: "event",
        labelX: 296,
        labelY: 349,
      },
      {
        id: "agent-notebook",
        from: "research",
        out: "result",
        to: "notebook",
        in: "write",
        label: "",
      },
      {
        id: "agent-transcript",
        from: "research",
        out: "record",
        to: "transcript",
        in: "event",
        label: "",
      },
    ],
  },
  sources: {
    name: "Sources",
    nodes: [
      {
        id: "query",
        title: "Query",
        type: "input",
        icon: "ArrowDownLeft",
        x: 92,
        y: 242,
        width: 216,
        health: "running",
        activity: "Idle",
        view: "boundary",
        in: [],
        boundaryPort: "query",
        out: [["query", "Any", 90]],
        config: { label: "Query" },
      },
      {
        id: "source-file",
        title: "Project context",
        type: "file",
        icon: "FileText",
        x: 412,
        y: 213,
        width: 231,
        health: "running",
        activity: "Idle",
        view: "notebook",
        in: [
          ["write", "Any", 90],
          ["read", "Any", 125],
        ],
        out: [
          ["content", "Bytes", 90],
          ["written", "Int", 125],
        ],
        config: { path: "./sources/context.md" },
      },
      {
        id: "context",
        title: "Context",
        type: "output",
        icon: "ArrowUpRight",
        x: 756,
        y: 242,
        width: 216,
        health: "running",
        activity: "Idle",
        view: "boundary",
        boundaryPort: "context",
        in: [["context", "Any", 90]],
        out: [],
        config: { label: "Context" },
      },
    ],
    edges: [
      {
        id: "query-file",
        from: "query",
        out: "query",
        to: "source-file",
        in: "read",
        label: "bang",
        labelX: 349,
        labelY: 340,
      },
      {
        id: "file-context",
        from: "source-file",
        out: "content",
        to: "context",
        in: "context",
        label: "",
      },
    ],
  },
  journal: [
    {
      actor: "tools",
      index: 42,
      event: "actor_arrival",
      detail: "call ← research.tool_request",
      time: "14:32:08.241",
      value: { name: "read_file", path: "./sources/context.md" },
    },
    {
      actor: "research",
      index: 108,
      event: "actor_arrival",
      detail: "turn ← prompt.event",
      time: "14:32:08.216",
      value: "What makes a useful research companion?",
    },
    {
      actor: "transcript",
      index: 67,
      event: "actor_arrival",
      detail: "event ← research.record",
      time: "14:32:08.218",
      value: { kind: "message", text: "I’ll start with the project context." },
    },
    {
      actor: "notebook",
      index: 12,
      event: "actor_arrival",
      detail: "write ← research.result",
      time: "14:31:42.083",
      value: "Three observations, with sources attached.",
    },
    {
      actor: "research",
      index: 107,
      event: "actor_arrival",
      detail: "tool_result ← tools.result",
      time: "14:31:40.706",
      value: { name: "read_file", result: "Project context loaded." },
    },
    {
      actor: "sources",
      index: 8,
      event: "actor_health_transition",
      detail: "running",
      time: "14:30:00.000",
      value: { state: "running" },
    },
  ],
};

const previewObservations = {
  research: { text: "Reading the project context.", bytes: 2184 },
  tools: {
    rows: [
      ["read_file", "active"],
      ["write_file", "idle"],
    ],
    bytes: 2184,
  },
  transcript: {
    rows: [
      ["#67", "tool_call"],
      ["#66", "message"],
      ["#65", "arrival"],
    ],
    kinds: ["message", "tool_call", "tool_result"],
  },
  notebook: {
    heading: "Research companion",
    lines: [
      "Keep the context close.",
      "Make sources traceable.",
      "Leave a useful next step.",
    ],
    bytes: 2184,
  },
  "source-file": {
    heading: "Project context",
    lines: [
      "Fieldnotes / working brief",
      "Local-first research.",
      "Sources stay inspectable.",
    ],
    bytes: 2184,
  },
};
for (const scope of Object.values(STUDY).filter((s) => s.nodes))
  for (const node of scope.nodes) node.preview = previewObservations[node.id];
window.STUDY_MESSAGES = { prompt: "What makes a useful research companion?" };

window.StudyPrototype = {
  install({ P, A, S, C, $, e, copy, wait, show, dialog, banner, refreshApprovalBadge, resetHistory,
    appendRecord, initializeActor, countApprovals, make }) {
    const baseline = copy(A.liveScopes());
    const machine = $(".local-machine > div > span");
    if (machine) {
      machine.lastChild.nodeValue = " Demo connected";
      machine.querySelector(".tiny-dot").classList.add("green");
    }
    function addApprovals() {
      const research = A.allNode("research"), notebook = A.allNode("notebook");
      if (research) research.config.approval = "required";
      if (notebook) notebook.config.capabilities = { FsWrite: { approval: "required", roots: ["./notes"] } };
      P.approvals = [
        {
          id: "approval-write",
          actor: "tools",
          action: "Write the research note",
          target: "./notes/research.md",
          tool: "write_file",
          state: "requested",
          outcome: "accept",
          args: { path: "./notes/research.md", mode: "replace" },
        },
        {
          id: "approval-request",
          actor: "research",
          action: "Continue the agent’s external action",
          target: "Local research harness",
          state: "requested",
          outcome: "fail",
          args: { harness: "local-research" },
        },
        {
          id: "approval-stale",
          actor: "notebook",
          action: "Read the working notebook",
          target: "./notes/research.md",
          state: "requested",
          outcome: "stale",
          args: { path: "./notes/research.md" },
        },
      ];
      countApprovals();
      refreshApprovalBadge();
      A.renderGraph();
      banner("3 actions need your decision. Open Requests to review.");
    }
    const scenarioDescriptions = {
      normal: "Calm research desk",
      idle_failed: "Idle next to a failed actor",
      approvals: "Three requests; accepted, transport failure, stale",
      config_rejected: "Keep a rejected draft and correct it",
      connection_accepted: "Unproven ports accepted after validation",
      connection_rejected: "Unproven ports rejected after validation",
      preprocess_failed: "Lawful wire, failed input preprocessing",
      overload: "Full receiving inlet, reliable vs shedding",
      disconnected: "Last observations, then reconnect",
      restart: "Recorded actor restart",
      instances: "Instance created, then retired",
      history: "Past topology, then return to Live",
      combined: "A pending approval and a local failure",
      empty_projects: "First project experience",
      empty_canvas: "An empty editable canvas",
      empty_outputs: "No output has arrived yet",
      stale_outputs: "Content is old but still readable",
      catalog: "All published types in one exploration scope",
      dense: "Dense wires for interaction checks",
      long: "Long names and content",
    };
    function developerDialog() {
      dialog(
        "Prototype tools",
        `<p>These controls create synthetic observations. They are not runtime operations.</p><div class="developer-grid"><button data-appearance>Typography & palette<small>Compare local font families and light/dark</small></button><button data-sample-selected>Observe sample on selection<small>Use after creating any actor type</small></button><button data-empty-selected>Clear selected observation<small>Keep actor alive and idle</small></button><button data-fail-selected>Fail selected actor<small>Local failure with evidence</small></button>${Object.entries(
          scenarioDescriptions,
        )
          .map(
            ([key, value]) =>
              `<button data-scenario="${key}">${e(value)}<small>${key.replaceAll("_", " ")}</small></button>`,
          )
          .join("")}</div>`,
      );
    }
    function resetScenario() {
      A.timeMachine.live();
      if (A.timeMachine.mode !== "live") A.timeMachine.live();
      for (const k of Object.keys(A.liveScopes())) delete STUDY[k];
      Object.assign(STUDY, copy(baseline));
      S.scope = "root";
      S.selected = "research";
      S.selectedSet = new Set(["research"]);
      S.edge = null;
      S.paused.clear();
      P.approvals = [];
      P.connection = "connected";
      P.surfaceState = "data";
      P.nextConnection = null;
      P.nextApply = null;
      StudyViewer.clear("draft", "submission", "message");
      for (const [id, text] of Object.entries(STUDY_MESSAGES)) StudyViewer.message(id, text);
      $("#connection-strip").hidden = true;
      $("#canvas-notice").hidden = true;
      P.project = "fieldnotes";
      P.projects = [
        {
          id: "fieldnotes",
          name: "Fieldnotes",
          description:
            "A research desk that reads, connects ideas, and keeps a working notebook.",
          scope: "root",
        },
      ];
      refreshApprovalBadge();
    }
    P.scenario = async (name) => {
      if (!Object.hasOwn(scenarioDescriptions, name)) return A.gestureCode("SCENARIO_UNKNOWN");
      resetScenario();
      if ($("#product-dialog").open) $("#product-dialog").close();
      show("canvas");
      const research = STUDY.root.nodes.find((n) => n.id === "research"),
        tools = STUDY.root.nodes.find((n) => n.id === "tools");
      switch (name) {
        case "idle_failed":
          research.activity = "Idle";
          tools.health = "failed";
          tools.activity = "Failed";
          tools.issue = {
            message: "The tool process exited unexpectedly.",
            code: "RestartableActorFailure::Panicked",
            value: { exit: 1 },
          };
          break;
        case "approvals":
          addApprovals();
          break;
        case "combined":
          addApprovals();
          tools.issue = {
            message: "One file read could not be completed.",
            code: "EffectFailure::TransportTerminal",
          };
          break;
        case "config_rejected":
          P.nextApply = "reject";
          StudyViewer.draft("research", {
            shown: { ...research.config, harness: "unavailable-local" },
          });
          S.tab = "configure";
          break;
        case "connection_accepted":
        case "connection_rejected": {
          const tap = make("tap", "validation-tap", 1100, 240);
          STUDY.root.nodes.push(tap);
          P.nextConnection = name === "connection_rejected" ? "reject" : "accept";
          P.connect(
            { node: "research", name: "result" },
            { node: tap.id, name: "event" },
          );
          break;
        }
        case "preprocess_failed": {
          const w = STUDY.root.edges[0];
          w.combinators = [
            {
              id: "failed-parse",
              kind: "parse",
              expression: "JSON",
              cue: "JSON",
              x: 294,
              y: 350,
            },
          ];
          w.issue = {
            message: "The received text is not valid JSON.",
            code: "ProcessingCause::InputOutOfDomain",
            input: "{topic: missing quote}",
          };
          S.edge = w.id;
          S.selected = null;
          research.activity = "Waiting for input";
          break;
        }
        case "overload": {
          const w = STUDY.root.edges.find((w) => w.to === "research");
          w.capacity = 4;
          w.delivery = "Lossless";
          w.pressure = true;
          w.issue = {
            message: "Inlet full · reliable delivery is waiting.",
            code: "CapacityDecision::BlockReliable",
            queued: 4,
          };
          w.demoRate = 60;
          S.edge = w.id;
          S.selected = null;
          research.activity = "Backpressure";
          break;
        }
        case "disconnected":
          P.connection = "disconnected";
          $("#connection-strip").hidden = false;
          break;
        case "restart":
          tools.activity = "Restarted · idle";
          tools.preview.rows = [
            ["read_file", "idle"],
            ["write_file", "idle"],
          ];
          appendRecord(
            "tools",
            "ActorRestarted",
            "Actor restarted; prior failure remains recorded",
            { cause: "observed failure" },
          );
          banner(
            "Workspace tools restarted. The restart is recorded.",
            "accepted",
          );
          break;
        case "instances": {
          const n = make("replicator", "workers", 1040, 200);
          n.instances = [
            {
              name: "Project alpha",
              scope: "instance-alpha",
              state: "active",
              activity: "Idle",
            },
          ];
          STUDY.root.nodes.push(n);
          STUDY[n.scope] = {
            name: "Workers",
            parent: "root",
            nodes: [],
            edges: [],
          };
          initializeActor(n);
          STUDY["instance-alpha"] = {
            name: "Project alpha",
            parent: n.scope,
            nodes: [make("tap", "alpha-tap", 120, 120)],
            edges: [],
          };
          n.preview.disposition = "Project alpha created";
          S.selected = n.id;
          S.selectedSet = new Set([n.id]);
          appendRecord(n.id, "InstanceCreated", "Project alpha is available", {
            key: "alpha",
          });
          wait(4500).then(() => {
            if (!STUDY.root.nodes.includes(n)) return;
            n.instances[0].state = "retired";
            n.preview.disposition = "Project alpha retired";
            appendRecord(n.id, "InstanceRetired", "Project alpha retired", {
              key: "alpha",
            });
            A.renderGraph();
          });
          break;
        }
        case "history":
          A.timeMachine.seek(70);
          break;
        case "empty_projects":
          P.projects = [];
          show("projects");
          break;
        case "empty_canvas":
          STUDY.empty = {
            name: "Untitled program",
            nodes: [],
            edges: [],
            notes: [],
          };
          S.scope = "empty";
          S.selected = null;
          S.selectedSet.clear();
          break;
        case "empty_outputs":
          P.surfaceState = "empty";
          show("outputs");
          break;
        case "stale_outputs":
          P.surfaceState = "stale";
          show("outputs");
          break;
        case "catalog": {
          const nodes = C.items.map((s, i) => {
            const n = make(
              s.type,
              "catalog-" + s.type,
              60 + (i % 5) * 350,
              70 + Math.floor(i / 5) * 390,
            );
            if (n.scope)
              STUDY[n.scope] = {
                name: n.title,
                parent: "catalog",
                nodes: [],
                edges: [],
              };
            initializeActor(n);
            if (n.scope) STUDY[n.scope].parent = "catalog";
            return n;
          });
          STUDY.catalog = {
            name: "Published actor library",
            nodes,
            edges: [],
            notes: [],
          };
          S.scope = "catalog";
          S.selected = nodes[0].id;
          S.selectedSet = new Set([S.selected]);
          break;
        }
        case "dense": {
          const nodes = Array.from({ length: 30 }, (_, i) => {
            const n = make(
              i % 4 === 0 ? "counter" : "tap",
              "dense-" + i,
              50 + (i % 6) * 330,
              70 + Math.floor(i / 6) * 340,
            );
            return n;
          });
          const edges = nodes.slice(0, -1).flatMap((n, i) =>
            [1, 6]
              .filter((d) => nodes[i + d])
              .map((d) => ({
                id: `dense-${i}-${d}`,
                from: n.id,
                out: n.out[0][0],
                to: nodes[i + d].id,
                in: nodes[i + d].in[0][0],
                combinators: [],
                demoRate: 60,
              })),
          );
          STUDY.dense = {
            name: "Dense observation study",
            nodes,
            edges,
            notes: [],
          };
          S.scope = "dense";
          S.selected = null;
          S.selectedSet.clear();
          break;
        }
        case "long":
          research.title =
            "Research agent for a very long, evolving local project with multiple sources";
          research.preview.text =
            "The current question brings together several related sources. ".repeat(
              12,
            );
          break;
      }
      A.renderGraph();
      if (["catalog", "dense", "empty_canvas", "long"].includes(name))
        A.fitCanvas();
      resetHistory();
      P.lastScenario = name;
    };
    function sampleActor(n) {
      const t = n.type;
      n.activity = "Observed input";
      n.health = "running";
      n.preview ??= {};
      const updates = {
        counter: { value: 18, label: "accepted arrivals" },
        ema: { value: 12.4, label: "weighted mean · 18 samples" },
        windowed_reduce: { value: 46, label: "current window" },
        timer: { value: 24 },
        debounce: { value: 3 },
        route: { message: 8, task: 4 },
        match: { ok: 12, err: 1 },
        alert: { firing: true },
        keyed_reduce: {
          rows: [
            ["alpha", 12.4],
            ["beta", 8.2],
          ],
        },
        join: {
          rows: [
            ["alpha", "latest context"],
            ["beta", "latest status"],
          ],
        },
        assemble: {
          rows: [
            ["question-1", "3 parts"],
            ["question-2", "1 part"],
          ],
        },
        request: {
          response: "The source returned a readable response.",
          status: "200 · response",
        },
        file: {
          heading: "Working note",
          lines: [
            "A new observation has arrived.",
            "The local program remains available.",
          ],
          bytes: 184,
        },
        json: { value: n.config.initial },
        notify: { message: "Research note updated" },
        peer: {
          binding: "Bound locally",
          message: "Context received from a peer.",
        },
        listener: { rows: [["context.log", "project context updated"]] },
        otlp: {
          rows: [
            ["service", "local-research"],
            ["latency", "42 ms"],
          ],
        },
        output: { value: "A useful result from the child scope" },
        agent: {
          text: "Reading the project context.",
          result: "Two open questions are ready for review.",
        },
        tool_executor: {
          rows: [
            ["read_file", "complete"],
            ["write_file", "idle"],
          ],
        },
        tap: {
          rows: [
            ["1", "message"],
            ["2", "result"],
          ],
        },
      };
      Object.assign(n.preview, updates[t] || {});
      appendRecord(
        n.id,
        "actor_arrival",
        "Synthetic sample observed",
        n.preview,
      );
      A.renderGraph();
    }
    P.sampleActor = sampleActor;
    P.actions.developer = developerDialog;
    document.addEventListener(
      "click",
      (ev) => {
        const b = ev.target.closest("button,[data-action]");
        if (!b) return;
        if (b.hasAttribute("data-sample-selected")) {
          const n = A.selectedNode();
          if (n) sampleActor(n);
          $("#product-dialog").close();
          return;
        }
        if (b.hasAttribute("data-empty-selected")) {
          const n = A.selectedNode();
          if (n) {
            n.preview = {};
            n.activity = "Idle";
            n.health = "running";
            delete n.issue;
            A.renderGraph();
          }
          $("#product-dialog").close();
          return;
        }
        if (b.hasAttribute("data-fail-selected")) {
          const n = A.selectedNode();
          if (n) {
            n.health = "failed";
            n.activity = "Failed";
            n.issue = {
              message: "The actor failed while handling an arrival.",
              code: "RestartableActorFailure::Panicked",
            };
            A.renderGraph();
          }
          $("#product-dialog").close();
          return;
        }
        if (b.dataset.scenario) {
          P.scenario(b.dataset.scenario);
          return;
        }
      },
      true,
    );
  },
};
