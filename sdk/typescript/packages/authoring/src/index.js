import { exceptionDiagnostic } from './exception-diagnostic.js';
import { installCodeExecutionHost } from "./installed-host.js";
export { defaultEdgePolicies } from "./installed-host.js";
import { isDeepStrictEqual } from "node:util";
/** Client-side authoring orchestration. The engine never imports this module. */

import { createExecutionEpoch } from "@circular/client/internal/execution";
import { runWithExecutionContext } from "@circular/core/internal";
import { runWithCurrentNamespace } from "./current-context.js";
import { standingPresentations } from "./standing-presentations.js";

function makeDiagnostic(phase, message, source = "<authoring>") {
  return Object.freeze({
    phase,
    class: "Rejection",
    code: 0,
    primary: {
      kind: "Source",
      span: {
        source,
        startLine: 1,
        startColumn: 1,
        endLine: 1,
        endColumn: 1,
      },
    },
    related: Object.freeze([]),
    message,
    args: Object.freeze([]),
  });
}

function rejected(phase, message, source) {
  return Object.freeze({
    status: "rejected",
    diagnostics: Object.freeze([makeDiagnostic(phase, message, source)]),
  });
}

export { semanticPrepass } from "./prepass.js";

function cloneLookup(source, name) {
  if (source === null || typeof source?.[Symbol.iterator] !== "function") {
    throw new TypeError(`current snapshot index ${name} must be a ReadonlyMap`);
  }
  return new Map(source);
}

function exactLookup(table, key, kind) {
  const normalized = String(key);
  const handle = table.get(normalized);
  if (handle === undefined) {
    throw new ReferenceError(`No ${kind} named ${JSON.stringify(normalized)} exists at this snapshot anchor`);
  }
  return handle;
}

function sameSnapshotAnchor(left, right) {
  return left?.scope === right?.scope
    && sameRevision(left?.authoringRevision, right?.authoringRevision)
    && sameRevision(left?.topologyRevision, right?.topologyRevision)
    && left?.cursor === right?.cursor
    && left?.environment === right?.environment;
}

/** Creates a resolver that eagerly builds an immutable local index before evaluation. */
export function createCurrentProjectResolver(indexer) {
  if (indexer === null || typeof indexer !== "object" || typeof indexer.index !== "function") {
    throw new TypeError("createCurrentProjectResolver requires a CurrentSnapshotIndexer");
  }

  return Object.freeze({
    resolve(request) {
      if (request?.specifier !== "circular:current") {
        return rejected("Host", "authoring.current.invalid-module-specifier");
      }

      let indexed;
      try {
        indexed = indexer.index(request.snapshot);
      } catch {
        return rejected("Host", "authoring.current.indexer-threw");
      }
      if (indexed.status === "rejected") return indexed;
      if (!sameSnapshotAnchor(indexed.value.anchor, request.snapshot.anchor)) {
        return rejected("Host", "authoring.current.anchor-mismatch");
      }

      let actors;
      let edges;
      let scopes;
      let exports;
      let annotations;
      try {
        actors = cloneLookup(indexed.value.actors, "actors");
        edges = cloneLookup(indexed.value.edges, "edges");
        scopes = cloneLookup(indexed.value.scopes, "scopes");
        exports = cloneLookup(indexed.value.exports, "exports");
        annotations = cloneLookup(indexed.value.annotations, "annotations");
      } catch {
        return rejected("Host", "authoring.current.invalid-complete-index");
      }

      const current = Object.freeze({
        actor: (binding) => exactLookup(actors, binding, "actor"),
        edge: (id) => exactLookup(edges, id, "edge"),
        scope: (id) => exactLookup(scopes, id, "scope"),
        export: (name) => exactLookup(exports, name, "export"),
        annotation: (id) => exactLookup(annotations, id, "annotation"),
      });

      return Object.freeze({
        status: "complete",
        value: Object.freeze({ anchor: request.snapshot.anchor, current }),
        diagnostics: Object.freeze([]),
      });
    },
  });
}

function sameRevision(left, right) {
  return left?.kind === right?.kind
    && (left?.kind !== "At" || isDeepStrictEqual(left.revision, right.revision));
}

function validateCurrentBaseline(snapshot, options) {
  if (snapshot === null) return null;
  if (!isDeepStrictEqual(snapshot.anchor.scope, options.targetScope)) {
    return "authoring.current.target-scope-mismatch";
  }
  if (!sameRevision(snapshot.anchor.authoringRevision, options.expectedRevision)) {
    return "authoring.current.revision-mismatch";
  }
  if (!isDeepStrictEqual(snapshot.anchor.environment, options.expectedEnvironment)) {
    return "authoring.current.environment-mismatch";
  }
  return null;
}

