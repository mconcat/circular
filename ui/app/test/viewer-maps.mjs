import { viewer } from '../renderer/viewer.mjs';

const part = (read, write) => ({
  get: id => read(viewer.card(id)),
  has: id => read(viewer.card(id)) !== undefined,
  set(id, value) { write(id, value); return this; },
  delete(id) { const had = this.has(id); write(id, undefined); return had; },
});

export const viewerMaps = () => ({
  drafts: part(entry => entry.draft?.shown, (id, value) => viewer.draft(id, { shown: value })),
  rawDrafts: part(entry => entry.draft?.raw, (id, value) => viewer.draft(id, { raw: value })),
  submissions: part(entry => entry.submission, (id, value) => viewer.change(id, { submission: value })),
  modes: part(entry => entry.mode, (id, value) => viewer.setMode(id, value)),
});
