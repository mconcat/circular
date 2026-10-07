import assert from "node:assert/strict";
import test from "node:test";

import {
  defineExport,
  grid,
  messages,
  surface,
  textInput,
  window,
} from "../src/index.js";

function runMode() {
  return defineExport({
    roles: {
      request: { type: "json" },
      result: { type: "json" },
    },
    surfaces: (roles) => [
      window("Run", grid([
        textInput(roles.request).placeholder("Enter a request"),
        messages(roles.result),
      ])),
    ],
  });
}

test("export structural builders compose and remain immutable", () => {
  const definition = runMode();
  const serialized = JSON.parse(JSON.stringify(definition));

  assert(Object.isFrozen(definition));
  assert(Object.isFrozen(definition.surfaces));
  assert.equal(serialized.surfaces[0].mark, "window");
  assert.equal(serialized.surfaces[0].children[0].mark, "grid");
});

test("fixed role direction is checked while building controls", () => {
  assert.throws(() => defineExport({
    roles: { result: { type: "text" } },
    surfaces: (roles) => [window("Invalid", textInput(roles.result))],
  }), /requires the writable request role/);
});

test("surface requires an installed collector", () => {
  const definition = runMode();
  assert.throws(
    () => surface("run", definition),
    { code: "CIRCULAR_NO_EXECUTION_CONTEXT" },
  );
});

test('exports does not advertise the retired standalone mount function', async () => {
  const exported = await import('@circular/exports');
  assert.equal(Object.hasOwn(exported, 'mount'), false);
  assert.equal(typeof exported.surface, 'function');
});
