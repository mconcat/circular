/** Explicit normalized structural authoring. The surrounding host owns the epoch. */
import { emitDeclaration } from "./internal.js";

/** Authors an exact, already normalized actor value using an SDK constructor's provenance. */
export function declareActor(constructor, actor, config, flags, actorType) {
  if (typeof constructor !== "function" || typeof actorType !== "string") {
    throw new TypeError("declareActor requires a resolved catalog constructor and its canonical type");
  }
  emitDeclaration({ kind: "UpsertActor", actor, declaration: { actorType, config, flags } });
}

export function declareEdge(edge, from, to, ordinal, attrs) {
  emitDeclaration({ kind: "UpsertEdge", edge, declaration: { from, to, ordinal, attrs } });
}

export function declareScope(scope, role, boundary) {
  emitDeclaration({ kind: "UpsertScope", scope, declaration: { role, boundary } });
}

export function moveToScope(actors, target) {
  emitDeclaration({ kind: "MoveToScope", actors, target });
}

export function setFlags(actor, flags) {
  emitDeclaration({ kind: "SetFlags", actor, flags });
}
