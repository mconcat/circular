export const gestureKinds = Object.freeze([
  'createActor', 'retireActors', 'connect', 'retireEdge', 'moveActors', 'resize', 'alignTops', 'group',
  'configure', 'setFlag', 'rename', 'setView',
  'addStep', 'applyStep', 'moveStep', 'removeStep', 'inletSettings',
  'createNote', 'noteBody', 'retireNote', 'moveNote', 'resizeNote',
  'togglePause', 'inject', 'decide', 'undo', 'redo',
]);

export function performer(handlers) {
  const missing = gestureKinds.filter(kind => typeof handlers[kind] !== 'function');
  const extra = Object.keys(handlers).filter(kind => !gestureKinds.includes(kind));
  if (missing.length || extra.length) throw new TypeError(`gesture table: missing [${missing}] extra [${extra}]`);
  return gesture => {
    if (!gestureKinds.includes(gesture?.kind)) throw new TypeError(`no such gesture: ${gesture?.kind}`);
    return handlers[gesture.kind](gesture);
  };
}
