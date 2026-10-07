import { TIMELINE_MAX_BINS } from '@circular/protocol';
import { reason } from './reasons.mjs';
import { lensTransport } from './replay.mjs';
import { tallyCell } from './journal.mjs';
import { RATE_SECONDS } from './activity.mjs';
import { identity, actorKey } from './query.mjs';
import { isEmission } from './arrivals.mjs';
import { readTimelineBins } from './session.mjs';

export function binsRequest(range, width, span = null, most = Number(TIMELINE_MAX_BINS)) {
  const from = Math.max(0, Math.floor(range[0] * 1000)), to = Math.ceil(range[1] * 1000);
  const pixels = Math.max(1, Math.min(most - 2, Math.floor(width)));
  if (!(to > from) || !Number.isFinite(to)) return null;
  const bin = Math.max(1, Math.ceil((span === null ? to - from : span * 1000) / pixels));
  const first = Math.floor(from / bin), bins = Math.floor(to / bin) - first + 1;
  return { args: { from_ms: BigInt(first * bin), to_ms: BigInt((first + bins) * bin), bins: BigInt(bins) },
    key: `${first * bin}:${bin}:${bins}` };
}

async function scopeBins(session, args, actors) {
  const summary = await readTimelineBins(session, args);
  const bins = summary.bins.map(bin => ({ ...bin, count: 0n }));
  let clock_regressions = 0n;
  for (const actor of actors) {
    const answer = await readTimelineBins(session, { ...args, actor });
    for (let i = 0; i < bins.length; i += 1) bins[i].count += answer.bins[i].count;
    clock_regressions += answer.clock_regressions;
  }
  return { ...summary, bins, clock_regressions };
}

export function viewKey(machine, width, recorded) {
  const pixels = Math.floor(width);
  if (!(pixels > 0)) return null;
  const range = machine.range ?? [0, 0];
  if (machine.mode === 'live' && Number.isFinite(machine.window)) return `live ${machine.window} ${pixels} ${recorded}`;
  if (machine.mode === 'live') return `all ${pixels} ${Math.floor(Math.log2(Math.max(1, range[1] * 1000) / pixels))} ${recorded}`;
  return `held ${Math.round(range[0] * 1000)} ${Math.round(range[1] * 1000)} ${pixels} ${recorded}`;
}

export function extendedBuckets(answer, local, range, head, version, localIncidents) {
  const bin = Number(answer.bin_ms), seconds = bin / 1000;
  const first = Number(answer.from_ms) / bin, end = first + answer.bins.length;
  const lo = Math.max(first, Math.floor(range[0] * 1000 / bin)), hi = Math.max(end - 1, Math.floor(head * 1000 / bin));
  const count = Math.max(0, hi - lo + 1), values = new Float64Array(count), incidents = new Array(count).fill(0);
  for (let i = 0; i < count; i += 1) {
    const b = lo + i, answered = b < end ? answer.bins[b - first] : null;
    values[i] = ((answered ? Number(answered.count) : 0) + (local.get(b) ?? 0)) / seconds;
    incidents[i] = (answered ? Number(answered.incidents) : 0) + (localIncidents.get(b) ?? 0);
  }
  return { first: lo, seconds, values, incidents, version };
}

const markLabels = { edit: 'Edit', restart: 'Restart', pause: 'Pause', resume: 'Resume' };
export const answeredMarks = answer => answer.marks.map(mark => ({ at: Number(mark.at_ms) / 1000, kind: mark.kind,
  label: markLabels[mark.kind] }));

export function clockNote(regressions) {
  const n = Number(regressions);
  if (!(n > 0)) return null;
  const one = n === 1, note = reason('CLOCK_REGRESSIONS');
  return { code: note.code, text: `${note.label} · ${n}`,
    title: `${n} ${one ? 'arrival was' : 'arrivals were'} recorded with an earlier clock than the one before ${one ? 'it' : 'them'}, so ${one ? 'its time is' : 'their times are'} read as the running maximum` };
}

