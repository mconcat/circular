import { decodeValueBeta, encodeValueBeta } from './value.js';
import { EstablishmentValueError, SESSION_ROLE_ARMS, scopeIdentityFromValue } from './establishment.js';
import { PARTITION_TAGS } from './wire.js';

const fail = (code, message) => { throw new EstablishmentValueError(code, message); };
const object = value => value !== null && typeof value === 'object'
  && !Array.isArray(value) && !(value instanceof Uint8Array);
const int = (value, maximum, key) => {
  if (typeof value !== 'bigint' || value < 0n || value > maximum) fail('HELLO_WIDTH', `${key} has the wrong Int width`);
  return value;
};

/** Token bytes are never interpolated into diagnostics. No issuer or comparison policy lives here. */
export function sessionTokenFromValue(value) {
  if (!(value instanceof Uint8Array)) fail('TOKEN_CARRIER', 'a session token is Bytes');
  if (value.length !== 32) fail('TOKEN_WIDTH', 'a session token is exactly 32 bytes');
  return Uint8Array.from(value);
}

function roleFromValue(value) {
  const sequence = Array.isArray(value);
  const tag = sequence ? value[0] : value;
  const args = sequence ? value.slice(1) : [];
  if (typeof tag !== 'bigint' || (sequence && value.length < 2)) fail('ROLE_SHAPE', 'invalid session role carrier');
  const entry = Object.entries(SESSION_ROLE_ARMS).find(([, arm]) => BigInt(arm.tag) === tag);
  if (!entry) fail('ROLE_UNKNOWN', 'unassigned session role arm');
  const [kind, arm] = entry;
  if (args.length !== (arm.argument === null ? 0 : 1)) fail('ROLE_SHAPE', 'wrong session role arity');
  if (arm.argument === null) return { kind };
  const scope = args[0];
  if (!Array.isArray(scope)) fail('ROLE_SHAPE', 'scope must be a segment sequence');
  for (const segment of scope) {
    if (!Array.isArray(segment) || typeof segment[1] !== 'string'
      || !(segment[0] === 1n && segment.length === 2
        || segment[0] === 2n && segment.length === 3
          && ['string', 'bigint', 'boolean'].includes(typeof segment[2]))) {
      fail('ROLE_SHAPE', 'invalid scope segment');
    }
  }
  return { kind, scope: scopeIdentityFromValue(scope) };
}

function compareBytes(a, b) {
  for (let i = 0; i < Math.min(a.length, b.length); ++i) if (a[i] !== b[i]) return a[i] - b[i];
  return a.length - b.length;
}

/** Decode a Hello; admission and policy remain the daemon’s job. */
export function decodeHello(bytes, resourceCeilings) {
  const value = decodeValueBeta(bytes, resourceCeilings);
  if (!object(value)) fail('HELLO_SHAPE', 'Hello must be an object');
  const allowed = new Set(['features', 'protocol_version', 'requested_roles']);
  for (const key of Object.keys(value)) if (!allowed.has(key)) fail('HELLO_SHAPE', 'unexpected Hello member');
  for (const key of ['features', 'protocol_version', 'requested_roles']) {
    if (!Object.hasOwn(value, key)) fail('HELLO_SHAPE', `Hello requires ${key}`);
  }
  const protocolVersion = int(value.protocol_version, 65535n, 'protocol_version');
  if (!object(value.features)) fail('HELLO_SHAPE', 'features must be an object');
  const features = new Map();
  for (const key of Object.keys(value.features)) {
    if (!Object.hasOwn(PARTITION_TAGS, key)) fail('HELLO_SHAPE', 'unknown feature partition');
  }
  for (const key of Object.keys(PARTITION_TAGS)) {
    if (Object.hasOwn(value.features, key)) features.set(key, int(value.features[key], 255n, 'features'));
  }
  if (!Array.isArray(value.requested_roles)) fail('ROLE_SHAPE', 'requested_roles must be a sequence');
  let previous;
  const requestedRoles = value.requested_roles.map(role => {
    const encoded = encodeValueBeta(role, resourceCeilings);
    if (previous && compareBytes(previous, encoded) >= 0) fail('ROLE_ORDER', 'role set is duplicated or not in canonical byte order');
    previous = encoded;
    return roleFromValue(role);
  });
  return { protocolVersion, features, requestedRoles };
}
