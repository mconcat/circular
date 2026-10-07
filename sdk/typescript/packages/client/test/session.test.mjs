import assert from "node:assert/strict";
import test from "node:test";

import { createExecutionEpoch } from "../src/internal/execution.js";

function accepted(value) {
  return { status: "accepted", value };
}

test("commit rejection is terminal and is not followed by AbortEpoch", async () => {
  const calls = [];
  const correlated = (value) => ({ correlation: `c${calls.length}`, completion: Promise.resolve(value) });
  const session = {
    declarations: {
      begin(command) {
        calls.push(command.kind);
        return correlated(accepted({ epoch: "epoch-rejected" }));
      },
      apply(request) {
        calls.push(request.command.kind);
        return correlated(accepted({ epoch: request.epoch }));
      },
      validate(command) {
        calls.push(command.kind);
        return correlated(accepted({ epoch: command.epoch, restartUpperBound: new Set() }));
      },
      commit(command) {
        calls.push(command.kind);
        return correlated({
          status: "rejected",
          reason: "Stale",
          diagnostics: [{ code: 1, message: "stale", hint: null, at: null }],
        });
      },
      abort(command) {
        calls.push(command.kind);
        return correlated(accepted({ epoch: command.epoch }));
      },
    },
  };

  const opened = await createExecutionEpoch(session, {
    kind: "BeginEpoch",
    scope: "root",
    commitId: "commit-rejected",
    expectedRevision: { kind: "At", revision: "old" },
    expectedEnvironment: "env-1",
  });
  assert.equal(opened.status, "accepted");
  opened.value.emit({ kind: "RetireActor", actor: "n1" });
  const result = await opened.value.complete();
  assert.equal(result.status, "rejected");
  assert.equal(opened.value.state, "rejected");
  assert.deepEqual(calls, ["BeginEpoch", "RetireActor", "ValidateEpoch", "CommitEpoch"]);
});

test("an accepted BeginEpoch without an EpochId is rejected before evaluation", async () => {
  const opened = await createExecutionEpoch({
    declarations: {
      begin: () => ({ correlation: "c1", completion: Promise.resolve(accepted({})) }),
    },
  }, {
    kind: "BeginEpoch",
    scope: "root",
    commitId: "bad-open",
    expectedRevision: { kind: "Absent" },
    expectedEnvironment: "env-1",
  });
  assert.equal(opened.status, "rejected");
  assert.equal(opened.reason, "Malformed");
});

for (const abortFirst of [false, true]) {
  test(`explicit abort and a delayed apply rejection share one terminal request (abort first: ${abortFirst})`, async () => {
    const calls = [];
    let finishApply, finishAbort;
    const applyDone = new Promise(resolve => { finishApply = resolve; });
    const abortDone = new Promise(resolve => { finishAbort = resolve; });
    const response = completion => ({ correlation: `c${calls.length}`, completion });
    const session = { declarations: {
      begin(command) { calls.push(command.kind); return response(Promise.resolve(accepted({ epoch: "race" }))); },
      apply(request) { calls.push(request.command.kind); return response(applyDone); },
      abort(command) { calls.push(command.kind); return response(abortDone); },
      validate() { assert.fail("aborted candidate cannot validate"); },
      commit() { assert.fail("aborted candidate cannot commit"); },
    } };
    const opened = await createExecutionEpoch(session, { kind: "BeginEpoch" });
    const epoch = opened.value;
    epoch.emit({ kind: "RetireActor", actor: "n1" });
    const completion = epoch.complete();
    assert.equal(epoch.complete(), completion);
    const abortion = epoch.abort();
    assert.equal(epoch.abort(), abortion);
    const applyFailure = { status: "rejected", reason: "Invalid", diagnostics: [] };
    if (abortFirst) {
      finishAbort(accepted({ epoch: "race" }));
      await abortion;
      finishApply(applyFailure);
    } else {
      finishApply(applyFailure);
      await new Promise(resolve => setImmediate(resolve));
      finishAbort(accepted({ epoch: "race" }));
    }
    assert.equal(await completion, applyFailure);
    assert.equal((await abortion).status, "accepted");
    assert.equal(epoch.state, "aborted");
    assert.deepEqual(calls, ["BeginEpoch", "RetireActor", "AbortEpoch"]);
  });
}

for (const held of ["ValidateEpoch", "CommitEpoch"]) {
  test(`abort during ${held} preserves the one terminal request`, async () => {
    const calls = [];
    let finish, reached;
    const pending = new Promise(resolve => { finish = resolve; });
    const submitted = new Promise(resolve => { reached = resolve; });
    const send = command => {
      calls.push(command.kind);
      if (command.kind === held) reached();
      return {
        correlation: `c${calls.length}`,
        completion: command.kind === held ? pending : Promise.resolve(accepted({ epoch: "held" })),
      };
    };
    const opened = await createExecutionEpoch({ declarations: {
      begin: send, validate: send, commit: send, abort: send,
    } }, { kind: "BeginEpoch" });
    const epoch = opened.value;
    const completion = epoch.complete();
    await submitted;
    const aborted = await epoch.abort();
    finish(accepted({ epoch: "held" }));
    const result = await completion;

    assert.equal(epoch.complete(), completion);
    assert.deepEqual(epoch.trace.map(command => command.kind), calls);
    if (held === "ValidateEpoch") {
      assert.equal(aborted.status, "accepted");
      assert.equal(result.status, "rejected");
      assert.equal(result.reason, "Invalid");
      assert.equal(epoch.state, "aborted");
      assert.deepEqual(calls, ["BeginEpoch", "ValidateEpoch", "AbortEpoch"]);
    } else {
      assert.equal(aborted.status, "rejected");
      assert.equal(aborted.reason, "Invalid");
      assert.equal(result.status, "accepted");
      assert.equal(epoch.state, "committed");
      assert.deepEqual(calls, ["BeginEpoch", "ValidateEpoch", "CommitEpoch"]);
    }
  });
}
