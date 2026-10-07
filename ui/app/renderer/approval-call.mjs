import { actorIdentityFromValue } from '@circular/protocol/establishment';
import { identity, readFirstPage } from './query.mjs';
import { declaredEdge } from './activity.mjs';
import { authoredName } from './journal.mjs';
import { bytesText, recordValue } from './record-text.mjs';
import { valueText } from './value-text.mjs';
import { effectText, kindText } from './record-words.mjs';

export const causeUnrecorded = 'APPROVAL_CAUSE_UNRECORDED';
export const causeUnread = 'APPROVAL_CAUSE_UNREAD';

export async function readCause(session, cause) {
  const actor = identity(cause.actor);
  const first = await readFirstPage(session, 'actor.events', null, 1);
  const cut = Array.isArray(first.cut) ? first.cut : [];
  const since = cut.map(component => identity(component.actor) === actor
    ? { actor: component.actor, index: cause.index } : component);
  if (!since.some(component => identity(component.actor) === actor)) since.push({ actor: cause.actor, index: cause.index });
  const page = await readFirstPage(session, 'actor.events', null, 1, since);
  return page.items.find(row => row.kind === 'actor_arrival' && identity(row.actor) === actor
    && row.index === cause.index) ?? null;
}

const text = value => value instanceof Uint8Array ? bytesText(value)
  : typeof value === 'string' ? value : undefined;
const isObject = value => value !== null && typeof value === 'object' && !Array.isArray(value)
  && !(value instanceof Uint8Array);
const program = template => isObject(template) ? {
  effect: typeof template.effect === 'string' ? template.effect : null,
  does: typeof template.effect === 'string' ? effectText(template.effect) : null,
  program: typeof template.program === 'string' ? template.program
    : typeof template.path === 'string' ? template.path : null,
  arguments: Array.isArray(template.arguments) ? template.arguments.map(String) : [],
} : null;
const stepText = step => [step.kind, step.config?.transform ?? step.config?.predicate].filter(Boolean).join(' ');

export function recordedCall(arrival, graph) {
  const wire = arrival.edge === undefined ? undefined
    : graph?.edges?.find(edge => declaredEdge(edge) === identity(arrival.edge));
  const steps = arrival.edge === undefined ? [] : wire ? (wire.attributes?.preprocess ?? []).map(stepText) : null;
  const body = arrival.body;
  const call = steps?.length === 0 && isObject(body) && typeof body.tool === 'string' ? body : null;
  return { wire, steps, call };
}

export function approvalCall(arrival, graph) {
  const node = graph?.nodes?.find(n => n.id === identity(actorIdentityFromValue(arrival.actor)));
  const tools = isObject(node?.declaration?.config?.tools) ? node.declaration.config.tools : {};
  const { wire, steps, call } = recordedCall(arrival, graph);
  const source = wire && graph.nodes.find(n => n.id === wire.from);
  const body = arrival.body;
  const declared = call ? program(tools[call.tool]) : null;
  return {
    index: String(arrival.index),
    port: arrival.port ?? null,
    at: typeof arrival.observed_at_ms === 'bigint' ? Number(arrival.observed_at_ms) / 1000 : null,
    from: source ? authoredName(source.address) : null,
    body: valueText(body).text,
    steps,
    tool: call?.tool ?? null,
    input: call ? text(call.arguments) ?? null : null,
    ...(declared ?? { effect: null, does: null, program: null, arguments: [] }),
    tools: call ? [] : Object.entries(tools).map(([name, template]) => ({ name, ...program(template) })),
  };
}

export function causeRecord(arrival, graph, toDomId) {
  const id = identity(actorIdentityFromValue(arrival.actor));
  const node = graph?.nodes?.find(n => n.id === id);
  return { actor: toDomId(id), actorName: authoredName(node?.address), index: String(arrival.index),
    event: arrival.kind, eventName: kindText(arrival.kind), detail: arrival.port ?? '—', time: String(arrival.observed_at_ms ?? '—'),
    at: Number(arrival.observed_at_ms ?? 0) / 1000, value: recordValue(arrival.body) };
}
