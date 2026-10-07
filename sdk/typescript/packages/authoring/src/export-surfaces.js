import ts from 'typescript';
import * as builders from '@circular/exports';
import { surfaceValue } from '@circular/exports/internal';

import { EXPORT_MARK_NAMES as NAMES } from '@circular/generator/internal';

export function collectExportSurface(actor, imported, fail) {
  const unwrap = n => ts.isParenthesizedExpression(n) || ts.isAsExpression(n) || ts.isSatisfiesExpression(n) ? unwrap(n.expression) : n;
  const read = (input, refs = new Map()) => {
    const n = unwrap(input);
    if (ts.isStringLiteral(n) || ts.isNoSubstitutionTemplateLiteral(n)) return n.text;
    if (ts.isNumericLiteral(n)) return Number(n.text);
    if (n.kind === ts.SyntaxKind.TrueKeyword) return true;
    if (n.kind === ts.SyntaxKind.FalseKeyword) return false;
    if (n.kind === ts.SyntaxKind.NullKeyword) return null;
    if (ts.isPrefixUnaryExpression(n) && n.operator === ts.SyntaxKind.MinusToken && ts.isNumericLiteral(n.operand)) return -Number(n.operand.text);
    if (ts.isArrayLiteralExpression(n)) return n.elements.map(x => read(x, refs));
    if (ts.isObjectLiteralExpression(n)) {
      const pairs = n.properties.map(p => {
        if (!ts.isPropertyAssignment(p)) return fail(n);
        const key = ts.isComputedPropertyName(p.name) && ts.isStringLiteral(p.name.expression) ? p.name.expression.text : p.name.text;
        if (typeof key !== 'string') return fail(p);
        return [key, read(p.initializer, refs)];
      });
      if (new Set(pairs.map(p => p[0])).size !== pairs.length) return fail(n);
      return Object.fromEntries(pairs);
    }
    if (ts.isPropertyAccessExpression(n) || ts.isElementAccessExpression(n)) {
      const key = ts.isPropertyAccessExpression(n) ? n.name.text : ts.isStringLiteral(n.argumentExpression) ? n.argumentExpression.text : null;
      if (!ts.isIdentifier(n.expression) || !refs.has(n.expression.text) || key === null) return fail(n);
      const record = refs.get(n.expression.text);
      if (!Object.hasOwn(record, key)) return fail(n);
      return record[key];
    }
    if (ts.isCallExpression(n)) {
      const calleeRoot = ts.isPropertyAccessExpression(n.expression) ? n.expression.expression : n.expression;
      const shadowed = ts.isIdentifier(calleeRoot) && refs.has(calleeRoot.text);
      const name = shadowed ? null : imported(n.expression);
      if (NAMES.has(name)) {
        const [minimum, maximum] = ({ tab: [2, 2], window: [2, 2], grid: [1, 2], messages: [1, 1], terminal: [1, 1],
          label: [1, 1], transcript: [1, 2], textInput: [1, 1], button: [1, 2], toggle: [1, 2], select: [2, 2], prompt: [1, 2], composer: [1, 2] })[name];
        if (n.arguments.length < minimum || n.arguments.length > maximum) return fail(n);
        return builders[name](...n.arguments.map(a => read(a, refs)));
      }
      if (ts.isPropertyAccessExpression(n.expression) && ['cell', 'placeholder'].includes(n.expression.name.text)) {
        const value = read(n.expression.expression, refs), method = n.expression.name.text;
        if (n.arguments.length !== (method === 'cell' ? 4 : 1)) return fail(n);
        if (!value?.[Symbol.for('@circular/exports/mark')] || typeof value[method] !== 'function') return fail(n);
        return value[method](...n.arguments.map(a => read(a, refs)));
      }
    }
    return fail(n);
  };
  const expression = unwrap(actor);
  if (!ts.isCallExpression(expression) || imported(expression.expression) !== 'defineExport' || expression.arguments.length !== 1) return fail(actor);
  const specActor = unwrap(expression.arguments[0]);
  if (!ts.isObjectLiteralExpression(specActor)) return fail(specActor);
  const spec = Object.create(null);
  for (const p of specActor.properties) {
    if (!ts.isPropertyAssignment(p) || ts.isComputedPropertyName(p.name) || Object.hasOwn(spec, p.name.text)) return fail(p);
    if (p.name.text !== 'surfaces') { spec[p.name.text] = read(p.initializer); continue; }
    const value = unwrap(p.initializer);
    if (!ts.isArrowFunction(value)) { spec.surfaces = read(value); continue; }
    if (value.modifiers?.length || value.parameters.length > 2 || value.parameters.some(p => !ts.isIdentifier(p.name) || p.initializer || p.dotDotDotToken)
      || new Set(value.parameters.map(p => p.name.text)).size !== value.parameters.length || ts.isBlock(value.body)) return fail(value);
    spec.surfaces = (roles, params) => read(value.body, new Map(value.parameters.map((p, i) => [p.name.text, [roles, params][i]])));
  }
  return surfaceValue(builders.defineExport(spec));
}

/** Accumulate one complete Export; the shared declaration codec carries its optional surface Value. */
export function createExportCollector(scope, fail) {
  const mounts = new Map();
  const get = (activeScope, name) => {
    const id = JSON.stringify([activeScope, name]);
    if (!mounts.has(id)) mounts.set(id, { kind: 'UpsertExportMount', mount: {
      arm: 'epochLocal', value: { scope: [...scope, ...activeScope], local: name } }, declaration: { roles: {} } });
    return mounts.get(id).declaration;
  };
  return {
    role(activeScope, name, role, endpoint) {
      const d = get(activeScope, name);
      if (Object.hasOwn(d.roles, role)) return fail('authoring.export.role-bound-twice');
      d.roles[role] = endpoint;
    },
    surface(activeScope, name, value) {
      const d = get(activeScope, name);
      if (Object.hasOwn(d, 'surface')) return fail('authoring.export.surface-bound-twice');
      d.surface = value;
      if (value.operations !== null) d.operations = value.operations;
    },
    commands() {
      for (const { declaration: d } of mounts.values()) if (d.surface
        && JSON.stringify(Object.keys(d.roles).sort()) !== JSON.stringify(Object.keys(d.surface.roles).sort())) fail('authoring.export.surface-role-mismatch');
      return [...mounts.values()];
    },
  };
}
