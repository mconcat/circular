import { registerHooks } from 'node:module';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { readRecent, rememberState, chooseState, createState, installedCli, RECENT_FILE } from '../bridge/desktop.mjs';

let sequence = 0;
export async function launchShell({ entry = 'main.mjs', args = [], named = false, capture = false, code = 'SOCKET_MISSING', recentWriteCode } = {}) {
  const id = ++sequence, key = `circularShellTest${id}`;
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'circular-shell-test-'));
  const paths = { appData: path.join(root, 'device'), temp: root, exe: path.join(root, 'Circular') };
  const handlers = new Map(), appEvents = new Map(), calls = [], loads = [], menus = [], styles = [];
  let ready, quitDone;
  const quit = new Promise(resolve => { quitDone = resolve; });
  const app = {
    isPackaged: false, setName() {}, getPath: name => paths[name], setPath: (name, value) => { paths[name] = value; },
    setAppLogsPath() {}, commandLine: { getSwitchValue: () => named ? path.join(root, 'named') : '', appendSwitch() {} },
    on: (name, handler) => appEvents.set(name, handler),
    whenReady: () => ({ then(fn) { ready = Promise.resolve().then(fn); return ready; } }),
    quit: () => appEvents.get('will-quit')({ preventDefault() {} }), exit: status => quitDone(status),
  };
  const webContents = {
    setWindowOpenHandler() {}, on() {}, isDestroyed: () => false, send() {}, insertCSS: async css => { styles.push(css); },
    executeJavaScript: async () => true, capturePage: async () => ({ toPNG: () => Buffer.from('capture') }),
  };
  class BrowserWindow {
    webContents = webContents;
    async loadFile(file, options) { loads.push({ file, ...options }); }
    show() {}
    close() { app.quit(); }
  }
  const dial = method => async options => { calls.push({ method, ...options }); throw { code }; };
  globalThis[key] = {
    app, BrowserWindow, ipcMain: { handle: (name, handler) => handlers.set(name, handler) },
    Menu: { buildFromTemplate: menu => menu, setApplicationMenu: menu => menus.push(menu) },
    dialog: { showOpenDialog: async () => ({ canceled: true, filePaths: [] }) },
    shell: { showItemInFolder: item => calls.push({ method: 'showItemInFolder', item }) },
    connect: dial('connect'), attachState: dial('attachState'), startDaemon: dial('startDaemon'), stopDaemon: dial('stopDaemon'),
    readRecent, rememberState: async (...options) => {
      if (recentWriteCode) throw { code: recentWriteCode };
      return rememberState(...options);
    },
    chooseState, createState, installedCli, RECENT_FILE,
  };
  const moduleURL = names => 'data:text/javascript,' + encodeURIComponent(
    `const host = globalThis[${JSON.stringify(key)}];\n` + names.map(name => `export const ${name} = host.${name};`).join('\n'));
  const hooks = registerHooks({
    resolve(specifier, context, next) {
      if (specifier === 'electron') return { url: moduleURL(['app', 'BrowserWindow', 'ipcMain', 'Menu', 'dialog', 'shell']), shortCircuit: true };
      if (specifier.endsWith('/bridge/desktop.mjs')) return {
        url: moduleURL(['attachState', 'startDaemon', 'stopDaemon', 'readRecent', 'rememberState', 'chooseState', 'createState', 'installedCli', 'RECENT_FILE']), shortCircuit: true,
      };
      if (specifier === './bridge/frames.mjs' && new URL(context.parentURL).pathname.endsWith('/main.mjs'))
        return { url: moduleURL(['connect']), shortCircuit: true };
      const resolved = next(specifier, context);
      if (resolved.url.endsWith('/shell.mjs')) return { ...resolved, url: `${resolved.url}?case=${id}` };
      return resolved;
    },
  });
  const argv = process.argv;
  const signals = new Map(['SIGINT', 'SIGTERM'].map(signal => [signal, new Set(process.listeners(signal))]));
  const close = async () => {
    await handlers.get('frames:close')?.({}, (await handlers.get('circular:connection')?.({}))?.attachment);
    hooks.deregister();
    process.argv = argv;
    for (const [signal, previous] of signals) for (const listener of process.listeners(signal))
      if (!previous.has(listener)) process.removeListener(signal, listener);
    delete globalThis[key];
    await fs.rm(root, { recursive: true, force: true });
  };
  try {
    process.argv = ['electron', new URL('../' + entry, import.meta.url).pathname, ...args,
      ...(capture ? ['--capture', path.join(root, 'capture.png')] : [])];
    await import(new URL(`../${entry}?case=${id}`, import.meta.url));
    await ready;
    return { root, paths, loads, calls, menus, styles, app, quit, close, webContents,
      ask: request => handlers.get('circular:connection')({}, request),
      send: (attachment, bytes) => handlers.get('frames:send')({}, attachment, bytes),
    };
  } catch (error) { await close(); throw error; }
}
