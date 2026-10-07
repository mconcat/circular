window.STUDY_HISTORY = {
  duration: 120,
  start: 14 * 3600 + 30 * 60,
  stages: [
    {
      at: 0,
      omit: ["notebook"],
      nodes: {
        research: {
          activity: "Listening",
          preview: { text: "Waiting for a question.", bytes: 0 },
        },
        tools: { activity: "Idle", preview: { bytes: 0 } },
      },
    },
    {
      at: 18,
      label: "Question received",
      actor: "research",
      omit: ["notebook"],
      nodes: {
        research: {
          activity: "Reading context",
          preview: {
            text: "Finding the context for your question.",
            bytes: 412,
          },
        },
        tools: { activity: "Reading file", preview: { bytes: 412 } },
      },
    },
    {
      at: 42,
      label: "Context loaded",
      actor: "tools",
      omit: ["notebook"],
      nodes: {
        research: {
          activity: "Comparing sources",
          preview: {
            text: "Comparing the brief with the local sources.",
            bytes: 1460,
          },
        },
        tools: { activity: "Reading file", preview: { bytes: 1460 } },
      },
    },
    {
      at: 78,
      label: "Notes attached",
      actor: "notebook",
      nodes: {
        research: {
          activity: "Writing notes",
          preview: {
            text: "A place for the findings is now connected.",
            bytes: 1460,
          },
        },
        tools: { activity: "Writing file", preview: { bytes: 312 } },
        notebook: {
          activity: "Writing",
          preview: {
            heading: "Working notes",
            lines: [
              "Keep the context close.",
              "Comparing the available sources…",
            ],
            bytes: 312,
          },
        },
      },
    },
    {
      at: 99,
      label: "Note written",
      actor: "notebook",
      nodes: {
        research: {
          activity: "Awaiting tool",
          preview: {
            text: "Two observations written. Checking the next step.",
            bytes: 1840,
          },
        },
        tools: { activity: "Idle", preview: { bytes: 1840 } },
        notebook: {
          activity: "Idle",
          preview: {
            lines: ["Keep the context close.", "Make sources traceable."],
            bytes: 1840,
          },
        },
      },
    },
  ],
};

window.StudyFixture = (() => {
  let started = performance.now(),
    through = 0,
    lastCapture = "",
    lastCaptureSecond = -1;
  const phases = new Map();
  const fixture = {
    get head() {
      return STUDY_HISTORY.duration + (performance.now() - started) / 1000;
    },
    get through() {
      return through;
    },
    rate(edge, t) {
      const activity =
        t >= 120
          ? 1
          : t < 18
            ? 0.09
            : 0.24 +
              1.6 * Math.exp(-(((t - 43) / 15) ** 2)) +
              0.9 * Math.exp(-(((t - 82) / 7) ** 2)) +
              1.4 * Math.exp(-(((t - 106) / 8) ** 2));
      return (
        (edge.demoRate ?? 2) *
        activity *
        (0.91 + 0.09 * Math.sin(t * 1.2 + edge.from.length))
      );
    },
    advance(archive, t, pausedScopes = new Set()) {
      const end = Math.floor(t * 4) / 4;
      for (let at = through + 0.25; at <= end; at += 0.25) {
        const scopes = archive.frame(at).scopes;
        for (const [scope, g] of Object.entries(scopes)) {
          if (pausedScopes.has(scope)) continue;
          for (const e of g.edges) {
            let phase = (phases.get(e.id) || 0) + fixture.rate(e, at) * 0.25;
            const count = Math.floor(phase);
            phases.set(e.id, phase - count);
            if (count)
              archive.append({
                at,
                count,
                actor: e.to,
                event: "actor_arrival",
                edge: e.id,
                detail: `${e.in} ← ${e.from}.${e.out}${count > 1 ? ` · ${count} arrivals` : ""}`,
                value: g.nodes.find((n) => n.id === e.to)?.preview?.text || {
                  inlet: e.in,
                  source: `${e.from}.${e.out}`,
                  kind: g.nodes.find((n) => n.id === e.to)?.preview?.kinds?.[
                    (archive.indices.get(e.to) || 0) %
                      (g.nodes.find((n) => n.id === e.to)?.preview?.kinds
                        ?.length || 1)
                  ],
                },
              });
          }
        }
      }
      through = Math.max(through, end);
    },
    capture(archive, t, scopes) {
      const content = JSON.stringify(scopes);
      if (content === lastCapture) return;
      lastCapture = content;
      archive.frames.push({ at: t, scopes: structuredClone(scopes) });
    },
    start(archive, machine, scopes) {
      started = performance.now();
      lastCapture = JSON.stringify(scopes);
      fixture.advance(archive, STUDY_HISTORY.duration);
      machine.refresh();
      machine.interval = setInterval(() => {
        if (document.hidden) return;
        if (machine.options.observing?.() !== false)
          fixture.advance(archive, machine.head, machine.options.pausedScopes());
        else through = machine.head;
        if (lastCaptureSecond !== Math.floor(machine.head)) {
          lastCaptureSecond = Math.floor(machine.head);
          fixture.capture(archive, machine.head, machine.options.scopes());
          if (machine.mode === "live") machine.options.onLiveTick?.();
        }
        machine.refresh();
        window.StudyApp?.field?.wake();
        if (machine.mode === "replay" && machine.options.visible())
          machine.options.onView(machine.position, machine.mode, false);
      }, 200);
    },
  };
  return fixture;
})();
