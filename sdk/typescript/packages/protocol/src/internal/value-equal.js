import { encodeValueBeta } from '../value.js';

const FORMAT_BOUNDS = Object.freeze({
  maximumBytes: 0xffff_ffff,
  maximumDepth: 64,
  maximumContainerEntries: 0xffff_ffff,
  maximumStringBytes: 0xffff_ffff,
});

const canonicalBytes = value => encodeValueBeta(value, FORMAT_BOUNDS);

/** Canonical codec bytes as lowercase hexadecimal, suitable for a Map key. */
export function valueKey(value) {
  return Array.from(canonicalBytes(value), byte => byte.toString(16).padStart(2, '0')).join('');
}

/** Byte-for-byte equality of two byte strings. */
export function sameBytes(left, right) {
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) if (left[index] !== right[index]) return false;
  return true;
}

/**
 * Whether two values are the same value. An absent value equals only an absent value; a value the
 * codec cannot write is no value of the model, so it equals nothing.
 */
export function sameValue(left, right) {
  if (left === undefined || right === undefined) return left === right;
  try {
    return sameBytes(canonicalBytes(left), canonicalBytes(right));
  } catch {
    return false;
  }
}
