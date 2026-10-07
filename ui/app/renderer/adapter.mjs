import { readScene, readRuntime, readPorts, applyPorts, readHealth, applyHealth, joined, key, scopeLabel, addressPath, ORIGIN, newCardSize, landing as landingIn, unansweredPorts, PORT_UNANSWERED, portShape, runPause, viewContext } from './scene.mjs';
import { portCaption, captionReach } from './card-size.mjs';
import { actorIdentityFromValue } from '@circular/protocol/establishment';
import { followRecords } from './records.mjs';
import { chipRoom, UNMEASURED } from './card-size.mjs';
import { followCommits, applyCommit, revisionShort } from './commits.mjs';
import { wirePerSecond, WIRE_RATE_SECONDS, recordedRate, recordedPerSecond, wireArrival, declaredEdge } from './activity.mjs';
import { timeReading } from './history.mjs';
import { arrivalWindow, authoredName, projectJournal, journalProjection, projectAcceptedCommit, journalStatus, journalReason, applyJournal, recordValueUnavailable } from './journal.mjs';
import { address, editor, move, dragMove, dragResize, organize, connect, configure, present, setFlags, disconnect, removeActors, moveIntoScope, createActor, freshLocal, foldIntoNewScope, foldIntoNewReplicator, mountRequest, bindHarness } from './edit.mjs';
import { organizeTargets, bindOrganize } from './organize.mjs';
import { footprint } from './layout.mjs';
import { undoEntry, restore, undoHistory, touchedKeys, boundaryPorts } from './undo.mjs';
import { verbOf, keysOf } from './verbs.mjs';
import { reason, inSpace, codeText, noDaemon, noProject, healthText, revisionConflict, detailText, reasonText, refusalCode } from './reasons.mjs';
import { chipToasts } from './toast.mjs';
import { decide, readApprovalQueue, decoded } from './session.mjs';
import { lifecycleResultFromValue, sameValue } from '@circular/protocol';
import { catalogItems, bindCatalogAvailability, configDeclared, containerCardinality, paletteAvailability } from './catalog.mjs';
import { configFormHTML, fieldsHTML, fieldName, configPath, setAllApprovals } from './config-form.mjs';
import { performer } from './gesture.mjs';
import { configFieldList, readConfigForm, readCreateInputs, changeStructure, portDraft } from './config-fields.mjs';
import { openCreateDialog, refusalText } from './authoring-forms.mjs';
import { harnessSectionHTML, harnessReading, readingSettled, readingDone, bindHarnessForms } from './harness-binding.mjs';
import { askDepths, depthsOf, forgetDepths, pools, inletDepths } from './edge-depths.mjs';
import { emptyScene } from './fold.mjs';
import { projectSDKProgram, sdkProgram } from './sdk-program.mjs';
import { bindProductUtility } from './product-utility.mjs';
import { bindProjects, baseName, projectDaemonHTML, projectPaneHTML, projectHarnessesHTML, bindProjectControls, stateLine, recentRefusal } from './projects.mjs';
import { outputsFromScene } from './outputs.mjs';
import { renderSurface, updateSurface, bindSurfaceInputs } from './surface.mjs';
import { updateLocalMachine } from './local-machine.mjs';
import { healthSet, healthReading } from './health-count.mjs';
import { reattacher, awaitsDaemon, startRefused, sessionLost, rejoinedText, rejoinedFact } from './attachment.mjs';
import { updateConnectionEvidence, evidenceText } from './connection-evidence.mjs';
import { scopeHealth } from './scope-health.mjs';
import { statusbarText, updateStatusbar } from './statusbar.mjs';
import { noteBody, retireNote, createNote, placeNotes } from './annotations.mjs';
import { changeCombinator, changeStep, chipPlaces, draftChipPlace, readPreprocessKinds, changeInletSettings, deliveryName, deliveryExecuted } from './combinators.mjs';
import { cardFace } from './node-state.mjs';
import { viewer } from './viewer.mjs';
import { journalRows, identity, domId, actorKey } from './query.mjs';
import { views } from './views.mjs';
import { distribute, instanceContainers, latestRow, outletReading, isEmission } from './arrivals.mjs';
import { observationsLine } from './inspector-connection.mjs';
import { actorLine } from './inspector-identity.mjs';
import { issueUnobserved, markIssueLayer, updateWireInspector, wireProblems, wireIssue } from './wire-inspector.mjs';
import { newReasons, showBanner, holds } from './banner.mjs';
import { portTitle, permissionsView, heldPermissions } from './inspect-tab.mjs';
import { publishWallClock, wallClockOf } from './wall-clock.mjs';
import { recordValue } from './record-text.mjs';
import { valueText } from './value-text.mjs';
import { readCause, approvalCall, causeRecord, causeUnrecorded, causeUnread } from './approval-call.mjs';
export const display = value => value === undefined ? undefined : JSON.parse(JSON.stringify(value, (_k,v) =>
  typeof v === 'bigint' ? String(v) : v instanceof Uint8Array ? [...v] : v instanceof Map ? Object.fromEntries(v) : v));
export { domId };
const roleLooks = new Map([
  [[3n, 'source'], ['ArrowDownLeft', 'Sources']],
  [[1n], ['Box', 'Processing']], [[2n], ['SlidersHorizontal', 'Inline operations']],
  [[3n, 'sink'], ['ArrowUpRight', 'Destinations']],
  [[4n, 'one'], ['Layers', 'Containers']], [[4n, 'keyed_many'], ['Layers', 'Containers']],
].map(([role, look]) => [key(role), look]));
const groupRanks = [...new Set([...roleLooks.values()].map(([, group]) => group))];
const catalogIcon = role => role === undefined ? undefined : roleLooks.get(key(role))?.[0];
const catalogGroup = role => {
  const group = role === undefined ? undefined : roleLooks.get(key(role))?.[1];
  return group && { group, rank: groupRanks.indexOf(group) };
};
function sameFields(a, b) {
  if (a === b || (a !== a && b !== b)) return true;
  if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) return false;
  const proto = Object.getPrototypeOf(a);
  if (proto !== Object.getPrototypeOf(b)) return false;
  if (Array.isArray(a) || a instanceof Uint8Array) {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (!sameFields(a[i], b[i])) return false;
    return true;
  }
  if (a instanceof Map) return a.size === b.size && [...a].every(([k, v]) => b.has(k) && sameFields(v, b.get(k)));
  if (a instanceof Set) return a.size === b.size && [...a].every(k => b.has(k));
  if (proto !== Object.prototype && proto !== null) return false;
  const keys = Object.keys(a);
  return keys.length === Object.keys(b).length && keys.every(k => Object.hasOwn(b, k) && sameFields(a[k], b[k]));
}
const cards = new WeakMap();
function cardOf(n, graph, input, project) {
  const held = cards.get(n);
  if (held?.declared === graph.declared && held.createInputs === graph.createInputs && sameFields(held.input, input)) return held.card;
  const card = project();
  cards.set(n, {declared:graph.declared, createInputs:graph.createInputs, input, card});
  return card;
}
const canvasScope = (graph, scope) => scope.id === key(graph.declared.root) ? 'root' : domId(scope.id);
const scopeNamed = name => graph?.scopes.find(scope => canvasScope(graph, scope) === name);
export function studyFromScene(graph, only) {
  const scopes = new Map(graph.scopes.map(s => [s.id, canvasScope(graph, s)]));
  const ctx = viewContext(graph);
  const metrics = graph.metrics ?? UNMEASURED;
  const unanswered = new Map(graph.nodes.filter(n => !only || only.has(n.id)).map(n => [n.id, unansweredPorts(n, graph.edges, metrics)]));
  const study = {journal:[]};
  const above = address => {
    for (let n = address.length - 1; n >= 0; n--) if (scopes.has(key(address.slice(0, n)))) return n;
    return null;
  };
  for (const scope of graph.scopes) {
    const n = above(scope.address);
    study[scopes.get(scope.id)] = {name:scope.name,
      parent:n === null ? null : scopes.get(key(scope.address.slice(0, n))),
      segment:n === null ? scope.name : scopeLabel(scope.address.slice(n)),
      nodes:[],edges:[],notes:[]};
  }
  for (const n of graph.nodes) {
    if (only && !only.has(n.id)) continue;
    const view = views.of(n.viewKind.kind), input = view.input?.(n,ctx);
    study[scopes.get(n.scope)].nodes.push(cardOf(n, graph, input, () => {
      const childScope = key([...n.address.scope,{name:n.address.local}]);
      const child = scopes.get(childScope);
      const icon = catalogIcon(n.registration?.presentation_role);
      const iconDiagnostic = icon ? null : reason('ACTOR_ICON_UNAVAILABLE');
      const health = n.health?.state ?? 'unobserved';
      const code = n.health?.reason?.code ?? n.health?.reason?.value ?? n.health?.reason;
      const detail = n.health?.detail ?? null;
      const slot = detail?.slot == null ? undefined : graph.createInputs?.entries.get(n.declaration.actorType)?.slots?.find(s => s.key === detail.slot);
      const field = slot ? fieldName(slot) : detail?.slot ?? null;
      const issue = code == null ? undefined : {code:codeText(code),message:[reasonText(codeText(code)),detailText(detail,field)].filter(Boolean).join(' · '),detail,field};
      const activity = healthText({health,issue});
      const latest=n.arrivals.reduce((last,row) => !last || row.index > last.index ? row : last,null);
      const u = unanswered.get(n.id);
      return {id:domId(n.id),path:addressPath(n.address),title:n.title,type:n.type,icon:icon ?? 'Box',iconDiagnostic,
        typeLabel:n.registration?.label ?? n.type,
        x:n.x,y:n.y,width:n.width,height:n.height,viewKind:n.viewKind,
        ...(child ? {scope:child} : {}), config:display(n.declaration.config),flags:display(n.declaration.flags),
        in:[...n.in.map(row=>{const [id,flow,y,label]=row; return [id,portShape(flow),y,label ?? id,portCaption(row)];}),...u.in.map(([id,code,y])=>[id,code,y,id,id])],
        out:[...n.out.map(row=>{const [id,flow,y,label]=row; return [id,portShape(flow),y,label ?? id,portCaption(row)];}),...u.out.map(([id,code,y])=>[id,code,y,id,id])],
        portsUnavailableReason:n.portsUnavailableReason,
        portsUnanswered:[...u.in,...u.out].map(p=>p[0]),
        health,
        recordedArrivals:n.recordedArrivals == null ? null : String(n.recordedArrivals),
        earlierRevision:latest?.origin?.[4]?.value < graph.anchor?.cursor,
        activity,
        diagnostics:[iconDiagnostic?.code,n.portsUnavailableReason,
          u.in.length+u.out.length ? PORT_UNANSWERED : null].filter(Boolean).map(code => codeText(code)),
        ...(issue ? {issue} : {}),
        approvalCount:n.approvalCount ?? 0,
        preview:{rows:n.arrivals.map(r=>[r.port,valueText(r.body).text])},
        ...input};
    }));
    study.journal.push(...projectJournal({items:n.arrivals},[n],domId,recordValue));
  }
  if (only) return study;
  const chipsBy = new Map(), problems = wireProblems(graph.problems), unobserved = issueUnobserved(graph.problems);
  for (const edge of graph.edges) {
    const node = graph.nodes.find(n=>n.id===edge.to);
    const scope = study[scopes.get(node?.scope)];
    const steps=edge.attributes.preprocess ?? [];
    if (scope && !chipsBy.has(scope)) chipsBy.set(scope,[]);
    const inlet=[...(node?.in ?? []),...(unanswered.get(node?.id)?.in ?? [])].find(p=>p[0]===edge.in);
    const places=chipPlaces(node && {x:node.x,y:node.y+(inlet?.[2] ?? 0)+metrics.inset,card:scope?.nodes.find(card=>card.id===domId(node.id))},steps.length,
      chipsBy.get(scope) ?? [],scope?.nodes ?? [],chipRoom(metrics),metrics);
    scope?.edges.push({id:domId(edge.id),from:domId(edge.from),to:domId(edge.to),in:edge.in,out:edge.out,
      fromName:authoredName(graph.nodes.find(n=>n.id===edge.from)?.address),toName:authoredName(node?.address),
      wire:domId(declaredEdge(edge)),
      ordinal:display(edge.address.ordinal),
      label:'',combinators:steps.map((step,i)=>({
        id:domId(edge.id)+'-'+i+'-'+step.kind,kind:step.kind,config:display(step.config),
        expression:String(step.config.transform ?? step.config.predicate ?? ''),cue:step.kind,
        ...places[i],
      })),
      declaredDelay:Number(edge.attributes.delay.num*1000n)/Number(edge.attributes.delay.den),
      delivery:deliveryName(edge.attributes.policy.delivery),
      ...(deliveryExecuted(edge.attributes.policy.delivery) ? {} : {deliveryIssue:(r => ({code:r.code,message:r.label}))(reason('DELIVERY_NOT_EXECUTED'))}),
      capacity:display(edge.attributes.policy.capacity),
      declaration:display({...edge.address,attrs:edge.attributes}),
      ...(problems.has(declaredEdge(edge)) ? {issue:wireIssue(problems.get(declaredEdge(edge)))} : {}),...(unobserved ? {issueUnobserved:unobserved} : {})});
  }
  for (const note of graph.annotations ?? []) {
    const projection = study[scopes.get(note.scope)];
    projection?.notes.push({id:domId(note.id),text:note.body,x:note.x,y:note.y,width:note.width,height:note.height,presentation:note.presentation});
  }
  for (const scope of Object.values(study)) if (scope?.notes?.length) placeNotes(scope.notes, scope.nodes ?? []);
  return study;
}
export function updateViewer(el,n,win = globalThis.window) {
  win.LiveViewers.update(el,n,win.StudyApp?.displayTime?.() ?? win.StudySource.head);
}
let opener, session, graph, source, diagnostic, following, followingCommits, writer, journalCode;
let cardMetrics;
let acceptedEdits = [];
let ended = null;
const endedCode = () => ended?.code;
const attachCode = () => ended?.by === 'attach' ? ended.code : undefined;
const standingCode = () => endedCode() ?? graph?.healthDiagnostic;
function endBy(by, code) {
  if (by === 'records' && ended?.by === 'attach') return;
  ended = { by, code };
}
let history = undoHistory();
let retrying = false, recentCode;
let connectionEvidence;
let away, rejoined, seenAt = null;
const reattach = reattacher({ ask: () => reconnect(false, true) });
const recordedAt = seconds => seconds == null ? 'unrecorded'
  : globalThis.window?.studyTimeFormat?.(seconds) ?? `${seconds.toFixed(3)} s`;
