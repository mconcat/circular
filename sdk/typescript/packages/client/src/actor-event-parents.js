import { sameValue } from '../../protocol/src/internal/value-equal.js';

const WIRE = 1n;
/** A `_lifecycle` row's recorded parents are the record that minted its cell (`kernel/actor.rs`
 * `fly`: `cell_minted`, an instance-transition observation) or none. No arrival is one of them, so a
 * row that carries one shares that record's stamp. */
const LIFECYCLE = '_lifecycle';

/** The declared wire's sending actor; `undefined` for a row that did not arrive over a wire. */
function sendingActor(row) {
  return Array.isArray(row.edge) && row.edge[0] === WIRE ? row.edge[1]?.actor : undefined;
}

/** Stamps are `[l, c, producer, sequence, revision]`; the four UInts are compared first. */
function sameStamp(left, right) {
  if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) return false;
  for (const index of [0, 1, 3, 4]) {
    if (left[index]?.value !== right[index]?.value) return false;
  }
  return sameValue(left[2], right[2]);
}

function sameActor(left, right) {
  return left?.local === right?.local && sameValue(left, right);
}

/** Whether `row` carried the event `stamp` names, and by which recorded field. */
function carried(row, stamp) {
  if (sameStamp(row.at, stamp)) return 'at';
  if (sendingActor(row) !== undefined && sameStamp(row.origin, stamp)) return 'origin';
  return null;
}

function parent(stamp, sender, rows, record) {
  const carriers = [];
  let own = 0;
  for (const candidate of rows) {
    if (sender !== undefined && !sameActor(candidate.actor, sender)) continue;
    const field = carried(candidate, stamp);
    if (field === null) continue;
    carriers.push(candidate);
    if (field === 'at') own += 1;
  }
  if (record ? carriers.length > 0 : own > 0 && carriers.length > 1) {
    return { stamp, status: 'undecidable', code: 'stamp_shared', candidates: carriers };
  }
  if (own === 1) return { stamp, status: 'found', row: carriers[0] };
  if (carriers.length === 0) return { stamp, status: 'absent', code: 'not_in_rows' };
  if (sender === undefined) return { stamp, status: 'undecidable', code: 'no_sending_end', candidates: carriers };
  if (carriers.length === 1) return { stamp, status: 'found', row: carriers[0] };
  return { stamp, status: 'undecidable', code: 'emission_arrived_twice', candidates: carriers };
}

/** The rows `row`'s recorded `causal_parents` name, one answer per recorded parent, in order. */
export function actorEventParents(row, rows) {
  if (!Object.hasOwn(row, 'causal_parents')) return { status: 'unknown' };
  const sender = sendingActor(row), record = row.port === LIFECYCLE;
  return { status: 'recorded', parents: row.causal_parents.map(stamp => parent(stamp, sender, rows, record)) };
}
