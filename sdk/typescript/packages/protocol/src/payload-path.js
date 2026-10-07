export function exactPayloadPathIssue(value) {
  if (!Array.isArray(value)) return 'path must be an array';
  for (const [at, segment] of value.entries()) {
    if (typeof segment === 'string') continue;
    if (typeof segment === 'bigint' && segment >= 0n && segment <= 0x7fffffffffffffffn) continue;
    return `path segment ${at} must be a String key or nonnegative Int index`;
  }
  return null;
}