const lastSeen = () => recordedAt(away?.at ?? seenAt);
function scopeAddress(scopeId = 'root') {
  const scope = scopeNamed(scopeId);
  return scope ? scope.address : scopeId === 'root' ? [] : null;
}
function localMachine(root) {
  const code = standingCode();
  updateLocalMachine(root, graph && healthSet(graph, []), graph?.healthPage, code, lastSeen());
}
function observationEnded() {
  const code = endedCode();
  return code === undefined || !graph ? null : { code, at: lastSeen() };
}
function awaitDaemon(code) {
  if (awaitsDaemon(code)) {
    const at = graph ? source.observedHead : null;
    away ??= { code, at };
    reattach.watch({ code: away.code, at: away.at });
  }
  else if (!startRefused(code)) reattach.stop();
}
let arrivals, arriving, arrivalCode;
let view = null;
const shownArrivals = () => view?.arrivals ?? arrivals;
let selection = {key:undefined};
function selectionRecords(ids) {
  const nodes=[...new Set(ids)].map(original).filter(Boolean), shown=shownArrivals();
  if (!nodes.length || !shown) return undefined;
  const key=nodes.map(n => n.id).join(' ');
  if (selection.key !== key) {
    const asked=selection={key, rows:undefined, code:undefined};
    shown.selection(nodes.map(n => n.id)).then(items => {
      if (selection !== asked) return;
      asked.rows=projectJournal({items},graph.nodes,domId,recordValue);
      window.StudyApp?.renderJournal?.();
    }, error => {
      if (selection !== asked) return;
      asked.code=codeText(error.code ?? 'READ_UNAVAILABLE');
      window.StudyApp?.renderJournal?.();
    });
  }
  const rows=new Map();
  for (const row of [...(selection.rows ?? []), ...nodes.flatMap(n => source.actorRecords(domId(n.id)))])
    rows.set(`${row.actor}:${row.index}`, row);
  return {rows:[...rows.values()].sort((x,y) => y.at-x.at).slice(0,journalRows), code:selection.code, pending:selection.rows === undefined && selection.code === undefined};
}
let turn = Promise.resolve();
const inTurn = pass => (turn = turn.then(pass, pass));
const actorRows = views.rows();
let approvalPage, approvalCode;
let preprocessKinds=[], preprocessCode, preprocessRead;
function readCombinators() {
  if (preprocessRead || !session || attachCode()) return;
  preprocessRead=readPreprocessKinds(session).then(kinds => {
    preprocessKinds=kinds;preprocessCode=undefined;
  },error => {preprocessCode=error.code ?? 'READ_UNAVAILABLE';}).then(() => {
    if (globalThis.document?.querySelector?.('#palette')?.open)
      document.querySelector('#palette-input').dispatchEvent(new Event('input'));
  });
}
let organizeControl, projectsView, projectsShown, redrawProjects, awaitedReading, harnessDialog;
let timeBar;
const carries=(node,at) => node !== undefined
  && !('x' in at ? dragMove(node,at.x,at.y) : dragResize(node,at.width,at.height)).length;
let refreshHistory = () => {};
export let observation;
let toasts = chipToasts();
let bannerFact;
let bannerBaseline;
function report(code) {
  diagnostic = reason(code);
  if (source.noProject || (endedCode() !== undefined && reason(endedCode()).code === diagnostic.code)) console.info(code);
  else toasts.notify(code);
  source.updateStatusbar();
}
function refuse(refusal, detail) {
  const code = refusalCode(refusal), said = refusal.message || undefined;
  if (said !== undefined) console.info(reason(code).code, said);
  diagnostic = reason(code);
  if (source.noProject) {
    console.info(diagnostic.code);
    window.Product.projectNotice(diagnostic.code);
    return;
  }
  const lost = diagnostic.code === 'EDIT_BUSY' && writer?.uncertain;
  toasts.notify(code, { refusal: true, ...(detail === undefined ? {} : { detail }),
    ...(said ? { sentence: String(said) } : {}),
    ...(lost ? { action: { name: 'connection', label: 'Reconnect' } } : {}) });
  source.updateStatusbar();
}
const unwrapped = value => value?.arm !== undefined ? value.value : value;
function commandTarget(command) {
  const at = verbOf(command.kind)?.at(command) ?? unwrapped(command.mount ?? command.actor ?? command.scope);
  if (typeof at === 'string') return at;
  if (at?.local !== undefined) return addressPath(at);
  if (at?.from) return `${addressPath(at.from.actor)}.${at.from.port} → ${addressPath(at.to.actor)}.${at.to.port}`;
  if (Array.isArray(at)) return '/' + scopeLabel(at).split(' / ').join('/');
}
const refusalAt = value => typeof value === 'string' ? value : JSON.stringify(value, (_key, item) =>
  typeof item === 'bigint' ? (Number.isSafeInteger(Number(item)) ? Number(item) : String(item))
    : item instanceof Uint8Array ? [...item] : item);
const epochVerbs = new Set(['BeginEpoch', 'ValidateEpoch', 'CommitEpoch']);
const commandText = command => [command.kind, commandTarget(command)].filter(Boolean).join(' ');
function refusedDetail(command, at, commands = []) {
  const held = command && epochVerbs.has(command.kind) && commands.length
    ? (commands.length === 1 ? commandText(commands[0]) : `${commands.length} commands`) : null;
  const parts = [command && commandText(command), held, at != null && `at ${refusalAt(at)}`].filter(Boolean);
  return parts.length ? parts.join(' · ') : undefined;
}
const byDomId = new WeakMap();
const original = id => {
  let index = byDomId.get(graph.nodes);
  if (!index) byDomId.set(graph.nodes, index = new Map(graph.nodes.map(n => [domId(n.id), n])));
  return index.get(id);
};
let named, naming;
function keepInPalette(type,at,local) {
  const a=window.StudyApp, box=globalThis.document?.querySelector?.('#palette-input');
  if (!a?.openPalette || !box) return;
  a.openPalette();
  naming={type,at};
  box.value=local; box.select?.();
}
function bindPaletteName(root) {
  const box=root.querySelector?.('#palette-input'), palette=root.querySelector?.('#palette');
  if (!box?.addEventListener || !palette?.addEventListener) return;
  palette.addEventListener('close',()=>{ naming=undefined; });
  box.addEventListener('keydown',e=>{
    if (!naming || e.key!=='Enter') return;
    e.preventDefault(); e.stopImmediatePropagation();
    const {type,at}=naming;
    named=box.value.trim();
    palette.close();
    createActorGesture(type,at);
  },true);
}
function renderGraph() { projectRun(window.StudyApp); window.StudyApp.renderGraph(); }
const drawnScopes=() => new Map(Object.entries(window.STUDY ?? {}).filter(([,scope]) => Array.isArray(scope?.nodes))
  .map(([name,scope]) => [name,new Set(scope.nodes.map(n => n.id))]));
function projectRun(a) {
  if (!a?.state) return;
  const stopped=runPause(graph?.healthPage) !== null;
  a.state.paused=new Set(stopped ? Object.keys(window.STUDY ?? {}).filter(scope=>window.STUDY[scope]?.nodes) : []);
}
const sessionCode = () => ended?.by === 'attach' || sessionLost.has(endedCode()) ? endedCode() : undefined;
function lifecycleCode(force) {
  if (sessionCode() !== undefined) return sessionCode();
  if (window.StudyApp?.historical?.()) return 'EDIT_UNAVAILABLE';
  if (graph?.healthDiagnostic != null) return graph.healthDiagnostic;
  const anchor=graph?.healthPage?.anchor;
  if (!anchor) return 'unobserved';
  if (anchor.lifecycle === 'stopped' && !force)
    return graph.anchor?.authoringRevision?.revision ? undefined : 'EDIT_BASELINE_UNAVAILABLE';
}
function keepSelection(a) {
  if (!original(a.state.selected)) a.state.selected=null;
  if (a.state.selectedSet) a.state.selectedSet=new Set([...a.state.selectedSet].filter(id=>original(id)));
}
const PORT_ROWS=new Set(['actor','edge','template']);
function commitPorts(held,commit) {
  const rows=commit.delta.flatMap(keysOf);
  const only=new Set(rows.filter(([kind]) => kind === 'actor').map(([,at]) => key(at))
    .filter(id => held.declared.actors.get(id)?.declaration));
  if (!rows.some(([kind]) => PORT_ROWS.has(kind))) return {only};
  const actors=[...only].map(id => held.declared.actors.get(id)).map(entry => ({kind:'UpsertActor',actor:address(entry.address),declaration:entry.declaration}));
  return {only,ports:readPorts(session,[],actors,held.catalog)};
}
const programPorts=(held,{ports,only}) => ports?.then(read => ({...read,admissions:new Map([
  ...held.nodes.filter(n => !only.has(n.id)).map(n => [n.id,held.observed.actors.get(n.id)?.admission]).filter(([,a]) => a),
  ...read.admissions])}));
