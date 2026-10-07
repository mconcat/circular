export const GRID = 24;
export const MARGIN = 48;

const span = b => ({ x: b.x - (b.left ?? 0), w: b.w + (b.left ?? 0) + (b.right ?? 0) });

export const clear = (a, b, margin = MARGIN) => {
  const p = span(a), q = span(b);
  return p.x >= q.x + q.w + margin || q.x >= p.x + p.w + margin || a.y >= b.y + b.h + margin || b.y >= a.y + a.h + margin;
};

function blocked(boxes, standing, grid, margin) {
  const ranges = [];
  for (const b of boxes) for (const s of standing) {
    const p = span(b), q = span(s);
    ranges.push([(q.x - p.x - p.w - margin) / grid, (q.x + q.w + margin - p.x) / grid,
      (s.y - b.y - b.h - margin) / grid, (s.y + s.h + margin - b.y) / grid]);
  }
  return (dx, dy) => ranges.some(([x0, x1, y0, y1]) => dx > x0 && dx < x1 && dy > y0 && dy < y1);
}

function nearestShift(boxes, standing, grid, margin) {
  const meets = blocked(boxes, standing, grid, margin);
  let best = null;
  const take = (dx, dy) => {
    if (meets(dx, dy)) return;
    const d = dx * dx + dy * dy;
    if (best === null || d < best.d || (d === best.d && (dy > best.dy || (dy === best.dy && dx > best.dx)))) best = { dx, dy, d };
  };
  for (let r = 0; best === null || best.d >= r * r; r++) {
    if (r === 0) { take(0, 0); continue; }
    for (let i = -r; i <= r; i++) { take(i, -r); take(i, r); }
    for (let j = -r + 1; j < r; j++) { take(-r, j); take(r, j); }
  }
  return best;
}

/**
 * The legal places of `moving` among `standing`. Each box is {x, y, w, h}, with the room its captions take
 * beside it where it carries it (`left`, `right`); `moving` holds the places asked for, and the answer is
 * their places, in the same order, as [{x, y}].
 *
 * Boxes moved together keep their arrangement: each corner is put on the grid, and the whole group
 * takes the nearest grid shift at which every one of them is clear of `standing`. A box of the group
 * that would then stand within the margin of another of the group (they were asked for closer than
 * that) is given the nearest clear place of its own, in order, so no answer overlaps another.
 */
export function place(moving, standing, { grid = GRID, margin = MARGIN } = {}) {
  const snap = value => Math.round(value / grid) * grid;
  const on = moving.map(b => ({ ...b, x: snap(b.x), y: snap(b.y) }));
  const together = nearestShift(on, standing, grid, margin);
  const taken = [...standing], places = [];
  for (const b of on) {
    const at = { ...b, x: b.x + together.dx * grid, y: b.y + together.dy * grid };
    const own = nearestShift([at], taken, grid, margin);
    const put = { ...b, x: at.x + own.dx * grid, y: at.y + own.dy * grid };
    taken.push(put);
    places.push({ x: put.x, y: put.y });
  }
  return places;
}
