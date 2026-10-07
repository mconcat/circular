
export const silent = first => `(() => {
  const out = [];
  // The box an element clips its content to (its padding box, less a scroll bar), on screen.
  const clip = a => { const b = a.getBoundingClientRect(), k = a.offsetWidth ? b.width / a.offsetWidth : 1;
    const left = b.left + a.clientLeft * k, top = b.top + a.clientTop * k;
    return { left, top, right: left + a.clientWidth * k, bottom: top + a.clientHeight * k }; };
  const scrolls = a => { const s = getComputedStyle(a);
    return /auto|scroll/.test(s.overflowY) && s.scrollbarWidth !== 'none' && a.scrollHeight > a.clientHeight
      && a.offsetWidth - a.clientWidth - parseFloat(s.borderLeftWidth) - parseFloat(s.borderRightWidth) > 0; };
  for (const card of document.querySelectorAll('#nodes > article.node')) {
    const name = card.querySelector('.node-title').textContent, viewer = card.querySelector('.node-viewer'), lines = [];
    const walk = document.createTreeWalker(viewer, NodeFilter.SHOW_TEXT);
    for (let t; (t = walk.nextNode()); ) {
      const el = t.parentElement;
      if (!t.data.trim() || !el.getClientRects().length || getComputedStyle(el).visibility === 'hidden') continue;
      // A line's box is the element's line height about its glyphs' middle (a glyph may reach past its line), as wide
      // as the glyphs on it (the white space that ends a line hangs past it and draws nothing).
      const k = el.offsetWidth ? el.getBoundingClientRect().width / el.offsetWidth : 1, height = parseFloat(getComputedStyle(el).lineHeight) * k;
      // The block whose lines hold the text: an ellipsis marks the cut of that block's own lines only.
      let block = el;
      while (block !== viewer && getComputedStyle(block).display === 'inline') block = block.parentElement;
      const own = [];
      for (let i = 0; i < t.data.length; i++) {
        if (/\\s/.test(t.data[i])) { if (own.length) own.at(-1).text += t.data[i]; continue; }
        const range = document.createRange(); range.setStart(t, i); range.setEnd(t, i + 1);
        const g = [...range.getClientRects()].find(g => g.width > 0);
        if (!g) continue;
        const line = own.find(line => Math.abs(line.middle - (g.top + g.bottom) / 2) < 2);
        if (line) { line.right = Math.max(line.right, g.right); line.text += t.data[i]; continue; }
        const half = (Number.isFinite(height) ? Math.min(height, g.height) : g.height) / 2, middle = (g.top + g.bottom) / 2;
        own.push({ el, block, text: t.data[i], middle, right: g.right, top: middle - half, bottom: middle + half });
      }
      lines.push(...own);
    }
    const top = Math.min(...lines.map(line => line.top));
    for (const line of lines) {
      const cuts = [];
      // What of the line is still in sight inside the boxes passed so far.
      let sight = { top: line.top, bottom: line.bottom, right: line.right };
      for (let a = line.el; a !== card.parentElement && sight.top < sight.bottom; a = a.parentElement) {
        const s = getComputedStyle(a);
        if (s.overflowX === 'visible' && s.overflowY === 'visible') continue;
        const c = clip(a);
        cuts.push({ a, s, down: sight.bottom > c.bottom + 1 || sight.top < c.top - 1, across: sight.right > c.right + 1 });
        sight = { top: Math.max(sight.top, c.top), bottom: Math.min(sight.bottom, c.bottom), right: Math.min(sight.right, c.right) };
      }
      // At the names tier a line after the first row that a box cuts down is past the card's foot: not in sight.
      if (${first} && line.top > top + 2 && cuts.some(cut => cut.down)) continue;
      const at = a => (a.className || a.tagName).toString();
      for (const { a, s, down, across } of cuts) {
        if (down && !scrolls(a) && s.webkitLineClamp === 'none')
          out.push(\`\${name}: "\${line.text.slice(0, 32)}" cut down by \${at(a)}\`);
        if (across && !(s.textOverflow === 'ellipsis' && a === line.block))
          out.push(\`\${name}: "\${line.text.slice(0, 32)}" cut across by \${at(a)}\`);
      }
    }
  }
  return out;
})()`;

export const boxes = `(() => {
  const all = [...document.querySelectorAll('#nodes .node-viewer, #nodes .node-viewer *')];
  const named = el => el.closest('article.node').querySelector('.node-title').textContent + ' ' + (el.className || el.tagName).toString();
  const lines = el => { const tops = new Set();
    for (const t of [...el.childNodes].filter(node => node.nodeType === 3)) for (let i = 0; i < t.data.length; i++) {
      const range = document.createRange(); range.setStart(t, i); range.setEnd(t, i + 1);
      for (const g of range.getClientRects()) if (g.width > 0) tops.add(Math.round(g.top));
    }
    return tops.size; };
  return { sideways: all.filter(el => /auto|scroll/.test(getComputedStyle(el).overflowX) && el.scrollWidth > el.clientWidth + 1).map(named),
    readings: all.filter(el => el.getClientRects().length && el.textContent.trim() && getComputedStyle(el).textOverflow === 'ellipsis')
      .map(el => ({ at: named(el), text: el.textContent.trim().slice(0, 40), y: getComputedStyle(el).overflowY, x: getComputedStyle(el).overflowX,
        wide: el.scrollWidth > el.clientWidth + 1, lines: getComputedStyle(el).whiteSpace === 'nowrap' ? lines(el) : null })) };
})()`;
