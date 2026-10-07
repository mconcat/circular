/**
 * Fixed `circular:current` namespace facade.
 *
 * The host installs a complete, immutable snapshot-backed namespace for the duration of synchronous
 * module evaluation. These methods never perform RPC and never lazily hydrate a field.
 */

import { activeCurrentNamespace } from "./current-context.js";

/** The single stable export of the `circular:current` virtual module. */
export const current = Object.freeze({
  actor(binding) {
    return activeCurrentNamespace().actor(binding);
  },
  edge(...args) {
    return activeCurrentNamespace().edge(...args);
  },
  scope(id) {
    return activeCurrentNamespace().scope(id);
  },
  export(name) {
    return activeCurrentNamespace().export(name);
  },
  annotation(id) {
    return activeCurrentNamespace().annotation(id);
  },
});
