import assert from 'node:assert/strict';
import { wireEnvelopeCodec } from '@circular/protocol';
import { editPeer } from './edit-peer.mjs';
import { product } from './gesture-harness.mjs';

const codec = wireEnvelopeCodec({ maximumBytes: 1048576, maximumDepth: 64, maximumContainerEntries: 4096, maximumStringBytes: 65536 });

export function sameFrame(actual, expected, label) {
  const a = codec.decode(actual), b = codec.decode(expected);
  assert.equal(a.status, 'complete'); assert.equal(b.status, 'complete');
  assert.deepEqual(codec.encode({ ...a.envelope, correlation: b.envelope.correlation }), expected, label);
}

export async function sameEpochBytes(gesture, commands, options = {}) {
  const side = () => typeof options.fixtureValue === 'function' ? { ...options, fixtureValue: options.fixtureValue() } : options;
  const peer = await editPeer(side()), independent = await editPeer(side());
  try {
    await product(peer, notes => gesture(notes));
    assert.deepEqual(peer.sent.map(c => c.kind), ['BeginEpoch', ...commands.map(c => c.kind), 'ValidateEpoch', 'CommitEpoch']);
    const opened = await independent.session.declare(peer.sent[0]);
    assert.equal(opened.status, 'accepted');
    for (const command of commands) assert.equal((await independent.session.declare(command)).status, 'accepted');
    await independent.session.declare({ kind: 'ValidateEpoch', epoch: opened.value.epoch });
    await independent.session.declare({ kind: 'CommitEpoch', epoch: opened.value.epoch });
    assert.equal(independent.frames.length, commands.length + 3);
    assert.equal(peer.frames.length, commands.length + 3);
    for (let i = 0; i < independent.frames.length; i++) sameFrame(peer.frames[i], independent.frames[i], `frame ${i}`);
    assert.deepEqual(peer.f.snapshot, independent.f.snapshot, 'both spellings fold to the same authoring state');
  } finally { await peer.session.close?.(); await independent.session.close(); }
}
