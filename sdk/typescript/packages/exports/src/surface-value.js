export function surfaceValue(value) {
  if (value === null || typeof value !== 'object' && typeof value !== 'function') return value;
  if (typeof value.toJSON === 'function') return surfaceValue(value.toJSON());
  if (Array.isArray(value)) return Object.freeze(value.map(surfaceValue));
  return Object.freeze(Object.fromEntries(Object.entries(value).filter(([, v]) => v !== undefined).map(([k, v]) => [k, surfaceValue(v)])));
}
