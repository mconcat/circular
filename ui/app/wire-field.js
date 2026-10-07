(() => {
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
  const smooth = (x) => x * x * (3 - 2 * x);
  const hash = (n) => {
    const v = Math.sin(n * 127.1 + 311.7) * 43758.5453;
    return v - Math.floor(v);
  };
  const noise = (x) => {
    const i = Math.floor(x),
      f = smooth(x - i);
    return hash(i) * (1 - f) + hash(i + 1) * f;
  };

  function clearLine(a, b, obstacles) {
    return !obstacles.some((r) =>
      Math.abs(a.y - b.y) < 0.01
        ? a.y > r.top + 0.1 &&
          a.y < r.bottom - 0.1 &&
          Math.max(a.x, b.x) > r.left + 0.1 &&
          Math.min(a.x, b.x) < r.right - 0.1
        : a.x > r.left + 0.1 &&
          a.x < r.right - 0.1 &&
          Math.max(a.y, b.y) > r.top + 0.1 &&
          Math.min(a.y, b.y) < r.bottom - 0.1,
    );
  }
  const SAME = 1e-6;
  const less = (a, b, aSoft = 0, bSoft = 0) =>
    Math.abs(a - b) > SAME || aSoft === bSoft ? a < b : aSoft < bSoft;
  const before = (a, b) => less(a.cost, b.cost, a.soft, b.soft);
  class Heap {
    items = [];
    push(value) {
      const q = this.items;
      q.push(value);
      let i = q.length - 1;
      while (i) {
        const p = (i - 1) >> 1;
        if (!before(value, q[p])) break;
        q[i] = q[p];
        i = p;
      }
      q[i] = value;
    }
    pop() {
      const q = this.items,
        first = q[0],
        last = q.pop();
      if (q.length) {
        let i = 0;
        while (i * 2 + 1 < q.length) {
          let child = i * 2 + 1;
          if (child + 1 < q.length && before(q[child + 1], q[child]))
            child++;
          if (!before(q[child], last)) break;
          q[i] = q[child];
          i = child;
        }
        q[i] = last;
      }
      return first;
    }
  }
  function simplify(points) {
    const out = [];
    for (const p of points) {
      const a = out.at(-2),
        b = out.at(-1);
      if (b && Math.hypot(b.x - p.x, b.y - p.y) < 0.01) continue;
      if (
        a &&
        b &&
        ((a.x === b.x && b.x === p.x) || (a.y === b.y && b.y === p.y))
      )
        out.pop();
      out.push(p);
    }
    return out;
  }
  function route(start, end, obstacles, soft = []) {
    if (
      clearLine(start, end, obstacles) &&
      (start.x === end.x || start.y === end.y)
    )
      return [start, end];
    const xs = [
      ...new Set([
        start.x,
        end.x,
        ...[...obstacles, ...soft].flatMap((r) => [r.left, r.right]),
      ]),
    ].sort((a, b) => a - b);
    const ys = [
      ...new Set([
        start.y,
        end.y,
        ...[...obstacles, ...soft].flatMap((r) => [r.top, r.bottom]),
      ]),
    ].sort((a, b) => a - b);
    const w = xs.length,
      h = ys.length,
      total = w * h;
    const first = ys.indexOf(start.y) * w + xs.indexOf(start.x),
      last = ys.indexOf(end.y) * w + xs.indexOf(end.x);
    const acrossBlocked = new Uint8Array(total),
      downBlocked = new Uint8Array(total);
    for (const r of obstacles) {
      for (let y = 0; y < h; y++) {
        if (ys[y] > r.top + 0.1 && ys[y] < r.bottom - 0.1)
          for (let x = 0; x + 1 < w; x++)
            if (xs[x + 1] > r.left + 0.1 && xs[x] < r.right - 0.1) acrossBlocked[y * w + x] = 1;
      }
      for (let x = 0; x < w; x++) {
        if (xs[x] > r.left + 0.1 && xs[x] < r.right - 0.1)
          for (let y = 0; y + 1 < h; y++)
            if (ys[y + 1] > r.top + 0.1 && ys[y] < r.bottom - 0.1) downBlocked[y * w + x] = 1;
      }
    }
    const blocked = (x, y, nx, ny) =>
      ny === y ? acrossBlocked[y * w + Math.min(x, nx)] : downBlocked[Math.min(y, ny) * w + x];
    const acrossSoft = new Float64Array(soft.length ? total : 0),
      downSoft = new Float64Array(soft.length ? total : 0);
    for (const r of soft) {
      for (let y = 0; y < h; y++) {
        if (ys[y] > r.top + 0.1 && ys[y] < r.bottom - 0.1)
          for (let x = 0; x + 1 < w; x++)
            if (xs[x + 1] > r.left + 0.1 && xs[x] < r.right - 0.1) acrossSoft[y * w + x] = xs[x + 1] - xs[x];
      }
      for (let x = 0; x < w; x++) {
        if (xs[x] > r.left + 0.1 && xs[x] < r.right - 0.1)
          for (let y = 0; y + 1 < h; y++)
            if (ys[y + 1] > r.top + 0.1 && ys[y] < r.bottom - 0.1) downSoft[y * w + x] = ys[y + 1] - ys[y];
      }
    }
    const over = (x, y, nx, ny) =>
      !soft.length ? 0 : ny === y ? acrossSoft[y * w + Math.min(x, nx)] : downSoft[Math.min(y, ny) * w + x];
    const distance = new Float64Array(total * 2).fill(Infinity),
      overlay = new Float64Array(total * 2),
      prev = new Int32Array(total * 2).fill(-1),
      heap = new Heap();
    distance[first * 2] = 0;
    heap.push({ key: first * 2, cost: 0, travel: 0, soft: 0 });
    let winner = -1;
    while (heap.items.length) {
      const current = heap.pop(),
        key = current.key,
        id = key >> 1,
        dir = key & 1;
      if (current.travel !== distance[key] || current.soft !== overlay[key]) continue;
      if (id === last) {
        winner = key;
        break;
      }
      const x = id % w,
        y = (id / w) | 0,
        a = { x: xs[x], y: ys[y] };
      for (const [nx, ny, nd] of [
        [x - 1, y, 0],
        [x + 1, y, 0],
        [x, y - 1, 1],
        [x, y + 1, 1],
      ]) {
        if (nx < 0 || ny < 0 || nx >= w || ny >= h) continue;
        const ni = ny * w + nx;
        const b = { x: xs[nx], y: ys[ny] };
        if (blocked(x, y, nx, ny)) continue;
        const travel =
            current.travel +
            Math.abs(b.x - a.x) +
            Math.abs(b.y - a.y) +
            (nd !== dir ? 34 : 0),
          laid = current.soft + over(x, y, nx, ny),
          nk = ni * 2 + nd;
        if (less(travel, distance[nk], laid, overlay[nk])) {
          distance[nk] = travel;
          overlay[nk] = laid;
          prev[nk] = key;
          heap.push({
            key: nk,
            travel,
            soft: laid,
            cost: travel + Math.abs(b.x - end.x) + Math.abs(b.y - end.y),
          });
        }
      }
    }
    if (winner < 0) {
      const lane =
        Math.max(start.y, end.y, ...obstacles.map((r) => r.bottom)) + 24;
      return simplify([
        start,
        { x: start.x, y: lane },
        { x: end.x, y: lane },
        end,
      ]);
    }
    const result = [];
    for (let key = winner; key >= 0; key = prev[key]) {
      const id = key >> 1;
      result.push({ x: xs[id % w], y: ys[(id / w) | 0] });
    }
    return simplify(result.reverse());
  }
  const RADIUS = 30;
  const inside = (pt, r) => pt.x > r.left + 0.1 && pt.x < r.right - 0.1 && pt.y > r.top + 0.1 && pt.y < r.bottom - 0.1;
  function cornerRadius(a, b, c, around) {
    const ab = Math.hypot(b.x - a.x, b.y - a.y),
      bc = Math.hypot(c.x - b.x, c.y - b.y);
    const clearAt = (r) => {
      const q = { x: b.x + ((a.x - b.x) / ab) * r, y: b.y + ((a.y - b.y) / ab) * r },
        s = { x: b.x + ((c.x - b.x) / bc) * r, y: b.y + ((c.y - b.y) / bc) * r };
      for (let k = 1; k < 8; k++) {
        const t = k / 8, u = 1 - t,
          pt = { x: u * u * q.x + 2 * u * t * b.x + t * t * s.x, y: u * u * q.y + 2 * u * t * b.y + t * t * s.y };
        if (around.some((box) => inside(pt, box))) return false;
      }
      return true;
    };
    for (let r = Math.min(RADIUS, ab / 2, bc / 2); r >= 1; r /= 2) if (clearAt(r)) return r;
    return 0;
  }
  function rounded(points, around = []) {
    const p = simplify(points);
    if (p.length < 2) return "";
    around = around.filter((box) => !box.caption || p.every((q, i) => i === 0 || clearLine(p[i - 1], q, [box])));
    let d = `M${p[0].x},${p[0].y}`;
    for (let i = 1; i < p.length - 1; i++) {
      const a = p[i - 1],
        b = p[i],
        c = p[i + 1],
        r = cornerRadius(a, b, c, around);
      if (r === 0) {
        d += ` L${b.x},${b.y}`;
        continue;
      }
      const ab = Math.hypot(b.x - a.x, b.y - a.y),
        bc = Math.hypot(c.x - b.x, c.y - b.y);
      d += ` L${b.x + ((a.x - b.x) / ab) * r},${b.y + ((a.y - b.y) / ab) * r} Q${b.x},${b.y} ${b.x + ((c.x - b.x) / bc) * r},${b.y + ((c.y - b.y) / bc) * r}`;
    }
    return d + ` L${p.at(-1).x},${p.at(-1).y}`;
  }
  function winding(ring, p) {
    let n = 0;
    for (let i = 0; i < ring.length; i++) {
      const a = ring[i],
        b = ring[(i + 1) % ring.length],
        side = (b.x - a.x) * (p.y - a.y) - (p.x - a.x) * (b.y - a.y);
      if (a.y <= p.y && b.y > p.y && side > 0) n++;
      else if (a.y > p.y && b.y <= p.y && side < 0) n--;
    }
    return n;
  }
  const length = (p) => p.reduce((sum, q, i) => (i ? sum + Math.abs(q.x - p[i - 1].x) + Math.abs(q.y - p[i - 1].y) : 0), 0);
  function leg(from, to, obstacles) {
    if (to.x >= from.x && Math.abs(to.y - from.y) < 0.01) {
      const contains = (r, p) =>
        p.x >= r.left && p.x <= r.right && p.y >= r.top && p.y <= r.bottom;
      const between = obstacles.filter(
        (r) => !contains(r, from) && !contains(r, to),
      );
      if (clearLine(from, to, between)) return [from, to];
    }
    const start = { x: from.x + 20, y: from.y },
      end = { x: to.x - 20, y: to.y },
      around = route(start, end, obstacles),
      walls = obstacles.filter((r) => !r.caption);
    if (walls.length === obstacles.length) return simplify([from, ...around, to]);
    const over = route(start, end, walls, obstacles.filter((r) => r.caption)),
      ring = [...around, ...over.slice().reverse()];
    const longWay =
      length(around) > length(over) + EPS &&
      walls.some((r) => winding(ring, { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 }) !== 0);
    return simplify([from, ...(longWay ? over : around), to]);
  }
  const EPS = 0.01;
  const sideOf = (v) => (v > EPS ? 1 : v < -EPS ? -1 : 0);
  const upright = (p, k) => Math.abs(p[k - 1].x - p[k].x) < EPS;
  function diverge(P, i, Q, j, w, depth = 0) {
    const along = upright(P, i) ? "y" : "x",
      across = along === "y" ? "x" : "y";
    const outer = (p, k) => ((p[k][along] - p[k - 1][along]) * w > 0 ? k : k - 1);
    const next = (p, k, e) =>
      e === k
        ? k + 1 < p.length ? { run: k + 1, far: k + 1 } : null
        : k - 1 >= 1 ? { run: k - 1, far: k - 2 } : null;
    const eP = outer(P, i), eQ = outer(Q, j), nP = next(P, i, eP), nQ = next(Q, j, eQ);
    const turn = (p, e, n) => sideOf(p[n.far][across] - p[e][across]);
    const pAt = P[eP][along] * w, qAt = Q[eQ][along] * w;
    if (pAt < qAt - EPS) return nP ? turn(P, eP, nP) : 0;
    if (qAt < pAt - EPS) return nQ ? -turn(Q, eQ, nQ) : 0;
    if (!nP || !nQ || depth > 64) return 0;
    const dP = turn(P, eP, nP), dQ = turn(Q, eQ, nQ);
    if (dP !== dQ) return dP || -dQ;
    return -dP * w * diverge(P, nP.run, Q, nQ.run, dP, depth + 1);
  }
  function lay(wires, walls, spacing, axis) {
    const along = axis === "x" ? "y" : "x";
    const runs = [];
    wires.forEach((wire, index) => {
      const p = wire.points;
      for (let k = 1; k < p.length; k++) {
        const a = p[k - 1],
          b = p[k];
        if (Math.abs(a[axis] - b[axis]) >= EPS || Math.abs(a[along] - b[along]) < EPS) continue;
        const c = a[axis],
          lo = Math.min(a[along], b[along]),
          hi = Math.max(a[along], b[along]);
        const held =
          k === 1 ||
          k === p.length - 1 ||
          wire.held.some(
            (h) => Math.abs(h[axis] - c) < EPS && h[along] > lo - EPS && h[along] < hi + EPS,
          );
        runs.push({ wire, index, k, c, lo, hi, held, L: c, R: c, wallL: c, wallR: c });
      }
    });
    for (const r of runs) {
      if (r.held) continue;
      let L = -Infinity,
        R = Infinity;
      for (const o of walls) {
        const [lo, hi, low, high] =
          axis === "x" ? [o.top, o.bottom, o.left, o.right] : [o.left, o.right, o.top, o.bottom];
        if (lo >= r.hi - EPS || hi <= r.lo + EPS) continue;
        if (high <= r.c + EPS) L = Math.max(L, high);
        else if (low >= r.c - EPS) R = Math.min(R, low);
        else {
          L = R = r.c;
          break;
        }
      }
      r.wallL = L;
      r.wallR = R;
      const p = r.wire.points;
      for (const [far, end] of [
        [r.k - 2, r.k - 2 === 0],
        [r.k + 1, r.k + 1 === p.length - 1],
      ]) {
        if (far < 0 || far >= p.length) continue;
        const f = p[far][axis],
          fixed = end || r.wire.held.some((h) => Math.abs(h.x - p[far].x) < EPS && Math.abs(h.y - p[far].y) < EPS);
        const bound = fixed ? f : (f + r.c) / 2;
        if (f < r.c - EPS) L = Math.max(L, fixed ? bound + 1 : bound);
        else if (f > r.c + EPS) R = Math.min(R, fixed ? bound - 1 : bound);
      }
      if (L > R) L = R = r.c;
      r.L = L;
      r.R = R;
    }
    const overlap = (al, ah, bl, bh) =>
      al === ah || bl === bh
        ? Math.max(al, bl) <= Math.min(ah, bh) + EPS
        : Math.max(al, bl) < Math.min(ah, bh) - EPS;
    const reach = (r) => [
      Number.isFinite(r.L) ? r.L : r.c - spacing,
      Number.isFinite(r.R) ? r.R : r.c + spacing,
    ];
    const parent = runs.map((_, i) => i);
    const root = (i) => (parent[i] === i ? i : (parent[i] = root(parent[i])));
    const near = (a, b, m) => a.lo < b.hi + m && b.lo < a.hi + m;
    for (let i = 0; i < runs.length; i++)
      for (let j = i + 1; j < runs.length; j++) {
        const a = runs[i],
          b = runs[j];
        if (a.wire === b.wire || (a.held && b.held) || !near(a, b, spacing)) continue;
        const [al, ah] = reach(a),
          [bl, bh] = reach(b);
        if (overlap(al, ah, bl, bh)) parent[root(i)] = root(j);
      }
    const channels = new Map();
    runs.forEach((r, i) => {
      const at = root(i);
      if (!channels.has(at)) channels.set(at, []);
      channels.get(at).push(r);
    });
    for (const members of channels.values()) {
      if (members.every((r) => r.held)) continue;
      place(members, spacing, axis, along);
    }
  }
  function place(members, spacing, axis, along) {
    const free = members.filter((r) => !r.held);
    let wallLow = Math.max(...free.map((r) => r.wallL)),
      wallHigh = Math.min(...free.map((r) => r.wallR));
    if (wallLow > wallHigh) [wallLow, wallHigh] = [-Infinity, Infinity];
    const outward = !Number.isFinite(wallLow) && Number.isFinite(wallHigh) ? -1 : 1;
    const n = members.length;
    const order = (a, b) => {
      if (a.held && b.held) return sideOf(a.c - b.c);
      const first = a.index <= b.index ? a : b,
        pa = a.wire.points,
        w = sideOf(first.wire.points[first.k][along] - first.wire.points[first.k - 1][along]);
      const found =
        diverge(pa, a.k, b.wire.points, b.k, w) || diverge(pa, a.k, b.wire.points, b.k, -w);
      if (found) return found;
      if (a.wire.back !== b.wire.back) return (a.wire.back ? 1 : -1) * outward;
      return sideOf(a.c - b.c) || sideOf(a.index - b.index);
    };
    const before = members.map(() => new Set()),
      waiting = members.map(() => 0);
    for (let i = 0; i < n; i++)
      for (let j = i + 1; j < n; j++) {
        const a = members[i],
          b = members[j];
        if (a.lo >= b.hi - EPS || b.lo >= a.hi - EPS) continue;
        const o = order(a, b);
        if (o < 0) {
          before[i].add(j);
          waiting[j]++;
        } else if (o > 0) {
          before[j].add(i);
          waiting[i]++;
        }
      }
    const sorted = [],
      done = new Set();
    while (sorted.length < n) {
      let pick = -1;
      for (let i = 0; i < n; i++) {
        if (done.has(i)) continue;
        const a = members[i],
          b = members[pick];
        if (pick < 0 || waiting[i] < waiting[pick] || (waiting[i] === waiting[pick] && (sideOf(a.c - b.c) || sideOf(a.index - b.index)) < 0))
          pick = i;
      }
      done.add(pick);
      sorted.push(pick);
      for (const j of before[pick]) waiting[j]--;
    }
    const track = new Map();
    let tracks = 0;
    sorted.forEach((i, at) => {
      let t = 0;
      for (const j of sorted.slice(0, at))
        if (members[i].lo < members[j].hi + spacing && members[j].lo < members[i].hi + spacing)
          t = Math.max(t, track.get(j) + 1);
      track.set(i, t);
      tracks = Math.max(tracks, t + 1);
    });
    const stand = new Array(tracks).fill(undefined),
      wanted = new Array(tracks).fill(0),
      count = new Array(tracks).fill(0);
    members.forEach((r, i) => {
      const t = track.get(i);
      if (r.held && stand[t] === undefined) stand[t] = r.c;
      if (!r.held) {
        wanted[t] += r.c;
        count[t]++;
      }
    });
    const at = stand.map((c, t) => (c !== undefined ? c : wanted[t] / count[t]));
    for (let t = 0; t < tracks; ) {
      if (stand[t] !== undefined) {
        t++;
        continue;
      }
      let u = t;
      while (u < tracks && stand[u] === undefined) u++;
      const m = u - t,
        low = t > 0 ? stand[t - 1] : wallLow,
        high = u < tracks ? stand[u] : wallHigh,
        lowGap = t > 0 ? spacing : 0,
        highGap = u < tracks ? spacing : 0;
      if (Number.isFinite(low) && Number.isFinite(high)) {
        const step = Math.min(spacing, (high - low) / (m + 1)),
          first = (low + high) / 2 - (step * (m - 1)) / 2;
        for (let k = 0; k < m; k++) at[t + k] = first + k * step;
      } else if (Number.isFinite(low)) {
        for (let k = t; k < u; k++) at[k] = Math.max(at[k], k === t ? low + lowGap : at[k - 1] + spacing);
      } else if (Number.isFinite(high)) {
        for (let k = u - 1; k >= t; k--) at[k] = Math.min(at[k], k === u - 1 ? high - highGap : at[k + 1] - spacing);
      } else {
        const mean = at.slice(t, u).reduce((sum, c) => sum + c, 0) / m;
        for (let k = t + 1; k < u; k++) at[k] = Math.max(at[k], at[k - 1] + spacing);
        const shift = mean - at.slice(t, u).reduce((sum, c) => sum + c, 0) / m;
        for (let k = t; k < u; k++) at[k] += shift;
      }
      t = u;
    }
    members.forEach((r, i) => {
      if (r.held) return;
      const to = Math.min(r.R, Math.max(r.L, at[track.get(i)]));
      const p = r.wire.points;
      p[r.k - 1] = { ...p[r.k - 1], [axis]: to };
      p[r.k] = { ...p[r.k], [axis]: to };
    });
  }
  function tracks(wires, walls, spacing) {
    const laid = wires.map((w) => ({ ...w, points: simplify(w.points.map((p) => ({ x: p.x, y: p.y }))) }));
    lay(laid, walls, spacing, "x");
    lay(laid, walls, spacing, "y");
    return new Map(laid.map((w) => [w.id, simplify(w.points)]));
  }
  const TAIL = 56,
    TAIL_STEP = 0.5;
  function sample(d) {
    const tokens = d.match(/[MLQ]|[-+]?(?:\d*\.)?\d+(?:e[-+]?\d+)?/gi) || [];
    const segments = [];
    let x = 0,
      y = 0,
      length = 0;
    function lineTo(nx, ny) {
      const size = Math.hypot(nx - x, ny - y);
      if (size > 0) {
        segments.push({ x, y, dx: nx - x, dy: ny - y, start: length, size });
        length += size;
      }
      x = nx;
      y = ny;
    }
    for (let i = 0; i < tokens.length;) {
      const command = tokens[i++];
      if (command === "M") {
        x = Number(tokens[i++]);
        y = Number(tokens[i++]);
      } else if (command === "L")
        lineTo(Number(tokens[i++]), Number(tokens[i++]));
      else if (command === "Q") {
        const ax = x,
          ay = y,
          bx = Number(tokens[i++]),
          by = Number(tokens[i++]),
          cx = Number(tokens[i++]),
          cy = Number(tokens[i++]);
        const steps = Math.max(
          1,
          Math.ceil(
            Math.sqrt(Math.hypot(ax - 2 * bx + cx, ay - 2 * by + cy) / 0.32),
          ),
        );
        for (let j = 1; j <= steps; j++) {
          const t = j / steps,
            u = 1 - t;
          lineTo(
            u * u * ax + 2 * u * t * bx + t * t * cx,
            u * u * ay + 2 * u * t * by + t * t * cy,
          );
        }
      } else
        throw new Error(
          "Unexpected command in generated wire geometry: " + command,
        );
    }
    function at(distance) {
      if (!segments.length) return { x, y };
      let lo = 0,
        hi = segments.length - 1;
      while (lo < hi) {
        const mid = (lo + hi) >> 1,
          s = segments[mid];
        if (s.start + s.size < distance) lo = mid + 1;
        else hi = mid;
      }
      const s = segments[lo],
        t = clamp((distance - s.start) / s.size, 0, 1);
      return { x: s.x + s.dx * t, y: s.y + s.dy * t };
    }
    const count = clamp(Math.ceil(length / 5) + 1, 12, 240),
      points = new Float32Array(count * 4);
    for (let i = 0; i < count; i++) {
      const distance = (length * i) / (count - 1),
        p = at(distance),
        before = at(Math.max(0, distance - 1)),
        after = at(Math.min(length, distance + 1));
      const dx = after.x - before.x,
        dy = after.y - before.y,
        m = Math.hypot(dx, dy) || 1;
      points.set([p.x, p.y, -dy / m, dx / m], i * 4);
    }
    const tailLength = Math.min(length, TAIL),
      tailCount = Math.max(2, Math.ceil(tailLength / TAIL_STEP) + 1),
      tail = {
        count: tailCount,
        points: new Float32Array(tailCount * 4),
        s: new Float32Array(tailCount),
        radii: new Float32Array(tailCount),
        offsets: new Float32Array(tailCount),
        light: new Float32Array(tailCount),
      };
    for (let k = 0; k < tailCount; k++) {
      const back = (tailLength * k) / (tailCount - 1),
        distance = length - back,
        p = at(distance),
        before = at(Math.max(0, distance - 1)),
        after = at(Math.min(length, distance + 1));
      const dx = after.x - before.x,
        dy = after.y - before.y,
        m = Math.hypot(dx, dy) || 1;
      tail.points.set([p.x, p.y, -dy / m, dx / m], k * 4);
      tail.s[k] = back;
    }
    return {
      d,
      length,
      count,
      points,
      radii: new Float32Array(count),
      offsets: new Float32Array(count),
      light: new Float32Array(count),
      tail,
    };
  }
  function flowMaterial() {
    const css = typeof getComputedStyle === "function"
      ? getComputedStyle(document.documentElement)
      : null;
    const token = (name) => css?.getPropertyValue(name).trim();
    return {
      strand: token("--hf-06-wire-strand"),
      light: rgb(token("--hf-06-wire-light")) ?? [255, 255, 255],
      backlog: rgb(token("--red")) ?? [165, 68, 55],
    };
  }
  const rgb = (hex) => {
    const m = /^#([0-9a-f]{6})$/i.exec(hex ?? "");
    return m ? [0, 2, 4].map((i) => parseInt(m[1].slice(i, i + 2), 16)) : null;
  };
  const paintOf = ([r, g, b]) => `rgb(${Math.round(r)},${Math.round(g)},${Math.round(b)})`;
  const mix = (a, b, f) => a.map((v, i) => v + (b[i] - v) * f);
  const flowLight = (y) => {
    const x = 3 * y;
    return x > 0 ? (x * x) / (x + 8) : 0;
  };
  function outline(ctx, shape, widths, inset = 0) {
    const n = shape.count,
      points = shape.points,
      offsets = shape.offsets;
    ctx.beginPath();
    for (let side = 1; side >= -1; side -= 2)
      for (let j = 0; j < n; j++) {
        const i = side === 1 ? j : n - 1 - j,
          p = i * 4,
          r = Math.max(0, widths[i] - inset) * side + offsets[i],
          x = points[p] + points[p + 2] * r,
          y = points[p + 1] + points[p + 3] * r;
        if (side === 1 && j === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
    ctx.closePath();
  }
  function glow(ctx, shape, widths, paint, layers) {
    ctx.fillStyle = paint;
    for (const [inset, alpha] of layers) {
      outline(ctx, shape, widths, inset);
      ctx.globalAlpha = alpha;
      ctx.fill();
    }
    ctx.globalAlpha = 1;
  }
  function poolProfile(s, depth) {
    if (!(depth > 0)) return 0;
    const P = Field.pool,
      size = Math.min(P.most, P.first + P.grow * Math.log2(1 + depth)),
      drops = Math.min(Math.ceil(depth), P.drops);
    let sum = 0;
    for (let j = 0; j < drops; j++) {
      const r = size * (1 - 0.1 * j) * clamp(depth - j, 0, 1),
        u = (s - (P.lip + size * (0.95 + 1.15 * j))) / (r * 1.3 + 1e-6);
      if (Math.abs(u) < 1) sum += (r * Math.sqrt(1 - u * u)) ** 4;
    }
    return sum ** 0.25;
  }
  function extentOf(geometry) {
    const p = geometry.points;
    let left = Infinity,
      top = Infinity,
      right = -Infinity,
      bottom = -Infinity;
    for (let i = 0; i < geometry.count; i++) {
      left = Math.min(left, p[i * 4]);
      right = Math.max(right, p[i * 4]);
      top = Math.min(top, p[i * 4 + 1]);
      bottom = Math.max(bottom, p[i * 4 + 1]);
    }
    return { left, top, right, bottom };
  }
  class Field {
    static transitMs = 500;
    static still = 1 / 256;
    static light = [
      [0, 0.35],
      [0.3, 0.5],
      [0.6, 0.65],
      [0.9, 0.8],
    ];
    static lightShare = 0.66;
    static pool = {
      first: 2,
      grow: 1.2,
      most: 7,
      drops: 5,
      lip: 5,
      ease: 7,
      light: [
        [0, 0.22],
        [0.5, 0.26],
        [1, 0.3],
        [1.5, 0.36],
      ],
    };
    static strand = 0.55;
    constructor(canvas, options) {
      this.canvas = canvas;
      this.ctx = canvas.getContext("2d");
      this.lightCanvas = options.lightCanvas;
      this.lightCtx = this.lightCanvas.getContext("2d");
      this.options = options;
      this.lines = [];
      this.mailboxes = new Map();
      this.raf = 0;
      this.last = 0;
      this.samples = 0;
      this.elapsed = 0;
      this.clock = 0;
      this.recordedAt = null;
      this.unpresented = 0;
      this.moving = false;
      this.active = false;
      this.reduced = matchMedia("(prefers-reduced-motion: reduce)");
      this.stale = [];
      this.view = null;
      this.loop = this.loop.bind(this);
      this.reduced.addEventListener("change", () => this.wake());
      document.addEventListener("visibilitychange", () => this.wake());
    }
    setLines(lines) {
      const previous = new Map(this.lines.map((l) => [l.id, l]));
      this.lines = lines.map((l) => {
        const old = previous.get(l.id);
        previous.delete(l.id);
        if (old && old.points === l.geometry.points) {
          old.seed = l.seed;
          old.d = l.d;
          return old;
        }
        if (old) this.stale.push(old);
        return {
          ...l,
          ...l.geometry,
          bounds: extentOf(l.geometry),
          drawn: null,
          volume: old?.volume ?? 0,
          pool: old?.pool ?? 0,
          red: old?.red ?? 0,
          pressure: old?.pressure ?? {
            history: new Float32Array(128),
            head: 0,
            remainder: 0,
            phase: l.seed % 1,
            p: 0,
            v: 0,
          },
        };
      });
      for (const gone of previous.values()) this.stale.push(gone);
      this.wake();
    }
    setMailboxes(rows) {
      const next = new Map();
      for (const row of rows ?? []) {
        const inlet = Number(row?.depth),
          depth = inlet + Number(row?.queued ?? 0);
        if (row?.wire == null || !(depth > 0)) continue;
        const capacity = Number(row.capacity);
        next.set(row.wire, {
          depth,
          inlet,
          capacity: capacity > 0 ? capacity : null,
          backpressure: row.backpressure === true,
        });
      }
      this.mailboxes = next;
      this.wake();
    }
    seek(seconds) {
      this.clock = seconds * 1000;
      this.last = 0;
      for (const line of this.lines) {
        const rate = this.options.rate(line.id, seconds),
          pressure = line.pressure;
        line.volume = 1 - Math.exp((-rate * 0.5) / 2.5);
        pressure.p = 0;
        pressure.v = 0;
        pressure.head = 0;
        pressure.remainder = 0;
        pressure.phase =
          (line.seed + Math.max(0, seconds - 128 / 120) * rate) % 1;
        for (let i = 0; i < 128; i++) {
          pressure.phase +=
            this.options.rate(line.id, Math.max(0, seconds - (127 - i) / 120)) /
            120;
          const arrivals = Math.floor(pressure.phase);
          pressure.phase -= arrivals;
          pressure.v += arrivals * 115;
          pressure.v += (-36 * pressure.v - 324 * pressure.p) / 120;
          pressure.p = Math.max(0, pressure.p + pressure.v / 120);
          pressure.head = (pressure.head + 1) % 128;
          pressure.history[pressure.head] = 1 - Math.exp(-pressure.p * 1.5);
        }
      }
      this.draw(seconds, 0);
    }
    resize(width, height) {
      const dpr = Math.min(devicePixelRatio || 1, 2);
      for (const canvas of [this.canvas, this.lightCanvas])
        if (
          canvas.width !== Math.round(width * dpr) ||
          canvas.height !== Math.round(height * dpr)
        ) {
          canvas.width = Math.round(width * dpr);
          canvas.height = Math.round(height * dpr);
          canvas.style.width = width + "px";
          canvas.style.height = height + "px";
          this.view = null;
        }
      if (this.dpr !== dpr) this.view = null;
      this.dpr = dpr;
      this.wake();
    }
    wake() {
      if (!this.raf && this.options.visible() && !document.hidden)
        this.raf = requestAnimationFrame(this.loop);
    }
    loop(time) {
      this.raf = 0;
      if (document.hidden || !this.options.visible()) {
        this.last = 0;
        return;
      }
      const dt = this.last ? Math.min(time - this.last, 100) : 0;
      this.last = time;
      const previousClock = this.clock;
      const at = this.options.time ? this.options.time() : null,
        history = at !== null && Boolean(this.options.recorded?.());
      if (history) this.clock = at * 1000;
      else if (!this.options.paused()) this.clock += dt;
      let presenting = at === null;
      if (!history && at !== null) {
        if (at !== this.recordedAt) {
          this.recordedAt = at;
          this.unpresented = this.options.rateSeconds;
        }
        presenting = this.unpresented > 0;
        this.unpresented = Math.max(0, this.unpresented - dt / 1000);
      }
      this.draw(
        this.clock / 1000,
        history
          ? Math.max(0, Math.min(0.4, (this.clock - previousClock) / 1000))
          : dt / 1000,
        false,
        history || presenting ? (at ?? this.clock / 1000) : null,
      );
      this.options.tick?.(at ?? this.clock / 1000);
      const decaying = !history && this.lines.some((line) => line.flowing);
      if (!this.moving && !presenting && !decaying) {
        this.last = 0;
        return;
      }
      if (!this.reduced.matches && !this.options.paused())
        this.raf = requestAnimationFrame(this.loop);
    }
    draw(seconds, dt = 1 / 60, full = true, rateAt = seconds) {
      const ctx = this.ctx,
        z = this.options.transform(),
        dpr = this.dpr || 1,
        width = this.canvas.width / dpr,
        height = this.canvas.height / dpr;
      const view = this.view;
      if (!view || view.x !== z.x || view.y !== z.y || view.zoom !== z.zoom) {
        full = true;
        this.lightCanvas.style.transform =
          `scale(${1 / z.zoom}) translate(${-z.x}px,${-z.y}px)`;
      }
      this.view = { x: z.x, y: z.y, zoom: z.zoom };
      if (full) this.material = flowMaterial();
      const still = Field.still / (z.zoom * dpr);
      const changed = [],
        present = !this.options.recorded?.();
      let easing = false;
      for (const line of this.lines) {
        const rate = rateAt === null ? 0 : this.options.rate(line.id, rateAt);
        const target = 1 - Math.exp((-rate * 0.5) / 2.5);
        line.volume +=
          (target - line.volume) * (1 - Math.exp(-Math.max(dt, 0.001) * 5));
        const pressure = line.pressure,
          step = 1 / 120;
        pressure.remainder += dt;
        while (pressure.remainder >= step) {
          pressure.remainder -= step;
          pressure.phase += rate * step;
          const arrivals = Math.floor(pressure.phase);
          pressure.phase -= arrivals;
          pressure.v += arrivals * 115;
          pressure.v += (-36 * pressure.v - 324 * pressure.p) * step;
          pressure.p = Math.max(0, pressure.p + pressure.v * step);
          pressure.head = (pressure.head + 1) % 128;
          pressure.history[pressure.head] = 1 - Math.exp(-pressure.p * 1.5);
        }
        const density = clamp(line.volume / 0.38, 0, 1),
          dense = smooth(density);
        const live =
          !this.reduced.matches &&
          (!this.options.paused() || this.options.recorded?.());
        const mailbox = present ? this.mailboxes.get(line.id) : undefined,
          depth = mailbox?.depth ?? 0,
          red = mailbox
            ? mailbox.backpressure
              ? 1
              : mailbox.capacity
                ? clamp(mailbox.inlet / mailbox.capacity, 0, 1)
                : 0
            : 0;
        const follow = live ? 1 - Math.exp(-dt * Field.pool.ease) : 1;
        line.pool += (depth - line.pool) * follow;
        line.red += (red - line.red) * follow;
        if (Math.abs(depth - line.pool) < 0.004) line.pool = depth;
        if (Math.abs(red - line.red) < 0.004) line.red = red;
        if (line.pool !== depth || line.red !== red) easing = true;
        let thinnest = Infinity;
        for (let i = 0; i < line.count; i++) {
          const t = i / (line.count - 1),
            local = seconds + line.seed - t * 0.5;
          const age = t * 60,
            whole = Math.floor(age),
            fraction = age - whole;
          const p0 = pressure.history[(pressure.head - whole + 128) % 128],
            p1 = pressure.history[(pressure.head - whole - 1 + 128) % 128];
          const merged = p0 * (1 - fraction) + p1 * fraction;
          const swell =
            (noise(local * 0.8 + line.seed * 11) - 0.5) * 1.5 +
            (noise(local * 1.3 + t * 1.8 + 21) - 0.5) * 0.6;
          const radius =
            0.85 +
            (1 - dense) * merged * 3.7 +
            dense * (1.5 + line.volume * 4.1 + swell);
          line.radii[i] = live ? radius : 0.85;
          line.offsets[i] = live
            ? dense * (noise(local * 0.9 + t * 2.3 + 44) - 0.5) * 0.9
            : 0;
          thinnest = Math.min(thinnest, line.radii[i]);
        }
        for (let i = 0; i < line.count; i++)
          line.light[i] = live
            ? Math.min(line.radii[i] * Field.lightShare, flowLight(line.radii[i] - thinnest))
            : 0;
        let reach = 0,
          lit = 0,
          moved = !line.drawn;
        for (let i = 0; i < line.count; i++) {
          reach = Math.max(reach, Math.abs(line.radii[i]) + Math.abs(line.offsets[i]));
          lit = Math.max(lit, line.light[i]);
          if (
            !moved &&
            (Math.abs(line.drawn.radii[i] - line.radii[i]) > still ||
              Math.abs(line.drawn.offsets[i] - line.offsets[i]) > still ||
              Math.abs(line.drawn.light[i] - line.light[i]) > still)
          )
            moved = true;
        }
        line.flowing = live && reach > 0.85 + still;
        line.lit = lit > still;
        if (line.pool > 0) {
          const tail = line.tail,
            last = line.count - 1;
          for (let k = 0; k < tail.count; k++) {
            const at = line.length > 0 ? last * (1 - tail.s[k] / line.length) : last,
              i = Math.min(Math.floor(at), last - 1),
              f = at - i,
              flow = line.radii[i] * (1 - f) + line.radii[i + 1] * f,
              pool = poolProfile(tail.s[k], line.pool),
              r = Math.max(line.flowing ? flow : Field.strand, Field.strand + pool);
            tail.radii[k] = r;
            tail.offsets[k] = line.offsets[i] * (1 - f) + line.offsets[i + 1] * f;
            tail.light[k] = Math.min(r * Field.lightShare, pool);
            reach = Math.max(reach, r + Math.abs(tail.offsets[k]));
          }
        }
        moved ||= line.drawn?.flowing !== line.flowing || line.drawn?.pool !== line.pool ||
          line.drawn?.red !== line.red;
        line.reach = reach;
        if (moved) changed.push(line);
      }
      this.moving = changed.length > 0 || easing;
      const keep = (line) => {
        line.drawn ||= {
          radii: new Float32Array(line.count),
          offsets: new Float32Array(line.count),
          light: new Float32Array(line.count),
        };
        line.drawn.radii.set(line.radii);
        line.drawn.offsets.set(line.offsets);
        line.drawn.light.set(line.light);
        line.drawn.reach = line.reach;
        line.drawn.flowing = line.flowing;
        line.drawn.pool = line.pool;
        line.drawn.red = line.red;
      };
      const band = (line, reach) => {
        const b = line.bounds,
          pad = (reach + 0.25) * z.zoom + 1 / dpr;
        return {
          left: Math.floor((z.x + b.left * z.zoom - pad) * dpr) / dpr,
          top: Math.floor((z.y + b.top * z.zoom - pad) * dpr) / dpr,
          right: Math.ceil((z.x + b.right * z.zoom + pad) * dpr) / dpr,
          bottom: Math.ceil((z.y + b.bottom * z.zoom + pad) * dpr) / dpr,
        };
      };
      let paint = this.lines,
        bands = null;
      if (full) this.stale = [];
      else {
        bands = [
          ...this.stale.filter((l) => l.drawn).map((l) => band(l, l.drawn.reach)),
          ...changed.map((l) => band(l, Math.max(l.reach, l.drawn?.reach ?? 0))),
        ]
          .map((r) => ({
            left: Math.max(0, r.left),
            top: Math.max(0, r.top),
            right: Math.min(width, r.right),
            bottom: Math.min(height, r.bottom),
          }))
          .filter((r) => r.right > r.left && r.bottom > r.top);
        this.stale = [];
        if (!bands.length) {
          changed.forEach(keep);
          return;
        }
        const meets = (a, r) =>
          a.left < r.right && a.right > r.left && a.top < r.bottom && a.bottom > r.top;
        paint = this.lines.filter((l) =>
          bands.some((r) => meets(band(l, l.reach), r)),
        );
      }
      const lightCtx = this.lightCtx;
      for (const c of [ctx, lightCtx]) {
        c.setTransform(dpr, 0, 0, dpr, 0, 0);
        if (full) {
          c.clearRect(0, 0, width, height);
        } else {
          c.save();
          c.beginPath();
          for (const r of bands) {
            c.rect(r.left, r.top, r.right - r.left, r.bottom - r.top);
            c.clearRect(r.left, r.top, r.right - r.left, r.bottom - r.top);
          }
          c.clip();
        }
        c.translate(z.x, z.y);
        c.scale(z.zoom, z.zoom);
      }
      const light = this.material.light,
        lightPaint = paintOf(light);
      for (const line of paint) {
        ctx.fillStyle = this.material.strand;
        if (line.flowing) {
          outline(ctx, line, line.radii);
          ctx.fill();
        }
        if (line.pool > 0) {
          outline(ctx, line.tail, line.tail.radii);
          ctx.fill();
        }
        if (line.lit) glow(lightCtx, line, line.light, lightPaint, Field.light);
        if (line.pool > 0)
          glow(ctx, line.tail, line.tail.light,
            paintOf(mix(light, this.material.backlog, line.red)), Field.pool.light);
        keep(line);
      }
      changed.forEach(keep);
      if (!full) for (const c of [ctx, lightCtx]) c.restore();
    }
  }
  window.WireGeometry = {
    leg,
    rounded,
    route,
    sample,
    tracks,
  };
  class Travel {
    static omega = 25;
    static stagger = 25;
    static still = 0.1;
    static spring(o0, u0, t) {
      const w = Travel.omega,
        e = Math.exp(-w * t),
        c = u0 + w * o0;
      return { offset: e * (o0 + c * t), speed: e * (u0 - w * c * t) };
    }
    static settles(o0, u0) {
      const w = Travel.omega,
        axes = [[Math.abs(o0.x), Math.abs(u0.x + w * o0.x)], [Math.abs(o0.y), Math.abs(u0.y + w * o0.y)]],
        peak = Math.max(0, ...axes.map(([a, b]) => (b > 0 ? (b - w * a) / (w * b) : 0))),
        above = (t) => axes.some(([a, b]) => Math.exp(-w * t) * (a + b * t) > Travel.still);
      let t = 0;
      while (t < 5 && (t < peak || above(t))) t += 1 / 240;
      return t;
    }
    constructor({ frame, now, reduced, onFrame, onEnd }) {
      Object.assign(this, { frame, now, reduced, onFrame, onEnd });
      this.tracks = new Map();
      this.raf = 0;
      this.loop = this.loop.bind(this);
    }
    get moving() {
      return this.tracks.size > 0;
    }
    state(track, time) {
      const t = Math.max(0, time - track.start - track.delay) / 1000;
      return { x: Travel.spring(track.o0.x, track.u0.x, t), y: Travel.spring(track.o0.y, track.u0.y, t) };
    }
    at(id, time = this.now()) {
      const track = this.tracks.get(id);
      if (!track) return undefined;
      const s = this.state(track, time);
      return { x: track.to.x + s.x.offset, y: track.to.y + s.y.offset };
    }
    go(element, id, from, to, order = 0) {
      const time = this.now(),
        held = this.tracks.get(id);
      let seen = from,
        u0 = { x: 0, y: 0 };
      if (held) {
        const s = this.state(held, time);
        seen = { x: held.to.x + s.x.offset, y: held.to.y + s.y.offset };
        u0 = { x: s.x.speed, y: s.y.speed };
      }
      const o0 = { x: seen.x - to.x, y: seen.y - to.y },
        duration = this.reduced() ? 0 : Travel.settles(o0, u0) * 1000;
      if (!(duration > 0) || typeof element.animate !== "function") return this.stop(id);
      const steps = Math.max(2, Math.ceil(duration / (1000 / 60)) + 1),
        keyframes = [];
      for (let k = 0; k < steps; k++) {
        const t = ((k / (steps - 1)) * duration) / 1000,
          x = k === steps - 1 ? 0 : Travel.spring(o0.x, u0.x, t).offset,
          y = k === steps - 1 ? 0 : Travel.spring(o0.y, u0.y, t).offset;
        keyframes.push({ translate: `${x}px ${y}px` });
      }
      const delay = held ? 0 : order * Travel.stagger,
        animation = element.animate(keyframes, { duration, delay, fill: "backwards", easing: "linear" });
      animation.startTime = time;
      held?.animation.cancel();
      this.tracks.set(id, { element, to: { x: to.x, y: to.y }, o0, u0, start: time, delay, end: time + delay + duration, animation });
      if (!this.raf) this.raf = this.frame(this.loop);
    }
    stop(id) {
      const track = this.tracks.get(id);
      if (!track) return;
      track.animation.cancel();
      this.tracks.delete(id);
      if (!this.tracks.size) this.onEnd();
    }
    loop(time) {
      this.raf = 0;
      for (const [id, track] of this.tracks)
        if (time >= track.end || !track.element.isConnected || this.reduced()) {
          track.animation.cancel();
          this.tracks.delete(id);
        }
      if (!this.tracks.size) return this.onEnd();
      this.onFrame(new Set(this.tracks.keys()), time);
      this.raf = this.frame(this.loop);
    }
  }
  window.WireField = Field;
  window.CardTravel = Travel;
})();
