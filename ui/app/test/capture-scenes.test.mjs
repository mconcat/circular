import test from 'node:test';
import assert from 'node:assert/strict';
import vm from 'node:vm';
import fs from 'node:fs/promises';
const script = await fs.readFile(new URL('../scripts/canvas-fixture.js', import.meta.url), 'utf8');
for (const scene of ['canvas','dense','configure','approvals','error']) {
  test(`capture fixture selects ${scene} and preserves the requested dark theme`, async () => {
    const calls = [], actors = {research:{preview:{}},prompt:{}};
    const app = { state:{}, allNode:id=>actors[id], timeMachine:{head:0, refresh(){}}, archive:{advance(){}},
      renderGraph(){}, transformWorld(){}, renderInspector(){calls.push(['inspect',app.state.tab]);} };
    const context = {
      URLSearchParams, location:{search:`?scene=${scene}&theme=dark`},
      StudyApp:app,
      StudyViewer:{message(id,text){calls.push(['message',id,text]);}},
      Product:{async scenario(name){calls.push(['scenario',name]); app.state.zoom=.4;}, actions:{approvals(){calls.push(['approvals']);}}},
      Appearance:{async apply(theme){calls.push(['appearance',theme]);}},
      document:{documentElement:{dataset:{theme:'dark'}}, querySelector:()=>({classList:{remove(){}}})},
    };
    await vm.runInNewContext(script,context);
    assert.deepEqual(calls.at(-1),['appearance','dark']);
    assert.deepEqual(calls.find(c=>c[0]==='message'),['message','prompt','Please read the local sources.']);
    assert.deepEqual(actors.prompt,{});
    if (scene === 'dense') {assert.deepEqual(calls[0],['scenario','dense']);assert.equal(app.state.zoom,.4);}
    if (scene === 'configure') assert.ok(calls.some(c=>c[0]==='inspect' && c[1]==='configure'));
    if (scene === 'approvals') assert.ok(calls.some(c=>c[0]==='approvals'));
    if (scene === 'error') {assert.deepEqual(calls[0],['scenario','idle_failed']);assert.equal(app.state.selected,'tools');}
    if (scene === 'canvas') assert.equal(app.state.zoom,.85);
  });
}
