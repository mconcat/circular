/**
 * The fold row each keyed content verb writes, and the field that names the row: the daemon's own
 * keys (crates/engine/src/authoring_assembly/delta_rows.rs `place`). One table for every SDK reader;
 * readers keep no copy. SetFlags writes its actor's row (fold.rs), not a table of its own. A verb that
 * writes no keyed row (the epoch verbs, MoveToScope) has none: null.
 */
const ROWS = Object.freeze({
  UpsertActor: ['Actor', 'actor'], RetireActor: ['Actor', 'actor'], SetFlags: ['Actor', 'actor'],
  UpsertEdge: ['Edge', 'edge'], RetireEdge: ['Edge', 'edge'],
  UpsertScope: ['Scope', 'scope'], RetireScope: ['Scope', 'scope'],
  UpsertExportMount: ['ExportMount', 'mount'], RetireExportMount: ['ExportMount', 'mount'],
  UpsertAnnotation: ['Annotation', 'annotation'], RetireAnnotation: ['Annotation', 'annotation'],
  UpsertTemplate: ['Template', 'name'], RetireTemplate: ['Template', 'name'],
  SetPresentation: ['Presentation', 'owner'],
});

export function declarationRow(kind) {
  const row = Object.hasOwn(ROWS, kind) ? ROWS[kind] : null;
  return row && Object.freeze({ table: row[0], field: row[1] });
}
