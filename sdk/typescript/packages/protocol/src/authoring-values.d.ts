import { DEFAULT_EDGE_ATTRS } from "@circular/protocol/declaration";

/** Wraps an existing identity in an absolute address without copying it. */
export declare function address<Value>(value: Value): { readonly arm: "absolute"; readonly value: Value };

/** A scope identity is a segment sequence, not a joined string. */
export declare function scope(...names: string[]): {
  readonly arm: "absolute";
  readonly value: { name: string }[];
};

export declare const FLAGS: { readonly bypass: false; readonly mute: false; readonly pause: false };

/** Derived from the single product edge default. */
export declare const EDGE_CAPACITY: typeof DEFAULT_EDGE_ATTRS.policy.capacity;
export declare const EDGE_ATTRS: typeof DEFAULT_EDGE_ATTRS;

export declare const PLACEHOLDER_ENVIRONMENT: {
  readonly declarationSchema: Uint8Array;
  readonly specSet: Uint8Array;
};
