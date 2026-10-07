import { GRID, clear, place } from './placement.mjs';
import { captionReach, captionRise, portCaption } from './card-size.mjs';

export { GRID };

export const GAP = 113;
export const STEP_ROOM = 90;
export const TRACK = 16;
export const channel = tracks => GAP + (Math.max(tracks, 1) - 1) * TRACK;
const reach = (n, side, metrics) => captionReach(side, (n[side] ?? []).map(portCaption), metrics);
export const footprint = (n, metrics, { x, y } = n) =>
  ({ x, y, w: Number(n.width), h: Number(n.height), left: reach(n, 'in', metrics), right: reach(n, 'out', metrics) });

export function portSlot(n, metrics) {
  const { head, rise } = captionRise(metrics);
  return Math.ceil(head + rise) + n * Math.ceil(TRACK / 2 + Math.max(TRACK / 2, rise));
}
export const portsFoot = (count, metrics) =>
  count > 0 ? portSlot(count - 1, metrics) + Math.max(TRACK / 2, metrics.inset + metrics.portFoot) : 0;

const onGrid = value => Math.round(value / GRID) * GRID;
const upToGrid = value => Math.ceil(value / GRID) * GRID;
const mean = values => values.reduce((sum, value) => sum + value, 0) / values.length;

const groupBy = (values, of) => {
  const groups = new Map();
  for (const value of values) {
    const at = of(value);
    if (!groups.has(at)) groups.set(at, []);
    groups.get(at).push(value);
  }
  return groups;
};

function condense(nodes, leaving) {
  const seen = new Map(), low = new Map(), open = new Set(), stack = [], component = new Map();
  let count = 0;
  for (const root of nodes) {
    if (seen.has(root.id)) continue;
    const frames = [{ id: root.id, at: 0 }];
    while (frames.length) {
      const frame = frames[frames.length - 1], out = leaving.get(frame.id) ?? [];
      if (!seen.has(frame.id)) {
        seen.set(frame.id, seen.size); low.set(frame.id, seen.get(frame.id));
        stack.push(frame.id); open.add(frame.id);
      }
      if (frame.at < out.length) {
        const to = out[frame.at++].to;
        if (!seen.has(to)) frames.push({ id: to, at: 0 });
        else if (open.has(to)) low.set(frame.id, Math.min(low.get(frame.id), seen.get(to)));
        continue;
      }
      if (low.get(frame.id) === seen.get(frame.id)) {
        for (let member; member !== frame.id; ) {
          member = stack.pop();
          open.delete(member);
          component.set(member, count);
        }
        count++;
      }
      frames.pop();
      const parent = frames[frames.length - 1];
      if (parent) low.set(parent.id, Math.min(low.get(parent.id), low.get(frame.id)));
    }
  }
  return component;
}

function connected(nodes, wires) {
  const parent = new Map(nodes.map(n => [n.id, n.id]));
  const find = id => { while (parent.get(id) !== id) { parent.set(id, parent.get(parent.get(id))); id = parent.get(id); } return id; };
  for (const wire of wires) parent.set(find(wire.from), find(wire.to));
  const numbers = new Map(), group = new Map();
  for (const n of nodes) {
    const root = find(n.id);
    if (!numbers.has(root)) numbers.set(root, numbers.size);
    group.set(n.id, numbers.get(root));
  }
  return group;
}

function unitsOf(nodes, inner, seat) {
  const cycle = condense(nodes, groupBy(inner, wire => wire.from)), bySeat = (a, b) => seat.get(a) - seat.get(b);
  const unit = new Map();
  let next = nodes.length;
  for (const [, members] of [...groupBy(nodes, n => cycle.get(n.id))].sort(([a], [b]) => b - a)) {
    const held = new Set(members.map(n => n.id));
    const entered = inner.filter(wire => held.has(wire.to) && !held.has(wire.from)).map(wire => wire.to).sort(bySeat);
    const stack = [entered[0] ?? members[0].id];
    while (stack.length) {
      const id = stack.pop();
      if (unit.has(id)) continue;
      unit.set(id, --next);
      stack.push(...inner.filter(wire => wire.from === id && held.has(wire.to) && !unit.has(wire.to)).map(wire => wire.to).sort(bySeat).reverse());
    }
  }
  return unit;
}

function flowOf(nodes, wires) {
  const held = new Set(nodes.map(n => n.id)), seat = new Map(nodes.map((n, i) => [n.id, i]));
  const inner = wires.filter(wire => held.has(wire.from) && held.has(wire.to));
  const unit = unitsOf(nodes, inner, seat);
  const across = inner.filter(wire => unit.get(wire.from) > unit.get(wire.to));
  const inFlow = [...across].sort((a, b) => unit.get(b.from) - unit.get(a.from));
  const column = new Map();
  for (const wire of inFlow) column.set(wire.to, Math.max(column.get(wire.to) ?? 0, (column.get(wire.from) ?? 0) + 1));
  const steps = new Map();
  for (const wire of wires) if (held.has(wire.to))
    steps.set(wire.to, Math.max(steps.get(wire.to) ?? 0, wire.attributes?.preprocess?.length ?? 0));
  return { inner, across, inFlow, unit, group: connected(nodes, inner), steps, seat, columnOf: id => column.get(id) ?? 0 };
}

