import { contextBridge, ipcRenderer } from 'electron';
contextBridge.exposeInMainWorld('circularConnection', request => ipcRenderer.invoke('circular:connection', request));
contextBridge.exposeInMainWorld('circularFrames', {
  send: (attachment, bytes) => ipcRenderer.invoke('frames:send', attachment, bytes),
  onFrame: handler => { ipcRenderer.on('frames:incoming', (_event, attachment, bytes) => handler(attachment, bytes)); },
  close: attachment => ipcRenderer.invoke('frames:close', attachment),
});
