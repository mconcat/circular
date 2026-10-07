import type { Transport } from "@circular/protocol";
export { OWNER_LOCAL_SOCKET_NAME, OWNER_LOCAL_RESOURCE_CEILINGS } from './index.js';

/** A failure at the OwnerLocal boundary, before or during the byte stream. */
export declare class OwnerLocalTransportError extends Error {
  /** Names the exact refusal or failure. */
  readonly code: string;
}

/** Splits a byte stream into complete frames without assuming chunk boundaries. */
export declare class FrameReader {
  /** Bounds a declared frame length before anything is allocated for it. */
  constructor(maximumFrameBytes: number);
  /** Adds bytes and returns every complete frame they finished. */
  push(chunk: Uint8Array): readonly Uint8Array[];
  /** Bytes held that do not yet complete a frame. */
  readonly pending: number;
}

/** How many complete frames may wait before the transport stops reading. */
export declare const DEFAULT_MAXIMUM_PENDING_FRAMES: number;

/** Names one OwnerLocal endpoint and the frame bound its caller admits. */
export interface OwnerLocalOptions {
  /** The absolute, owner-only `0700` directory holding the socket. */
  readonly root: string;
  /** The socket's single path component inside that root. */
  readonly socketName: string;
  /** The largest frame this caller will buffer; the wire's declared length is bounded by it. */
  readonly maximumFrameBytes: number;
  /** How many complete frames may wait unread before reading pauses. */
  readonly maximumPendingFrames?: number;
}

/** Verifies the endpoint's ownership and mode, connects, and returns a frame transport. */
export declare function connectOwnerLocal(
  options: OwnerLocalOptions,
): Promise<Transport<Uint8Array>>;

/**
 * The part of a connected byte stream this transport uses.
 *
 * Declared structurally rather than as an Actor socket type so that the framing can be driven
 * over any stream pair, which is what lets it be tested without a socket.
 */
export interface ByteStream {
  /** Registers a listener for `data`, `error`, or `close`. */
  on(event: string, listener: (...args: readonly never[]) => void): unknown;
  /** Stops reading, so a backlog accumulates on the stream rather than in memory. */
  pause(): unknown;
  /** Resumes reading once the backlog has drained. */
  resume(): unknown;
  /** Writes bytes, reporting completion or failure to the callback. */
  write(bytes: Uint8Array, callback: (error?: Error | null) => void): unknown;
  /** Ends the stream, calling back when it has closed. */
  end(callback: () => void): unknown;
  /** Discards the stream without waiting for it to drain. */
  destroy(): unknown;
}

/** Wraps an already-connected byte stream as a frame transport. */
export declare function frameTransport(
  socket: ByteStream,
  maximumFrameBytes: number,
  options?: { readonly maximumPendingFrames?: number },
): Transport<Uint8Array>;
