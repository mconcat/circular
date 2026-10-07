/**
 * Host-adapter primitive for one implicit Circular code-execution epoch.
 *
 * This is intentionally not an authored mutation API. Agent/user programs call
 * fluent SDK operations; the outer code host owns this object and installs its
 * synchronous `emit` function as the command sink while the module evaluates.
 */

import { EpochLifetime, rejected } from "./epoch-lifetime.js";
import { CommitUnanswered, commitUnknown } from "./commit-outcome.js";

function accepted(value) {
  return Object.freeze({ status: "accepted", value });
}

function normalizeCompletion(value, phase) {
  if (value !== null && typeof value === "object" &&
      (value.status === "accepted" || value.status === "rejected")) {
    return value;
  }
  return rejected("Malformed", `${phase} completion did not yield a Circular Result.`);
}

function unavailableCompletion(phase) {
  return rejected("Unavailable", `${phase} completion failed before yielding a Circular Result.`);
}

class ExecutionEpochImpl {
  #declarations;
  #epoch;
  #content = [];
  #requests = [];
  #trace;
  #lifetime;

  constructor(declarations, begin, epoch) {
    this.#declarations = declarations;
    this.#epoch = epoch;
    this.#trace = [begin];
    this.#lifetime = new EpochLifetime(kind => this.#request(kind), {
      unanswered: async (error) => (
        await declarations.recorded?.(begin.commitId) ?? commitUnknown(begin.commitId, error)
      ),
    });
  }

  get epoch() {
    return this.#epoch;
  }

  get state() {
    return this.#lifetime.state;
  }

  /** Exact content command objects emitted by the authored SDK. */
  get commands() {
    return Object.freeze([...this.#content]);
  }

  /** Exact existing declaration requests sent so far, including epoch brackets. */
  get trace() {
    return Object.freeze([...this.#trace]);
  }

  /**
   * Enqueues a command immediately and synchronously returns its correlation.
   * The completion is intentionally retained by the host rather than exposed to
   * the authored program, so fluent calls do not acquire `await`.
   */
  emit(command) {
    if (this.state !== "open") {
      throw new Error(`Cannot emit into a Circular execution epoch in state ${this.state}`);
    }
    const request = Object.freeze({ epoch: this.#epoch, command });
    this.#content.push(command);
    this.#trace.push(request);
    const correlated = this.#declarations.apply(request);
    this.#requests.push(correlated);
    return correlated.correlation;
  }

  complete() {
    return this.#lifetime.complete(async () => {
      const settled = await Promise.allSettled(this.#requests.map(request => (
        Promise.resolve().then(() => request.completion)
      )));
      return settled.map((outcome, index) => (
        outcome.status === "fulfilled"
          ? normalizeCompletion(outcome.value, `Apply[${index}]`)
          : unavailableCompletion(`Apply[${index}]`)
      )).find(result => result.status === "rejected") ?? null;
    });
  }

  abort(_reason) {
    return this.#lifetime.abort();
  }

  async #request(kind) {
    const command = Object.freeze({ kind, epoch: this.#epoch });
    this.#trace.push(command);
    try {
      return normalizeCompletion(
        await this.#declarations[kind.slice(0, -5).toLowerCase()](command).completion,
        kind,
      );
    } catch (error) {
      if (kind === "CommitEpoch") throw new CommitUnanswered(error);
      return unavailableCompletion(kind);
    }
  }
}

/**
 * Opens the epoch before authored module evaluation begins.
 * Returns a normal protocol rejection if the mandatory baseline cannot be opened.
 */
export async function createExecutionEpoch(session, begin) {
  let opened;
  try {
    opened = normalizeCompletion(
      await session.declarations.begin(begin).completion,
      "BeginEpoch",
    );
  } catch {
    opened = unavailableCompletion("BeginEpoch");
  }
  if (opened.status === "rejected") return opened;
  if (opened.value === null || typeof opened.value !== "object" || typeof opened.value.epoch !== "string") {
    return rejected("Malformed", "BeginEpoch was accepted without a valid EpochId.");
  }
  return accepted(new ExecutionEpochImpl(session.declarations, begin, opened.value.epoch));
}
