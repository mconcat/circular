import fs from 'node:fs';
import vm from 'node:vm';
const root = new URL('../', import.meta.url);
const html = fs.readFileSync(new URL('index.html', root), 'utf8');
const { imports } = JSON.parse(html.match(/<script type="importmap">\s*([\s\S]*?)\s*<\/script>/)[1]);
const context = vm.createContext({ TextEncoder, TextDecoder, URL, setTimeout, clearTimeout });
context.input = fs.readFileSync(0, 'utf8');
vm.runInContext(`globalThis.fixture = JSON.parse(input, (_k, v) => v?.$bigint ? BigInt(v.$bigint) : v?.$bytes ? new Uint8Array(v.$bytes) : v);`, context);
const modules = new Map();
function load(url) {
  if (modules.has(url.href)) return modules.get(url.href);
  const source = fs.readFileSync(url, 'utf8');
  const module = url.pathname.endsWith('.json')
    ? new vm.SyntheticModule(['default'], function () { this.setExport('default', JSON.parse(source)); }, { context, identifier: url.href })
    : new vm.SourceTextModule(source, { context, identifier: url.href, importModuleDynamically: async (specifier, parent) => {
      const module = resolve(specifier, parent);
      if (module.status === 'unlinked') await module.link(resolve);
      await module.evaluate();
      return module;
    } });
  modules.set(url.href, module);
  return module;
}
const entry = new vm.SourceTextModule(`
  import { snapshotToScene } from './renderer/scene.mjs';
  import { UNMEASURED } from './renderer/card-size.mjs';
  import { projectSDKProgram, sdkProgram } from './renderer/sdk-program.mjs';
  const graph = snapshotToScene(fixture.snapshot, fixture.ports, fixture.catalog, null, { items: [] }, UNMEASURED);
  graph.sdkProgram = projectSDKProgram(graph);
  await graph.sdkProgram.ready;
  export const result = sdkProgram(graph, { scope: [], local: 'result' });
`, { context, identifier: new URL('sdk-tab-witness.mjs', root).href });
function resolve(specifier, parent) {
  if (specifier.startsWith('.')) return load(new URL(specifier, parent.identifier));
  if (!imports[specifier]) throw new Error(`Unmapped browser dependency: ${specifier}`);
  return load(new URL(imports[specifier], root));
}
await entry.link(resolve);
await entry.evaluate();
if ('ts' in context || [...modules.keys()].some(url => /typescript\/lib|typescript\.mjs/.test(url))) throw new Error('printer loaded TypeScript');
process.stdout.write(JSON.stringify(entry.namespace.result));
