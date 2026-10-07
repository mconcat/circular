import path from 'node:path';
import fs from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import { app, BrowserWindow, ipcMain, Menu, dialog, shell } from 'electron';
import { frameRelay } from './bridge/frames.mjs';

import { readRecent, rememberState, chooseState, createState, installedCli, stopDaemon, RECENT_FILE } from './bridge/desktop.mjs';

export const MINIMUM_WINDOW = Object.freeze({ width: 901, height: 609 });

export async function runShell({ openTransport, initialState, args = process.argv }) {
  const here = import.meta.dirname;
  const brandRoot = pathToFileURL(path.resolve(here,
    app.isPackaged ? '../assets/brand' : '../../assets/brand') + path.sep).href;
  const value = flag => args.includes(flag) ? args[args.indexOf(flag) + 1] : undefined;
  const fixture = args.includes('--fixture');
  const shot = value('--capture');
  let state = initialState ?? value('--state'), connectionCode, attaching = false;
  const scene = value('--scene') ?? 'canvas';
  app.setName('Circular');
  const named = app.commandLine.getSwitchValue('user-data-dir') || undefined;
  if (!named && (fixture || shot || state !== undefined)) {
    console.error('profile', 'PROFILE_REQUIRED: a launch that names its state, a fixture or a capture must also name its profile with --user-data-dir');
    app.exit(1);
    return;
  }
  app.setPath('userData', named ?? path.join(app.getPath('appData'), 'Circular'));
  if (named) app.setPath('sessionData', named);
  if (shot) app.commandLine.appendSwitch('force-device-scale-factor', '1');
  const recentFile = path.join(app.getPath('userData'), RECENT_FILE);
  const cli = installedCli(app.getPath('exe'));
  let connectionEvidence;
  let recent = { paths: [] };
  try { recent = { paths: await readRecent(recentFile) }; }
  catch (error) { recent = { paths: [], code: error.code ?? 'RECENT_READ_FAILED' }; }
  let documentFrames;
  const owner = frameRelay((attachment, bytes) => { if (documentFrames && !documentFrames.isDestroyed()) documentFrames.send('frames:incoming', attachment, bytes); });
  let reattach = async () => {}, reopen = async () => {}, create = async () => ({ created: false }), open = async () => ({ opened: false });
  let attachment = Promise.resolve();
  ipcMain.handle('circular:connection', async (_event, request) => {
    if (request?.create === true) return create();
    if (request?.open === true) return open();
    if (request?.choose === 'program') {
      const name = typeof request.name === 'string' && /^[A-Za-z0-9._-]+$/.test(request.name) ? request.name : undefined;
      const chosen = await dialog.showOpenDialog({ title: name ? `Choose the ${name} program` : 'Choose a program',
        message: name ? `In a terminal, command -v ${name} prints its path.` : undefined,
        buttonLabel: 'Choose', properties: ['openFile', 'showHiddenFiles'] });
      return chosen.canceled ? null : chosen.filePaths[0] ?? null;
    }
    if (request?.daemon === 'stop') {
      if (!state || fixture) return { code: 'STATE_REQUIRED' };
      if (attaching) return { code: 'ATTACHING' };
      try { return await stopDaemon({ state, cli }); }
      catch (error) { return { code: error.code ?? 'CLI_UNAVAILABLE', ...(error.evidence ? { evidence: error.evidence } : {}) }; }
    }
    if (request?.reveal === 'config') {
      if (!state || fixture) return { code: 'STATE_REQUIRED' };
      const file = path.join(state, 'config.toml');
      const present = await fs.stat(file).then(entry => entry.isFile(), () => false);
      shell.showItemInFolder(present ? file : state);
      return { revealed: present ? 'config' : 'state' };
    }
    if (typeof request?.state === 'string') {
      if (!recent.paths.includes(request.state)) return { connection: 'STATE_REQUIRED' };
      void reopen(request.state);
      return { connection: 'ATTACHING' };
    }
    if (request?.attach === true) await reattach({ start: request.start === true });
    await attachment;
    const answer = await owner.connection();
    return answer.connection !== 'connected' && connectionEvidence ? { ...answer, evidence: connectionEvidence } : answer;
  });
  ipcMain.handle('frames:send', (_event, attachment, bytes) => owner.send(attachment, bytes));
  ipcMain.handle('frames:close', (_event, attachment) => owner.close(attachment));

  app.on('will-quit', event => {
    event.preventDefault();
    owner.end().catch(error => console.error('session-end', error?.code ?? 'READ_UNAVAILABLE'))
      .finally(() => { app.exit(process.exitCode ?? 0); });
  });
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => app.quit());

  app.on('window-all-closed', () => app.quit());
  app.whenReady().then(ready).catch(error => { console.error(error); app.exit(1); });
  async function ready() {
    if (!app.isPackaged && app.dock) {
      app.dock.setIcon(path.join(here, 'build/dock.png'));
    }
    const window = new BrowserWindow({
      useContentSize: true, width: 1600, height: 1050, show: false,
      minWidth: MINIMUM_WINDOW.width, minHeight: MINIMUM_WINDOW.height,
      backgroundColor: '#101114', title: 'Circular',
      titleBarStyle: 'hiddenInset',
      webPreferences: { preload: path.join(here, 'preload.mjs'), contextIsolation: true,
        nodeIntegration: false, sandbox: false, spellcheck: false, backgroundThrottling: !shot },
    });
    documentFrames = window.webContents;
    window.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
    window.webContents.on('will-navigate', event => event.preventDefault());
    if (shot && fixture) {
      window.webContents.debugger.attach('1.3');
      await window.webContents.debugger.sendCommand('Page.enable');
      await window.webContents.debugger.sendCommand('Page.addScriptToEvaluateOnNewDocument', {
        source: 'performance.now = () => 0; Date.now = () => 0;',
      });
    }
    async function loadCanvas({ created = false } = {}) {
      await window.loadFile(path.join(here, 'index.html'), {
        query: { scene, brandRoot, ...(fixture ? { fixture: '1' } : {}), ...(created ? { created: '1' } : {}),
          ...(connectionCode ? { connectionCode } : {}), ...(state ? { state } : {}),
          ...(fixture ? {} : { recent: JSON.stringify(recent.paths), ...(recent.code ? { recentCode: recent.code } : {}) }),
          ...(value('--theme') ? { theme: value('--theme') } : {}) }, hash: state || fixture ? scene : 'projects',
      });
      await window.webContents.insertCSS(`
      ${process.platform === 'darwin' ? ':root { --window-controls-inset: 88px; }' : ''}
      :root .appbar { -webkit-app-region: drag; }
      .appbar a, .appbar button, .appbar input, .appbar select { -webkit-app-region: no-drag; }
      `);
    }
    async function changeState({ chosen, start = false, created = false } = {}) {
      if (attaching || fixture) return;
      attaching = true;
      connectionEvidence = undefined;
      let settled;
      attachment = new Promise(resolve => { settled = resolve; });
      try {
        if (chosen) state = chosen;
        try { recent = { paths: await rememberState(recentFile, state) }; }
        catch (error) { recent = { paths: recent.paths, code: error.code ?? 'RECENT_WRITE_FAILED' }; }
        installMenu();
        connectionCode = undefined;
        await loadCanvas({ created });
        if (!shot) window.show();
        const opening = owner.attach(() => openTransport({ state, cli, start, capture: Boolean(shot) }));
        try { await opening; connectionCode = undefined; }
        catch (error) { connectionCode = error.code ?? 'READ_UNAVAILABLE'; connectionEvidence = error.evidence; }
      } finally { attaching = false; settled(); }
    }
    reopen = chosen => changeState({ chosen });
    create = async () => {
      if (attaching || fixture) return { code: 'ATTACHING' };
      let chosen;
      try { chosen = await createState(dialog, { recent: recent.paths, home: app.getPath('home') }); }
      catch (error) { return { code: error.code ?? 'PROJECT_CREATE_FAILED' }; }
      if (chosen === undefined) return { created: false };
      void changeState({ chosen, start: true, created: true });
      return { created: true };
    };
    open = async () => {
      if (attaching || fixture) return { code: 'ATTACHING' };
      const chosen = await chooseState(dialog);
      if (chosen === undefined) return { opened: false };
      if (chosen === state) void sameState('connection');
      else void changeState({ chosen });
      return { opened: true };
    };
    const act = action => {
      const name = JSON.stringify(action);
      return window.webContents.executeJavaScript(
        `Boolean(window.Product?.actions?.[${name}]) && (window.Product.actions[${name}](), true)`).catch(() => false);
    };
    const press = selector => window.webContents.executeJavaScript(
      `(control => Boolean(control) && (control.click(), true))(document.querySelector(${JSON.stringify(selector)}))`).catch(() => false);
    async function sameState(action, options) {
      if (!state || fixture) return;
      if (!(await act(action))) await changeState(options);
    }
    function installMenu() {
      const project = Boolean(state) && !fixture;
      const named = recent.paths.map(at => [at, path.basename(at.replace(/\/+$/, '')) || at]);
      const recentItems = named.map(([at, name]) => ({
        label: named.filter(([, other]) => other === name).length > 1 ? `${name} — ${path.dirname(at)}` : name,
        toolTip: at, click: () => at === state ? press('.view-switch [data-view="canvas"]') : reopen(at),
      }));
      Menu.setApplicationMenu(Menu.buildFromTemplate([
        { role: 'appMenu' },
        { label: 'File', submenu: [
          { label: 'New Project…', accelerator: 'CmdOrCtrl+N', click: () => act('new-project') },
          { label: 'Open Project…', accelerator: 'CmdOrCtrl+O', click: () => act('open-project') },
          { label: 'Open Recent', enabled: recentItems.length > 0, submenu: recentItems },
          { type: 'separator' },
          { label: 'Reconnect', enabled: project, click: () => sameState('connection') },
          { label: 'Start Daemon', enabled: project, click: () => sameState('start-daemon', { start: true }) },
          { type: 'separator' },
          { role: 'close' },
        ] },
        { label: 'Edit', submenu: [
          { label: 'Undo', accelerator: 'CmdOrCtrl+Z', click: (_item, focused, event) =>
            event?.triggeredByAccelerator ? focused?.webContents.undo() : act('undo') },
          { label: 'Redo', accelerator: 'Shift+CmdOrCtrl+Z', click: (_item, focused, event) =>
            event?.triggeredByAccelerator ? focused?.webContents.redo() : act('redo') },
          { type: 'separator' },
          { role: 'cut' }, { role: 'copy' }, { role: 'paste' }, { role: 'selectAll' },
        ] },
        { label: 'View', submenu: [
          ...[['projects', 'Projects', '1'], ['canvas', 'Canvas', '2'], ['outputs', 'Outputs', '3']].map(([view, label, key]) =>
            ({ label, accelerator: `CmdOrCtrl+${key}`, enabled: project, click: () => press(`.view-switch [data-view="${view}"]`) })),
          { type: 'separator' },
          { role: 'togglefullscreen' },
        ] },
        { role: 'windowMenu' },
        { role: 'help', submenu: [
          { label: 'Keyboard Shortcuts', enabled: project, click: () => press('#help-button') },
        ] },
      ]));
    }
    async function reattachSession({ start = false } = {}) {
      if (attaching || fixture || !state) return;
      attaching = true;
      connectionEvidence = undefined;
      try {
        const opening = owner.attach(() => openTransport({ state, cli, start, capture: Boolean(shot) }));
        try { await opening; connectionCode = undefined; }
        catch (error) { connectionCode = error.code ?? 'READ_UNAVAILABLE'; connectionEvidence = error.evidence; }
      } finally { attaching = false; }
    }
    reattach = reattachSession;
    installMenu();
    if (fixture) await loadCanvas();
    else if (!state) {
      connectionCode = 'STATE_REQUIRED';
      await loadCanvas();
    } else await changeState();
    if (!shot) window.show();
    if (shot) {
      try {
        const ready = await window.webContents.executeJavaScript('window.circularReady');
        if (!fixture && ready !== true) throw new Error('Daemon scene did not connect; capture refused');
        if (!fixture) {
          const observed = await window.webContents.executeJavaScript("import('./renderer/adapter.mjs').then(adapter => adapter.observation)");
          if (observed !== true) throw new Error('Daemon scene could not be read; capture refused');
        }
        if (fixture) await window.webContents.executeJavaScript(await fs.readFile(path.join(here, 'scripts/canvas-fixture.js'), 'utf8'));
        await window.webContents.executeJavaScript(`(async () => {
          await document.fonts.ready;
          await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
        })()`);
        await fs.mkdir(path.dirname(path.resolve(shot)), { recursive: true });
        await fs.writeFile(path.resolve(shot), (await window.webContents.capturePage()).toPNG());
        console.log(`capture: ${path.resolve(shot)}`);
      } catch (error) { console.error(error); process.exitCode = 1; }
      window.close();
    }
  }
}
