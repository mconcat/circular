import { COMBINATOR_NAMES } from '@circular/core/internal';

export function preprocessRecipe(steps, edges) {
  const combinators = new Map(steps.filter(step => COMBINATOR_NAMES.includes(step.actorType)).map(step => [step.id, step]));
  const expand = (edge, seen = new Set()) => {
    const step = combinators.get(edge.from);
    if (!step) return [edge];
    if (seen.has(step.id)) throw new Error(`Cyclic preprocessing recipe: ${step.id}`);
    const incoming = edges.filter(candidate => candidate.to === step.id);
    if (!incoming.length) throw new Error(`Preprocessing requires upstream: ${step.id}`);
    return incoming.flatMap(upstream => expand({ ...edge, from: upstream.from, fromPort: upstream.fromPort,
      preprocess: [...(upstream.preprocess ?? []), {kind:step.actorType, config:step.config}, ...(edge.preprocess ?? [])] }, new Set([...seen, step.id])));
  };
  return { actors: steps.filter(step => !combinators.has(step.id)),
    edges: edges.filter(edge => !combinators.has(edge.to)).flatMap(edge => expand(edge)) };
}
