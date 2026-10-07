import { reason, reasonText } from './reasons.mjs';

export const configDeclared = schema => Array.isArray(schema) && schema[0] === 2n;

export function catalogItems(rows, iconFor, viewFor, groupFor = () => undefined) {
  return rows.map(row => {
    const icon = iconFor(row.presentation_role), grouped = groupFor(row.presentation_role);
    return {...row, type:row.actor_type, title:row.label, icon:icon ?? 'Box',
      iconDiagnostic:icon ? null : reason('ACTOR_ICON_UNAVAILABLE'),
      configDeclared:configDeclared(row.config_schema),
      group:grouped?.group ?? 'Actors', groupRank:grouped?.rank, view:viewFor(row.actor_type)};
  });
}

export const unjudged = 'UNJUDGED';
export const containerCardinality = row => Array.isArray(row?.presentation_role) && Number(row.presentation_role[0]) === 4
  ? row.presentation_role[1] : undefined;
export function paletteAvailability(row, entry, unread) {
  if (containerCardinality(row)) return { enabled: false, code: 'CREATE_BY_GROUPING', detail: row.unavailable_reason };
  if (row.creatable && row.unavailable_reason == null) return { enabled: true };
  if (entry?.slots?.length) return { enabled: true, settings: true };
  const code = entry === undefined ? unread ?? 'CREATE_INPUTS_UNAVAILABLE'
    : entry.code ? 'CREATE_INPUTS_UNAVAILABLE' : 'ADMISSION_UNAVAILABLE';
  return { enabled: false, code, detail: entry?.code ?? row.unavailable_reason };
}
export function bindCatalogAvailability(root, items, onWire = false, inputs, unread) {
  for (const button of root.querySelectorAll('#palette-results [data-add]')) {
    const row = items.find(item => item.actor_type === button.dataset.add);
    if (!row) continue;
    const availability = paletteAvailability(row, inputs?.get(row.actor_type), inputs ? undefined : unread ?? 'CREATE_INPUTS_UNREAD');
    const icon = row.iconDiagnostic ?? null;
    if (icon) button.dataset.iconCode = icon.code;
    if (!availability.enabled) {
      button.disabled = true;
      const why = reason(availability.code), line = button.querySelector('small');
      button.dataset.reason = why.code;
      if (line) { line.textContent = why.label; line.dataset.reason = why.code; }
      button.title = [why.label, availability.detail && reasonText(availability.detail), icon?.label].filter(Boolean).join(' — ');
      continue;
    }
    if (availability.settings) {
      const name = button.querySelector('strong');
      if (name && !name.textContent.endsWith('…')) name.textContent += '…';
    }
    const code = onWire ? reason(unjudged) : icon;
    if (code) { button.dataset.reason = code.code; button.title = [code.label, onWire && icon?.label].filter(Boolean).join(' — '); }
  }
}
