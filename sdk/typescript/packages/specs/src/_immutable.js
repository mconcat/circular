export function deepFreeze(value, seen = new Set()) {
  if (value === null || typeof value !== "object" || seen.has(value)) return value;
  seen.add(value);
  if (value instanceof Set || value instanceof Map) {
    for (const member of value instanceof Map ? value.entries() : value.values()) deepFreeze(member, seen);
  } else {
    for (const member of Object.values(value)) deepFreeze(member, seen);
  }
  return Object.freeze(value);
}

export function diagnostic(code, message, path) {
  return Object.freeze(path === undefined ? { code, message } : { code, message, path });
}
