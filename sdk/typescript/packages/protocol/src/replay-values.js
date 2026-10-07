
import { canonicalValueSequence, actorIdentityFromValue, actorIdentityValue } from "./establishment.js";
import { reducedRatioValue } from "./declaration-values.js";
import { CircularUInt, encodeValueBeta, uint } from "./value.js";

export class ReplayValueError extends Error {
  constructor(code, message) {
    super(`${code}: ${message}`);
    this.code = code;
  }
}

function fail(code, message) {
  throw new ReplayValueError(code, message);
}

/** `Arrangement = Observational | Counterfactual | Divergent | DryRun` — a closed four. */
export const ARRANGEMENT_ARMS = Object.freeze({ Observational: 1, Counterfactual: 2, Divergent: 3, DryRun: 4 });

/**
 * `Tail = Live | DryRun` — whether the branch actually goes out.
 *
 * **This is how the sixth combination is kept from existing.** Because the value is `Divergent`'s
 * argument rather than a name of its own, there are four constructors and five named
 * combinations. A fifth constructor would have to say what a tail means for the three arms that
 * have no branch point.
 */
export const TAIL_ARMS = Object.freeze({ Live: 1, DryRun: 2 });

/** `Pace = Free | Paused | Step(ReplayTarget) | Realtime(ratio)` — a closed four. */
export const PACE_ARMS = Object.freeze({ Free: 1, Paused: 2, Step: 3, Realtime: 4 });

function readUnsigned(value, key) {
  const number = typeof value === "bigint" ? value : BigInt(value);
  if (number < 0n) fail("OUT_OF_RANGE", `${key} is unsigned`);
  return number;
}

/**
 * A log cut: **a vector over actors, not a scalar**.
 *
 * A scalar target would need a mapping from that one number to a per-actor cursor, **and that
 * mapping is a global clock** — the thing the whole execution model is built to not have. So the
 * carrier is one prefix length per actor, and a cut that names no actor at all is still a cut.
 *
 * Two rules govern the sequence and **they are not the same rule**, which is why the generic
 * `canonicalValueSequence` is not what stands here:
 *
 *   - **Order** is over the *whole element's* encoded bytes, because the map is
 *     being carried as a column and the elements are what is being ordered.
 *   - **Identity** is over the *actor key's* bytes alone, because the column is a map and a map's
 *     keys are unique. A cut naming one actor twice with two prefix lengths claims two positions
 *     for one cursor, and that is not a value.
 *
 * Deduplicating by the whole element would admit exactly that cut: `{map: 1, map: 2}` has two
 * distinct elements and one repeated key. The far end refuses it with `NotCanonical`, so a
 * builder that used the generic helper would encode happily and be rejected on arrival — the
 * shape of failure this track has already paid for twice.
 */
export function logCutValue(components, ceilings, key = "cut") {
  if (!Array.isArray(components)) fail("CUT_NOT_ARRAY", "a log cut is a sequence of per-actor components");
  const elements = components.map((component) => {
    if (component === null || typeof component !== "object") fail("CUT_COMPONENT_SHAPE", "a cut component is an object");
    for (const name of Object.keys(component)) {
      if (name !== "index" && name !== "actor") {
        fail("CUT_COMPONENT_UNEXPECTED", `a cut component carries no \`${name}\``);
      }
    }
    return { index: readUnsigned(component.index, "index"), actor: actorIdentityValue(component.actor) };
  });
  const keys = elements.map((element) => encodeValueBeta(element.actor, ceilings));
  for (let outer = 0; outer < keys.length; outer += 1) {
    for (let inner = outer + 1; inner < keys.length; inner += 1) {
      if (keys[outer].length === keys[inner].length && keys[outer].every((byte, at) => byte === keys[inner][at])) {
        fail("CUT_ACTOR_TWICE", `${key} names one actor twice, which claims two positions for one cursor`);
      }
    }
  }
  return canonicalValueSequence(elements, ceilings);
}

