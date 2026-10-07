import { reason, healthState } from './reasons.mjs';

export function showBanner(draw, code, kind) {
  const value = reason(code);
  draw?.(value.label, kind, value.code);
  return value;
}

const reasonCode = node => node.health?.reason?.code ?? node.health?.reason?.value ?? node.health?.reason ?? null;

export function newReasons(before, after) {
  const was = new Map(before.map(n => [n.id, reasonCode(n)]));
  return after.flatMap(n => {
    const code = reasonCode(n);
    if (code == null || String(was.get(n.id)) === String(code)) return [];
    return [{ actor: n.id, code: String(code), kind: healthState(n.health?.state) === 'dead' ? 'failed' : 'pending' }];
  });
}

export function holds(fact, nodes) {
  const node = nodes?.find(n => n.id === fact.actor);
  return node?.health == null || String(reasonCode(node)) === fact.code;
}
