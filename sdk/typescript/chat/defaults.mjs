import fs from 'node:fs';
import path from 'node:path';

export const CHAT_DIRECTORY = 'chat';
export const DEFAULTS_FILE = 'defaults.json';
export const DEFAULTS_VERSION = 1;

export const defaultsPath = state => path.join(state, CHAT_DIRECTORY, DEFAULTS_FILE);

export function readChatDefaults(state) {
  const file = defaultsPath(state);
  let text;
  try { text = fs.readFileSync(file, 'utf8'); }
  catch (error) {
    if (error.code === 'ENOENT') return { agentCli: null, path: file, diagnostic: null };
    return { agentCli: null, path: file, diagnostic: `unreadable: ${error.message}` };
  }
  let value;
  try { value = JSON.parse(text); }
  catch (error) { return { agentCli: null, path: file, diagnostic: `not JSON: ${error.message}` }; }
  if (value?.version !== DEFAULTS_VERSION) {
    return { agentCli: null, path: file, diagnostic: `unknown document version ${JSON.stringify(value?.version)}` };
  }
  if (typeof value.agentCli !== 'string' || value.agentCli.length === 0) {
    return { agentCli: null, path: file, diagnostic: 'agentCli is absent or not a name' };
  }
  return { agentCli: value.agentCli, path: file, diagnostic: null };
}

export function writeChatDefault(state, agentCli) {
  const directory = path.join(state, CHAT_DIRECTORY);
  try { fs.mkdirSync(directory, { mode: 0o700 }); }
  catch (error) { if (error.code !== 'EEXIST') throw error; }
  const file = defaultsPath(state);
  const temporary = `${file}.${process.pid}.tmp`;
  fs.writeFileSync(temporary, `${JSON.stringify({ version: DEFAULTS_VERSION, agentCli }, null, 2)}\n`, { mode: 0o600 });
  fs.renameSync(temporary, file);
  return file;
}
