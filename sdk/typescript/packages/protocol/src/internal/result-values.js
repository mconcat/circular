/**
 * The one reader of a rejected result body `{code, message, hint?, at?}`. The Declaration
 * `CommandResult` and the Lifecycle `LifecycleResult` share it (`lifecycle_payload.rs`
 * reuses `declaration_payload::Rejected`), so there is one TypeScript reader too.
 */
function fail(code, message) {
  const error = new TypeError(`${code}: ${message}`);
  error.code = code;
  throw error;
}

export function rejectionFromValue(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail("REJECTION_SHAPE", "a rejection is an object");
  }
  if (typeof value.code !== "bigint" || typeof value.message !== "string") {
    fail("REJECTION_SHAPE", "a rejection carries Int code and String message");
  }
  return Object.freeze({
    status: "rejected",
    reason: "Invalid",
    diagnostics: Object.freeze([Object.freeze({
      code: Number(value.code),
      message: value.message,
      hint: value.hint ?? null,
      at: value.at ?? null,
    })]),
  });
}
