import * as builders from '@circular/exports';
import { surfaceValue } from '@circular/exports/internal';
import { propertyKey } from './generator-syntax.js';

export const EXPORT_MARK_NAMES = new Set(['tab', 'window', 'grid', 'messages', 'terminal', 'label', 'transcript',
  'textInput', 'button', 'toggle', 'select', 'prompt', 'composer']);

export function printExportSurface(value, literal, prefix = '') {
  const atom = value => ({ text: literal(value), value });
  const call = (name, args) => ({ text: `${prefix}${name}(${args.map(a => a.text).join(', ')})`,
    value: builders[name](...args.map(a => a.value)) });
  let printed;
  if (!value || value.$circular !== 'export-definition') throw new TypeError('invalid export definition');
  const definition = builders.defineExport({ roles: value.roles, params: value.params,
    ...(value.operations === null ? {} : { operations: value.operations }),
    surfaces: (roles, params) => {
      const staticValue = v => v && typeof v === 'object' && v.$circular === 'parameter'
        && Object.keys(v).length === 2 && typeof v.name === 'string'
        ? { text: `params[${literal(v.name)}]`, value: params[v.name] } : atom(v);
      const role = v => {
        if (!v || v.$circular !== 'export-role' || Object.keys(v).length !== 2
          || !['request', 'progress', 'result', 'error'].includes(v.role)) throw new TypeError('invalid export role token');
        return { text: `roles.${v.role}`, value: roles[v.role] };
      };
      const object = v => {
        const entries = Object.keys(v).sort().map(k => [k, staticValue(v[k])]);
        return { text: `{ ${entries.map(([k, v]) => `${propertyKey(k)}: ${v.text}`).join(', ')} }`,
          value: Object.fromEntries(entries.map(([k, v]) => [k, v.value])) };
      };
      const mark = m => {
        if (!m || !EXPORT_MARK_NAMES.has(m.mark)) throw new TypeError('invalid export mark');
        const s = m.spec ?? {}, children = m.children ?? [];
        let args;
        if (['tab', 'window'].includes(m.mark)) {
          if (children.length !== 1) throw new TypeError('surface requires one child');
          args = [staticValue(s.title), mark(children[0])];
        } else if (m.mark === 'grid') {
          const items = children.map(mark);
          args = [object(s), { text: `[${items.map(m => m.text).join(', ')}]`, value: items.map(m => m.value) }];
        } else if (m.mark === 'label') args = [staticValue(s.value)];
        else if (m.mark === 'transcript') args = [role(s.role), ...(Object.hasOwn(s, 'own') ? [role(s.own)] : [])];
        else {
          const { role: r, ...config } = s;
          args = [role(r), ...(Object.keys(config).length ? [object(config)] : [])];
        }
        let result = call(m.mark, args);
        for (const [key, modifier] of Object.entries(m.modifiers ?? {}).sort(([a], [b]) => a.localeCompare(b))) {
          const args = key === 'cell' ? ['col', 'row', 'width', 'height'].map(k => atom(modifier[k]))
            : key === 'placeholder' && m.mark === 'textInput' ? [staticValue(modifier)] : null;
          if (!args) throw new TypeError('invalid export modifier');
          result = { text: `${result.text}.${key}(${args.map(a => a.text).join(', ')})`,
            value: result.value[key](...args.map(a => a.value)) };
        }
        return result;
      };
      printed = value.surfaces.map(mark);
      return printed.map(m => m.value);
    },
  });
  if (literal(value) !== literal(surfaceValue(definition))) throw new TypeError('surface cannot be printed without loss');
  return `${prefix}defineExport({ roles: ${literal(value.roles)}, params: ${literal(value.params)}, ${value.operations === null ? '' : `operations: ${literal(value.operations)} as import("@circular/protocol").OperationDeclaration, `}surfaces: (roles, params) => [${printed.map(m => m.text).join(', ')}] })`;
}
