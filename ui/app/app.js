(() => {
  "use strict";
  const source = window.StudySource;
  const viewer = window.StudyViewer;
  const $ = (s, root = document) => root.querySelector(s);
  const $$ = (s, root = document) => [...root.querySelectorAll(s)];
  const esc = (value) =>
    String(value).replace(
      /[&<>"']/g,
      (c) =>
        ({
          "&": "&amp;",
          "<": "&lt;",
          ">": "&gt;",
          '"': "&quot;",
          "'": "&#39;",
        })[c],
    );
  const icon = (name) => {
    const known = Object.hasOwn(window.ICONS, name);
    return `<svg class="icon" aria-hidden="true" viewBox="0 0 24 24"${
      known ? "" : ` data-icon-missing="${esc(name)}"`
    }>${known ? "" : "<title>ICON_UNREGISTERED</title>"}${(
      known ? window.ICONS[name] : window.ICONS.Box
    )
      .map(
        ([tag, attrs]) =>
          `<${tag} ${Object.entries(attrs)
            .map(([k, v]) => `${k}="${esc(v)}"`)
            .join(" ")}></${tag}>`,
      )
      .join("")}</svg>`;
  };
  function hydrate(root = document) {
    $$("[data-icon]", root).forEach(
      (el) => (el.outerHTML = icon(el.dataset.icon)),
    );
  }
  const state = {
    scope: "root",
    selected: null,
    edge: null,
    tab: "inspect",
    view: "canvas",
    zoom: 1,
    tier: "detail",
    x: 0,
    y: 0,
    tool: "select",
    space: false,
    journal: "all",
    pendingPort: null,
    paused: new Set(),
    journalRows: [...source.projection.journal],
    selectedRecord: null,
    generation: 0,
    component: null,
    addAt: null,
    edgeAnchor: null,
    hoverEdge: null,
    selectedSet: new Set(),
    cameras: new Map(),
    fitOwed: new Set(),
    cameraTaken: new Set(),
  };
  let archive, timeMachine;
  const historical = () => Boolean(timeMachine && timeMachine.mode !== "live");
  const graph = () =>
    (state.historyGraph || source.projection)[state.scope] || {
      name: source.projection[state.scope]?.name || "Recorded scope",
      nodes: [],
      edges: [],
      notes: [],
      absent: true,
    };
  const liveScopes = () =>
    Object.fromEntries(Object.entries(source.projection).filter(([, g]) => g.nodes));
  const displayTime = () => timeMachine?.position ?? 120;
  const visibleRecords = () =>
    archive ? archive.rows(displayTime()) : state.journalRows;
  const findNode = (id) => graph().nodes.find((n) => n.id === id);
  const allNode = (id) =>
    Object.values(state.historyGraph || source.projection)
      .filter((g) => g.nodes)
      .flatMap((g) => g.nodes)
      .find((n) => n.id === id);
  const selectedNode = () => findNode(state.selected);
  const paused = () => state.paused.has(state.scope);
  const size = (n) => LiveViewers.size(n);
  const nodeHeight = (n) => n.height || size(n).height;
  const portY = (n, p) => (viewTraits(n).portsAtFoot ? nodeHeight(n) - 39 : p[2]);
  const portAt = (n, p) => portY(n, p) + source.metrics.inset;
  const toWorld = (x, y) => {
    const r = $("#canvas").getBoundingClientRect();
    return {
      x: (x - r.left - state.x) / state.zoom,
      y: (y - r.top - state.y) / state.zoom,
    };
  };
  const stepLooks = {
    flatten: { icon: "Layers", detail: "Emit each item of an incoming array" },
    map: { icon: "Braces", detail: "Shape a value" },
    filter: { icon: "Filter", detail: "Keep matching events" },
    parse: { icon: "Braces", detail: "Decode the incoming value" },
    bang: { icon: "Zap", detail: "Turn an arrival into a trigger" },
  };
  const stepLook = (kind) =>
    stepLooks[kind] ?? {
      icon: "Braces",
      detail: "",
      iconDiagnostic: "COMBINATOR_ICON_UNAVAILABLE",
    };
  let field;
  let travel;
  const geometry = new Map();
  const geometryDirty = new Set();
  let geometryAll = false;
  let geometryFrame = 0,
    probeTimer,
    probePosition = { x: 0, y: 0 },
    probeLast = 0,
    suppressClickUntil = 0;
  function scheduleWires(edgeId = null) {
    if (edgeId) geometryDirty.add(edgeId);
    else geometryAll = true;
    if (!geometryFrame)
      geometryFrame = requestAnimationFrame(() => {
        geometryFrame = 0;
        const onlyIds = geometryAll ? null : new Set(geometryDirty);
        renderWires(onlyIds);
      });
  }
  function edgeRate(id, t = 0) {
    const e = graph().edges.find((e) => e.id === id);
    if (!e || (!historical() && (paused() || window.Product?.edgeQuiet(e))))
      return 0;
    return source.rate(e, t) ?? 0;
  }
  const edgeValue = (e) => source.edgeValue(e);
  function componentCue(c) {
    if (c.kind === "map")
      return c.expression === "value => value"
        ? "identity"
        : c.expression.replace(/^\s*\w+\s*=>\s*/, "").slice(0, 15);
    if (c.kind === "filter")
      return c.expression.includes("null") ? "non-null" : "predicate";
    return c.kind === "parse" ? "JSON" : "trigger";
  }
  let toastTimer;
  const toastIcons = { success: "Check", notice: "CircleAlert", failure: "CircleX" };
  function toast(message, action, severity = "notice", code, sentence) {
    const el = $("#toast");
    el.dataset.severity = severity;
    if (code !== undefined) el.dataset.reason = code;
    else delete el.dataset.reason;
    const words = `<span>${esc(message)}</span>`;
    el.innerHTML =
      icon(toastIcons[severity] ?? toastIcons.notice) +
      (sentence
        ? `<div class="toast-lines">${words}<small class="toast-sentence" title="${esc(sentence)}">${esc(sentence)}</small></div>`
        : words) +
      (action
        ? `<button data-action="${esc(action.name)}">${esc(action.label)}</button>`
        : "");
    el.classList.add("visible");
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => el.classList.remove("visible"), 3800);
  }
  function gestureCode(code, refusal = true) {
    return refusal ? source.refuse({ code }) : source.notice(code);
  }
  function portHTML(node, [name, type, y, label = name, caption = label], side) {
    const outlet = side === "out" ? source.outletReading(node, name, graph().edges) : null;
    const connected = outlet ? outlet.wired
      : graph().edges.some((edge) => edge.to === node.id && edge.in === name);
    const pending =
      state.pendingPort?.node === node.id && state.pendingPort?.name === name;
    const unwired =
      outlet?.unwired
        ? source.codeLabel("OUTLET_UNWIRED")
        : null;
    const said = unwired
      ? ` · ${unwired.label} · drag it to an inlet or onto the canvas to wire it`
      : "";
    return `<button class="node-port ${side}${connected ? " connected" : ""}${unwired ? " unwired" : ""}${pending ? " pending" : ""}" style="top:${portY(node, [name, type, y])}px" data-port="${esc(name)}" data-node="${esc(node.id)}" data-side="${side}"${unwired ? ` data-code="${esc(unwired.code)}"` : ""} title="${esc(name)} · ${esc(source.codeLabel(type).code === type ? source.codeLabel(type).label : type)} · ${side === "in" ? "inlet" : "outlet"}${esc(said)}" aria-label="${esc(node.title)} ${esc(name)} ${side === "in" ? "inlet" : "outlet"}${unwired ? ` · ${esc(unwired.label)}` : ""}"><span class="port-jack"></span><span title="${esc(label)}"${caption ? "" : " hidden"}>${esc(label)}</span></button>`;
  }
  const viewTraits = (n) => LiveViewers.traits(LiveViewers.select(n).kind);
  const interactive = (n) =>
    historical()
      ? false
      : (viewer.card(n.id).mode ?? viewTraits(n).interactive === true);
  const hasInlineEditor = (n) =>
    ProductCatalog.byType.get(n.type)?.configDeclared === true &&
    viewTraits(n).inlineSettings !== false;
  function nodeBody(n, note = "", accepts = true) {
    const fields = hasInlineEditor(n)
      ? (x) => ProductCatalog.configFields(x)
      : null;
    return (
      LiveViewers.render(n, historical() ? {} : viewer.card(n.id), state.tier, accepts) +
      (fields
        ? `<form class="node-inline-editor" data-node-config="${n.id}"><div class="inline-editor-fields">${note}<div class="inline-editor-heading">ACTOR SETTINGS</div>${fields({ ...n, config: historical() ? n.config : viewer.card(n.id).draft?.shown || n.config })}</div><button class="inline-apply" type="submit" ${drafted(n) && viewer.card(n.id).submission?.status !== "submitting" ? "" : "disabled"}>${icon("Check")}Apply changes</button></form>`
        : "")
    );
  }
  function toggleNodeMode(id) {
    const n = findNode(id);
    if (state.selected !== id) selectNode(id);
    viewer.setMode(id, !interactive(n));
    drawCards([id]);
    if (interactive(n))
      requestAnimationFrame(() =>
        $("input,textarea,select", $("#node-" + id))?.focus({
          preventScroll: true,
        }),
      );
  }
  function renderComponentEditor() {
    const box = $("#component-editor");
    if (!state.editComponent) {
      box.classList.add("hidden");
      return;
    }
    const pending = state.pendingSteps?.find((c) => c.id === state.editComponent);
    const draft =
      state.draftStep?.id === state.editComponent ? state.draftStep : null;
    const e = draft
        ? graph().edges.find((e) => e.id === draft.edge)
        : graph().edges.find((e) =>
            e.combinators.some((c) => c.id === state.editComponent),
          ),
      c = pending ?? draft ?? e?.combinators.find((c) => c.id === state.editComponent);
    if (!c) {
      box.classList.add("hidden");
      return;
    }
    box.anchor = { x: c.x + 46, y: c.y + 22 };
    const opening = box.classList.contains("hidden");
    box.classList.remove("hidden");
    const contentKey = JSON.stringify([c.id, c.kind, c.expression]);
    if (opening || box.dataset.contentKey !== contentKey) {
      box.dataset.contentKey = contentKey;
      box.innerHTML = `<form data-inline-comb="${c.id}" data-inline-edge="${e?.id ?? ""}"><header><span>${esc(c.kind)}</span><button type="button" data-close-comb-editor="1" aria-label="Return to combinator view">${icon("X")}</button></header><label>${c.kind === "bang" ? "OUTPUT" : c.kind === "parse" ? "DECODER" : "EXPRESSION"}${["map", "filter"].includes(c.kind) ? `<textarea name="expression" spellcheck="false" aria-label="Inline combinator expression">${esc(c.expression)}</textarea>` : `<input name="expression" value="${esc(c.expression)}" readonly>`}</label><footer><span>${e ? `${esc(e.to)}.in.${esc(e.in)}` : "Choose the receiving inlet to finish this wire."}</span><button class="inline-apply" type="submit">${icon("Check")}Apply</button></footer></form>`;
      window.Product?.afterComponentEditor(box, e, c);
    }
    placeComponentEditor();
  }
  const drawnFaces = new WeakMap();
  const faceKey = (face) => JSON.stringify(face);
  const deviceState = (element, name) =>
    (name === "open" && (element.nodeName === "DETAILS" || element.nodeName === "DIALOG")) ||
    name === "data-settings-short";
  const textControl = (element) =>
    element.nodeName === "TEXTAREA" ||
    (element.nodeName === "INPUT" &&
      !["checkbox", "radio", "hidden", "button", "submit", "reset", "file"].includes(element.type));
  function syncNodeElement(current, next) {
    const drawn = textControl(current) ? current.defaultValue : undefined,
      edited = current === document.activeElement && !current.readOnly;
    for (const name of Array.from(current.attributes, (a) => a.name))
      if (!next.hasAttribute(name) && !deviceState(current, name))
        current.removeAttribute(name);
    for (const attribute of next.attributes)
      if (
        current.getAttribute(attribute.name) !== attribute.value &&
        !deviceState(current, attribute.name)
      )
        current.setAttribute(attribute.name, attribute.value);
    const kept = current.childNodes,
      fresh = next.childNodes;
    for (let i = 0; i < fresh.length; i++) {
      const want = fresh[i],
        have = kept[i];
      if (!have) current.appendChild(want.cloneNode(true));
      else if (have.nodeName !== want.nodeName)
        current.replaceChild(want.cloneNode(true), have);
      else if (have.attributes) syncNodeElement(have, want);
      else if (have.nodeValue !== want.nodeValue)
        have.nodeValue = want.nodeValue;
    }
    while (kept.length > fresh.length)
      current.removeChild(kept[kept.length - 1]);
    if (
      current.nodeName === "SELECT" &&
      current !== document.activeElement &&
      current.value !== next.value
    )
      current.value = next.value;
    if (
      textControl(current) &&
      !(edited && !current.readOnly) &&
      current.value !== next.value &&
      (drawn !== next.defaultValue || next.value !== next.defaultValue)
    )
      current.value = next.value;
  }
  function cardElement(frame, node, face) {
    frame.innerHTML = nodeHTML(node, face);
    const card = frame.content.firstElementChild,
      past = historical();
    for (const control of card.querySelectorAll("input, textarea"))
      if (past || (control.matches(".prompt-input") && !interactive(node)))
        control.setAttribute("readonly", "");
    if (past)
      for (const control of card.querySelectorAll("select, .prompt-send"))
        control.setAttribute("disabled", "");
    return card;
  }
  function renderNodes() {
    const host = $("#nodes"),
      nodes = graph().nodes,
      present = new Set(nodes.map((n) => n.id));
    for (const card of Array.from(host.children))
      if (!present.has(card.dataset.nodeId)) card.remove();
    const standing = new Map(
      Array.from(host.children, (card) => [card.dataset.nodeId, card]),
    );
    const frame = document.createElement("template");
    let order = 0;
    nodes.forEach((node, index) => {
      const face = source.face(node),
        drawn = cardElement(frame, node, face);
      let card = standing.get(node.id);
      const seen = card && placeOf(card);
      if (card) syncNodeElement(card, drawn);
      else card = drawn;
      if (host.children[index] !== card)
        host.insertBefore(card, host.children[index] || null);
      drawnFaces.set(card, faceKey(face));
      if (seen && (seen.x !== node.x || seen.y !== node.y))
        travel.go(card, node.id, seen, node, order++);
    });
    for (const form of host.querySelectorAll("form[data-node-config]"))
      if (!viewer.card(form.dataset.nodeConfig).draft) form.reset();
    if (drag?.kind === "resize" && $("#node-" + drag.node.id)) {
      $("#node-" + drag.node.id).classList.add("resizing");
      applyNodeSize(findNode(drag.node.id) ?? drag.node);
    }
    if (state.pendingPort) window.Product?.showCompatibility();
    drawBodies(nodes);
  }
  function nodeHTML(node, face = source.face(node)) {
    const activity = [!historical() && paused() ? "Paused" : node.activity,
      ...(node.diagnostics ?? []).map((code) => source.codeLabel(code).label)].filter(Boolean).join(" · ");
    const diagnosed = node.diagnostics?.length ? ` data-reason="${esc(node.diagnostics.join(" "))}"` : "";
    const past = historical() ? " disabled" : "";
    const accepts = CanvasTier.holdsControl({ width: node.width, height: nodeHeight(node) }, state.zoom, state.controlRoom?.[state.tier]);
    return `<article class="node node-${LiveViewers.select(node).kind}${interactive(node) ? " is-interactive" : ""}${hasInlineEditor(node) ? " has-inline-editor" : ""}${nodeHeight(node) < size(node).height ? " compact" : ""}${state.selectedSet.has(node.id) ? " selected" : ""}${node.issue ? " issue" : ""}${drafted(node) ? " has-draft" : ""}" id="node-${node.id}" data-node-id="${node.id}" data-view-host="${esc(node.id)}"${diagnosed}${face.life === undefined ? "" : ` data-life="${esc(face.life)}"`}${accepts ? "" : ' data-places="none"'} style="left:${node.x}px;top:${node.y}px;width:${node.width}px;height:${nodeHeight(node)}px;--prompt-port-y:${nodeHeight(node) - 39}px" tabindex="0" aria-label="${esc(node.title)}, ${esc(node.type)}, ${esc(activity)}"><header class="node-header" data-drag="${node.id}"><span class="node-icon">${icon(node.icon)}</span><div class="node-titles" title="${esc(node.title)} · ${esc(node.typeLabel ?? node.type)}"><div class="node-title">${state.tier === CanvasTier.DETAIL ? esc(node.title) : CanvasTier.wordsHTML(node.title, esc)}</div><div class="node-type">${(node.typeLabel ?? node.type) === node.title ? "" : esc(node.typeLabel ?? node.type)}</div></div><span class="${dotClass("node-life tiny-dot", face.dot)}"${dotAttrs(face.dot)}></span><button class="viewer-toggle ${interactive(node) ? "active" : ""}" data-viewer-toggle="${node.id}"${past} aria-pressed="${interactive(node)}" aria-label="${esc(node.title)}: ${interactive(node) ? "switch to View" : "switch to Interact"}" title="${interactive(node) ? "Interact · switch to View" : "View · switch to Interact"}">${icon(interactive(node) ? "MousePointer2" : "SlidersHorizontal")}</button></header>${stateRow(face.row)}${node.in.map((p) => portHTML(node, p, "in")).join("")}${node.out.map((p) => portHTML(node, p, "out")).join("")}${nodeBody(node, statusNote(node, face.flags, face.revisionNote, face.row.band), accepts)}<button class="node-resize" data-resize="${node.id}"${past} aria-label="Resize ${esc(node.title)}" title="Drag to resize; arrow keys adjust size"><span></span></button><span class="resize-dimensions">${Math.round(node.width)} × ${Math.round(nodeHeight(node))}</span>${statusNote(node, face.flags, face.revisionNote, face.row.band)}</article>`;
  }
  const dotClass = (base, dot) => `${base}${dot.color ? " " + dot.color : ""}`;
  const dotAttrs = (dot) =>
    `${dot.health === undefined ? "" : ` data-health="${esc(dot.health)}"`}${dot.state === undefined ? "" : ` data-state="${esc(dot.state)}"`}${dot.title === undefined ? "" : ` title="${esc(dot.title)}"`}${dot.unavailable === undefined ? "" : ` aria-disabled="${dot.unavailable}"`}${dot.fill === undefined ? "" : ` style="display:inline-block;background-color:${esc(dot.fill)}"`}`;
  const stateRow = (row) =>
    `<div class="node-state"${row.state === undefined ? "" : ` data-state="${esc(row.state)}"`}${row.band ? ` data-band="${esc(row.band)}"` : ""}${row.text === undefined ? "" : ` title="${esc(row.text)}"`}${row.reasons?.length ? ` data-reason="${esc(row.reasons.join(" "))}"` : ""}${row.unavailable === undefined ? "" : ` aria-disabled="${row.unavailable}"`}><span class="${dotClass("tiny-dot", row)}"${dotAttrs({ ...row, title: row.text })}></span>${esc(row.code)}${row.phrase ? `<span class="node-state-phrase">${esc(row.phrase)}</span>` : ""}</div>`;
  function statusNote(node, flags, revisionNote, band) {
    const approval = node.approvalCount || 0;
    const text =
      node.issue?.message ||
      flags ||
      (approval ? `${approval} approval ${approval === 1 ? "request" : "requests"}` : "");
    const reason = (node.issue?.message && node.issue.code != null
      ? ` data-reason="${esc([node.issue.code, node.issue.detail?.code].filter((code) => code != null).join(" "))}"` : "")
      + (node.issue?.message && band ? ` data-band="${esc(band)}"` : "");
    if (revisionNote) return `<div class="node-status-note"${reason} title="${esc(revisionNote)}">Earlier revision${text ? ` · ${esc(text)}` : ""}</div>`;
    return text
      ? `<div class="node-status-note"${reason}>${esc(text)}${approval ? '<button data-action="approvals">Review</button>' : ""}</div>`
      : "";
  }
  function renderGraph() {
    renderNodes();
    $("#actor-list").innerHTML = graph()
      .nodes.map((n) => listRowHTML(n))
      .join("");
    $("#actor-count").textContent = graph().nodes.length;
    $("#scope-title").innerHTML =
      `${esc(graph().name)} <span class="title-count">${graph().nodes.length}</span>`;

    $$(".scope-row").forEach((b) =>
      b.classList.toggle("active", b.dataset.scope === state.scope),
    );
    updatePause();
    renderWires();
    renderJournal();
    renderInspector();
    syncHistoryControls();
    window.Product?.afterGraph();
  }
  function listRowHTML(n, face = source.face(n)) {
    const dot = face.dot,
      health =
        dot.health === undefined
          ? ""
          : ` data-health="${esc(dot.health)}" title="${esc(dot.title)}"`;
    return `<button data-select="${n.id}" class="${state.selectedSet.has(n.id) ? "selected" : ""}"${health}>${icon(n.icon)}<span title="${esc(n.title)}">${esc(n.title)}</span><span class="tiny-dot"${health}${dot.health === undefined ? "" : ` aria-disabled="${dot.unavailable}"`}></span></button>`;
  }
  function drawCards(ids, reformed = false) {
    const frame = document.createElement("template"),
      nodes = [];
    let moved = false,
      order = 0;
    for (const id of ids) {
      const node = graph().nodes.find((n) => n.id === id),
        card = $("#node-" + id);
      if (!node || !card) continue;
      const face = source.face(node),
        seen = placeOf(card),
        box = [card.style.width, card.style.height];
      syncNodeElement(card, cardElement(frame, node, face));
      drawnFaces.set(card, faceKey(face));
      if (seen.x !== node.x || seen.y !== node.y) {
        travel.go(card, id, seen, node, order++);
        moved = true;
      } else if (box[0] !== card.style.width || box[1] !== card.style.height)
        moved = true;
      const row = $(`#actor-list button[data-select="${id}"]`);
      if (row) {
        frame.innerHTML = listRowHTML(node, face);
        syncNodeElement(row, frame.content.firstElementChild);
      }
      const form = $("form[data-node-config]", card);
      if (form && !viewer.card(id).draft) form.reset();
      if (drag?.kind === "resize" && drag.node.id === id) {
        card.classList.add("resizing");
        applyNodeSize(node);
      }
      nodes.push(node);
    }
    drawBodies(nodes);
    if (reformed) tierForm(nodes);
    if (moved) {
      if (!travel.moving) renderWires();
      window.Product?.afterPlaces?.();
    }
    return nodes;
  }
  const TIER_FORM = ".node-state, .node-icon, .node-type, .node-port > span:last-child, .viewer-toggle";
  function tierForm(nodes) {
    if (state.tier !== CanvasTier.DETAIL || matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const root = getComputedStyle(document.documentElement),
      timing = { duration: parseFloat(root.getPropertyValue("--hf-10-panel")) || 0, easing: root.getPropertyValue("--hf-10-set").trim() || "linear" };
    if (!timing.duration) return;
    for (const n of nodes) {
      const card = $("#node-" + n.id);
      if (card) for (const part of $$(TIER_FORM, card)) part.animate({ opacity: [0, 1] }, timing);
    }
  }
  const placeOf = (card) => ({
    x: parseFloat(card.style.left),
    y: parseFloat(card.style.top),
  });
  const drawnAt = () => ({ tier: state.tier, scale: CanvasTier.symbolScale(state.zoom) });
  function drawBodies(nodes) {
    const t = displayTime(),
      records = archive?.rows(t) || [];
    LiveViewers.draw(nodes, t, actorRate, (id) =>
      records.filter((r) => r.actor === id),
    drawnAt());
    for (const n of nodes) {
      const area = $("#node-" + n.id + " .inline-editor-fields");
      if (area) settingsRoom.observe(area);
    }
  }
  function renderActors(ids) {
    const nodes = drawCards(ids);
    if (!state.edge && ids.includes(state.selected)) renderInspector();
    return nodes;
  }
  function refreshFaces(except = []) {
    const moved = graph()
      .nodes.filter((n) => {
        const card = !except.includes(n.id) && $("#node-" + n.id);
        return Boolean(card) && drawnFaces.get(card) !== faceKey(source.face(n));
      })
      .map((n) => n.id);
    return moved.length ? renderActors(moved) : [];
  }
  function actorRate(id, t) {
    return graph()
      .edges.filter((e) => e.to === id)
      .reduce((sum, e) => sum + edgeRate(e.id, t), 0);
  }
  let lastViewerTick = -1;
  function updateViewers(t, force = false) {
    if (
      !historical() &&
      window.Product?.connection === "disconnected" &&
      !force
    )
      return;
    if (!force && t >= lastViewerTick && t - lastViewerTick < 0.2) return;
    lastViewerTick = t;
    const records = archive?.rows(t) || [];
    LiveViewers.tick(graph().nodes, t, actorRate, force, (id) =>
      records.filter((r) => r.actor === id),
    drawnAt());
  }
  function endpoints(edge, at = foldPlace) {
    const a = findNode(edge.from),
      b = findNode(edge.to);
    if (!a || !b) return null;
    const out = a.out.find((p) => p[0] === edge.out),
      inp = b.in.find((p) => p[0] === edge.in);
    if (!out || !inp) return null;
    const from = at(a),
      to = at(b);
    return {
      from: { x: from.x + a.width, y: from.y + portAt(a, out) },
      to: { x: to.x, y: to.y + portAt(b, inp) },
    };
  }
  const foldPlace = (n) => n;
  function nodeObstacles(at = foldPlace) {
    const hose = CanvasLayout.TRACK / 2;
    return graph().nodes.flatMap((n) => {
      const p = at(n);
      return [
        {
          left: p.x - 12,
          right: p.x + n.width + 12,
          top: p.y - 14,
          bottom: p.y + nodeHeight(n) + 14,
        },
        ...CardSize.captionBoxes(n, source.metrics, p, (row) => portAt(n, row)).map((b) => ({
          left: b.x - hose,
          right: b.x + b.w + hose,
          top: b.y - hose,
          bottom: b.y + b.h + hose,
          caption: true,
        })),
      ];
    });
  }
  const chipBox = (c) => {
    const room = CardSize.chipRoom(source.metrics);
    return {
      left: c.x - room.w / 2,
      right: c.x + room.w / 2,
      top: c.y - room.h / 2,
      bottom: c.y + room.h / 2,
    };
  };
  function chipsOf(e, at = foldPlace) {
    const n = findNode(e.to),
      p = n && at(n),
      dx = p ? p.x - n.x : 0,
      dy = p ? p.y - n.y : 0;
    return (e.combinators || []).map((c) =>
      dx || dy ? { ...c, x: c.x + dx, y: c.y + dy } : c,
    );
  }
  function wireRoute(e, around, chips, at) {
    const ends = endpoints(e, at);
    if (!ends) return null;
    const points = [],
      held = [];
    let from = ends.from;
    for (const c of chips) {
      const left = { x: c.x - 32, y: c.y },
        right = { x: c.x + 32, y: c.y };
      points.push(...WireGeometry.leg(from, left, around));
      held.push(left, right);
      from = right;
    }
    points.push(...WireGeometry.leg(from, ends.to, around));
    return { id: e.id, points, held, back: ends.to.x < ends.from.x };
  }
  function nearestWirePoint(id, p) {
    const line = geometry.get(id);
    if (!line) return p;
    let best = p,
      dist = Infinity;
    for (let i = 0; i < line.count; i++) {
      const x = line.points[i * 4],
        y = line.points[i * 4 + 1],
        d = (x - p.x) ** 2 + (y - p.y) ** 2;
      if (d < dist) {
        dist = d;
        best = { x, y };
      }
    }
    return best;
  }
  const wireElements = new Map(),
    componentElements = new Map(),
    wireRoutes = new Map();
  let wireLayer,
    wireLayout = null;
  function positionComponent(c, element) {
    element.body.style.left = c.x + "px";
    element.body.style.top = c.y + "px";
    element.edit.style.left = `calc(${c.x}px + 39px * var(--hf-09-symbol-scale, 1))`;
    element.edit.style.top = `calc(${c.y}px - 10px * var(--hf-09-symbol-scale, 1))`;
  }
  function renderWires(onlyIds = null) {
    if (geometryFrame) {
      cancelAnimationFrame(geometryFrame);
      geometryFrame = 0;
    }
    geometryDirty.clear();
    geometryAll = false;
    if (!wireLayer) {
      $("#wires").innerHTML = '<g transform="translate(1500 1500)"></g>';
      wireLayer = $("#wires>g");
    }
    const lines = [],
      liveEdges = new Set(),
      liveComponents = new Set(),
      travelling = travel?.moving ?? false,
      at = travelling ? (n) => travel.at(n.id) ?? n : foldPlace,
      obstacles = nodeObstacles(at),
      chips = new Map(graph().edges.map((e) => [e.id, chipsOf(e, at)]));
    const layout = travelling
      ? null
      : JSON.stringify([
          obstacles,
          graph().nodes.map((n) => [n.id, LiveViewers.select(n).kind, n.in, n.out]),
          graph().edges.map((e) => [
            e.id,
            e.from,
            e.out,
            e.to,
            e.in,
            chips.get(e.id).map((c) => [c.x, c.y]),
          ]),
        ]);
    if (!onlyIds && layout && layout === wireLayout) onlyIds = new Set();
    else wireLayout = onlyIds ? null : layout;
    const laying =
      !onlyIds ||
      onlyIds.size > 0 ||
      graph().edges.some((e) => !geometry.has(e.id));
    let laid = null,
      around = [];
    if (laying) {
      around = [...obstacles, ...[...chips.values()].flat().map(chipBox)];
      for (const e of graph().edges)
        if (!onlyIds || onlyIds.has(e.id) || !wireRoutes.has(e.id)) {
          const route = wireRoute(e, around, chips.get(e.id), at);
          if (route) wireRoutes.set(e.id, route);
          else wireRoutes.delete(e.id);
        }
      laid = WireGeometry.tracks(
        graph()
          .edges.map((e) => wireRoutes.get(e.id))
          .filter(Boolean),
        around,
        CanvasLayout.TRACK,
      );
    }
    for (const [i, e] of graph().edges.entries()) {
      liveEdges.add(e.id);
      const old = geometry.get(e.id),
        points = laid?.get(e.id),
        d = laid ? (points ? WireGeometry.rounded(points, around) : "") : old?.d;
      if (!d) continue;
      const changed = old?.d !== d,
        samples = changed ? WireGeometry.sample(d) : old;
      if (changed) geometry.set(e.id, samples);
      lines.push({ id: e.id, d, seed: i * 0.273, geometry: samples });
      let element = wireElements.get(e.id);
      if (!element) {
        const group = document.createElementNS(
          "http://www.w3.org/2000/svg",
          "g",
        );
        group.setAttribute("class", "wire-group");
        group.dataset.edge = e.id;
        group.innerHTML = `<path id="path-${e.id}" class="wire"/><path class="wire-hit" data-edge="${e.id}" tabindex="0" role="button" aria-label="Connection ${esc(e.fromName ?? e.from)} ${esc(e.out)} to ${esc(e.toName ?? e.to)} ${esc(e.in)}"/>`;
        wireLayer.append(group);
        element = { group, paths: [...group.children] };
        wireElements.set(e.id, element);
      }
      if (changed) for (const path of element.paths) path.setAttribute("d", d);
      element.group.classList.toggle("is-selected", state.edge === e.id);
      element.group.classList.toggle("is-hovered", state.hoverEdge === e.id);
      for (const c of chips.get(e.id)) {
        liveComponents.add(c.id);
        let component = componentElements.get(c.id);
        if (!component) {
          const body = document.createElement("button"),
            edit = document.createElement("button");
          body.className = "wire-component";
          body.dataset.comb = c.id;
          body.dataset.edge = e.id;
          body.innerHTML = `<span class="component-caption"></span><span class="component-terminal left"></span><span class="component-core">${icon(stepLook(c.kind).icon)}<strong>${esc(c.kind)}</strong></span><span class="component-terminal right"></span>`;
          edit.className = "component-edit-toggle";
          edit.dataset.combEdit = c.id;
          edit.dataset.combEdge = e.id;
          edit.title = "Edit combinator";
          edit.setAttribute("aria-label", `Edit ${c.kind} inline`);
          edit.innerHTML = icon("SlidersHorizontal");
          $("#wire-components").append(body, edit);
          component = { body, edit, caption: $(".component-caption", body) };
          componentElements.set(c.id, component);
        }
        const cue = componentCue(c);
        if (component.caption.textContent !== cue)
          component.caption.textContent = cue;
        component.body.setAttribute(
          "aria-label",
          `${c.kind} combinator, ${cue}`,
        );
        component.body.classList.toggle("selected", state.component === c.id);
        component.edit.classList.toggle("selected", state.component === c.id);
        positionComponent(c, component);
      }
    }
    for (const [id, element] of wireElements)
      if (!liveEdges.has(id)) {
        element.group.remove();
        wireElements.delete(id);
        geometry.delete(id);
        wireRoutes.delete(id);
      }
    for (const id of wireRoutes.keys())
      if (!liveEdges.has(id)) wireRoutes.delete(id);
    for (const [id, element] of componentElements)
      if (!liveComponents.has(id)) {
        element.body.remove();
        element.edit.remove();
        componentElements.delete(id);
      }
    field?.setLines(lines);
    renderWireActions();
    renderComponentEditor();
  }
  function refreshWireSelection() {
    $$(".wire-group").forEach((g) => {
      g.classList.toggle("is-selected", g.dataset.edge === state.edge);
      g.classList.toggle("is-hovered", g.dataset.edge === state.hoverEdge);
    });
    $$(".wire-component").forEach((c) =>
      c.classList.toggle("selected", c.dataset.comb === state.component),
    );
    $$(".component-edit-toggle").forEach((c) =>
      c.classList.toggle("selected", c.dataset.combEdit === state.component),
    );
  }
  function showMailboxes(rows) {
    field?.setMailboxes(
      (rows ?? []).map(({ wire, depth, queued = 0, capacity = null }) => ({
        wire,
        depth,
        queued,
        capacity,
        backpressure:
          findNode(graph().edges.find((e) => e.id === wire)?.to)?.health ===
          "backpressure",
      })),
    );
  }
  function renderWireActions() {
    if (historical()) {
      $("#wire-actions").classList.add("hidden");
      return;
    }
    const box = $("#wire-actions"),
      e = graph().edges.find((e) => e.id === state.edge);
    if (!e || drag?.moved || state.editComponent) {
      box.classList.add("hidden");
      return;
    }
    const line = geometry.get(e.id);
    if (!line) return;
    const p = state.edgeAnchor || {
      x: line.points[Math.floor(line.count * 0.6) * 4],
      y: line.points[Math.floor(line.count * 0.6) * 4 + 1],
    };
    box.classList.remove("hidden");
    box.innerHTML = `<button id="insert-combinator">${icon("Plus")}Combinator</button>`;
    const w = box.offsetWidth,
      h = box.offsetHeight,
      around = [
        ...nodeObstacles(),
        ...graph().edges.flatMap((wire) => chipsOf(wire)).map(chipBox),
      ],
      clear = (x, y) =>
        around.every(
          (o) =>
            x + w / 2 <= o.left ||
            x - w / 2 >= o.right ||
            y + 17 + h <= o.top ||
            y + 17 >= o.bottom,
        );
    let at = p;
    if (!clear(p.x, p.y))
      for (let i = 0, nearest = Infinity; i < line.count; i++) {
        const x = line.points[i * 4],
          y = line.points[i * 4 + 1],
          d = (x - p.x) ** 2 + (y - p.y) ** 2;
        if (d < nearest && clear(x, y)) {
          nearest = d;
          at = { x, y };
        }
      }
    box.style.left = at.x + "px";
    box.style.top = at.y + 17 + "px";
  }
  function openCombinatorMenu() {
    openPalette("combinators");
  }
  function addCombinator(kind) {
    return source.perform({
      kind: "addStep",
      edge: state.edge,
      step: kind,
      at: state.edgeAnchor || { x: 300, y: 350 },
    });
  }
  function showProbe(id, x, y) {
    if (drag) return;
    probePosition = { x, y };
    if (state.hoverEdge === id) {
      positionProbe();
      return;
    }
    clearTimeout(probeTimer);
    state.hoverEdge = id;
    refreshWireSelection();
    probeTimer = setTimeout(() => {
      if (state.hoverEdge !== id) return;
      const e = graph().edges.find((e) => e.id === id);
      if (!e) return;
      probeLast = 0;
      const value = edgeValue(e);
      const delay = e.declaredDelay == null ? null : LiveViewers.formatDuration(e.declaredDelay);
      $("#wire-probe").innerHTML =
        `<header><span class="tiny-dot green"></span><strong>${esc(e.fromName ?? e.from)} → ${esc(e.toName ?? e.to)}</strong></header><div class="probe-metrics"><strong id="probe-rate"></strong><span>events/s</span><small>${delay === null ? `<span data-reason="UNDECLARED">${esc(source.codeLabel("UNDECLARED").label)}</span>` : `<span title="${esc(delay.title)}">${esc(delay.text)}</span> delay`}</small></div><small id="probe-reason" hidden></small><canvas id="probe-spark" width="400" height="78" aria-label="Event density over the last six seconds"></canvas><div class="probe-value"><span>LAST VALUE</span><span data-probe-value title="${esc(value ?? "")}">${esc(value === undefined ? "—" : value)}</span></div><footer><span>Type</span><code>${esc(findNode(e.from).out.find((p) => p[0] === e.out)?.[1] || "Any")}</code></footer>`;
      $("#wire-probe").classList.remove("hidden");
      updateProbe(displayTime(), true);
    }, 160);
  }
  function positionProbe() {
    const r = $("#canvas").getBoundingClientRect(),
      box = $("#wire-probe");
    box.style.left =
      Math.max(10, Math.min(r.width - box.offsetWidth - 10, probePosition.x - r.left + 18)) +
      "px";
    box.style.top =
      Math.max(10, Math.min(r.height - box.offsetHeight - 10, probePosition.y - r.top + 20)) +
      "px";
  }
  function hideProbe() {
    clearTimeout(probeTimer);
    state.hoverEdge = null;
    $("#wire-probe").classList.add("hidden");
    refreshWireSelection();
  }
  function updateProbe(t, force = false) {
    const probe = $("#wire-probe");
    if (probe.classList.contains("hidden") || (!force && t >= probeLast && t - probeLast < 0.1))
      return;
    probeLast = t;
    const e = graph().edges.find((e) => e.id === state.hoverEdge);
    if (!e) return;
    const reading = LiveViewers.formatReading(source.rate(e, t), { fixed: true, label: "events / s" });
    const rate = $("#probe-rate"), reason = $("#probe-reason");
    const unread = reading ? null : source.codeLabel("DENSITY_UNREAD");
    rate.textContent = reading?.text ?? "—";
    rate.title = reading?.title ?? unread.label;
    if (unread) rate.dataset.reason = unread.code;
    else delete rate.dataset.reason;
    reason.textContent = unread?.label ?? "";
    reason.hidden = !unread;
    const history = Array.from({ length: 60 }, (_, i) => source.rate(e, t - (59 - i) * 0.1));
    LiveViewers.drawSpark($("#probe-spark"), history, window.StudyPaint);
    positionProbe();
  }
  function selectNode(id, additive = false) {
    const before = new Set(state.selectedSet);
    if (additive) {
      state.selectedSet.has(id)
        ? state.selectedSet.delete(id)
        : state.selectedSet.add(id);
    } else if (!state.selectedSet.has(id)) state.selectedSet = new Set([id]);
    state.editComponent = null;
    renderComponentEditor();
    state.selected = id;
    state.edge = null;
    state.component = null;
    state.selectedRecord = null;
    state.tab = "inspect";
    drawSelection(before);
    $(".workspace").classList.add("mobile-inspecting");
    refreshWireSelection();
    renderWireActions();
    renderInspector();
    renderJournal();
    window.Product?.afterSelection();
  }
  function selectEdge(id, point = null, component = null) {
    const before = new Set(state.selectedSet);
    state.selectedSet.clear();
    state.component = component;
    state.edgeAnchor = point ? nearestWirePoint(id, point) : null;
    state.edge = id;
    state.selected = null;
    state.selectedRecord = null;
    state.tab = "inspect";
    $(".workspace").classList.add("mobile-inspecting");
    drawSelection(before);
    refreshWireSelection();
    renderWireActions();
    renderInspector();
    renderJournal();
    window.Product?.afterSelection();
  }
  function drawSelection(before) {
    drawCards(
      graph()
        .nodes.filter((n) => before.has(n.id) !== state.selectedSet.has(n.id))
        .map((n) => n.id),
    );
  }
  const heading = (title, right = "") =>
    `<div class="detail-heading"><h3>${title}</h3>${right}</div>`;
  const pretty = (value) => JSON.stringify(value, null, 2);
  const sdkText = (n) => source.sdkText(n);
  function inspectorFace() {
    return JSON.stringify(
      state.edge
        ? ["wire", state.edge, state.component ?? null]
        : [
            "actor",
            state.selected ?? null,
            state.tab,
            state.selectedRecord
              ? [state.selectedRecord.actor, state.selectedRecord.index]
              : null,
          ],
    );
  }
  function renderInspector() {
    const held = $("#inspector"),
      face = inspectorFace();
    if (held.dataset.face !== face) {
      held.dataset.face = face;
      drawInspector();
      return;
    }
    const next = held.cloneNode(false),
      shelf = document.createElement("div");
    shelf.hidden = true;
    shelf.style.display = "none";
    shelf.append(next);
    held.before(shelf);
    try {
      drawInspector();
      syncNodeElement(held, next);
    } finally {
      shelf.remove();
    }
    const form = $("#config-form", held);
    if (form && !viewer.card(state.selected).draft) form.reset();
  }
  function drawInspector() {
    const box = $("#inspector");
    if (state.edge) {
      renderEdgeInspector();
      window.Product?.afterEdgeInspector();
      syncHistoryControls();
      return;
    }
    const n = selectedNode();
    if (!n) {
      box.innerHTML = LiveViewers.empty("INSPECTOR_EMPTY", "", "inspector");
      return;
    }
    box.innerHTML = `<header class="inspector-head"><div class="inspector-eyebrow">ACTOR <button class="icon-button" id="close-inspector" aria-label="Close inspector">${icon("X")}</button></div><div class="inspector-identity"><span class="node-icon">${icon(n.icon)}</span><div><h2>${esc(n.title)}</h2><div class="node-type">${esc(source.identityLine(n))}</div></div></div></header><nav class="inspector-tabs" aria-label="Actor inspector tabs"><button data-tab="inspect" class="${state.tab === "inspect" ? "active" : ""}">Inspect</button><button data-tab="configure" class="${state.tab === "configure" ? "active" : ""}">Configure</button><button data-tab="sdk" class="${state.tab === "sdk" ? "active" : ""}">SDK ${icon("Code2")}</button></nav><div class="inspector-content">${state.tab === "inspect" ? window.Product.inspectContent(n) : state.tab === "configure" ? configContent(n) : sdkContent(n)}</div><footer class="inspector-footer">${icon("CheckCheck")}${observationsSpan()}<button id="inspect-records">View records</button></footer>`;
    syncConfigActions();
    syncHistoryControls();
    window.Product?.afterInspector(n);
  }
  function observationsSpan() {
    const observations = source.inspectorObservations();
    return `<span${observations.code == null ? "" : ` data-reason="${esc(observations.code)}"`}>${esc(observations.label)}</span>`;
  }
  function renderInspectorFooter() {
    const span = $("#inspector .inspector-footer > span");
    if (span) span.outerHTML = observationsSpan();
  }
  function configContent(node) {
    const n = {
      ...node,
      config: historical()
        ? node.config
        : viewer.card(node.id).draft?.shown || node.config,
    };
    return Product.configContent(node, n.config);
  }
  function changedKeys(n) {
    if (historical()) return [];
    const draft = viewer.card(n.id).draft?.shown;
    return draft
      ? Object.keys(draft).filter(
          (k) => JSON.stringify(draft[k]) !== JSON.stringify(n.config[k]),
        )
      : [];
  }
  function drafted(n) {
    if (historical()) return false;
    const raw = viewer.card(n.id).draft?.raw || {},
      typed = (value) =>
        typeof value === "object"
          ? JSON.stringify(value, null, 2)
          : String(value ?? "");
    return (
      changedKeys(n).length > 0 ||
      Object.entries(raw).some(([name, text]) => text !== typed(n.config[name]))
    );
  }
  function syncConfigActions() {
    const n = selectedNode(),
      form = $("#config-form");
    if (!n || !form) return;
    const keys = changedKeys(n);
    $("#config-change-summary").innerHTML = keys.length
      ? `<strong>${keys.length} unsaved ${keys.length === 1 ? "change" : "changes"}</strong><span>${keys.map(esc).join(" · ")}</span>`
      : "";
    $(".config-apply", form).disabled =
      !drafted(n) || viewer.card(n.id).submission?.status === "submitting";
    $("#discard-config").disabled = !keys.length;
    $$("[name]", form).forEach((input) =>
      input
        .closest(".config-field")
        ?.classList.toggle("is-changed", keys.includes(input.name)),
    );
  }
  function captureDraft(form) {
    const n = form.dataset.nodeConfig
      ? findNode(form.dataset.nodeConfig)
      : selectedNode();
    if (!n) return;
    return Product.captureDraft(form, n);
  }
  function sdkContent(n) {
    const notice = (code, diagnostic) => {
      const value = source.codeLabel(code);
      const label = value.code === 'UNRECOGNIZED_REASON' ? source.codeLabel('SDK_PROGRAM_UNAVAILABLE').label : value.label;
      return `<p data-reason="${esc(value.code === 'UNRECOGNIZED_REASON' ? code : value.code)}"${diagnostic ? ` data-diagnostic-code="${esc(diagnostic.code)}"` : ''}>${esc(label)}</p>`;
    };
    try {
      const program = source.sdkProgram(n);
      if (program.pending) return '<p class="subtle-note">Loading SDK program…</p>';
      if (program.diagnostics.length) {
        return `<section class="detail-section">${program.diagnostics.map(d => notice(d.reason ?? d.code, d)).join('')}</section>`;
      }
      const runs = (spans) => spans.reduce((all, span) => {
        const last = all[all.length - 1];
        if (last?.module === span.module && last.owner === span.owner) last.code.push(span.code);
        else all.push({ module: span.module, owner: span.owner, code: [span.code] });
        return all;
      }, []);
      const block = (run) => `<pre class="code-block">${run.code.map(esc).join('\n')}</pre>`;
      const own = program.spans.filter((span) => span.own),
        named = program.spans.filter((span) => !span.own);
      const ownHTML = runs(own).map((run, index) => `<div class="code-title"><span>${esc(run.module)}</span>${index ? '' : `<button class="code-copy" id="copy-sdk">${icon("Copy")}Copy</button>`}</div>${block(run)}`).join('');
      const namedHTML = runs(named).map((run) => `<div class="code-title" data-sdk-owner="${esc(run.owner)}"><span>→ ${esc(run.owner)}</span><span>${esc(run.module)}</span></div>${block(run)}`).join('');
      const note = own.length
        ? `Lines of ${[...new Set(own.map((span) => span.module))].join(' and ')} that declare ${own[0].owner}. Wires into other actors are shown under the actor they flow into.`
        : 'No line of the SDK program declares this actor.';
      return `${own.length ? `<section class="detail-section" data-sdk-own>${ownHTML}</section>` : ''}${named.length ? `<section class="detail-section" data-sdk-references style="opacity:.55">${namedHTML}</section>` : ''}<p class="subtle-note">${esc(note)}</p>`;
    } catch (error) {
      console.error('SDK program unavailable', error);
      return notice(error.code ?? 'SDK_PROGRAM_UNAVAILABLE');
    }
  }
  function renderEdgeInspector() {
    const e = graph().edges.find((e) => e.id === state.edge);
    if (!e) return;
    const a = findNode(e.from),
      b = findNode(e.to),
      { steps, draft, active } = inletSteps(e);
    const caption =
      active === draft
        ? `<span>${esc(draft.kind)} · draft, not applied</span><button data-close-comb-editor="1" aria-label="Discard ${esc(draft.kind)} draft">Discard</button>`
        : active
          ? `<span>${esc(active.kind)} · ${esc(componentCue(active))}</span><button data-remove-comb="${active.id}" aria-label="Remove ${esc(active.kind)}">Remove</button>`
          : "";
    const rows = steps
      .map(
        (c, i) =>
          `<button class="processing-row ${c === active ? "active" : ""}" data-edge="${e.id}" data-comb="${c.id}"><span>${String(i + 1).padStart(2, "0")}</span>${icon(stepLook(c.kind).icon)}<strong>${esc(c.kind)}</strong><small>${esc(componentCue(c))}</small></button>`,
      )
      .concat(
        draft
          ? [
              `<button class="processing-row draft ${draft === active ? "active" : ""}" data-edge="${e.id}" data-comb="${draft.id}"><span>${String(steps.length + 1).padStart(2, "0")}</span>${icon(stepLook(draft.kind).icon)}<strong>${esc(draft.kind)}</strong><small>draft · not applied</small></button>`,
            ]
          : [],
      )
      .join("");
    const delay = e.declaredDelay == null ? null : LiveViewers.formatDuration(e.declaredDelay);
    const editor = active
      ? `<section class="detail-section"><div class="processing-caption">${caption}</div><form id="preprocess-form" data-component="${active.id}">${["map", "filter"].includes(active.kind) ? `<label class="config-field"><span class="config-label">Expression</span><textarea name="expression" aria-label="Preprocessing expression" spellcheck="false">${esc(active.expression ?? "")}</textarea></label>` : active.kind === "parse" ? `<label class="config-field"><span class="config-label">Decoder</span><select name="expression"><option>JSON</option></select></label>` : `<input name="expression" type="hidden" value="null">`}<button class="quiet-button" type="submit">${icon("Check")}Apply to inlet</button></form></section>`
      : "";
    $("#inspector").innerHTML =
      `<header class="inspector-head"><div class="inspector-eyebrow">WIRE <button id="close-inspector" class="icon-button" aria-label="Close inspector">${icon("X")}</button></div><div class="inspector-identity"><span class="node-icon">${icon("GitBranch")}</span><div><h2>${esc(b.title)}</h2><div class="node-type">in.${esc(e.in)}</div></div></div><div class="wire-inspector-subtitle">${esc(e.fromName ?? a.id)}.${esc(e.out)} → ${esc(e.toName ?? b.id)}.${esc(e.in)}</div></header><div class="inspector-content"><div class="wire-summary"><span>Event stream</span>${delay === null ? `<span data-reason="UNDECLARED">${esc(source.codeLabel("UNDECLARED").label)}</span>` : `<span title="${esc(delay.title)}">${esc(delay.text)} delay</span>`}</div><section class="detail-section">${heading("Inlet processing")}${rows ? `<div class="processing-list">${rows}</div>` : `<div class="empty-processing" data-reason="INLET_PROCESSING_EMPTY">${esc(source.codeLabel("INLET_PROCESSING_EMPTY").label)}</div>`}<button class="quiet-button processing-add" id="add-inlet-processing">${icon("Plus")}Add combinator</button></section>${editor}<button class="quiet-button danger-button" id="disconnect-wire">${icon("Trash2")}Disconnect</button></div>`;
  }
  function inletSteps(e) {
    const steps = e?.combinators || [],
      draft =
        state.draftStep &&
        state.draftStep.edge === e?.id &&
        state.editComponent === state.draftStep.id
          ? state.draftStep
          : null;
    return {
      steps,
      draft,
      active: steps.find((c) => c.id === state.component) ?? draft ?? steps[0],
    };
  }
  function closeComponentEditor() {
    const draft =
      Boolean(state.draftStep) && state.editComponent === state.draftStep.id;
    state.editComponent = null;
    renderComponentEditor();
    if (draft) renderInspector();
  }
  function renderJournal() {
    let rows = visibleRecords().filter((r) =>
      graph().nodes.some((n) => n.id === r.actor),
    );
    let empty = "JOURNAL_EMPTY", selected;
    if (state.journal === "selected") {
      selected = state.edge
        ? [graph().edges.find((e) => e.id === state.edge)?.from, graph().edges.find((e) => e.id === state.edge)?.to].filter(Boolean)
        : [state.selected].filter(Boolean);
      const own = source.selectionRecords(selected);
      rows = own
        ? own.rows.filter((r) => r.at <= displayTime())
        : rows.filter((r) => selected.includes(r.actor));
      if (own?.code) empty = own.code;
      else if (own?.pending && !rows.length) empty = "SELECTION_RECORDS_PENDING";
    }
    const edits = source.acceptedRecords?.(state.scope, selected) ?? [];
    $("#journal-count").textContent = rows.length + edits.length;
    rows = [...edits, ...rows.slice(0, 100)];
    const from = Math.min(...rows.filter((r) => !r.timeReason && r.time !== "—" && Number.isFinite(r.at)).map((r) => r.at));
    if (!$("#journal").classList.contains("collapsed"))
      $("#journal-rows").innerHTML = rows.length
        ? rows
            .map(
              (r) =>
                `<button class="journal-row ${state.selectedRecord === r ? "selected" : ""}" ${r.commitId ? `data-authoring-commit="${esc(r.id)}"` : `data-record="${esc(r.actor)}:${esc(r.index)}"`}><span>${icon(r.commitId ? "Check" : allNode(r.actor)?.icon || "Box")}${esc(r.actorName ?? r.actor)} <small>${r.commitId ? "Accepted edit" : `#${esc(r.index)}`}</small></span><span class="event-kind" title="${esc(r.eventName ?? r.event)}">${esc(r.eventName ?? r.event)}</span><span title="${esc(r.detail)}">${esc(r.detail)}</span><span${r.timeReason ? ` data-reason="${esc(r.timeReason)}" title="${esc(source.codeLabel(r.timeReason).label)}"` : r.time === "—" ? "" : ` title="${esc(studyTimeTitle(r.at))}"`}>${esc(r.timeReason ? source.codeLabel(r.timeReason).label : r.time === "—" ? r.time : studyTimeFormat(r.at, true, from))}</span></button>`,
            )
            .join("")
        : `<div class="journal-empty" data-reason="${esc(source.codeLabel(empty).code)}">${esc(source.codeLabel(empty).label)}</div>`;
    $$(".journal-tabs button").forEach((b) =>
      b.classList.toggle("active", b.dataset.journal === state.journal),
    );
  }
  function applyConfig(form) {
    const n = form.dataset.nodeConfig
      ? findNode(form.dataset.nodeConfig)
      : selectedNode();
    if (!n) return;
    return Product.applyConfig(form, n);
  }
  function setScope(scope) {
    if (!(state.historyGraph || source.projection)[scope]?.nodes) return;
    state.cameras.set(state.scope, {
      x: state.x,
      y: state.y,
      zoom: state.zoom,
    });
    window.Product?.scopeVisited(scope);
    state.editComponent = null;
    clearPlacement();
    state.scope = scope;
    state.selected = null;
    state.selectedSet = new Set();
    state.edge = null;
    state.tab = "inspect";
    state.selectedRecord = null;
    state.pendingPort = null;
    $(".workspace").classList.remove("mobile-inspecting");
    renderGraph();
    if (!state.cameras.has(scope)) return oweFit([scope]);
    Object.assign(state, state.cameras.get(scope));
    transformWorld();
    settleFit();
  }
  function oweFit(scopes) {
    for (const scope of scopes)
      if (!state.cameraTaken.has(scope)) state.fitOwed.add(scope);
    settleFit();
  }
  function settleFit() {
    if (state.view !== "canvas" || !state.fitOwed.has(state.scope)) return;
    const box = $("#canvas").getBoundingClientRect();
    if (!box.width || !box.height) return;
    state.fitOwed.delete(state.scope);
    fitCanvas();
  }
  function takeCamera() {
    state.cameraTaken.add(state.scope);
    state.fitOwed.delete(state.scope);
  }
  function fitCanvas() {
    const to = fitCamera();
    if (!to) return;
    Object.assign(state, to);
    transformWorld();
  }
  function fitCamera() {
    const rect = $("#canvas").getBoundingClientRect(),
      tools = $(".canvas-tools")?.getBoundingClientRect(),
      floor = tools?.height ? tools.top - rect.top : rect.height;
    if (rect.width <= 28 || floor <= 28) return null;
    const cards = graph().nodes.map((n) => ({
      x: n.x, y: n.y, width: n.width, name: n.title,
      height: document.getElementById("node-" + n.id)?.offsetHeight || 200,
    }));
    const boxes = [...cards];
    for (const note of $$("#annotations .annotation"))
      boxes.push({ x: note.offsetLeft, y: note.offsetTop, width: note.offsetWidth, height: note.offsetHeight });
    for (const path of $$("#wires .wire")) boxes.push(path.getBBox());
    if (!boxes.length) return null;
    const left = Math.min(...boxes.map((b) => b.x)),
      top = Math.min(...boxes.map((b) => b.y)),
      width = Math.max(...boxes.map((b) => b.x + b.width)) + 45 - left,
      tall = Math.max(...boxes.map((b) => b.y + b.height)) + 32 - top;
    const zoom = Math.max(Math.min((rect.width - 28) / width, (floor - 28) / tall, ZOOM.fit),
      CardSize.nameFloor(cards, source.metrics));
    return { zoom, x: (rect.width - width * zoom) / 2 - left * zoom, y: (floor - tall * zoom) / 2 - top * zoom };
  }
  const ZOOM = { min: CanvasTier.leastZoom("reduced"), max: 2, fit: 1.13, step: 1.15, wheel: 1 / 100 };
  const WHEEL_LINE = 40;
  function wheelPixels(e) {
    const unit = e.deltaMode === 1 ? WHEEL_LINE : e.deltaMode === 2 ? $("#canvas").clientHeight : 1;
    return { x: e.deltaX * unit, y: e.deltaY * unit };
  }
  const SCROLL_AXES = {
    x: ["scrollLeft", "scrollWidth", "clientWidth", "offsetWidth", "width", "overflowX"],
    y: ["scrollTop", "scrollHeight", "clientHeight", "offsetHeight", "height", "overflowY"],
  };
  function scrollPart(at, axis, pixels) {
    const [offset, whole, shown, own, drawn, overflow] = SCROLL_AXES[axis];
    if (!pixels || at[whole] <= at[shown] || !/auto|scroll/.test(getComputedStyle(at)[overflow])) return 0;
    const scale = at[own] ? at.getBoundingClientRect()[drawn] / at[own] : 1,
      asked = pixels / scale,
      by = Math.max(-at[offset], Math.min(asked, Math.max(0, at[whole] - at[shown] - at[offset])));
    at[offset] += by;
    return by === asked ? pixels : by * scale;
  }
  let bodyScale = CanvasTier.symbolScale(state.zoom);
  function transformWorld() {
    $("#world").style.transform =
      `translate(${state.x}px,${state.y}px) scale(${state.zoom})`;
    placeComponentEditor();
    $("#zoom-label").textContent = Math.round(state.zoom * 100) + "%";
    const tier = CanvasTier.tierAt(state.zoom, state.tier),
      moved = tier !== state.tier,
      scale = CanvasTier.symbolScale(state.zoom),
      rescaled = !glide && scale !== bodyScale;
    state.tier = tier;
    if (!glide) bodyScale = scale;
    $("#canvas").setAttribute("data-tier", tier);
    $("#canvas").style.setProperty("--card-title-lines", CanvasTier.TITLE_LINES[tier] ?? "none");
    $("#world").style.setProperty("--hf-09-symbol-scale", scale);
    $("#zoom-label").title = CanvasTier.TIER_TITLE[tier];
    field?.wake();
    const again = graph().nodes.filter((n) => moved
      || CanvasTier.holdsControl({ width: n.width, height: nodeHeight(n) }, state.zoom, state.controlRoom?.[tier])
        === ($("#node-" + n.id)?.getAttribute("data-places") === "none"));
    if (again.length) drawCards(again.map((n) => n.id), moved);
    if (rescaled) {
      const drawn = new Set(again.map((n) => n.id)),
        rest = graph().nodes.filter((n) => !drawn.has(n.id));
      if (rest.length) drawBodies(rest);
    }
  }
  const FRAME = 1000 / 60;
  let glide = null;
  function placeComponentEditor() {
    const box = $("#component-editor"),
      anchor = box.anchor;
    if (!anchor || box.classList.contains("hidden")) return;
    const r = $("#canvas").getBoundingClientRect(),
      tools = $(".canvas-tools")?.getBoundingClientRect(),
      floor = tools?.height ? tools.top - r.top : r.height,
      z = state.zoom,
      margin = 8 / z;
    const inside = (at, low, high, size) =>
      Math.max(low + margin, Math.min(at, high - size - margin));
    box.style.left =
      inside(anchor.x, -state.x / z, (r.width - state.x) / z, box.offsetWidth) + "px";
    box.style.top =
      inside(anchor.y, -state.y / z, (floor - state.y) / z, box.offsetHeight) + "px";
  }
  const zoomWithin = (zoom, old) => Math.min(ZOOM.max, Math.max(Math.min(ZOOM.min, old), zoom));
  function zoomTo(zoom, anchor) {
    const r = $("#canvas").getBoundingClientRect(),
      old = state.zoom;
    state.zoom = zoomWithin(zoom, old);
    const cx = anchor ? anchor.x - r.left : r.width / 2,
      cy = anchor ? anchor.y - r.top : r.height / 2;
    state.x = cx - ((cx - state.x) * state.zoom) / old;
    state.y = cy - ((cy - state.y) * state.zoom) / old;
    transformWorld();
  }
  function zoomBy(factor, anchor) {
    zoomTo(state.zoom * factor, anchor);
  }
  const aimed = () => (glide ? { ...glide.to } : { zoom: state.zoom, x: state.x, y: state.y });
  function glideBy(factor, anchor) {
    const r = $("#canvas").getBoundingClientRect(),
      from = aimed(),
      zoom = zoomWithin(from.zoom * factor, from.zoom),
      cx = anchor ? anchor.x - r.left : r.width / 2,
      cy = anchor ? anchor.y - r.top : r.height / 2;
    glideTo({ zoom, x: cx - ((cx - from.x) * zoom) / from.zoom, y: cy - ((cy - from.y) * zoom) / from.zoom });
  }
  function glideTo(to) {
    if (!to) return;
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) {
      glide = null;
      Object.assign(state, to);
      transformWorld();
      return;
    }
    if (glide) {
      glide.to = { ...to };
      return;
    }
    const tau = parseFloat(getComputedStyle(document.documentElement).getPropertyValue("--hf-10-overlay")) / 3;
    const own = (glide = { to: { ...to }, tau: tau || 0, at: undefined, camera: { zoom: state.zoom, x: state.x, y: state.y } });
    const frame = (now) => {
      if (glide !== own) return;
      const was = own.camera;
      if (was.zoom !== state.zoom || was.x !== state.x || was.y !== state.y) {
        glide = null;
        transformWorld();
        return;
      }
      const k = 1 - Math.exp(-(own.at === undefined ? FRAME : Math.min(Math.max(now - own.at, 0), FRAME)) / own.tau),
        { zoom, x, y } = own.to,
        r = zoom / state.zoom,
        rk = r ** k,
        q = Math.abs(1 - r) < 1e-12 ? k : (1 - rk) / (1 - r),
        next = { zoom: state.zoom * rk, x: (x - state.x * r) * q + state.x * rk, y: (y - state.y * r) * q + state.y * rk },
        box = $("#canvas").getBoundingClientRect(),
        off = Math.max(...[[0, 0], [box.width, 0], [0, box.height], [box.width, box.height]].map(([px, py]) =>
          Math.hypot((px - next.x) * (zoom / next.zoom - 1) + x - next.x, (py - next.y) * (zoom / next.zoom - 1) + y - next.y)));
      own.at = now;
      const done = off < 0.5;
      Object.assign(state, done ? own.to : next);
      if (done) glide = null;
      transformWorld();
      own.camera = { zoom: state.zoom, x: state.x, y: state.y };
      if (!done) requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
  }
  function setTool(tool) {
    state.tool = tool;
    $("#select-tool").classList.toggle("active", tool === "select");
    $("#pan-tool").classList.toggle("active", tool === "pan");
    $("#canvas").classList.toggle("pan", tool === "pan" || state.space);
  }
  function syncHistoryControls() {
    const readOnly = historical();
    $(".workspace").classList.toggle("history-view", readOnly);
    for (const [selector, how] of [
      ["#add-actor", "toggle"],
      ["#new-actor-sidebar", "toggle"],
      ["#pause-menu-button", "toggle"],
      ['.canvas-edit-tools [data-action="group"]', "toggle"],
      ['.canvas-edit-tools [data-action="note"]', "toggle"],
      ["#inspector form input", "disabled"],
      ["#inspector form select", "disabled"],
      ["#inspector form textarea", "disabled"],
      ["#inspector form button", "disabled"],
      ["#disconnect-wire", "disabled"],
      ["#add-inlet-processing", "disabled"],
      ["[data-add-comb]", "disabled"],
      ["[data-remove-comb]", "disabled"],
      ["[data-comb-edit]", "disabled"],
      ["[data-delete-note]", "disabled"],
    ])
      if (how === "toggle") $(selector).disabled = readOnly;
      else if (readOnly) $$(selector).forEach((el) => (el.disabled = true));
    $("#history-caption").hidden = !readOnly;
    renderJournalHeader();
  }
  function renderJournalHeader() {
    const header = $(".journal-follow"), past = historical(), line = source.journalLine(past);
    header.innerHTML = `${past ? icon("History") : `<span class="tiny-dot" data-health="${line.live === false ? "unobserved" : "alive"}"></span>`} ${line.code ? `<span data-reason="${esc(line.code)}">${esc(line.label)}</span>` : esc(line.label)}`;
    header.title = line.title;
  }
  let historyFrame = null,
    lastHistoryTime = null;
  function showRecordedTime(t, mode, seek) {
    const readOnly = mode !== "live";
    const frame = readOnly ? archive.frame(t) : null;
    const changed =
      frame !== historyFrame || Boolean(state.historyGraph) !== readOnly;
    if (seek) {
      hideProbe();
      state.selectedRecord = null;
      state.editComponent = null;
      renderComponentEditor();
      renderWireActions();
      clearPlacement();
      $("#pause-menu").classList.add("hidden");
    }
    if (changed) {
      const previous = graph();
      const wasHistorical = Boolean(state.historyGraph);
      historyFrame = frame;
      state.historyGraph = frame ? structuredClone(frame.scopes) : null;
      if (state.selected && !findNode(state.selected)) state.selected = null;
      if (state.edge && !graph().edges.some((e) => e.id === state.edge))
        state.edge = null;
      const shape = (g) =>
        JSON.stringify({
          nodes: g.nodes.map(
            ({ preview, activity, config, interactive, ...rest }) => rest,
          ),
          edges: g.edges,
        });
      if (!wasHistorical || !readOnly || shape(previous) !== shape(graph()))
        renderGraph();
      else {
        renderActors(
          graph()
            .nodes.filter(
              (node) =>
                JSON.stringify(previous.nodes.find((n) => n.id === node.id)) !==
                JSON.stringify(node),
            )
            .map((node) => node.id),
        );
        updateViewers(t, true);
      }
    }
    if (seek) {
      field.seek(t);
      LiveViewers.reset();
      updateViewers(t, true);
      field.wake();
    } else if (mode === "replay") {
      updateViewers(t);
      field.wake();
    }
    if (changed || seek || t !== lastHistoryTime) {
      lastHistoryTime = t;
      renderJournal();
    }
    if ((changed || seek) && state.tab === "inspect") renderInspector();
    syncHistoryControls();
    updatePause();
    $("#history-caption-label").textContent =
      mode === "replay" ? "Replaying" : "Viewing history";
    refreshFaces();
  }
  const updateStatusbar = () => source.updateStatusbar();
  function updateScopeHealth() {
    const value = historical()
      ? { state: "history", text: timeMachine.mode === "replay" ? "Replay" : "History" }
      : source.scopeHealth(state.scope);
    const label = $("#scope-health");
    label.dataset.health = value.state;
    if (value.code) label.dataset.reason = value.code;
    else delete label.dataset.reason;
    label.title = value.title ?? value.text;
    label.innerHTML = `${esc(value.text)}<span class="live-dot"></span>`;
  }
  function updatePause() {
    updateStatusbar();
    updateScopeHealth();
    if (historical()) {
      const replay = timeMachine.mode === "replay";
      $("#canvas").classList.remove("paused-canvas");
      $("#pause-button").innerHTML =
        icon(replay ? "Pause" : "Play") +
        `<span>${replay ? "Pause replay" : "Resume"}</span>`;
      $("#pause-button").title = replay
        ? "Pause replay"
        : "Resume from the selected time";
      return;
    }
    $("#canvas").classList.toggle("paused-canvas", paused());
    $("#pause-button").innerHTML =
      icon(paused() ? "Play" : "Pause") +
      `<span>${paused() ? "Resume" : "Pause"}</span>`;
    $("#pause-button").title = (paused() ? "Resume" : "Pause") + " the whole pipeline";
  }
  function togglePause(force = false) {
    $("#pause-menu").classList.add("hidden");
    return source.perform({ kind: "togglePause", force });
  }
  function drawPrompt(id) {
    drawCards([id]);
    const live = Object.values(liveScopes())
      .flatMap((g) => g.nodes)
      .find((n) => n.id === id);
    if (live) window.Product?.refreshOutputNodes?.([live]);
  }
  function typePrompt(box) {
    viewer.message(box.dataset.prompt, box.value);
    drawPrompt(box.dataset.prompt);
  }
  function sendPrompt(button) {
    const id = button.dataset.send,
      box = $(".prompt-input", button.closest(".node, .output-content")),
      text = viewer.card(id).message ?? "";
    if (!text.trim()) {
      box?.focus();
      return;
    }
    return Promise.resolve(
      source.perform({ kind: "inject", actor: id, entered: { text }, line: null }),
    ).then((sent) => {
      if (sent !== true) return;
      viewer.message(id, undefined);
      if (box) box.value = "";
      drawPrompt(id);
    });
  }
  function openPalette(mode = "actors") {
    if (historical()) {
      gestureCode("EDIT_UNAVAILABLE");
      return;
    }
    hideProbe();
    state.paletteMode = mode;
    const p = $("#palette");
    const canvas = $("#canvas").getBoundingClientRect(),
      point = state.connectionAt || state.addAt ||
        state.cursorWorld || {
          x: (canvas.width / 2 - state.x) / state.zoom,
          y: (canvas.height * 0.3 - state.y) / state.zoom,
        };
    state.addAt = point;
    paletteAnchor = {
      x: canvas.left + state.x + point.x * state.zoom,
      y: canvas.top + state.y + point.y * state.zoom - 35,
    };
    p.returnValue = "";
    if (!p.open) p.showModal();
    $("#palette-input").value = "";
    renderPalette("");
    placePalette();
    $("#palette-input").focus();
  }
  let paletteAnchor = { x: 0, y: 0 };
  function placePalette() {
    const p = $("#palette");
    p.style.maxHeight = "";
    const top = Math.max(65, Math.min(innerHeight - p.offsetHeight - 12, paletteAnchor.y));
    p.style.left = Math.max(12, Math.min(innerWidth - p.offsetWidth - 12, paletteAnchor.x)) + "px";
    p.style.top = top + "px";
    p.style.maxHeight = innerHeight - top - 12 + "px";
  }
  function renderPalette(query) {
    const q = query.toLowerCase().trim(),
      mode = state.paletteMode;
    $("#palette-tabs").innerHTML = [
      ["actors", "Actors"],
      ["combinators", "Combinators"],
      ["find", "In this scope"],
    ]
      .map(
        ([m, title]) =>
          `<button data-library="${m}" class="${m === mode ? "active" : ""}" aria-pressed="${m === mode}">${title}</button>`,
      )
      .join("");
    $("#palette-context").textContent =
      mode === "combinators"
        ? state.addOnWire
          ? "Add preprocessing to the wire you are drawing."
          : state.edge
          ? "Insert into the selected receiving inlet."
          : "Choose a combinator, then place it on a wire."
        : state.addOnWire
          ? "Create and connect to the port you dragged."
          : mode === "find"
            ? "Jump to an actor in this scope."
            : "Choose an actor, then click the canvas to place it.";
    $("#palette-input").placeholder =
      mode === "find"
        ? "Type the name of an actor in this scope…"
        : "Type an actor or combinator name…";
    const steps = mode === "combinators" || (q && mode !== "find") ? source.stepKinds() : null;
    const combinators = (steps?.kinds ?? []).map((kind) => ({
      title: kind,
      type: kind,
      description: stepLook(kind).detail,
      icon: stepLook(kind).icon,
      diagnostic: stepLook(kind).iconDiagnostic,
      combinator: true,
      group: "Combinators",
      groupRank: mode === "combinators" ? -1 : undefined,
    }));
    const items =
      mode === "find"
        ? graph().nodes
        : mode === "combinators"
          ? q ? [...combinators, ...ProductCatalog.items] : combinators
          : q ? [...ProductCatalog.items, ...combinators] : ProductCatalog.items;
    const matches = items.filter((n) =>
      `${n.title} ${n.type} ${n.description || ""}`.toLowerCase().includes(q),
    );
    const sections =
      mode === "find" ? [{ rows: matches }] : ProductCatalog.sections(matches);
    const row = (n) =>
      `<button class="palette-result" ${mode === "find" ? `data-find="${n.id}"` : n.combinator ? `data-pick-combinator="${n.type}" data-add-combinator="${n.type}"` : `data-add="${n.type}"`}${n.diagnostic ? ` title="${esc(source.codeLabel(n.diagnostic).label)}" data-reason="${esc(source.codeLabel(n.diagnostic).code)}"` : ""}><span class="node-icon">${icon(n.icon)}</span><div><strong>${esc(n.title)}</strong><small>${esc(n.description || n.type)}</small></div></button>`;
    const empty = mode === "combinators" && steps?.code && !steps.kinds.length ? steps.code : "PALETTE_NO_MATCH";
    $("#palette-results").innerHTML = matches.length
      ? sections
          .map(
            (section) =>
              (sections.length > 1
                ? `<p class="palette-group" role="presentation">${esc(section.group)}</p>`
                : "") + section.rows.map(row).join(""),
          )
          .join("")
      : `<p class="journal-empty" data-reason="${esc(empty)}">${esc(source.codeLabel(empty).label)}</p>`;
    $(".palette-footer").innerHTML =
      `<span><kbd>↑ ↓</kbd> Browse <kbd>↵</kbd> Choose</span><span><kbd>Esc</kbd> Close</span>`;
    window.Product?.paletteCompatibility();
  }
  function startPlacement(type, kind = "actor") {
    state.placement = { type, kind };
    $("#palette").close();
    $("#canvas").focus({ preventScroll: true });
    const hint = $("#placement-hint");
    hint.innerHTML = `${icon(kind === "actor" ? "Plus" : "GitBranch")}<span>${kind === "actor" ? "Click to place" : "Click on a wire to insert"} <strong>${esc(type)}</strong></span><kbd>Esc</kbd>`;
    hint.classList.remove("hidden");
    $("#canvas").classList.add("placing");
    movePlacement(state.addAt);
  }
  function landingOf(type, p) {
    const corner = source.landing(type, { x: p.x - 115, y: p.y - 50 });
    return { x: corner.x + 115, y: corner.y + 50 };
  }
  function movePlacement(p, edge) {
    if (!state.placement) return;
    state.addAt = p;
    const { type, kind } = state.placement,
      ghost = $("#placement-ghost");
    ghost.classList.remove("hidden");
    ghost.classList.toggle("combinator-ghost", kind === "combinator");
    ghost.style.setProperty("--placement-grid", CanvasLayout.GRID + "px");
    if (kind === "combinator") {
      if (edge) p = nearestWirePoint(edge, p);
      ghost.innerHTML = `<span>${icon(stepLook(type).icon)}${esc(type)}</span>`;
    } else {
      const item = ProductCatalog.items.find((n) => n.type === type);
      ghost.innerHTML = `<header>${icon(item.icon)}${esc(item.title)}</header><div class="ghost-view"><span></span><span></span><span></span></div><footer>Click to place · Esc to cancel</footer>`;
      p = landingOf(type, p);
    }
    ghost.style.left = p.x - (kind === "actor" ? 115 : 32) + "px";
    ghost.style.top = p.y - (kind === "actor" ? 50 : 13) + "px";
  }
  function clearPlacement() {
    state.placement = null;
    clearConnection();
    $("#placement-ghost").classList.add("hidden");
    $("#placement-hint").classList.add("hidden");
    $("#canvas").classList.remove("placing");
  }
  function commitPlacement(point, edge) {
    const p = state.placement;
    if (!p) return;
    if (p.kind === "combinator") {
      if (!edge) return;
      selectEdge(edge, point);
      clearPlacement();
      addCombinator(p.type);
      state.addAt = null;
    } else {
      state.addAt = landingOf(p.type, point);
      clearPlacement();
      addActor(p.type);
    }
  }
  function pickCombinator(kind) {
    if (state.addOnWire && state.pendingPort) {
      const steps = state.pendingSteps ??= [];
      const at = state.connectionAt ?? state.addAt;
      const step = { id: `pending-step-${steps.length}`, kind, config: {}, x: at.x, y: at.y };
      if (state.pendingPort.side === "out") steps.push(step);
      else steps.unshift(step);
      steps.forEach((c, i) => { c.x = at.x - (steps.length - 1 - i) * 96; });
      state.editComponent = step.id;
      $("#palette").close("continue-wire");
      $("#canvas").focus({ preventScroll: true });
      drawPending(at);
      renderComponentEditor();
    } else if (state.edge) {
      $("#palette").close();
      addCombinator(kind);
      state.addAt = null;
    } else startPlacement(kind, "combinator");
  }
  function addActor(type) {
    const preprocess = state.pendingSteps?.length ? Product.pendingPreprocess() : [];
    if (!preprocess) return;
    const at = state.connectionAt ?? state.addAt;
    const created = source.perform({
      kind: "createActor",
      type,
      at: at && { x: at.x - 115, y: at.y - 50 },
      connectFrom: state.addOnWire ? state.pendingPort : null,
      ...(preprocess.length ? { preprocess } : {}),
    });
    $("#palette").close();
    state.addAt = null;
    state.addOnWire = false;
    return created;
  }
  function setView(view) {
    if (source.noProject) view = "projects";
    state.view = view;
    $("#canvas-view").classList.toggle("hidden", view !== "canvas");
    if (view === "canvas" && !state.controlRoom) {
      state.controlRoom = CardSize.controlRoom(document);
      transformWorld();
    }
    $$("[data-view]").forEach((b) =>
      b.classList.toggle("active", b.dataset.view === view),
    );
    window.Product?.viewChanged(view);
    location.hash = view;
    hideProbe();
    field?.wake();
    timeMachine?.refresh();
    settleFit();
  }
  document.addEventListener(
    "pointerdown",
    (e) => {
      if (!e.target.closest(".pause-control"))
        $("#pause-menu").classList.add("hidden");
    },
    true,
  );
  document.addEventListener("click", (e) => {
    if (performance.now() < suppressClickUntil) {
      e.preventDefault();
      return;
    }
    const target = e.target.closest("button,a,[data-edge],.node");
    if (!target) return;
    if (target.dataset.viewerToggle) {
      toggleNodeMode(target.dataset.viewerToggle);
      return;
    }
    if (target.dataset.pendingStep) {
      state.editComponent = target.dataset.pendingStep;
      renderComponentEditor();
      return;
    }
    if (target.dataset.combEdit) {
      state.editComponent =
        state.editComponent === target.dataset.combEdit
          ? null
          : target.dataset.combEdit;
      selectEdge(target.dataset.combEdge, null, target.dataset.combEdit);
      renderComponentEditor();
      return;
    }
    if (target.dataset.closeCombEditor) {
      closeComponentEditor();
      renderWireActions();
      return;
    }
    if (target.dataset.view) {
      setView(target.dataset.view);
      return;
    }
    if (target.dataset.scope) {
      setScope(target.dataset.scope);
      return;
    }
    if (target.dataset.enter) {
      setScope(target.dataset.enter);
      return;
    }
    if (target.dataset.select) {
      selectNode(target.dataset.select, e.shiftKey);
      return;
    }
    if (target.dataset.tab) {
      state.tab = target.dataset.tab;
      renderInspector();
      return;
    }
    if (target.dataset.journal) {
      state.journal = target.dataset.journal;
      renderJournal();
      return;
    }
    if (target.dataset.close) {
      $("#" + target.dataset.close).close();
      return;
    }
    if (target.dataset.authoringCommit) {
      Product.openAuthoringCommit(target.dataset.authoringCommit);
      return;
    }
    if (target.dataset.record) {
      const r = state.journalRows.find(
        (r) => `${r.actor}:${r.index}` === target.dataset.record,
      ) ?? source.recordOf(target.dataset.record);
      if (!r) return;
      selectNode(r.actor);
      state.selectedRecord = r;
      renderInspector();
      renderJournal();
      return;
    }
    if (target.dataset.find) {
      selectNode(target.dataset.find);
      $("#palette").close();
      return;
    }
    if (target.dataset.library) {
      state.paletteMode = target.dataset.library;
      renderPalette($("#palette-input").value);
      $("#palette-input").focus();
      return;
    }
    if (target.dataset.pickCombinator) {
      pickCombinator(target.dataset.pickCombinator);
      return;
    }
    if (target.dataset.add) {
      if (state.addOnWire) addActor(target.dataset.add);
      else startPlacement(target.dataset.add);
      return;
    }
    if (target.dataset.send) {
      sendPrompt(target);
      return;
    }
    if (target.dataset.port) {
      selectPort(target);
      return;
    }
    if (target.dataset.addCombinator) {
      addCombinator(target.dataset.addCombinator);
      return;
    }
    if (target.dataset.removeComb) {
      const e = graph().edges.find((w) => w.id === state.edge);
      if (!e) return;
      return source.perform({
        kind: "removeStep",
        edge: e.id,
        index: e.combinators.findIndex((c) => c.id === target.dataset.removeComb),
      });
    }
    if (target.dataset.comb) {
      selectEdge(
        target.dataset.edge,
        toWorld(e.clientX, e.clientY),
        target.dataset.comb,
      );
      return;
    }
    if (target.dataset.edge) {
      selectEdge(target.dataset.edge, toWorld(e.clientX, e.clientY));
      return;
    }
    if (target.dataset.nodeId) {
      if (
        !e.target.closest("input,textarea,select") &&
        state.selected !== target.dataset.nodeId
      )
        selectNode(target.dataset.nodeId, e.shiftKey);
      return;
    }
    switch (target.id) {
      case "insert-combinator":
      case "add-inlet-processing":
        openCombinatorMenu();
        break;
      case "new-actor-sidebar":
        openPalette();
        break;
      case "search-button":
        openPalette("find");
        break;
      case "add-actor":
      case "canvas-empty-add":
        openPalette();
        break;
      case "help-button":
        $("#help-dialog").showModal();
        break;
      case "pause-button":
        togglePause();
        break;
      case "pause-menu-button":
        $("#pause-menu").classList.toggle("hidden");
        break;
      case "force-pause":
        togglePause(true);
        break;
      case "fit-button":
        takeCamera();
        glideTo(fitCamera());
        break;
      case "zoom-in":
        takeCamera();
        glideBy(ZOOM.step);
        break;
      case "zoom-out":
        takeCamera();
        glideBy(1 / ZOOM.step);
        break;
      case "select-tool":
        setTool("select");
        break;
      case "pan-tool":
        setTool("pan");
        break;
      case "journal-toggle":
        $("#journal").classList.toggle("collapsed");
        $("#journal").classList.remove("expanded");
        renderJournal();
        break;
      case "journal-expand":
        $("#journal").classList.toggle("expanded");
        $("#journal").classList.remove("collapsed");
        renderJournal();
        break;
      case "discard-config":
        window.Product?.discardDraft(state.selected);
        renderGraph();
        break;
      case "close-inspector":
        state.selected = null;
        state.edge = null;
        $(".workspace").classList.remove("mobile-inspecting");
        renderGraph();
        break;
      case "inspect-records":
        state.journal = "selected";
        $("#journal").classList.remove("collapsed");
        renderJournal();
        break;
      case "copy-sdk":
        if (!navigator.clipboard) {
          gestureCode("CLIPBOARD_UNAVAILABLE");
          break;
        }
        Promise.resolve(sdkText(selectedNode()))
          .then(text => navigator.clipboard.writeText(text))
          .then(() => gestureCode("SDK_PROGRAM_COPIED", false))
          .catch(() => gestureCode("CLIPBOARD_UNAVAILABLE"));
        break;
      case "disconnect-wire": {
        const edge = graph().edges.find((w) => w.id === state.edge);
        Promise.resolve(source.perform({ kind: "retireEdge", edge: state.edge })).then((retired) => {
          if (!retired || state.edge !== edge?.id) return;
          state.edge = null;
          state.selected = edge.to;
          renderGraph();
        });
        break;
      }
    }
  });
  document.addEventListener("input", (e) => {
    if (e.target.matches(".prompt-input")) typePrompt(e.target);
    const form = e.target.closest("#config-form,[data-node-config]");
    if (form) {
      e.target.setCustomValidity("");
      captureDraft(form);
    }
  });
  document.addEventListener("change", (e) => {
    const form = e.target.closest("#config-form,[data-node-config]");
    if (form) captureDraft(form);
  });
  document.addEventListener("submit", (e) => {
    if (e.target.id === "config-form" || e.target.dataset.nodeConfig) {
      e.preventDefault();
      applyConfig(e.target);
    }
  });
  $("#palette-input").addEventListener("input", (e) =>
    renderPalette(e.target.value),
  );
  $("#palette-input").addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      $(".palette-result")?.click();
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      $(".palette-result")?.focus();
    }
  });
  $("#palette").addEventListener("close", () => {
    if ($("#palette").returnValue === "continue-wire") return;
    if (!state.placement) {
      state.addAt = null;
      state.addOnWire = false;
      clearConnection();
    }
  });
  $("#palette").addEventListener("keydown", (e) => {
    if (
      !["ArrowDown", "ArrowUp"].includes(e.key) ||
      e.target.id === "palette-input"
    )
      return;
    const buttons = $$(".palette-result"),
      i = buttons.indexOf(e.target);
    if (i < 0) return;
    e.preventDefault();
    buttons[
      (i + (e.key === "ArrowDown" ? 1 : buttons.length - 1)) % buttons.length
    ]?.focus();
  });
  $$(".palette,.help-dialog").forEach((d) =>
    d.addEventListener("click", (e) => {
      if (e.target !== d) return;
      const r = d.getBoundingClientRect();
      if (
        e.clientX < r.left ||
        e.clientX > r.right ||
        e.clientY < r.top ||
        e.clientY > r.bottom
      )
        d.close();
    }),
  );
  function clearConnection() {
    state.pendingPort = null;
    state.connectionAt = null;
    state.addOnWire = false;
    if (state.pendingSteps?.some((c) => c.id === state.editComponent)) {
      state.editComponent = null;
      renderComponentEditor();
    }
    state.pendingSteps = [];
    $("#pending-steps").textContent = "";
    if (!state.placement) $("#placement-hint").classList.add("hidden");
    window.Product?.clearCompatibility();
    $("#canvas").classList.remove("connecting", "connecting-from-in");
    $$(".node-port").forEach((p) =>
      p.classList.remove("pending", "drop-target"),
    );
    $("#connection-preview")?.setAttribute("d", "");
  }
  function beginConnection(el) {
    clearConnection();
    state.pendingPort = {
      node: el.dataset.node,
      name: el.dataset.port,
      side: el.dataset.side,
    };
    $("#canvas").classList.add("connecting");
    $("#canvas").classList.toggle(
      "connecting-from-in",
      el.dataset.side === "in",
    );
    $$(".node-port").forEach((p) => p.classList.toggle("pending", p === el));
    window.Product?.showCompatibility();
  }
  function completeConnection(target) {
    const start = state.pendingPort;
    if (!start || target.dataset.side === start.side) return false;
    const outlet =
      start.side === "out"
        ? start
        : { node: target.dataset.node, name: target.dataset.port };
    const inlet =
      start.side === "in"
        ? start
        : { node: target.dataset.node, name: target.dataset.port };
    return Product.connect(outlet, inlet);
  }
  function selectPort(el) {
    if (state.pendingPort && el.dataset.side !== state.pendingPort.side) {
      completeConnection(el);
      return;
    }
    beginConnection(el);
  }
  function drawPending(point) {
    const start = state.pendingPort;
    if (!start) return;
    const n = findNode(start.node),
      ports = start.side === "out" ? n.out : n.in,
      p = ports.find((p) => p[0] === start.name),
      a = {
        x: n.x + (start.side === "out" ? n.width : 0),
        y: n.y + portAt(n, p),
      };
    const from = start.side === "out" ? a : point,
      to = start.side === "in" ? a : point;
    const steps = state.pendingSteps ?? [];
    const points = [from, ...steps.map((c) => ({ x: c.x, y: c.y })), to];
    $("#connection-preview")?.setAttribute("d", points.slice(1).map((end, i) => {
      const start = points[i], mid = (start.x + end.x) / 2;
      return WireGeometry.rounded([start, { x: mid, y: start.y }, { x: mid, y: end.y }, end]);
    }).join(" "));
    const chips = $("#pending-steps");
    if (chips) chips.innerHTML = steps.map((c) =>
      `<button class="wire-component" data-pending-step="${c.id}" style="left:${c.x}px;top:${c.y}px" aria-label="Edit pending ${esc(c.kind)}"><span class="component-core">${icon(stepLook(c.kind).icon)}<strong>${esc(c.kind)}</strong></span></button>`).join("");
    if (steps.length) {
      const hint = $("#placement-hint");
      hint.innerHTML = `<span>Choose an ${start.side === "out" ? "inlet" : "outlet"}, or click the canvas to add an actor.</span><kbd>Esc</kbd>`;
      hint.classList.remove("hidden");
    }
  }
  let drag = null;
  function hold(kind, id, value) {
    return source.hold(kind, id, value);
  }
  function applyNodeSize(n) {
    const el = $("#node-" + n.id);
    el.style.width = n.width + "px";
    el.style.height = nodeHeight(n) + "px";
    el.style.setProperty("--prompt-port-y", nodeHeight(n) - 39 + "px");
    el.classList.toggle(
      "compact",
      nodeHeight(n) < size(n).height,
    );
    el.querySelector(".resize-dimensions").textContent =
      `${Math.round(n.width)} × ${Math.round(nodeHeight(n))}`;
  }
  $("#canvas").addEventListener("pointerdown", (e) => {
    if (e.button !== 0 && e.button !== 1) return;
    if (historical() && e.target.closest(".node,[data-comb],.annotation")) return;
    if (state.placement && !e.target.closest(".canvas-tools")) {
      commitPlacement(
        toWorld(e.clientX, e.clientY),
        e.target.closest("[data-edge]")?.dataset.edge,
      );
      suppressClickUntil = performance.now() + 240;
      e.preventDefault();
      return;
    }
    hideProbe();
    if (!e.target.closest("input,textarea,select,button"))
      $("#canvas").focus({ preventScroll: true });
    const port = e.target.closest(".node-port"),
      resize = e.target.closest("[data-resize]"),
      component = e.target.closest("[data-comb]"),
      header = e.target.closest("[data-drag]");
    const common = {
      startX: e.clientX,
      startY: e.clientY,
      moved: false,
      pointer: e.pointerId,
    };
    const noteHandle = e.target.closest("[data-note-resize],[data-note-drag]");
    if (noteHandle && !e.target.closest("[data-delete-note]") && !state.space && state.tool !== "pan") {
      const resizeNote = Boolean(noteHandle.dataset.noteResize);
      const note = graph().notes.find(n => n.id === (noteHandle.dataset.noteResize ?? noteHandle.dataset.noteDrag));
      const el = noteHandle.closest(".annotation");
      drag = { ...common, kind: resizeNote ? "note-resize" : "note-move", note, el,
        x: note.x, y: note.y, width: el.offsetWidth, height: el.offsetHeight };
      drag.preview = resizeNote ? { width: drag.width, height: drag.height } : { x: drag.x, y: drag.y };
    } else if (port && !state.space && state.tool !== "pan") {
      if (state.pendingPort && state.pendingPort.side !== port.dataset.side) {
        completeConnection(port);
        suppressClickUntil = performance.now() + 200;
        return;
      }
      beginConnection(port);
      drag = { ...common, kind: "connect" };
    } else if (resize) {
      const n = findNode(resize.dataset.resize);
      selectNode(n.id);
      drag = {
        ...common,
        kind: "resize",
        node: n,
        width: n.width,
        height: nodeHeight(n),
      };
      $("#node-" + n.id).classList.add("resizing");
    } else if (component) {
      const e = graph().edges.find((w) => w.id === component.dataset.edge),
        c = e.combinators.find((c) => c.id === component.dataset.comb);
      selectEdge(e.id, { x: c.x, y: c.y }, c.id);
    } else if (
      e.target.closest(
        "button,textarea,input,select,.canvas-tools,[data-edge],.annotation",
      )
    )
      return;
    else if (header && state.tool !== "pan" && !state.space) {
      const n = findNode(header.dataset.drag);
      selectNode(n.id, e.shiftKey);
      drag = {
        ...common,
        kind: "node",
        node: n,
        x: n.x,
        y: n.y,
        group: graph()
          .nodes.filter((a) => state.selectedSet.has(a.id))
          .map((a) => ({ node: a, x: a.x, y: a.y })),
      };
      for (const item of drag.group) travel.stop(item.node.id);
    } else if (
      !e.target.closest(".node") ||
      state.tool === "pan" ||
      state.space
    ) {
      drag = { ...common, kind: "pan", x: state.x, y: state.y };
      $("#canvas").classList.add("dragging");
    }
    if (drag) {
      $("#canvas").setPointerCapture(e.pointerId);
      e.preventDefault();
    }
  });
  $("#canvas").addEventListener("pointermove", (e) => {
    state.cursorWorld = toWorld(e.clientX, e.clientY);
    if (state.placement) {
      movePlacement(
        state.cursorWorld,
        e.target.closest("[data-edge]")?.dataset.edge,
      );
      return;
    }
    if (state.pendingPort && !drag && !$("#palette").open && !e.target.closest("#component-editor")) {
      drawPending(state.cursorWorld);
      return;
    }
    if (!drag) {
      const wire = e.target.closest("[data-edge]");
      if (wire && !e.target.closest("#wire-actions"))
        showProbe(wire.dataset.edge, e.clientX, e.clientY);
      else if (state.hoverEdge) hideProbe();
      return;
    }
    const dx = e.clientX - drag.startX,
      dy = e.clientY - drag.startY;
    if (Math.abs(dx) + Math.abs(dy) > 4) drag.moved = true;
    if (drag.kind === "connect") {
      drawPending(toWorld(e.clientX, e.clientY));
      const over = document
        .elementFromPoint(e.clientX, e.clientY)
        ?.closest(".node-port");
      $$(".node-port").forEach((p) =>
        p.classList.toggle(
          "drop-target",
          p === over && p.dataset.side !== state.pendingPort.side,
        ),
      );
    } else if (drag.kind === "note-move" || drag.kind === "note-resize") {
      if (drag.kind === "note-move") {
        drag.preview = { x: Math.round(drag.x + dx / state.zoom), y: Math.round(drag.y + dy / state.zoom) };
        drag.el.style.left = drag.preview.x + "px";
        drag.el.style.top = drag.preview.y + "px";
      } else {
        drag.preview = { width: Math.max(80, Math.round(drag.width + dx / state.zoom)), height: Math.max(60, Math.round(drag.height + dy / state.zoom)) };
        drag.el.style.width = drag.preview.width + "px";
        drag.el.style.height = drag.preview.height + "px";
      }
    } else if (drag.kind === "resize") {
      const n = hold("sizes", drag.node.id, {
        width: Math.max(180, Math.round(drag.width + dx / state.zoom)),
        height: Math.max(
          size(drag.node).min,
          Math.round(drag.height + dy / state.zoom),
        ),
      });
      if (n) applyNodeSize(n);
      scheduleWires();
    } else if (drag.kind === "node") {
      for (const item of drag.group) {
        const at = { x: item.x + dx / state.zoom, y: item.y + dy / state.zoom };
        hold("moves", item.node.id, at);
        const el = $("#node-" + item.node.id);
        el.style.left = at.x + "px";
        el.style.top = at.y + "px";
      }
      scheduleWires();
    } else {
      takeCamera();
      state.x = drag.x + dx;
      state.y = drag.y + dy;
      transformWorld();
    }
  });
  $("#canvas").addEventListener("pointerleave", () => {
    if (!drag) hideProbe();
  });
  function endDrag(e) {
    if (!drag) return;
    const previous = drag;
    drag = null;
    const cancelled = e?.type === "pointercancel";
    if (previous.kind === "note-move" || previous.kind === "note-resize")
      source.perform({ kind: previous.kind === "note-move" ? "moveNote" : "resizeNote", note: previous.note.id, ...previous.preview, cancelled });
    if (previous.kind === "node")
      source.perform({
        kind: "moveActors",
        moves: previous.group.map((item) => {
          const n = allNode(item.node.id) ?? item.node;
          return { actor: n.id, x: n.x, y: n.y };
        }),
        cancelled: cancelled || !previous.moved,
      });
    if (previous.kind === "resize") {
      const n = allNode(previous.node.id) ?? previous.node;
      source.perform({ kind: "resize", actor: n.id, width: n.width, height: n.height, cancelled });
    }
    $("#canvas").classList.remove("dragging");
    $$(".resizing").forEach((n) => n.classList.remove("resizing"));
    if (previous.kind === "connect") {
      const target = e
        ? document.elementFromPoint(e.clientX, e.clientY)?.closest(".node-port")
        : null;
      if (target && target.dataset.side !== state.pendingPort?.side)
        completeConnection(target);
      else if (previous.moved && e && e.type !== "pointercancel") {
        const hit = document.elementFromPoint(e.clientX, e.clientY);
        if (
          hit?.closest("#canvas") &&
          !hit.closest(".node,.canvas-tools")
        ) {
          state.connectionAt = state.addAt = toWorld(e.clientX, e.clientY);
          state.addOnWire = true;
          openPalette();
        } else clearConnection();
      }
    }
    if (previous.moved) {
      suppressClickUntil = performance.now() + 240;
      if (previous.kind === "node" || previous.kind === "resize")
        renderWires();
    } else if (previous.kind === "pan") {
      state.edge = null;
      state.component = null;
      refreshWireSelection();
      if (state.pendingPort && !cancelled) {
        state.connectionAt ??= toWorld(e.clientX, e.clientY);
        state.addAt = state.connectionAt;
        state.addOnWire = true;
        drawPending(state.connectionAt);
        openPalette();
      }
    }
  }
  $("#canvas").addEventListener("pointerup", endDrag);
  $("#canvas").addEventListener("pointercancel", (e) => {
    clearConnection();
    endDrag(e);
  });
  $("#canvas").addEventListener("dblclick", (e) => {
    const n = e.target.closest(".node"),
      node = n && findNode(n.dataset.nodeId);
    if (node?.scope) setScope(node.scope);
    else if (
      !n &&
      !e.target.closest("button,[data-edge],.canvas-tools")
    ) {
      state.addAt = toWorld(e.clientX, e.clientY);
      openPalette();
    }
  });
  document.addEventListener("keydown", (e) => {
    const handle = e.target.closest("[data-resize]");
    if (
      !handle ||
      !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key)
    )
      return;
    e.preventDefault();
    const n = findNode(handle.dataset.resize),
      step = e.shiftKey ? 24 : 8;
    hold("sizes", n.id, {
      width: Math.max(
        180,
        n.width +
          (e.key === "ArrowRight" ? step : e.key === "ArrowLeft" ? -step : 0),
      ),
      height: Math.max(
        size(n).min,
        nodeHeight(n) +
          (e.key === "ArrowDown" ? step : e.key === "ArrowUp" ? -step : 0),
      ),
    });
    applyNodeSize(n);
    renderWires();
  });
  document.addEventListener("keyup", (e) => {
    const handle = e.target.closest("[data-resize]");
    if (
      !handle ||
      !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key)
    )
      return;
    const n = findNode(handle.dataset.resize);
    source.perform({ kind: "resize", actor: n.id, width: n.width, height: n.height, cancelled: false });
  });
  $("#canvas").addEventListener(
    "wheel",
    (e) => {
      if (e.target.closest?.("#component-editor")) return;
      e.preventDefault();
      const moved = wheelPixels(e);
      if (e.ctrlKey || e.metaKey) {
        takeCamera();
        glideBy(Math.exp(-moved.y * ZOOM.wheel), { x: e.clientX, y: e.clientY });
        return;
      }
      const card = e.target.closest?.("#nodes .node"),
        parts = [];
      for (let at = e.target; card && at && at !== card; at = at.parentElement) parts.push(at);
      const body = [...document.querySelectorAll("#nodes > .node > .node-viewer")].reverse().find((viewer) => {
        const box = viewer.getBoundingClientRect();
        return e.clientX >= box.left && e.clientX < box.right && e.clientY >= box.top && e.clientY < box.bottom;
      });
      if (body) for (const box of [body.querySelector(":scope > .glance"), body]) if (box && !parts.includes(box)) parts.push(box);
      const rest = { ...moved };
      for (const at of parts) for (const axis of ["x", "y"]) rest[axis] -= scrollPart(at, axis, rest[axis]);
      if (!rest.x && !rest.y) return;
      takeCamera();
      state.x -= rest.x;
      state.y -= rest.y;
      transformWorld();
    },
    { passive: false },
  );
  document.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
      if (state.view !== "canvas") return;
      e.preventDefault();
      openPalette("find");
      return;
    }
    const input = /INPUT|TEXTAREA|SELECT/.test(e.target.tagName);
    const cancelWire = e.key === "Escape" && state.pendingPort;
    if (cancelWire) {
      e.preventDefault();
      if ($("#palette").open) $("#palette").close();
    }
    if (e.key === "Escape" && !cancelWire && state.editComponent && !$("dialog[open]")) {
      e.preventDefault();
      closeComponentEditor();
      renderWireActions();
      $("#canvas").focus({ preventScroll: true });
      return;
    }
    if (!cancelWire && (input || $("dialog[open]"))) return;
    if (e.key === "Escape") {
      if (state.placement) clearPlacement();
      state.addAt = null;
      state.editComponent = null;
      renderComponentEditor();
      clearConnection();
      hideProbe();
      state.component = null;
      state.pendingPort = null;
      $("#canvas").classList.remove("connecting");
      state.selected = null;
      state.selectedSet.clear();
      state.edge = null;
      $(".workspace").classList.remove("mobile-inspecting");
      $("#pause-menu").classList.add("hidden");
      renderGraph();
    }
    if (state.view !== "canvas") return;
    if (
      e.key.toLowerCase() === "n" ||
      (e.key === "Tab" &&
        !e.shiftKey &&
        e.target.closest("#canvas") &&
        !e.target.closest("button"))
    ) {
      e.preventDefault();
      state.addAt = state.cursorWorld;
      openPalette(state.edge ? "combinators" : "actors");
      return;
    }
    if (e.key === "Enter" && state.placement) {
      e.preventDefault();
      commitPlacement(state.addAt, state.edge);
      return;
    }
    if (e.key.toLowerCase() === "f") {
      takeCamera();
      glideTo(fitCamera());
    }
    if (e.key.toLowerCase() === "h") setTool("pan");
    if (e.key.toLowerCase() === "v") setTool("select");
    if (e.key === " ") {
      e.preventDefault();
      state.space = true;
      $("#canvas").classList.add("pan");
    }
    if (e.key === "Enter") {
      if (e.target.dataset.nodeId) selectNode(e.target.dataset.nodeId);
      else if (e.target.dataset.edge) selectEdge(e.target.dataset.edge);
    }
  });
  document.addEventListener("keyup", (e) => {
    if (e.key === " ") {
      state.space = false;
      $("#canvas").classList.toggle("pan", state.tool === "pan");
    }
  });
  window.addEventListener("blur", () => {
    state.space = false;
    endDrag();
  });
  new ResizeObserver(() => {
    const r = $("#canvas").getBoundingClientRect();
    field?.resize(r.width, r.height);
    field?.wake();
  }).observe($("#canvas"));
  archive = new PresentationArchive(liveScopes());
  const settingsRoom = new ResizeObserver((entries) => {
    for (const { target: area } of entries) {
      const card = area.isConnected && area.closest(".node");
      if (!card) {
        settingsRoom.unobserve(area);
        continue;
      }
      const first = area.querySelector(".config-field"),
        control = first?.querySelector("input, select, textarea");
      const need = control
        ? control.getBoundingClientRect().bottom - first.getBoundingClientRect().top
        : 0;
      card.toggleAttribute("data-settings-short", need > area.getBoundingClientRect().height);
    }
  });
  state.journalRows = archive.entries;
  travel = new CardTravel({
    frame: (callback) => requestAnimationFrame(callback),
    now: () => document.timeline?.currentTime ?? performance.now(),
    reduced: () => matchMedia("(prefers-reduced-motion: reduce)").matches,
    onFrame: (ids) =>
      renderWires(
        new Set(
          graph()
            .edges.filter((e) => ids.has(e.from) || ids.has(e.to))
            .map((e) => e.id),
        ),
      ),
    onEnd: () => renderWires(),
  });
  $("#canvas").style.setProperty("--wire-track", CanvasLayout.TRACK + "px");
  field = new WireField($("#wire-fluid"), {
    lightCanvas: $("#wire-light"),
    visible: () => state.view === "canvas",
    paused: () => (historical() ? timeMachine.mode !== "replay" : paused()),
    recorded: historical,
    time: displayTime,
    transform: () => state,
    rate: edgeRate,
    rateSeconds: source.rateSeconds,
    selected: () => state.edge,
    tick: (t) => {
      updateProbe(t);
      updateViewers(t);
    },
  });
  timeMachine = new TimeMachine(archive, {
    transport: source.transport,
    bar: source.timeBar,
    head: () => source.head,
    scopes: liveScopes,
    pausedScopes: () => state.paused,
    observing: () => source.observing(),
    ended: () => source.endedText(),
    actors: () =>
      new Set([
        ...archive.frames.flatMap(
          (f) => f.scopes[state.scope]?.nodes.map((n) => n.id) || [],
        ),
        ...source.projection[state.scope].nodes.map((n) => n.id),
      ]),
    visible: () => state.view === "canvas",
    onLiveTick: () => {
      renderJournal();
    },
    onView: showRecordedTime,
    reading: (text) => {
      $("#history-caption-time").textContent = text;
    },
  });
  window.StudyApp = {
    state,
    graph,
    liveScopes,
    findNode,
    allNode,
    selectedNode,
    historical,
    displayTime,
    visibleRecords,
    renderGraph,
    renderActors,
    drawCards,
    syncNodeElement,
    refreshFaces,
    renderInspector,
    renderWires,
    showMailboxes,
    renderComponentEditor,
    renderJournal,
    renderJournalHeader,
    renderInspectorFooter,
    inletSteps,
    selectNode,
    selectEdge,
    setScope,
    setView,
    fitCanvas,
    oweFit,
    transformWorld,
    field,
    timeMachine,
    archive,
    toast,
    gestureCode,
    hydrate,
    icon,
    esc,
    nodeHeight,
    addActor,
    openPalette,
    clearConnection,
    scheduleWires,
    updateViewers,
    changedKeys,
    syncConfigActions,
    completeConnection,
    toWorld,
    togglePause,
    sdkContent,
    updatePause,
    updateScopeHealth,
  };
  const bounds = $("#canvas").getBoundingClientRect();
  field.resize(bounds.width, bounds.height);
  hydrate();
})();
