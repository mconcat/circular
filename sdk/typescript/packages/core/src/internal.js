import { joinConfigIssue } from "./join-config.js";
import { assembleConfigIssue } from "./assemble-config.js";
import {
  createCurrentEdgeHandleFromDescriptor,
  createCurrentActorHandleFromDescriptor,
} from "./runtime.js";

const CONTEXT_STACK = [];
const HANDLE_METADATA = new WeakMap();
const ENDPOINT_METADATA = new WeakMap();
const REFERENCE_KINDS = new Set(["actor", "edge", "scope", "export", "annotation"]);

function circularError(code, message) {
  const error = new Error(message);
  error.code = code;
  return error;
}

/** Runs a strictly synchronous authored callback inside one nested-safe execution context. */
export function runWithExecutionContext(context, callback) {
  if (!context || typeof context !== "object") throw new TypeError("Circular execution context is required");
  if (typeof context.allocateReference !== "function") throw new TypeError("context.allocateReference(kind) is required");
  if (typeof context.emit !== "function") throw new TypeError("context.emit(command) is required");
  if (typeof context.resolveActor !== "function") throw new TypeError("context.resolveActor(spelling, config) is required");
  if (typeof context.resolveEdge !== "function") throw new TypeError("context.resolveEdge(from, to, options) is required");
  if (typeof callback !== "function") throw new TypeError("Circular execution callback must be a function");
  CONTEXT_STACK.push(context);
  try {
    const result = callback();
    if (result && typeof result.then === "function") {
      throw circularError(
        "CIRCULAR_ASYNC_AUTHORED_CALLBACK",
        "The core execution context is synchronous; the host must await outside authored SDK evaluation",
      );
    }
    return result;
  } finally {
    if (CONTEXT_STACK.pop() !== context) {
      CONTEXT_STACK.length = 0;
      throw circularError("CIRCULAR_CONTEXT_STACK_CORRUPTED", "Circular execution context stack was corrupted");
    }
  }
}

/** Returns the active context or fails before a partial command can be emitted. */
export function getExecutionContext() {
  const context = CONTEXT_STACK.at(-1);
  if (!context) {
    throw circularError(
      "CIRCULAR_NO_EXECUTION_CONTEXT",
      "Circular constructors may only run inside an authoring execution epoch",
    );
  }
  return context;
}

/** Synchronously forwards one existing protocol content command to the active host sink. */
export function emitDeclaration(command) {
  if (!command || typeof command !== "object" || typeof command.kind !== "string") {
    throw new TypeError("emitDeclaration requires a protocol declaration command");
  }
  const result = getExecutionContext().emit(command);
  if (result && typeof result.then === "function") {
    throw circularError(
      "CIRCULAR_ASYNC_COMMAND_SINK",
      "context.emit(command) must enqueue synchronously and return void",
    );
  }
}

/** Allocates one epoch-local symbolic reference from the host-owned namespace. */
export function allocateReference(kind, declaration) {
  if (!REFERENCE_KINDS.has(kind)) throw new TypeError(`Unknown Circular reference kind: ${String(kind)}`);
  const reference = getExecutionContext().allocateReference(kind, declaration);
  if (!((typeof reference === "string" && reference.length > 0)
    || (reference && ["epochLocal", "absolute"].includes(reference.arm) && reference.value))) {
    throw new TypeError(`context.allocateReference(${kind}) must return a symbolic reference or declaration address`);
  }
  return reference;
}

/** Resolves one public constructor through the host's pinned provider/spec environment. */
export function resolveActor(spelling, config) {
  if (spelling === "assemble") {
    const issue = assembleConfigIssue(config);
    if (issue) throw new TypeError(issue);
  }
  if (spelling === "join") {
    const issue = joinConfigIssue(config);
    if (issue) throw new TypeError(issue);
  }

  const resolved = getExecutionContext().resolveActor(spelling, config);
  if (!resolved || typeof resolved !== "object" || !resolved.declaration || !resolved.ports) {
    throw circularError(
      "CIRCULAR_ACTOR_RESOLUTION_INVALID",
      `resolveActor(${String(spelling)}) must return { declaration, ports }`,
    );
  }
  return resolved;
}

/** Registers opaque actor metadata for other Circular package runtimes. */
export function registerActorHandle(handle, metadata) {
  HANDLE_METADATA.set(handle, metadata);
  return handle;
}

