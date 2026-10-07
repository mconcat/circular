import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { pathToFileURL } from 'node:url';

const root = path.resolve(import.meta.dirname, '..');

export async function canvas(run, label = 'canvas', { search = '?fixture=1&theme=light', page = 'index.html', setup, onEvent, daemon,
  relay, daemonSearch = '?recent=%5B%5D&theme=light', cards: least = 1 } = {}) {
  const profile = await fs.mkdtemp(path.join(os.tmpdir(), 'chrome-canvas-'));
  const chrome = process.platform === 'darwin'
    ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : 'chromium';
  const child = spawn(chrome, ['--headless=new', '--no-first-run', '--no-default-browser-check',
    '--disable-background-networking', '--allow-file-access-from-files', '--remote-debugging-pipe',
    '--window-size=1600,1050', `--user-data-dir=${profile}`], {stdio:['ignore', 'ignore', 'pipe', 'pipe', 'pipe']});
  const closed = new Promise(resolve => child.once('close', resolve));
  const pending = new Map();
  let sequence = 0, buffer = '', errors = '', failure, bridge;
  const documents = new Map(), relays = [];
  let relayed;
  const fail = error => {
    failure ??= error;
    for (const request of pending.values()) request.reject(failure);
    pending.clear();
  };
  const end = id => {
    const document = documents.get(id);
    if (!document) return;
    documents.delete(id);
    document.ended = true;
    void document.connection.close().catch(fail);
  };
  child.on('error', fail);
  child.on('exit', (code, signal) => fail(new Error(`${label} Chrome exited (${code ?? signal}): ${errors}`)));
  child.stderr.setEncoding('utf8').on('data', chunk => { errors += chunk; });
  child.stdio[3].on('error', fail);
  child.stdio[4].on('error', fail);
  child.stdio[4].setEncoding('utf8').on('data', chunk => {
    buffer += chunk;
    let end;
    while ((end = buffer.indexOf('\0')) >= 0) {
      const line = buffer.slice(0, end); buffer = buffer.slice(end + 1);
      if (!line) continue;
      try {
        const message = JSON.parse(line), request = pending.get(message.id);
        if (!request) { bridge?.(message); onEvent?.(message); continue; }
        pending.delete(message.id);
        message.error ? request.reject(new Error(JSON.stringify(message.error))) : request.resolve(message.result);
      } catch (error) { fail(error); }
    }
  });
  const send = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
    if (failure) { reject(failure); return; }
    pending.set(++sequence, {resolve, reject});
    child.stdio[3].write(JSON.stringify({id:sequence, method, params, sessionId}) + '\0');
  });
  const deadline = setTimeout(() => fail(new Error(`${label}: no canvas observation arrived`)), 60000);
  try {
    const {targetId} = await send('Target.createTarget', {url:'about:blank'});
    const {sessionId} = await send('Target.attachToTarget', {targetId, flatten:true});
    const call = (method, params) => send(method, params, sessionId);
    await call('Page.enable');
    await call('Runtime.enable');
    if (onEvent) await call('Debugger.enable');
    await call('Emulation.setDeviceMetricsOverride', {width:1600, height:1050, deviceScaleFactor:1, mobile:false});
    const evaluate = async expression => {
      const result = await call('Runtime.evaluate', {expression, awaitPromise:true, returnByValue:true});
      assert.equal(result.exceptionDetails, undefined, JSON.stringify(result.exceptionDetails));
      return result.result.value;
    };
    if (setup) await call('Page.addScriptToEvaluateOnNewDocument', { source: setup });
    if (daemon) {
      const open = contextId => {
        const document = { connection: daemon.connect(), ended: false };
        relays.push((async () => {
          for await (const bytes of document.connection.incoming) {
            if (document.ended) continue;
            const result = await call('Runtime.evaluate', { contextId, returnByValue: true,
              expression: `window.__canvasIncoming(new Uint8Array(${JSON.stringify(Array.from(bytes))}))` })
              .catch(error => { if (!document.ended) throw error; });
            if (!document.ended) assert.equal(result.exceptionDetails, undefined, JSON.stringify(result.exceptionDetails));
          }
        })().catch(fail));
        documents.set(contextId, document);
        return document;
      };
      bridge = ({ method, params }) => {
        if (method === 'Runtime.executionContextDestroyed') end(params.executionContextId);
        if (method === 'Runtime.executionContextsCleared') for (const id of [...documents.keys()]) end(id);
        if (method !== 'Runtime.bindingCalled' || params.name !== '__canvasSend') return;
        const document = documents.get(params.executionContextId) ?? open(params.executionContextId);
        void document.connection.send(Uint8Array.from(JSON.parse(params.payload))).catch(fail);
      };
      await call('Runtime.addBinding', { name: '__canvasSend' });
      await call('Page.addScriptToEvaluateOnNewDocument', { source: `
        window.circularConnection = async () => ({ connection: 'connected' });
        {
          let handler = null, named;
          window.__canvasIncoming = bytes => { handler?.(named, bytes); };
          window.circularFrames = {
            send: (attachment, bytes) => { named = attachment; return window.__canvasSend(JSON.stringify(Array.from(bytes))); },
            onFrame: hook => { handler = hook; },
            close: () => {}
          };
        }
      ` });
    }
    if (relay) {
      const { frameRelay } = await import('../bridge/frames.mjs');
      const wire = value => value instanceof Uint8Array ? { bytes: Array.from(value) } : value === undefined ? { none: true } : value;
      const unwire = value => value?.bytes ? Uint8Array.from(value.bytes) : value?.none ? undefined : value;
      const literal = value => `JSON.parse(${JSON.stringify(JSON.stringify(wire(value)))})`;
      let toPage = Promise.resolve(), held = null, replacing = false;
      const hand = expression => { toPage = toPage.then(() => evaluate(expression)).catch(fail); return toPage; };
      const deliver = args => hand(`window.__canvasIncoming(${args.map(literal).join(', ')})`);
      const release = () => { const late = held ?? []; held = null; relayed.late += late.length; for (const each of late) void deliver(each); };
      const owner = frameRelay((...args) => {
        if (replacing) { held.push(args); return; }
        void deliver(args);
        if (held) release();
      });
      relayed = { attachments: 0, late: 0 };
      const open = () => { relayed.attachments += 1; return relay.connect(); };
      const answer = (id, value) => hand(`window.__canvasAnswer(${id}, ${literal(value)})`);
      bridge = ({ method, params }) => {
        if (method !== 'Runtime.bindingCalled' || params.name !== '__canvasRelay') return;
        const { id, op, args } = JSON.parse(params.payload), given = args.map(unwire);
        void (async () => {
          if (op === 'send') return answer(id, await owner.send(...given));
          if (op === 'close') return answer(id, await owner.close(...given));
          if (given[0]?.attach === true) {
            replacing = true; held = [];
            await owner.attach(open).catch(() => {});
            replacing = false;
            await answer(id, await owner.connection());
            if ((await owner.connection()).connection !== 'connected' && held) release();
            return;
          }
          return answer(id, await owner.connection());
        })().catch(fail);
      };
      await owner.attach(open).catch(() => {});
      relayed.end = () => owner.end();
      await call('Runtime.addBinding', { name: '__canvasRelay' });
      await call('Page.addScriptToEvaluateOnNewDocument', { source: `
        {
          const wire = value => value instanceof Uint8Array ? { bytes: Array.from(value) } : value === undefined ? { none: true } : value;
          const unwire = value => value?.bytes ? Uint8Array.from(value.bytes) : value?.none ? undefined : value;
          const asked = new Map();
          let sequence = 0, handler = null;
          const ask = (op, args) => new Promise(resolve => { asked.set(++sequence, resolve);
            window.__canvasRelay(JSON.stringify({ id: sequence, op, args: args.map(wire) })); });
          window.__canvasAnswer = (id, value) => { const resolve = asked.get(id); asked.delete(id); resolve?.(unwire(value)); };
          window.__canvasIncoming = (...args) => { handler?.(...args.map(unwire)); };
          window.circularConnection = request => ask('connection', [request]);
          window.circularFrames = {
            send: (...args) => ask('send', args),
            onFrame: hook => { handler = hook; },
            close: (...args) => ask('close', args),
          };
        }
      ` });
    }
    const url = `${pathToFileURL(path.join(root, page)).href}${daemon || relay ? daemonSearch : search}`;
    await call('Page.navigate', {url});
    let stood = false;
    for (let attempt = 0; attempt < 200 && !stood; attempt++) {
      stood = await evaluate(`(async () => {
        if (!window.circularReady) return false;
        await window.circularReady;
        return document.querySelectorAll('#nodes > article.node').length >= ${least};
      })()`);
      if (!stood) await new Promise(resolve => setTimeout(resolve, 100));
    }
    assert.ok(stood, least ? 'no actor card appeared on the fixture canvas' : 'the document did not stand');
    const drop = async () => {
      for (const id of [...documents.keys()]) {
        end(id);
        const result = await call('Runtime.evaluate', { contextId: id, returnByValue: true, expression: 'window.__canvasIncoming(null)' });
        assert.equal(result.exceptionDetails, undefined, JSON.stringify(result.exceptionDetails));
      }
    };
    await run(evaluate, call, drop, relayed);
  } finally {
    clearTimeout(deadline);
    await Promise.resolve().then(() => relayed?.end()).catch(() => {});
    for (const id of [...documents.keys()]) end(id);
    await Promise.all(relays);
    child.kill();
    await closed;
    await fs.rm(profile, {recursive:true, force:true});
  }
}