function dispatch(event) {
  const before=graph, answer=reduce(graph,event);
  graph=answer.graph;
  if (event.kind === 'scene' || event.kind === 'commit') {
    const changed = !before?.sdkProgram
      || !sameValue(before.anchor.authoringRevision, graph.anchor.authoringRevision)
      || !sameValue(before.declarationCut, graph.declarationCut);
    if (event.kind === 'commit' && answer.changed) Object.assign(answer,commitPorts(graph,event.commit));
    graph={...graph,sdkProgram:changed ? projectSDKProgram(graph,programPorts(graph,answer)) : before.sdkProgram};
    if (changed) {
      const projection=graph.sdkProgram, app=window.StudyApp;
      projection.ready.then(() => {
        if (graph?.sdkProgram === projection && globalThis.window?.StudyApp === app && app?.state?.tab === 'sdk')
          app.renderInspector?.();
      });
    }
  }
  if (bannerFact && !graph.declared.actors.get(bannerFact.actor)?.declaration) { bannerFact=undefined; window.Product?.unbanner?.(); }
  viewer.keep(id => original(id) !== undefined);
  for (const [id,preview] of viewer.previews())
    for (const [kind,at] of Object.entries(preview)) if (carries(original(id),at)) viewer.release(id,kind);
  render(before, answer.touched);
  return answer;
}
function applyLiveObservation(event) {
  const before=bannerBaseline, answer=dispatch(event);
  if (graph.healthDiagnostic != null) return answer;
  if (bannerFact && !holds(bannerFact,graph.nodes)) { bannerFact=undefined; window.Product?.unbanner?.(); }
  if (before) {
    const fresh=newReasons(before.nodes,graph.nodes).at(-1);
    if (fresh) { showBanner(window.Product?.banner,fresh.code,fresh.kind); bannerFact=fresh; }
  }
  bannerBaseline={nodes:graph.nodes};
  return answer;
}
const redraw=() => render(graph);
function reduce(held, event) {
  switch (event.kind) {
    case 'scene': {
      const scene=event.scene;
      let next={...scene,
        ...(!scene.createInputs && held?.createInputs ? {createInputs:held.createInputs} : {}),
        ...(!scene.approvals && held?.approvals ? {approvals:held.approvals} : {})};
      return {graph:joined(views.page(next,scene.journalPage))};
    }
    case 'health': return {graph:{...applyHealth(held,event.observed),...event.runtime}};
    case 'approval-queue': return {graph:joined({...held,approvals:event.rows})};
    case 'approval-decision': return {graph:joined({...held,approvals:(held.approvals ?? []).map(row => row.id !== event.id ? row
      : Object.freeze({...row,state:event.state,decision:event.decision,
        ...(event.code === undefined ? {} : {code:event.code,reason:event.reason})}))})};
    case 'ports': return {graph:applyPorts(held,event.ports,event.only)};
    case 'commit': return applyCommit(held,event.commit,{own:event.own});
    case 'arrivals': {
      const touched=new Set();
      for (const row of event.fresh) {
        touched.add(actorKey(row.actor));
        for (const container of instanceContainers(row)) touched.add(container);
      }
      let next=views.page(distribute(held,event.page,touched),event.page,touched);
      return {graph:{...next,journalPage:event.page},touched};
    }
    default: throw new TypeError(`no such event: ${event.kind}`);
  }
}
let drawn;
const geometry=['x','y','width','height'];
const heldLook=id => { const preview=viewer.card(id).preview; return {...preview?.moves,...preview?.sizes}; };
function shownStudy(projection) {
  const shown=new Map(), study={};
  for (const [name,scope] of Object.entries(projection)) {
    if (!scope?.nodes) { study[name]=scope; continue; }
    const nodes=scope.nodes.map(card => {
      const want={...card,...heldLook(card.id)}, held=drawn?.shown.get(card.id);
      const node=held?.card === card && geometry.every(k => held.node[k] === want[k]) ? held.node : want;
      shown.set(card.id,{card,node});
      return node;
    });
    const edges=scope.edges.map(edge => {
      const to=shown.get(edge.to);
      if (!to || !edge.combinators?.length) return edge;
      const dx=to.node.x-to.card.x, dy=to.node.y-to.card.y;
      return dx || dy ? {...edge,combinators:edge.combinators.map(c => ({...c,x:c.x+dx,y:c.y+dy}))} : edge;
    });
    study[name]={...scope,nodes,edges};
  }
  return {study,shown};
}
function reprojected(only) {
  const fresh=new Map(Object.values(studyFromScene(graph,only)).flatMap(scope => scope?.nodes ?? []).map(card => [card.id,card]));
  const projection={...drawn.projection};
  for (const [name,scope] of Object.entries(projection))
    if (scope?.nodes?.some(card => fresh.has(card.id))) projection[name]={...scope,nodes:scope.nodes.map(card => fresh.get(card.id) ?? card)};
  if (!opener) projection.journal=graph.nodes.flatMap(n => projectJournal({items:n.arrivals},[n],domId,recordValue));
  return projection;
}
const structure=study => Object.entries(study).filter(([,scope]) => scope?.nodes)
  .map(([name,scope]) => [name,scope.name,scope.parent,scope.segment,scope.nodes.map(n => [n.id,n.scope,n.in,n.out]),
    scope.edges.map(({combinators,...edge}) => [edge,(combinators ?? []).map(({x,y,...step}) => step)]),
    scope.notes.map(({x,y,width,height,...note}) => note)]);
const sameCard=(held,node) => held !== undefined && sameFields(held,node);
function render(before, only) {
  const a=window.StudyApp, first=!drawn, partial=!first && only !== undefined;
  const projection=partial ? reprojected(only) : studyFromScene(graph);
  const reads=first || before.catalog !== graph.catalog || (before.problems !== graph.problems && !sameValue(before.problems,graph.problems)) || before.createInputs !== graph.createInputs;
  const folded=!first && (before.declared !== graph.declared || before.anchor !== graph.anchor);
  const {study,shown}=shownStudy(projection), drawnStructure=partial ? drawn.structure : structure(study);
  const structural=reads || !sameFields(drawnStructure,drawn.structure);
  const paged=structural || folded || before.journalPage !== graph.journalPage;
  const arrivalHead=Math.max(0,...projection.journal.map(r=>r.at));
  const journal=partial ? drawn.journal : journalProjection(graph.nodes,domId,recordValue);
  if (opener) {
    if (paged) {
      study.journal=graph.journalPage === undefined ? [] : journal(graph.journalPage);
      journalCode=shownCode() ?? (graph.journalPage !== undefined && journalStatus(graph.journalPage) != null ? {label:journalStatus(graph.journalPage),
        ...(journalReason(graph.journalPage) ? {code:journalReason(graph.journalPage)} : {})}
        : headerCode(graph.journalDiagnostic));
    } else study.journal=drawn.study.journal;
  }
  const whole=structural;
  const lifecycle=!first && before?.healthPage?.anchor?.lifecycle !== graph.healthPage?.anchor?.lifecycle;
  const woken=whole ? [] : [...shown].filter(([id,held]) => lifecycle || !sameCard(drawn.shown.get(id)?.node,held.node)).map(([id]) => id);
  const moved=!first && before.journalPage !== graph.journalPage;
  if (first || whole || before.healthPage !== graph.healthPage || before.healthDiagnostic !== graph.healthDiagnostic)
    localMachine(globalThis.document);
  if (!window.STUDY) window.STUDY=study;
  else {
    for (const k of Object.keys(window.STUDY)) delete window.STUDY[k];
    Object.assign(window.STUDY,study);
  }
  drawn={projection,study,shown,structure:drawnStructure,journal};
  if (first || before.catalog !== graph.catalog) {
    if (globalThis.document?.querySelector('#palette')?.open)
      document.querySelector('#palette-input').dispatchEvent(new Event('input'));
  }
  if (whole) window.Product?.refreshOutputs?.();
  if (structural || before.approvals !== graph.approvals) projectApprovals();
  const observedHead=Math.max(arrivalHead,...study.journal.map(r=>r.at)), headBefore=source.observedHead;
  if (observedHead>source.observedHead) source.head=observedHead;
  if (graph.healthPage && graph.healthDiagnostic == null && endedCode() === undefined) seenAt=source.observedHead;
  window.STUDY_HISTORY ??= {start:0,duration:source.head,stages:[],wallClock:null};
  window.STUDY_HISTORY.duration=source.head;
  const clockMoved=publishWallClock(window.STUDY_HISTORY,graph.healthPage?.anchor);
  if (!a) return;
  if (whole) {
    a.archive.entries=window.STUDY.journal; a.archive.version++;
    a.state.journalRows=a.archive.entries;
    if(!window.STUDY[a.state.scope]?.nodes) a.state.scope='root';
    keepSelection(a);
    renderGraph(); a.timeMachine.refresh();
    window.StudyApp?.renderJournalHeader?.();
  } else {
    if (lifecycle) projectRun(a);
    if (folded) keepSelection(a);
    if (woken.length) {
      window.Product?.refreshOutputNodes?.(woken.map(id => shown.get(id).node));
      a.renderActors?.(woken);
      if (!partial) a.updateScopeHealth?.();
    }
    if (paged) {
      applyJournal(a,window.STUDY.journal);
      window.StudyApp?.renderJournalHeader?.();
    }
    if (moved) { a.updateViewers?.(source.observedHead); a.renderWires?.(); }
    if (source.observedHead !== headBefore) a.refreshFaces?.(woken);
    if (clockMoved && !paged) a.renderJournal?.();
    if (paged || clockMoved) a.timeMachine.refresh();
  }
  if (lifecycle) { a.updatePause?.(); pauseControls(document); }
  if (!partial) source.updateStatusbar();
}
function pauseControls(root) {
  for (const [selector, force] of [['#pause-button', false], ['#force-pause', true]]) {
    const control = root.querySelector(selector);
    if (!control) continue;
    const code = lifecycleCode(force);
    control.disabled = code !== undefined;
    if (code !== undefined) { control.title = reason(code).label; control.setAttribute?.('data-reason', reason(code).code); }
    else control.removeAttribute?.('data-reason');
    if (code === undefined && force) control.title = '';
  }
}
const detachedSelectors = ['#add-actor', '#new-actor-sidebar',
  '.canvas-edit-tools [data-action="group"]', '.canvas-edit-tools [data-action="note"]'];
