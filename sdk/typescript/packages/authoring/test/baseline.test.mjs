import assert from "node:assert/strict";
import test from "node:test";
import { createProviderBindingRegistry } from "@circular/specs";

import { semanticPrepass } from "../src/index.js";
import { generateProgram } from "@circular/generator";

function diagnosticMessages(result) {
  return result.diagnostics.map((diagnostic) => diagnostic.message);
}

test("a bundle without modules rejects in the prepass with its diagnostic", () => {
  const prepass = semanticPrepass({ entry: "main.ts" }, {}, {});
  assert.equal(prepass.status, "rejected");
  assert.deepEqual(diagnosticMessages(prepass), ["authoring.prepass.invalid-bundle"]);
});

const currentBundle = (source) => ({ entry: "main.ts", modules: new Map([["main.ts", source]]) });

test("authoring that uses the current namespace beyond its lookups rejects in the prepass", () => {
  const attempts = [
    'import cur from "circular:current"; cur.refresh();',
    'import cur from "circular:current"; cur.close();',
    'import cur from "circular:current"; cur.current();',
    'import cur from "circular:current"; cur["refresh"]();',
    'import cur from "circular:current"; export let held = cur.refresh;',
    'import cur from "circular:current"; for await (const update of cur) { }',
    'import cur from "circular:current"; const tracker = cur; tracker.refresh();',
    'import * as cur from "circular:current"; cur.refresh();',
    'import * as cur from "circular:current"; for await (const update of cur) { }',
    'import { current } from "circular:current"; current.refresh();',
    'import { current as tracker } from "circular:current"; tracker.close();',
    'import { current } from "circular:current"; for await (const update of current) { }',
  ];
  for (const attempt of attempts) {
    const result = semanticPrepass(currentBundle(attempt), {}, {});
    assert.equal(result.status, "rejected", attempt);
    assert.deepEqual(diagnosticMessages(result), [
      "authoring.current.namespace-use-not-supported",
    ], attempt);
    assert.equal(result.diagnostics[0].phase, "Prepass", attempt);
    assert.equal(result.diagnostics[0].primary.span.source, "main.ts", attempt);
    assert.match(result.diagnostics[0].args[0], /take a complete snapshot again/, attempt);
  }
});

test("explicit resnapshot authoring keeps passing the prepass unchanged", () => {
  const accepted = [
    'import { x } from "circular:current"; x.setFlags({ bypass: false, mute: true, pause: false });',
    'import { current } from "circular:current";\nimport { note } from "@circular/core"; export let n = note({refs:[current.actor("x")],text:"kept"});\n',
    'import { json } from "@circular/core"; export let source = json({ value: "kept" });',
  ];
  for (const source of accepted) {
    const result = semanticPrepass(currentBundle(source), {}, {});
    assert.equal(result.status, "complete", `${source} ${JSON.stringify(result.diagnostics ?? [])}`);
  }
});

test("a generated program still round-trips through the prepass byte for byte", () => {
  const generated = generateProgram([{
    kind: "UpsertActor", actor: { arm: "epochLocal", value: { scope: [], local: "source" } },
    declaration: { actorType: "json", config: { value: "editable" }, flags: { bypass: false, mute: false, pause: false } },
  }], {
    sdkVersion: "0.1.0", specSet: new Uint8Array([3]),
    bindings: createProviderBindingRegistry({ bindings: [{ actorTypeId: "json", importSpecifier: "@circular/core", constructorExport: "json" }] }),
  });
  assert.equal(generated.status, "complete");
  const before = new TextDecoder().decode(generated.value.program.modules.get("main.ts"));
  const result = semanticPrepass(generated.value.program, {}, {});
  assert.equal(result.status, "complete", JSON.stringify(result.diagnostics ?? []));
  assert.equal(new TextDecoder().decode(generated.value.program.modules.get("main.ts")), before);
});
