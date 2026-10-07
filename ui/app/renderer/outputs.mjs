import { key, viewContext } from './scene.mjs';
import { codeText } from './reasons.mjs';
import { surfaceTitle } from './surface.mjs';

function spelledName(name) {
  const words = String(name).replace(/(\p{Ll}|\p{N})(\p{Lu})/gu, '$1 $2').split(/[\s._-]+/u).filter(Boolean)
    .map(word => /^\p{Lu}\p{Ll}/u.test(word) ? word.toLowerCase() : word).join(' ');
  return words ? words.charAt(0).toUpperCase() + words.slice(1) : String(name);
}

export function outputsFromScene(graph, domId, connectionCode, ended = null) {
  if (!graph) return { surfaces: [], code: codeText(connectionCode ?? 'READ_UNAVAILABLE') };
  const context = viewContext(graph);
  const surfaces = graph.exportMounts.map(mount => {
    const bindings = Object.entries(mount.declaration.roles).filter(([, binding]) => binding != null);
    const views = bindings.map(([role, binding]) => ({ role, port: binding.port, actor: domId(key(binding.actor)),
      node: graph.nodes.find(node => node.id === key(binding.actor)),
      context }));
    return {
      id: domId(key(mount.address)), name: surfaceTitle(mount.declaration.surface) ?? spelledName(mount.address.local),
      address: mount.address, declaration: mount.declaration, views, ended,
      submitCode: connectionCode ?? null,
    };
  });
  return { surfaces, count: graph.exportMounts.length,
    cursor: graph.anchor.cursor === undefined ? undefined : String(graph.anchor.cursor),
    terminal: graph.snapshotPage.terminal,
    code: connectionCode == null ? null : codeText(connectionCode) };
}
