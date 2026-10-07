const reserved = new Set(('await break case catch class const continue debugger default delete do else '
  + 'enum export extends false finally for function if import in instanceof new null return super switch '
  + 'this throw true try typeof var void while with yield let static implements interface package private '
  + 'protected public eval arguments').split(' '));
export const identifierName = name => /^[$_\p{ID_Start}][$\u200C\u200D\p{ID_Continue}]*$/u.test(name);
export const propertyKey = name => name === '__proto__' ? '["__proto__"]'
  : identifierName(name) ? name : JSON.stringify(name);
export function bindingIdentifier(name) {
  return typeof name === 'string' && name === name.normalize('NFC') && identifierName(name) && !reserved.has(name);
}
const BOUNDARY_SPELLINGS = Object.freeze({ input: 'project_input', output: 'project_output' });
export const boundarySpelling = actorType => BOUNDARY_SPELLINGS[actorType];
export function actorSpelling(actorType, scope) {
  return scope.length && Object.hasOwn(BOUNDARY_SPELLINGS, actorType) ? BOUNDARY_SPELLINGS[actorType] : actorType;
}
export function replicatorPolicy(config) {
  const integer = (value, minimum) => typeof value === 'bigint' && value >= BigInt(minimum) && value <= 0x7fffffffffffffffn;
  return config !== null && typeof config === 'object' && !Array.isArray(config)
    && Object.keys(config).length === 3 && ['at', 'ttl', 'capacity'].every(key => Object.hasOwn(config, key))
    && Array.isArray(config.at) && config.at.every(segment => typeof segment === 'string' || integer(segment, 0))
    && integer(config.ttl, 1) && integer(config.capacity, 1);
}
export const replicatorInlet = config => Array.isArray(config?.in) && config.in.length === 1;
