import { actorIdentityFromValue } from '@circular/protocol/establishment';
import { reason, reasonText, inSpace } from './reasons.mjs';
import { addressPath } from './scene.mjs';
import { domId, identity, journalRows, latestWindow, readFirstPage, readCompleteAnswer, actorKey } from './query.mjs';
import { columnEnds, isOwnOutcome, causingArrival, isReservedIngress, isEmission } from './arrivals.mjs';
import { recordValue } from './record-text.mjs';
import { formatReading, formatTime } from './view-registry.mjs';
import { valueText } from './value-text.mjs';
import { kindText, outcomeFacts, causedBy, verbText } from './record-words.mjs';
import { ACTOR_KINDS, keysOf } from './verbs.mjs';

export const recordValueUnavailable = 'BODY_UNRECORDED';

export function observedWindow(page, spell = globalThis.window?.studyTimeFormat, counted) {
  const stamps = (page?.items ?? [])
    .map(row => row?.observed_at_ms ?? row?.at_ms)
    .filter(stamp => typeof stamp === 'bigint');
  if (!stamps.length) return null;
  const start = stamps.reduce((low, stamp) => stamp < low ? stamp : low);
  const from = formatTime(start, typeof spell === 'function' ? seconds => spell(seconds, true) : null).text;
  return counted === undefined ? `held from ${from}` : `${counted} · list holds rows from ${from}`;
}

export const recordedTotal = page => [...columnEnds(page).values()].reduce((sum, end) => sum + end, 0n);
export const journalReason = page => Array.isArray(page?.terminal) && page.terminal[0] === 3n
  ? reason(inSpace('Query', page.terminal[1])).code : undefined;
export function journalStatus(page) {
  if (Array.isArray(page?.terminal) && page.terminal[0] === 3n) return `Partial · ${reason(inSpace('Query', page.terminal[1])).label}`;
  const span = observedWindow(page, undefined, `${recordedTotal(page)} arrivals recorded`);
  if (page?.folded_from !== undefined) return [reason('folded_from').label, span].filter(Boolean).join(' · ');
  return span;
}

export const tallyCell = 0.25;
const cellOf = at => Math.floor((at - 0.000001) / tallyCell);
const deliveryEdge = row => Array.isArray(row?.edge) && row.edge[0] === 1n ? domId(identity(row.edge)) : null;
const cellKey = actor => domId(actorKey(actor));
export function arrivalTally() {
  const cells = new Map(), wires = new Map(), held = new Map();
  let from = Infinity;
  const since = actor => Math.max(from, ...(actor === undefined ? held.values() : [held.get(actor) ?? -Infinity]));
  const known = actor => { const at = since(actor); return at === Infinity ? Infinity : (cellOf(at) + 1) * tallyCell; };
  const add = (map, id, at) => {
    const key = `${id} ${cellOf(at)}`, cell = map.get(key);
    if (cell) { cell.count += 1; cell.at = Math.max(cell.at, at); }
    else map.set(key, {actor: id, at, count:1, event:'actor_arrival'});
  };
  const within = (map, id, start, end, floor) => {
    const lower = Math.max(start, floor);
    if (!(end > lower)) return null;
    let count = 0;
    const whole = Math.floor(end / tallyCell + 1e-9) - 1;
    for (let at = cellOf(lower) + 1; at <= whole; at += 1) count += map.get(`${id} ${at}`)?.count ?? 0;
    const partial = map.get(`${id} ${whole + 1}`);
    if (whole + 1 <= cellOf(end) && partial && partial.at <= end) count += partial.count;
    return {count, seconds:end - lower, whole:floor <= Math.max(start, 0)};
  };
  return {
    get from() { return from; },
    set from(value) { from = value; },
    hold(actor, at) { const key = cellKey(actor); held.set(key, Math.max(held.get(key) ?? -Infinity, at)); },
    cells: {[Symbol.iterator]: () => cells.values()},
    get size() { return cells.size; },
    count(row) {
      if (row?.kind !== 'actor_arrival' || typeof row.observed_at_ms !== 'bigint') return;
      const at = Number(row.observed_at_ms) / 1000, wire = deliveryEdge(row);
      if (!isReservedIngress(row)) add(cells, cellKey(row.actor), at);
      if (wire) add(wires, wire, at);
    },
    within(actor, start, end) { return within(cells, actor, start, end, known(actor)); },
    wireWithin(wire, start, end) { return within(wires, wire, start, end, known()); },
    series(actor, end, count) {
      const last = cellOf(end), first = cellOf(known(actor)) + 1;
      return Array.from({length: count}, (_, i) => {
        const at = last - count + 1 + i, cell = cells.get(`${actor} ${at}`);
        if (since(actor) === Infinity || at < first) return null;
        return at === last && cell && cell.at > end ? 0 : cell?.count ?? 0;
      });
    },
    prune(before) {
      if (!Number.isFinite(before)) return;
      for (const map of [cells, wires]) for (const [key, cell] of map) if (cell.at < before) map.delete(key);
      from = Math.max(from, before);
    },
  };
}

