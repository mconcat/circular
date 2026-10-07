import { declarationRow } from '@circular/core/internal';
import { sameValue } from '@circular/protocol';

const rowKey = row => declarationRow(row.kind)
  ?? (() => { throw new TypeError(`a commit delta row of kind ${row.kind} names no fold row`); })();

const plain = (_key, value) => typeof value === 'bigint' ? `${value}n`
  : value instanceof Uint8Array ? [...value] : value;
const unarmed = address => address && typeof address === 'object' && 'arm' in address && 'value' in address ? address.value : address;
const addressOf = row => unarmed(row[rowKey(row).field]);
const keyOf = row => `${rowKey(row).table}\0${JSON.stringify(addressOf(row), plain)}`;
const valueOf = row => ({ ...row, [rowKey(row).field]: addressOf(row) });

const path = scope => (scope ?? []).map(segment => segment.name ?? JSON.stringify(segment, plain)).map(name => `${name}/`).join('');
const endpoint = end => `${path(end.actor.scope)}${end.actor.local}.${end.port}`;
const word = table => ({ ExportMount: 'mount', Annotation: 'note' })[table] ?? table.toLowerCase();

/** One readable name for the declaration a row touches. */
export function rowLabel(row) {
  const { table } = rowKey(row);
  const address = addressOf(row);
  switch (table) {
    case 'Actor': return `actor ${path(address.scope)}${address.local}${row.declaration ? ` (${row.declaration.actorType})` : ''}`;
    case 'Edge': return `edge ${endpoint(address.from)} -> ${endpoint(address.to)}${address.ordinal ? ` #${address.ordinal}` : ''}`;
    case 'Scope': return `scope ${(Array.isArray(address) ? address : []).map(segment => segment.name ?? JSON.stringify(segment, plain)).join('/')}`;
    case 'Template': return `template ${address}`;
    default: return `${word(table)} ${address && typeof address === 'object' && 'local' in address ? `${path(address.scope)}${address.local}` : JSON.stringify(address, plain)}`;
  }
}

/**
 * The cut to read after, from an accepted root authoring snapshot, including the empty first cut.
 * A refusal or partial answer cannot classify a delta; the reason says why.
 */
export function cutOf(snapshot) {
  if (snapshot?.status === 'accepted') return { cut: snapshot.value };
  return { reason: unaccepted('before', snapshot) };
}

/** Why a snapshot answer is no cut: its status, and for a refusal the protocol's reason arm and first code. */
function unaccepted(at, snapshot) {
  const code = snapshot?.diagnostics?.[0]?.code ?? snapshot?.diagnostic?.code;
  return { at, code: snapshot?.status === 'rejected' ? snapshot.reason ?? 'rejected' : snapshot?.status ?? 'absent',
    ...(code !== undefined ? { diagnostic: Number(code) } : {}) };
}

/** A read that threw: the client error's own code. */
const thrown = (at, error) => ({ at, code: error?.code ?? error?.name ?? 'Error' });

/**
 * Opens the feed of the commits after one cut. It is called before the program that commits runs, on
 * the cut just read, so the feed opens at the tail. A failed open is kept as the reason, at `open`, for
 * a read that needs the feed.
 */
export async function openCommits(session, { cut, reason }) {
  if (!cut) return { reason };
  try { return { cut, feed: await session.authoringCommits([], cut.anchor.cursor) }; }
  catch (error) { return { cut, feed: null, reason: thrown('open', error) }; }
}

const changedNothing = (sent, after) => sent?.cursor === after + 1n
  && [...sent.revisions?.keys() ?? []].some(scope => scope.length === 0)
  && [...sent.revisions.values()].every(transition => sameValue(transition.authoringBefore, transition.authoringAfter));

/**
 * The commits after one cut, each as the rows that added, changed and retired something, read off the
 * feed `openCommits` opened before the commit. The feed is read up to the cursor the daemon answers
 * now, so a commit another author made in between is reported too, as its own commit. `sent` is the
 * metadata of the commit this caller sent, if it sent one: when that commit is the only one after the cut
 * and changed nothing, no frame is waited for. `complete` is false, with a reason, when that cursor was
 * not reached. The feed is released here.
 */