function columnsNear(nodes, flow, current) {
  const { unit, group, across, inFlow } = flow;
  const early = new Map(nodes.map(n => [n.id, flow.columnOf(n.id)]));
  const ahead = new Map();
  for (const wire of [...inFlow].reverse()) ahead.set(wire.from, Math.max(ahead.get(wire.from) ?? 0, (ahead.get(wire.to) ?? 0) + 1));
  const last = new Map();
  for (const [id, column] of early) last.set(group.get(id), Math.max(last.get(group.get(id)) ?? 0, column));
  const late = id => last.get(group.get(id)) - (ahead.get(id) ?? 0);
  const anchors = new Map();
  for (const [id, column] of early) if (late(id) === column) {
    const key = `${group.get(id)}:${column}`;
    anchors.set(key, [...(anchors.get(key) ?? []), current.get(id).x]);
  }
  const leaving = groupBy(across, wire => wire.from);
  const chosen = new Map(), least = new Map();
  for (const { id } of [...nodes].sort((a, b) => unit.get(b.id) - unit.get(a.id))) {
    const low = Math.max(early.get(id), least.get(id) ?? 0), high = Math.max(late(id), low);
    const far = column => Math.abs(mean(anchors.get(`${group.get(id)}:${column}`)) - current.get(id).x);
    let best = low;
    for (let column = low + 1; column <= high; column++) if (far(column) < far(best)) best = column;
    chosen.set(id, best);
    for (const wire of leaving.get(id) ?? []) least.set(wire.to, Math.max(least.get(wire.to) ?? 0, best + 1));
  }
  return id => chosen.get(id);
}

const SWEEPS = 8;
function ordered(columns, flow, columnOf, current) {
  const { seat, inner } = flow;
  if (current) {
    for (const held of columns) held.sort((a, b) => current.get(a.id).y - current.get(b.id).y || seat.get(a.id) - seat.get(b.id));
    return columns;
  }
  const neighbours = new Map(columns.flat().map(n => [n.id, []]));
  for (const wire of inner) {
    if (!neighbours.has(wire.from) || columnOf(wire.from) === columnOf(wire.to)) continue;
    neighbours.get(wire.to).push(wire.from);
    neighbours.get(wire.from).push(wire.to);
  }
  const order = new Map(columns.flatMap(held => held.map((n, i) => [n.id, i])));
  for (let sweep = 0, moved = true; sweep < SWEEPS && moved; sweep++) {
    moved = false;
    for (const side of [-1, 1]) {
      for (const held of side < 0 ? columns.slice(1) : columns.slice(0, -1).reverse()) {
        const pull = new Map(held.map(n => {
          const near = neighbours.get(n.id).filter(id => Math.sign(columnOf(id) - columnOf(n.id)) === side);
          return [n.id, near.length ? mean(near.map(id => order.get(id))) : order.get(n.id)];
        }));
        const before = held.map(n => n.id);
        held.sort((a, b) => pull.get(a.id) - pull.get(b.id) || seat.get(a.id) - seat.get(b.id));
        for (const [i, n] of held.entries()) {
          order.set(n.id, i);
          if (before[i] !== n.id) moved = true;
        }
      }
    }
  }
  return columns;
}

function formOf(nodes, flow, columnOf, current, metrics) {
  const columns = ordered([...groupBy(nodes, n => columnOf(n.id))].sort(([a], [b]) => a - b).map(([, held]) => [...held]), flow, columnOf, current);
  const index = new Map(columns.flatMap((held, i) => held.map(n => [n.id, i])));
  const tracks = columns.map(() => 0);
  for (const wire of flow.inner) {
    if (!index.has(wire.from)) continue;
    const from = index.get(wire.from), to = index.get(wire.to);
    tracks[from]++;
    if (to - 1 !== from && to > 0) tracks[to - 1]++;
  }
  const rows = columns.map(held => {
    const tops = [0];
    for (let i = 1; i < held.length; i++) tops.push(upToGrid(tops[i - 1] + Number(held[i - 1].height) + GAP));
    return { tops, height: tops[tops.length - 1] + Number(held[held.length - 1].height) };
  });
  const tall = Math.max(...rows.map(row => row.height)), at = new Map();
  let x = 0;
  for (const [i, held] of columns.entries()) {
    const top = onGrid((tall - rows[i].height) / 2);
    for (const [row, n] of held.entries()) at.set(n.id, { x, y: top + rows[i].tops[row] });
    const next = columns[i + 1] ?? [], steps = Math.max(0, ...next.map(n => flow.steps.get(n.id) ?? 0));
    x = upToGrid(x + Math.max(...held.map(n => Number(n.width))) + Math.max(...held.map(n => reach(n, 'out', metrics)))
      + channel(tracks[i]) + steps * STEP_ROOM + Math.max(0, ...next.map(n => reach(n, 'in', metrics))));
  }
  return { at, height: tall };
}

