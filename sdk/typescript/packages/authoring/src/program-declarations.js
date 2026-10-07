import { bindingIdentifier } from '@circular/generator/internal';
import { portValueType, inputPortHints } from './portflow-types.js';
import { posix } from 'node:path';

const portValues = (ports, actorType, direction) => `{ ${ports.map(p => `readonly ${JSON.stringify(p.id)}: ${portValueType(p, actorType, direction)};`).join(' ')} }`;
/** Admission supplies names only; fresh hints stay broad except the approved agent turn domain. */
export function generateProgramDeclarations(program, table, module = program.executable.entry) {
  const template = program.templates?.find(t => t.module === module);
  if (template) return Object.freeze({ path: module.replace(/\.ts$/, '.d.ts'), text: `export declare function ${template.name}(): void;\n` });
  const lines = [
    '// Generated from the admission answers of the daemon for this program; do not edit.',
    'import type { CircularValue } from "@circular/protocol";',
    'import type { ActorHandle, NewHandleMode, SourceEndpoint, TargetEndpoint } from "@circular/core";',
  ];
  let ordinal = 0;
  for (const binding of program.bindings) {
    if (binding.module !== module || binding.call === undefined) continue;
    if (program.calls[binding.call].annotation) {
      lines.push(`export declare const ${binding.name}: ReturnType<typeof import("@circular/core").note>;`);
      continue;
    }
    const resolution = table.get(binding.call);
    if (!resolution) throw new TypeError('Missing admission for program declaration');
    let { ports } = resolution;
    const templateCall = program.calls[binding.call].templateName ? program.calls[binding.call] : null;
    if (templateCall) ports = { ...ports,
      inputs: templateCall.actorType === 'replicator' ? ports.inputs : templateCall.config.in.map(id => ({ id, primary: false })),
      outputs: templateCall.config.out.map(id => ({ id, primary: false })),
    };
    const call = program.calls[binding.call];
    const actorType = resolution.declaration?.actorType ?? call.actorType ?? call.spelling;
    const inputs = inputPortHints(actorType, ports.inputs);
    let type = `ActorHandle<${JSON.stringify(call.spelling)}, NewHandleMode, ${portValues(inputs, actorType, "input")}, ${portValues(ports.outputs)}>`;
    if (ports.defaultInput != null) type += ` & TargetEndpoint<${portValueType(inputs.find(p => p.id === ports.defaultInput), actorType, "input")}, NewHandleMode>`;
    if (ports.defaultOutput != null) type += ['input', 'project_input'].includes(call.spelling)
      ? ` & import("@circular/core").WritableBoundaryEndpoint<${portValueType(ports.outputs.find(p => p.id === ports.defaultOutput))}, NewHandleMode>` : ` & SourceEndpoint<${portValueType(ports.outputs.find(p => p.id === ports.defaultOutput))}, NewHandleMode>`;
    if (call.spelling === "assemble") type = 'ReturnType<typeof import("@circular/core").assemble>';
    if (call.childModule && !call.templateName) {
      const path = posix.relative(posix.dirname(module), call.childModule).replace(/\.ts$/, '');
      const relative = path.startsWith('.') ? path : './' + path;
      type += ` & { readonly actors: typeof import(${JSON.stringify(relative)}) }`;
    }
    if (bindingIdentifier(binding.name)) lines.push(`export declare let ${binding.name}: ${type};`);
    else {
      const id = `__binding${ordinal++}`;
      lines.push(`declare let ${id}: ${type};`, `export { ${id} as ${JSON.stringify(binding.name)} };`);
    }
  }
  const modules = module === program.executable.entry && program.executable.modules?.size > 1
    ? new Map([...program.executable.modules.keys()].map(path => {
      const declaration = path === module ? null : generateProgramDeclarations(program, table, path);
      return declaration ? [declaration.path, declaration.text] : [path.replace(/\.ts$/, '.d.ts'), lines.join('\n') + '\n'];
    })) : null;
  return Object.freeze({ ...(modules ? { modules } : {}), path: module.replace(/\.(?:[cm]?ts|[cm]?js|tsx|jsx)$/, '') + '.d.ts', text: lines.join('\n') + '\n' });
}
