import assert from 'node:assert/strict';
import test from 'node:test';
import * as core from '@circular/core';
import { constructorSpelling } from '@circular/core/internal';
import { createProviderBindingRegistry } from '@circular/specs';
import { DEFAULT_EDGE_ATTRS } from '@circular/protocol/declaration';
import { generateProgram } from '@circular/generator';

const seed = 0x0c1a0985;
function mulberry32(value) {
  return () => {
    value = (value + 0x6d2b79f5) | 0;
    let n = Math.imul(value ^ value >>> 15, 1 | value);
    n ^= n + Math.imul(n ^ n >>> 7, 61 | n);
    return ((n ^ n >>> 14) >>> 0) / 0x100000000;
  };
}
const random = mulberry32(seed);
const shuffle = values => {
  const result = [...values];
  for (let i = result.length - 1; i > 0; i--) {
    const j = Math.floor(random() * (i + 1));
    [result[i], result[j]] = [result[j], result[i]];
  }
  return result;
};
const bindings = createProviderBindingRegistry({ bindings: [{
  actorTypeId: 'tap', constructorExport: 'tap', importSpecifier: '@circular/core',
}] });
assert.equal(constructorSpelling(core.tap), 'tap');
const options = { bindings, sdkVersion: '0.1.0-dev', specSet: new Uint8Array(32) };
const key = local => ({ scope: [], local });
const actor = local => ({ kind: 'UpsertActor', actor: { arm: 'epochLocal', value: key(local) },
  declaration: { actorType: 'tap', config: null, flags: { bypass: false, mute: false, pause: false } } });
const edge = (from, to) => {
  const source = { actor: key(from), port: 'event' }, target = { actor: key(to), port: 'event' };
  return { kind: 'UpsertEdge', edge: { arm: 'epochLocal', value: { from: source, to: target, ordinal: 0 } },
    declaration: { from: source, to: target, ordinal: 0, attrs: DEFAULT_EDGE_ATTRS } };
};
const context = (label, graph) => `${label}; seed=0x${seed.toString(16)}; graph=${JSON.stringify(graph)}`;
function declarationOrder(graph, names = graph.names, wires = graph.wires) {
  const result = generateProgram([...names.map(actor), ...wires.map(([from, to]) => edge(from, to))], options);
  assert.equal(result.status, 'complete', `${context('generator rejected graph', graph)}; diagnostics=${JSON.stringify(result.diagnostics)}`);
  const source = new TextDecoder().decode(result.value.program.modules.get(result.value.program.entry));
  return [...source.matchAll(/^export let (\w+) = /gm)].map(match => match[1]);
}
function checkGraph(graph, label, acyclicFailures, permute = true) {
  const order = declarationOrder(graph);
  assert.deepEqual([...order].sort(), [...graph.names].sort(), context(`${label}: every actor exactly once`, graph));
  if (permute) {
    const permuted = declarationOrder(graph, shuffle(graph.names), shuffle(graph.wires));
    assert.deepEqual(permuted, order, context(`${label}: declaration order depends on input order`, graph));
  }
  if (acyclicFailures) {
    const position = new Map(order.map((name, index) => [name, index]));
    for (const [from, to] of graph.wires) {
      if (position.get(from) >= position.get(to)) {
        acyclicFailures.push({ graph, order, wire: [from, to] });
        break;
      }
    }
  }
}
const namesOf = count => Array.from({ length: count }, (_, i) => `a${String(i).padStart(2, '0')}`);

test('property: every acyclic wire declares its source first (64 exhaustive + 1000 seeded graphs)', () => {
  const failures = [];
  const names = namesOf(4), possible = names.flatMap((from, i) => names.slice(i + 1).map(to => [from, to]));
  for (let mask = 0; mask < 1 << possible.length; mask++) {
    checkGraph({ names, wires: possible.filter((_, i) => mask & 1 << i) }, `four-actor DAG mask=${mask}`, failures);
  }
  for (let caseIndex = 0; caseIndex < 1000; caseIndex++) {
    const names = namesOf(1 + Math.floor(random() * 30));
    const hiddenTopologicalOrder = shuffle(names);
    const density = 0.03 + random() * 0.12;
    const wires = hiddenTopologicalOrder.flatMap((from, i) =>
      hiddenTopologicalOrder.slice(i + 1).filter(() => random() < density).map(to => [from, to]));
    checkGraph({ names, wires }, `random DAG case=${caseIndex}`, failures, caseIndex % 5 === 0);
  }
  assert.equal(failures.length, 0,
    `acyclic upstream violations=${failures.length}; ${context('first failing graph', failures[0]?.graph ?? null)}; order=${JSON.stringify(failures[0]?.order)}; wire=${JSON.stringify(failures[0]?.wire)}`);
});

test('property: cycles, self-wires, fan-in, fan-out and disconnected parts terminate and are stable (512 exhaustive + 1000 seeded graphs)', () => {
  const names = namesOf(3), possible = names.flatMap(from => names.map(to => [from, to]));
  for (let mask = 0; mask < 1 << possible.length; mask++) {
    checkGraph({ names, wires: possible.filter((_, i) => mask & 1 << i) }, `three-actor graph mask=${mask}`, null, mask % 8 === 0);
  }
  for (let caseIndex = 0; caseIndex < 1000; caseIndex++) {
    const names = namesOf(1 + Math.floor(random() * 30));
    const density = 0.02 + random() * 0.08;
    const wires = names.flatMap(from => names.filter(() => random() < density).map(to => [from, to]));
    checkGraph({ names, wires }, `random general graph case=${caseIndex}`, null, caseIndex % 5 === 0);
  }
});
