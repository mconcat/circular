export function installCaptureFrame() {
  let now = 0, sequence = 0;
  const timers = new Map(), frames = new Map();
  performance.now = () => now;
  Date.now = () => now;
  Math.random = () => 0.5;
  const timer = (callback, delay, repeat, args) => {
    const id = ++sequence;
    timers.set(id, {callback, at:now + delay, repeat, args});
    return id;
  };
  window.setTimeout = (callback, delay = 0, ...args) => timer(callback, delay, 0, args);
  window.setInterval = (callback, delay, ...args) => timer(callback, delay, delay, args);
  window.clearTimeout = window.clearInterval = id => timers.delete(id);
  window.requestAnimationFrame = callback => { const id = ++sequence; frames.set(id, callback); return id; };
  window.cancelAnimationFrame = id => frames.delete(id);
  const paint = () => {
    for (const [id, callback] of [...frames]) {
      if (!frames.delete(id)) continue;
      callback(now);
    }
  };
  window.captureLayout = async () => {
    paint(); await Promise.resolve(); paint(); await Promise.resolve();
  };
  window.captureFrame = async () => {
    for (let frame = 1; frame <= 480; frame++) {
      const at = frame * 1000 / 60;
      for (;;) {
        const next = [...timers].filter(([,t]) => t.at <= at).sort((a,b) => a[1].at - b[1].at)[0];
        if (!next) break;
        const [id, t] = next;
        now = t.at;
        if (t.repeat) t.at += t.repeat; else timers.delete(id);
        t.callback(...t.args);
        await Promise.resolve();
      }
      now = at;
      paint();
      await Promise.resolve();
    }
    return {elapsed:now, frames:480};
  };
}