function executionRejected(protocol, diagnostic) {
  return Object.freeze({
    status: "rejected",
    protocol,
    diagnostics: Object.freeze([diagnostic]),
  });
}

function protocolExecutionRejected(protocol, phase) {
  if (protocol.diagnostics?.length) return Object.freeze({ status: "rejected", protocol,
    diagnostics: Object.freeze(protocol.diagnostics.map(detail => Object.freeze({
      ...makeDiagnostic(phase, detail.message), code: detail.code ?? 0,
      ...(detail.primary ? { primary: detail.primary } : {}), protocol: detail,
    }))),
  });
  const reason = String(protocol.reason ?? "Invalid").toLowerCase();
  return executionRejected(
    protocol,
    makeDiagnostic(phase, `authoring.execution.protocol-${reason}`),
  );
}

function makeReferenceAllocator() {
  const counters = new Map();
  return (kind) => {
    const next = (counters.get(kind) ?? 0) + 1;
    counters.set(kind, next);
    return `$circular:${kind}:${next}`;
  };
}

function actorIdentity(actor) {
  return actor && typeof actor === "object" && !Array.isArray(actor) && "arm" in actor && "value" in actor
    ? actor.value : actor;
}

function edgePairKey(from, to) {
  return JSON.stringify([actorIdentity(from.actor), from.port, actorIdentity(to.actor), to.port], (_key, value) => {
    if (value && typeof value === "object" && !Array.isArray(value)) {
      return Object.fromEntries(Object.keys(value).sort().map(key => [key, value[key]]));
    }
    return value;
  });
}

function checkedOrdinal(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new TypeError(`${label} must be a non-negative safe integer`);
  }
  return value;
}

export function createAuthoringEdgeResolverFactory(options) {
  if (typeof options?.policies?.resolve !== "function") {
    throw new TypeError("createAuthoringEdgeResolverFactory requires a pinned policy resolver");
  }

  return Object.freeze({
    create() {
      const reservations = new Map();
      const usedFor = (from, to) => {
        const key = edgePairKey(from, to);
        let used = reservations.get(key);
        if (used === undefined) reservations.set(key, used = new Set());
        return used;
      };

      return Object.freeze({
        resolve(from, to, authoredOptions = {}, existingOrdinal) {
          if (existingOrdinal !== undefined) checkedOrdinal(existingOrdinal, "existing edge ordinal");
          if (authoredOptions.ordinal !== undefined) {
            checkedOrdinal(authoredOptions.ordinal, "edge ordinal");
          }

          const ordinal = authoredOptions.ordinal ?? existingOrdinal ?? 0;
          const used = usedFor(from, to);
          if (used.has(ordinal) && ordinal !== existingOrdinal) {
            const error = new Error(`Edge ordinal ${ordinal} of these endpoints is already declared by another line of this program; a parallel wire takes an explicit, unused { ordinal }`);
            error.code = "CIRCULAR_EDGE_ORDINAL_CONFLICT";
            throw error;
          }
          used.add(ordinal);

          const attrs = options.policies.resolve(authoredOptions);
          if (attrs === null || typeof attrs !== "object") {
            throw new TypeError("edge policy resolver must return complete EdgeAttributes");
          }
          return Object.freeze({ ordinal, attrs });
        },
        release(edge) {
          const key = actorIdentity(edge);
          if (!key?.from || !key?.to || !Number.isSafeInteger(key.ordinal)) return;
          reservations.get(edgePairKey(key.from, key.to))?.delete(key.ordinal);
        },
      });
    },
  });
}

