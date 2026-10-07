import { reason } from './reasons.mjs';

export function observationsLine(code) {
  return code == null ? { label: 'Recorded observations' } : { label: 'Recorded observations', code: reason(code).code };
}
