import type { ActorFlags, ScopeRole } from "@circular/protocol";

/**
 * Explicit normalized authoring: identities and values are validated by the shared command codec.
 * These inputs also admit the byte codec's structured addresses, Int and Bytes carriers; the older
 * nominal protocol declarations do not yet describe those runtime carriers.
 */
export declare function declareActor(
  constructor: (...args: never[]) => unknown,
  actor: unknown, config: unknown, flags: ActorFlags, actorType: string,
): void;
/** Declares explicit endpoints, parallel-edge identity, and full normalized attrs without defaults. */
export declare function declareEdge(edge: unknown, from: unknown, to: unknown, ordinal: number, attrs: unknown): void;
/** Declares a complete concrete or template scope and its boundary. */
export declare function declareScope(scope: unknown, role: ScopeRole, boundary: unknown): void;
/** Moves the exact actor sequence without allocating new identities. */
export declare function moveToScope(actors: readonly unknown[], target: unknown): void;
/** Replaces all three flags. */
export declare function setFlags(actor: unknown, flags: ActorFlags): void;
