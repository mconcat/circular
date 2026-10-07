import type { CircularValue } from "./index.js";

/** A refusal while building a session-establishment value. */
export declare class EstablishmentValueError extends Error {
  /** Names the exact refusal. */
  readonly code: string;
}

/** The transport trust grades; all are argument-free. */
export declare const TRANSPORT_TRUST_TAGS: Readonly<Record<string, number>>;

/** One session role's assigned tag and the argument its arm carries, if any. */
export interface SessionRoleArm {
  /** The integer that names this arm. */
  readonly tag: number;
  /** The argument's published name, or `null` for an argument-free arm. */
  readonly argument: string | null;
}

/** The session roles with their argument order. */
export declare const SESSION_ROLE_ARMS: Readonly<Record<string, SessionRoleArm>>;

/** Scope segment arms: a named segment and an instance segment. */
export declare const SCOPE_SEGMENT_ARMS: Readonly<Record<string, number>>;

/** Orders a set's elements by their canonical bytes, refusing duplicates. */
export declare function canonicalValueSequence(
  values: readonly CircularValue[],
  resourceCeilings: unknown,
): readonly CircularValue[];

/** The value of one session role: the tag alone, or a sequence whose head is the tag. */
export declare function sessionRoleValue(role: string, argument?: CircularValue): CircularValue;

/** The value of one transport trust grade. */
export declare function transportTrustValue(grade: string): CircularValue;

/** A scope identity: an ordered sequence of segments, never sorted. */
export declare function scopeIdentityValue(
  segments: readonly unknown[],
): readonly CircularValue[];

/** One scope segment: `[1, name]` or `[2, of, key]`. */
export declare function scopeSegmentValue(segment: unknown): CircularValue;

/** Reads a scope identity back into segments; the inverse of `scopeIdentityValue`. */
export declare function scopeIdentityFromValue(value: unknown): readonly unknown[];

/** A plan actor key: the product of a scope and a local name, with no arm tag of its own. */
export declare function actorIdentityValue(key: unknown): CircularValue;

/** Reads a plan actor key back; the inverse of `actorIdentityValue`. */
export declare function actorIdentityFromValue(value: unknown): unknown;