/** Reads a log cut back into the components it was built from. */
export function logCutFromValue(value) {
  if (!Array.isArray(value)) fail("CUT_NOT_ARRAY", "a log cut is a sequence of per-actor components");
  return value.map((element) => {
    if (element === null || typeof element !== "object" || Array.isArray(element)) {
      fail("CUT_COMPONENT_SHAPE", "a cut component is an object");
    }
    if (typeof element.index !== "bigint") fail("CUT_INDEX_CARRIER", "a prefix length is carried as an Int");
    return { index: element.index, actor: actorIdentityFromValue(element.actor) };
  });
}

/** One tail, always a bare tag. */
export function tailValue(tail) {
  const tag = TAIL_ARMS[tail];
  if (tag === undefined) fail("TAIL_UNKNOWN", `${String(tail)} is not a declared tail`);
  return BigInt(tag);
}

export function arrangementValue(arrangement, ceilings) {
  if (arrangement === "DryRun") return BigInt(ARRANGEMENT_ARMS.DryRun);
  if (arrangement === null || typeof arrangement !== "object") {
    fail("ARRANGEMENT_SHAPE", "an arrangement is `DryRun` or an arm object");
  }
  const tag = ARRANGEMENT_ARMS[arrangement.kind];
  if (tag === undefined) fail("ARRANGEMENT_ARM_UNKNOWN", `${String(arrangement.kind)} is not an arrangement arm`);
  if (arrangement.kind === "DryRun") {
    fail("ARRANGEMENT_DRY_RUN_ARGUMENT", "DryRun is the tag itself; the absence of an origin is what the arm is");
  }
  if (arrangement.kind !== "Divergent") {
    for (const name of Object.keys(arrangement)) {
      if (name !== "kind" && name !== "from") fail("ARRANGEMENT_UNEXPECTED", `${arrangement.kind} carries no \`${name}\``);
    }
    return [BigInt(tag), replayTargetValue(arrangement.from, ceilings, "from")];
  }
  for (const name of Object.keys(arrangement)) {
    if (name !== "kind" && name !== "at" && name !== "tail") {
      fail("ARRANGEMENT_UNEXPECTED", `Divergent carries no \`${name}\``);
    }
  }
  return [BigInt(tag), { at: replayTargetValue(arrangement.at, ceilings, "at"), tail: tailValue(arrangement.tail) }];
}

const nameFor = (table, tag) => Object.keys(table).find((name) => BigInt(table[name]) === tag);

/** Reads an arrangement back into the shape it was built from. */
export function arrangementFromValue(value) {
  if (typeof value === "bigint") {
    const name = nameFor(ARRANGEMENT_ARMS, value);
    if (name === undefined) fail("ARRANGEMENT_ARM_UNKNOWN", `arrangement tag ${value} is unassigned`);
    if (name !== "DryRun") fail("ARRANGEMENT_ARGUMENT_MISSING", `${name} carries an argument`);
    return "DryRun";
  }
  if (!Array.isArray(value) || value.length !== 2) fail("ARRANGEMENT_SHAPE", "an arrangement arm is a two-part sequence");
  const name = nameFor(ARRANGEMENT_ARMS, value[0]);
  if (name === undefined) fail("ARRANGEMENT_ARM_UNKNOWN", `arrangement tag ${value[0]} is unassigned`);
  if (name === "DryRun") fail("ARRANGEMENT_DRY_RUN_ARGUMENT", "DryRun is the tag itself");
  if (name !== "Divergent") return { kind: name, from: replayTargetFromValue(value[1], "from") };
  const argument = value[1];
  if (argument === null || typeof argument !== "object" || Array.isArray(argument)) {
    fail("ARRANGEMENT_SHAPE", "Divergent carries an object of two");
  }
  for (const key of Object.keys(argument)) {
    if (key !== "at" && key !== "tail") fail("ARRANGEMENT_UNEXPECTED", `Divergent carries no \`${key}\``);
  }
  const tail = nameFor(TAIL_ARMS, argument.tail);
  if (tail === undefined) fail("TAIL_UNKNOWN", `tail tag ${argument.tail} is unassigned`);
  return { kind: "Divergent", at: replayTargetFromValue(argument.at, "at"), tail };
}

