import { deepFreeze, diagnostic } from "./_immutable.js";

function rejected(code, message) {
  return deepFreeze({ status: "rejected", diagnostics: [diagnostic(code, message)] });
}

/** Creates immutable catalog-type to SDK-constructor lookup. */
export function createProviderBindingRegistry({ bindings = [] } = {}) {
  const canonicalTable = new Map();
  for (const { actorTypeId, importSpecifier, constructorExport } of bindings) {
    const binding = deepFreeze({ actorTypeId, importSpecifier, constructorExport });
    const candidates = canonicalTable.get(actorTypeId) ?? [];
    candidates.push(binding);
    canonicalTable.set(actorTypeId, candidates);
  }
  return Object.freeze({
    resolve(request) {
      const candidates = canonicalTable.get(request.expectedActorType) ?? [];
      const matches = candidates.filter((binding) =>
        binding.importSpecifier === request.importSpecifier &&
        binding.constructorExport === request.constructorExport,
      );
      if (matches.length !== 1) {
        return rejected(
          matches.length === 0 ? "PROVIDER_BINDING_UNKNOWN" : "PROVIDER_BINDING_AMBIGUOUS",
          `Constructor provenance did not resolve uniquely for ${request.constructorExport}`,
        );
      }
      return deepFreeze({ status: "resolved", binding: matches[0] });
    },
    canonical(actorType) {
      const candidates = canonicalTable.get(actorType) ?? [];
      if (candidates.length !== 1) {
        return rejected(
          candidates.length === 0 ? "PROVIDER_CANONICAL_MISSING" : "PROVIDER_CANONICAL_AMBIGUOUS",
          `Canonical constructor did not resolve uniquely for ${actorType}`,
        );
      }
      return deepFreeze({ status: "resolved", binding: candidates[0] });
    },
  });
}
