import { reason, severity } from './reasons.mjs';
import { onNextAction } from './reconnect.mjs';

export const noticeText = ({ label, count }) => `${label}${count > 1 ? ` · ${count}×` : ''}`;

export function toastQueue({ draw, retext, clear, onAction = () => () => {} }) {
  const waiting = [];
  let shown, release;
  const drawShown = () => draw(noticeText(shown), shown.action, shown.severity, shown.code, shown.sentence);
  function show(notice) {
    release?.(); release = undefined;
    shown = notice;
    drawShown();
    if (shown.refusal && !shown.acknowledged) release = onAction(acknowledged);
  }
  function acknowledged() {
    release = undefined;
    if (!shown) return;
    shown.acknowledged = true;
    if (waiting.length) show(waiting.shift());
  }
  return {
    notify(code, { refusal = false, answer = refusal, detail, action, sentence } = {}) {
      const { code: value, label } = reason(code);
      console.info(value, ...(detail === undefined ? [] : [detail]));
      const held = [shown, ...waiting].find(notice => notice && !notice.acknowledged
        && notice.code === value && notice.refusal === refusal);
      if (held) {
        held.count += 1;
        if (detail !== undefined) { held.detail = detail; held.details.push(detail); }
        const said = held.sentence;
        if (sentence === undefined) delete held.sentence; else held.sentence = sentence;
        if (held === shown) {
          if (held.sentence === said) retext(noticeText(held));
          else drawShown();
        }
        return;
      }
      const notice = { code: value, label, severity: severity(code), count: 1, refusal,
        details: detail === undefined ? [] : [detail], ...(detail === undefined ? {} : { detail }),
        ...(sentence === undefined ? {} : { sentence }), ...(action ? { action } : {}) };
      if (!shown) return show(notice);
      if (shown.acknowledged) {
        waiting.push(notice);
        return show(waiting.shift());
      }
      if (!answer || shown.refusal) return void waiting.push(notice);
      waiting.unshift(shown);
      show(notice);
    },
    hidden() {
      if (shown?.refusal && !shown.acknowledged) return show(shown);
      if (waiting.length) return show(waiting.shift());
      release?.(); release = undefined;
      shown = undefined;
      clear();
    },
    get notices() {
      return [shown, ...waiting].filter(Boolean).map(({ code, label, severity, count, refusal, detail, details, sentence }) =>
        ({ code, label, severity, count, refusal, ...(detail === undefined ? {} : { detail }),
          ...(details.length > 1 ? { details: [...details] } : {}), ...(sentence === undefined ? {} : { sentence }) }));
    },
    stop() { release = undefined; shown = undefined; waiting.length = 0; },
  };
}

export function chipToasts() {
  const chip = () => globalThis.document?.querySelector?.('#toast');
  const watched = new WeakSet();
  const queue = toastQueue({
    draw(text, action, severity, code, sentence) { watch(chip()); globalThis.window?.StudyApp?.toast(text, action, severity, code, sentence); },
    retext(text) { const words = chip()?.querySelector?.('span'); if (words) words.textContent = text; },
    clear() {
      const el = chip();
      if (!el) return;
      const empty = () => { if (!el.classList.contains('visible')) { el.replaceChildren(); delete el.dataset.reason; } };
      const exit = globalThis.window?.getComputedStyle?.(el).transitionDuration ?? '0s';
      if (exit.split(',').some(value => parseFloat(value) > 0)) el.addEventListener('transitionend', empty, { once: true });
      else empty();
    },
    onAction: fn => onNextAction(globalThis.document, fn),
  });
  function watch(el) {
    const Observer = globalThis.window?.MutationObserver;
    if (!el || !Observer || watched.has(el)) return;
    watched.add(el);
    new Observer(records => {
      const was = records.some(record => /(^|\s)visible(\s|$)/.test(record.oldValue ?? ''));
      if (was && !el.classList.contains('visible')) queue.hidden();
    }).observe(el, { attributes: true, attributeFilter: ['class'], attributeOldValue: true });
  }
  return queue;
}