/**
 * One pace.
 *
 * **The multiplier is a rational, and its carrier is the edge delay's** — one implementation, not
 * two agreeing comments. An integer-only multiplier cannot express playback slower than the
 * original, and `1/2` is exactly that. Irreducibility is the canonical form for the same reason
 * it is there: a reducible pair would give one speed two byte strings.
 */
export function paceValue(pace, ceilings) {
  if (typeof pace === "string") {
    const tag = PACE_ARMS[pace];
    if (tag === undefined) fail("PACE_UNKNOWN", `${pace} is not a declared pace`);
    if (pace !== "Free" && pace !== "Paused") fail("PACE_ARGUMENT_MISSING", `${pace} carries an argument`);
    return BigInt(tag);
  }
  if (pace === null || typeof pace !== "object") fail("PACE_SHAPE", "a pace is a name or an arm object");
  if (pace.kind === "Step") {
    return [BigInt(PACE_ARMS.Step), replayTargetValue(pace.upto, ceilings, "upto")];
  }
  if (pace.kind === "Realtime") {
    return [BigInt(PACE_ARMS.Realtime), reducedRatioValue({ num: pace.num, den: pace.den }, "multiplier")];
  }
  return fail("PACE_ARM_UNKNOWN", `${String(pace.kind)} is not a pace arm`);
}

/** Reads a pace back into the shape it was built from. */
export function paceFromValue(value) {
  if (typeof value === "bigint") {
    const name = nameFor(PACE_ARMS, value);
    if (name === undefined) fail("PACE_ARM_UNKNOWN", `pace tag ${value} is unassigned`);
    if (name !== "Free" && name !== "Paused") fail("PACE_ARGUMENT_MISSING", `${name} carries an argument`);
    return name;
  }
  if (!Array.isArray(value) || value.length !== 2) fail("PACE_SHAPE", "a pace arm is a two-part sequence");
  const name = nameFor(PACE_ARMS, value[0]);
  if (name === undefined) fail("PACE_ARM_UNKNOWN", `pace tag ${value[0]} is unassigned`);
  if (name === "Step") return { kind: "Step", upto: replayTargetFromValue(value[1], "upto") };
  if (name === "Realtime") {
    const ratio = value[1];
    if (ratio === null || typeof ratio !== "object" || Array.isArray(ratio)) fail("PACE_SHAPE", "a multiplier is a ratio");
    return { kind: "Realtime", num: ratio.num, den: ratio.den };
  }
  return fail("PACE_ARGUMENT_UNEXPECTED", `${name} carries no argument`);
}

/** `ReplayStart { arrangement, pace }` — two places, and there is no third. */
export function replayStartValue(start, ceilings) {
  if (start === null || typeof start !== "object") fail("START_SHAPE", "a ReplayStart is an object");
  for (const name of Object.keys(start)) {
    if (name !== "arrangement" && name !== "pace") {
      fail("START_UNEXPECTED", `a ReplayStart carries no \`${name}\`; the origin lives inside the arrangement`);
    }
  }
  return { arrangement: arrangementValue(start.arrangement, ceilings), pace: paceValue(start.pace, ceilings) };
}

/** Reads a start back into the shape it was built from. */
export function replayStartFromValue(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) fail("START_SHAPE", "a ReplayStart is an object");
  for (const name of Object.keys(value)) {
    if (name !== "arrangement" && name !== "pace") fail("START_UNEXPECTED", `a ReplayStart carries no \`${name}\``);
  }
  return { arrangement: arrangementFromValue(value.arrangement), pace: paceFromValue(value.pace) };
}

