export function cardViewer() {
  const cards = new Map();
  const none = Object.freeze({});
  const entry = id => cards.get(id) ?? none;
  const parts = (held, next) => {
    const value = { ...held, ...next };
    for (const [name, part] of Object.entries(value)) if (part === undefined) delete value[name];
    return Object.keys(value).length ? Object.freeze(value) : undefined;
  };
  function change(id, next) {
    const value = parts(entry(id), next);
    if (value) cards.set(id, value);
    else cards.delete(id);
  }
  return {
    card: entry,
    change,
    setMode: (id, interactive) => change(id, { mode: interactive }),
    hold: (id, kind, at) => change(id, { preview: parts(entry(id).preview, { [kind]: at }) }),
    release(id, kind, at) {
      const held = entry(id).preview?.[kind];
      if (held === undefined || (at !== undefined && held !== at)) return;
      change(id, { preview: parts(entry(id).preview, { [kind]: undefined }) });
    },
    previews: () => [...cards].filter(([, value]) => value.preview).map(([id, value]) => [id, value.preview]),
    draft: (id, next) => change(id, { draft: parts(entry(id).draft, next) }),
    answer: (id, kind, answer) => change(id, { answers: parts(entry(id).answers, { [kind]: answer }) }),
    message: (id, text) => change(id, { message: text ? text : undefined }),
    keep(holds) {
      for (const id of [...cards.keys()]) if (!holds(id)) cards.delete(id);
    },
    clear(...names) {
      if (!names.length) { cards.clear(); return; }
      for (const id of [...cards.keys()]) change(id, Object.fromEntries(names.map(name => [name, undefined])));
    },
  };
}

export const viewer = cardViewer();
