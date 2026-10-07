
/**
 * One `actor.catalog` row in decoded wire form (bigint tags), as `actorCatalogItemFromValue` reads it.
 * Unvaried fields describe a creatable Flow actor with static ports and no view config.
 */
export const catalogRow = (actor_type, fields = {}) => ({
  actor_type, label: actor_type, description: '', presentation_role: [2n], source: false, view_config: null,
  config_schema: 1n, creatable: true, template_config: null, in_ports: [], out_ports: [],
  ports_unavailable_reason: null, unavailable_reason: null, ...fields,
});

/**
 * One `actor.create-inputs` schema slot. `path`, `shape` and `requirement` are always given, in the
 * form the caller uses (capture JSON `'1n'` or decoded `1n`); `policies` only when the slot has them.
 * Unvaried fields carry no constraint, no snippet and no authored label, description or group.
 */
export const createInputSlot = fields => ({
  constraint: null, snippet: null, label: null, description: null, group: null, ...fields,
});

const joinSchemaStop = 'registry ConfigSchema is an incomplete frame: no complete payload admission/default projection is published';
export const joinRow = catalogRow('join', {
  label: 'Join', description: 'Join each event with the latest reference state for its key.',
  presentation_role: [1n], creatable: false, config_schema: [2n, joinSchemaStop], unavailable_reason: joinSchemaStop,
  in_ports: [{ id: 'event', primary: true }, { id: 'state', primary: false }, { id: 'remove', primary: false }],
  out_ports: [{ id: 'event', primary: true }],
});
