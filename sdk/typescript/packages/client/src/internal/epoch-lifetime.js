import { CommitUnanswered } from "./commit-outcome.js";

/** One epoch owns completion, cancellation and its terminal request. */
export function rejected(reason, message) {
  return Object.freeze({
    status: "rejected",
    reason,
    diagnostics: Object.freeze([
      Object.freeze({ code: 0, message, hint: null, at: null }),
    ]),
  });
}

export class EpochLifetime {
  #request;
  #unanswered;
  #state = "open";
  #completion = null;
  #abortion = null;

  /**
   * `unanswered(error)` settles a CommitEpoch that was submitted and never answered — the
   * request threw {@link CommitUnanswered}. It answers from the durable commit record: the
   * accepted value, or an `unknown` outcome. Without it the error propagates as before.
   */
  constructor(request, { unanswered } = {}) {
    this.#request = request;
    this.#unanswered = unanswered ?? null;
  }

  get state() {
    if (this.#state === "committing") return "completing";
    if (this.#state.endsWith("-rejected")) return "rejected";
    if (this.#state === "commit-unknown") return "unknown";
    return this.#state;
  }

  complete(prepare, { commit = true, beforeCommit } = {}) {
    if (this.#completion !== null) return this.#completion;
    if (this.#state !== "open") {
      return Promise.resolve(rejected("Invalid", `Cannot complete an epoch in state ${this.state}.`));
    }
    this.#state = "completing";
    this.#completion = this.#completeOnce(prepare, commit, beforeCommit);
    return this.#completion;
  }

  async #completeOnce(prepare, commit, beforeCommit) {
    try {
      const applied = await prepare();
      if (applied?.status === "rejected") {
        await this.abort();
        return applied;
      }
      if (this.#abortion !== null) return this.#superseded();
      const validated = await this.#request("ValidateEpoch");
      if (validated.status === "rejected") {
        await this.abort();
        return validated;
      }
      if (this.#abortion !== null) return this.#superseded();
      if (!commit) return await this.abort();
      if (beforeCommit !== undefined) await beforeCommit();
      if (this.#abortion !== null) return this.#superseded();
      return await this.#terminate("CommitEpoch");
    } catch (error) {
      if (this.#state === "open" || this.#state === "completing") {
        try {
          await this.abort();
        } catch (abortError) {
          error.message = `${error.message}; candidate abort also failed: ${abortError.message}`;
        }
      }
      throw error;
    }
  }

  abort() {
    if (this.#abortion !== null) return this.#abortion;
    if (this.#state === "committed") {
      return Promise.resolve(rejected("Invalid", "A committed Circular epoch cannot be aborted."));
    }
    if (this.#state === "committing" || this.#state === "commit-rejected" || this.#state === "commit-unknown") {
      return Promise.resolve(rejected(
        "Invalid",
        "CommitEpoch has already been submitted; reconcile its outcome by CommitId instead of aborting.",
      ));
    }
    this.#abortion = this.#terminate("AbortEpoch");
    return this.#abortion;
  }

  async #terminate(kind) {
    const commit = kind === "CommitEpoch";
    this.#state = commit ? "committing" : "aborting";
    let result;
    try {
      result = await this.#request(kind);
    } catch (error) {
      if (!commit || !(error instanceof CommitUnanswered) || this.#unanswered === null) throw error;
      result = await this.#unanswered(error);
    }
    this.#state = result.status === "accepted"
      ? (commit ? "committed" : "aborted")
      : result.status === "unknown" && commit
        ? "commit-unknown"
        : (commit ? "commit-rejected" : "abort-rejected");
    return result;
  }

  async #superseded() {
    await this.#abortion;
    return rejected("Invalid", "Epoch completion was superseded by an explicit abort.");
  }
}