function holdSince(tally, latest, read) {
  if (read.terminal !== 2n) { tally.from = Infinity; return; }
  tally.from = 0;
  for (const component of latest?.since ?? []) {
    if (!(component.index > 0n)) continue;
    const actor = identity(component.actor);
    const row = read.items.find(item => identity(item?.actor) === actor && typeof item?.observed_at_ms === 'bigint');
    tally.hold(component.actor, row ? Number(row.observed_at_ms) / 1000 : Infinity);
  }
}

export function arrivalWindow(session, rows = journalRows, perActor = 0, lens) {
  let answer, items = [];
  const reached = new Map(), tally = arrivalTally();
  let focused = new Set();
  const addressOf = actor => identity(actorIdentityFromValue(actor));
  const note = row => {
    const actor = identity(row.actor), next = row.index + 1n;
    if (!(reached.get(actor) >= next)) reached.set(actor, next);
  };
  let answered = new Set();
  const held = all => {
    const kept = [], seen = new Map();
    for (let at = all.length - 1; at >= 0; at -= 1) {
      const actor = actorKey(all[at].actor), side = `${isEmission(all[at])} ${actor}`, count = seen.get(side) ?? 0;
      const depth = focused.size && focused.has(actor) ? rows : perActor;
      if (all.length - at <= rows || count < depth) kept.push(all[at]);
      seen.set(side, count + 1);
    }
    return kept.reverse();
  };
  const page = () => ({ ...answer, items });
  return {
    tally,
    get page() { return answer === undefined ? undefined : page(); },
    async selection(ids) {
      focused = new Set(ids);
      const first = await readFirstPage(session, 'actor.events', null, 1, undefined, lens);
      const depth = BigInt(rows);
      const since = (first.cut ?? []).map(component => focused.has(addressOf(component.actor))
        ? { actor: component.actor, index: component.index > depth ? component.index - depth : 0n } : component);
      const read = since.length ? await readCompleteAnswer(session, 'actor.events', null, rows * Math.max(1, focused.size), lens, since) : first;
      return read.items.filter(row => row?.actor && focused.has(addressOf(row.actor)));
    },
    async read() {
      if (answer !== undefined) return page();
      const first = await readFirstPage(session, 'actor.events', null, rows, undefined, lens);
      const latest = latestWindow(first.cut, rows, perActor);
      const read = latest === null ? first
        : await readCompleteAnswer(session, 'actor.events', null, latest.limit, lens, latest.since);
      for (const row of read.items) if (!isEmission(row) && typeof row?.index === 'bigint') note(row);
      for (const row of read.items) tally.count(row);
      answered = new Set(read.items.filter(isEmission).map(row => identity(row.at)));
      holdSince(tally, latest, read);
      answer = read;
      items = held(read.items);
      return page();
    },
    append(arrived) {
      const fresh = arrived.filter(row => isEmission(row) ? !answered.has(identity(row.at))
        : typeof row?.index === 'bigint' && !(reached.get(identity(row.actor)) > row.index));
      for (const row of fresh) {
        if (isEmission(row)) continue;
        const at = typeof row.observed_at_ms === 'bigint' ? Number(row.observed_at_ms) / 1000 : Infinity;
        const next = reached.get(identity(row.actor));
        if (tally.from === Infinity) tally.from = at;
        else if (next !== undefined && row.index > next) tally.hold(row.actor, at);
        tally.count(row);
        note(row);
      }
      if (fresh.length) items = held([...items, ...fresh]);
      return { page: page(), added: fresh.length, fresh };
    },
  };
}

