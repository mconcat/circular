/** Registered display-range descriptor. */
export const displayRange = Object.freeze({
  name: "display-range",
  paging: "cursor",
  anchorKind: "Records",
});

function stampAsBigInt(stamp) {
  if (typeof stamp === "bigint") return stamp;
  if (typeof stamp === "number" && Number.isSafeInteger(stamp)) return BigInt(stamp);
  if (typeof stamp === "string" && /^-?[0-9]+$/.test(stamp)) return BigInt(stamp);
  throw new TypeError("chooseDisplayScale requires logical stamps with an integer external representation");
}

/**
 * Selects the finest recorded bucket that does not oversample the viewport.
 * Falls back to the coarsest recorded scale when every scale is finer.
 */
export function chooseDisplayScale(interval, recordedScales, width) {
  if (!Array.isArray(recordedScales) || recordedScales.length === 0) {
    throw new TypeError("recordedScales must be non-empty");
  }
  if (!Number.isSafeInteger(width) || width <= 0) {
    throw new RangeError("width must be a positive safe integer");
  }
  const start = stampAsBigInt(interval.start);
  const end = stampAsBigInt(interval.end);
  if (end < start) throw new RangeError("display interval end must not precede its start");
  const scales = [...recordedScales].sort((left, right) => left - right);
  if (scales.some((scale) => !Number.isSafeInteger(scale) || scale <= 0)) {
    throw new RangeError("recorded display scales must be positive safe integers");
  }
  const span = end - start + 1n;
  const minimum = (span + BigInt(width) - 1n) / BigInt(width);
  return scales.find((scale) => BigInt(scale) >= minimum) ?? scales.at(-1);
}
