import assert from "node:assert/strict";
import test from "node:test";

import {
  accepted,
  defineQuery,
  defineSubscription,
  envelope,
  identityEnvelopeCodec,
  isAccepted,
  isRejected,
  rejected,
} from "../src/index.js";

test("protocol helpers preserve the existing envelope and Result shapes", () => {
  const success = accepted({ epoch: "e1" });
  const refusal = rejected("Conflict", [{ code: 1, message: "conflict", hint: null, at: null }]);
  assert.equal(isAccepted(success), true);
  assert.equal(isRejected(refusal), true);

  const value = envelope("Declaration", "BeginEpoch", "c1", { kind: "BeginEpoch" });
  const decoded = identityEnvelopeCodec.decode(identityEnvelopeCodec.encode(value));
  assert.equal(decoded.status, "complete");
  assert.deepEqual(decoded.envelope, value);
});

test("descriptors contain only registration-owned wire fields", () => {
  assert.deepEqual(
    defineQuery({ name: "authoring-snapshot", paging: "cursor", anchorKind: "AuthoringSnapshot" }),
    { name: "authoring-snapshot", paging: "cursor", anchorKind: "AuthoringSnapshot" },
  );
  assert.deepEqual(
    defineSubscription({ name: "authoring-commits", discipline: "lossless" }),
    { name: "authoring-commits", discipline: "lossless" },
  );
});
