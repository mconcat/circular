import { actorIdentityFromValue } from '@circular/protocol/establishment';
import { declarationAddressValue } from '@circular/protocol/declaration';
import { identity } from './query.mjs';
import { reasonText } from './reasons.mjs';

const recorded = page => (page?.items ?? []).filter(row => row.kind === 'actor_arrival'
  && typeof row.observed_at_ms === 'bigint');

export const declaredEdge = edge =>
  identity(declarationAddressValue({ arm: 'relative', value: edge.address }, 'edge', 'snapshot')[1]);

export const RATE_SECONDS = 60;
export const RATE_UNIT = '/min';
export function recordedPerSecond(tally, actor, t) {
  const counted = tally?.within?.(actor, t - 1, t);
  return counted?.seconds ? counted.count / counted.seconds : null;
}
export function recordedRate(tally, actor, head, seconds = RATE_SECONDS) {
  const counted = tally?.within?.(actor, head - seconds, head);
  return counted ? { ...counted, head } : null;
}
export function rateText(rate) {
  return rate?.whole && rate.count ? `${rate.count}${RATE_UNIT}` : null;
}
export const rateTitle = (rate, format = seconds => `${seconds.toFixed(3)} s`) => !rate ? null
  : rate.whole ? `${rate.count} ${rate.count === 1 ? 'arrival' : 'arrivals'} recorded in the last ${RATE_SECONDS} s of recorded time up to ${format(rate.head)}`
    : reasonText('DENSITY_UNREAD');

export const WIRE_RATE_SECONDS = 1;
export function wirePerSecond(tally, wire, t) {
  const counted = tally?.wireWithin?.(wire, t - WIRE_RATE_SECONDS, t);
  return counted?.seconds ? counted.count / counted.seconds : null;
}

export function wireArrival(edge, page) {
  const declared = declaredEdge(edge);
  return recorded(page).findLast(row => row.edge !== undefined && identity(row.edge) === declared);
}
