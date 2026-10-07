import { CircularUInt } from './value.js';
import { actorIdentityValue } from './establishment.js';
import { replayTargetFromCheckpoint } from './replay-values.js';
import { TimelineMarkKind } from './internal/closed-tables.js';

/** The registered query names. */
export const TIMELINE_BINS_QUERY = 'timeline.bins';
export const TIMELINE_AT_QUERY = 'timeline.at';
/** The largest bin count one answer carries. */
export const TIMELINE_MAX_BINS = 4096n;
/** `TimelineMarkKind` spellings in declaration order, from `@circular/protocol/tables`. */
export const TIMELINE_MARK_KINDS = TimelineMarkKind;

const UINT_MAX = 0xffff_ffff_ffff_ffffn;

function fail(place) { throw new TypeError(`invalid timeline value: ${place}`); }
function record(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    && !(value instanceof Uint8Array) && !(value instanceof CircularUInt);
}
function fields(value, required, optional, place) {
  if (!record(value) || required.some(key => !Object.hasOwn(value, key))
    || Object.keys(value).some(key => !required.includes(key) && !optional.includes(key))) fail(place);
}
/** An argument instant or count: a BigInt or the UInt wrapper, 0..2^64-1. */
function argument(value, place) {
  const carried = value instanceof CircularUInt ? value.value : value;
  if (typeof carried !== 'bigint' || carried < 0n || carried > UINT_MAX) fail(place);
  return carried;
}
/** An answered instant or count: the UInt kind only. */
function uint(value, place) {
  if (!(value instanceof CircularUInt)) fail(place);
  return value.value;
}

/**
 * Rust `TimelineBinsArgs::coverage`: the bin width and the end the bins cover. A range that does not
 * divide rounds the bin up, so bin `i` is `[from_ms + i·bin_ms, from_ms + (i+1)·bin_ms)` and the
 * answer reports the covered end as its `to_ms`.
 */
export function timelineCoverage({ from_ms, to_ms, bins }) {
  const from = argument(from_ms, 'from_ms');
  const to = argument(to_ms, 'to_ms');
  const count = argument(bins, 'bins');
  if (from >= to) fail('from_ms must precede to_ms');
  if (count < 1n || count > TIMELINE_MAX_BINS) fail(`bins must be in 1..=${TIMELINE_MAX_BINS}`);
  const binMs = (to - from + count - 1n) / count;
  const covered = from + binMs * count;
  if (covered > UINT_MAX) fail('the binned range exceeds the UInt width');
  return { bin_ms: binMs, to_ms: covered };
}

/** `timeline.bins` arguments `{from_ms, to_ms, bins, actor?}`; refuses what the daemon refuses. */
export function timelineBinsArgsValue(args) {
  fields(args, ['from_ms', 'to_ms', 'bins'], ['actor'], 'timeline.bins arguments');
  timelineCoverage(args);
  const value = {
    bins: new CircularUInt(argument(args.bins, 'bins')),
    from_ms: new CircularUInt(argument(args.from_ms, 'from_ms')),
    to_ms: new CircularUInt(argument(args.to_ms, 'to_ms')),
  };
  if (args.actor !== undefined) value.actor = actorIdentityValue(args.actor);
  return value;
}

/** `timeline.at` arguments `{at_ms}`. */
export function timelineAtArgsValue(args) {
  fields(args, ['at_ms'], [], 'timeline.at arguments');
  return { at_ms: new CircularUInt(argument(args.at_ms, 'at_ms')) };
}

/**
 * Reads a `timeline.bins` answer (the page anchor). The bins cover `[from_ms, to_ms)` exactly, and
 * every mark is one of the closed four inside that range — one per kind per bin.
 */
export function timelineBinsFromValue(value) {
  fields(value, ['from_ms', 'to_ms', 'bin_ms', 'bins', 'marks', 'clock_regressions'], [], 'timeline.bins');
  const from = uint(value.from_ms, 'from_ms');
  const to = uint(value.to_ms, 'to_ms');
  const binMs = uint(value.bin_ms, 'bin_ms');
  if (!Array.isArray(value.bins) || value.bins.length === 0) fail('bins');
  if (binMs === 0n || from + binMs * BigInt(value.bins.length) !== to) fail('bins cover [from_ms, to_ms)');
  const bins = value.bins.map(bin => {
    fields(bin, ['count', 'incidents'], [], 'bin');
    return Object.freeze({ count: uint(bin.count, 'count'), incidents: uint(bin.incidents, 'incidents') });
  });
  if (!Array.isArray(value.marks)) fail('marks');
  const seen = new Set();
  const marks = value.marks.map(mark => {
    fields(mark, ['kind', 'at_ms'], [], 'mark');
    if (!TIMELINE_MARK_KINDS.includes(mark.kind)) fail('mark kind');
    const at = uint(mark.at_ms, 'mark at_ms');
    if (at < from || at >= to) fail('mark outside the range');
    const key = `${(at - from) / binMs}:${mark.kind}`;
    if (seen.has(key)) fail('two marks of one kind in one bin');
    seen.add(key);
    return Object.freeze({ kind: mark.kind, at_ms: at });
  });
  return Object.freeze({
    from_ms: from,
    to_ms: to,
    bin_ms: binMs,
    bins: Object.freeze(bins),
    marks: Object.freeze(marks),
    clock_regressions: uint(value.clock_regressions, 'clock_regressions'),
  });
}

/**
 * Reads a `timeline.at` answer (the page anchor). `target` is the coordinate `{stream, revision_epoch,
 * cut}` exactly as the wire carries it — the spelling a `timeline` checkpoint has, so the client's
 * `replay.start({from})` · the lens's `rewind({to})` · Step pace read it through `replayTargetFromCheckpoint`
 * unchanged. One coordinate spelling; nothing is inferred.
 */
export function timelineAtFromValue(value) {
  fields(value, ['requested_ms', 'resolved_ms', 'stream', 'revision_epoch', 'cut'], [], 'timeline.at');
  const requested = uint(value.requested_ms, 'requested_ms');
  const resolved = uint(value.resolved_ms, 'resolved_ms');
  if (resolved > requested) fail('the cut stands after the requested instant');
  const target = Object.freeze({ stream: value.stream, revision_epoch: value.revision_epoch, cut: value.cut });
  replayTargetFromCheckpoint(target);
  return Object.freeze({ requested_ms: requested, resolved_ms: resolved, target });
}
