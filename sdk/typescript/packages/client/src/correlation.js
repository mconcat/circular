
/** The ceiling: the number of correlations alive at once, not the number ever issued. */
export const MAXIMUM_LIVE_CORRELATIONS = 4096;

export class CorrelationPoolError extends Error {
  constructor(code, message) {
    super(`${code}: ${message}`);
    this.code = code;
  }
}

function fail(code, message) {
  throw new CorrelationPoolError(code, message);
}

export class CorrelationPool {
  #bySlot = new Map();
  #byCorrelation = new Map();
  #nextSlot = 0;
  #maximumLive;

  constructor(options = {}) {
    const maximumLive = options.maximumLive ?? MAXIMUM_LIVE_CORRELATIONS;
    if (!Number.isSafeInteger(maximumLive) || maximumLive <= 0) {
      fail("MAXIMUM_LIVE_INVALID", "maximumLive must be a positive integer");
    }
    this.#maximumLive = maximumLive;
  }

  /** How many slots are held right now. */
  get live() {
    return this.#bySlot.size;
  }

  /** The ceiling this pool enforces. */
  get maximumLive() {
    return this.#maximumLive;
  }

  /**
   * Takes a slot for one exchange.
   *
   * Slot `0` is a real slot. Only the three tag positions exclude zero, and they do so because a
   * zero-filled buffer must not read as a live head; the correlation sits behind four bytes that
   * already refuse it, so the same argument does not reach here. Reserving it anyway would make
   * the pool 4095 wide for nothing.
   */
  allocate(correlationId) {
    if (typeof correlationId !== "string" || correlationId.length === 0) {
      fail("CORRELATION_ID_INVALID", "a correlation identity must be a non-empty string");
    }
    if (this.#byCorrelation.has(correlationId)) {
      fail("CORRELATION_ALREADY_LIVE", `${correlationId} already holds slot ${this.#byCorrelation.get(correlationId)}`);
    }
    if (this.#bySlot.size >= this.#maximumLive) {
      fail("EXHAUSTED", `all ${this.#maximumLive} correlation slots are live`);
    }

    for (let attempt = 0; attempt < this.#maximumLive; attempt += 1) {
      const slot = (this.#nextSlot + attempt) % this.#maximumLive;
      if (this.#bySlot.has(slot)) continue;
      this.#nextSlot = (slot + 1) % this.#maximumLive;
      this.#bySlot.set(slot, correlationId);
      this.#byCorrelation.set(correlationId, slot);
      return slot;
    }
    fail("EXHAUSTED", `all ${this.#maximumLive} correlation slots are live`);
    return undefined;
  }

  /** The slot held by one exchange, or `undefined` if it holds none. */
  slotFor(correlationId) {
    return this.#byCorrelation.get(correlationId);
  }

  /** The exchange holding one slot, or `undefined` if the slot is free. */
  correlationFor(slot) {
    return this.#bySlot.get(slot);
  }

  release(slot) {
    const correlationId = this.#bySlot.get(slot);
    if (correlationId === undefined) fail("SLOT_NOT_LIVE", `slot ${slot} is not held`);
    this.#bySlot.delete(slot);
    this.#byCorrelation.delete(correlationId);
    return correlationId;
  }

  /** Returns the slot held by one exchange, by its local identity. */
  releaseCorrelation(correlationId) {
    const slot = this.#byCorrelation.get(correlationId);
    if (slot === undefined) fail("CORRELATION_NOT_LIVE", `${correlationId} holds no slot`);
    this.release(slot);
    return slot;
  }

  releaseAll() {
    const released = [...this.#bySlot.keys()];
    this.#bySlot.clear();
    this.#byCorrelation.clear();
    return released;
  }
}

export const EXCHANGE_ROLE = Object.freeze({
  Hello: "opens",
  HelloAck: "closes",
  Goodbye: "none",

  BeginEpoch: "opens",
  ValidateEpoch: "opens",
  CommitEpoch: "opens",
  AbortEpoch: "opens",
  UpsertActor: "opens",
  RetireActor: "opens",
  UpsertEdge: "opens",
  RetireEdge: "opens",
  UpsertScope: "opens",
  RetireScope: "opens",
  MoveToScope: "opens",
  UpsertExportMount: "opens",
  RetireExportMount: "opens",
  UpsertAnnotation: "opens",
  RetireAnnotation: "opens",
  SetPresentation: "opens",
  SetFlags: "opens",
  UpsertTemplate: "opens",
  RetireTemplate: "opens",
  CommandResult: "closes",

  Query: "opens",
  QueryResult: "closes",
  QueryClose: "continues",

  Subscribe: "opens",
  SubscribeAck: "continues",
  Credit: "continues",
  Unsubscribe: "continues",
  Frame: "continues",
  SubscriptionEnded: "closes",

  Inject: "opens",
  InjectAck: "closes",

  ApprovalDecide: "opens",
  SetObservationControl: "opens",
  TransitionResult: "closes",
  SetAgentHarness: "opens",

  ReplayStart: "opens",
  ReplayRewind: "opens",
  ReplayEnd: "opens",
  ReplayResult: "closes",

  Resume: "opens",
  Pause: "opens",
  LifecycleResult: "closes",
});

/**
 * Whether an arriving envelope ends the exchange on its correlation.
 *
 * Takes the verb of what **arrived**, never of what was sent. A subscription can end without the
 * client asking — `ScopeGone` and `ResetRequired` are server-initiated — so a pool that released
 * on its own sends would leak every slot the server closed.
 */
export function terminatesExchange(verb) {
  const role = EXCHANGE_ROLE[verb];
  if (role === undefined) fail("VERB_UNKNOWN", `${verb} has no recorded exchange disposition`);
  return role === "closes";
}

/** Whether sending this verb should take a slot. */
export function opensExchange(verb) {
  const role = EXCHANGE_ROLE[verb];
  if (role === undefined) fail("VERB_UNKNOWN", `${verb} has no recorded exchange disposition`);
  return role === "opens";
}