export const authoredName = address => address === undefined ? undefined
  : [String(address.local), ...(address.scope.length ? [addressPath({...address, local:''}).slice(0, -1)] : [])].join(' · ');

const ownKind = body => body !== null && typeof body === 'object' && !Array.isArray(body) && !(body instanceof Uint8Array)
  && typeof body.kind === 'string' && body.kind ? body.kind : null;
function outcomeDetail(item, column) {
  const cause = causingArrival(item, column);
  return [...outcomeFacts(item.body), cause.row ? causedBy(formatReading(cause.row.index)?.text ?? cause.row.index) : null]
    .filter(Boolean).join(' · ');
}
function arrivalDetail(body, kind) {
  if (kind === null) return valueText(body).text;
  const { kind: _named, ...fields } = body;
  return Object.keys(fields).length ? valueText(fields).text : '';
}
export function journalProjection(nodes, toDomId, show = value => value) {
  const actors = new Map(nodes.map(node => [node.id, toDomId(node.id)]));
  const names = new Map(nodes.map(node => [node.id, authoredName(node.address)]));
  const project = (item, outcome, page) => {
    if (item.kind !== 'actor_arrival') return [];
    const id = actorKey(item.actor), actor = actors.get(id);
    if (!actor) return [];
    const kind = ownKind(item.body);
    return [{ actor, actorName: names.get(id), index: formatReading(item.index)?.text ?? '—', event: item.kind,
      eventName: outcome || kind === null ? kindText(outcome ? kind : item.kind) : kind,
      detail: item.body === undefined ? item.port ?? reasonText('BODY_UNRECORDED')
        : outcome ? outcomeDetail(item, page.items) : arrivalDetail(item.body, kind),
      port: item.port, record: show(item), time: String(item.observed_at_ms ?? '—'),
      ...(item.observed_at_ms === undefined ? {timeReason:'TIME_UNRECORDED'} : {}),
      at: Number(item.observed_at_ms ?? 0) / 1000,
      ...(item.body === undefined ? { value: '—', valueTitle: recordValueUnavailable } : { value: show(item.body) }) }];
  };
  const projected = new WeakMap();
  const once = (item, page) => {
    let rows = item !== null && typeof item === 'object' ? projected.get(item) : undefined;
    if (rows) return rows;
    const outcome = item?.kind === 'actor_arrival' && isOwnOutcome(item);
    rows = project(item, outcome, page);
    if (!outcome && item !== null && typeof item === 'object') projected.set(item, rows);
    return rows;
  };
  return page => page.items.flatMap(item => once(item, page)).slice(-journalRows);
}
export const projectJournal = (page, nodes, toDomId, show) => journalProjection(nodes, toDomId, show)(page);

export function projectAcceptedCommit(commit) {
  const commitId = Array.from(commit.epoch.begin.commitId, byte => byte.toString(16).padStart(2, '0')).join('');
  const shown = recordValue(commit);
  return commit.epoch.content.map((command, index) => {
    const keys = keysOf(command);
    const addresses = keys.flatMap(([kind, at]) => ACTOR_KINDS.includes(kind)
      ? kind === 'presentation' ? [at.actor].filter(Boolean) : [at]
      : kind === 'edge' ? [at.from.actor, at.to.actor] : []);
    const actors = [...new Map(addresses.map(at => [domId(identity(at)), at])).entries()];
    const scopes = [...actors.map(([, at]) => at.scope),
      ...keys.flatMap(([kind, at]) => kind === 'scope' ? [at] : at?.scope ? [at.scope] : [])];
    return { id: `${commitId}:${index}`, commitId, event: command.kind, eventName: verbText(command.kind),
      actorName: actors.map(([, at]) => authoredName(at)).join(' → ') || 'Authoring',
      actors: actors.map(([id]) => id),
      scopes: (scopes.length ? scopes : [commit.metadata.targetScope]).map(identity),
      detail: 'Accepted', timeReason: 'TIME_UNRECORDED',
      command: shown.epoch.content[index], commit: shown };
  });
}

export function applyJournal(app, rows) {
  app.state.journalRows = rows;
  app.archive.entries = rows;
  const selected = app.state.selectedRecord;
  if (selected) app.state.selectedRecord = rows.find(row =>
    row.actor === selected.actor && row.index === selected.index) ?? null;
  app.archive.version++;
  app.renderJournal();
}
