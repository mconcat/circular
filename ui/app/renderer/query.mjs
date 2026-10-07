import { inSpace } from './reasons.mjs';
import { valueKey as identity } from '@circular/protocol';
import { actorIdentityFromValue } from '@circular/protocol/establishment';
export { identity };
const spelled = new WeakMap();
export function actorKey(actor) {
  if (actor === null || typeof actor !== 'object') return identity(actorIdentityFromValue(actor));
  let id = spelled.get(actor);
  if (id === undefined) spelled.set(actor, id = identity(actorIdentityFromValue(actor)));
  return id;
}
export const journalRows = 100;
export const domId = value => 'a' + Array.from(new TextEncoder().encode(value), b => b.toString(16).padStart(2,'0')).join('');
export const fault = (code, message = String(code?.code ?? code)) => Object.assign(new Error(message), { code });
export function queryPage(answer, complete = true) {
  if (answer?.kind?.verb !== 'QueryResult') throw fault('QUERY_RESULT_REQUIRED');
  if (answer.payload?.[0] === 2n) throw fault(inSpace('Query', answer.payload[1]?.code) ?? 'QUERY_REJECTED');
  if (answer.payload?.[0] !== 1n || !Array.isArray(answer.payload[1]?.items)) throw fault('QUERY_PAGE_INVALID');
  const page = answer.payload[1];
  if (complete && page.terminal !== 2n) throw fault(
    Array.isArray(page.terminal) && page.terminal[0] === 3n ? inSpace('Query', page.terminal[1]) : 'QUERY_INCOMPLETE',
    'complete observation page required');
  return page;
}
const lensField = lens => lens === undefined ? {} : {lens:BigInt(lens.correlation)};
export async function readFirstPage(session, name, args = null, limit, since, lens) {
  const stream = session.hold(name);
  try {
    await stream.send('Query', 'Query', {name, args,
      ...(since === undefined ? {} : {since}),
      ...lensField(lens),
      ...(limit === undefined ? {} : {page:{limit:BigInt(limit)}})});
    const page = queryPage(await stream.next(), false);
    if (Array.isArray(page.terminal) && page.terminal[0] === 3n) throw Object.assign(fault(inSpace('Query', page.terminal[1])), { page });
    return page;
  } finally { stream.release(); }
}
export async function readCompleteAnswer(session, name, args = null, limit, lens, since) {
  const stream = session.hold(name);
  try {
    const items = [];
    let first, cursor;
    for (;;) {
      await stream.send('Query', 'Query', {name, args, ...(since === undefined ? {} : {since}), ...lensField(lens),
        page:{limit:BigInt(limit), ...(cursor === undefined ? {} : {cursor})}});
      const page = queryPage(await stream.next(), false);
      first ??= page;
      if (identity(page.anchor) !== identity(first.anchor)) throw fault('QUERY_PAGE_INVALID');
      items.push(...page.items);
      if (page.terminal === 2n) return { ...first, items, terminal: 2n };
      if (Array.isArray(page.terminal) && page.terminal[0] === 3n)
        throw Object.assign(fault(inSpace('Query', page.terminal[1])), { page: { ...first, items, terminal: page.terminal } });
      if (!Array.isArray(page.terminal) || page.terminal[0] !== 1n || page.terminal[1] == null) throw fault('QUERY_PAGE_INVALID');
      cursor = page.terminal[1];
    }
  } finally { stream.release(); }
}

export function latestWindow(cut, rows = journalRows, perActor = 0) {
  if (!Array.isArray(cut) || cut.length === 0 || !(rows > 0)) return null;
  const depth = BigInt(Math.max(perActor, Math.ceil(rows / cut.length)));
  return {
    depth,
    limit: Number(depth) * cut.length,
    since: cut.map(component => ({ actor: component.actor,
      index: component.index > depth ? component.index - depth : 0n })),
  };
}
