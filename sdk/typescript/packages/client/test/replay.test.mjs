import assert from "node:assert/strict";
import test from "node:test";

import { chooseDisplayScale, displayRange } from "../src/replay.js";

test("display range registration and scale choice are deterministic", () => {
  assert.deepEqual(displayRange, {
    name: "display-range",
    paging: "cursor",
    anchorKind: "Records",
  });
  assert.equal(Object.isFrozen(displayRange), true);

  const scales = [100, 1, 10];
  assert.equal(chooseDisplayScale({ start: "0", end: "999" }, scales, 100), 10);
  assert.deepEqual(scales, [100, 1, 10], "selection must not mutate the registered scale list");
  assert.equal(chooseDisplayScale({ start: 0n, end: 9999n }, [1, 10, 100], 10), 100);
});

test("display scale selection rejects incomplete or invalid requests", () => {
  assert.throws(() => chooseDisplayScale({ start: 0, end: 1 }, [], 10), /non-empty/);
  assert.throws(() => chooseDisplayScale({ start: 2, end: 1 }, [1], 10), /must not precede/);
  assert.throws(() => chooseDisplayScale({ start: 0, end: 1 }, [0], 10), /positive/);
  assert.throws(() => chooseDisplayScale({ start: 0, end: 1 }, [1], 0), /positive/);
});
