/** Pause/ForcePause control one pipeline per state directory.
 * Request codecs are internal; `lifecycleResultFromValue` is public.
 */
import { decodeValueBeta, encodeValueBeta, CircularUInt } from './value.js';
import { rejectionFromValue } from './internal/result-values.js';

const fail = message => { throw new TypeError(message); };
const object = value => value !== null && typeof value === 'object'
  && !Array.isArray(value) && !(value instanceof Uint8Array) && !(value instanceof CircularUInt);
function fields(value, required, optional = []) {
  if (!object(value)) fail('payload is not an object');
  for (const key of required) if (!Object.hasOwn(value, key)) fail(`payload lacks ${key}`);
  for (const key of Object.keys(value)) if (!required.includes(key) && !optional.includes(key)) fail(`unknown field ${key}`);
}
function resumeValue(request) {
  fields(request, ['expectedAuthoringRevision']);
  const revision = request.expectedAuthoringRevision;
  if (!(revision instanceof Uint8Array) || revision.length !== 32) fail('InvalidAuthoringRevision');
  return { expected_authoring_revision: revision };
}
export function encodeResume(request, ceilings) {
  return encodeValueBeta(resumeValue(request), ceilings);
}
/** Owner-local exchange also accepts canonical wire Values, as before. */
export function lifecyclePayloadValue(verb, request) {
  if (verb === 'Resume' && object(request) && Object.hasOwn(request, 'expectedAuthoringRevision')) {
    return resumeValue(request);
  }
  if (verb === 'Pause' && object(request) && typeof request.mode === 'string') {
    return pauseValue(request);
  }
  return request;
}
function pauseValue(request) {
  fields(request, [], ['mode']);
  const value = {};
  if (request.mode !== undefined) {
    if (request.mode !== 'Pause' && request.mode !== 'ForcePause') fail('MalformedValue');
    value.mode = request.mode === 'Pause' ? 1n : 2n;
  }
  return value;
}
export function encodePause(request, ceilings) {
  return encodeValueBeta(pauseValue(request), ceilings);
}
export function decodePause(bytes, ceilings) {
  const value = decodeValueBeta(bytes, ceilings);
  fields(value, [], ['mode']);
  if (!Object.hasOwn(value, 'mode')) return {};
  if (value.mode !== 1n && value.mode !== 2n) fail('MalformedValue');
  return { mode: value.mode === 1n ? 'Pause' : 'ForcePause' };
}

/** Accepted lifecycle unit arms: [1, 1] is Resumed and [1, 2] is Paused.
 * A state directory serves one pipeline; replies carry no run number.
 */
export function lifecycleResultFromValue(value) {
  if (!Array.isArray(value) || value.length !== 2) fail('InvalidResult');
  if (value[0] === 2n) return rejectionFromValue(value[1]);
  if (value[0] !== 1n) fail('InvalidResult');
  const accepted = value[1];
  if (accepted !== 1n && accepted !== 2n) fail('InvalidResult');
  return Object.freeze({ status: 'accepted',
    value: Object.freeze({ kind: accepted === 1n ? 'Resumed' : 'Paused' }) });
}
