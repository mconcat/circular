import { setAgentHarnessAcceptedFromValue } from '../../protocol/src/internal/observation-values.js';

export async function setAgentHarness(session, request) {
  if (!request || typeof request !== 'object' || Object.keys(request).some(key => !['name', 'program'].includes(key))
    || typeof request.name !== 'string' || request.name === ''
    || !Object.hasOwn(request, 'program') || (request.program !== null && typeof request.program !== 'string')) {
    throw new TypeError('invalid SetAgentHarness request');
  }
  const answer = await session.exchange('LedgerTransition', 'SetAgentHarness', {
    name: request.name,
    program: request.program,
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
      throw new TypeError('invalid SetAgentHarness rejection');
    }
  } else if (result[0] === 1n) {
    setAgentHarnessAcceptedFromValue(result[1]);
  } else throw new TypeError('unknown TransitionResult arm');
  return result;
}
