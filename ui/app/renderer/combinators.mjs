import { address } from './edit.mjs';
import { GAP, STEP_ROOM, TRACK } from './layout.mjs';
import { place, clear } from './placement.mjs';
import { captionBoxes } from './card-size.mjs';
import { readQueryCatalog } from './session.mjs';

const CARD_CLEAR = 12;
const chipBox = (at, room) => ({ x: at.x - room.w / 2, y: at.y - room.h / 2, w: room.w, h: room.h });
const cardBox = n => ({ x: n.x, y: n.y, w: Number(n.width), h: Number(n.height) });

export function chipPlaces(inlet, steps, placed, cards, room, metrics) {
  if (!inlet || steps === 0) return [];
  const front = Math.min(inlet.x, ...(inlet.card ? captionBoxes(inlet.card, metrics) : []).filter(box => box.side === 'in').map(box => box.x));
  const chips = placed.map(at => chipBox(at, room)), boxes = cards.flatMap(card => [cardBox(card), ...captionBoxes(card, metrics)]);
  const preferred = Array.from({ length: steps }, (_, i) => front - GAP / 2 - STEP_ROOM / 2 - (steps - 1 - i) * STEP_ROOM);
  const band = { y: inlet.y - room.h / 2, h: room.h };
  const left = boxes.filter(b => b.x + b.w <= front && !(b.y >= band.y + band.h + CARD_CLEAR || band.y >= b.y + b.h + CARD_CLEAR));
  const need = Math.max(0, ...left.map(b => b.x + b.w + CARD_CLEAR - (preferred[0] - room.w / 2)));
  const shift = need <= front - CARD_CLEAR - (preferred[steps - 1] + room.w / 2) ? need : 0;
  const xs = preferred.map(x => x + shift);
  let top = inlet.y - room.h / 2, lowered = false;
  for (;;) {
    const row = xs.map(x => ({ x: x - room.w / 2, y: top, w: room.w, h: room.h }));
    const below = [...chips.filter(c => row.some(b => !clear(b, c, TRACK))).map(c => c.y + c.h + TRACK),
      ...boxes.filter(c => row.some(b => !clear(b, c, CARD_CLEAR))).map(c => c.y + c.h + CARD_CLEAR)];
    if (!below.length) break;
    top = Math.max(...below);
    lowered = true;
  }
  const y = lowered ? top + room.h / 2 : inlet.y;
  const places = xs.map(x => ({ x, y }));
  placed.push(...places);
  return places;
}

export function draftChipPlace(at, cards, room, metrics) {
  const around = cards.flatMap(card => [cardBox(card), ...captionBoxes(card, metrics)])
    .map(c => ({ x: c.x - room.w / 2, y: c.y - room.h / 2, w: c.w + room.w, h: c.h + room.h }));
  return place([{ x: at.x, y: at.y, w: 0, h: 0 }], around, { grid: 1, margin: CARD_CLEAR })[0];
}

export function changeCombinator(edge, index, direction) {
  const preprocess = [...(edge.attributes.preprocess ?? [])];
  if (!Number.isInteger(index) || index < 0 || index >= preprocess.length) return null;
  if (direction === null) preprocess.splice(index, 1);
  else {
    if (direction !== -1 && direction !== 1) return null;
    const next = index + direction;
    if (next < 0 || next >= preprocess.length) return null;
    [preprocess[index], preprocess[next]] = [preprocess[next], preprocess[index]];
  }
  return {kind:'UpsertEdge', edge:address(edge.address),
    declaration:{...edge.address, attrs:{...edge.attributes, preprocess}}};
}

export function changeStep(edge, index, kind, config, kinds) {
  const preprocess = [...(edge.attributes.preprocess ?? [])];
  const refuse = code => { throw Object.assign(new Error(code), { code }); };
  if (index === preprocess.length) {
    if (!kinds.includes(kind)) refuse('EDGE_PREPROCESS_STEP');
    preprocess.push({ kind, config });
  } else if (preprocess[index]?.kind === kind) preprocess[index] = { kind, config: { ...preprocess[index].config, ...config } };
  else refuse('EDIT_UNAVAILABLE');
  return {kind:'UpsertEdge', edge:address(edge.address),
    declaration:{...edge.address, attrs:{...edge.attributes, preprocess}}};
}

export async function readPreprocessKinds(session) {
  return (await readQueryCatalog(session)).anchor.preprocess.map(entry => entry.kind);
}

const DELIVERY = { Lossless: 'Lossless',
  BestEffortDropNewest: { mode: 'BestEffort', onFull: 'DropNewest' },
  BestEffortDropOldest: { mode: 'BestEffort', onFull: 'DropOldest' } };
export const deliveryName = delivery => typeof delivery === 'string' ? delivery : delivery.mode + delivery.onFull;
export const deliveryExecuted = delivery => Object.hasOwn(DELIVERY, deliveryName(delivery));
const gcd = (a, b) => b === 0n ? a : gcd(b, a % b);
export function changeInletSettings(edge, shownDelay, values) {
  const observed = edge.attributes.policy.delivery;
  const delivery = Object.hasOwn(DELIVERY, values.delivery) ? DELIVERY[values.delivery]
    : values.delivery === deliveryName(observed) ? observed : undefined;
  if (!delivery) return null;
  let delay = edge.attributes.delay;
  if (Number(values.delay) !== shownDelay) {
    if (!/^\d+$/.test(values.delay)) return null;
    const num = BigInt(values.delay), den = 1000n, d = num === 0n ? den : gcd(num, den);
    delay = { num: num / d, den: den / d };
  }
  if (values.capacity !== '' && !/^[1-9]\d*$/.test(values.capacity)) return null;
  const policy = values.capacity === '' ? { delivery } : { delivery, capacity: BigInt(values.capacity) };
  return {kind:'UpsertEdge', edge:address(edge.address),
    declaration:{...edge.address, attrs:{...edge.attributes, delay, policy}}};
}

