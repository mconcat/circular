import assert from "node:assert/strict";
import test from "node:test";
import { createProviderBindingRegistry } from "@circular/specs";
import { generateProgram } from "@circular/generator";
import { generateProgramFromSession } from '@circular/generator/internal';
export const options = {
  sdkVersion: "0.1.0", specSet: new Uint8Array([3]),
  bindings: createProviderBindingRegistry({ bindings: ["json", "map", "tap", "pipeline_actor"].map((actorTypeId) => ({
    actorTypeId, importSpecifier: "@circular/core", constructorExport: actorTypeId,
  })) }),
};
const flags = { bypass: false, mute: true, pause: false };
const key = (local) => ({ scope: [], local });
const address = (local) => ({ arm: "epochLocal", value: key(local) });
const scope = { arm: "epochLocal", value: [{ name: "child" }] };
export const smallLog = [
  { kind: "BeginEpoch", scope: { arm: "absolute", value: [] }, commitId: new Uint8Array(16),
    expectedRevision: { kind: "Absent" }, expectedEnvironment: { declarationSchema: new Uint8Array([1]), specSet: options.specSet } },
  { kind: "UpsertActor", actor: address("source"), declaration: { actorType: "json", flags, config: {
    value: { count: 9007199254740993n, fraction: 1.5, minusZero: -0, bytes: new Uint8Array([0, 255]),
      list: [null, true, "quote\"\n; throw new Error('injected') //"], ["__proto__"]: { safe: true } },
  } } },
  { kind: "UpsertActor", actor: address("map"), declaration: { actorType: "tap", flags, config: null } },
  { kind: "UpsertActor", actor: address("child"), declaration: { actorType: "pipeline_actor", flags, config: null } },
  { kind: "UpsertScope", scope, declaration: { role: "Template", boundary: { inlets: [], outlets: [] } } },
  { kind: "UpsertEdge", edge: { arm: "epochLocal", value: { from: { actor: key("source"), port: "value" },
    to: { actor: key("map"), port: "event" }, ordinal: 7 } }, declaration: {
    from: { actor: key("source"), port: "value" }, to: { actor: key("map"), port: "event" }, ordinal: 7,
    attrs: { preprocess: [{ kind: "map", config: { transform: "event.value + 1" } }], delay: { num: 1n, den: 2n }, policy: { capacity: 8n, delivery: { mode: "BestEffort", onFull: "DropOldest" } } },
  } },
  { kind: "MoveToScope", actors: [address("map")], target: scope },
  { kind: "SetFlags", actor: address("source"), flags: { bypass: true, mute: false, pause: true } },
  { kind: "ValidateEpoch", epoch: new Uint8Array([9]) },
  { kind: "CommitEpoch", epoch: new Uint8Array([9]) },
];

for (const kind of ['UpsertAnnotation', 'SetPresentation', 'UpsertExportMount', 'RetireActor']) {
  test(`product generator refuses ${kind} with exact command location`, () => {
    const result = generateProgram([smallLog[1], {kind}], options);
    assert.equal(result.status, 'rejected');
    assert.equal(result.diagnostics[0].primary.commandIndex, 1);
    assert.equal(result.diagnostics[0].args[0], kind);
    assert.ok(!('value' in result));
  });
}
test('product snapshot acquisition rejects partial, thrown and pin-mismatched results', async () => {
  for (const answer of [{status:'partial'}, {status:'rejected'}, {status:'accepted',value:{commands:[],anchor:{environment:{specSet:new Uint8Array([9])}}}}]) {
    assert.equal((await generateProgramFromSession({authoringSnapshot:async()=>answer},options)).status, 'rejected');
  }
  const result = await generateProgramFromSession({authoringSnapshot:async()=>{throw new Error('read failed');}}, options);
  assert.equal(result.diagnostics[0].message, 'authoring.generator.snapshot-failed');
});
