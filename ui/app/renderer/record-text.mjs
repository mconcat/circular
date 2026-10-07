const utf8 = new TextDecoder('utf-8', { fatal: true });
export function bytesText(bytes) {
  try { return utf8.decode(bytes); } catch { return `<${bytes.length} bytes>`; }
}
const uint = value => value !== null && typeof value === 'object' && !Array.isArray(value)
  && Object.isFrozen(value) && Object.keys(value).length === 1 && typeof value.value === 'bigint';
export const recordValue = value => value === undefined ? undefined : JSON.parse(JSON.stringify(value, (_key, item) =>
  typeof item === 'bigint' ? String(item) : item instanceof Uint8Array ? bytesText(item)
    : item instanceof Map ? Object.fromEntries(item) : uint(item) ? String(item.value) : item));
export function recordText(value) {
  const shown = recordValue(value);
  return typeof shown === 'string' ? shown : JSON.stringify(shown) ?? '—';
}
