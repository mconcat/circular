/** Exact projections of the existing daemon queries; no local catalog or config fold. */
import { declarationAddressValue, declarationAddressFromValue } from './declaration-address.js';
import { authoringSnapshotArgumentsValue } from './authoring-query.js';
import { portFlowAvailabilityFromValue } from './port-type.js';

function fail(message) { throw new TypeError(`authoring.query.invalid-shape: ${message}`); }
function fields(value, names) {
  if (!value || typeof value !== 'object' || Array.isArray(value)
    || Object.keys(value).sort().join(',') !== [...names].sort().join(',')) fail(names.join(','));
}
function ports(value) {
  if (!Array.isArray(value)) fail('ports');
  for (const port of value) {
    fields(port, ['id', 'primary']);
    if (typeof port.id !== 'string' || typeof port.primary !== 'boolean') fail('port');
  }
}
function flowPorts(value) {
  if (!Array.isArray(value)) fail('ports');
  const seen = new Set();
  return Object.freeze(value.map(port => {
    fields(port, ['id', 'flow', 'label']);
    if (typeof port.id !== 'string' || !port.id.length || seen.has(port.id)) fail('port');
    if (port.label !== null && (typeof port.label !== 'string' || !port.label.length)) fail('port.label');
    seen.add(port.id);
    return Object.freeze({ id: port.id, flow: portFlowAvailabilityFromValue(port.flow), label: port.label });
  }));
}
export function actorCatalogItemFromValue(value) {
  fields(value, ['actor_type', 'label', 'description', 'presentation_role', 'source', 'view_config', 'config_schema',
    'creatable', 'template_config', 'in_ports', 'out_ports', 'ports_unavailable_reason', 'unavailable_reason']);
  for (const key of ['actor_type', 'label', 'description']) if (typeof value[key] !== 'string') fail(key);
  for (const key of ['source', 'creatable']) if (typeof value[key] !== 'boolean') fail(key);
  for (const key of ['ports_unavailable_reason', 'unavailable_reason']) {
    if (value[key] !== null && typeof value[key] !== 'string') fail(key);
  }
  const role = value.presentation_role;
  if (!Array.isArray(role) || !((role.length === 1 && [1n, 2n].includes(role[0]))
    || (role.length === 2 && ((role[0] === 3n && ['source', 'sink'].includes(role[1]))
      || (role[0] === 4n && ['one', 'keyed_many'].includes(role[1])))))) fail('presentation_role');
  const schema = value.config_schema;
  if (schema !== 1n && !(Array.isArray(schema) && schema.length === 2
    && schema[0] === 2n && typeof schema[1] === 'string')) fail('config_schema');
  ports(value.in_ports); ports(value.out_ports);
  return value;
}
export function actorCreateAdmissionArgumentsValue(actorType, config, authoredActor) {
  return { actor_type: actorType, config,
    authored_actor: declarationAddressValue({ arm: 'absolute', value: authoredActor }, 'actor', 'mutation')[1] };
}
export function actorCreateAdmissionItemFromValue(value) {
  fields(value, ['authored_actor', 'config', 'in_ports', 'actor_type', 'out_ports']);
  if (typeof value.actor_type !== 'string') fail('actor_type');
  if (value.authored_actor !== null) declarationAddressFromValue([1n, value.authored_actor], 'actor', 'mutation');
  ports(value.in_ports); ports(value.out_ports);
  return value;
}
export function authoringActorPortsItemFromValue(value) {
  fields(value, ['actor', 'in_ports', 'out_ports']);
  declarationAddressFromValue(value.actor, 'actor', 'acceptedHistory');
  return Object.freeze({ actor: value.actor, in_ports: flowPorts(value.in_ports), out_ports: flowPorts(value.out_ports) });
}
export { authoringSnapshotArgumentsValue as authoringActorPortsArgumentsValue };
export function actorQueryResultFromValue(value, itemDecoder) {
  if (!Array.isArray(value) || value.length !== 2) fail('QueryResult');
  if (value[0] === 2n) {
    const r = value[1];
    if (!r || typeof r !== 'object' || Array.isArray(r) || typeof r.code !== 'bigint' || typeof r.message !== 'string') fail('Rejected');
    const known = new Set(['code', 'message', 'hint', 'at']);
    if (Object.keys(r).some(key => !known.has(key))) fail('Rejected');
    return { status: 'rejected', reason: 'Invalid',
      diagnostics: [{ code: Number(r.code), message: r.message, hint: r.hint ?? null, at: r.at ?? null }] };
  }
  if (value[0] !== 1n) fail('QueryResult arm');
  const page = value[1];
  fields(page, ['anchor', 'items', 'terminal']);
  if (page.terminal !== 2n || !Array.isArray(page.items)) fail('complete unpaged query');
  return { status: 'accepted', value: { anchor: page.anchor, items: page.items.map(itemDecoder) } };
}
