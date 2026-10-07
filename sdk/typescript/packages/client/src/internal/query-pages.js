import { logCutFromValue } from '../../../protocol/src/replay-values.js';
import { sameValue } from '@circular/protocol';
import { recordRegistrations, recordQueryPageFromValue } from '../../../protocol/src/internal/record-values.js';

export async function completeQuery(session, name, args, pageLimit, timeoutMs = 5000, since, lens) {
  if (!Number.isSafeInteger(pageLimit) || pageLimit <= 0) throw new Error('query page limit must be positive');
  const registration=Object.hasOwn(recordRegistrations,name) ? recordRegistrations[name] : undefined;
  if (registration) registration.args(args);
  if (since !== undefined) logCutFromValue(since);
  const stream = session.hold(name);
  const items = [];
  const issuedCursors = [];
  let anchor, cursor, cut, foldedFrom, first = true;
  try {
    for (;;) {
      const page = { limit: BigInt(pageLimit) };
      if (cursor !== undefined) page.cursor = cursor;
      const request = { name, args, page };
      if (first && since !== undefined) request.since = since;
      if (lens !== undefined) request.lens = BigInt(lens.correlation);
      await stream.send('Query', 'Query', request);
      const answer = await stream.next(timeoutMs);
      if (answer?.kind?.verb !== 'QueryResult') throw new Error(`${name}: QueryResult required`);
      const body = answer.payload;
      if (Array.isArray(body) && body.length === 2 && body[0] === 2n) {
        return answer;
      }
      if (!Array.isArray(body) || body.length !== 2 || body[0] !== 1n || !Array.isArray(body[1]?.items)) {
        throw new Error(`${name}: malformed query page`);
      }
      const value = body[1];
      if (registration) recordQueryPageFromValue(value,registration.item);
      if (first) { anchor = value.anchor; cut = value.cut; foldedFrom = value.folded_from; first = false; }
      else if (!sameValue(cut, value.cut) || !sameValue(foldedFrom, value.folded_from)) throw new Error(`${name}: query cut changed`);
      else if (!sameValue(anchor, value.anchor)) throw new Error(`${name}: query anchor changed`);
      items.push(...value.items);
      if (value.terminal === 2n) return { ...answer, payload: [1n, { ...value, anchor, items }] };
      const terminal = value.terminal;
      if (!Array.isArray(terminal) || terminal.length !== 2 || terminal[0] !== 1n
        || terminal[1] === undefined || terminal[1] === null
        || issuedCursors.some(prior => sameValue(prior, terminal[1])) || value.items.length === 0) {
        throw new Error(`${name}: complete query required; invalid or diagnostic terminal`);
      }
      cursor = terminal[1];
      issuedCursors.push(cursor);
    }
  } finally { stream.release(); }
}
