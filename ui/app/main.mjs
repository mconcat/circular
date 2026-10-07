import { runShell } from './shell.mjs';
import { connect } from './bridge/frames.mjs';
import { attachState, startDaemon } from './bridge/desktop.mjs';

await runShell({
  openTransport: ({ state, cli, start, capture }) =>
    capture ? connect({ state }) : (start ? startDaemon : attachState)({ state, cli }),
});
