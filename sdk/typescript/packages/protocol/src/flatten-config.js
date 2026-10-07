import { exactPayloadPathIssue } from './payload-path.js';
export function flattenConfigIssue(config) {
  if (!config || typeof config !== 'object' || Array.isArray(config) || config instanceof Uint8Array) return 'flatten config must be an object';
  if (Object.keys(config).length !== 1) return 'flatten requires only at';
  if (!Object.hasOwn(config, 'at')) return 'flatten requires at';
  const issue = exactPayloadPathIssue(config.at);
  if (issue) return `flatten at: ${issue}`;
  return config.at.length ? null : 'flatten at must be a nonempty path';
}
