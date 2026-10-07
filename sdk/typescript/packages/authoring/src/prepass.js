import { bindingIdentifier, replicatorPolicy } from '@circular/generator/internal';
import { joinConfigIssue } from "../../core/src/join-config.js";
import { lowerConfig } from "../../core/src/form-config.js";
import { flattenConfigIssue } from "../../protocol/src/flatten-config.js";
import { assembleConfigIssue } from "../../core/src/assemble-config.js";
import { collectTemplateFunctions, templateScopePlan } from './template-functions.js';
import ts from 'typescript';
import { lowerCallback, callbackSourceSegments } from './callback-expression.js';
import { Buffer } from 'node:buffer';
import { collectExportSurface } from './export-surfaces.js';
import * as core from '@circular/core';
import * as exportsModule from '@circular/exports';
import { constructorSpelling, COMBINATOR_NAMES } from '@circular/core/internal';

const prepared = new WeakMap();
export function isPreparedProgram(value) {
  const sealed = prepared.get(value);
  if (!sealed || value.executable.modules.size !== sealed.modules.size
    || value.source.entry !== sealed.entry || value.source.modules.size !== sealed.source.size) return false;
  for (const [path, bytes] of sealed.source) {
    const source = value.source.modules.get(path);
    const actual = typeof source === 'string' ? new TextEncoder().encode(source) : source;
    if (!(actual instanceof Uint8Array) || actual.length !== bytes.length || !actual.every((b, i) => b === bytes[i])) return false;
  }
  return sealed.literalBytes.every(([actual, bytes]) => actual.length === bytes.length && actual.every((byte, i) => byte === bytes[i]))
    && [...sealed.modules].every(([path, bytes]) => {
    const actual = value.executable.modules.get(path);
    return actual instanceof Uint8Array && actual.length === bytes.length && actual.every((byte, i) => byte === bytes[i]);
  });
}
function freezeMetadata(value, literalBytes = []) {
  if (value instanceof Uint8Array) { literalBytes.push([value, value.slice()]); return value; }
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    for (const item of Object.values(value)) freezeMetadata(item, literalBytes);
    Object.freeze(value);
  }
  return value;
}
export function sourceSpan(file, start, end) {
  const a = file.getLineAndCharacterOfPosition(start), b = file.getLineAndCharacterOfPosition(end);
  return { source: file.fileName, startLine: a.line + 1, startColumn: a.character + 1,
    endLine: b.line + 1, endColumn: b.character + 1 };
}
export function diagnostic(message, span, phase = 'Prepass', protocol) {
  return Object.freeze({ phase, class: 'Rejection', code: protocol?.code ?? 0,
    primary: { kind: 'Source', span }, related: [], message,
    args: message === 'authoring.prepass.note-not-installed' ? ['Use export let explanation = note({ refs: [result], text: "…" }); explanation.at(40, 20).size(240, 120).'] : message === 'authoring.prepass.combinator-not-a-binding' ? ['a combinator is not an actor; bind it only as a non-exported const alias of a chain'] : [], ...(protocol ? { protocol } : {}) });
}
export function authoringError(message, span, phase = 'Host', protocol) {
  const error = new Error(message);
  error.circularDiagnostics = [diagnostic(message, span, phase, protocol)];
  return error;
}
const supported = new Set(['@circular/core', '@circular/exports', 'circular:current']);
const structuralReason = Object.freeze({
  surfaceStatement: 'The published export screen spelling is one top-level statement: surface("name", defineExport({ ... })) with a literal name. A nested, conditional, aliased, or computed surface() call has no compacted-log form.',
  builderTree: 'Only the published builder tree is read: defineExport({ ... }) with literal values and the published surface builders. Arbitrary JavaScript is never executed to build an export screen.',
  matchChain: 'match takes its upstream explicitly, as match(upstream). There is no .match() chain spelling and the generator prints only the explicit form.',
  computedMember: 'A computed member name is not a published actor handle spelling. Only literal method names are read here and printed by the generator.',
  mountArity: 'The published mount spelling is handle.mount(name) or handle.mount(name, role). There is no third mount argument and no standalone mount().',
  standaloneDefinition: 'defineExport({ ... }) contributes a screen only inside surface("name", defineExport({ ... })). A standalone definition is not an authoring state.',
  standaloneMount: '@circular/exports publishes no standalone mount(). handle.mount(name, role) binds the role and surface(name, defineExport({ ... })) attaches the screen of that name.',
  staticView: 'view is only an actor handle presentation axis: handle.view(kind, config). A standalone view(...) call has no published spelling and no compacted-log form.',
  exportRoleBinding: 'export(role) names a role without a mount, and no binding says which mount owns it: the daemon publishes no export, mount or role fact for the prepass to read, so the mount name could only be guessed. Write handle.mount(name, role), which names the mount the author chose.',
});
const handleMethodReason = method => method === undefined ? structuralReason.computedMember
  : `.${method}() is not a published actor handle spelling. A handle carries the published constructors, the wire combinators, the presentation axes, and mount, into, setFlags, remove, replaceConfig, replaceOptions, disconnect.`;
const CURRENT_HANDLE_METHODS = Object.freeze({
  edge: ['disconnect', 'replaceOptions'], scope: ['remove'], export: ['remove'], annotation: ['remove'],
});
const currentHandleMethodReason = (lookup, method) => method === undefined ? structuralReason.computedMember
  : `.${method}() is not a published spelling on the result of current.${lookup}(name). That handle carries ${CURRENT_HANDLE_METHODS[lookup].map(name => `${name}()`).join(' and ')} directly on itself, and no ports, presentation axes, or member path.`;
