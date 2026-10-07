import { noDaemon, reason, reasonText } from './reasons.mjs';

export const sessionLost = new Set(['SESSION_CLOSED', 'TRANSPORT_FAILED', 'STREAM_CLOSED', 'STREAM_FAILED', 'SEND_FAILED']);
export const awaitsDaemon = code => noDaemon.has(code) || code === 'DAEMON_NOT_ANSWERING' || sessionLost.has(code);
export const startRefused = code => ['CLI_UNAVAILABLE', 'CLI_START_FAILED', 'CLI_STATUS_UNAVAILABLE'].includes(code);

export const REATTACH_EVERY_MS = 1000;

export function reattacher({ ask, every = REATTACH_EVERY_MS, timers = globalThis }) {
  let timer = null, waiting = null;
  const later = () => { timer = timers.setTimeout(tick, every); timer?.unref?.(); };
  const tick = async () => {
    timer = null;
    if (!waiting) return;
    let attached = false;
    try { attached = await ask(); } catch { attached = false; }
    if (waiting && !attached && timer === null) later();
  };
  return {
    get waiting() { return waiting; },
    watch(fact = {}) {
      waiting = { tried: null, ...(waiting ?? {}), ...fact };
      if (timer === null) later();
    },
    tried(code) { if (waiting) waiting = { ...waiting, tried: code }; },
    stop() {
      waiting = null;
      if (timer !== null) timers.clearTimeout(timer);
      timer = null;
    },
  };
}

export function rejoinedText(rejoined) {
  if (!rejoined) return null;
  return `reattached ${rejoined.atText} after ${reasonText(rejoined.code)}`;
}
export function rejoinedFact(rejoined) {
  if (!rejoined) return null;
  return { text: `reattached ${rejoined.atText}`, code: reason(rejoined.code).code, title: reasonText(rejoined.code) };
}
