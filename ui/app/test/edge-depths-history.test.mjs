import test from 'node:test';
import assert from 'node:assert/strict';
import { reconstructiveCommandFromValue } from '@circular/protocol/declaration';
import { productMachine, recordedArrival } from './time-harness.mjs';
import { lensPeer, turns } from './replay-peer.mjs';

const end = (local, port) => ({ actor: { scope: [], local }, port });
const commands = [
  reconstructiveCommandFromValue({ kind: 'UpsertActor', actor: [3n, { local: 'two', scope: [] }],
    declaration: { domain: { actor_type: 'synthetic', config: {} }, flags: { bypass: false, mute: false, pause: false } } }),
  reconstructiveCommandFromValue({ kind: 'UpsertEdge', edge: [3n, [1n, end('one', 'event'), end('two', 'input'), 0n]],
    declaration: { from: [{ scope: [], local: 'one' }, 'event'], to: [{ scope: [], local: 'two' }, 'input'], ordinal: 0n,
      attrs: { delay: { num: 0n, den: 1n }, policy: { delivery: 2n }, preprocess: [] } } }),
];
const wire = [1n, end('one', 'event'), end('two', 'input'), 0n];

test('the past draws no pool and no depth, asks nothing while live arrivals come, and Live asks for the value now', async t => {
  const peer = await lensPeer({ commands });
  t.after(() => peer.replay.session.close());
  peer.depths = [{ edge: wire, depth: 2n, queued: 1n, capacity: 4n }];
  await productMachine(t, peer.sdk, async ({ source, machine }) => {
    const shown = [], reads = () => peer.questions();
    window.StudyApp.showMailboxes = rows => shown.push(rows);
    window.StudyApp.historical = () => machine.mode !== 'live';
    window.StudyApp.displayTime = () => machine.position;
    await turns();
    const two = window.STUDY.root.nodes.find(n => n.title === 'two'), [line] = window.STUDY.root.edges;
    assert.ok(two && line);

    const first = reads();
    peer.arrive(recordedArrival('one', 2n, 3_000n), { lens: false });
    await turns();
    assert.equal(reads(), first + 1);
    assert.deepEqual(shown.at(-1), [{ wire: line.id, depth: 2, queued: 1, capacity: 4 }]);
    const live = source.mailbox(two);
    assert.deepEqual(live.rows.map(r => [r.wire, r.depth, r.queued, r.capacity]), [[line.id, 2n, 1n, 4n]]);

    await machine.seek(1);
    await turns();
    assert.equal(machine.mode, 'history');
    const past = reads(), handed = shown.length;
    for (const [index, at] of [[3n, 4_000n], [4n, 5_000n]]) {
      peer.arrive(recordedArrival('one', index, at), { lens: false });
      await turns();
    }
    assert.equal(reads(), past, 'no question while the past is shown');
    assert.equal(shown.length, handed, 'nothing handed to the canvas while the past is shown');
    assert.deepEqual(source.mailbox(two), { code: 'MAILBOX_DEPTH_UNRECORDED' });

    peer.depths = [{ edge: wire, depth: 4n, capacity: 4n }];
    await machine.live();
    await turns();
    assert.equal(machine.mode, 'live');
    assert.equal(reads(), past + 1);
    assert.deepEqual(shown.at(-1), [{ wire: line.id, depth: 4, queued: 0, capacity: 4 }]);
    assert.deepEqual(source.mailbox(two).rows.map(r => [r.depth, r.queued, r.capacity]), [[4n, 0n, 4n]]);
  });
});
