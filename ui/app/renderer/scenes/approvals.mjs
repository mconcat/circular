import { lane, readRecords, recordRows } from './shared.mjs';
export default { name: 'approvals', target: '#read-surface', label: 'Requests',
  async read(session, graph, scope) { return { graph, records: await readRecords(session, scope),
    approvals: await lane(session, 'runtime.approvals') }; },
  project({ graph, records, approvals }) {
    return { kind: 'approvals', graph, records, recordRows: recordRows(records), approvals,
      rows: approvals.status === 'available' ? approvals.page.items.map(row => ({
        item: row.item, emitter: row.emitter, state: row.state, target: row.target_effect,
        cause: row.cause,
      })) : [] };
  },
  records: true };