const structuralImportReason = (local, { specifier, imported }) => {
  const subject = imported === null ? `${local} is the ${specifier} namespace` : `${imported}() is imported from ${specifier}`;
  return `${subject} and is published only inside one top-level surface("name", defineExport({ ... })) statement, whose tree the prepass reads without executing it. Aliasing, passing, or calling ${local} anywhere else would run the builder instead of reading it.`;
};
const CURRENT_NAMESPACE_EXPORTS = ['current', 'default'];
const CURRENT_LOOKUPS = ['actor', 'edge', 'scope', 'export', 'annotation'];
function literal(actor, file) {
  const fail = () => { throw authoringError('authoring.prepass.config-not-literal', sourceSpan(file, actor.getStart(file), actor.end), 'Prepass'); };
  if (ts.isParenthesizedExpression(actor) || ts.isAsExpression(actor) || ts.isSatisfiesExpression(actor)) return literal(actor.expression, file);
  if (ts.isStringLiteral(actor) || ts.isNoSubstitutionTemplateLiteral(actor)) return actor.text;
  if (ts.isNumericLiteral(actor)) return Number(actor.text);
  if (ts.isBigIntLiteral(actor)) return BigInt(actor.text.slice(0, -1));
  if (actor.kind === ts.SyntaxKind.NullKeyword) return null;
  if (actor.kind === ts.SyntaxKind.TrueKeyword) return true;
  if (actor.kind === ts.SyntaxKind.FalseKeyword) return false;
  if (ts.isPrefixUnaryExpression(actor) && [ts.SyntaxKind.MinusToken, ts.SyntaxKind.PlusToken].includes(actor.operator)) {
    const value = literal(actor.operand, file);
    if (!['number', 'bigint'].includes(typeof value)) return fail();
    return actor.operator === ts.SyntaxKind.MinusToken ? -value : value;
  }
  if (ts.isNewExpression(actor) && ts.isIdentifier(actor.expression) && actor.expression.text === 'Uint8Array'
    && !actor.typeArguments?.length && actor.arguments?.length === 1 && ts.isArrayLiteralExpression(actor.arguments[0])) {
    const values = actor.arguments[0].elements.map(item => {
      if (!ts.isNumericLiteral(item)) return fail();
      const value = Number(item.text);
      if (!Number.isInteger(value) || value < 0 || value > 255) return fail();
      return value;
    });
    return new Uint8Array(values);
  }
  if (ts.isArrayLiteralExpression(actor)) return actor.elements.map(x => literal(x, file));
  if (ts.isObjectLiteralExpression(actor)) {
    const pairs = actor.properties.map(p => {
      if (!ts.isPropertyAssignment(p)) return fail();
      if (ts.isComputedPropertyName(p.name)) {
        if (!ts.isStringLiteral(p.name.expression)) return fail();
        return [p.name.expression.text, literal(p.initializer, file)];
      }
      return [p.name.text, literal(p.initializer, file)];
    });
    if (new Set(pairs.map(x => x[0])).size !== pairs.length) return fail();
    return Object.fromEntries(pairs);
  }
  return fail();
}
function routePortKeyIssue(config) {
  const cases = config?.cases;
  if (!cases || typeof cases !== 'object' || Array.isArray(cases) || cases instanceof Uint8Array) return null;
  const keys = Object.keys(cases).map(key => [key, Buffer.from(key, 'utf8')]);
  keys.sort((a, b) => Buffer.compare(a[1], b[1]));
  for (const [key, bytes] of keys) {
    const attempted = `route_${key}`;
    if (!bytes.length) return ['dynamic port key must not be empty', attempted];
    const offset = bytes.findIndex(byte => !(byte >= 0x61 && byte <= 0x7a)
      && !(byte >= 0x30 && byte <= 0x39) && byte !== 0x5f);
    if (offset !== -1) return [
      `dynamic port key has non-canonical byte 0x${bytes[offset].toString(16).padStart(2, '0')} at offset ${offset}`,
      attempted,
    ];
    if (Buffer.byteLength(attempted, 'utf8') > 32) return ['port id must be at most 32 bytes', attempted];
  }
  return null;
}
const runtimePackages = Object.freeze({ '@circular/core': core, '@circular/exports': exportsModule });
function unexportedValueUses(bundle) {
  const refusals = [];
  for (const [path, bytes] of bundle.modules) {
    const text = typeof bytes === 'string' ? bytes : new TextDecoder('utf-8', { fatal: true }).decode(bytes);
    const file = ts.createSourceFile(path, text, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TS);
    const named = new Map(), namespaces = new Map(), found = [];
    for (const statement of file.statements) {
      if (!ts.isImportDeclaration(statement) || !Object.hasOwn(runtimePackages, statement.moduleSpecifier.text)) continue;
      const specifier = statement.moduleSpecifier.text, clause = statement.importClause;
      if (!clause?.namedBindings || clause.isTypeOnly) continue;
      if (ts.isNamespaceImport(clause.namedBindings)) namespaces.set(clause.namedBindings.name.text, specifier);
      else for (const entry of clause.namedBindings.elements) if (!entry.isTypeOnly) {
        named.set(entry.name.text, { specifier, at: entry.propertyName ?? entry.name, used: false });
      }
    }
    const walk = node => {
      if (ts.isImportDeclaration(node) || ts.isInterfaceDeclaration(node) || ts.isTypeAliasDeclaration(node)
        || ts.isHeritageClause(node) && node.token === ts.SyntaxKind.ImplementsKeyword) return;
      if (ts.isExpressionWithTypeArguments(node)) return walk(node.expression);
      if (ts.isTypeNode(node)) return;
      const parent = node.parent;
      if (ts.isIdentifier(node) && named.has(node.text) && (ts.isShorthandPropertyAssignment(parent)
        || parent.name !== node && parent.propertyName !== node && parent.label !== node)) named.get(node.text).used = true;
      if ((ts.isPropertyAccessExpression(node) || ts.isElementAccessExpression(node) && ts.isStringLiteralLike(node.argumentExpression))
        && ts.isIdentifier(node.expression) && namespaces.has(node.expression.text)) {
        const member = ts.isPropertyAccessExpression(node) ? node.name : node.argumentExpression;
        found.push({ at: member, name: member.text, specifier: namespaces.get(node.expression.text) });
      }
      ts.forEachChild(node, walk);
    };
    walk(file);
    for (const { specifier, at, used } of named.values()) if (used) found.push({ at, name: at.text, specifier });
    for (const { at, name, specifier } of found.sort((a, b) => a.at.getStart(file) - b.at.getStart(file))) {
      const module = runtimePackages[specifier];
      if (Object.hasOwn(module, name)) continue;
      const refusal = diagnostic('authoring.prepass.import-provenance', sourceSpan(file, at.getStart(file), at.end), 'Prepass');
      refusals.push(Object.freeze({ ...refusal, args: Object.freeze([
        `${name} is not exported by ${specifier}; it exports ${Object.keys(module).sort().join(', ')}`]) }));
    }
  }
  return refusals;
}
/** The pinned TS parser builds metadata and inserts range-preserving wrappers, never a Plan. */
export function semanticPrepass(bundle, profile, options = {}) {
  const diagnostics = [], modules = new Map(), bindings = [], calls = [], imports = [], forwardReferences = [], segments = [], surfaces = [];
  const originalBundle = bundle;
  let templateCollection;
  try {
    templateCollection = collectTemplateFunctions(bundle, (message, path, actor) => {
      const text = originalBundle.modules.get(path);
      const file = ts.createSourceFile(path, typeof text === 'string' ? text : new TextDecoder().decode(text), ts.ScriptTarget.ES2022, true);
      throw authoringError(message, sourceSpan(file, actor?.getStart() ?? 0, actor?.end ?? 0), 'Prepass');
    });
    if (templateCollection) bundle = templateCollection.bundle;
    if (ts.version !== '5.9.3') throw new TypeError('pinned parser mismatch');
    if (options.maximumDepth !== undefined && (!Number.isSafeInteger(options.maximumDepth) || options.maximumDepth < 0 || options.maximumDepth > 64)) throw new TypeError('invalid depth ceiling');
    if (!(bundle?.modules instanceof Map) || !bundle.modules.has(bundle.entry)) throw new TypeError('bundle entry missing');
    for (const [path, bytes] of bundle.modules) {
      const text = typeof bytes === 'string' ? bytes : new TextDecoder('utf-8', { fatal: true }).decode(bytes);
      const file = ts.createSourceFile(path, text, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TS);
      const span = actor => sourceSpan(file, actor.getStart(file), actor.end);
      const reject = (message, actor = file, args) => {
        const error = authoringError(message, span(actor), 'Prepass');
        if (args) error.circularDiagnostics = error.circularDiagnostics.map(value =>
          Object.freeze({ ...value, args: Object.freeze(args) }));
        throw error;
      };
      const literalRanges = [];
      function collectLiterals(actor) {
        if (ts.isStringLiteralLike(actor) || ts.isTemplateLiteralToken(actor) || ts.isRegularExpressionLiteral(actor)) literalRanges.push([actor.getStart(file), actor.end]);
        else ts.forEachChild(actor, collectLiterals);
      }
      collectLiterals(file);
      const scanner = ts.createScanner(ts.ScriptTarget.ES2022, false, ts.LanguageVariant.Standard, text);
      for (let token = scanner.scan(); token !== ts.SyntaxKind.EndOfFileToken; token = scanner.scan()) {
        const literalRange = literalRanges.find(([start, end]) => start <= scanner.getTokenPos() && scanner.getTokenPos() < end);
        if (literalRange) { scanner.setTextPos(literalRange[1]); continue; }
        if (![ts.SyntaxKind.SingleLineCommentTrivia, ts.SyntaxKind.MultiLineCommentTrivia].includes(token)) continue;
        const comment = scanner.getTokenText();
        if (/^\/(?:\/|\*)\s*@note(?:\s|$)/.test(comment)) {
          throw authoringError('authoring.prepass.note-not-installed', sourceSpan(file, scanner.getTokenPos(), scanner.getTextPos()), 'Prepass');
        }
      }
      const exportAliases = new Map(), exportNamespaces = new Set();
      const structuralImports = new Map();
      const aliases = new Map(), namespaces = new Set(), exported = new Map(), currentBindings = new Set();
      const currentNamespaces = new Set();
      for (const statement of file.statements) {
        if (ts.isImportDeclaration(statement)) {
          const specifier = statement.moduleSpecifier.text;
          if (!supported.has(specifier)) reject('authoring.prepass.import-provenance', statement.moduleSpecifier);
          imports.push({ module: path, specifier, origin: span(statement.moduleSpecifier) });
          const clause = statement.importClause;
          if (clause?.name && specifier !== 'circular:current') reject('authoring.prepass.import-provenance', clause);
          const named = clause?.namedBindings;
          if (specifier === 'circular:current' && named && ts.isNamedImports(named)) {
            for (const entry of named.elements) {
              if (CURRENT_NAMESPACE_EXPORTS.includes((entry.propertyName ?? entry.name).text)) currentNamespaces.add(entry.name.text);
              else currentBindings.add(entry.name.text);
            }
          }
          if (specifier === 'circular:current') {
            if (clause?.name) currentNamespaces.add(clause.name.text);
            if (named && ts.isNamespaceImport(named)) currentNamespaces.add(named.name.text);
          }
          if (specifier === '@circular/exports' && named) {
            if (ts.isNamespaceImport(named)) exportNamespaces.add(named.name.text);
            else for (const entry of named.elements) exportAliases.set(entry.name.text, (entry.propertyName ?? entry.name).text);
          }
          if (specifier === '@circular/exports' && named) {
            if (ts.isNamespaceImport(named)) structuralImports.set(named.name.text, { specifier, imported: null });
            else for (const entry of named.elements) {
              structuralImports.set(entry.name.text, { specifier, imported: (entry.propertyName ?? entry.name).text });
            }
          }
          if (specifier !== 'circular:current' && named) {
            if (ts.isNamespaceImport(named)) namespaces.add(named.name.text);
            else for (const entry of named.elements) aliases.set(entry.name.text, (entry.propertyName ?? entry.name).text);
          }
        }
        if (ts.isExportAssignment(statement) || statement.modifiers?.some(x => x.kind === ts.SyntaxKind.DefaultKeyword)) reject('authoring.prepass.invalid-binding', statement);
        if (ts.isExportDeclaration(statement)) reject('authoring.prepass.invalid-binding', statement);
        if (ts.isVariableStatement(statement) && statement.modifiers?.some(x => x.kind === ts.SyntaxKind.ExportKeyword)) {
          for (const decl of statement.declarationList.declarations) {
            if (!ts.isIdentifier(decl.name) || !bindingIdentifier(decl.name.getText(file)) || !decl.initializer
              || !(statement.declarationList.flags & (ts.NodeFlags.Let | ts.NodeFlags.Const))) reject('authoring.prepass.invalid-binding', decl);
            const name = decl.name.text;
            if (exported.has(name)) reject('authoring.prepass.duplicate-binding', decl.name);
            exported.set(name, decl);
          }
        }
      }
      if (file.parseDiagnostics.length) reject('authoring.prepass.invalid-syntax', file);
      const insertions = new Map(), replacements = [], loweredCallbacks = new Map();
      const callbackReject = (part, reason) => reject('authoring.prepass.config-not-literal', part, [reason]);
      const insert = (at, value) => insertions.set(at, (insertions.get(at) ?? '') + value);
      let activeBinding = null;
      const pendingAliases = new Set();
      const handleAliases = new Set();
      const handleAlias = expr => {
        if (ts.isParenthesizedExpression(expr) || ts.isAsExpression(expr) || ts.isSatisfiesExpression(expr) || ts.isNonNullExpression(expr)) return handleAlias(expr.expression);
        return ts.isIdentifier(expr) && (exported.has(expr.text) || currentBindings.has(expr.text) || handleAliases.has(expr.text));
      };
      function pendingExpression(actor) {
        if (ts.isParenthesizedExpression(actor) || ts.isAsExpression(actor) || ts.isSatisfiesExpression(actor) || ts.isNonNullExpression(actor)) return pendingExpression(actor.expression);
        return ts.isIdentifier(actor) ? pendingAliases.has(actor.text)
          : ts.isCallExpression(actor) && ts.isPropertyAccessExpression(actor.expression) && COMBINATOR_NAMES.includes(actor.expression.name.text);
      }
      const exportImport = expr => ts.isIdentifier(expr) ? exportAliases.get(expr.text)
        : ts.isPropertyAccessExpression(expr) && ts.isIdentifier(expr.expression) && exportNamespaces.has(expr.expression.text) ? expr.name.text : null;
      const currentNamespaceBinding = expr => ts.isIdentifier(expr) && currentNamespaces.has(expr.text);
      const currentNamespaceMember = expr => {
        if (!(ts.isPropertyAccessExpression(expr) || ts.isElementAccessExpression(expr)) || !currentNamespaceBinding(expr.expression)) return null;
        const member = ts.isPropertyAccessExpression(expr) ? expr.name.text
          : ts.isStringLiteral(expr.argumentExpression) || ts.isNoSubstitutionTemplateLiteral(expr.argumentExpression) ? expr.argumentExpression.text : null;
        if (member !== null && CURRENT_LOOKUPS.includes(member)) return null;
        return member === null ? `the computed member ${expr.argumentExpression.getText(file)}` : member;
      };
      const currentLookupCall = node => {
        while (ts.isParenthesizedExpression(node) || ts.isAsExpression(node) || ts.isSatisfiesExpression(node) || ts.isNonNullExpression(node)) node = node.expression;
        if (!ts.isCallExpression(node)) return null;
        const callee = node.expression;
        if (!(ts.isPropertyAccessExpression(callee) || ts.isElementAccessExpression(callee)) || !currentNamespaceBinding(callee.expression)) return null;
        const member = ts.isPropertyAccessExpression(callee) ? callee.name.text
          : ts.isStringLiteral(callee.argumentExpression) || ts.isNoSubstitutionTemplateLiteral(callee.argumentExpression) ? callee.argumentExpression.text : null;
        return member !== null && CURRENT_LOOKUPS.includes(member) ? member : null;
      };
      const namespaceUseReject = (part, attempted) => reject('authoring.current.namespace-use-not-supported', part,
        [`circular:current publishes ${CURRENT_LOOKUPS.join(', ')} on one fixed snapshot anchor; ${attempted} is not supported. The namespace does not follow the commit feed: to read newer state, take a complete snapshot again.`]);
      const declaredSurfaceNames = new Set();
      function visit(actor) {
        if (ts.isCallExpression(actor) && exportImport(actor.expression) === 'surface') {
          if (!ts.isExpressionStatement(actor.parent) || actor.parent.parent !== file || actor.arguments.length !== 2 || !ts.isStringLiteral(actor.arguments[0])) reject('authoring.prepass.structural-not-installed', actor, [structuralReason.surfaceStatement]);
          const name = actor.arguments[0].text.normalize('NFC');
          if (!name.length || /[\u0000-\u001f\u007f]/u.test(name)) reject('authoring.prepass.config-not-literal', actor.arguments[0]);
          if (declaredSurfaceNames.has(name)) reject('authoring.export.surface-bound-twice', actor.arguments[0], [name]);
          declaredSurfaceNames.add(name);
          let value;
          try { value = collectExportSurface(actor.arguments[1], exportImport, n => reject('authoring.prepass.structural-not-installed', n, [structuralReason.builderTree])); }
          catch (error) { if (error.circularDiagnostics) throw error; reject('authoring.prepass.structural-not-installed', actor, [structuralReason.builderTree]); }
          const id = surfaces.length;
          surfaces.push({ module: path, name, value, origin: span(actor) });
          replacements.push({ start: actor.getStart(file), end: actor.end, text: `__circular.surface(${id})` });
          return;
        }
        if (ts.isIdentifier(actor) && actor.text === '__circular') reject('authoring.prepass.reserved-instrumentation', actor);
        if (ts.isForOfStatement(actor) && currentNamespaceBinding(actor.expression)) namespaceUseReject(actor, actor.awaitModifier ? 'for await iteration' : 'for of iteration');
        const unpublished = currentNamespaceMember(ts.isCallExpression(actor) ? actor.expression : actor);
        if (unpublished) namespaceUseReject(actor, unpublished);
        if (ts.isIdentifier(actor) && currentNamespaces.has(actor.text)
          && !((ts.isPropertyAccessExpression(actor.parent) || ts.isElementAccessExpression(actor.parent)) && actor.parent.expression === actor)) {
          namespaceUseReject(actor, `${actor.text} as a value`);
        }
        if (ts.isIdentifier(actor) && structuralImports.has(actor.text)
          && !((ts.isPropertyAccessExpression(actor.parent) || ts.isPropertyAssignment(actor.parent)) && actor.parent.name === actor)) {
          reject('authoring.prepass.structural-not-installed', actor, [structuralImportReason(actor.text, structuralImports.get(actor.text))]);
        }
        if (ts.isImportEqualsDeclaration(actor)) reject('authoring.prepass.import-provenance', actor);
        if (ts.isAwaitExpression(actor)) reject('authoring.prepass.top-level-await', actor);
        if (loweredCallbacks.has(actor)) return;
        if (ts.isArrowFunction(actor) || ts.isFunctionExpression(actor)) callbackReject(actor, 'callbacks require a map, filter, or alert expression argument compiled by semanticPrepass; use a CEL string otherwise');
        if (ts.isCallExpression(actor)) {
          if (actor.expression.kind === ts.SyntaxKind.ImportKeyword) reject('authoring.prepass.dynamic-import', actor);
          const expr = actor.expression;
          let spelling;
          if (ts.isIdentifier(expr)) spelling = aliases.get(expr.text);
          else if (ts.isPropertyAccessExpression(expr)) {
            if (ts.isIdentifier(expr.expression) && namespaces.has(expr.expression.text)) spelling = expr.name.text;
            else if (constructorSpelling(core[expr.name.text]) || COMBINATOR_NAMES.includes(expr.name.text)) spelling = expr.name.text;
          }
          const propertyCall = ts.isPropertyAccessExpression(expr) || ts.isElementAccessExpression(expr) && ts.isStringLiteral(expr.argumentExpression);
          const method = ts.isPropertyAccessExpression(expr) ? expr.name.text
            : propertyCall ? expr.argumentExpression.text : spelling;
          const presentationMethods = ['label', 'at', 'size', 'board', 'group', 'collapsed', 'view',
            'before', 'after', 'alignHorizontal', 'alignVertical', 'replacePresentation'];
          let receiver = ts.isPropertyAccessExpression(expr) || ts.isElementAccessExpression(expr) ? expr.expression : null, receiverCreatesActor = false, receiverLookup = null;
          while (receiver && (ts.isPropertyAccessExpression(receiver) || ts.isElementAccessExpression(receiver) || ts.isCallExpression(receiver))) {
            if (ts.isCallExpression(receiver)) {
              const callee = receiver.expression;
              const name = ts.isIdentifier(callee) ? aliases.get(callee.text)
                : ts.isPropertyAccessExpression(callee) ? callee.name.text : null;
              receiverCreatesActor ||= Boolean(constructorSpelling(core[name]));
              receiverLookup = currentLookupCall(receiver) ?? receiverLookup;
            }
            receiver = receiver.expression;
          }
          const actorMethod = receiverCreatesActor || receiverLookup === 'actor' || receiver && ts.isIdentifier(receiver)
            && (exported.has(receiver.text) || currentBindings.has(receiver.text) || handleAliases.has(receiver.text));
          if (receiverLookup && receiverLookup !== 'actor'
            && (currentLookupCall(expr.expression) !== receiverLookup || !CURRENT_HANDLE_METHODS[receiverLookup].includes(method))) {
            reject('authoring.prepass.structural-not-installed', actor, [currentHandleMethodReason(receiverLookup, method)]);
          }
          if (method === 'export' && propertyCall && !currentNamespaceBinding(expr.expression)) {
            reject('authoring.prepass.structural-not-installed', actor, [structuralReason.exportRoleBinding]);
          }
          if (actorMethod && method === 'match') reject('authoring.prepass.structural-not-installed', actor, [structuralReason.matchChain]);
          if (actorMethod && !(constructorSpelling(core[method]) || COMBINATOR_NAMES.includes(method)) && !presentationMethods.includes(method)
            && !['mount', 'into', 'setFlags', 'remove', 'replaceConfig', 'replaceOptions', 'disconnect'].includes(method)) {
            reject('authoring.prepass.structural-not-installed', actor, [handleMethodReason(method)]);
          }
          if (['declareActor', 'declareEdge', 'declareScope'].includes(method)) reject('authoring.prepass.normalized-structure-not-supported', actor);
          if (method === 'mount' && propertyCall
            && !(ts.isIdentifier(expr.expression) && namespaces.has(expr.expression.text))) {
            if (actor.arguments.length < 1 || actor.arguments.length > 2) reject('authoring.prepass.structural-not-installed', actor, [structuralReason.mountArity]);
            if (!ts.isStringLiteral(actor.arguments[0]) || actor.arguments[1] &&
              (!ts.isStringLiteral(actor.arguments[1]) || !['request', 'progress', 'result', 'error'].includes(actor.arguments[1].text))) {
              reject('authoring.prepass.config-not-literal', actor);
            }
            let receiver = expr.expression;
            while (ts.isPropertyAccessExpression(receiver) || ts.isElementAccessExpression(receiver)) {
              if (ts.isElementAccessExpression(receiver) && !ts.isStringLiteral(receiver.argumentExpression)) reject('authoring.prepass.mount-receiver', receiver);
              receiver = receiver.expression;
            }
            if (!ts.isIdentifier(receiver) || !currentBindings.has(receiver.text) &&
              (!exported.has(receiver.text) || exported.get(receiver.text).end >= actor.getStart(file)
                || bindings.find(b => b.module === path && b.name === receiver.text)?.call === undefined)) {
              reject('authoring.prepass.mount-receiver', expr.expression);
            }
          } else if (['defineExport', 'mount', 'view'].includes(method) && !(actorMethod && presentationMethods.includes(method))) {
            reject('authoring.prepass.structural-not-installed', actor, [structuralReason[
              method === 'defineExport' ? 'standaloneDefinition' : method === 'mount' ? 'standaloneMount' : 'staticView']]);
          }
          if (['inlet', 'outlet'].includes(method)) reject('authoring.prepass.retired-boundary-callback', actor);
          const id = calls.length;
          const call = { id, module: path, origin: span(actor), binding: activeBinding, spelling: null };
          calls.push(call);
          const combinator = COMBINATOR_NAMES.includes(spelling);
          const canonical = constructorSpelling(core[spelling]);
          if (canonical) spelling = canonical;
          if (combinator) {
            if (!ts.isPropertyAccessExpression(expr) || ts.isIdentifier(expr.expression) && namespaces.has(expr.expression.text)) reject('authoring.prepass.detached-combinator', actor);
            if (activeBinding && exported.get(activeBinding).initializer === actor) reject('authoring.prepass.combinator-not-a-binding', actor);
          }
          if (spelling === 'note') {
            const binding = bindings.find(x => x.module === path && x.name === activeBinding);
            if (!binding || binding.call !== undefined) reject('authoring.prepass.unbound-actor', actor);
            if (actor.arguments.length !== 1 || !ts.isObjectLiteralExpression(actor.arguments[0])) reject('authoring.prepass.invalid-note', actor);
            const names = actor.arguments[0].properties.map(p => p.name?.text);
            if (new Set(names).size !== names.length || names.some(name => !['id', 'refs', 'text'].includes(name))) reject('authoring.prepass.invalid-note', actor);
            call.annotation = true;
            binding.call = id;
          }
          if (canonical || combinator) {
            if (!combinator && (!activeBinding || bindings.find(x => x.module === path && x.name === activeBinding)?.call !== undefined)) reject('authoring.prepass.unbound-actor', actor);
            call.spelling = combinator ? null : spelling;
            if (combinator) call.combinator = spelling;
            call.actorType = ({ project_input: 'input', project_output: 'output' })[spelling] ?? spelling;
            for (const [index, arg] of actor.arguments.entries()) {
              let expression = arg;
              while (ts.isParenthesizedExpression(expression) || ts.isAsExpression(expression) || ts.isSatisfiesExpression(expression)) expression = expression.expression;
              if (!ts.isArrowFunction(expression) && !ts.isFunctionExpression(expression)) continue;
              if (index !== 0 || !['map', 'filter', 'alert'].includes(spelling)) {
                callbackReject(arg, 'this argument is not a dataflow expression; use the declared configuration shape');
              }
              const cel = lowerCallback(expression, callbackReject);
              loweredCallbacks.set(arg, cel);
              replacements.push({ start: arg.getStart(file), end: arg.end, text: JSON.stringify(cel), original: span(expression.body) });
            }
            const configCount = ['bang', 'tap', 'counter', 'match'].includes(spelling) ? 0 : spelling === 'alert' ? 2 : 1;
            if (actor.arguments.length > configCount + (combinator ? 0 : 1)) reject('authoring.prepass.config-not-literal', actor);
            const wiring = actor.arguments[configCount];
            if (wiring && ts.isObjectLiteralExpression(wiring)) {
              const names = wiring.properties.filter(p => !ts.isSpreadAssignment(p) && !ts.isComputedPropertyName(p.name)).map(p => p.name.text);
              if (new Set(names).size !== names.length) reject('CIRCULAR_INPUT_BINDING_DUPLICATE', wiring);
            }
            let args;
            try { args = actor.arguments.slice(0, configCount).map(x => loweredCallbacks.has(x) ? loweredCallbacks.get(x) : literal(x, file)); }
            catch (error) {
              if (call.actorType === 'replicator') reject('authoring.prepass.replicator-policy', actor);
              throw error;
            }
            if ((['map', 'filter'].includes(spelling) && (args.length !== 1 || typeof args[0] !== 'string'))
              || (spelling === 'alert' && (args.length !== 2 || typeof args[0] !== 'string'))) reject('authoring.prepass.config-not-literal', actor);
            if (spelling === 'map') call.config = { transform: args[0] };
            else if (spelling === 'filter') call.config = { predicate: args[0] };
            else if (spelling === 'alert') {
              if (args[1] === null || typeof args[1] !== 'object' || Array.isArray(args[1]) || 'predicate' in args[1]) reject('authoring.prepass.config-not-literal', actor);
              call.config = { predicate: args[0], ...args[1] };
            }
            else call.config = args.length ? args[0] : configCount === 0 && !combinator ? null : {};
            try { call.config = lowerConfig(spelling, call.config); }
            catch (error) { reject(error.code, actor.arguments[0] ?? actor, [error.message]); }
            if (combinator && spelling === 'flatten') {
              const issue = flattenConfigIssue(call.config);
              if (issue) reject('authoring.prepass.config-not-literal', actor.arguments[0] ?? actor, [issue]);
            }
            if (call.actorType === 'assemble') {
              const issue = assembleConfigIssue(call.config);
              if (issue) reject('authoring.prepass.config-not-literal', actor.arguments[0] ?? actor, [issue]);
            }
            if (call.actorType === "join") {
              const issue = joinConfigIssue(call.config);
              if (issue) reject('authoring.prepass.config-not-literal', actor.arguments[0] ?? actor, [issue]);
            }
            if (call.actorType === 'route') {
              const issue = routePortKeyIssue(call.config);
              if (issue) reject('authoring.prepass.config-not-literal', actor.arguments[0] ?? actor, issue);
            }

            if (combinator && (!call.config || typeof call.config !== 'object' || Array.isArray(call.config) || call.config instanceof Uint8Array)) reject('authoring.prepass.config-not-literal', actor);
            if (call.actorType === 'replicator' && !replicatorPolicy(call.config !== null && typeof call.config === 'object' && !Array.isArray(call.config)
              ? { at: call.config.at, ttl: call.config.ttl, capacity: call.config.capacity } : null)) reject('authoring.prepass.replicator-policy', actor);
            const literalConfig = call.config !== null && typeof call.config === 'object' && !Array.isArray(call.config);
            if (call.actorType === 'replicator' || call.actorType === 'pipeline_actor' && literalConfig && Object.hasOwn(call.config, 'template')) {
              const keys = call.actorType === 'pipeline_actor' ? ['template','in','out'] : ['template','in','out','at','ttl','capacity'];
              if (!call.config || Object.keys(call.config).length !== keys.length || keys.some(k => !Object.hasOwn(call.config,k))
                || typeof call.config.template !== 'string') reject('authoring.prepass.boundary-field-not-carried', actor);
              call.authoredConfig = call.config;
            } else if (spelling === 'pipeline_actor') {
              call.authoredConfig = call.config;
              if (!call.config || typeof call.config !== 'object' || Array.isArray(call.config)
                || Object.keys(call.config).some(key => !['source', 'in', 'out'].includes(key))) reject('authoring.prepass.boundary-field-not-carried', actor);
              if (typeof call.config.source !== 'string') reject('authoring.prepass.bundle-path-outside', actor);
              for (const direction of ['in', 'out']) {
                const names = call.config[direction];
                if (!Array.isArray(names) || names.some(name => typeof name !== 'string' || !name.length)) reject('authoring.prepass.boundary-field-not-carried', actor);
                if (new Set(names).size !== names.length) reject('authoring.prepass.boundary-duplicate-topic', actor);
              }
              call.childModule = call.config.source;
              call.config = null;
            } else if (['project_input', 'project_output'].includes(spelling)) {
              call.authoredConfig = call.config;
            }
            const binding = bindings.find(x => x.module === path && x.name === activeBinding);
            if (!combinator) binding.call = id;
          }
          insert(actor.getStart(file), `__circular.call(${id},()=> (`);
          ts.forEachChild(actor, visit);
          insert(actor.end, '))');
          return;
        }
        if (ts.isIdentifier(actor) && exported.has(actor.text) && actor.getStart(file) < exported.get(actor.text).getStart(file)
          && !((ts.isPropertyAccessExpression(actor.parent) || ts.isPropertyAssignment(actor.parent)) && actor.parent.name === actor)) {
          forwardReferences.push({ module: path, binding: actor.text, origin: span(actor) });
          replacements.push({ start: actor.getStart(file), end: actor.end, text: `__circular.forward(${JSON.stringify(actor.text)})` });
        }
        ts.forEachChild(actor, visit);
      }
      for (const [name, decl] of exported) bindings.push({ module: path, name, origin: span(decl) });
      for (const statement of file.statements) {
        if (ts.isImportDeclaration(statement)) continue;
        if (ts.isVariableStatement(statement)) {
          for (const decl of statement.declarationList.declarations) {
            activeBinding = exported.get(decl.name.text) === decl ? decl.name.text : null;
            if (decl.initializer) {
              if (pendingExpression(decl.initializer)) {
                if (activeBinding || !(statement.declarationList.flags & ts.NodeFlags.Const)) reject('authoring.prepass.combinator-not-a-binding', decl);
                pendingAliases.add(decl.name.text);
              }
              if (activeBinding) insert(decl.initializer.getStart(file), `__circular.bind(${JSON.stringify(activeBinding)},()=> (`);
              if (!activeBinding && ts.isIdentifier(decl.name) && handleAlias(decl.initializer)) handleAliases.add(decl.name.text);
              visit(decl.initializer);
              if (activeBinding) insert(decl.initializer.end, '))');
            }
          }
          activeBinding = null;
        } else visit(statement);
      }
      let executable = '', offset = 0;
      const replacementAt = new Map(replacements.map(x => [x.start, x]));
      while (offset <= text.length) {
        executable += insertions.get(offset) ?? '';
        if (offset === text.length) break;
        const replacement = replacementAt.get(offset);
        if (replacement) {
          const generatedStart = executable.length;
          executable += replacement.text;
          if (replacement.original) segments.push({ path, generatedStart, generatedEnd: executable.length, original: replacement.original, transformed: true });
          offset = replacement.end; continue;
        }
        const start = offset;
        do { offset++; } while (offset < text.length && !insertions.has(offset) && !replacementAt.has(offset));
        const generatedStart = executable.length;
        executable += text.slice(start, offset);
        segments.push({ path, generatedStart, generatedEnd: executable.length, original: sourceSpan(file, start, offset) });
      }
      const generatedFile = ts.createSourceFile(path, executable, ts.ScriptTarget.ES2022, true);
      for (const segment of segments.filter(x => x.path === path)) segment.generated = sourceSpan(generatedFile, segment.generatedStart, segment.generatedEnd);
      modules.set(path, new TextEncoder().encode(executable));
    }
    let dependencies = [], scopePlan = [];
    const fail = (message, call) => { throw authoringError(message, call?.origin ?? {
      source: bundle.entry, startLine: 1, startColumn: 1, endLine: 1, endColumn: 1 }, 'Prepass'); };
    const validPath = path => typeof path === 'string' && path.endsWith('.ts')
      && ['scopes', 'templates'].includes(path.split('/')[0]) && path.slice(path.indexOf('/') + 1, -3).split('/').every(bindingIdentifier);
    if (modules.size > 1 && bundle.entry !== 'main.ts') fail('authoring.prepass.bundle-path-outside');
    for (const path of modules.keys()) if (path !== bundle.entry && !validPath(path)) fail('authoring.prepass.bundle-path-outside');
    const visiting = new Set(), visited = new Set();
    const walk = (module, scope, container = null) => {
      if (visiting.has(module)) fail('authoring.prepass.bundle-cycle', container);
      if (visited.has(module)) fail('authoring.prepass.bundle-multiple-parents', container);
      if (scope.length > (options.maximumDepth ?? 64)) fail('authoring.prepass.bundle-depth-exceeded', container);
      visiting.add(module); visited.add(module);
      const plan = { module, scope, role: 'Concrete', container: container?.id ?? null, boundaries: [] };
      scopePlan.push(plan);
      const localCalls = calls.filter(c => c.module === module && c.spelling);
      for (const call of localCalls) call.scope = scope;
      const boundaryCalls = localCalls.filter(c => ['project_input', 'project_output'].includes(c.spelling));
      if (!container && boundaryCalls.length) fail('authoring.prepass.boundary-counterpart-missing', boundaryCalls[0]);
      if (container) {
        for (const [direction, spelling] of [['in', 'project_input'], ['out', 'project_output']]) {
          const children = boundaryCalls.filter(c => c.spelling === spelling);
          const topics = children.map(c => c.config.label), names = container.authoredConfig[direction];
          if (new Set(topics).size !== topics.length) fail('authoring.prepass.boundary-duplicate-topic', children[0]);
          if (topics.some(topic => !names.includes(topic)) || names.some(name => !topics.includes(name))) {
            const opposite = boundaryCalls.filter(c => c.spelling !== spelling).map(c => c.config.label);
            fail(names.some(name => opposite.includes(name)) ? 'authoring.prepass.boundary-direction-mismatch'
              : 'authoring.prepass.boundary-counterpart-missing', container);
          }
          plan.boundaries.push(...children.map(c => ({ call: c.id, direction, topic: c.config.label })));
        }
      }
      for (const call of localCalls.filter(c => c.childModule)) {
        const child = call.childModule;
        if (visiting.has(child)) fail('authoring.prepass.bundle-cycle', call);
        if (!validPath(child)) fail('authoring.prepass.bundle-path-outside', call);
        if (!modules.has(child)) fail('authoring.prepass.bundle-module-missing', call);
        if (options.moduleResolver) {
          const resolved = options.moduleResolver.resolve(bundle, { referrer: module, specifier: child });
          if (resolved.status !== 'complete') fail('authoring.prepass.bundle-module-missing', call);
          const bytes = value => typeof value === 'string' ? new TextEncoder().encode(value) : value;
          const expected = bytes(bundle.modules.get(child)), actual = bytes(resolved.value.bytes);
          if (resolved.value.path !== child || actual.length !== expected.length || !actual.every((b, i) => b === expected[i])) fail('authoring.prepass.bundle-digest-mismatch', call);
        }
        const childScope = [...scope, { name: call.binding }];
        if (child !== `scopes/${childScope.map(s => s.name).join('/')}.ts`) fail('authoring.prepass.bundle-identity-mismatch', call);
        dependencies.push({ referrer: module, origin: call.origin, resolved: child });
        walk(child, childScope, call);
      }
      visiting.delete(module);
    };
    walk(bundle.entry, []);
    if (templateCollection) {
      const planned = templateScopePlan(bundle, templateCollection.templates, calls, fail, options.maximumDepth, [...visited]);
      scopePlan.push(...planned.scopePlan);
      dependencies.push(...planned.dependencies);
    }
    if (visited.size + (templateCollection?.templates.length ?? 0) !== modules.size) fail('authoring.prepass.bundle-module-unreachable');
    const unexported = unexportedValueUses(originalBundle);
    if (unexported.length) return { status: 'rejected', diagnostics: Object.freeze(unexported) };
    if (templateCollection) {
      for (const entries of [calls, bindings, imports, forwardReferences, surfaces, dependencies]) {
        for (const entry of entries) if (entry.origin) entry.origin = templateCollection.remap(entry.origin);
      }
      for (const segment of segments) segment.original = templateCollection.remap(segment.original);
    }
    const result = Object.freeze({ source: originalBundle, templates: templateCollection?.templates ?? null, executable: { entry: bundle.entry, modules }, profile,
      sourceMap: { segments: segments.map(({ generated, original, transformed }) => {
        const segment = { generated, original };
        if (transformed) callbackSourceSegments.add(segment);
        return segment;
      }) }, dependencies, scopePlan,
      bindings, calls, imports, forwardReferences, surfaces });
    const literalBytes = [];
    freezeMetadata(result.templates); freezeMetadata(result.dependencies); freezeMetadata(result.scopePlan);
    freezeMetadata(result.calls, literalBytes); freezeMetadata(result.bindings); freezeMetadata(result.imports);
    freezeMetadata(result.surfaces); freezeMetadata(result.forwardReferences); freezeMetadata(result.sourceMap);
    Object.freeze(result.executable);
    prepared.set(result, { entry: bundle.entry, source: new Map([...originalBundle.modules].map(([path, source]) => [path, typeof source === 'string' ? new TextEncoder().encode(source) : source.slice()])), modules: new Map([...modules].map(([path, bytes]) => [path, bytes.slice()])), literalBytes });
    return { status: 'complete', value: result, diagnostics };
  } catch (error) {
    if (templateCollection && error.circularDiagnostics) for (const diagnostic of error.circularDiagnostics) {
      if (diagnostic.primary?.kind === 'Source') diagnostic.primary.span = templateCollection.remap(diagnostic.primary.span);
    }
    return { status: 'rejected', diagnostics: error.circularDiagnostics ?? [diagnostic('authoring.prepass.invalid-bundle',
      { source: bundle?.entry ?? '<authoring>', startLine: 1, startColumn: 1, endLine: 1, endColumn: 1 })] };
  }
}
