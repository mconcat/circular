import test from 'node:test';
import assert from 'node:assert/strict';
import { OWNER_LOCAL_RESOURCE_CEILINGS } from '@circular/client';
import { envelope, HEADER_BYTES, wireEnvelopeCodec } from '@circular/protocol';
import { declarationPayloadValue } from '@circular/protocol/declaration';
import { configFieldList, readConfigForm } from '../renderer/config-fields.mjs';
import { createActor, editor } from '../renderer/edit.mjs';
import { snapshotToScene } from '../renderer/scene.mjs';
import { UNMEASURED } from '../renderer/card-size.mjs';
import { editPeer } from './edit-peer.mjs';
import { fixture, catalogRow, answered } from './fixtures.mjs';

test('canvas field rows commit the independent form declaration and fold bytes', async t => {
  const fixtureValue = () => {
    const f = fixture();
    f.catalog.value.items = [catalogRow('form', [2n, 'incomplete'], { creatable: true, source: true })];
    return f;
  };
  const canvas = await editPeer({ fixtureValue: fixtureValue() });
  try {
    const entry = { slots: [{ key: 'fields', path: [[1n, 'fields']], required: true,
      shape: { kind: 'Array', item: { kind: 'Any' } }, constraint: [7n] }] };
    const rows = [{ name: 'question', base: 'string', nameInput: 'name0', typeInput: 'type0' },
      { name: 'count', base: 'int', nameInput: 'name1', typeInput: 'type1' }];
    const list = configFieldList({}, true, entry);
    assert.equal(list.fields[0].kind, 'type-fields');
    list.fields[0].rows = rows;
    const values = { name0: 'question', type0: 'string', name1: 'count', type1: 'int' };
    const config = readConfigForm({}, { elements: { namedItem: name => ({ value: values[name] }) } }, list.fields);
    const expected = { kind: 'UpsertActor', actor: { arm: 'absolute', value: { scope: [{ name: 'desk' }], local: 'request' } },
      declaration: { actorType: 'form', config: { fields: [1n, [4n,
        [{ name: 'question', shape: [2n, 'string'] }, { name: 'count', shape: [2n, 'int'] }], false]] },
      flags: { bypass: false, mute: false, pause: false } } };
    assert.deepEqual(config, expected.declaration.config);
    const graph = snapshotToScene(canvas.f.snapshot, answered(canvas.f.ports), canvas.f.catalog, canvas.f.health, canvas.f.events, UNMEASURED);
    const created = await createActor(canvas.session, graph, graph.anchor.scope, 'request',
      { actor_type: 'form' }, config, { bypass: false, mute: false, pause: false });
    assert.equal(created.status, 'accepted');
    const edit = editor(canvas.session, () => new Uint8Array(16));
    assert.equal((await edit.prepare(graph.anchor, [created.value])).status, 'accepted');
    assert.equal((await edit.commit()).status, 'accepted');

    const canvasCommands = canvas.sent.filter(command => command.kind === 'UpsertActor');
    assert.equal(canvasCommands.length, 1);
    const codec = wireEnvelopeCodec(OWNER_LOCAL_RESOURCE_CEILINGS);
    const valueBytes = value => codec.encode(envelope('Query', 'QueryResult', 1, value)).subarray(HEADER_BYTES);
    const body = command => declarationPayloadValue(command).declaration;
    assert.deepEqual(valueBytes(body(canvasCommands[0])), valueBytes(body(expected)));
    const folded = peer => peer.f.snapshot.value.commands.find(command => command.kind === 'UpsertActor'
      && command.actor.value.local === 'request');
    const expectedFold = { ...expected, actor: { arm: 'relative', value: { scope: [], local: 'request' } } };
    const bytes = command => valueBytes(declarationPayloadValue(command, { context: 'snapshot', includeKind: true }));
    assert.deepEqual(bytes(folded(canvas)), bytes(expectedFold));
    t.diagnostic(`declaration body ${valueBytes(body(expected)).length} bytes; folded command ${bytes(expectedFold).length} bytes`);
  } finally {
    await canvas.session.close();
  }
});
