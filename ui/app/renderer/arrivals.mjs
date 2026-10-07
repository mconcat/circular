import { actorIdentityFromValue } from '@circular/protocol/establishment';
import { identity, actorKey } from './query.mjs';
import { observeActors } from './scene.mjs';
import { reasonText } from './reasons.mjs';

export const emissionKind = 'actor_emission';
export const isEmission = row => row?.kind === emissionKind;
const edgeTag = row => Array.isArray(row?.edge) ? row.edge[0] : null;
export const wireSource = row => edgeTag(row) === 1n ? row.edge[1] : null;
export const isReservedIngress = row => edgeTag(row) === 2n;
export const producerStamp = row => {
  const stamp = row?.origin?.[2];
  return stamp && typeof stamp === 'object' && !Array.isArray(stamp) ? stamp : null;
};
const ownRecord = row => {
  const stamp = producerStamp(row);
  return stamp !== null && row.edge === undefined && row.actor !== undefined
    && actorKey(stamp) === actorKey(row.actor);
};
export const isOwnOutcome = row => ownRecord(row) && row.body !== null && typeof row.body === 'object'
  && typeof row.body.kind === 'string' && typeof row.body.ok === 'boolean';
export const isReceived = row => !isReservedIngress(row) && !ownRecord(row);

export function viewRecords(node, direction = 'arrivals', { sided = false, project } = {}) {
  const read = (rows, side) => project ? rows.map(row => project(row, recordPort(node, row, side), side)) : rows;
  if (node.portRecords) return sided && direction !== node.portRecords.direction ? []
    : read(node.portRecords.rows, node.portRecords.direction);
  if (direction === 'both') return [...read(node.arrivals ?? [], 'arrivals'), ...read(node.emitted ?? [], 'emitted')];
  return read(node[direction] ?? [], direction);
}

export const recordPort = (node, row) => node.portRecords?.port ?? row.port;
export const drawnSide = (node, side = 'emitted') => node.portRecords?.direction ?? side;
export const drawnPorts = (node, side = 'out') => node.portRecords
  ? (node.portRecords.direction === 'arrivals' ? node.in : node.out) : node[side];

export const stampSequence = row => {
  const sequence = row?.at?.[3];
  return typeof sequence === 'bigint' ? sequence : typeof sequence?.value === 'bigint' ? sequence.value : null;
};
const most = (map, id, value) => { if (value !== null && !(map.get(id) >= value)) map.set(id, value); };
export function columnEnds(page) {
  const ends = new Map();
  for (const component of page?.cut ?? []) most(ends, identity(actorIdentityFromValue(component.actor)), component.index);
  for (const row of page?.items ?? [])
    if (row?.kind === 'actor_arrival' && typeof row.index === 'bigint') most(ends, actorKey(row.actor), row.index + 1n);
  return ends;
}
export function distribute(graph, page, only) {
  const received = new Map(), emitted = new Map(), stamped = new Map(), ends = columnEnds(page);
  const add = (map, id, row) => { const rows = map.get(id); if (rows) rows.push(row); else map.set(id, [row]); };
  for (const row of page?.items ?? []) {
    if (isEmission(row)) {
      const producer = actorKey(row.actor);
      add(emitted, producer, row);
      const sequence = stampSequence(row);
      most(stamped, producer, sequence === null ? null : sequence + 1n);
    } else if (row.kind === 'actor_arrival') add(received, actorKey(row.actor), row);
  }
  const known = Array.isArray(page?.cut) || page?.terminal === 2n;
  return observeActors(graph, node => ({ arrivals: received.get(node.id) ?? [], emitted: emitted.get(node.id) ?? [],
    recordedArrivals: ends.get(node.id) ?? (known ? 0n : null), recordedEmissions: stamped.get(node.id) ?? null }), only);
}

export const recordedArrivalsText = node => typeof node?.recordedArrivals === 'bigint'
  ? `${node.recordedArrivals} ${node.recordedArrivals === 1n ? 'arrival' : 'arrivals'} recorded at this actor`
  : reasonText('READ_UNAVAILABLE');

export const latestRow = rows => (rows ?? []).reduce((last, row) => !last
  || (typeof row.observed_at_ms === 'bigint' && !(row.observed_at_ms < last.observed_at_ms)) ? row : last, null);

export function outletReading(node, port, edges = []) {
  const rows = viewRecords(node, 'emitted').filter(row => recordPort(node, row) === port);
  const wired = edges.some(edge => edge.from === node.id && edge.out === port);
  const latest = latestRow(rows);
  return { port, wired, unwired: !wired && port !== errorOutlet, count: rows.length, latest,
    code: latest ? null : 'EMISSION_UNOBSERVED' };
}
export const primaryOutlet = node => node.portRecords?.port ?? node.registration?.out_ports?.find(row => row.primary)?.id
  ?? node.out?.[0]?.[0];
export function primaryOutletReading(node, edges = []) {
  const port = primaryOutlet(node);
  return port === undefined ? null : outletReading(node, port, edges);
}
export const errorOutlet = '_error';
export function outletReadings(node, edges = []) {
  if (node.portRecords) return [outletReading(node, node.portRecords.port, edges)];
  const declared = (node.out ?? []).map(([port]) => port);
  const recorded = viewRecords(node, 'emitted').map(row => row.port).filter(port => port != null);
  return [...new Set([...declared, ...recorded])].map(port => outletReading(node, port, edges));
}
export function instanceContainers(row) {
  const { scope } = actorIdentityFromValue(row.actor);
  return scope.flatMap((segment, n) => segment.of === undefined ? []
    : [identity({ scope: scope.slice(0, n), local: segment.of })]);
}

export const causeUnrecorded = 'OUTCOME_CAUSE_UNRECORDED';
export const causeUnread = 'OUTCOME_CAUSE_UNREAD';
export const causeAmbiguous = 'OUTCOME_CAUSE_AMBIGUOUS';
export function causingArrival(outcome, column) {
  const occasion = outcome?.occasion;
  if (!occasion || typeof occasion !== 'object' || occasion.origin === undefined) return { code: causeUnrecorded };
  const origin = identity(occasion.origin);
  const edge = occasion.edge === undefined ? null : identity(occasion.edge);
  const actor = identity(outcome.actor);
  const rows = (column ?? []).filter(row => identity(row.actor) === actor
    && row.origin !== undefined && identity(row.origin) === origin
    && (edge === null || row.edge === undefined || identity(row.edge) === edge));
  return rows.length === 1 ? { row: rows[0] } : { code: rows.length ? causeAmbiguous : causeUnread };
}
