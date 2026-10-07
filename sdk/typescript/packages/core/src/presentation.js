import { flowColumn, flowGrid, flowRow } from "./runtime.js";

/** Emits only constraints representable by existing SetPresentation commands. */
export const flow = Object.freeze({
  row: flowRow,
  column: flowColumn,
  grid: flowGrid,
});