/** Registers opaque endpoint metadata for other Circular package runtimes. */
export function registerEndpoint(endpoint, metadata) {
  ENDPOINT_METADATA.set(endpoint, metadata);
  return endpoint;
}

/** Returns internal actor metadata to package runtimes without exposing it on authored values. */
export function metadataOf(handle) {
  return HANDLE_METADATA.get(handle);
}

function portFrom(metadata, direction, name) {
  const ports = metadata.ports;
  if (name === undefined) {
    const selected = direction === "source" ? ports.defaultOutput : ports.defaultInput;
    if (selected === undefined || selected === null) {
      throw circularError(
        direction === "source" ? "CIRCULAR_NO_DEFAULT_OUTPUT" : "CIRCULAR_NO_DEFAULT_INPUT",
        `${metadata.spelling} has no unambiguous default ${direction === "source" ? "output" : "input"}`,
      );
    }
    return selected;
  }
  const resolver = direction === "source" ? ports.output : ports.input;
  const table = direction === "source" ? ports.outputs : ports.inputs;
  const selected = typeof resolver === "function" ? resolver(name) : table?.find(p => p.id === name)?.id;
  if (selected === undefined || selected === null) {
    throw circularError(
      "CIRCULAR_PORT_UNKNOWN",
      `${metadata.spelling} has no ${direction === "source" ? "output" : "input"} port ${String(name)}`,
    );
  }
  return selected;
}

/**
 * Converts a core handle/endpoint to the exact protocol endpoint shape.
 * Structural view/export packages use this instead of inspecting authored objects.
 */
export function endpointOf(value, direction = "source") {
  if (!["source", "target", "writable"].includes(direction)) {
    throw new TypeError(`Unknown endpoint direction: ${String(direction)}`);
  }
  const endpoint = ENDPOINT_METADATA.get(value);
  if (endpoint) {
    if (direction === "writable" && !endpoint.writable) {
      throw circularError("CIRCULAR_ENDPOINT_NOT_WRITABLE", "Only injection boundaries are writable export endpoints");
    }
    if (direction !== "writable" && endpoint.direction !== direction) {
      throw circularError("CIRCULAR_ENDPOINT_DIRECTION", `Expected a ${direction} endpoint`);
    }
    return Object.freeze({ actor: endpoint.actor, port: endpoint.port });
  }
  const actor = HANDLE_METADATA.get(value);
  if (!actor) throw circularError("CIRCULAR_NOT_AN_ENDPOINT", "Value is not a Circular actor or endpoint handle");
  if (direction === "writable" && !actor.writable) {
    throw circularError("CIRCULAR_ENDPOINT_NOT_WRITABLE", "Only injection boundaries are writable export endpoints");
  }
  const port = portFrom(actor, direction === "target" ? "target" : "source");
  return Object.freeze({ actor: actor.actor, port });
}

/** Returns the stable or epoch-local protocol actor address represented by a handle. */
export function actorOf(handle) {
  const metadata = HANDLE_METADATA.get(handle);
  if (!metadata) throw circularError("CIRCULAR_NOT_A_ACTOR", "Value is not a Circular actor handle");
  return metadata.actor;
}

/** Creates an anchored current actor handle using host-resolved snapshot metadata. */
export function createCurrentActorHandle(address, spelling) {
  const descriptor = address && typeof address === "object" && address.actor
    ? address
    : getExecutionContext().resolveCurrentActor?.(address, spelling);
  if (!descriptor) {
    throw circularError(
      "CIRCULAR_CURRENT_ACTOR_UNRESOLVED",
      "The active context cannot resolve this current actor address",
    );
  }
  return createCurrentActorHandleFromDescriptor(descriptor, spelling);
}

/** Creates an anchored current edge handle using host-resolved snapshot metadata. */
export function createCurrentEdgeHandle(address) {
  const descriptor = address && typeof address === "object" && address.edge
    ? address
    : getExecutionContext().resolveCurrentEdge?.(address);
  if (!descriptor) {
    throw circularError(
      "CIRCULAR_CURRENT_EDGE_UNRESOLVED",
      "The active context cannot resolve this current edge address",
    );
  }
  return createCurrentEdgeHandleFromDescriptor(descriptor);
}

export { COMBINATOR_NAMES } from "./combinators.js";
export { constructorSpelling, publicName } from "./runtime.js";
export { declarationRow } from "../../protocol/src/internal/declaration-rows.js";

export { joinConfigIssue, assembleConfigIssue };
