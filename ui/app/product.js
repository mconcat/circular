(() => {
  "use strict";
  const source = window.StudySource;
  const viewer = window.StudyViewer;
  const A = window.StudyApp,
    S = A?.state,
    C = ProductCatalog,
    $ = (q, r = document) => r.querySelector(q),
    $$ = (q, r = document) => [...r.querySelectorAll(q)],
    e = C.escape,
    ic = A?.icon ?? ((name) => `<svg class="icon" aria-hidden="true" viewBox="0 0 24 24">${window.ICONS[name]
      .map(([tag, attrs]) => `<${tag} ${Object.entries(attrs).map(([k, v]) => `${k}="${e(v)}"`).join(" ")}></${tag}>`).join("")}</svg>`),
    copy = (v) => structuredClone(v),
    wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const P = (window.Product = {
    selection: S?.selectedSet,
    connection: "connected",
    screen: "canvas",
    surfaceState: "data",
    nav: ["root"],
    navAt: 0,
    notes: [],
    nextConnection: null,
    nextApply: null,
  });
  Object.assign(P, source.product);
  if (source.noProject) {
    P.screen = "projects";
    P.actions = source.actions;
    P.projectNotice = (code) => {
      const value = source.codeLabel(code);
      $("[data-project-notice]").innerHTML = notice(value.label, "failed", value.code);
    };
    document.addEventListener("click", (ev) => {
      const b = ev.target.closest("[data-action],[data-open-project],.brand");
      if (!b) return;
      ev.preventDefault();
      $("[data-project-notice]").innerHTML = "";
      if (b.dataset.action) P.actions[b.dataset.action]?.();
      else if (b.dataset.openProject) openProject(b.dataset.openProject);
    });
    renderProjects();
    return;
  }
  P.undo = () => source.perform({ kind: "undo" });
  P.redo = () => source.perform({ kind: "redo" });
  P.refreshEditTools = () => {
    const can = source.editHistory();
    $('[data-action="undo"]')?.toggleAttribute("disabled", !can.undo || A.historical());
    $('[data-action="redo"]')?.toggleAttribute("disabled", !can.redo || A.historical());
  };
  const live = () => !A.historical();
  const currentIds = () => [...S.selectedSet].filter((id) => A.findNode(id));
  function notice(text, kind = "pending", code = "") {
    return `<div class="inline-notice ${kind === "failed" || kind === "rejected" ? "failure" : kind === "accepted" ? "success" : ""}" role="status"${code ? ` data-reason="${e(code)}"` : ""}>${e(text)}</div>`;
  }
  function banner(text, kind = "pending", code = "") {
    const el = $("#canvas-notice"),
      small = $("[data-notice-code]", el);
    $("[data-notice-text]", el).textContent = text;
    small.textContent = "";
    small.hidden = true;
    if (code) el.dataset.reason = code; else delete el.dataset.reason;
    el.dataset.kind = kind;
    el.hidden = false;
  }
  P.banner = banner;
  function unbanner() {
    $("#canvas-notice").hidden = true;
  }
  P.unbanner = unbanner;
  function dialog(title, body) {
    const d = $("#product-dialog");
    $("#product-dialog-title", d).textContent = title;
    const slot = $("[data-dialog-body]", d),
      actions = $("[data-dialog-actions]", d);
    slot.innerHTML = body;
    const row = $(":scope > .dialog-actions", slot);
    actions.replaceChildren(...(row ? row.childNodes : []));
    actions.hidden = !row;
    row?.remove();
    d.onclick = (ev) => {
      if (ev.target !== d) return;
      const box = d.getBoundingClientRect();
      if (ev.clientX < box.left || ev.clientX > box.right ||
          ev.clientY < box.top || ev.clientY > box.bottom) d.close();
    };
    if (!d.open) d.showModal();
    return d;
  }
  const dialogContents = new Map();
  dialog.register = (name, open) => dialogContents.set(name, open);
  dialog.open = (name) => dialogContents.get(name)();
  dialog.register("appearance", window.Appearance.open);
  P.dialog = dialog;
  P.edgeQuiet = (edge) =>
    P.connection !== "connected" ||
    edge.pending ||
    (edge.issue &&
      !(edge.pressure && edge.delivery?.startsWith("BestEffort"))) ||
    A.findNode(edge.from)?.health === "failed" ||
    (A.findNode(edge.from)?.flags?.mute && edge.out !== "_error") ||
    A.findNode(edge.to)?.flags?.pause;
  P.discardDraft = (id) => viewer.change(id, { draft: undefined, submission: undefined });
  P.configContent = (n, draft) =>
    !source.settingsDeclared(n)
      ? C.configFields(n, draft)
      : `<form id="config-form" novalidate><div class="config-fields">${C.configFields(n, draft)}</div>${viewer.card(n.id).submission && viewer.card(n.id).submission.field === undefined ? notice(viewer.card(n.id).submission.message, viewer.card(n.id).submission.status, viewer.card(n.id).submission.code) : ""}<div class="config-action-bar"><div id="config-change-summary" aria-live="polite"></div><div><button id="discard-config" type="button" class="quiet-button">Discard</button><button class="dark-button config-apply" type="submit">${ic("Check")}Apply changes</button></div></div></form>`;
  P.captureDraft = (form, n) => {
    const raw = Object.fromEntries(
      $$("[name]", form)
        .filter((i) => i.type !== "hidden")
        .map((i) => [i.name, i.value]),
    );
    viewer.draft(n.id, { raw: { ...(viewer.card(n.id).draft?.raw || {}), ...raw } });
    try {
      viewer.draft(n.id, {
        shown: C.readConfig(form, { ...n, config: viewer.card(n.id).draft?.shown || n.config }),
      });
      viewer.change(n.id, { submission: undefined });
    } catch (err) {
      viewer.change(n.id, {
        submission: {
          status: "draft",
          message: err?.label ?? "JSON is incomplete",
          code: err?.code ?? "",
        },
      });
    }
    A.drawCards([n.id]);
    if (S.selected === n.id && !S.edge) A.renderInspector();
  };
  P.applyConfig = (form, n) => source.perform({ kind: "configure", actor: n.id, form });
  function permissions(n, v = C.permissions(n)) {
    const decisions = n.permissionDecisions ?? [];
    if (!v?.note && !v?.rows?.length && !v?.empty && !v?.groups?.length && !v?.code && !decisions.length) return "";
    const groups = (v?.groups ?? []).map(group => `<section class="detail-section"><div class="detail-heading"><h3>${e(group.title)}</h3></div>${group.note ? `<p${group.noteCode ? ` data-reason="${e(group.noteCode)}"` : ""}>${e(group.note)}</p>` : ""}${group.rows.map(row => `<div class="detail-row" data-reason="${e(row.code)}"><span>${e(row.label)}${row.subject ? ` · ${e(row.subject)}` : ""}</span><span title="${e(row.value)}">${e(row.value)}</span></div>${row.detail ? `<p data-reason="${e(row.code)}">${e(row.detail)}</p>` : ""}`).join("")}</section>`).join("");
    return `<details class="permissions"><summary>Permissions & decisions</summary>${v?.code ? `<p data-reason="${e(v.code)}" title="${e(v.value)}">${e(v.value)}</p>` : ""}${groups}${v?.note ? `<p>${e(v.note)}</p>` : ""}${decisions.length ? decisions.map((d) => `<div class="detail-row"><span>${e(d.outcome)} · ${e(d.action)}</span><span>${e(d.reason)}</span></div>`).join("") : v?.empty ? `<div class="detail-row" data-approval-decisions><span>Decisions</span><span${v.empty.title ? ` title="${e(v.empty.title)}"` : ""}>${e(v.empty.text)}</span></div>` : ""}${(v?.rows ?? []).map(([name, value]) => `<div class="detail-row"><span>${e(name)}</span><span>${e(value)}</span></div>`).join("")}</details>`;
  }
  function evidenceTarget(r) {
    const [name, value] = source.evidenceTarget(r);
    return `data-${name}="${e(value)}"`;
  }
  P.openAuthoringCommit = (id) => {
    const row = source.acceptedRecords().find(row => row.id === id);
    if (!row) return;
    const time = source.codeLabel(row.timeReason);
    dialog("Accepted edit", `<section class="detail-section"><div class="detail-heading"><h3>${e(row.eventName ?? row.event)}</h3></div><p>${e(row.actorName)}</p><div class="detail-heading"><h3>Authoring commit</h3></div><pre class="code-block" data-commit-id>${e(row.commitId)}</pre><p class="subtle-note" data-reason="${e(time.code)}" title="${e(time.label)}">${e(time.label)}</p><pre class="code-block" data-accepted-command>${e(JSON.stringify(row.command, null, 2))}</pre><details><summary>Authoring commit details</summary><pre class="code-block" data-authoring-commit-detail>${e(JSON.stringify(row.commit, null, 2))}</pre></details></section>`);
  };
  P.inspectContent = (n) => {
    const reading = value => {
      const number = LiveViewers.formatReading(value);
      return number ? e(number.text) : unavailable("UNDECLARED");
    };
    const unavailable = code => {
      const value = source.codeLabel(code);
      return `<span data-reason="${e(code)}" title="${e(value.label)}">${e(value.label)}</span>`;
    };
    function runtimeDetails(records, banner) {
      const latest = records.reduce((last, row) => {
        const index = LiveViewers.formatReading(row.index), previous = LiveViewers.formatReading(last?.index);
        return !last || (index && (!previous || BigInt(index.full) > BigInt(previous.full))) ? row : last;
      }, null);
      const index = LiveViewers.formatReading(latest?.index);
      const arrival = latest
        ? `<button class="runtime-arrival" ${evidenceTarget(latest)} data-evidence-actor="${e(n.id)}"><span>${index ? `#${e(index.text)}` : unavailable("READ_UNAVAILABLE")}</span><small>${latest.timeReason ? unavailable(latest.timeReason) : e(studyTimeFormat(latest.at))}</small></button>`
        : unavailable("ARRIVAL_UNOBSERVED");
      const phaseReason = n.issue?.code ?? (banner.unavailable ? "unobserved" : null);
      return `<section class="detail-section runtime-summary"><div class="detail-heading"><h3>Runtime</h3>${ic("Activity")}</div><div class="detail-row"><span>Location</span><span data-actor-address>${e(source.actorAddress(n))}</span></div><div class="detail-row"><span>Last arrival</span>${arrival}</div><div class="detail-row"><span>Phase</span><span class="state-chip"${bannerAttributes(banner)}${phaseReason ? ` data-reason="${e(phaseReason)}"` : ""}>${e(banner.strong)}</span></div></section><section class="detail-section runtime-mailbox"><div class="detail-heading"><h3>Mailbox</h3></div><div class="detail-row"><span>Declared capacity</span><span data-mailbox-capacity>${reading(LiveViewers.capacity(n))}</span></div>${wireDepths(source.mailbox?.(n))}</section>`;
    }
    function wireDepths(depths) {
      if (!depths) return "";
      const said = (code) => {
        const value = source.codeLabel(code);
        return `<span data-reason="${e(value.code)}" title="${e(value.label)}">${e(value.label)}</span>`;
      };
      if (depths.code !== undefined) return `<div class="detail-row" data-mailbox-depths><span>Inlet depth</span>${said(depths.code)}</div>`;
      return depths.rows.map((row) => `<div class="detail-row" data-mailbox-wire="${e(row.wire)}"><span>From ${e(row.origin)}</span>${row.code !== undefined
        ? said(row.code)
        : `<span>at inlet <span data-depth>${reading(row.depth)}</span> · in mailbox <span data-queued>${reading(row.queued)}</span> · capacity <span data-capacity>${row.capacity === null ? unavailable("UNDECLARED") : reading(row.capacity)}</span></span>`}</div>`).join("");
    }
    function recordedPayload(r) {
      if (!r) return "";
      const missing = r.valueTitle ? source.codeLabel(r.valueTitle) : null;
      return `<section class="detail-section recorded-payload"><div class="detail-heading"><h3>Recorded payload</h3><small>#${e(r.index)}</small></div><p class="subtle-note">${e(r.eventName ?? r.event)}${r.port ? ` · ${e(r.port)}` : ""}</p><pre class="code-block" data-selected-payload${missing ? ` data-reason="${e(missing.code)}" title="${e(missing.label)}" aria-disabled="true"` : ""}>${e(missing ? missing.label : JSON.stringify(r.value, null, 2))}</pre>${r.record ? `<details><summary>Arrival details</summary><pre class="code-block">${e(JSON.stringify(r.record, null, 2))}</pre></details>` : ""}</section>`;
    }
    function portSection(n, side) {
      const ports = side === "in" ? n.in : n.out,
        wires = (p) =>
          A.graph().edges.filter((w) =>
            side === "in" ? w.to === n.id && w.in === p[0] : w.from === n.id && w.out === p[0],
          );
      return ports.length
        ? `<section class="detail-section"><div class="detail-heading"><h3>${side === "in" ? "Inlets" : "Outlets"}</h3></div>${ports.map((p) => {
          const connected = wires(p), shape = source.codeLabel(p[1]);
          const open = side === "out" && source.outletReading(n, p[0], A.graph().edges).unwired
            ? source.codeLabel("OUTLET_UNWIRED") : null;
          const processing = connected.length
            ? `<div class="inlet-processing${connected.length > 1 ? " multiple" : ""}">${connected.map((wire, i) => {
              const origin = `${wire.fromName ?? wire.from}.${wire.out}`;
              return `<button type="button" data-edge="${e(wire.id)}" title="${e(`Open processing from ${origin} to ${n.title}.${p[3] || p[0]}`)}">processing${connected.length > 1 ? ` ${reading(i + 1)} · ${e(origin)}` : ""}</button>`;
            }).join("")}</div>`
            : `<button type="button" disabled data-reason="INLET_UNWIRED" title="${e(source.codeLabel("INLET_UNWIRED").label)}">processing</button>`;
          return `<div class="inspector-port-row${open ? " unwired" : ""}" data-port-row="${e(p[0])}" data-port-shape="${e(p[1])}"${open ? ` data-code="${e(open.code)}"` : ""} title="${e(source.portTitle(p))}${open ? ` · ${e(open.label)}` : ""}"><span class="port-jack"></span><span>${e(p[3] || p[0])}</span><small>${reading(connected.length)} ${connected.length === 1 ? "wire" : "wires"} · ${e(shape.code === p[1] ? shape.label : p[1])}${open ? ` · ${e(open.label)}` : ""}</small>${side === "in" ? processing : ic("ArrowUpRight")}</div>`;
        }).join("")}</section>`
        : "";
    }
    const past = A.historical(),
      banner = source.healthBanner(n, past),
      records = source.actorRecords(n.id),
      instances = source.instances?.(n) ?? n.instancesView,
      access = source.inspectorAccess?.(n) ?? {};
    return `<div class="health-banner"${bannerAttributes(banner)}>${ic(past ? "History" : n.health === "failed" ? "CircleAlert" : n.flags?.pause ? "Pause" : "Radio")}<div><strong>${e(banner.strong)}</strong><small>${e(banner.small)}</small></div></div>${n.issue ? notice(n.issue.message, "failed", [n.issue.code, n.issue.detail?.code].filter((code) => code != null).join(" ")) : ""}${recordedPayload(S.selectedRecord)}${access.harness ?? ""}<div class="viewer-choice"><span>Show</span><select data-view-choice="${n.id}" aria-label="Actor content view">${(C.choices(n) || [[LiveViewers.select(n).kind, "Content"]]).map(([v, t]) => `<option value="${v}" ${LiveViewers.select(n).kind === v ? "selected" : ""}>${e(t)}</option>`).join("")}</select></div><div class="inspector-actions"><button class="quiet-button" data-action="rename" ${past ? "disabled" : ""}>Rename</button></div>${n.scope ? `<button class="quiet-button" data-enter="${n.scope}">${ic("Layers")}${C.containerCardinality(n) === "keyed_many" ? "Open template" : "Enter " + e(n.title)}</button>` : ""}${C.containerCardinality(n) === "keyed_many" || instances?.current?.length || instances?.history?.length ? instanceDetails(instances) : ""}<details class="runtime-details"><summary>Ports & runtime details</summary>${runtimeDetails(records, banner)}${portSection(n, "in")}${portSection(n, "out")}</details><div data-inspector-permissions>${permissions(n, access.permissions)}</div><details><summary>Advanced controls</summary><button class="quiet-button" data-flag="mute" ${past ? "disabled" : ""}>${n.flags?.mute ? "Unmute" : "Mute normal emissions"}</button>${["tap", "debounce"].includes(n.type) ? `<button class="quiet-button" data-flag="bypass" ${past ? "disabled" : ""}>${n.flags?.bypass ? "Disable bypass" : "Bypass processing"}</button>` : ""}<button class="quiet-button" data-flag="pause" ${past ? "disabled" : ""}>${n.flags?.pause ? "Resume actor" : "Pause actor"}</button></details><details><summary>Related events & raw observations</summary>${
      records
        .slice(0, 5)
        .map(
          (r) =>
            `<button class="evidence-link quiet-button" ${evidenceTarget(r)} data-evidence-actor="${n.id}">${e(r.eventName ?? r.event)} · #${e(r.index)} · ${r.timeReason ? `<span data-reason="${e(r.timeReason)}">${e(source.codeLabel(r.timeReason).label)}</span>` : studyTimeFormat(r.at)}</button>`,
        )
        .join("") || LiveViewers.empty("ARRIVAL_UNOBSERVED")
    }<details><summary>Raw observation</summary><pre class="code-block" data-raw-observation>${e(JSON.stringify(source.rawObservation(n.id), null, 2))}</pre></details></details><button class="quiet-button danger-button" data-action="delete" ${past ? "disabled" : ""}>${ic("Trash2")}Delete actor</button>`;
  };
  P.afterInspector = (n) => {
    const content = $("#inspector .inspector-content");
    if (!content) return;
    if (S.tab === "configure") {
      const raw = A.historical() ? null : viewer.card(n.id).draft?.raw;
      if (raw)
        for (const input of $$("[name]", content))
          if (raw[input.name] !== undefined) input.value = raw[input.name];
      content.insertAdjacentHTML("beforeend", permissions(n));
      if (raw) {
        $("#discard-config").disabled = false;
        if (viewer.card(n.id).submission?.status === "rejected")
          $(".config-apply", content).disabled = false;
      }
      if (viewer.card(n.id).submission?.status === "submitting")
        $$("input,textarea,select,.config-apply", content).forEach(
          (i) => (i.disabled = true),
        );
    }
    $("#inspector").classList.remove("folded");
    $("#canvas-view").classList.remove("inspector-folded");
    if (!$(".panel-resizer", $("#inspector")))
      $("#inspector").insertAdjacentHTML(
        "beforeend",
        `<div class="panel-resizer" role="separator" aria-label="Resize inspector" aria-orientation="vertical" aria-valuemin="260" aria-valuemax="520" aria-valuenow="${panelWidth()}" tabindex="0"></div>`,
      );
    source.afterInspector(n);
  };
  const bannerAttributes = (banner) =>
    banner.health === undefined
      ? ""
      : ` aria-disabled="${banner.unavailable}" data-health="${e(banner.health)}" title="${e(banner.title)}"`;
  function instanceDetails(view) {
    const code = view?.code ?? (view?.current ? null : "READ_UNAVAILABLE");
    const unavailable = code ? source.codeLabel(code) : null;
    const count = unavailable ? "—" : LiveViewers.formatReading(view.current.length).text;
    const rows = unavailable
      ? `<p data-reason="${e(unavailable.code)}" title="${e(unavailable.label)}">${e(unavailable.label)}</p>`
      : view.current.map(i => {
        const scope = (S.historyGraph || STUDY)[i.scope];
        const missing = scope?.nodes ? null : source.codeLabel("INSTANCE_SCOPE_UNAVAILABLE");
        const phase = source.codeLabel(i.phase ? `INSTANCE_PHASE_${i.phase}` : "INSTANCE_PHASE_UNOBSERVED");
        return `<div class="instance-row"><button class="quiet-button"${missing ? ` disabled data-reason="${e(missing.code)}" title="${e(missing.label)}"` : ` data-enter="${e(i.scope)}"`}>${e(i.key)} ↗</button>${missing ? `<p class="subtle-note" data-reason="${e(missing.code)}">${e(missing.label)}</p>` : ""}<p class="subtle-note" data-reason="${e(phase.code)}" title="${e(phase.label)}">${e(phase.label)}</p></div>`;
      }).join("") || LiveViewers.empty("INSTANCES_EMPTY");
    const untimed = source.codeLabel("INSTANCE_TRANSITION_UNTIMED");
    const history = unavailable ? "" : `<details class="instance-history"${view.history.length ? ` data-reason="${e(untimed.code)}" title="${e(untimed.label)}"` : ""}><summary>Investigate in history</summary>${view.history.length
      ? view.history.map(row => `<div class="detail-row"><span>${e(row.key)}</span><span>${row.kind === "Minted" ? "Created" : "Retired"}</span></div>`).join("")
      : LiveViewers.empty("INSTANCE_TRANSITIONS_EMPTY")}</details>`;
    return `<details class="instance-details"${unavailable ? ` data-reason="${e(unavailable.code)}"` : ""}><summary>Current instances (${e(count)})</summary>${rows}${history}</details>`;
  }
  P.afterSelection = () => selectionToolbar();
  function renderScopeNavigation() {
    const project = P.projects.find((p) => p.id === P.project),
      home = project?.scope || "root";
    const below = new Map();
    for (const [id, g] of Object.entries(STUDY))
      if (g?.parent != null) below.set(g.parent, [...(below.get(g.parent) || []), id]);
    const scopes = [],
      seen = new Set();
    const visit = (id, name, depth) => {
      if (seen.has(id) || (depth && !STUDY[id])) return;
      seen.add(id);
      scopes.push([id, name, depth]);
      const titles = new Map();
      for (const n of STUDY[id]?.nodes || []) if (n.scope) titles.set(n.scope, n.title);
      for (const child of [...titles.keys(), ...(below.get(id) || [])])
        visit(child, titles.get(child) ?? STUDY[child]?.segment ?? STUDY[child]?.name, depth + 1);
    };
    visit(home, STUDY[home]?.name || "Project", 0);
    $("#scope-navigation").innerHTML = scopes
      .map(
        ([id, name, depth]) =>
          `<button class="scope-row ${depth ? "nested" : ""} ${S.scope === id ? "active" : ""}" data-scope="${id}">${ic(depth ? "Layers" : "Workflow")}<span>${e(name)}</span></button>`,
      )
      .join("");
    $(".project-path").innerHTML =
      `<span class="project-mark">${e((project?.name ?? "")[0] ?? "")}</span><span>${e(project?.name ?? "")}</span>`;
    const crumb = $(".breadcrumb button");
    crumb.dataset.scope = home;
    crumb.textContent = project?.name ?? "";
    const graphs = S.historyGraph || STUDY,
      path = [];
    for (let id = S.scope; graphs[id]; id = graphs[id].parent) path.unshift(id);
    $("#scope-breadcrumb").innerHTML = path
      .map(
        (id) =>
          `<button data-scope="${e(id)}">${e(graphs[id].segment ?? graphs[id].name)}</button>`,
      )
      .join(" / ");
  }
  function selectionToolbar() {
    const toolbar = $("#selection-toolbar"),
      ids = currentIds();
    toolbar.hidden = ids.length < 2 || A.historical();
    $("[data-selection-count]", toolbar).textContent = `${ids.length} selected`;
  }
  function wireIssueMarkers() {
    const markers = $("#wire-issues"),
      said = (code) => source.codeLabel(code);
    markers.innerHTML = A.graph()
      .edges.filter((w) => w.issue || w.pending)
      .map((w) => {
        const n = A.findNode(w.to),
          c = w.combinators?.at(-1),
          port = n?.in.find((p) => p[0] === w.in);
        const x = c?.x ?? n?.x ?? 0,
          y = c?.y ?? (n?.y || 0) + (port?.[2] || 80);
        const r = said(w.pending ? "UNJUDGED" : w.issue.code);
        const values = [w.issue?.step, w.issue?.count].filter((v) => v != null).map((v) => ` · ${e(v)}`).join("");
        return `<button class="wire-issue-marker ${w.pending ? "checking" : w.pressure ? "pressure" : ""}" data-edge="${w.id}" data-code="${e(r.code)}" style="left:${x - 100}px;top:${y - 31}px" title="${e(r.label)}${values}">${w.pending ? "◌" : "!"} ${e(r.label)}${values}</button>`;
      })
      .join("");
  }
  P.afterGraph = () => {
    renderScopeNavigation();
    selectionToolbar();
    renderNotes();
    wireIssueMarkers();
    P.refreshEditTools();
    source.afterGraph();
  };
  P.afterPlaces = () => {
    renderNotes();
    wireIssueMarkers();
  };
  P.afterEdgeInspector = () => {
    const edge = A.graph().edges.find((w) => w.id === S.edge);
    if (!edge) return;
    const content = $("#inspector .inspector-content");
    if (edge.issue)
      $(".wire-summary", content).insertAdjacentHTML(
        "afterend",
        notice(edge.issue.message, "failed", edge.issue.code) + (edge.issue.count == null ? "" :
          `<div class="detail-row" data-dead-letters${edge.issue.failure ? ` data-failure="${e(edge.issue.failure)}"` : ""} data-ordinal="${e(edge.issue.ordinal)}"><span>Dead letters</span><span>${e(edge.issue.count)}</span></div>`),
      );
    else if (edge.issueUnobserved)
      $(".wire-summary", content).insertAdjacentHTML(
        "afterend",
        `<div class="detail-row" data-reason="${e(edge.issueUnobserved.code)}"><span>${e(edge.issueUnobserved.message)}</span></div>`,
      );
    renderCombinatorConfig(edge);
    const section = $(".processing-list", content);
    if (section && edge.combinators.length)
      section.insertAdjacentHTML(
        "afterend",
        `<div class="processing-tools"><button data-reorder="-1">↑ Earlier</button><button data-reorder="1">↓ Later</button></div>`,
      );
    content.insertAdjacentHTML(
      "beforeend",
      `<details class="wire-policy"><summary>Delay, delivery & inlet capacity</summary><form data-wire-policy="${edge.id}"><label>Declared delay${((shown) => `<span class="config-unit-input"><input name="declaredDelay" type="number" min="0" value="${e(shown.text)}" data-duration>${LiveViewers.durationUnitSelect(LiveViewers.durationUnitName("declaredDelay"), shown.unit, "Declared delay")}</span>`)(LiveViewers.durationShown(edge.declaredDelay ?? ""))}</label><label>Delivery<select name="delivery">${((choices, shown) => [...choices, ...(choices.includes(shown) ? [] : [shown])])(["Lossless", "BestEffortDropNewest", "BestEffortDropOldest"], edge.delivery || "Lossless").map((v) => `<option value="${e(v)}" ${v === (edge.delivery || "Lossless") ? "selected" : ""}>${e(deliveryWords[v] ?? v)}</option>`).join("")}</select></label>${edge.deliveryIssue ? `<small class="delivery-issue" data-delivery-code="${e(edge.deliveryIssue.code)}">${e(edge.deliveryIssue.message)}</small>` : ""}<label>Inlet capacity (optional)<input name="capacity" type="number" min="1" value="${edge.capacity ?? ""}"></label><button class="quiet-button">Apply inlet settings</button></form></details><details><summary>Related arrivals & raw detail</summary><pre class="code-block">${e(JSON.stringify({ from: edge.from, out: edge.out, to: edge.to, in: edge.in, issue: edge.issue }, null, 2))}</pre>${A.visibleRecords()
        .filter((r) => r.actor === edge.to)
        .slice(0, 3)
        .map(
          (r) =>
            `<button class="quiet-button" ${evidenceTarget(r)}>${e(r.eventName ?? r.event)} · LOCAL #${e(r.index)} · ${r.timeReason ? `<span data-reason="${e(r.timeReason)}">${e(source.codeLabel(r.timeReason).label)}</span>` : studyTimeFormat(r.at)}</button>`,
        )
        .join("")}</details>`,
    );
    $("#inspector").classList.remove("folded");
    $("#canvas-view").classList.remove("inspector-folded");
    source.afterEdgeInspector(edge);
  };
  const deliveryWords = { Lossless: "Lossless", BestEffortDropNewest: "Best effort, drop newest", BestEffortDropOldest: "Best effort, drop oldest" };
  P.paletteCompatibility = () => source.afterPalette();
  function combinatorEditor(c) {
    if (c.kind === "bang")
      return { slots: [], note: "Bang has no settings." };
    if (c.kind === "flatten")
      return {
        slots: [{ name: "at", label: "Array path · at", initial: [], json: true, required: true }],
        note: "The path must select an array of objects.",
        validate(config) {
          if (!Array.isArray(config.at)) throw Error("Array path must be an array.");
        },
      };
    if (c.kind === "parse")
      return {
        slots: [
          { name: "decoder", label: "Decoder", initial: "json", options: ["json", "kv", "regex"] },
          { name: "field", label: "Input text field", initial: "text", required: true },
          { name: "arguments", label: "Decoder arguments", initial: {}, json: true, indent: 2,
            multiline: true, omitBlank: true, hint: "kv: pair_separator, value_separator · regex: pattern · json: {}" },
        ],
        validate(config) {
          if (!config.field ||
              (config.decoder === "kv" && (!config.arguments?.pair_separator || !config.arguments?.value_separator)) ||
              (config.decoder === "regex" && !config.arguments?.pattern))
            throw Error("Provide the fields required by this decoder.");
        },
      };
    const key = c.kind === "map" ? "transform" : "predicate";
    return {
      slots: [{ name: key, label: `${key === "transform" ? "Transform" : "Predicate"} expression`,
        initial: c.expression, multiline: true, spellcheck: false,
        hint: "The value is named event." }],
      validate(config) {
        if (["map", "filter"].includes(c.kind) && !String(config[key] ?? "").trim())
          throw Error("Enter an expression.");
      },
    };
  }
  const stepDrafts = new Map();
  const pendingStep = (c) => S.pendingSteps?.includes(c);
  const draftKey = (c) => (c === S.draftStep || pendingStep(c) ? c : c.id);
  function keepStepDrafts() {
    const held = new Set(A.graph().edges.flatMap((w) => (w.combinators || []).map((c) => c.id)));
    for (const key of stepDrafts.keys())
      if (typeof key === "string" ? !held.has(key) : key !== S.draftStep && !pendingStep(key)) stepDrafts.delete(key);
  }
  function observedText(c) {
    return Object.fromEntries(combinatorEditor(c).slots.map((slot) => {
      const value = c.config ? c.config[slot.name] : slot.initial;
      return [slot.name, slot.json ? JSON.stringify(value, null, slot.indent) ?? ""
        : slot.options ? value : value ?? ""];
    }));
  }
  function stepText(c) {
    return { ...observedText(c), ...stepDrafts.get(draftKey(c)) };
  }
  function draftMark(c) {
    const draft = c === S.draftStep ? undefined : stepDrafts.get(c.id),
      observed = observedText(c);
    return draft && Object.keys(observed).some((name) => name in draft && draft[name] !== observed[name])
      ? `<span>Draft · not applied</span><button type="button" data-discard-step="${e(c.id)}" aria-label="Discard ${e(c.kind)} draft">Discard</button>`
      : "";
  }
  const markHTML = (c) => `<p class="step-draft-note" data-step-mark>${draftMark(c)}</p>`;
  function drawStepEditors(id, except) {
    for (const form of $$("form[data-product-comb]")) {
      if (form.dataset.productComb !== id) continue;
      const { w, c } = stepOf(form);
      if (!c) continue;
      if (form === except) $("[data-step-mark]", form).outerHTML = markHTML(c);
      else if (form.id === "preprocess-form") renderCombinatorConfig(w);
      else P.afterComponentEditor($("#component-editor"), w, c);
    }
  }
  function stepOf(form) {
    const w = A.graph().edges.find((w) => w.id === form.dataset.productEdge);
    return {
      w,
      c:
        (w?.combinators || []).find((c) => c.id === form.dataset.productComb) ??
        (S.draftStep?.id === form.dataset.productComb ? S.draftStep : null) ??
        S.pendingSteps?.find((c) => c.id === form.dataset.productComb),
    };
  }
  function combinatorFields(c) {
    const editor = combinatorEditor(c), text = stepText(c);
    return editor.slots.map((slot) => {
      const name = `name="${e(slot.name)}"`, value = e(text[slot.name]);
      const control = slot.options
        ? `<select ${name}>${slot.options.map((v) => `<option ${text[slot.name] === v ? "selected" : ""}>${e(v)}</option>`).join("")}</select>`
        : slot.multiline
          ? `<textarea ${name}${slot.spellcheck === false ? ' spellcheck="false"' : ""}>${value}</textarea>`
          : `<input ${name} value="${value}"${slot.required ? " required" : ""}>`;
      return `<label class="config-field"><span class="config-label">${e(slot.label)}</span>${control}${slot.hint ? `<small>${e(slot.hint)}</small>` : ""}</label>`;
    }).join("") + (editor.note ? `<p class="subtle-note">${e(editor.note)}</p>` : "");
  }
  P.afterComponentEditor = (box, edge, c) => {
    keepStepDrafts();
    const form = box.querySelector("form");
    form.dataset.productComb = c.id;
    form.dataset.productEdge = edge?.id ?? "";
    form.innerHTML =
      `<header><span>${e(c.kind)}</span><button type="button" data-close-comb-editor="1" aria-label="Close combinator editor">${ic("X")}</button></header>` +
      combinatorFields(c) +
      markHTML(c) +
      `<button class="inline-apply" type="submit">${pendingStep(c) ? "Keep on wire" : "Apply"}</button>`;
  };
  function renderCombinatorConfig(edge) {
    keepStepDrafts();
    const c = A.inletSteps(edge).active,
      form = $("#preprocess-form");
    if (c && form) {
      form.dataset.productComb = c.id;
      form.dataset.productEdge = edge.id;
      form.innerHTML =
        combinatorFields(c) +
        markHTML(c) +
        '<button class="quiet-button" type="submit">Apply to inlet</button>';
    }
  }
  function stepConfig(c) {
    const editor = combinatorEditor(c), text = stepText(c);
    const config = Object.fromEntries(editor.slots
      .filter((slot) => !slot.omitBlank || text[slot.name].trim())
      .map((slot) => [slot.name, slot.json ? JSON.parse(text[slot.name]) : text[slot.name]]));
    editor.validate?.(config);
    return config;
  }
  function stepError(form, error) {
    let out = $("output", form);
    if (!out) { out = document.createElement("output"); form.append(out); }
    out.className = "inline-notice failure";
    out.dataset.reasonCode = error.code ?? "EDGE_PREPROCESS_CONFIG";
    out.textContent = error.code === "EDIT_UNAVAILABLE" ? "This step is no longer available."
      : error.code ? source.codeLabel(error.code).label
      : error instanceof SyntaxError ? "JSON could not be read." : error.message;
  }
  P.pendingPreprocess = () => {
    const preprocess = [];
    for (const c of S.pendingSteps ?? []) {
      try { preprocess.push({ kind: c.kind, config: stepConfig(c) }); }
      catch (error) {
        if ($("#palette").open) $("#palette").close("continue-wire");
        S.editComponent = c.id;
        A.renderComponentEditor();
        stepError($("#component-editor form"), error);
        return null;
      }
    }
    return preprocess;
  };
  document.addEventListener(
    "submit",
    (ev) => {
      const form = ev.target;
      if (!form.dataset.productComb) return;
      ev.preventDefault();
      ev.stopImmediatePropagation();
      const { w, c } = stepOf(form);
      let config;
      try {
        if (!c || (!w && !pendingStep(c))) throw Object.assign(Error("EDIT_UNAVAILABLE"), { code: "EDIT_UNAVAILABLE" });
        config = stepConfig(c);
      } catch (error) {
        stepError(form, error);
        return;
      }
      $("output", form)?.remove();
      const key = draftKey(c);
      if (pendingStep(c)) {
        c.config = config;
        stepDrafts.delete(key);
        S.editComponent = null;
        A.renderComponentEditor();
        A.openPalette("actors");
        return;
      }
      return Promise.resolve(source.perform({ kind: "applyStep", edge: w.id, step: c, config })).then((accepted) => {
        if (accepted !== true) return;
        stepDrafts.delete(key);
        drawStepEditors(c.id);
      });
    },
    true,
  );
  document.addEventListener("input", (ev) => {
    const form = ev.target.closest?.("form[data-product-comb]");
    if (!form || !ev.target.name) return;
    const { c } = stepOf(form);
    if (!c) return;
    stepDrafts.set(draftKey(c), { ...stepDrafts.get(draftKey(c)), [ev.target.name]: ev.target.value });
    drawStepEditors(c.id, form);
  });
  document.addEventListener("click", (ev) => {
    const button = ev.target.closest?.("[data-discard-step]");
    if (!button) return;
    stepDrafts.delete(button.dataset.discardStep);
    drawStepEditors(button.dataset.discardStep);
  });
  const unproven = new WeakMap();
  P.clearCompatibility = () => {
    for (const p of $$(".node-port")) {
      p.classList.remove("unproven");
      delete p.dataset.compatibilityCode;
      if (!unproven.has(p)) continue;
      p.title = unproven.get(p);
      unproven.delete(p);
    }
  };
  P.showCompatibility = () => {
    P.clearCompatibility();
    const start = S.pendingPort;
    if (!start) return;
    const said = source.codeLabel("UNJUDGED");
    for (const p of $$(".node-port")) {
      if (p.dataset.side === start.side) continue;
      unproven.set(p, p.title);
      p.classList.add("unproven");
      p.dataset.compatibilityCode = said.code;
      p.title = `${p.dataset.port} · ${said.label}`;
    }
  };
  P.connect = (outlet, inlet) => {
    const preprocess = A.state?.pendingSteps ? P.pendingPreprocess() : [];
    if (!preprocess) return false;
    A.clearConnection();
    return source.perform({ kind: "connect", outlet, inlet, ...(preprocess.length ? { preprocess } : {}) });
  };
  function rename() {
    const n = A.selectedNode();
    if (!n || !live()) return;
    dialog(
      "Rename actor",
      `<form data-rename="${n.id}"><label>Name<input name="name" value="${e(n.title)}" required autofocus></label><div class="dialog-actions"><button class="dark-button">Rename</button></div></form>`,
    );
  }
  function remove() {
    return source.perform({ kind: "retireActors", actors: currentIds() });
  }
  function groupDialog() {
    const ids = currentIds();
    if (!ids.length) return;
    const pipelines = A.graph().nodes.filter(
      (n) => C.containerCardinality(n) === "one" && !ids.includes(n.id),
    );
    const fresh = C.items.filter((row) => C.containerCardinality({ type: row.actor_type }));
    const d = dialog(
      "Group actors",
      `<form data-group><label>Destination<select name="target">${fresh.map((row) => `<option value="new" data-type="${e(row.actor_type)}">New ${e(row.label)}</option>`).join("")}${pipelines.map((n) => `<option value="${n.id}">${e(n.title)}</option>`).join("")}</select></label><label data-group-name>New pipeline name<input name="name" value="Working group"></label><div class="dialog-actions"><button class="dark-button">Move actors</button></div></form>`,
    );
    const select = d?.querySelector?.("form[data-group] select"), name = d?.querySelector?.("[data-group-name]");
    if (!select || !name) return;
    const named = () => { const type = select.selectedOptions[0]?.dataset.type; name.hidden = !type || C.containerCardinality({ type }) !== "one"; };
    select.addEventListener("change", named);
    named();
  }
  function group(target, name, type) {
    return source.perform({ kind: "group", actors: currentIds(), into: target, ...(type ? { type } : {}), name });
  }
  function align() {
    return source.perform({ kind: "alignTops", actors: currentIds() });
  }
  P.scopeVisited = (scope) => {
    if (P.navigating) return;
    if (P.nav[P.navAt] !== scope) {
      P.nav.splice(P.navAt + 1);
      P.nav.push(scope);
      P.navAt = P.nav.length - 1;
    }
  };
  function navigate(direction) {
    const at = P.navAt + direction;
    if (at < 0 || at >= P.nav.length) return;
    P.navAt = at;
    P.navigating = true;
    A.setScope(P.nav[at]);
    P.navigating = false;
  }
  function renderNotes() {
    const box = $("#annotations");
    if (!box) return;
    box.innerHTML = (A.graph().notes || [])
      .map(
        (n) =>
          `<article class="annotation" data-note="${n.id}" style="left:${n.x}px;top:${n.y}px;width:${n.width ?? 230}px;height:${n.height ?? 120}px"><header data-note-drag="${n.id}"><span>NOTE</span> <button data-delete-note="${n.id}" aria-label="Delete note">×</button></header><textarea aria-label="Canvas note" ${live() ? "" : "readonly"}>${e(n.text)}</textarea><button class="note-resize" data-note-resize="${n.id}" aria-label="Resize note" ${live() ? "" : "disabled"}></button></article>`,
      )
      .join("");
  }
  const addNote = () => source.perform({ kind: "createNote" });
  P.actions = {
    undo: P.undo,
    redo: P.redo,
    rename,
    delete: remove,
    group: groupDialog,
    align,
    note: addNote,
    back: () => navigate(-1),
    forward: () => navigate(1),
  };
  P.viewChanged = (view) => {
    P.screen = view;
    for (const el of $$(".product-page"))
      el.classList.toggle("hidden", el.id !== view + "-view");
    if (view === "projects") renderProjects();
    if (view === "outputs") renderOutputs();
  };
  function show(view) {
    A.setView(view);
  }
  function projectHeader() {
    return `<div class="page-topline"><div><h1>Projects</h1></div><div class="page-actions"><button class="quiet-button" data-action="open-project">${ic("FolderOpen")}Open project</button>${P.projects.length ? `<button class="dark-button" data-action="new-project">${ic("Plus")}New project</button>` : ""}</div></div>`;
  }
  function stateLine(id) {
    const line = source.projectState?.(id);
    return line
      ? `<div class="surface-state" title="${e(line.label)}" data-reason="${e(line.code)}">${e(line.label)}</div>`
      : `<div class="surface-state">${P.connection === "connected" ? '<span class="tiny-dot green"></span> Available locally' : ""}</div>`;
  }
  function emptyProjects() {
    return LiveViewers.empty(source.recentRefusal?.()?.code ?? "PROJECTS_EMPTY",
      `<button class="dark-button" data-action="new-project">${ic("Plus")}New project</button>`, "projects");
  }
  function renderProjects() {
    const box = $("#projects-view");
    box.innerHTML =
      projectHeader() +
      `<div data-project-notice role="status"></div>` +
      (P.projects.length
        ? `<div class="project-grid">${P.projects.map((p) => `<article class="project-card">
            <span class="project-mark-large" aria-hidden="true">${e(p.name[0])}</span>
            <h2>${e(p.name)}</h2>
            <p class="project-location"><span class="project-location-label">State folder</span><span class="project-location-path">${e(p.id)}</span></p>
            ${stateLine(p.id)}
            <div class="card-actions">
              <button class="quiet-button" data-open-project="${e(p.id)}">Open canvas ${ic("ArrowRight")}</button>
              ${!source.noProject && p.id === P.project && surfaces().length ? '<button class="quiet-button" data-open-output>Outputs ↗</button>' : ""}
              ${!source.noProject && p.id === P.project ? source.projectDaemon?.() ?? "" : ""}
            </div>
          </article>`).join("")}</div>${!source.noProject && P.projects.some((p) => p.id === P.project) ? source.projectPane?.() ?? "" : ""}`
        : emptyProjects()) +
      (source.noProject ? "" : `<div style="margin-top:40px"><button class="quiet-button" data-action="harnesses">${ic("Cpu")}Harnesses</button></div>`);
    source.afterProjects();
  }
  function openProject(id) {
    if (!P.projects.some((p) => p.id === id)) return;
    return source.openState(id);
  }
  const declaredOutputs = () => source.outputs();
  function surfaces() {
    return declaredOutputs().surfaces;
  }
  const PINNED_OUTPUTS = "circular.pinned-outputs";
  function pinnedOutputs() {
    try {
      const kept = JSON.parse(localStorage.getItem(PINNED_OUTPUTS));
      if (kept === null) return {};
      return typeof kept === "object" && !Array.isArray(kept) && Object.values(kept).every(Array.isArray) ? kept : undefined;
    } catch {
      return undefined;
    }
  }
  function pinnedHere() {
    return new Set(pinnedOutputs()?.[P.project]);
  }
  let pinRefused = null;
  function pinOutput(id, pinned) {
    const kept = pinnedOutputs(),
      ids = pinnedHere();
    if (pinned) ids.add(id);
    else ids.delete(id);
    pinRefused = null;
    if (kept) {
      if (ids.size) kept[P.project] = [...ids];
      else delete kept[P.project];
      try {
        localStorage.setItem(PINNED_OUTPUTS, JSON.stringify(kept));
        return;
      } catch {}
    }
    pinRefused = { id, code: "PREFERENCE_UNSAVED" };
  }
  const cardPin = (id, pinned = pinnedHere()) => ({ pinned: pinned.has(id), pinRefused: pinRefused?.id === id ? pinRefused.code : undefined });
  function renderOutputs() {
    const box = $("#outputs-view");
    const observed = declaredOutputs();
    const declared = observed.surfaces;
    const pinned = pinnedHere();
    const shown = [...declared.filter((s) => pinned.has(s.id)), ...declared.filter((s) => !pinned.has(s.id))];
    box.dataset.exportQuery = observed.code || observed.terminal;
    if (observed.count !== undefined) box.dataset.exportCount = String(observed.count);
    else delete box.dataset.exportCount;
    if (observed.cursor !== undefined) box.dataset.exportCursor = observed.cursor;
    else delete box.dataset.exportCursor;
    const title = `<div class="product-eyebrow">${e(P.projects.find((p) => p.id === P.project)?.name || "PROJECT")}</div><h1>Outputs</h1>`;
    if (!declared.length) {
      box.innerHTML = `<div class="page-topline"><div>${title}</div></div>` + LiveViewers.empty(observed.code ?? "OUTPUTS_EMPTY", '<button class="quiet-button" data-view="canvas">Open canvas</button>', "outputs");
      return;
    }
    box.innerHTML = `<div class="page-topline"><div>${title}</div><button class="quiet-button" data-view="canvas">${ic("Workflow")}Open canvas</button></div>` + (observed.code ? notice(source.codeLabel(observed.code).label, "pending", observed.code) : "") +
      `<div class="surface-grid">${shown.map((s) => source.renderSurface(s, cardPin(s.id, pinned))).join("")}</div>`;
    for (const element of box.querySelectorAll("[data-export-surface]")) {
      const surface = shown.find(surface => surface.id === element.dataset.exportSurface);
      if (surface) source.updateSurface(element, surface);
    }
    source.bindSurfaces(box);
  }
  P.refreshOutputs = renderOutputs;
  const refreshApprovalBadge = () => source.refreshApprovalBadge();
  P.refreshOutputNodes = (nodes) => {
    const changed = new Set(nodes.map(node => node.id));
    const declared = declaredOutputs().surfaces;
    for (const surface of declared.filter(surface => surface.views.some(view => changed.has(view.actor))))
      for (const element of $$(`[data-export-surface="${surface.id}"]`)) {
        const next = element.cloneNode(false);
        next.innerHTML = source.renderSurface(surface, cardPin(surface.id));
        A.syncNodeElement(element, next.firstElementChild);
        source.updateSurface(element, surface);
      }
  };
  P.refreshApprovals = () => {
    if ($("#product-dialog").open && $("#product-dialog .approval-list")) approvalDialog();
  };
  function approvalDialog() {
    dialog(
      "Requests",
      `<div class="approval-list">${P.approvals.length ? P.approvals.map((r) => `<article class="approval-card"><header><button class="quiet-button" data-locate="${r.actor}">${e(A.allNode(r.actor)?.title || r.actor)}</button>${r.state === "requested" ? "" : `<span class="status-pill ${r.state}">${e({ submitting: "Submitting…", accepted: r.decision === "deny" ? "Denied" : "Approved", failed: "Not submitted", stale: "No longer pending" }[r.state])}</span>`}</header>${approvalCall(r)}${r.reason ? notice(r.reason, r.state, r.code) : ""}<details><summary>Raw request</summary><pre>${e(JSON.stringify({ id: r.id, actor: r.actor, target: r.target, args: r.args }, null, 2))}</pre></details>${["requested", "failed"].includes(r.state) ? `<div class="approval-actions"><button class="dark-button" data-decide="approve" data-request="${e(r.id)}">${r.state === "failed" ? "Retry approval" : "Approve"}</button><button class="quiet-button" data-decide="deny" data-request="${e(r.id)}">Deny</button></div>` : ""}</article>`).join("") : LiveViewers.empty("REQUESTS_EMPTY")}</div>`,
    );
  }
  function approvalCall(r) {
    const c = r.call;
    if (!c) return `<h3>Approve this request?</h3>${r.callCode ? notice(r.callReason ?? r.callCode, "pending", r.callCode) : ""}`;
    const code = (v) => `<code>${e(v)}</code>`;
    const row = (label, value) => `<div class="approval-call-row"><span>${e(label)}</span><span>${value}</span></div>`;
    const command = (t) => t.program ? code([t.program, ...t.arguments].join(" ")) : "";
    const does = (t) => [t.does ? e(t.does) : "", command(t)].filter(Boolean).join(" ");
    const called = `Arrival #${e(c.index)}${c.port ? ` on ${e(c.port)}` : ""}${c.from ? ` from ${e(c.from)}` : ""}${c.at === null ? "" : ` · ${e(studyTimeFormat(c.at, true))}`} <button class="quiet-button" data-cause-record="${e(r.id)}">Show arrival</button>`;
    return `<h3>${e(c.tool ? `Approve running ${c.tool}?` : "Approve this tool call?")}</h3><div class="approval-call">${
      c.tool ? `${c.does || c.program ? row("Does", does(c)) : ""}${c.input === null ? "" : row("Input", e(c.input))}` : ""
    }${c.tool ? "" : row("Arrived value", e(c.body))}${
      c.steps === null ? row("Wire steps", e("Not known on this screen")) : c.steps.length ? row("Wire steps", c.steps.map(code).join(" → ")) : ""
    }${c.tool ? "" : c.tools.map((t) => row(`Tool ${t.name}`, does(t))).join("")
    }${row("Called by", called)}</div>`;
  }
  async function decide(id, decision) {
    const r = P.approvals.find((r) => r.id === id);
    if (!r || r.state === "submitting") return;
    await source.perform({ kind: "decide", request: { ...r, decision } });
    approvalDialog();
    refreshApprovalBadge();
    A.renderGraph();
  }
  function locate(id, scope) {
    if ($("#product-dialog").open) $("#product-dialog").close();
    const found = Object.entries(S.historyGraph || A.liveScopes()).find(
      ([, g]) => g.nodes?.some((n) => n.id === id),
    );
    if (!found) return;
    if (S.scope !== found[0]) A.setScope(found[0]);
    show("canvas");
    A.selectNode(id);
    const n = A.findNode(id),
      r = $("#canvas").getBoundingClientRect();
    S.x = r.width / 2 - (n.x + n.width / 2) * S.zoom;
    S.y = r.height / 2 - (n.y + A.nodeHeight(n) / 2) * S.zoom;
    A.transformWorld();
  }
  Object.assign(P.actions, { approvals: approvalDialog }, source.actions);
  document.addEventListener(
    "click",
    (ev) => {
      const b = ev.target.closest("button,[data-action]");
      if (!b) return;
      if (b.id === "close-inspector") {
        $("#inspector").classList.add("folded");
        $("#canvas-view").classList.add("inspector-folded");
        S.selectedSet.clear();
        return;
      }
      if (b.dataset.action) {
        ev.stopImmediatePropagation();
        P.actions[b.dataset.action]?.();
        return;
      }
      if (b.hasAttribute("data-product-close")) {
        $("#product-dialog").close();
        return;
      }
      if (b.dataset.openProject) {
        openProject(b.dataset.openProject);
        return;
      }
      if (b.hasAttribute("data-open-output")) {
        show("outputs");
        return;
      }
      if (b.dataset.pinSurface) {
        const id = b.dataset.pinSurface;
        pinOutput(id, !pinnedHere().has(id));
        renderOutputs();
        $$("[data-pin-surface]", $("#outputs-view")).find((pin) => pin.dataset.pinSurface === id)?.focus({ preventScroll: true });
        return;
      }
      if (b.dataset.decide) {
        decide(b.dataset.request, b.dataset.decide);
        return;
      }
      if (b.dataset.locate) {
        locate(b.dataset.locate);
        return;
      }
      if (b.dataset.causeRecord) {
        const r = P.approvals.find((row) => row.id === b.dataset.causeRecord);
        if (!r?.record) return;
        locate(r.record.actor);
        S.selectedRecord = r.record;
        A.renderInspector();
        A.renderJournal();
        return;
      }
      if (b.dataset.eventAt) {
        A.timeMachine.seek(Number(b.dataset.eventAt));
        return;
      }
      if (b.dataset.flag)
        return source.perform({ kind: "setFlag", actor: S.selected, flag: b.dataset.flag });
      if (b.dataset.reorder) {
        const w = A.graph().edges.find((w) => w.id === S.edge);
        if (!w) return;
        return source.perform({
          kind: "moveStep",
          edge: w.id,
          index: w.combinators.findIndex((c) => c.id === (S.component || w.combinators[0]?.id)),
          direction: Number(b.dataset.reorder),
        });
      }
      if (b.dataset.showInstances) {
        A.selectNode(b.dataset.showInstances);
        $(".instance-details").open = true;
        return;
      }
      if (b.dataset.editValue) {
        A.selectNode(b.dataset.editValue);
        S.tab = "configure";
        A.renderInspector();
        return;
      }
      if (b.hasAttribute("data-dismiss-notice")) {
        unbanner();
        return;
      }
      if (b.dataset.deleteNote)
        return source.perform({ kind: "retireNote", note: b.dataset.deleteNote });
    },
    true,
  );
  document.addEventListener(
    "toggle",
    (ev) => {
      const details = ev.target;
      if (!details.matches?.(".sdk-details") || !details.open) return;
      const n = A.selectedNode();
      if (!n) return;
      const body = $(".sdk-body", details);
      body.innerHTML = A.sdkContent(n);
    },
    true,
  );
  document.addEventListener("change", (ev) => {
    const t = ev.target;
    if (t.dataset.viewChoice)
      return source.perform({ kind: "setView", actor: t.dataset.viewChoice, view: t.value });
    if (t.tagName === "TEXTAREA" && t.closest(".annotation"))
      return source.perform({ kind: "noteBody", note: t.closest(".annotation").dataset.note, text: t.value });
  });
  document.addEventListener("submit", (ev) => {
    const form = ev.target;
    if (form.dataset.rename) {
      ev.preventDefault();
      return Promise.resolve(
        source.perform({ kind: "rename", actor: form.dataset.rename, label: form.elements.name.value.trim() }),
      ).then((renamed) => renamed && $("#product-dialog").close());
    }
    if (form.hasAttribute("data-group")) {
      ev.preventDefault();
      const target = form.elements.target, chosen = target.selectedOptions[0];
      $("#product-dialog").close();
      group(target.value, form.elements.name.value, chosen?.dataset.type);
    }
    if (form.dataset.wirePolicy) {
      ev.preventDefault();
      if (!form.reportValidity()) return;
      const field = (name) => form.elements[name].value;
      const delay = LiveViewers.durationMs(field("declaredDelay"), field(LiveViewers.durationUnitName("declaredDelay")));
      return source.perform({
        kind: "inletSettings",
        edge: form.dataset.wirePolicy,
        values: { delay: delay ?? field("declaredDelay"), delivery: field("delivery"), capacity: field("capacity") },
      });
    }
    if (form.dataset.typedForm) {
      ev.preventDefault();
      return source.perform({
        kind: "inject",
        actor: form.dataset.typedForm,
        entered: Object.fromEntries([...form.elements].filter((el) => el.name)
          .map((el) => [el.name, el.type === "checkbox" ? el.checked : el.value])),
        line: $("output", form),
      });
    }
  });
  document.addEventListener(
    "keydown",
    (ev) => {
      const input = ev.target.closest(
        "input,textarea,select,[contenteditable]",
      );
      if (input || $("dialog[open]") || S.view !== "canvas") return;
      if ((ev.metaKey || ev.ctrlKey) && ev.key.toLowerCase() === "z") {
        ev.preventDefault();
        ev.stopImmediatePropagation();
        ev.shiftKey ? P.redo() : P.undo();
      }
      if ((ev.metaKey || ev.ctrlKey) && ev.key.toLowerCase() === "a") {
        ev.preventDefault();
        S.selectedSet = new Set(A.graph().nodes.map((n) => n.id));
        A.renderGraph();
      }
      if (["Delete", "Backspace"].includes(ev.key)) {
        ev.preventDefault();
        remove();
      }
      if (ev.key === "F2") {
        ev.preventDefault();
        rename();
      }
    },
    true,
  );
  let panelDrag = null;
  document.addEventListener(
    "pointerdown",
    (ev) => {
      const resize = ev.target.closest(".panel-resizer");
      if (resize) {
        panelDrag = {
          x: ev.clientX,
          width: $("#inspector").getBoundingClientRect().width,
          start: panelWidth(),
          next: null,
          frame: 0,
        };
        resize.setPointerCapture(ev.pointerId);
        ev.preventDefault();
      }
    },
    true,
  );
  document.addEventListener("pointermove", (ev) => {
    if (panelDrag) {
      const d = panelDrag;
      d.next = d.width + d.x - ev.clientX;
      d.frame ||= requestAnimationFrame(() => {
        d.frame = 0;
        panelWidth(d.next);
      });
    }
  });
  document.addEventListener("pointerup", () => {
    if (panelDrag) {
      const d = panelDrag;
      cancelAnimationFrame(d.frame);
      if (d.next !== null) panelWidth(d.next);
      if (panelWidth() !== d.start) panelSettled();
    }
    panelDrag = null;
  });
  document.addEventListener("keydown", (ev) => {
    if (
      !ev.target.closest?.(".panel-resizer") ||
      !["ArrowLeft", "ArrowRight"].includes(ev.key)
    )
      return;
    ev.preventDefault();
    ev.stopPropagation();
    const before = panelWidth(),
      step = ev.shiftKey ? 24 : 8;
    panelWidth(before + (ev.key === "ArrowLeft" ? step : -step));
    if (panelWidth() !== before) panelSettled();
  });
  function panelWidth(width) {
    const root = document.documentElement;
    if (width !== undefined) {
      root.style.setProperty(
        "--panel-width",
        Math.round(Math.min(520, Math.max(260, width))) + "px",
      );
      const now = panelWidth();
      for (const handle of $$(".panel-resizer"))
        handle.setAttribute("aria-valuenow", now);
    }
    return parseFloat(getComputedStyle(root).getPropertyValue("--panel-width"));
  }
  function panelSettled() {
    try {
      localStorage.setItem("circular.panel-width", String(panelWidth()));
    } catch {}
  }
  try {
    const saved = Number(localStorage.getItem("circular.panel-width"));
    if (saved) panelWidth(saved);
  } catch {}
  source.attach({ P, A, S, C, $, e, copy, wait, show, dialog, banner, notice,
    projectHeader, renderProjects, renderOutputs });
  A.renderGraph();
  A.oweFit([S.scope]);
  renderProjects();
  renderOutputs();
  const requested = location.hash.slice(1);
  show(["projects", "outputs"].includes(requested) ? requested : S.view);
})();
