function random(seed) {
  let a = seed >>> 0;
  return () => {
    a = a + 0x6D2B79F5 >>> 0;
    let t = a;
    t = Math.imul(t ^ t >>> 15, t | 1);
    t ^= t + Math.imul(t ^ t >>> 7, t | 61);
    return ((t ^ t >>> 14) >>> 0) / 4294967296;
  };
}
const alphabet = 'abcdefghijklmnopqrstuvwxyz0123456789 #%.-가나';
export function generator(seed) {
  const r = random(seed), pick = list => list[Math.floor(r() * list.length)];
  const word = (min = 1) => Array.from({ length: min + Math.floor(r() * 8) }, () => pick(alphabet)).join('').trim() || 'w';
  const name = () => Array.from({ length: 1 + Math.floor(r() * 6) }, () => pick('abcdefghijklmnopqrstuvwxyz')).join('');
  const strings = [], names = [];
  const value = depth => {
    const kind = pick(depth > 2 ? ['text', 'bytes', 'number', 'bigint', 'uint', 'flag']
      : ['text', 'bytes', 'number', 'bigint', 'uint', 'flag', 'list', 'record', 'record']);
    if (kind === 'text') { const s = word(); strings.push(s); return s; }
    if (kind === 'bytes') { const s = word(); strings.push(s); return new TextEncoder().encode(s); }
    if (kind === 'number') return Math.floor(r() * 2000) / 4;
    if (kind === 'bigint') return BigInt(Math.floor(r() * 1e9)) * 1000003n;
    if (kind === 'uint') return Object.freeze({ value: BigInt(Math.floor(r() * 1e6)) });
    if (kind === 'flag') return pick([true, false, null]);
    if (kind === 'list') return Array.from({ length: Math.floor(r() * 4) }, () => value(depth + 1));
    const record = {};
    for (let i = Math.floor(r() * 4); i > 0; i--) {
      const n = name();
      if (Object.hasOwn(record, n)) continue;
      names.push(n); record[n] = value(depth + 1);
    }
    return record;
  };
  return () => { strings.length = 0; names.length = 0; const v = value(0); return { value: v, strings: [...strings], names: [...names] }; };
}
