import fs from 'node:fs';
import { CEILINGS, SOCKET_NAME, socketPath } from './common.mjs';

export function socketPresent(state) {
  try { fs.lstatSync(socketPath(state)); return true; }
  catch (error) { if (error.code === 'ENOENT') return false; throw error; }
}

export async function connect(state) {
  const { connectOwnerLocal } = await import('@circular/client/owner-local');
  return connectOwnerLocal({ root: state, socketName: SOCKET_NAME });
}

export async function withSession(state, work, { requestTimeoutMs = 5000, roles = [1n], transport } = {}) {
  const { establish } = await import('@circular/client');
  transport ??= await connect(state);
  let session;
  try {
    session = await establish(transport, { hello: { requestedRoles: roles }, resourceCeilings: CEILINGS, requestTimeoutMs });
    return await work(session);
  } finally {
    if (session) await session.close().catch(() => {});
    else await transport.close();
  }
}

export async function query(session, name, args = null) {
  const answer = await session.exchange('Query', 'Query', { name, args });
  if (answer.kind?.verb !== 'QueryResult') throw new Error(`${name} returned ${answer.kind?.verb ?? 'no result'}`);
  const body = answer.payload;
  if (body?.[0] === 2n) throw new Error(`${name} refused: ${body[1]?.message ?? 'no diagnostic'}`);
  if (body?.[0] !== 1n) throw new Error(`${name} returned an unrecognised result discriminant`);
  return body[1];
}

export function plain(value) {
  if (typeof value === 'bigint') return value.toString();
  if (value instanceof Uint8Array) return Buffer.from(value).toString('hex');
  if (Array.isArray(value)) return value.map(plain);
  if (value && typeof value === 'object') {
    if (typeof value.value === 'bigint' && Object.keys(value).length === 1) return value.value.toString();
    return Object.fromEntries(Object.entries(value).map(([key, inner]) => [key, plain(inner)]));
  }
  return value;
}