/**
 * The flow form of one scope's actors: every actor's place, as a Map of id → {x, y}.
 *
 * Cold (no `current`): the groups stand one under another from `origin`, in declaration order.
 * With `current` — every actor's present place, id → {x, y} — the form nearest to those places is
 * answered (Organize): each actor keeps the column the viewer has it nearest to where the wires leave
 * it free, each column keeps the viewer's order top to bottom, and each group stays about where it
 * is (its form's centre on its present centre), moved clear of the groups already put.
 *
 * `metrics` are the measuring document's (card-size.mjs documentMetrics): each channel is laid
 * past the port captions they measure. Every caller names them; where nothing measures it passes
 * card-size.mjs UNMEASURED.
 */
export function organized(nodes, wires, origin, metrics, current) {
  const flow = flowOf(nodes, wires), at = new Map();
  const columnOf = current ? columnsNear(nodes, flow, current) : flow.columnOf;
  const taken = [];
  let y = onGrid(origin.y);
  for (const held of groupBy(nodes, n => flow.group.get(n.id)).values()) {
    const form = formOf(held, flow, columnOf, current, metrics);
    if (!current) {
      for (const n of held) at.set(n.id, { x: onGrid(origin.x) + form.at.get(n.id).x, y: y + form.at.get(n.id).y });
      y = upToGrid(y + form.height + GAP);
      continue;
    }
    const shift = axis => onGrid(mean(held.map(n => current.get(n.id)[axis])) - mean(held.map(n => form.at.get(n.id)[axis])));
    const dx = shift('x'), dy = shift('y');
    const asked = held.map(n => footprint(n, metrics, { x: form.at.get(n.id).x + dx, y: form.at.get(n.id).y + dy }));
    for (const [i, put] of place(asked, taken).entries()) {
      at.set(held[i].id, put);
      taken.push(footprint(held[i], metrics, put));
    }
  }
  return at;
}

function placeScope(nodes, wires, origin, metrics, shown) {
  const stand = new Map(nodes.filter(n => n.presentation?.fixed).map(n => [n.id, footprint(n, metrics)]));
  const declared = [...stand.values()], at = new Map();
  for (const n of nodes) {
    const was = !stand.has(n.id) && shown.get(n.id);
    if (was && declared.every(d => clear(footprint(n, metrics, was), d, 0))) { stand.set(n.id, footprint(n, metrics, was)); at.set(n.id, { x: was.x, y: was.y }); }
  }
  if (stand.size === nodes.length) return at;
  if (stand.size === 0) return organized(nodes, wires, origin, metrics);
  const flow = flowOf(nodes, wires), { seat, steps } = flow;
  const taken = [...stand.values()];
  const into = groupBy(flow.across, wire => wire.to), out = groupBy(flow.across, wire => wire.from);
  const byId = new Map(nodes.map(n => [n.id, n]));
  const room = id => GAP + (steps.get(id) ?? 0) * STEP_ROOM + reach(byId.get(id), 'in', metrics);
  const right = id => stand.get(id).x + stand.get(id).w + reach(byId.get(id), 'out', metrics);
  for (const n of [...nodes].sort((a, b) => flow.columnOf(a.id) - flow.columnOf(b.id) || seat.get(a.id) - seat.get(b.id))) {
    if (stand.has(n.id)) continue;
    const ups = (into.get(n.id) ?? []).map(wire => wire.from).filter(id => stand.has(id));
    const downs = (out.get(n.id) ?? []).filter(wire => stand.has(wire.to));
    const asked = ups.length ? { x: Math.max(...ups.map(right)) + room(n.id), y: Math.min(...ups.map(id => stand.get(id).y)) }
      : downs.length ? { x: Math.min(...downs.map(wire => stand.get(wire.to).x - room(wire.to))) - reach(n, 'out', metrics) - Number(n.width),
        y: Math.min(...downs.map(wire => stand.get(wire.to).y)) }
      : origin;
    const [put] = place([footprint(n, metrics, asked)], taken);
    stand.set(n.id, footprint(n, metrics, put));
    taken.push(stand.get(n.id));
    at.set(n.id, put);
  }
  return at;
}

/**
 * Places every actor that declared no position, one scope at a time, and returns the nodes.
 *
 * `shown` maps an actor id to the spot this viewer last drew it at while it declared no
 * position; such an actor keeps that spot. With none standing in a scope (a first screen of
 * actors that declare no position) the scope takes its flow form from `origin`.
 *
 * A node keeps its identity when its placement does not change, so folding one accepted
 * epoch forward stays a fold and never becomes a rebuild. Every node carries the width and
 * height its card is drawn at (scene.mjs). `metrics` are the measuring document's, as `organized`'s.
 */
export function placed(nodes, wires, origin, metrics, shown = new Map()) {
  const at = new Map();
  for (const [, held] of groupBy(nodes, n => n.scope))
    for (const entry of placeScope(held, wires, origin, metrics, shown)) at.set(entry[0], entry[1]);
  return nodes.map(n => {
    const put = at.get(n.id);
    return put === undefined || (n.x === put.x && n.y === put.y) ? n : { ...n, ...put };
  });
}
