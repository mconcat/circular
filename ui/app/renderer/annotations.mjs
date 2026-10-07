import { address } from './edit.mjs';

export const noteBody = (note, body) => ({ kind: 'UpsertAnnotation', annotation: address(note.address),
  declaration: { kind: note.kind, refs: note.references, body } });
export const retireNote = note => ({ kind: 'RetireAnnotation', annotation: address(note.address) });
export const createNote = (scope, local) => ({ kind: 'UpsertAnnotation', annotation: address({ scope, local }),
  declaration: { kind: 'Note', refs: [], body: '' } });

const NOTE = { width: 230, height: 100, stepX: 250, stepY: 120, clear: 5 };
export function placeNotes(notes, nodes, origin = { x: 24, y: 24 }, columns = 6) {
  const boxes = nodes.map(n => ({ x: n.x, y: n.y, width: n.width, height: n.height }));
  const free = (x, y, width, height) => !boxes.some(b => x + width > b.x - NOTE.clear && x < b.x + b.width + NOTE.clear
    && y + height > b.y - NOTE.clear && y < b.y + b.height + NOTE.clear);
  for (const note of notes.filter(n => n.presentation?.fixed)) boxes.push(note);
  for (const note of notes) {
    if (note.presentation?.fixed) continue;
    const width = note.width ?? NOTE.width, height = note.height ?? NOTE.height;
    let row = 0, column = 0;
    while (!free(origin.x + column * NOTE.stepX, origin.y + row * NOTE.stepY, width, height))
      if (++column === columns) { column = 0; row++; }
    note.x = origin.x + column * NOTE.stepX; note.y = origin.y + row * NOTE.stepY;
    boxes.push({ x: note.x, y: note.y, width, height });
  }
  return notes;
}
