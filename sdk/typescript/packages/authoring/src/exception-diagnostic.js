/** Preserve thrown details in the existing diagnostic arguments, including string error codes. */
export function exceptionDiagnostic(base, error) {
  const code = error?.code ?? null;
  const message = String(error?.message ?? error);
  return Object.freeze({ ...base,
    code: typeof code === 'number' ? code : base.code,
    args: Object.freeze([code, message]),
  });
}
