import { reason } from '../reasons.mjs';
import { readRecords } from './shared.mjs';
import { journalRows } from '../query.mjs';
import { readProblems, failureCode } from '../wire-inspector.mjs';
export default { name: 'error', target: '#read-surface', label: 'Problems',
  async read(session, graph, scope) { return { graph, letters: await readProblems(session, journalRows), records: await readRecords(session, scope) }; },
  project({ graph, letters, records }) {
    return { kind: 'error', graph, letters, records,
      health: graph.nodes.filter(node => node.health).map(node => ({ actor: node.address, ...reason(node.health.reason) })),
      deadLetters: letters.rows.map(row => ({ ordinal: row.ordinal, target: row.target, point: row.point, failure: failureCode(row), ...reason(row.reason.code) })),
      rejections: [letters.diagnostic && reason(letters.diagnostic), records.status === 'unavailable' && records.diagnostic].filter(Boolean),
      missing: [] };
  } };
