/** The framing version this profile emits and accepts. */
export declare const OWNER_LOCAL_FRAMING_VERSION: number;

/** `version:u8 · channel:u32be · operation:u8 · segment:u8 · body_len:u32be`. */
export declare const OWNER_LOCAL_FRAME_HEADER_BYTES: number;

/** The largest opaque body one frame carries. */
export declare const OWNER_LOCAL_MAX_FRAME_BODY_BYTES: number;

/** The largest reassembled message. */
export declare const OWNER_LOCAL_MAX_MESSAGE_BYTES: number;

/** Frame operations; only `data` may be fragmented. */
export declare const FRAME_OPERATIONS: Readonly<Record<string, number>>;

/** Segment tags: one whole frame, or a first/middle/last sequence. */
export declare const FRAME_SEGMENTS: Readonly<Record<string, number>>;

/** A refusal at the transport frame layer. */
export declare class CanonicalFrameError extends Error {
  /** Names the exact refusal. */
  readonly code: string;
}

/** One frame's bytes. */
export interface CanonicalFrameParts {
  readonly channel: number;
  readonly operation: number;
  readonly segment: number;
  readonly body: Uint8Array;
}

/** Encodes one frame. */
export declare function encodeCanonicalFrame(parts: CanonicalFrameParts): Uint8Array;

/** Bounds a message and splits it into the frames that carry it. */
export declare function encodeMessageFrames(
  channel: number,
  message: Uint8Array,
  options?: { readonly maximumMessageBytes?: number },
): readonly Uint8Array[];

/** Reads a byte stream into complete messages. */
export declare class OwnerLocalFrameReader {
  constructor(options?: { readonly maximumMessageBytes?: number });
  /** Adds bytes and returns every message they completed. */
  push(chunk: Uint8Array): readonly Uint8Array[];
  /** Bytes held that do not yet complete a frame. */
  readonly pending: number;
  /** Whether a message is half-arrived. */
  readonly collecting: boolean;
}