export function replayTargetValue(target, ceilings, key = "to") {
  if (target === null || typeof target !== "object" || Array.isArray(target)) {
    fail("REWIND_TARGET_SHAPE", `${key} is an object of a stream, a cut and its revision epoch`);
  }
  for (const name of Object.keys(target)) {
    if (name !== "cut" && name !== "revision_epoch" && name !== "stream") {
      fail("REWIND_TARGET_UNEXPECTED", `${key} carries no \`${name}\``);
    }
  }
  const epoch = target.revision_epoch;
  const carried = epoch instanceof CircularUInt ? epoch.value : epoch;
  if (typeof carried !== "bigint") fail("REWIND_EPOCH_CARRIER", "a revision epoch is carried as a UInt");
  if (carried <= 0n) fail("REWIND_EPOCH_ZERO", "a revision epoch is nonzero");
  if (typeof target.stream !== "bigint" || target.stream < 0n || target.stream > 0x7fffffffffffffffn) {
    fail("REPLAY_STREAM_CARRIER", "a stream is the Int the timeline publishes");
  }
  return {
    cut: logCutValue(target.cut, ceilings, `${key}.cut`),
    revision_epoch: uint(carried),
    stream: target.stream,
  };
}

/** Reads a replay coordinate back into the shape it was built from. */
export function replayTargetFromValue(value, key = "to") {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail("REWIND_TARGET_SHAPE", `${key} is an object of a stream, a cut and its revision epoch`);
  }
  for (const name of Object.keys(value)) {
    if (name !== "cut" && name !== "revision_epoch" && name !== "stream") {
      fail("REWIND_TARGET_UNEXPECTED", `${key} carries no \`${name}\``);
    }
  }
  if (!(value.revision_epoch instanceof CircularUInt)) {
    fail("REWIND_EPOCH_CARRIER", "a revision epoch is carried as a UInt");
  }
  if (value.revision_epoch.value <= 0n) fail("REWIND_EPOCH_ZERO", "a revision epoch is nonzero");
  if (typeof value.stream !== "bigint" || value.stream < 0n) {
    fail("REPLAY_STREAM_CARRIER", "a stream is the Int the timeline publishes");
  }
  return { cut: logCutFromValue(value.cut), revision_epoch: value.revision_epoch.value, stream: value.stream };
}

export function replayTargetFromCheckpoint(checkpoint) {
  if (checkpoint === null || typeof checkpoint !== "object" || Array.isArray(checkpoint)) {
    fail("CHECKPOINT_SHAPE", "a checkpoint is a timeline item");
  }
  const { at_ms: _instant, ...coordinate } = checkpoint;
  return replayTargetFromValue(coordinate, "checkpoint");
}

export function replayRewindValue(rewind, ceilings) {
  if (rewind === null || typeof rewind !== "object") fail("REWIND_SHAPE", "a ReplayRewind is an object");
  for (const name of Object.keys(rewind)) {
    if (name !== "to" && name !== "pace") {
      fail("REWIND_UNEXPECTED", `a rewind carries no \`${name}\`; the origin and the arrangement are the session's and fixed`);
    }
  }
  const pace = paceValue(rewind.pace, ceilings);
  return rewind.to === undefined ? { pace } : { pace, to: replayTargetValue(rewind.to, ceilings) };
}

/** Reads a rewind back into the shape it was built from. */
export function replayRewindFromValue(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) fail("REWIND_SHAPE", "a ReplayRewind is an object");
  for (const name of Object.keys(value)) {
    if (name !== "to" && name !== "pace") fail("REWIND_UNEXPECTED", `a rewind carries no \`${name}\``);
  }
  const pace = paceFromValue(value.pace);
  return value.to === undefined ? { pace } : { to: replayTargetFromValue(value.to), pace };
}

/**
 * `ReplayEnd` — no body.
 *
 * Which session is being closed is the envelope's correlation key's answer, the same position
 * `Unsubscribe` and `Goodbye` hold. The absence rather than an empty object: an empty object is
 * a value and encodes to bytes, and this encodes to none.
 */
export const REPLAY_END_BODY = null;