const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;
export const edgeText = ({letters, failed, failedAt}) => ['since summary', letters > 0 ? plural(letters, 'dead letter', 'dead letters') : '',
  failed !== failedAt ? `failed now: ${failed}` : ''].filter(Boolean).join(' · ');
export const edgeTitle = ({letters, failed, failedAt}) => `Since the bar's summary was read: ${plural(letters, 'dead letter', 'dead letters')} recorded; `
  + `${plural(failed, 'actor', 'actors')} failed now, ${failedAt} then. ${reason('INCIDENT_TIME_UNAVAILABLE').label}.`;

export function timeReading(session, source, history, actors, tally = () => undefined, root = () => globalThis.document) {
  const lens = lensTransport(session, source);
  let machine, answer = null, code = null, viewAsked = null, answers = 0;
  let local = new Map(), edge = 0, flight = null, drawn = null, since = Infinity;
  let localIncidents = new Map();
  const countIncident = (bins, bin, ms) => {
    const b = Math.floor(ms / bin);
    bins.set(b, (bins.get(b) ?? 0) + 1);
  };
  let pending = null, dirty = false;
  const place = ms => {
    if (!answer || !(ms > since)) return false;
    const b = Math.floor(ms / Number(answer.bin_ms));
    local.set(b, (local.get(b) ?? 0) + 1);
    return true;
  };
  const recorded = () => source.observedHead > 0;
  const readingKey = (m, width) => `${viewKey(m, width, recorded())}:${identity(actors())}`;
  const ask = () => {
    dirty = true;
    if (pending) return pending;
    pending = (async () => {
      while (dirty) {
        dirty = false;
        const m = machine, pixels = m?.trackWidth ?? 0;
        const asked = viewAsked = m ? readingKey(m, pixels) : null;
        const request = pixels > 0 ? binsRequest(m.range ?? [0, 0], pixels,
          m.mode === 'live' && Number.isFinite(m.window) ? m.window : null) : null;
        if (!request || source.observing?.() === false) break;
        const head = Math.round(source.head * 1000), counted = incidents;
        flight = { arrivals: [], incidents: new Map(),
          bin: Number((request.args.to_ms - request.args.from_ms) / request.args.bins) };
        let next = null, refused = null;
        try { next = await scopeBins(session(), request.args, actors()); }
        catch (error) { refused = error?.code ?? 'READ_UNAVAILABLE'; }
        if (asked !== readingKey(machine, machine.trackWidth)) { dirty = true; continue; }
        answer = next; code = refused;
        local = new Map(); since = head; incidentsAt = counted ?? incidents;
        localIncidents = flight.incidents;
        for (const ms of flight.arrivals) place(ms);
        flight = null; answers += 1; edge += 1; drawn = null;
        machine?.refresh?.();
      }
    })().finally(() => { pending = null; });
    return pending;
  };
  const restartHead = () => {
    const pair = history()?.wallClock;
    if (pair && pair.atMs / 1000 > source.observedHead) source.head = pair.atMs / 1000;
  };
  let restart;
  let incidents = null, incidentsAt = null;
  let lettersRead = null;
  const incidentsOf = (observed, runtime) => {
    const rows = runtime?.problems?.diagnostic || !Array.isArray(runtime?.problems?.rows) ? null : runtime.problems.rows;
    return {
      letters: rows?.length ?? null,
      untimed: rows?.filter(row => !Object.hasOwn(row, 'observationBucket')).length ?? null,
      failed: Array.isArray(observed?.page?.items) ? observed.page.items.filter(item => item?.state === 'failed').length : null,
    };
  };
  const edgeIncidents = () => {
    if (!answer || incidents === null || incidentsAt === null) return null;
    const letters = incidents.untimed !== null && incidentsAt.untimed !== null ? incidents.untimed - incidentsAt.untimed : 0;
    const known = incidents.failed !== null && incidentsAt.failed !== null;
    const failed = known ? incidents.failed : 0, failedAt = known ? incidentsAt.failed : 0;
    return letters > 0 || failed !== failedAt ? { letters, failed, failedAt } : null;
  };
  const unanswered = () => answer !== null ? null
    : reason(recorded() ? code ?? 'TIMELINE_UNREAD' : 'TIME_UNRECORDED');
  const bar = {
    prepare(m) {
      machine = m;
      restartHead();
      tally()?.prune(lens.position(m) - RATE_SECONDS - tallyCell);
      m.archive.markers = [...(answer ? answeredMarks(answer) : []), ...(m.archive.attachment ?? [])];
    },
    phrase: 'Arrivals in this scope',
    buckets(m, width) {
      machine = m;
      if (readingKey(m, width) !== viewAsked) {
        answer = null; code = null; drawn = null;
        void ask();
      }
      if (!answer) return { first: 0, seconds: 1, values: new Float64Array(0), incidents: [], version: `${answers}:${code ?? ''}` };
      const range = m.range ?? [0, 0], bin = Number(answer.bin_ms);
      const version = `${answers}:${edge}:${Math.floor(range[0] * 1000 / bin)}:${Math.floor(source.head * 1000 / bin)}`;
      if (drawn?.version !== version) {
        for (const b of local.keys()) if (b < Math.floor(range[0] * 1000 / bin)) local.delete(b);
        for (const b of localIncidents.keys()) if (b < Math.floor(range[0] * 1000 / bin)) localIncidents.delete(b);
        drawn = extendedBuckets(answer, local, range, source.head, version, localIncidents);
      }
      return drawn;
    },
    refresh(m) {
      const doc = root();
      const mode = doc.querySelector('#time-mode');
      const context = doc.querySelector('#time-context');
      const recordedState = source.recordedState?.();
      if (context) context.textContent = [context.textContent,
        lens.code ? reason(lens.code).label : '',
        recordedState ?? '',
      ].filter(Boolean).join(' · ');
      if (context) {
        context.title = code ? reason(code).label : '';
        if (code) context.dataset.reason = reason(code).code; else delete context.dataset.reason;
        if (lens.code) context.dataset.replayReason = reason(lens.code).code; else delete context.dataset.replayReason;
      }
      const recorded = doc.querySelector('#history-caption .history-readonly');
      if (recorded) recorded.textContent = recordedState ?? 'Recorded state';
      const held = lens.lens;
      if (mode) mode.title = held === null ? '' : held.pace === 'Free' ? 'To the recorded end'
        : held.pace.kind !== 'Realtime' ? String(held.pace)
          : m.mode === 'replay' ? `Realtime ${held.pace.num}/${held.pace.den}` : 'Paused';
      if (mode?.dataset) {
        if (held && held.pace !== 'Free') mode.dataset.lensAt = `${held.at.stream} ${held.at.revision_epoch.value}`;
        else delete mode.dataset.lensAt;
      }
      const note = doc.querySelector('#time-regressions'), regressions = answer ? clockNote(answer.clock_regressions) : null;
      if (note) {
        note.hidden = regressions === null;
        note.textContent = regressions?.text ?? '';
        note.title = regressions?.title ?? '';
        if (regressions) note.dataset.reason = regressions.code; else delete note.dataset.reason;
      }
      const edgeNote = doc.querySelector('#time-edge'), after = m.mode === 'live' ? edgeIncidents() : null;
      if (edgeNote) {
        edgeNote.hidden = after === null;
        edgeNote.textContent = after === null ? '' : edgeText(after);
        edgeNote.title = after === null ? '' : edgeTitle(after);
        if (after === null) delete edgeNote.dataset.reason;
        else edgeNote.dataset.reason = 'INCIDENT_TIME_UNAVAILABLE';
      }
      const coded = unanswered();
      for (const selector of ['#time-density', '#time-scale']) {
        const slot = doc.querySelector(selector);
        if (!slot?.dataset) continue;
        if (coded) slot.title = coded.label; else if (selector === '#time-density') slot.title = '';
        if (coded) slot.dataset.reason = coded.code; else delete slot.dataset.reason;
      }
    },
    hover(m) {
      const tip = root().querySelector('#time-hover');
      if (tip?.hidden !== false || m.hover == null || !m.buckets) return;
      const small = tip.querySelector('small');
      if (!small) return;
      const {seconds, first, incidents, values} = m.buckets;
      const i = Math.floor(m.hover / seconds) - first;
      const refusal = lens.unavailable(), coded = unanswered();
      const hit = incidents?.[i] > 0 ? `${incidents[i]} ${incidents[i] === 1 ? 'incident' : 'incidents'}` : '';
      if (coded) small.dataset.reason = coded.code; else delete small.dataset.reason;
      if (refusal) small.dataset.seekReason = reason(refusal).code; else delete small.dataset.seekReason;
      if (!refusal && !coded && !hit) return;
      const said = refusal && reason(refusal).code !== coded?.code ? reason(refusal) : null;
      small.textContent = [coded ? coded.label : `${Math.round(values[i] || 0)} events/s`, hit,
        said ? said.label : refusal || coded ? '' : 'click to seek'].filter(Boolean).join(' · ');
    },
    cursor() {
      const doc = root();
      const unrecorded = source.observedHead > 0 ? null : reason('TIME_UNRECORDED');
      for (const selector of ['#time-current', '#time-offset']) {
        const slot = doc.querySelector(selector);
        if (!slot) continue;
        if (unrecorded) slot.textContent = '';
        if (unrecorded) slot.title = unrecorded.label;
        else if (selector === '#time-offset') slot.title = '';
        if (unrecorded) slot.dataset.reason = unrecorded.code; else delete slot.dataset.reason;
      }
      if (unrecorded) doc.querySelector('#time-track')?.setAttribute?.('aria-valuetext', unrecorded.label);
    },
  };
  return {
    transport: lens, bar,
    read() { restartHead(); return ask(); },
    arrived(rows) {
      let placed = false;
      const members = new Set(actors().map(identity));
      for (const row of rows ?? []) {
        if (isEmission(row) || typeof row?.observed_at_ms !== 'bigint' || !members.has(actorKey(row.actor))) continue;
        const ms = Number(row.observed_at_ms);
        flight?.arrivals.push(ms);
        placed = place(ms) || placed;
      }
      if (placed) edge += 1;
    },
    observed(observed, runtime) {
      const now = observed?.page ? identity(observed.page.anchor?.wall_clock?.at_ms ?? null) : undefined;
      if (now !== undefined && restart !== undefined && now !== restart) void ask();
      if (now !== undefined) restart = now;
      const previous = lettersRead;
      incidents = incidentsOf(observed, runtime);
      if (incidents.letters !== null) lettersRead = incidents.letters;
      if (previous != null && incidents.letters !== null) {
        for (let i = previous; i < incidents.letters; i += 1) {
          const row = runtime.problems.rows[i];
          if (!Object.hasOwn(row, 'observationBucket')) continue;
          const ms = row.observationBucket;
          if (answer) countIncident(localIncidents, Number(answer.bin_ms), ms);
          if (flight) countIncident(flight.incidents, flight.bin, ms);
          if (ms / 1000 > source.observedHead) source.head = ms / 1000;
          edge += 1;
        }
      }
    },
    ran(records) {
      if ((records ?? []).some(record => record?.system?.kind === 'PauseAccepted' || record?.system?.kind === 'ResumeAccepted')) void ask();
    },
    edited() { return ask(); },
    forget() { lens.forget(machine); },
    get asked() { return answers; },
  };
}
