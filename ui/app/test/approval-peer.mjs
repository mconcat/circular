import { wireEnvelopeCodec, CircularUInt } from '@circular/protocol';
import { OWNER_LOCAL_RESOURCE_CEILINGS } from '@circular/client';

const codec = wireEnvelopeCodec(OWNER_LOCAL_RESOURCE_CEILINGS);
const wire = (partition, verb, payload) =>
  codec.decode(codec.encode({ kind: { partition, verb }, correlation: 1, payload })).envelope;
export const uint = value => new CircularUInt(value);
export const wireBytes = payload => codec.encode({ kind: { partition: 'LedgerTransition', verb: 'ApprovalDecide' }, correlation: 1, payload });
export const effect = n => [{ scope: [], local: 'remediate' }, [uint(n)], [uint(1n), new Uint8Array([7])], uint(0n)];
export const approvalRow = (item, state = 1n) => ({ item, emitter: [1n, [[1n, 'desk']], 'a'], target_effect: item,
  state, summary: [2n, 1n], cause: null });
export const approvalPage = (items) => ({ anchor: { producer: 1n, persistence: [3n] }, items, terminal: 2n });

export function decisionPeer({ page, answer }) {
  const decisions = [];
  let reads = 0;
  const session = {
    hold(name) {
      if (name !== 'runtime.approvals') throw new Error(`unexpected query ${name}`);
      return { send() {}, async next() { reads++; return wire('Query', 'QueryResult', [1n, page()]); }, release() {} };
    },
    async exchange(partition, verb, payload) {
      const request = wire(partition, verb, payload);
      decisions.push(request);
      return wire('LedgerTransition', 'TransitionResult', answer(request.payload));
    },
  };
  return { session, decisions, get reads() { return reads; } };
}