function detachedControls(root, code) {
  for (const selector of detachedSelectors) {
    const control = root.querySelector?.(selector);
    if (!control?.dataset) continue;
    if (code !== undefined) {
      control.disabled = true;
      control.title = reason(code).label;
      control.dataset.reason = reason(code).code;
      control.dataset.detached = '1';
    } else if (control.dataset.detached) {
      delete control.dataset.detached;
      delete control.dataset.reason;
      control.disabled = Boolean(window.StudyApp?.historical?.());
      control.title = '';
    }
  }
}
function inspectorHealth(n) {
  const ended=observationEnded();
  const state=healthText(n), detail=detailText(n.issue?.detail,n.issue?.field);
  const recorded=[state,detail].filter(Boolean).join(' · ');
  const pause=ended ? null : runPause(graph?.healthPage);
  const small=[pause?.text,detail].filter(Boolean).join(' · ');
  const text=pause ? `${pause.text} · ${recorded}` : recorded;
  const health=cardFace(n,Boolean(window.StudyApp?.historical?.()),{ended,pause}).dot.health;
  return {strong:state, small, health, title:text, unavailable:health === 'unobserved'};
}
function observedHealth(observed,runtime) {
  const held=graph?.healthDiagnostic;
  applyLiveObservation({kind:'health',observed,runtime});
  timeBar?.observed(observed,runtime);
  if (observed.diagnostic && (held == null || reason(held).code !== reason(observed.diagnostic).code)) report(observed.diagnostic);
}
async function rereadDeclarations() {
  const held=shownArrivals()?.page, readEvents=held ? async () => held : undefined;
  const lens=view?.lens;
  const scene=await readScene(session,[],graph.catalogObservation,readEvents,undefined,lens,
    lens === undefined ? undefined : held?.cut ?? timeBar.transport.lens.at.cut,cardMetrics);
  (lens === undefined ? applyLiveObservation : dispatch)({kind:'scene',scene:{...await views.read(session, scene, held ?? scene.journalPage ?? {items:[]}),drawn:graph.nodes}});
}
async function editRefused(result, commands) {
  const answered=result.diagnostics?.[0];
  const refusal={...(answered ?? result.diagnostic), code:inSpace('Declaration', answered?.code) ?? result.diagnostic?.code ?? 'EDIT_UNAVAILABLE'};
  refuse(refusal, refusedDetail(result.command, answered?.at, commands));
  if (reason(refusal.code).code === revisionConflict) await rereadDeclarations();
  return refusal;
}
async function write(commands, kept=slot => history.edited(slot)) {
  const edit=writer;
  let slot;
  const refused=refusal => { refuse(refusal); return refusal; };
  try {
    if (window.StudyApp?.historical?.()) return refused({code:'EDIT_UNAVAILABLE'});
    if (!commands.length) return;
    slot=history.watch(undoEntry(graph,commands));
    const checked=await edit.prepare(graph.anchor,commands);
    if (checked.status !== 'accepted') return await editRefused(checked,commands);
    const committed=await edit.commit();
    if (committed.status !== 'accepted') return await editRefused(committed,commands);
    kept(slot);
    if (!await committed.folded) await rereadDeclarations();
    toasts.notify(committed.status,{answer:true});
    return true;
  } catch (error) { return refused({code:error.code ?? 'EDIT_UNAVAILABLE'}); }
  finally { if (slot) history.unwatch(slot); window.Product?.refreshEditTools?.(); }
}
const submit = async (commands, kept) => (await write(commands, kept)) === true || undefined;
async function undo(from) {
  const slot=history.top(from);
  if (!slot) return toasts.notify(from === 'undo' ? 'UNDO_EMPTY' : 'REDO_EMPTY',{answer:true});
  const ports=await boundaryPorts(session,graph,slot.entry);
  const {commands, report}=restore(graph,slot.entry,slot,ports);
  const told=() => {
    for (const code of new Set(report.map(r => r.code)))
      toasts.notify(code,{answer:true,detail:[...new Set(report.filter(r => r.code === code).map(r => r.intent))].join(', ')});
  };
  if (!commands.length) {
    history.spent(from);
    window.Product?.refreshEditTools?.();
    return report.length ? told() : toasts.notify('UNDO_ALREADY_RESTORED',{answer:true});
  }
  const done=await submit(commands,next => history.moved(from,next));
  if (done) told();
  return done;
}
const noteOf = id => graph.annotations?.find(note => domId(note.id) === id);
const noteEdits = {
  create() {
    const a=window.StudyApp;
    const scope=scopeNamed(a.state.scope)?.address;
    if (!scope) return refuse({code:'EDIT_UNAVAILABLE'});
    return submit([createNote(scope,'note-'+crypto.randomUUID())]);
  },
  body(id,text) { const note=noteOf(id); if (!note) return refuse({code:'EDIT_UNAVAILABLE'}); if (note.body!==text) return submit([noteBody(note,text)]); },
  async place(id, changes, cancelled) {
    const note=noteOf(id), shown=window.StudyApp.graph().notes.find(n=>n.id===id);
    if (!note || !shown) return refuse({code:'EDIT_UNAVAILABLE'});
    const owner={annotation:address(note.address)}, from={...note,x:shown.x,y:shown.y,width:shown.width,height:shown.height};
    const commands=cancelled ? [] : 'x' in changes ? dragMove(from,changes.x,changes.y,owner) : dragResize(from,changes.width,changes.height,owner);
    if (commands.length) await submit(commands);
    redraw();
  },
  retire(id) { const note=noteOf(id); if (!note) return refuse({code:'EDIT_UNAVAILABLE'}); return submit([retireNote(note)]); },
};
const boxOf=n => footprint(n,graph.metrics ?? UNMEASURED);
const landing=(scope,asked,leaving) => landingIn(graph,scope,asked,leaving);
function newCardPlace(type,local,scope,at) {
  const size=newCardSize(type,local,cardMetrics),row=graph.catalog?.find(r=>r.actor_type===type);
  const reach=(side,ports=[])=>captionReach(side,ports.map(p=>p.id),cardMetrics ?? UNMEASURED);
  return landing(key(scope),[{x:at.x,y:at.y,w:size.w,h:size.h,left:reach('in',row?.in_ports),right:reach('out',row?.out_ports)}])[0];
}
function organizeCode() {
  if (sessionCode() !== undefined) return sessionCode();
  if (!graph?.anchor) return 'EDIT_BASELINE_UNAVAILABLE';
  if (window.StudyApp?.historical?.()) return 'EDIT_UNAVAILABLE';
  if (writer?.busy || writer?.pending || writer?.uncertain) return 'EDIT_BUSY';
}
async function organizeScope() {
  if (writer?.uncertain) await writer.settle();
  const code=organizeCode();
  if (code) return refuse({code});
  const a=window.StudyApp;
  const scope=scopeNamed(a.state.scope);
  const nodes=graph.nodes.filter(n=>n.scope===scope?.id);
  if (!nodes.length) return;
  const targets=organizeTargets(nodes,graph.edges,ORIGIN,graph.metrics ?? UNMEASURED);
  const commands=organize(nodes,targets);
  if (!commands.length) { toasts.notify('ORGANIZE_NOTHING_TO_MOVE',{answer:true}); return true; }
  const kept=[];
  for (const n of nodes) {
    const at=targets.get(n.id);
    if (carries(n,at)) continue;
    viewer.hold(domId(n.id),'moves',at);
    kept.push([domId(n.id),at]);
  }
  if (kept.length) redraw();
  const answer=submit(commands);
  organizeControl?.refresh();
  try { return await answer; }
  finally {
    for (const [id,at] of kept) viewer.release(id,'moves',at);
    if (kept.length) redraw();
    organizeControl?.refresh();
  }
}
function sdkView(n) { return sdkProgram(graph, original(n.id).address); }
const anySlots = config => ({slots:Object.keys(config).map(key=>({key,shape:{kind:'Any'}}))});
export function configFromForm(config, form, fields) {
  if (!config || typeof config !== 'object' || Array.isArray(config)) return config;
  return readConfigForm(config, form, fields ?? configFieldList(config, true, anySlots(config)).fields);
}
const STRUCTURE=Symbol('draft structure');
const draftBase=id => viewer.card(id).draft?.raw?.[STRUCTURE] ?? original(id)?.declaration.config;
const readFields=id => { const list=fieldsOf(id); return list.fields.length ? list.fields : undefined; };
function fieldsOf(id, base=draftBase(id)) {
  const node=original(id), row=graph?.catalog?.find(r=>r.actor_type===node?.declaration.actorType);
  return configFieldList(base, configDeclared(row?.config_schema), graph?.createInputs?.entries.get(node?.declaration.actorType),
    graph?.createInputs?.diagnostic ?? 'READ_UNAVAILABLE');
}
function bindConfigStructure(root) {
  root.addEventListener?.('click', e => {
    const all=e.target?.closest?.('[data-approval-all]');
    if (all?.closest?.('#config-form,[data-node-config]')) { e.preventDefault(); setAllApprovals(all); return; }
    const button=e.target?.closest?.('[data-config-insert],[data-config-remove]');
    const form=button?.closest?.('#config-form,[data-node-config]');
    if (!form) return;
    const id=form.dataset.nodeConfig || window.StudyApp?.state?.selected, keyName=button.dataset.configKeyName;
    source.configStructure(form, id, {insert:button.dataset.configInsert, remove:button.dataset.configRemove,
      key:button.dataset.configKey ?? (keyName ? form.elements.namedItem(keyName)?.value : undefined)});
  });
  root.addEventListener?.('change', e => {
    const select=e.target?.closest?.('[data-config-kind]');
    const form=select?.closest?.('#config-form,[data-node-config]');
    if (!form) return;
    source.configStructure(form, form.dataset.nodeConfig || window.StudyApp?.state?.selected,
      {retype:select.dataset.configKind, kind:select.value});
  });
}
async function placeActor(registration,at,wire,local,config,preprocess) {
  const type=registration.actor_type, other=wire && original(wire.node);
  const scope=scopeNamed(window.StudyApp.state.scope)?.address;
  try {
    const result=await createActor(session,graph,scope,local,registration,config,{bypass:false,mute:false,pause:false});
    if(result.status!=='accepted') {
      const answered=result.diagnostics?.[0];
      const refusal={...answered, code:inSpace('Query', answered?.code) ?? 'ADMISSION_UNAVAILABLE'};
      refuse(refusal, refusedDetail({kind:'actor.create-admission',actor:{scope,local}}, answered?.at));
      return refusal;
    }
    let edge=[];
    if (wire) {
      const ports=wire.side==='out' ? result.ports.in : result.ports.out;
      const port=ports.find(p=>p.primary) ?? ports[0];
      if (!port) { const refusal={code:'CONNECT_PORT_REQUIRED'}; refuse(refusal); return refusal; }
      const made={address:result.value.actor.value};
      edge=[wire.side==='out' ? connect(graph,other,wire.name,made,port.id,preprocess) : connect(graph,made,port.id,other,wire.name,preprocess)];
    }
    const view=views.of(views.viewOf(undefined,type).kind);
    const output=result.ports.out.find(p=>p.primary) ?? result.ports.out[0];
    const mount=view.traits.injects && output
      ? [mountRequest(result.value.actor.value,output.id)] : [];
    const put=at && newCardPlace(type,local,scope,at);
    const written=await write([result.value,...(put ? [move({address:result.value.actor.value},put.x,put.y)] : []),...edge,...mount]);
    if (written !== true) return written;
    const a=window.StudyApp;
    a.selectNode(domId(graph.nodes.find(n=>n.address.local===local).id));
    a.state.tab='configure';
    a.renderInspector();
    return true;
  } catch(error) {
    const refusal={code:error.code ?? 'EDIT_UNAVAILABLE'};
    refuse(refusal);
    return refusal;
  }
}
function makeSource() {
  let catalogRows, catalogShown;
  const port = {
    get projection() { return window.STUDY; },
    get metrics() { return cardMetrics ?? UNMEASURED; },
    outletReading,
    get noProject() { return noProject.has(attachCode()); },
    updateStatusbar() {
      const code = standingCode();
      const project = source.product.projects.find(value => value.id === source.product.project);
      const a = window.StudyApp;
      const past = code == null && a?.historical?.();
      const set = graph && healthSet(graph, scopeAddress(a?.state?.scope));
      const text = past
        ? ['Recorded state', window.studyTimeFormat(a.displayTime())]
        : statusbarText(set, graph?.healthPage, code, graph?.problems, endedCode() === undefined, lastSeen());
      const back = code === undefined || code === null ? rejoinedFact(rejoined) : null;
      updateStatusbar(document, [...(Array.isArray(text) ? text : [text]), back].filter(Boolean), project?.name ?? window.STUDY?.root?.name,
        past ? 'history' : healthReading(set, graph?.healthPage, code).state);
    },
    openState(path) {
      if (path===source.product.project && ended === null)
        return window.StudyApp?.setView?.('canvas');
      if (path===source.product.project) { window.StudyApp?.setView?.('canvas'); return reconnect(); }
      return window.circularConnection?.({state:path});
    },
    async newProject() {
      let answer;
      try { answer=await window.circularConnection?.({create:true}); }
      catch (error) { answer={code:error.code ?? 'BRIDGE_UNAVAILABLE'}; }
      if (!answer) return refuse({code:'BRIDGE_UNAVAILABLE'});
      if (answer.code) return refuse({code:answer.code});
      return answer.created===true;
    },
    async openProject() {
      let answer;
      try { answer=await window.circularConnection?.({open:true}); }
      catch (error) { answer={code:error.code ?? 'BRIDGE_UNAVAILABLE'}; }
      if (!answer) return refuse({code:'BRIDGE_UNAVAILABLE'});
      if (answer.code) return refuse({code:answer.code});
      return answer.opened===true;
    },
    get catalogAnchor() { return graph?.catalogObservation.anchor; },
    get healthPage() { return graph?.healthPage; },
    get healthDiagnostic() { return graph?.healthDiagnostic; },
    get problems() { return graph?.problems; },
    catalog() {
      const rows = graph?.catalog ?? [];
      if (rows !== catalogRows) {
        catalogRows = rows;
        catalogShown = catalogItems(rows, catalogIcon, type => window.LiveViewers?.select({type}).kind ?? 'basic', catalogGroup);
      }
      return catalogShown;
    },
    choices(n) { return window.LiveViewers?.choices(n, graph?.createInputs?.entries.get(n.type)?.slots?.map(slot => slot.key)); },
    scopeHealth(scope) {
      return scopeHealth(graph && healthSet(graph, scopeAddress(scope)), graph?.healthPage, standingCode(), lastSeen());
    },
    observing() { return following?.observing === true; },
    endedText() { return null; },
    observedHead:0,
    get head() { return this.observedHead; },
    set head(value) { this.observedHead=value; },
    refuse, updateViewer,
    editHistory() { const size=history.size; return {undo:size.undo > 0, redo:size.redo > 0}; },
    codeLabel:code => ({...reason(code), label:reasonText(code)}),
    notice(code) { toasts.notify(code,{answer:true}); },
    outputs() { return outputsFromScene(graph, domId, endedCode(), observationEnded()); },
    renderSurface(surface, page) { return renderSurface(surface, { ...page, viewer }); },
    updateSurface,
    bindSurfaces(root) { bindSurfaceInputs(root, { surfaces: () => port.outputs().surfaces, viewer, inject: injectExport }); },
    refreshApprovalBadge() { bindProductUtility(document, approvalPage, approvalCode ?? attachCode()); },
    stepKinds() {
      readCombinators();
      return {kinds:preprocessKinds, code:preprocessKinds.length ? undefined : codeText(attachCode() ?? preprocessCode)};
    },
    hold(kind,id,value) {
      viewer.hold(id,kind,value);
      const held=drawn?.shown.get(id);
      if (held) Object.assign(held.node,heldLook(id));
      return held?.node;
    },
    landing(type,at) {
      const scope=scopeNamed(window.StudyApp.state.scope)?.address;
      return scope ? newCardPlace(type,freshLocal(graph,scope,type),scope,at) : at;
    },
    face(n) {
      const recorded=Boolean(window.StudyApp?.historical?.());
      return {...cardFace(n, recorded,
        {rate:recordedRate(shownArrivals()?.tally, n.id, source.observedHead), ended:observationEnded(), pause:runPause(graph?.healthPage)}),
        revisionNote:recorded && n.earlierRevision ? 'At this time this actor was still running an earlier revision' : null};
    },
    inspectorObservations: () => observationsLine(endedCode()),
    journalLine: past => {
      const live=endedCode() === undefined && !journalCode, idle=past ? 'At selected time' : live ? 'Following arrivals' : 'Not following arrivals';
      return {...(endedCode() !== undefined && journalCode?.code !== undefined
        ? {label:(graph?.journalPage && journalStatus(graph.journalPage)) || idle, code:journalCode.code, title:''}
        : journalCode ? {...journalCode, title:journalCode.label}
          : {label:idle, title:''}), live};
    },
    identityLine: actorLine,
    actorAddress: n => n.path,
    healthBanner: n => inspectorHealth(n),
    portTitle,
    instances(n) {
      const actor = original(n.id);
      return actor ? views.of('instances').input(actor, {instances:graph.instances}).instancesView
        : {code:'READ_UNAVAILABLE'};
    },
    inspectorAccess(n) {
      const actor=original(n?.id);
      if (!actor) return {};
      const historical=Boolean(window.StudyApp?.historical?.()), reading=heldReading();
      return {permissions:historical ? undefined : heldPermissions(actor, reading.accessValue),
        harness:harnessSectionHTML(actor, reading, {historical, choose:Boolean(window.circularConnection)})};
    },
    mailbox(n) {
      const actor=original(n?.id);
      if (!actor) return {rows:[]};
      const past=Boolean(view || window.StudyApp?.historical?.()), depths=inletDepths(depthsOf(session), graph.edges, actor.id, {past});
      drawnDepths=past ? undefined : identity(depths);
      if (!depths.rows) return depths;
      const node=key => graph.nodes.find(node => node.id === key);
      const outlet=(key, port) => node(key)?.out?.find(([id]) => id === port)?.[3] ?? port;
      return {rows:depths.rows.map(row => ({...row, origin:`${authoredName(node(row.from)?.address)}.${outlet(row.from, row.out)}`}))};
    },
    afterInspector() {
      bindHarnessForms(document, 'data-bind-agent-harness', () => ({context:harnessContext(), redraw:redrawInspector}));
    },
    afterEdgeInspector(edge) {
      updateWireInspector(document,edge);
    },
    product:{projects:[],project:null,approvals:[]},
    attach({renderProjects}={}) { redrawProjects=renderProjects; },
    projectState(path) { return stateLine(path, source.product.project, endedCode()); },
    recentRefusal() { return recentRefusal(recentCode); },
    projectDaemon() { return projectDaemonHTML(projectFacts()); },
    projectPane() { return projectPaneHTML(source.product.project, projectFacts()); },
    actions:{
      connection:() => { void reconnect(); },
      'start-daemon':() => { void reconnect(true); },
      'stop-daemon':() => { void stopDaemon(); },
      'restart-daemon':() => { void stopDaemon(true); },
      'new-project':() => port.newProject(),
      'open-project':() => port.openProject(),
      organize:() => organizeScope(),
      harnesses:() => openHarnesses(),
    },
    evidenceTarget:r => ['record', `${r.actor}:${r.index}`],
    acceptedRecords(scope, actors) {
      if (view || Boolean(window.StudyApp?.historical?.())) return [];
      const address = scope === undefined ? undefined : scopeNamed(scope)?.id;
      return acceptedEdits.filter(row => (scope === undefined || row.scopes.includes(address))
        && (actors === undefined || row.actors.some(id => actors.includes(id))));
    },
    afterPalette() {
      const wire=window.StudyApp?.state;
      bindCatalogAvailability(document,window.ProductCatalog?.items ?? [],Boolean(wire?.addOnWire && wire?.pendingPort),
        graph?.createInputs?.entries,graph?.createInputs?.diagnostic ?? undefined);
    },
    afterProjects() { projectsView?.(); },
    settingsDeclared(n) { return source.configFields(n.id).declared; },
    rate(edge,t) { return wirePerSecond(shownArrivals()?.tally,edge.wire,t); },
    rateSeconds: WIRE_RATE_SECONDS,
    actorRecords(id) {
      const node=original(id);
      if (!node || !graph) return [];
      return projectJournal({items:node.arrivals ?? []},graph.nodes,domId,recordValue).sort((x,y) => y.at-x.at);
    },
    rawObservation(id) {
      const node=original(id), latest=latestRow(node?.arrivals), row=node?.health;
      return display({
        health:row ? {state:row.state, reason:row.reason ?? null, detail:row.detail ?? null, since_ms:row.since_ms?.value ?? row.since_ms ?? null} : null,
        latest:latest ? {index:latest.index ?? null, observed_at_ms:latest.observed_at_ms ?? null, port:latest.port ?? null,
          value:latest.body === undefined ? recordValueUnavailable : recordValue(latest.body)} : null,
      });
    },
    selectionRecords(ids) { return selectionRecords(ids); },
    recordOf(record) {
      const [actor]=record.split(':');
      const named=row => `${row.actor}:${row.index}` === record;
      return (selection.rows ?? []).find(named) ?? this.actorRecords(actor).find(named);
    },
    edgeValue(e) {
      const edge=graph?.edges.find(x=>domId(x.id)===e.id);
      const row=edge && wireArrival(edge,shownArrivals()?.page);
      if (row?.body === undefined) return undefined;
      return valueText(row.body).text;
    },
    observation(n,t) {
      const sample = views.of(n.viewKind?.kind).sample?.(n,t);
      return {value:sample && 'value' in sample ? sample.value : recordedPerSecond(shownArrivals()?.tally,n.id,t)};
    },
    arrivalsIn(id,from,to) { return shownArrivals()?.tally?.within(id,from,to) ?? null; },
    arrivalSeries(id,end,count) { return shownArrivals()?.tally?.series(id,end,count) ?? null; },
    permissions() { return permissionsView(); },
    configFields(id) {
      const list=fieldsOf(id);
      return {...list, ...(list.code ? {code:reason(list.code)} : {}),
        fields:list.fields.map(f=>f.code ? {...f,code:reason(f.code)} : f)};
    },
    configForm(n,draft) { return configFormHTML(source.configFields(n.id),n,draft,viewer.card(n.id).draft?.raw,reason,viewer.card(n.id).submission); },
    configStructure(form,id,op) {
      const a=window.StudyApp, list=fieldsOf(id);
      let next;
      try { next=changeStructure(configFromForm(draftBase(id),form,readFields(id)),list.fields,op); }
      catch (error) { return refuse({code:error.code, message:error.message}); }
      const raw=Object.fromEntries(Object.entries(viewer.card(id).draft?.raw ?? {}).filter(([k])=>!k.startsWith('[') && op.retype!==JSON.stringify([k])));
      viewer.draft(id,{raw:{...raw,[STRUCTURE]:next},shown:display(next)});
      a.renderActors([id]);
    },
    readDraft(form,n) {
      try { return display(configFromForm(draftBase(n.id),form,readFields(n.id))); }
      catch (error) { throw reason(error.code); }
    },
    async sdkText(n) {
      const shown=graph, actor=original(n.id).address;
      await shown.sdkProgram.ready;
      return sdkProgram(shown,actor).text;
    },
    sdkProgram(n) { return sdkView(n); },
    afterGraph() {
      const strip=document.querySelector('#connection-strip');
      const observationCode=endedCode();
      const canvas=document.querySelector('#canvas');
      if (canvas?.dataset) {
        if (observationCode === undefined) delete canvas.dataset.observation;
        else canvas.dataset.observation='ended';
      }
      if (strip) {
        strip.hidden=observationCode === undefined;
        const label=strip.querySelector?.('span');
        if (label && observationCode !== undefined) {
          const status=reason(observationCode), waiting=reattach.waiting;
          label.textContent=[status.label, graph && `last observation ${lastSeen()}`,
            waiting && 'reattaching when the daemon answers'].filter(Boolean).join(' · ');
          label.setAttribute?.('data-reason', status.code);
          label.title=waiting?.tried ? `last try ${reason(waiting.tried).label}` : '';
          if (waiting?.tried) label.setAttribute?.('data-last-try', reason(waiting.tried).code); else label.removeAttribute?.('data-last-try');
          const button=strip.querySelector?.('button'), start=noDaemon.has(status.code) || noDaemon.has(waiting?.tried);
          const open=noProject.has(status.code);
          if (button) { button.textContent=open ? 'Open project' : start ? 'Start daemon' : 'Reconnect';
            button.dataset.action=open ? 'open-project' : start ? 'start-daemon' : 'connection'; }
        }
      }
      const empty=document.querySelector('#canvas-empty'), scopeNodes=window.StudyApp?.graph?.()?.nodes;
      if (empty) empty.hidden=!(graph && Array.isArray(scopeNodes) && scopeNodes.length === 0);
      const line=empty?.querySelector?.(':scope > span');
      if (line) line.textContent=reasonText(empty.dataset.reason);
      localMachine(document);
      updateConnectionEvidence(document, connectionEvidence);
      const shownFacts={code:observationCode, tried:reattach.waiting?.tried, revision:graph?.anchor?.authoringRevision};
      if (!projectsShown || projectsShown.code !== shownFacts.code || projectsShown.tried !== shownFacts.tried
        || !sameValue(projectsShown.revision, shownFacts.revision)) { projectsShown=shownFacts; redrawProject(); }
      organizeControl?.refresh();
      window.StudyApp?.renderJournalHeader?.();
      pauseControls(document);
      detachedControls(document, observationCode);
      markIssueLayer(document,graph?.problems);
      const a=window.StudyApp;
      if (!a) return;
      window.Product?.showCompatibility?.();
      a.refreshFaces?.();
      source.updateStatusbar();
      a.updateScopeHealth?.();
    },
    perform:performer({
      createActor:({type,at,connectFrom,preprocess}) => createActorGesture(type,at,connectFrom,preprocess),
      retireActors:({actors}) => submit(removeActors(graph,actors.map(original))),
      connect:({outlet,inlet,preprocess}) => connectGesture(outlet,inlet,preprocess),
      retireEdge:({edge}) => retireEdgeGesture(edge),
      moveActors:({moves,cancelled}) => drop('moves',moves.map(({actor,x,y}) => ({actor,at:{x,y}})),cancelled),
      resize:({actor,width,height,cancelled}) => drop('sizes',[{actor,at:{width,height}}],cancelled),
      alignTops:({actors}) => alignTops(actors),
      group:({actors,into,type,name}) => groupGesture(actors,into,type,name),
      configure:({actor,form}) => configureGesture(actor,form),
      setFlag:({actor,flag}) => setFlagGesture(actor,flag),
      rename:({actor,label}) => renameGesture(actor,label),
      setView:({actor,view}) => { const n=original(actor); return submit([present(n,{view:{kind:view,config:n.presentation.view?.config ?? null}})]); },
      addStep:({edge,step,at}) => addStepGesture(edge,step,at),
      applyStep:({edge,step,config}) => applyStepGesture(edge,step,config),
      moveStep:({edge,index,direction}) => stepOrder(edge,index,direction),
      removeStep:({edge,index}) => stepOrder(edge,index,null),
      inletSettings:({edge,values}) => inletSettingsGesture(edge,values),
      createNote:() => noteEdits.create(),
      noteBody:({note,text}) => noteEdits.body(note,text),
      retireNote:({note}) => noteEdits.retire(note),
      moveNote:({note,x,y,cancelled}) => noteEdits.place(note,{x,y},cancelled),
      resizeNote:({note,width,height,cancelled}) => noteEdits.place(note,{width,height},cancelled),
      togglePause:({force}) => lifecycle(force),
      inject:({actor,entered,line}) => injectGesture(actor,entered,line),
      decide:({request}) => decideGesture(request),
      undo:() => undo('undo'),
      redo:() => undo('redo'),
    }),
  };
  timeBar=timeReading(() => session, port, () => window.STUDY_HISTORY,
    () => (window.STUDY[window.StudyApp?.state.scope ?? 'root']?.nodes ?? []).map(n => original(n.id).address),
    () => shownArrivals()?.tally);
  return Object.assign(port, {transport:timeBar.transport, timeBar:timeBar.bar,
    lensOpened:lens => { void inTurn(() => openView(lens)); },
    lensMoved:lens => { void inTurn(() => view?.lens === lens && readView(view)); },
    recordedState:() => view?.snapshotCode == null ? null
      : view.snapshotCode.space === 'Query' && Number(view.snapshotCode.code) === 2
        ? 'Nothing was recorded before this point.' : reason(view.snapshotCode).label,
    lensEnded:lens => closeView(lens)});
}
function alignTops(ids) {
  const nodes=ids.map(original).filter(Boolean),top=Math.min(...nodes.map(n=>n.y));
  if (!nodes.length) return refuse({code:'EDIT_UNAVAILABLE'});
  const places=landing(nodes[0].scope,nodes.map(n => ({...boxOf(n),y:top})),new Set(nodes.map(n => n.id)));
  return submit(nodes.flatMap((n,i)=>dragMove(n,places[i].x,places[i].y)));
}
function connectGesture(outlet,inlet,preprocess) {
  const from=original(outlet.node), to=original(inlet.node);
  if (!from || !to) return refuse({code:'EDIT_UNAVAILABLE'});
  return submit([connect(graph,from,outlet.name,to,inlet.name,preprocess)]);
}
async function retireEdgeGesture(id) {
  const edge=graph.edges.find(e=>domId(e.id)===id);
  if (!edge) return refuse({code:'EDIT_UNAVAILABLE'});
  return submit([disconnect(edge)]);
}
async function stepOrder(id,index,direction) {
  const edge=graph.edges.find(e=>domId(e.id)===id);
  if (!edge) return refuse({code:'EDIT_UNAVAILABLE'});
  const command=changeCombinator(edge,index,direction);
  if (!command || !await submit([command])) return;
  const a=window.StudyApp;
  if (a.state.edge!==id) return;
  const steps=a.graph().edges.find(e=>e.id===id)?.combinators ?? [];
  a.state.component=steps[direction===null ? Math.min(index,steps.length-1) : index+direction]?.id ?? null;
  a.renderWires();a.renderInspector();
}
async function applyStepGesture(id,step,config) {
  const a=window.StudyApp, edge=graph.edges.find(e=>domId(e.id)===id);
  const shown=a.graph().edges.find(e=>e.id===id);
  if (!edge || !shown) return refuse({code:'EDIT_UNAVAILABLE'});
  const draft=a.state.draftStep?.id===step.id;
  const index=draft ? shown.combinators.length : shown.combinators.findIndex(c=>c.id===step.id);
  let command;
  try { command=changeStep(edge,index,step.kind,config,preprocessKinds); }
  catch(error) { return refuse({code:error.code}); }
  if (!await submit([command])) return;
  if (draft) {
    a.state.draftStep=null;
    a.state.editComponent=a.state.component=a.graph().edges.find(e=>e.id===id)?.combinators[index]?.id ?? null;
  }
  a.renderWires();a.renderInspector();
  return true;
}
function addStepGesture(id,kind,at) {
  const a=window.StudyApp;
  if (a.historical()) return refuse({code:'EDIT_UNAVAILABLE'});
  if (!a.graph().edges.some(e=>e.id===id)) return a.gestureCode('COMBINATOR_WIRE_REQUIRED');
  if (!preprocessKinds.includes(kind)) return refuse({code:'EDGE_PREPROCESS_STEP'});
  const put=draftChipPlace(at,a.graph().nodes,chipRoom(cardMetrics ?? UNMEASURED),cardMetrics ?? UNMEASURED);
  a.state.draftStep={id:`${id}-draft-${kind}`,edge:id,kind,config:{},x:put.x,y:put.y};
  a.state.editComponent=a.state.draftStep.id;
  a.state.component=null;
  a.renderWires();a.renderInspector();
}
async function inletSettingsGesture(id,values) {
  const a=window.StudyApp, edge=graph.edges.find(e=>domId(e.id)===id);
  const shown=a.graph().edges.find(e=>e.id===id);
  const command=edge && shown && changeInletSettings(edge,shown.declaredDelay,values);
  if (!command) return refuse({code:'EDIT_UNAVAILABLE'});
  if (!await submit([command])) return;
  if (a.state.edge===id) a.renderInspector();
}
async function configureGesture(id,form) {
  const a=window.StudyApp, node=original(id);
  if (viewer.card(id).submission?.status === 'submitting') return;
  const answer=(status,code,message=reason(code).label) => {
    viewer.change(id,{submission:{status,code,message}});
    a.renderActors([id]);
  };
  const refused=refusal => {
    const {text,code,said}=refusalText(refusal), path=configPath(refusal.at);
    const field=path ? source.configFields(id).fields.find(f => f.key === String(path[0]))?.key : undefined;
    viewer.change(id,{submission:{status:'rejected',code,message:text,...(said === undefined ? {} : {said}),...(field === undefined ? {} : {field})}});
    a.renderActors([id]);
  };
  let command;
  try { command=configure(node,configFromForm(draftBase(id),form,readFields(id))); }
  catch(error) { const refusal={code:error.code, message:error.message}; refuse(refusal); return refused(refusal); }
  viewer.change(id,{submission:{status:'submitting',code:'',message:'Submitting this actor’s configuration…'}});
  a.renderActors([id]);
  const written=await write([command]);
  if (written !== true) return refused(written ?? {code:'EDIT_UNAVAILABLE'});
  viewer.change(id,{draft:undefined});
  answer('accepted','accepted');
  renderGraph();
}
async function injectGesture(id,entered,line) {
  const n=original(id), view=n && views.of(n.viewKind.kind);
  const asked=view?.submit?.(n,entered,{mounts:graph.exportMounts});
  if (!asked || asked.code) return refuse({code:asked?.code ?? 'INLET_INJECTION_UNAVAILABLE'});
  const {mount,payload}=asked;
  const result=await injectAtMount(mount,payload);
  if (result.shown) {
    view.answered?.(id,result.shown);
    if (line) { line.textContent=result.shown.text; line.title=result.shown.title; line.setAttribute?.('data-reason', result.shown.code); }
  }
  return result.accepted;
}
async function injectExport(id,payload) {
  const mount=graph?.exportMounts.find(m => domId(key(m.address)) === id);
  const code=endedCode() ?? (!mount?.declaration.roles.request ? 'INLET_INJECTION_UNAVAILABLE' : null);
  if (code) return {code};
  return injectAtMount(mount.address,payload);
}
async function injectAtMount(mount,payload) {
  let answer;
  try { answer=await session.interactions.inject({mount,payload,idempotency:crypto.getRandomValues(new Uint8Array(16))}); }
  catch(error) { const code=error.code ?? 'READ_UNAVAILABLE'; refuse({code}); return {accepted:false,code}; }
  const answered=answer.diagnostics?.[0];
  const refused=answer.status === 'accepted' ? 'accepted' : inSpace('EventInjection', answered?.code);
  const code=reason(refused);
  const shown={text:code.label, title:code.label, code:code.code};
  if (answer.status !== 'accepted') refuse({...answered, code:refused}, refusedDetail({kind:'Inject',mount}, answered?.at));
  return {accepted:answer.status === 'accepted',code:refused,shown};
}
async function lifecycle(force) {
  const code=lifecycleCode(force);
  if (code !== undefined) return refuse({code});
  const anchor=graph.healthPage.anchor;
  const [verb, request]=anchor.lifecycle === 'stopped' && !force
    ? ['Resume', {expectedAuthoringRevision:graph.anchor.authoringRevision.revision}]
    : ['Pause', {mode:force ? 'ForcePause' : 'Pause'}];
  try {
    const answer=await session.exchange('Lifecycle', verb, request);
    const result=decoded(() => lifecycleResultFromValue(answer.payload), 'RESULT_UNEXPECTED');
    if (result.status !== 'accepted') return refuse({...result.diagnostics[0], code:inSpace('Lifecycle', result.diagnostics[0].code)});
    toasts.notify(result.value.kind,{answer:true});
    return true;
  } catch(error) { refuse({code:error.code ?? 'EDIT_UNAVAILABLE'}); }
}
function setFlagGesture(id,flag) {
  const n=original(id);
  if (!n) return refuse({code:'EDIT_UNAVAILABLE'});
  const {bypass,mute,pause}=n.declaration.flags;
  return submit([setFlags(n,{bypass,mute,pause,[flag]:!n.declaration.flags[flag]})]);
}
function renameGesture(id,label) {
  const n=original(id);
  if (!n) return refuse({code:'EDIT_UNAVAILABLE'});
  return submit([present(n,{label:label === '' ? null : label})]);
}
function drop(kind,released,cancelled) {
  const commands=[], kept=[];
  let ended=false;
  const items=kind === 'moves' && !cancelled ? landed(released) : released;
  for (const {actor,at} of items) {
    const n=original(actor);
    const spelled=cancelled || !n ? [] : kind==='moves' ? dragMove(n,at.x,at.y) : dragResize(n,at.width,at.height);
    commands.push(...spelled);
    if (spelled.length) { viewer.hold(actor,kind,at); kept.push([actor,at]); }
    else { ended ||= viewer.card(actor).preview?.[kind] !== undefined; viewer.release(actor,kind); }
  }
  if (ended || items.some((item,i) => item !== released[i])) redraw();
  return submit(commands).finally(() => {
    for (const [id,at] of kept) viewer.release(id,kind,at);
    if (kept.length) redraw();
  });
}
function landed(items) {
  const moving=items.map((item,i) => ({i,at:item.at,n:original(item.actor)})).filter(({n,at}) => n && dragMove(n,at.x,at.y).length);
  if (!moving.length) return items;
  const places=landing(moving[0].n.scope,moving.map(({n,at}) => ({...boxOf(n),x:at.x,y:at.y})),new Set(moving.map(({n}) => n.id)));
  const next=[...items];
  for (const [k,{i,at}] of moving.entries())
    if (places[k].x !== at.x || places[k].y !== at.y) next[i]={...items[i],at:places[k]};
  return next;
}
async function createActorGesture(type,at,wire,preprocess) {
  const registration=graph.catalog.find(r=>r.actor_type===type);
  if (!registration) return refuse({code:'ADMISSION_UNAVAILABLE'});
  if (wire && !original(wire.node)) return refuse({code:'EDIT_UNAVAILABLE'});
  const local=named ?? freshLocal(graph,scopeNamed(window.StudyApp.state.scope)?.address,type); named=undefined;
  const entry=graph.createInputs?.entries.get(type);
  const dragged=wire && original(wire.node)[wire.side].find(port=>port[0]===wire.name)?.[1];
  const availability=paletteAvailability(registration,entry);
  if (availability.code==='CREATE_BY_GROUPING') return refuse({code:availability.code});
  if (availability.settings)
    return openCreateDialog({dialog:window.Product.dialog, fieldsHTML:(answer,config,raw) => fieldsHTML(answer,config,raw,reason),
      title:`New ${registration.label}`, description:registration.description, local,
      entry:{...entry, draft:{...entry.draft, ...portDraft(entry,dragged)}},
      place:(name,config) => placeActor(registration,at,wire,name,config,preprocess)});
  if (await placeActor(registration,at,wire,local,registration.template_config,preprocess) === true) return true;
  keepInPalette(type,at,local);
}
async function groupGesture(ids,target,type,name) {
  const nodes=ids.map(original),container=original(target);
  if(container) return submit(moveIntoScope(graph,nodes,[...container.address.scope,{name:container.address.local}]));
  const registration=graph.catalog.find(r=>r.actor_type===type);
  if (!registration) return refuse({code:'ADMISSION_UNAVAILABLE'});
  const scope=nodes[0].address.scope;
  if (containerCardinality(registration) === 'keyed_many') {
    const entry=graph.createInputs?.entries.get(type);
    if (!entry?.slots?.length) return refuse({code:entry?.code ? 'CREATE_INPUTS_UNAVAILABLE' : graph.createInputs?.diagnostic ?? 'CREATE_INPUTS_UNREAD'});
    return openCreateDialog({dialog:window.Product.dialog, fieldsHTML:(answer,config,raw) => fieldsHTML(answer,config,raw,reason),
      title:`New ${registration.label}`, description:registration.description, local:freshLocal(graph,scope,type), entry,
      place:async (local,policy) => {
        const members=ids.map(original);
        if (members.some(n => !n)) return {code:'EDIT_UNAVAILABLE'};
        let commands;
        try { commands=foldIntoNewReplicator(graph,members,scope,local,registration,policy); }
        catch(error) { if (error.code === undefined) throw error; console.info(error.code, error.message); return {code:error.code}; }
        return write(commands);
      }});
  }
  try { return await submit(foldIntoNewScope(graph,nodes,scope,name,registration)); }
  catch(error) { refuse({code:error.code ?? 'EDIT_UNAVAILABLE'}); }
}
async function decideGesture(row) {
  const decision=row.decision;
  dispatch({kind:'approval-decision',id:row.id,state:'submitting',decision});
  let refused;
  try {
    const answer=await decide(session,{item:row.item,decision:decision==='deny' ? 'Deny' : 'Approve'});
    if (answer[0] !== 1n) refused={...answer[1], code:inSpace('LedgerTransition', answer[1].code)};
  } catch(error) { refused={code:error.code ?? 'READ_UNAVAILABLE'}; }
  const refusal=refused === undefined ? undefined : reason(refusalCode(refused));
  if (!refusal) dispatch({kind:'approval-decision',id:row.id,state:'accepted',decision});
  else {
    dispatch({kind:'approval-decision',id:row.id,state:'failed',decision,code:refusal.code,reason:refusal.label});
    refuse(refused);
  }
  await inTurn(observeApprovals);
}
export async function initialize({ connect } = {}) {
  approvalPage=undefined;approvalCode=undefined;preprocessKinds=[];preprocessCode='unobserved';preprocessRead=undefined;
  viewer.clear();
  for (const element of globalThis.document?.querySelectorAll?.('[data-fixture-only]') ?? []) element.remove();
  organizeControl=undefined;projectsView=undefined;projectsShown=undefined;awaitedReading=undefined;harnessDialog=undefined;drawn=undefined;
  diagnostic=undefined;observation=undefined;graph=undefined;journalCode=null;opener=undefined;session=undefined;writer=undefined;ended=null;following=undefined;followingCommits=undefined;refreshHistory=async()=>{};arrivals=undefined;arriving=undefined;arrivalCode=undefined;view=null;turn=Promise.resolve();
  acceptedEdits=[];
  retrying=false;asking=undefined;recentCode=undefined;connectionEvidence=undefined;history=undoHistory();
  reattach.stop();away=undefined;rejoined=undefined;seenAt=null;selection={key:undefined};
  toasts.stop();bannerFact=undefined;bannerBaseline=undefined;
  const params=new URLSearchParams(location.search);
  source=makeSource(); window.StudySource=source;
  const state=params.get('state');
  if (state) {
    source.product.project=state;
    if (globalThis.document) globalThis.document.title=baseName(state);
  }
  const recent=params.get('recent');
  recentCode=params.get('recentCode') ?? (recent==null ? 'READ_UNAVAILABLE' : undefined);
  const paths=recent==null ? [] : JSON.parse(recent);
  if (state && !paths.includes(state)) paths.unshift(state);
  source.product.projects=paths.map(path=>({id:path,name:baseName(path),description:path,scope:'root'}));
  window.STUDY={root:{name:'Workspace',nodes:[],edges:[],notes:[]},journal:[]};
  window.PUBLISHED_ACTORS=[]; window.STUDY_HISTORY={start:0,duration:0,stages:[],wallClock:null};
  opener=connect;
  diagnostic=reason(!opener ? 'BRIDGE_UNAVAILABLE' : params.get('connectionCode') ?? 'ATTACHING');
  endBy('attach',diagnostic.code);
  connectionEvidence=undefined;
  source.product.connection='disconnected';
}
export async function mount({ metrics } = {}) {
  cardMetrics=metrics ?? undefined;
  projectsView=bindProjects(document,{current:source.product.project, controls:projectControls});
  projectsView();
  if (source.noProject) {
    console.info(diagnostic.code);
    return {connection:diagnostic.code};
  }
  source.refreshApprovalBadge();
  bindPaletteName(document);
  const a=window.StudyApp, p=window.Product;
  if (a?.timeMachine) refreshHistory = () => timeBar.read();
  if (document.addEventListener && p?.actions) {
    organizeControl=bindOrganize(document,a,{availability:organizeCode,action:organizeScope});
  }
  bindConfigStructure(document);
  p.projects=source.product.projects;p.project=source.product.project;
  p.connection=diagnostic?'disconnected':'connected';
  a.renderGraph();p.refreshEditTools();
  const params=new URLSearchParams(location.search);
  try {
    if (!opener || params.has('connectionCode')) return {connection:diagnostic.code};
    const result=await window.circularConnection();
    if (result.connection!=='connected') throw {code:result.connection,evidence:result.evidence};
    await attachSession(result.attachment);
    diagnostic=undefined;ended=null;p.connection='connected';
    observation=observe();
    if (params.get('created') === '1') observation.then(shown => { if (shown) openHarnesses(); });
    return true;
  } catch(error) {
    diagnostic=reason(error.code ?? 'READ_UNAVAILABLE');
    endBy('attach',diagnostic.code);connectionEvidence=error.evidence;
    awaitDaemon(diagnostic.code);
    return {connection:diagnostic.code};
  } finally {
    if(diagnostic) report(diagnostic.code);
    p.refreshOutputs?.();
    source.afterGraph();
    window.StudyApp?.renderInspectorFooter?.();
  }
}
function refused(code,evidence) {
  const previous=attachCode();
  diagnostic=reason(code);
  endBy('attach',diagnostic.code);connectionEvidence=evidence;
  awaitDaemon(diagnostic.code);
  const p=globalThis.window?.Product;
  if (p) {p.connection='disconnected';p.refreshOutputs?.();}
  if (diagnostic.code!==previous) report(diagnostic.code); else source.updateStatusbar();
  source.afterGraph();
  window.StudyApp?.renderInspectorFooter?.();
}
function openEditor() {
  writer?.release();
  writer=editor(session, undefined,
    { onSettled: outcome => { toasts.notify(outcome.code); window.Product?.refreshEditTools?.(); } });
}
async function attachSession(attachment) {
  session=await opener(attachment);
  openEditor();
  history=undoHistory();window.Product?.refreshEditTools?.();
  stopView();timeBar?.forget();
}
let releasing;
async function attempt(start, auto=false) {
  if (!auto && ended === null) { releasing = session; stopReaders(); }
  try {
    const result=await window.circularConnection({attach:true,...(start ? {start:true} : {})});
    if (result.connection!=='connected') throw {code:result.connection,evidence:result.evidence};
    await attachSession(result.attachment);
  } catch(error) {
    const code=error.code ?? 'READ_UNAVAILABLE';
    if (auto && awaitsDaemon(code)) { reattach.tried(code); source.afterGraph(); return false; }
    refused(code,error.evidence); return false;
  }
  return await reopen();
}
function rejoin() {
  reattach.stop();
  if (!away) return;
  const restart=wallClockOf(graph?.healthPage?.anchor)?.atMs;
  const restarted=restart != null && restart/1000 > (away.at ?? -Infinity) ? restart/1000 : -Infinity;
  const at=Math.max(source.observedHead, restarted);
  rejoined={code:away.code, from:away.at, at, atText:recordedAt(at)};
  away=undefined;
  markAttachment();
  source.updateStatusbar();
}
function markAttachment() {
  const archive=window.StudyApp?.archive;
  if (!archive || !rejoined) return;
  archive.attachment=[{at:rejoined.at, label:'Reattached', detail:rejoinedText(rejoined)}];
  window.StudyApp.timeMachine?.refresh?.();
}
function stopReaders() {
  following?.stop();following=undefined;
  followingCommits?.stop();followingCommits=undefined;
  arriving?.stop();arriving=undefined;arrivalCode=undefined;
}
async function reopen() {
  stopReaders();selection={key:undefined};
  diagnostic=undefined;ended=null;connectionEvidence=undefined;journalCode=null;
  acceptedEdits=[];
  preprocessRead=undefined;
  window.Product.connection='connected';
  observation=observe(true);
  const shown=await observation===true;
  const lens=shown ? view?.lens : undefined;
  if (lens) await inTurn(() => openView(lens));
  return shown;
}
function resetBy(reader) {
  if (reader===arriving || reader===following) void reopen();
}
async function openView(lens) {
  if (!graph || !session || timeBar?.transport.lens?.handle !== lens) return;
  const held=stopView();
  const next=view={lens, arrivals:arrivalWindow(session, journalRows, actorRows, lens), code:undefined,
    live:held ? held.live : graph.journalDiagnostic};
  selection={key:undefined};
  let ready, arrivalsOpened, recordsOpened;
  const first=new Promise(resolve => { ready=resolve; });
  const subscribed=Promise.all([new Promise(resolve => { arrivalsOpened=resolve; }), new Promise(resolve => { recordsOpened=resolve; })]);
  try {
    next.arriving=followRecords(session,null,async delivered => {
      if (!await first) return;
      await inTurn(() => viewArrivals(next,delivered));
    },(code,kind,why) => { arrivalsOpened(); viewEnded(next,code,kind,why); },() => arrivalsOpened(),
    {target:'actor.events',credit:BigInt(journalRows),lens});
    next.following=followRecords(session,graph.anchor.scope,async () => {
      if (!await first) return;
      await inTurn(() => viewHealth(next));
    },(code,kind,why) => { recordsOpened(); viewEnded(next,code,kind,why); },() => recordsOpened(),{liveOnly:true,lens});
    await subscribed;
    await readView(next);
    ready(true);
    window.StudyApp?.timeMachine?.refresh?.();
  } finally { ready(false); }
}
async function readView(v) {
  if (view !== v) return;
  const placed=timeBar.transport.lens.at.cut;
  const reading=arrivalWindow(session, journalRows, actorRows, v.lens);
  let page, journalDiagnostic;
  try { page=await reading.read(); }
  catch (error) { page=error.page; journalDiagnostic=error.code ?? 'READ_UNAVAILABLE'; }
  const upto=page?.cut ?? placed;
  let scene;
  try {
    scene=await readScene(session,[],graph.catalogObservation,async () => page ?? {items:[]},undefined,v.lens,upto,cardMetrics);
    v.snapshotCode=undefined;
  } catch (error) {
    v.snapshotCode=error.code ?? 'READ_UNAVAILABLE';
    scene=joined({...graph,declared:emptyScene(graph.anchor.scope),declarationCut:upto,observed:{actors:new Map()},journalPage:page,
      healthPage:null,health:null,problems:null,instances:null});
  }
  if (view !== v) return;
  v.arrivals=reading;
  dispatch({kind:'scene',scene:{...scene,journalDiagnostic,drawn:graph.nodes}});
  window.StudyApp?.timeMachine?.refresh?.();
}
async function viewArrivals(v, delivered) {
  if (view !== v) return;
  const {page, added, fresh}=v.arrivals.append(delivered);
  if (added && (v.snapshotCode != null || fresh.some(row => (isEmission(row) ? row.at : row.origin)?.[4]?.value > graph.anchor.cursor))) return readView(v);
  if (added) dispatch({kind:'arrivals',page,fresh});
}
async function viewHealth(v) {
  if (view !== v) return;
  const observed=await readHealth(session,v.lens), runtime=await readRuntime(session,v.lens);
  if (view !== v) return;
  dispatch({kind:'health',observed,runtime});
  window.StudyApp?.timeMachine?.refresh?.();
}
function viewEnded(v, code, kind, why) {
  if (view !== v || v.moved) return;
  if (why === 'ResetRequired') { v.moved=true; return void inTurn(() => openView(v.lens)); }
  if (why === 'TargetGone') return void closeView(v.lens);
  v.code=headerCode(code, kind);
  journalCode=v.code;window.StudyApp?.renderJournalHeader?.();
}
const shownCode=() => view ? view.code : arrivalCode;
const headerCode=(code, kind) => code === undefined || code === null ? undefined
  : {label:kind === 'SubscriptionEnded' ? `Arrivals stopped · ${reasonText(code)}` : reasonText(code), code:codeText(code)};
