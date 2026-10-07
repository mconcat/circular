import { scopeIdentityValue } from '@circular/protocol/establishment';
import { recordsPageFromValue } from '@circular/client';
import { readFirstPage, journalRows } from '../query.mjs';
import { reason, systemOutcomeReason } from '../reasons.mjs';
export async function lane(session, name, args = null, limit, read = page => page) {
  try { return { status: 'available', page: read(await readFirstPage(session, name, args, limit)) }; }
  catch (error) { return { status: 'unavailable', diagnostic: reason(error.code ?? 'READ_UNAVAILABLE'), ...(error.page ? { page: error.page } : {}) }; }
}
export const readRecords = async (session, scope) => ({ ...(await lane(session, 'records', { scope: scopeIdentityValue(scope) }, journalRows, recordsPageFromValue)), scope });
export function recordRows(source) {
  if (source.status !== 'available') return [];
  return source.page.items.map(item => ({ cursor: item.cursor, reached: item.reached,
    bytes: item.fact instanceof Uint8Array ? item.fact.byteLength : null,
    system: item.system ? { ...item.system,
      diagnostic: item.system.code == null ? null : systemOutcomeReason(item.system.code) } : null }));
}