export async function commitsAfter(session, { cut, feed, reason }, { sent = null, waitMs = 5000 } = {}) {
  if (!cut) return { commits: [], complete: false, reason };
  if (feed === undefined) throw new TypeError('commitsAfter reads the feed openCommits opened before the commit');
  try {
    let now;
    try { now = await session.authoringSnapshot([], 256); }
    catch (error) { return { commits: [], complete: false, reason: thrown('after', error) }; }
    if (now.status !== 'accepted') return { commits: [], complete: false, reason: unaccepted('after', now) };
    const until = now.value.anchor.cursor, after = cut.anchor.cursor;
    if (until <= after) return { commits: [], complete: true };
    if (until === sent?.cursor && changedNothing(sent, after)) {
      return { commits: [{ cursor: sent.cursor, added: [], changed: [], retired: [] }], complete: true };
    }
    if (feed === null) return { commits: [], complete: false, reason: { ...reason, ...reach(after, until) } };
    const values = new Map(cut.commands.map(row => [keyOf(row), valueOf(row)]));
    const commits = [];
    let reached = after, ended = null;
    try {
      await feed.grant(until - after);
      while (reached < until) {
        const frame = await feed.next(waitMs);
        if (frame?.kind !== 'Commit') {
          ended = frame === null ? { code: 'FRAME_TIMED_OUT' } : { code: frame.reason?.kind, diagnostic: frame.diagnostic?.code };
          break;
        }
        const commit = { cursor: frame.value.metadata.cursor, added: [], changed: [], retired: [] };
        for (const row of frame.value.delta) {
          const key = keyOf(row);
          if (row.kind.startsWith('Retire')) {
            if (values.delete(key)) commit.retired.push(row);
          } else {
            const before = values.get(key), value = valueOf(row);
            if (before === undefined) commit.added.push(row);
            else if (!sameValue(before, value)) commit.changed.push(row);
            values.set(key, value);
          }
        }
        commits.push(commit);
        reached = commit.cursor;
      }
    } catch (error) { ended = thrown('feed', error); }
    return reached >= until ? { commits, complete: true }
      : { commits, complete: false, reason: { at: 'feed', ...ended, ...reach(reached, until) } };
  } finally { feed?.release(); }
}

const reach = (cursor, until) => ({ cursor: String(cursor), until: String(until) });

/** The reason as one line: its code, where the read stopped, and how far it got. */
const reasonLine = reason => `${reason.code}${reason.diagnostic !== undefined ? ` (code ${reason.diagnostic})` : ''} at ${reason.at}`
  + (reason.until !== undefined ? `, cursor ${reason.cursor} of ${reason.until}` : '');

/** The rows as labels; presentation rows are counted, not listed. */
export function deltaSummary({ commits, complete, reason }) {
  const summary = { added: [], changed: [], retired: [], presentation: 0, commits: commits.length, complete };
  if (!complete) summary.reason = reason;
  for (const commit of commits) {
    for (const side of ['added', 'changed', 'retired']) {
      for (const row of commit[side]) {
        if (row.kind === 'SetPresentation') summary.presentation += 1;
        else summary[side].push(rowLabel(row));
      }
    }
  }
  return summary;
}

/** Lines for a person: per commit a count, then one line per declaration, `+` added, `~` changed, `-` retired. */
export function deltaLines(result) {
  const lines = [];
  for (const commit of result.commits) {
    const summary = deltaSummary({ commits: [commit], complete: true });
    lines.push(`${result.commits.length > 1 ? `commit ${commit.cursor}: ` : ''}${summary.added.length} added, ${summary.changed.length} changed, ${summary.retired.length} retired`
      + (summary.presentation ? ` (and ${summary.presentation} presentation rows)` : ''),
    ...summary.added.map(label => `  + ${label}`),
    ...summary.changed.map(label => `  ~ ${label} (an existing declaration was replaced)`),
    ...summary.retired.map(label => `  - ${label}`));
  }
  if (!result.complete) lines.push(`what the commit changed is not fully known: ${reasonLine(result.reason)}`);
  return lines;
}