function stopView() {
  const held=view;
  view=null;
  held?.arriving?.stop();held?.following?.stop();
  if (held) selection={key:undefined};
  return held;
}
function closeView(lens) {
  if (view?.lens !== lens) return;
  const held=stopView();
  return inTurn(() => showLive(held.live));
}
async function showLive(journalDiagnostic) {
  if (view || !graph || !session) return;
  const scene=await readScene(session,[],graph.catalogObservation,async () => arrivals?.page ?? {items:[]},undefined,undefined,undefined,cardMetrics);
  if (view) return;
  applyLiveObservation({kind:'scene',scene:{...scene,journalDiagnostic,drawn:graph.nodes}});
  readDepths();
  timeBar?.observed({page:scene.healthPage},scene);
  refreshHistory();
  window.StudyApp?.timeMachine?.refresh?.();
}
function harnessContext() {
  return {bind:setHarness,
    choose:window.circularConnection && (name => window.circularConnection({choose:'program', name}))};
}
async function setHarness(name, program) {
  let refusal;
  try {
    const answer=await bindHarness(session,name,program);
    if (answer[0] === 1n) return true;
    refusal={...answer[1],code:inSpace('LedgerTransition',answer[1].code)};
  } catch(error) { refusal={code:error.code ?? 'EDIT_UNAVAILABLE'}; }
  refuse(refusal);
  return refusal;
}
function projectFacts() {
  const code=endedCode(), tried=reattach.waiting?.tried;
  return {code, start:noDaemon.has(code) || noDaemon.has(tried),
    running:code === undefined || code === 'DAEMON_NOT_ANSWERING' || tried === 'DAEMON_NOT_ANSWERING',
    health:graph?.healthPage, harness:code === undefined && session ? heldReading() : undefined,
    choose:Boolean(window.circularConnection), reveal:Boolean(window.circularConnection)};
}
function projectControls() {
  return {context:endedCode() === undefined && session ? harnessContext() : undefined,
    reveal:window.circularConnection && (() => window.circularConnection({reveal:'config'})), redraw:redrawProject};
}
const currentReading = () => harnessReading(session, graph?.anchor?.authoringRevision);
function heldReading() {
  const reading=currentReading();
  if (!readingSettled(reading) && awaitedReading !== reading) {
    awaitedReading=reading;
    void readingDone(reading).then(() => { if (awaitedReading === reading) { awaitedReading=undefined; redrawProject(); redrawInspector(); } });
  }
  return reading;
}
function readDepths() {
  if (!session || !graph || view || globalThis.window?.StudyApp?.historical?.()) return;
  askDepths(session, showDepths);
}
let drawnDepths;
function showDepths(value) {
  const a=globalThis.window?.StudyApp;
  if (!graph || view || !a) return;
  a.showMailboxes?.(pools(value, graph.edges));
  const shown=a.state?.tab === 'inspect' && a.state.selected && !a.state.edge ? original(a.state.selected) : undefined;
  if (shown && drawnDepths && identity(inletDepths(value, graph.edges, shown.id)) !== drawnDepths) redrawInspector();
}
function redrawProject() {
  if (redrawProjects && window.Product?.screen === 'projects') redrawProjects();
  harnessDialog?.();
}
function redrawInspector() {
  const a=window.StudyApp;
  if (a?.state?.tab === 'inspect' && a.state.selected && !a.state.edge) a.renderInspector?.();
}
async function stopDaemon(restart=false) {
  let answer;
  try { answer=await window.circularConnection?.({daemon:'stop'}); }
  catch (error) { answer={code:error.code ?? 'BRIDGE_UNAVAILABLE'}; }
  if (!answer) return refuse({code:'BRIDGE_UNAVAILABLE'});
  if (answer.code) return refuse({code:answer.code, message:evidenceText(answer.evidence) || undefined});
  if (restart) return reconnect(true);
}
function openHarnesses() {
  const title=`Harnesses for ${baseName(source.product.project)}`;
  const draw=() => {
    const box=window.Product?.dialog?.(title, `<section class="detail-section" data-harness-dialog>${projectHarnessesHTML(projectFacts(), {titled:false})}</section>`);
    if (box) bindProjectControls(box, source.product.project, projectControls);
    return box;
  };
  const box=draw();
  if (box) harnessDialog=() => { if (box.open && box.querySelector('[data-harness-dialog]')) draw(); else harnessDialog=undefined; };
  return box;
}
let asking;
async function reconnect(start=false, auto=false) {
  if (!opener || !globalThis.window?.circularConnection) return false;
  if (retrying) {
    if (auto || asking?.viewer) return false;
    await asking?.done.catch(() => {});
    if (!start && ended === null) return true;
    if (retrying) return false;
  }
  retrying=true;
  const done=(async () => {
    try { return await attempt(start, auto); }
    catch(error) { refused(error.code ?? 'READ_UNAVAILABLE'); return false; }
    finally { retrying=false; asking=undefined; }
  })();
  asking={viewer:!auto, done};
  return done;
}
function followArrivals(firstScreen) {
  arrivals=arrivalWindow(session, journalRows, actorRows);
  arrivalCode=undefined;
  let opened;
  const subscribed=new Promise(resolve => { opened=resolve; });
  const arrivalReader=arriving=followRecords(session,null,async delivered => {
    if (!await firstScreen) return;
    await inTurn(() => observedArrivals(delivered));
  },(code,kind,why)=>{
    opened();
    if (why==='ResetRequired') return resetBy(arrivalReader);
    arrivalCode=headerCode(code, kind);
    if (!view) { journalCode=arrivalCode;window.StudyApp?.renderJournalHeader?.(); }
  },()=>opened(),{target:'actor.events',
    credit:BigInt(journalRows)});
  return subscribed;
}
async function observe(again=false) {
  const a=window.StudyApp, p=window.Product, own=session;
  let drawn;
  const firstScreen=new Promise(resolve => { drawn=resolve; });
  const subscribed=followArrivals(firstScreen);
  const followHealth=anchor => new Promise(resolve => {
    const healthReader=following=followRecords(session,anchor.scope,async records => {
      if (!await firstScreen) return;
      await inTurn(async () => {
        if (!view) observedHealth(await readHealth(session),await readRuntime(session));
        await observeApprovals();
        readDepths();
        timeBar?.ran(records);
        a.timeMachine.refresh();
      });
    },(code,kind,why)=>{
      resolve();
      if (why==='ResetRequired') return resetBy(healthReader);
      firstScreen.then(shown => {
        if (!shown || own === releasing) return;
        endBy('records',code);p.connection='disconnected';
        forgetDepths();a.showMailboxes?.([]);
        awaitDaemon(code);
        report(code);p.refreshOutputs?.();source.afterGraph();a.timeMachine.refresh();
        if (a.state?.tab === 'inspect' && a.state?.selected && !a.state.edge) a.renderInspector?.();
        else a.renderInspectorFooter?.();
      });
    },()=>{ resolve(); firstScreen.then(shown => shown && a.timeMachine.refresh()); },{liveOnly:true});
  });
  try {
    await subscribed;
    let observed;
    const scene=await readScene(session, [], undefined, async () => (observed=await arrivals.read()), followHealth, undefined, undefined, cardMetrics);
    const first=await views.read(session, scene, observed ?? scene.journalPage ?? {items:[]});
    const createInputs=await readCreateInputs(session);
    const approvals=await readApprovals();
    applyLiveObservation({kind:'scene',scene:{...first,createInputs,...(approvals ? {approvals} : {})}});p.refreshEditTools();
    if (!again) a.oweFit?.(drawnScopes().keys());
    if (graph.journalDiagnostic) report(graph.journalDiagnostic);
    if (graph.healthDiagnostic) report(graph.healthDiagnostic);
  } catch(error) {
    drawn(false);arriving.stop();following?.stop();
    endBy('attach',error.code ?? 'READ_UNAVAILABLE');
    awaitDaemon(endedCode());
    p.connection='disconnected';report(endedCode());source.afterGraph();a.renderInspectorFooter?.();
    p.refreshOutputs?.();
    return false;
  }
  drawn(true);
  readDepths();
  timeBar?.observed({page:graph.healthPage},graph);
  refreshHistory();
  rejoin();
  const edit=writer, after=graph.anchor.cursor;
  const feed=followCommits(session,graph.anchor.scope,after,async commit => {
    acceptedEdits=[...projectAcceptedCommit(commit),...acceptedEdits].slice(0,journalRows);
    timeBar?.edited();
    const own=edit.authored(commit.epoch.begin.commitId);
    if (!own) history.observed(touchedKeys(commit.epoch.content));
    if (view) { edit.observed(commit.metadata.cursor); return; }
    const cards=!own && drawnScopes(), {replaced,changed,ports,only}=dispatch({kind:'commit',commit,own});
    edit.observed(commit.metadata.cursor);
    if (ports) dispatch({kind:'ports',ports:await ports,only});
    readDepths();
    for (const detail of new Set(replaced.map(r=>revisionShort(r.before))))
      toasts.notify('ACTOR_REPLACED',{detail});
    if (!own && changed) {
      const grown=[...drawnScopes()].filter(([scope,ids]) => [...ids].some(id => !cards.get(scope)?.has(id))).map(([scope]) => scope);
      if (grown.length) a.oweFit?.(grown);
    }
  },code => {
    report(code);
    edit.release();
  },() => edit.following(after));
  followingCommits={stop:() => { edit.release(); return feed.stop(); }};
  const scene=new URLSearchParams(location.search).get('scene');
  if(scene==='configure') {a.state.tab='configure';a.renderInspector();}
  if(scene==='approvals') p.actions.approvals();
  return true;
}

