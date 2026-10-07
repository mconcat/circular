import { exactPayloadPathIssue } from '../../protocol/src/payload-path.js';
export function assembleConfigIssue(config) {
  const keys = ['at', 'inactivity_timeout', 'max_window', 'capacity'];
  const failure = 'assemble requires at, positive capacity and positive inactivity_timeout <= max_window';
  if (!config || typeof config !== 'object' || Array.isArray(config)
    || Object.keys(config).length !== keys.length || keys.some(key => !Object.hasOwn(config, key))) return failure;
  if (exactPayloadPathIssue(config.at)) return failure;
  for (const key of keys.slice(1)) {
    if (typeof config[key] !== 'bigint' || config[key] <= 0n || config[key] > 0x7fffffffffffffffn) return failure;
  }
  return config.inactivity_timeout <= config.max_window ? null : failure;
}
