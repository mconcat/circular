const millis = carrier => {
  const value = carrier && typeof carrier === 'object' && 'value' in carrier ? carrier.value : carrier;
  return typeof value === 'bigint' ? Number(value) : null;
};

export function wallClockOf(anchor) {
  const pair = anchor?.wall_clock;
  if (!pair) return null;
  const atMs = millis(pair.at_ms), wallMs = millis(pair.wall_ms);
  return atMs === null || wallMs === null ? null : Object.freeze({ atMs, wallMs });
}

export function publishWallClock(history, anchor) {
  if (!anchor || typeof anchor !== 'object') return false;
  const next = wallClockOf(anchor), prior = history.wallClock;
  history.wallClock = next;
  return prior === undefined
    || (prior?.atMs ?? null) !== (next?.atMs ?? null) || (prior?.wallMs ?? null) !== (next?.wallMs ?? null);
}
