import { encodeValueBeta } from '../../protocol/src/value.js';
import { approvalDecisionReceiptFromValue } from '../../protocol/src/internal/observation-values.js';
import { sameBytes } from '../../protocol/src/internal/value-equal.js';
import { OWNER_LOCAL_RESOURCE_CEILINGS } from './connection-defaults.js';

/** Sends the existing LedgerTransition.ApprovalDecide and retains its result arm. */
export async function decideApproval(session, request) {
  if (!request || Object.keys(request).some(key => !['item', 'decision'].includes(key))
    || !['Approve', 'Deny'].includes(request.decision)
    || !Array.isArray(request.item) || request.item.length !== 4) throw new TypeError('invalid ApprovalDecide request');
  const { decision } = request;
  const itemBytes = encodeValueBeta(request.item, OWNER_LOCAL_RESOURCE_CEILINGS);
  const sameItem = value => sameBytes(encodeValueBeta(value, OWNER_LOCAL_RESOURCE_CEILINGS), itemBytes);
  const answer = await session.exchange('LedgerTransition', 'ApprovalDecide', {
    item: request.item,
    decision: decision === 'Approve' ? 1n : 2n,
  });
  if (answer?.kind?.partition !== 'LedgerTransition' || answer.kind.verb !== 'TransitionResult') {
    throw new TypeError('expected LedgerTransition.TransitionResult');
  }
  const result = answer.payload;
  if (!Array.isArray(result) || result.length !== 2) throw new TypeError('invalid TransitionResult');
  if (result[0] === 2n) {
    const rejected = result[1];
    if (!rejected || typeof rejected.code !== 'bigint' || rejected.code < 0n
      || rejected.code > 0xffffffffn || typeof rejected.message !== 'string'
      || (Object.hasOwn(rejected, 'hint') && typeof rejected.hint !== 'string')
      || Object.keys(rejected).some(key => !['code', 'message', 'hint', 'at'].includes(key))) {
      throw new TypeError('invalid approval rejection');
    }
  } else if (result[0] === 1n) {
    const receipt = approvalDecisionReceiptFromValue(result[1]);
    if (!sameItem(receipt.item)
      || (decision === 'Approve' ? !Array.isArray(receipt.outcome)
        || !sameItem(receipt.outcome[1]) : receipt.outcome !== 2n)) {
      throw new TypeError('approval receipt does not match request');
    }
  } else throw new TypeError('unknown TransitionResult arm');
  return result;
}
