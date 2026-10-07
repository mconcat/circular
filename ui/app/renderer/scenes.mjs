import { readScene } from './scene.mjs';
import { UNMEASURED } from './card-size.mjs';
const definitions = await Promise.all([
  './scenes/canvas.mjs',
  './scenes/dense.mjs',
  './scenes/configure.mjs',
  './scenes/approvals.mjs',
  './scenes/error.mjs',
].map(path => import(path).then(module => module.default)));
export const scenes = new Map(definitions.map(scene => [scene.name, scene]));
export function sceneDefinition(name) {
  if (!scenes.has(name)) throw new Error(`Unknown scene: ${name}`);
  return scenes.get(name);
}
export async function readSurface(session, name, { scope = [], selected, observationScope } = {}) {
  const definition = sceneDefinition(name), graph = await readScene(session, scope, undefined, undefined, undefined, undefined, undefined, UNMEASURED);
  return definition.project(await definition.read(session, graph, observationScope ?? graph.anchor.scope), selected);
}
