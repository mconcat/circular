function span(text, head, from) {
  for (let at = text.indexOf(head, from); at >= 0; at = text.indexOf(head, at + 1)) {
    const start = text.lastIndexOf('\n', at - 1) + 1;
    if (text.slice(start, at).trim() !== '') continue;
    const indent = text.slice(start).search(/\S/);
    let end = text.indexOf('\n', at);
    if (end < 0) return [start, text.length];
    for (let next = end + 1; next < text.length;) {
      let stop = text.indexOf('\n', next);
      if (stop < 0) stop = text.length;
      const line = text.slice(next, stop), depth = line.search(/\S/);
      if (depth >= 0) {
        if (depth < indent || (depth === indent && !/[)\]}]/.test(line[depth]))) break;
        end = stop;
      }
      next = stop + 1;
    }
    return [start, end];
  }
  throw new Error(`no statement begins with ${JSON.stringify(head)}`);
}

export const unit = (text, head) => text.slice(...span(text, head, 0));

export function units(text, ...heads) {
  let cursor = 0;
  return heads.map(head => {
    const [start, end] = span(text, head, cursor);
    cursor = end;
    return text.slice(start, end);
  }).join('\n');
}
