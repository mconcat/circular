export function evidenceText(evidence) {
  if (!evidence) return '';
  return [evidence.reason, evidence.log && `log ${evidence.log}`, evidence.logTail, evidence.stderr]
    .filter(value => typeof value === 'string' && value.length > 0)
    .map(value => value.replace(/\n+$/, '')).join('\n');
}

const lines = text => typeof text === 'string' ? text.split('\n').filter(line => line.trim().length > 0) : [];
export function evidenceCause(evidence) {
  if (!evidence) return '';
  const last = (lines(evidence.logTail).length ? lines(evidence.logTail) : lines(evidence.stderr)).at(-1);
  return [last, evidence.reason].filter(value => typeof value === 'string' && value.length > 0).join('\n');
}

export function updateConnectionEvidence(root, evidence) {
  const text = evidenceText(evidence), cause = evidenceCause(evidence);
  const said = root?.querySelector?.('#connection-strip > .connection-cause');
  const whole = root?.querySelector?.('#connection-strip > .connection-evidence');
  const body = root?.querySelector?.('#connection-strip > .connection-evidence > pre');
  if (said) { said.textContent = cause; said.hidden = cause === ''; }
  if (body) body.textContent = text;
  if (whole) whole.hidden = text === '' || text === cause;
}