/** Creates the outer host adapter for one-program/one-epoch code execution. */
export function createCodeExecutionHost(dependencies) {
  if (dependencies === null || typeof dependencies !== "object") {
    throw new TypeError("createCodeExecutionHost requires dependencies");
  }
  if (dependencies.session?.declare && dependencies.evaluator === undefined) {
    return installCodeExecutionHost(dependencies, createCodeExecutionHost, createAuthoringEdgeResolverFactory);
  }
  if (!dependencies.session?.declarations) {
    throw new TypeError("CodeExecutionHost requires an established writer session");
  }
  if (typeof dependencies.currentProjectResolver?.resolve !== "function") {
    throw new TypeError("CodeExecutionHost requires a current project resolver");
  }
  if (typeof dependencies.actors?.resolve !== "function") {
    throw new TypeError("CodeExecutionHost requires a pinned actor resolver");
  }
  if (typeof dependencies.edges?.create !== "function") {
    throw new TypeError("CodeExecutionHost requires a per-execution edge resolver factory");
  }
  if (typeof dependencies.evaluator?.evaluate !== "function") {
    throw new TypeError("CodeExecutionHost requires a synchronous program evaluator");
  }

  return Object.freeze({
    async execute(program, options) {
      const baselineFailure = validateCurrentBaseline(options.currentSnapshot, options);
      if (baselineFailure !== null) {
        return executionRejected(null, makeDiagnostic("Host", baselineFailure));
      }

      let edgeResolver;
      try {
        edgeResolver = dependencies.edges.create(options);
      } catch {
        return executionRejected(null, makeDiagnostic("Host", "authoring.edge-resolver-invalid"));
      }
      if (typeof edgeResolver?.resolve !== "function") {
        return executionRejected(null, makeDiagnostic("Host", "authoring.edge-resolver-invalid"));
      }

      const begin = Object.freeze({
        kind: "BeginEpoch",
        scope: options.targetScope,
        commitId: options.commitId,
        expectedRevision: options.expectedRevision,
        expectedEnvironment: options.expectedEnvironment,
      });
      const opened = await createExecutionEpoch(dependencies.session, begin);
      if (opened.status === "rejected") {
        return protocolExecutionRejected(opened, "Admission");
      }

      const epoch = opened.value;
      const pendingCommands = [];
      const presentations = standingPresentations(options.currentSnapshot, options.targetScope);
      const context = Object.freeze({
        declareExportMount: dependencies.evaluator.declareExportMount,
        declareExportSurface: dependencies.evaluator.declareExportSurface,
        openScope: dependencies.evaluator.openScope,
        closeScope: dependencies.evaluator.closeScope,
        allocateReference: dependencies.evaluator.allocateReference ?? makeReferenceAllocator(),
        presentationOf: presentations.of,
        emit: (command) => {
          if (command?.kind === "RetireEdge") edgeResolver.release?.(command.edge);
          presentations.observe(command);
          const lowered = dependencies.evaluator.lowerCommand?.(command) ?? command;
          if (dependencies.evaluator.deferCommands) pendingCommands.push(lowered);
          else epoch.emit(lowered);
        },
        resolveActor: (spelling, config) => dependencies.actors.resolve(spelling, config),
        resolveEdge: (from, to, edgeOptions, existingOrdinal) => (
          edgeResolver.resolve(from, to, edgeOptions, existingOrdinal)
        ),
      });

      try {
        runWithExecutionContext(context, () => {
          let current = null;
          if (options.currentSnapshot !== null) {
            const resolved = dependencies.currentProjectResolver.resolve({
              specifier: "circular:current",
              snapshot: options.currentSnapshot,
            });
            if (resolved.status === "rejected") {
              const error = new Error("Current snapshot resolution was rejected");
              error.circularDiagnostics = resolved.diagnostics;
              throw error;
            }
            current = resolved.value?.current ?? null;
          }

          const evaluate = () => dependencies.evaluator.evaluate(program, { current });
          if (current === null) {
            return evaluate();
          }
          return runWithCurrentNamespace(current, evaluate);
        });
        for (const command of pendingCommands) epoch.emit(command);
      } catch (error) {
        const aborted = await epoch.abort(error);
        const diagnostics = Array.isArray(error?.circularDiagnostics)
          ? error.circularDiagnostics
          : [exceptionDiagnostic(makeDiagnostic("Host", "authoring.execution-program-threw"), error)];
        return Object.freeze({
          status: "rejected",
          protocol: aborted.status === "rejected" ? aborted : null,
          diagnostics: Object.freeze(diagnostics),
        });
      }

      const committed = await epoch.complete();
      if (committed.status === "rejected") {
        return protocolExecutionRejected(committed, "Commit");
      }
      if (committed.status === "unknown") {
        return Object.freeze({
          status: "unknown",
          commitId: options.commitId,
          commands: epoch.commands,
          diagnostics: Object.freeze(committed.diagnostics.map(detail => Object.freeze({
            ...makeDiagnostic("Commit", detail.message), protocol: detail,
          }))),
        });
      }
      return Object.freeze({
        status: "committed",
        commit: committed.value.metadata,
        commands: epoch.commands,
        diagnostics: Object.freeze([]),
      });
    },
  });
}

export { generateCurrentModule } from "./current-module.js";
