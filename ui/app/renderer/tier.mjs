export const TIERS = Object.freeze(['names', 'reduced', 'detail']);
export const DETAIL = 'detail';
export const NAMES = 'names';

const BOUNDS = Object.freeze([
  Object.freeze({ tier: 'names', enter: 0.35, leave: 0.4 }),
  Object.freeze({ tier: 'reduced', enter: 0.72, leave: 0.8 }),
]);
const rank = tier => TIERS.indexOf(tier);

export function tierAt(zoom, held = DETAIL) {
  for (const bound of BOUNDS)
    if (zoom < (rank(held) <= rank(bound.tier) ? bound.leave : bound.enter)) return bound.tier;
  return DETAIL;
}

export function leastZoom(tier) {
  const farther = BOUNDS[rank(tier) - 1];
  return farther ? farther.enter : 0;
}

export const symbolScale = zoom => Math.max(1, 1 / zoom);

export const TITLE_LINES = Object.freeze({ names: null, reduced: 2, detail: 1 });

export const wordSegments = text => String(text ?? '').split(/(?<=[\s_/])|(?<=[-.])(?![\d\s])|(?<=\p{Ll})(?=\p{Lu})/u).filter(Boolean);
export const wordsHTML = (text, escape) => wordSegments(text)
  .map((segment, i, all) => escape(segment) + (i < all.length - 1 && !/\s$/u.test(segment) ? '<wbr>' : '')).join('');

export function holdsControl(card, zoom, room) {
  if (!room) return true;
  const scale = symbolScale(zoom);
  return card.width >= room.symbol.width * scale + room.world.width
    && card.height >= room.symbol.height * scale + room.world.height;
}

export const TIER_TITLE = Object.freeze({
  names: 'Overview · names and health',
  reduced: 'Overview · names and live views',
  detail: 'Detail · ports and controls',
});
