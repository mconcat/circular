const namespaceStack = [];

export function activeCurrentNamespace() {
  const namespace = namespaceStack.at(-1);
  if (namespace === undefined) {
    throw new Error("circular:current is only available while an authoring program is executing");
  }
  return namespace;
}

/** @internal Host-only synchronous dynamic binding used by the virtual-module loader. */
export function runWithCurrentNamespace(namespace, callback) {
  if (namespace === null || typeof namespace !== "object") {
    throw new TypeError("current namespace must be a complete local object");
  }
  if (typeof callback !== "function") {
    throw new TypeError("current namespace callback must be a function");
  }

  namespaceStack.push(namespace);
  try {
    const value = callback();
    if (value !== null && typeof value === "object" && typeof value.then === "function") {
      throw new TypeError("authoring module evaluation must be synchronous");
    }
    return value;
  } finally {
    namespaceStack.pop();
  }
}
