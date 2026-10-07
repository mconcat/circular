import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import vm from 'node:vm';
import { recordedStudyEntry, recordedStudyFiles } from '../scripts/recorded-study.mjs';
import { fixtureSource } from '../fixture-source.mjs';
import { installViewers } from '../renderer/views.mjs';

test('shot-diff recorded document boots the existing fixture port on its supplied study', async () => {
  const files = recordedStudyFiles({
    study: { root: { name: 'Recorded desk', nodes: [], edges: [], notes: [] }, journal: [] },
    catalog: [], history: { start: 90, duration: 7, stages: [] },
  });
  const entry = new URL(recordedStudyEntry + '?fixture=1&scene=canvas', 'http://capture.invalid');
  const loaded = [], window = {};
  const context = vm.createContext({ window, URLSearchParams, location: { search: entry.search },
    fixturePort: { fixtureSource: () => fixtureSource(window) }, views: { installViewers },
    document: {
      fonts: { ready: Promise.resolve() }, querySelectorAll: () => [], createElement: () => ({}),
      head: { append(script) {
        loaded.push(script.src);
        const resource = files(new URL(script.src, entry).pathname);
        if (resource?.source !== undefined) vm.runInContext(resource.source, context);
        script.onload();
      } },
    },
  });
  const bootstrap = await fs.readFile(new URL('../bootstrap.js', import.meta.url), 'utf8');
  vm.runInContext(bootstrap.replace("await import('./fixture-source.mjs')", 'fixturePort')
    .replace("await import('./renderer/views.mjs')", 'views')
    .replace("await import('./renderer/layout.mjs')", '({})').replace("await import('./renderer/tier.mjs')", '({})').replace("await import('./renderer/card-size.mjs')", '({documentMetrics:async()=>null})'), context);
  assert.equal(await window.circularReady, true);
  assert.equal(window.STUDY.root.name, 'Recorded desk');
  assert.deepEqual(JSON.parse(JSON.stringify(window.STUDY.root.nodes)), []);
  assert.deepEqual(JSON.parse(JSON.stringify(window.PUBLISHED_ACTORS)), []);
  assert.equal(window.StudySource.head, 7);
  assert.equal(window.STUDY_HISTORY.start, 90);
  assert.equal(window.StudyFixture, undefined, 'a recorded study has no synthetic progressor');
  assert.deepEqual(loaded, ['fixture.js', 'catalog-data.js', 'time-fixture.js',
    'icons.js', 'product-catalog.js', 'wire-field.js', 'time-machine.js', 'app.js', 'product.js']);
  assert.deepEqual(files('/recorded-study/renderer/views.mjs'), { pathname: '/renderer/views.mjs', source: undefined });
  assert.equal(files('/fixture.js'), undefined, 'the ordinary fixture document uses its own samples');
  assert.equal(files('/index.html'), undefined, 'the product document is not a recorded-study entry');
});

test('shot-diff rejects an input without a study before creating its document', () => {
  assert.throws(() => recordedStudyFiles({ catalog: [], history: {} }), /Static study input is/);
});
