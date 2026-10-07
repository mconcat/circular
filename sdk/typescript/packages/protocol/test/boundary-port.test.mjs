import assert from "node:assert/strict";
import test from "node:test";

import { deriveBoundaryPortId, deriveSynthBoundaryLocal } from "../src/index.js";

/**
 * Shared literally with `crates/protocol/src/boundary_port.rs`.
 * These pin both scope-segment arms, both directions, and initial/nonzero generations.
 */
const KNOWN_ANSWERS = Object.freeze([
  {
    direction: "inlet",
    actor: { scope: [], local: "source" },
    generation: 0n,
    id: "_bi1_33577e6b2e50666123d8270f2a",
  },
  {
    direction: "inlet",
    actor: { scope: [{ name: "fleet" }, { name: "session-cell" }], local: "input" },
    generation: 0n,
    id: "_bi1_ac4647791b31cb74eef38fdbde",
  },
  {
    direction: "outlet",
    actor: { scope: [{ name: "fleet" }, { name: "session-cell" }], local: "output" },
    generation: 0n,
    id: "_bo1_150474c65a280d64734cea3258",
  },
  {
    direction: "outlet",
    actor: { scope: [{ name: "fleet" }, { of: "session-cell", key: "session-17" }], local: "output" },
    generation: 9n,
    id: "_bo1_252ae83bffa5809a613ccc033a",
  },
]);

test("boundary-port spellings match the Rust known-answer vectors", () => {
  for (const vector of KNOWN_ANSWERS) {
    assert.equal(
      deriveBoundaryPortId(vector.direction, vector.actor, vector.generation),
      vector.id,
    );
  }
});

test("boundary-port derivation rejects identities Rust cannot represent", () => {
  assert.throws(() => deriveBoundaryPortId("sideways", { scope: [], local: "source" }, 0n), TypeError);
  assert.throws(() => deriveBoundaryPortId("inlet", { scope: [], local: "" }, 0n), TypeError);
  assert.throws(() => deriveBoundaryPortId("inlet", { scope: [], local: "source" }, -1n), RangeError);
  assert.throws(() => deriveBoundaryPortId("inlet", { scope: [], local: "source" }, 2n ** 64n), RangeError);
});

test("synthesized boundary locals match the Rust known-answer vectors", () => {
  const vectors = [
    ["inlet", "source", "event", "_bni1_49acde745761d9598d1c726ec4"],
    ["outlet", "meter", "out", "_bno1_2a622b39ae28447eea187b5652"],
    /** Non-ASCII actor and port names, on purpose: the same spelling must derive the same local as in Rust. */
    ["inlet", "액터", "포트", "_bni1_6064a41f6a5b185491bbb2377f"],
    [
      "outlet",
      "_bni1_49acde745761d9598d1c726ec4",
      "relay",
      "_bno1_0a2689f81da7f117fed84f126d",
    ],
  ];

  for (const [direction, local, port, expected] of vectors) {
    const actual = deriveSynthBoundaryLocal(direction, local, port);
    assert.equal(actual, expected);
    assert.equal(actual.length, 32);
  }
});
