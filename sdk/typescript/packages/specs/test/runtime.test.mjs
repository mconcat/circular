import assert from "node:assert/strict";
import test from "node:test";
import { createProviderBindingRegistry } from "../src/index.js";

test("constructor bindings map catalog names to SDK imports without supplying type identity", () => {
  const binding = {
    actorTypeId: "json",
    importSpecifier: "@circular/core",
    constructorExport: "json",
  };
  const registry = createProviderBindingRegistry({ bindings: [binding] });
  const resolved = registry.canonical("json");
  assert.deepEqual(resolved, { status: "resolved", binding });
  assert.ok(Object.isFrozen(resolved.binding));
  assert.deepEqual(registry.resolve({
    importSpecifier: "@circular/core",
    constructorExport: "json",
    expectedActorType: "json",
  }), resolved);
  assert.equal(registry.resolve({
    importSpecifier: "unrelated:module",
    constructorExport: "json",
    expectedActorType: "json",
  }).diagnostics[0].code, "PROVIDER_BINDING_UNKNOWN");
  assert.equal(registry.canonical("missing").diagnostics[0].code, "PROVIDER_CANONICAL_MISSING");

  const duplicate = createProviderBindingRegistry({ bindings: [binding, {
    ...binding, importSpecifier: "another:module",
  }] });
  assert.equal(duplicate.canonical("json").diagnostics[0].code, "PROVIDER_CANONICAL_AMBIGUOUS");
  assert.deepEqual(duplicate.resolve({
    importSpecifier: "@circular/core",
    constructorExport: "json",
    expectedActorType: "json",
  }), resolved);
  const sameImport = createProviderBindingRegistry({ bindings: [binding, binding] });
  assert.equal(sameImport.resolve({
    importSpecifier: "@circular/core",
    constructorExport: "json",
    expectedActorType: "json",
  }).diagnostics[0].code, "PROVIDER_BINDING_AMBIGUOUS");
});