function observedArrivals(delivered) {
  const {page, added, fresh}=arrivals.append(delivered);
  if (added) timeBar?.arrived(fresh);
  if (added && !view) dispatch({kind:'arrivals',page,fresh});
  if (added) readDepths();
}

async function readApprovals() {
  try {
    approvalPage=await readApprovalQueue(session);
    approvalCode=undefined;
  } catch(error) {
    approvalPage=undefined;
    approvalCode=error.code ?? 'READ_UNAVAILABLE';
  }
  source.refreshApprovalBadge();
  return approvalPage ? await readCalls(approvalRows(approvalPage,graph?.approvals ?? [])) : undefined;
}
async function observeApprovals() {
  const rows=await readApprovals();
  if (!graph || !rows) return;
  dispatch({kind:'approval-queue',rows});
}
async function readCalls(rows) {
  const read=[];
  for (const row of rows) {
    const unread=code => { const r=reason(code); return {...row,callCode:r.code,callReason:r.label}; };
    let arrival=row.arrival, answer;
    if (!row.cause) answer=unread(causeUnrecorded);
    else {
      if (!arrival) try { arrival=await readCause(session,row.cause) ?? undefined; }
      catch (error) { if (error?.code === undefined) throw error; answer=unread(error.code); }
      answer ??= arrival ? {...row,arrival} : unread(causeUnread);
    }
    read.push(Object.freeze(answer));
  }
  return read;
}
function projectApprovals() {
  const p=window.Product;
  if (!p) return;
  const rows=(graph.approvals ?? []).map(row => row.arrival
    ? Object.freeze({...row,call:approvalCall(row.arrival,graph),record:causeRecord(row.arrival,graph,domId)}) : row);
  const shown=list => list.map(r=>[r.id,r.state,r.call ?? r.callCode ?? null]);
  const changed=!sameFields(shown(rows),shown(p.approvals ?? []));
  p.approvals=rows;
  if (changed) p.refreshApprovals?.();
}
function approvalRows(page,held) {
  return page.items.map(r => {
    const id=key(r.item), requested=r.state === 1n;
    const answer=held.find(row=>row.id===id && ['submitting','failed'].includes(row.state));
    const read=held.find(row=>row.id===id)?.arrival;
    return {id,item:r.item,cause:r.cause,...(read ? {arrival:read} : {}),
      actor:Array.isArray(r.emitter) && r.emitter[0] === 1n
        ? domId(key(actorIdentityFromValue({scope:r.emitter[1],local:r.emitter[2]}))) : undefined,
      action:'Approval',target:JSON.stringify(display(r.target_effect)),args:display(r),
      ...(requested ? {state:'requested'} : {state:'accepted',decision:'approve'}),
      ...(requested && answer ? {state:answer.state,decision:answer.decision,code:answer.code,reason:answer.reason} : {})};
  });
}
