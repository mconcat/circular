import ts from 'typescript';
import * as core from '@circular/core';
import { constructorSpelling } from '@circular/core/internal';
import { replicatorInlet } from '@circular/generator/internal';
import { posix } from 'node:path';

export function collectTemplateFunctions(bundle, reject) {
  if (!(bundle?.modules instanceof Map)) return null;
  const files = new Map(), definitions = new Map();
  for (const [path, bytes] of bundle.modules) {
    const text = typeof bytes === 'string' ? bytes : new TextDecoder('utf-8', { fatal: true }).decode(bytes);
    const ast = ts.createSourceFile(path, text, ts.ScriptTarget.ES2022, true, ts.ScriptKind.TS);
    const entry = { path, text, ast, functions: new Map(), imports: new Map(), aliases: new Map(), exports: new Map() };
    files.set(path, entry);
    for (const statement of ast.statements) {
      if (ts.isFunctionDeclaration(statement) && statement.name) {
        entry.functions.set(statement.name.text, statement);
        if (statement.modifiers?.some(m => m.kind === ts.SyntaxKind.ExportKeyword)) entry.exports.set(statement.name.text, statement.name.text);
      }
      if (ts.isImportDeclaration(statement) && statement.moduleSpecifier.text.startsWith('.')) {
        const named = statement.importClause?.namedBindings;
        if (!named || !ts.isNamedImports(named) || statement.importClause.name) continue;
        for (const name of named.elements) entry.imports.set(name.name.text, { path: statement.moduleSpecifier.text, name: (name.propertyName ?? name.name).text });
      }
      if (ts.isVariableStatement(statement)) for (const d of statement.declarationList.declarations) {
        if (ts.isIdentifier(d.name) && d.initializer && (ts.isArrowFunction(d.initializer) || ts.isFunctionExpression(d.initializer))) {
          entry.functions.set(d.name.text, d.initializer);
          if (statement.modifiers?.some(m => m.kind === ts.SyntaxKind.ExportKeyword)) entry.exports.set(d.name.text, d.name.text);
        }
        if (ts.isIdentifier(d.name) && d.initializer && ts.isIdentifier(d.initializer)) entry.aliases.set(d.name.text, d.initializer.text);
      }
      if (ts.isExportDeclaration(statement) && statement.exportClause && ts.isNamedExports(statement.exportClause)) {
        for (const name of statement.exportClause.elements) entry.exports.set(name.name.text,
          statement.moduleSpecifier ? { path: statement.moduleSpecifier.text, name: (name.propertyName ?? name.name).text } : (name.propertyName ?? name.name).text);
      }
    }
  }
  const bindings = new Set();
  const locate = (from, specifier) => {
    const raw = posix.normalize(posix.join(posix.dirname(from), specifier));
    const path = files.has(raw) ? raw : raw.replace(/\.js$/, '') + '.ts';
    if (raw.startsWith('../') || !files.has(path)) reject('authoring.prepass.bundle-module-missing', from);
    return path;
  };
  const resolve = (path, name, seen = new Set()) => {
    const key = `${path}\0${name}`;
    if (seen.has(key)) reject('authoring.prepass.bundle-cycle', path);
    seen.add(key); bindings.add(key);
    const file = files.get(path);
    if (file.functions.has(name)) return { file, fn: file.functions.get(name), name: file.functions.get(name).name?.text ?? name };
    const reference = file.imports.get(name) ?? file.aliases.get(name);
    if (typeof reference === 'string') return resolve(path, reference, seen);
    if (reference) {
      const target = locate(path, reference.path), exported = files.get(target).exports.get(reference.name);
      if (typeof exported === 'string') return resolve(target, exported, seen);
      if (exported) return resolveExport(target, exported, seen);
    }
    reject('authoring.prepass.invalid-template-reference', path);
  };
  const resolveExport = (path, reference, seen = new Set()) => {
    if (typeof reference === 'string') return resolve(path, reference, seen);
    const key = `export:${path}\0${reference.path}\0${reference.name}`;
    if (seen.has(key)) reject('authoring.prepass.bundle-cycle', path);
    seen.add(key);
    const target = locate(path, reference.path), exported = files.get(target).exports.get(reference.name);
    if (!exported) reject('authoring.prepass.invalid-template-reference', path);
    return resolveExport(target, exported, seen);
  };
  const isFunctionReference = (path, reference, seen = new Set()) => {
    const key = JSON.stringify([path, reference]);
    if (seen.has(key)) return false;
    seen.add(key);
    const file = files.get(path);
    if (typeof reference === 'string') {
      if (file.functions.has(reference)) return true;
      const next = file.imports.get(reference) ?? file.aliases.get(reference);
      return next ? isFunctionReference(path, next, seen) : false;
    }
    const target = locate(path, reference.path), next = files.get(target).exports.get(reference.name);
    return next ? isFunctionReference(target, next, seen) : false;
  };
  const roots = [], uses = new Map();
  const containerName = name => ['pipeline_actor', 'replicator'].includes(constructorSpelling(core[name]));
  function references(file, actor) {
    if (ts.isCallExpression(actor)) {
      const imports = file.ast.statements.filter(s => ts.isImportDeclaration(s) && s.moduleSpecifier.text === '@circular/core');
      const constructors = new Set(imports.flatMap(s => s.importClause?.namedBindings && ts.isNamedImports(s.importClause.namedBindings)
        ? s.importClause.namedBindings.elements.filter(n => containerName((n.propertyName ?? n.name).text)).map(n => n.name.text) : []));
      const e = actor.expression;
      const container = ts.isIdentifier(e) ? constructors.has(e.text) : ts.isPropertyAccessExpression(e)
        && containerName(e.name.text);
      const config = actor.arguments[0];
      if (container && config && ts.isObjectLiteralExpression(config)) {
        const property = config.properties.find(p => ts.isPropertyAssignment(p) && p.name.getText(file.ast).replace(/['"]/g, '') === 'template');
        if (property) {
          if (!ts.isIdentifier(property.initializer)) reject('authoring.prepass.invalid-template-reference', file.path, property.initializer);
          const definition = resolve(file.path, property.initializer.text);
          roots.push(definition); uses.set(property.initializer, definition);
        }
      }
    }
    ts.forEachChild(actor, child => { if (!ts.isFunctionLike(child)) references(file, child); });
  }
  const rootFile = files.get(bundle.entry);
  if (!rootFile) return null;
  references(rootFile, rootFile.ast);
  const entry = files.get(bundle.entry);
  for (const reference of entry?.exports.values() ?? []) {
    if (isFunctionReference(bundle.entry, reference)) roots.push(resolveExport(bundle.entry, reference));
  }
  if (!roots.length) return null;
  for (const reference of entry.exports.values()) if (!isFunctionReference(bundle.entry, reference)) reject('authoring.prepass.invalid-binding', entry.path);
  if (entry.ast.parseDiagnostics.length) reject('authoring.prepass.invalid-bundle', entry.path);
  for (const definition of roots) {
    const { fn, name, file } = definition;
    if (!fn.body || !ts.isBlock(fn.body) || fn.parameters.length || fn.asteriskToken || fn.typeParameters?.length
      || fn.modifiers?.some(m => [ts.SyntaxKind.AsyncKeyword, ts.SyntaxKind.DefaultKeyword].includes(m.kind))) reject('authoring.prepass.invalid-template-body', file.path, fn);
    const previous = definitions.get(name);
    if (previous && previous.fn !== fn) reject('authoring.template.name-conflict', file.path, fn);
    if (!previous) { definitions.set(name, definition); references(file, fn.body); }
  }
  for (const {file} of definitions.values()) {
    if (file.ast.parseDiagnostics.length) reject('authoring.prepass.invalid-bundle', file.path);
    if (file !== entry) for (const statement of file.ast.statements) {
      if (ts.isImportDeclaration(statement) || ts.isExportDeclaration(statement) || ts.isFunctionDeclaration(statement)) continue;
      if (ts.isVariableStatement(statement) && statement.declarationList.declarations.every(d => ts.isIdentifier(d.name)
        && (file.functions.has(d.name.text) || bindings.has(`${file.path}\0${d.name.text}`)))) continue;
      reject('authoring.prepass.invalid-template-body', file.path, statement);
    }
  }
  const sourceRanges = new Map();
  const printer = ts.createPrinter({ newLine: ts.NewLineKind.LineFeed, removeComments: true });
  const transform = file => context => {
    const visit = actor => {
      if (uses.has(actor)) return ts.factory.createStringLiteral(uses.get(actor).name);
      return ts.visitEachChild(actor, visit, context);
    };
    return actor => ts.visitNode(actor, visit);
  };
  const print = (file, statements, module, body = null) => {
    const imports = file.ast.statements.filter(s => ts.isImportDeclaration(s) && !s.moduleSpecifier.text.startsWith('.'));
    const removable = d => ts.isIdentifier(d.name) && (file.functions.has(d.name.text)
      || file.aliases.has(d.name.text) && bindings.has(`${file.path}\0${d.name.text}`));
    const output = statements.filter(s => !ts.isImportDeclaration(s) && !ts.isFunctionDeclaration(s) && !ts.isExportDeclaration(s));
    let text = ''; const ranges = [];
    const append = (printed, start, end) => {
      const generatedStart = text.length; text += printed + '\n';
      ranges.push({ generatedStart, generatedEnd: text.length, start, end, file });
    };
    for (let statement of [...imports, ...output]) {
      const start = statement.getStart(file.ast), end = statement.end;
      if (ts.isVariableStatement(statement)) {
        const declarations = statement.declarationList.declarations.filter(d => !removable(d));
        if (!declarations.length) continue;
        statement = ts.factory.updateVariableStatement(statement, statement.modifiers,
          ts.factory.updateVariableDeclarationList(statement.declarationList, declarations));
      }
      if (body && !ts.isImportDeclaration(statement) && !ts.isVariableStatement(statement) && !ts.isExpressionStatement(statement)) reject('authoring.prepass.invalid-template-body', file.path, statement);
      if (body && ts.isVariableStatement(statement)) statement = ts.factory.updateVariableStatement(statement,
        [ts.factory.createModifier(ts.SyntaxKind.ExportKeyword)], statement.declarationList);
      const result = ts.transform(statement, [transform(file)]);
      try { append(printer.printNode(ts.EmitHint.Unspecified, result.transformed[0], file.ast), start, end); }
      finally { result.dispose(); }
    }
    const literalRanges = [];
    function collectLiterals(actor) {
      if (ts.isStringLiteralLike(actor) || ts.isTemplateLiteralToken(actor) || ts.isRegularExpressionLiteral(actor)) literalRanges.push([actor.getStart(file.ast), actor.end]);
      else ts.forEachChild(actor, collectLiterals);
    }
    collectLiterals(file.ast);
    const scanner = ts.createScanner(ts.ScriptTarget.ES2022, false, ts.LanguageVariant.Standard, file.text);
    for (let token = scanner.scan(); token !== ts.SyntaxKind.EndOfFileToken; token = scanner.scan()) {
      const start = scanner.getTokenPos(), end = scanner.getTextPos();
      const literal = literalRanges.find(([a,b]) => a <= start && start < b);
      if (literal) { scanner.setTextPos(literal[1]); continue; }
      const inBody = body ? start > body.pos && end < body.end : ![...file.functions.values()].some(fn => start >= fn.getStart(file.ast) && end <= fn.end);
      if (inBody && [ts.SyntaxKind.SingleLineCommentTrivia, ts.SyntaxKind.MultiLineCommentTrivia].includes(token)
        && /^\/(?:\/|\*)\s*@note(?:\s|$)/.test(scanner.getTokenText())) {
        reject('authoring.prepass.note-not-installed', file.path);
      }
    }
    sourceRanges.set(module, { ast: ts.createSourceFile(module, text, ts.ScriptTarget.ES2022, true), ranges });
    return text;
  };
  const remap = span => {
    const source = sourceRanges.get(span.source);
    if (!source) return span;
    const position = source.ast.getPositionOfLineAndCharacter(span.startLine - 1, span.startColumn - 1);
    const range = source.ranges.find(r => position >= r.generatedStart && position < r.generatedEnd);
    if (!range) return span;
    const start = range.file.ast.getLineAndCharacterOfPosition(range.start), end = range.file.ast.getLineAndCharacterOfPosition(range.end);
    return { source: range.file.path, startLine: start.line + 1, startColumn: start.character + 1, endLine: end.line + 1, endColumn: end.character + 1 };
  };
  const modules = new Map([[bundle.entry, print(entry, entry.ast.statements, bundle.entry)]]), templates = [];
  for (const [name, { file, fn }] of definitions) {
    const module = `templates/${name}.ts`;
    if (module === bundle.entry) reject('authoring.prepass.bundle-identity-mismatch', file.path, fn);
    modules.set(module, print(file, fn.body.statements, module, fn.body));
    templates.push({ name, module, source: file.path, start: fn.getStart(file.ast) });
  }
  const definitionFiles = new Set([...definitions.values()].map(definition => definition.file.path));
  for (const [path, source] of bundle.modules) {
    if (path === bundle.entry || definitionFiles.has(path) || modules.has(path)) continue;
    modules.set(path, typeof source === 'string' ? source : new TextDecoder('utf-8', { fatal: true }).decode(source));
  }
  return { bundle: { entry: bundle.entry, modules }, templates, remap };
}

export function templateScopePlan(bundle, templates, calls, fail, maximumDepth = 64, concrete = [bundle.entry]) {
  const plans = new Map(), dependencies = [], visiting = new Set();
  const byName = new Map(templates.map(t => [t.name, t]));
  const templateBound = call => ['pipeline_actor','replicator'].includes(call.actorType)
    && call.config !== null && typeof call.config === 'object' && !Array.isArray(call.config)
    && typeof call.config.template === 'string';
  const link = (module, depth) => {
    for (const call of calls.filter(c => c.module === module && c.spelling && templateBound(c))) {
      const child = byName.get(call.config.template);
      if (!child) fail('authoring.prepass.invalid-template-reference', call);
      call.childModule = child.module; call.templateName = child.name;
      const childPlan = walk(child.module, child.name, depth + 1);
      for (const direction of ['in','out']) {
        const actual = childPlan.boundaries.filter(b => b.direction === direction).map(b => b.topic);
        const expected = call.config[direction];
        if (!Array.isArray(expected) || new Set(expected).size !== expected.length || expected.length !== actual.length || expected.some(n => !actual.includes(n))) fail('authoring.prepass.boundary-counterpart-missing', call);
      }
      if (call.actorType === 'replicator' && !replicatorInlet(call.config)) fail('authoring.prepass.template-inlet-count', call);
      dependencies.push({ referrer: module, resolved: child.module, origin: call.origin });
    }
  };
  const walk = (module, name, depth = 0) => {
    if (depth > maximumDepth) fail('authoring.prepass.bundle-depth-exceeded');
    if (visiting.has(module)) fail('authoring.prepass.bundle-cycle');
    if (plans.has(module)) return plans.get(module);
    visiting.add(module);
    const local = calls.filter(c => c.module === module && c.spelling);
    const plan = { module, name, scope: [], role: 'Template', container: null, boundaries: [] };
    for (const call of local) {
      call.scope = [];
      if (['project_input','project_output'].includes(call.spelling)) {
        plan.boundaries.push({ call: call.id, topic: call.config.label, direction: call.spelling === 'project_input' ? 'in' : 'out' });
      } else if (['input','output'].includes(call.actorType)) fail('authoring.prepass.boundary-counterpart-missing', call);
    }
    for (const direction of ['in','out']) {
      const names = plan.boundaries.filter(b => b.direction === direction).map(b => b.topic);
      if (new Set(names).size !== names.length) fail('authoring.prepass.boundary-duplicate-topic');
    }
    link(module, depth);
    visiting.delete(module); plans.set(module, plan); return plan;
  };
  for (const module of concrete) link(module, 0);
  for (const template of templates) walk(template.module, template.name);
  return { scopePlan: [...plans.values()], dependencies };
}
