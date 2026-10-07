import fs from 'node:fs';
import vm from 'node:vm';
import { configFieldList } from '../renderer/config-fields.mjs';
import { reason } from '../renderer/reasons.mjs';
import { installViewers } from '../renderer/views.mjs';
import { catalogItems } from '../renderer/catalog.mjs';
import { configFormHTML } from '../renderer/config-form.mjs';
import { mockupFace, mockupInspect } from '../fixture-source.mjs';
import { outletReading } from '../renderer/arrivals.mjs';
import { cardViewer } from '../renderer/viewer.mjs';
import * as CanvasTier from '../renderer/tier.mjs';
export const anySlots = config => ({ slots: Object.keys(config ?? {}).map(key => ({ key, shape: { kind: 'Any' } })) });
export function productFields(config, declared = true, entry = anySlots(config)) {
  const ui = originalUI();
  ui.StudySource.codeLabel = reason;
  ui.StudySource.configFields = () => {
    const list = configFieldList(config, declared, entry);
    return { ...list, ...(list.code ? { code: reason(list.code) } : {}), fields: list.fields.map(f => f.code ? { ...f, code: reason(f.code) } : f) };
  };
  ui.StudySource.configForm = (n, draft) => configFormHTML(ui.StudySource.configFields(n.id), n, draft, ui.viewer.card(n.id).draft?.raw, reason);
  ui.StudySource.settingsDeclared = n => ui.StudySource.configFields(n.id).declared;
  return ui;
}
export function originalUI() {
  const port = { get projection() { return context.STUDY; }, outletReading, rows: [], catalog() { return this.rows; }, stepKinds: () => ({ kinds: [] }),
    actorRecords: id => context.A?.visibleRecords?.().filter(r => r.actor === id) ?? [],
    rawObservation(id) {
      const n = context.node?.id === id ? context.node : undefined;
      return n && { id: n.id, health: n.health, activity: n.activity, preview: n.preview };
    },
    choices: n => context.LiveViewers?.choices(n),
    face: n => mockupFace(n, Boolean(context.paused?.()) && !context.historical?.()),
    ...mockupInspect,
    healthBanner: (n, past) => mockupInspect.healthBanner(n, past, past ? context.studyTimeFormat(context.A.displayTime()) : null),
    codeLabel: reason };
  const context=vm.createContext({structuredClone,console, document:{}, STUDY:{}, StudySource:port, source:port, CanvasTier});
  let document;
  Object.defineProperty(context, 'document', {
    get: () => document,
    set: value => { document = new Proxy(value, { get(target, key) {
      if (key !== 'querySelectorAll') return target[key];
      return selector => {
        const found = target.querySelectorAll?.(selector) ?? [];
        const id = selector.match(/^\[data-view-host="([^"]+)"\]$/)?.[1];
        return found.length || id === undefined ? found : [target.getElementById?.('node-' + id)].filter(Boolean);
      };
    } }); },
  });
  context.document = {};
  context.window=context;
  installViewers(context);
  context.StudyViewer = context.viewer = cardViewer();
  context.drafted = () => false;
  vm.runInContext(fs.readFileSync(new URL('../product-catalog.js',import.meta.url),'utf8'),context);
  return context;
}
export function approvalHTML(rows) {
  const ctx=originalUI();let html;
  Object.assign(ctx,{P:{approvals:rows},A:{allNode:()=>null},e:ctx.ProductCatalog.escape,
    notice:(text,_kind,code)=>`${code} ${text}`, dialog:(_title,body)=>{html=body;}});
  const source=fs.readFileSync(new URL('../product.js',import.meta.url),'utf8');
  vm.runInContext(source.slice(source.indexOf('  function approvalDialog()'),source.indexOf('  async function decide('))+'\napprovalDialog();',ctx);
  return html;
}

export function answerCatalog(ui, items, iconFor) {
  ui.StudySource.rows = catalogItems(items, iconFor, type => ui.LiveViewers.select({ type }).kind);
}
