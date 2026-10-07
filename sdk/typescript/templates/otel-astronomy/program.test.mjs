import assert from "node:assert/strict";
import test from "node:test";
import { semanticPrepass } from "@circular/authoring";
import * as core from "@circular/core";
import { astronomyProgram } from "./deploy.mjs";

test("the filled astronomy program uses this SDK's constructors and passes its prepass", () => {
  const program = astronomyProgram({ prometheus: "http://127.0.0.1:9090/", demo: "http://127.0.0.1:18080", template: "/Users/example/otel-astronomy" });
  const source = new TextDecoder().decode(program.modules.get(program.entry));
  for (const name of ["$TEMPLATE", "$PROMETHEUS", "$DEMO"]) assert.equal(source.includes(name), false, `${name} left unfilled`);
  const imported = /import \{([^}]+)\} from "@circular\/core"/.exec(source)[1].split(",").map((name) => name.trim());
  for (const name of imported) assert.equal(typeof core[name], "function", `@circular/core has no ${name}`);
  const prepared = semanticPrepass(program);
  assert.equal(prepared.status, "complete", JSON.stringify(prepared.diagnostics));
});
