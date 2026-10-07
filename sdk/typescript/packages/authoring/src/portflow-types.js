const BASE = { null: 'null', bool: 'boolean', int: 'bigint', float: 'number', string: 'string', bytes: 'Uint8Array', uint: 'import("@circular/protocol/actor-query").CircularUInt' };
function shapeType(shape) {
  switch (shape.kind) {
    case 'Base': return BASE[shape.base];
    case 'Array': return `readonly (${shapeType(shape.item)})[]`;
    case 'Object': {
      const fields = shape.fields.map(field => `readonly ${JSON.stringify(field.name)}: ${shapeType(field.shape)};`);
      return `{ ${fields.join(' ')} }${shape.open ? ' & { readonly [key: string]: CircularValue }' : ''}`;
    }
    default: return 'CircularValue';
  }
}

export const AGENT_TURN_VALUE_HINT = 'string | Uint8Array | { readonly op: never }';
export const inputPortHints = (actorType, ports) => actorType === 'agent' ? ports.filter(p => p.id !== 'control') : ports;

/** `flow` is the decoded authoring.actor-ports PortFlowAvailability; admission has none. */
export function portValueType(port, actorType, direction) {
  if (actorType === 'agent' && direction === 'input' && port?.id === 'turn') return AGENT_TURN_VALUE_HINT;
  return port?.flow?.kind === 'Known' ? shapeType(port.flow.flow.item) : 'CircularValue';
}
