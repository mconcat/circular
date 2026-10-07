import { flattenConfigIssue } from "../../protocol/src/flatten-config.js";
import { lowerConfig } from "./form-config.js";
import { validateViewConfig } from "./view-config.js";
import { presentationValue } from "../../protocol/src/declaration-values.js";
import { encodeValueBeta } from "../../protocol/src/value.js";
import {
  allocateReference,
  emitDeclaration,
  endpointOf,
  getExecutionContext,
  metadataOf,
  actorOf,
  registerEndpoint,
  registerActorHandle,
  resolveActor,
} from "./internal.js";

import { COMBINATOR_NAMES } from "./combinators.js";
import { DOWNSTREAM_NAMES, NO_CONFIG, WRITABLE_SPELLINGS } from "./surface.generated.js";

function circularError(code, message) {
  const error = new Error(message);
  error.code = code;
  return error;
}

function layoutCoord(value, name, least = -Infinity) {
  if (!Number.isInteger(value) || value < least) {
    const kind = least === 0 ? "a non-negative integer" : "an integer";
    throw circularError("CIRCULAR_LAYOUT_COORD_UNREPRESENTABLE", `${name} = ${String(value)} is not ${kind} layout unit`);
  }
  return value;
}

function authoredPoint(point) {
  if (point === null || typeof point !== "object") throw new TypeError("Presentation.fixed is { x, y } in layout units");
  return { x: layoutCoord(point.x, "x"), y: layoutCoord(point.y, "y") };
}

function authoredSize(size) {
  if (size === null || typeof size !== "object") throw new TypeError("Presentation.size is { w, h } in layout units");
  return { w: layoutCoord(size.w, "w", 0), h: layoutCoord(size.h, "h", 0) };
}

function constructorConfig(spelling, args) {
  if (["map", "filter", "alert"].includes(spelling) && typeof args[0] === "function") {
    throw circularError("authoring.prepass.config-not-literal", "callback expressions require semanticPrepass on the source bundle; pass a CEL string when calling the runtime directly");
  }
  if ((NO_CONFIG.has(spelling) && spelling !== "pipeline_actor") || spelling === "bang" || spelling === "match") {
    if (args.length !== 0) throw new TypeError(`${spelling}() does not accept configuration`);
    return COMBINATOR_NAMES.includes(spelling) ? {} : null;
  }
  if (spelling === "input") {
    if (args.length !== 1 || args[0] === null || typeof args[0] !== "object" || Array.isArray(args[0])) {
      throw new TypeError("input() requires one configuration object with label");
    }
    return lowerConfig(spelling, args[0]);
  }
  if (spelling === "map") {
    if (args.length !== 1 || typeof args[0] !== "string") throw new TypeError("map() requires one transform expression");
    return Object.freeze({ transform: args[0] });
  }
  if (spelling === "filter") {
    if (args.length !== 1 || typeof args[0] !== "string") throw new TypeError("filter() requires one predicate expression");
    return Object.freeze({ predicate: args[0] });
  }
  if (spelling === "alert") {
    if (args.length !== 2 || typeof args[0] !== "string" || args[1] === null || typeof args[1] !== "object" || Array.isArray(args[1])
      || "predicate" in args[1]) {
      throw new TypeError("alert() requires a predicate expression and a configuration object without predicate");
    }
    return Object.freeze({ predicate: args[0], ...args[1] });
  }
  if (args.length !== 1) throw new TypeError(`${publicName(spelling)}() requires one configuration value`);
  return lowerConfig(spelling, args[0]);
}

function presentationDefaults() {
  return {
    label: null,
    group: null,
    anchor: null,
    fixed: null,
    size: null,
    board: null,
    view: null,
    collapsed: false,
  };
}

function clonePresentationAnchor(anchor) {
  if (anchor === null || anchor === undefined || anchor === "Flow") return anchor ?? null;
  if (anchor.kind === "Relative") {
    return {
      kind: "Relative",
      target: anchor.target,
      relation: {
        RightOf: "After",
        LeftOf: "Before",
        Above: "Before",
        Below: "After",
        Before: "Before",
        After: "After",
      }[anchor.relation],
    };
  }
  if (anchor.kind === "Align") {
    return {
      kind: "Align",
      target: anchor.target,
      axis: {
        Top: "Horizontal",
        Bottom: "Horizontal",
        Left: "Vertical",
        Right: "Vertical",
        Horizontal: "Horizontal",
        Vertical: "Vertical",
      }[anchor.axis],
    };
  }
  return anchor;
}

function assertPresentationFieldNames(value) {
  for (const [field, aliases] of [['size', ['width', 'height']], ['board', ['column', 'width', 'height']]]) {
    if (value[field] != null && aliases.some(key => key in value[field])) {
      throw new TypeError(`Presentation.${field} uses only ${field === 'size' ? 'w, h' : 'col, row, w, h'}`);
    }
  }
}

function clonePresentation(value) {
  assertPresentationFieldNames(value);
  return {
    label: value.label,
    group: value.group,
    anchor: clonePresentationAnchor(value.anchor),
    fixed: value.fixed,
    size: value.size == null ? null : {
      w: value.size.w,
      h: value.size.h,
    },
    board: value.board == null ? null : {
      col: value.board.col,
      row: value.board.row,
      w: value.board.w,
      h: value.board.h,
    },
    view: value.view,
    collapsed: value.collapsed,
  };
}

function normalizePublicPresentation(value) {
  if (!value || typeof value !== "object") throw new TypeError("Presentation must be an object");
  assertPresentationFieldNames(value);
  const normalized = presentationDefaults();
  if (value.label !== undefined) normalized.label = value.label;
  if (value.group !== undefined) normalized.group = value.group;
  if (value.fixed !== undefined) normalized.fixed = value.fixed === null ? null : authoredPoint(value.fixed);
  if (value.size !== undefined) normalized.size = value.size === null ? null : authoredSize(value.size);
  if (value.board !== undefined) {
    normalized.board = value.board === null ? null : {
      col: value.board.col,
      row: value.board.row,
      w: value.board.w,
      h: value.board.h,
    };
  }
  if (value.view !== undefined) {
    if (value.view !== null && (typeof value.view !== "object" || typeof value.view.kind !== "string")) {
      throw new TypeError("a view is { kind, config } — the kind is a string and is not interpreted here");
    }
    if (value.view !== null) validateViewConfig(value.view.config);
    normalized.view = value.view;
  }
  if (value.collapsed !== undefined) normalized.collapsed = Boolean(value.collapsed);
  if (value.anchor !== undefined && value.anchor !== null) {
    const anchor = value.anchor;
    if (anchor.gap !== undefined) {
      throw circularError(
        "CIRCULAR_UNSUPPORTED_LAYOUT_GAP",
        "gap is not part of the canonical layout hints",
      );
    }
    normalized.anchor = anchor.kind === "relative"
      ? {
          kind: "Relative",
          target: actorOf(anchor.target),
          relation: {
            after: "After",
            before: "Before",
          }[anchor.relation],
        }
      : { kind: "Align", target: actorOf(anchor.target), axis: {
          "align-horizontal": "Horizontal",
          "align-vertical": "Vertical",
        }[anchor.relation] };
  }
  return normalized;
}

const COMPARISON_CEILINGS = Object.freeze({
  maximumBytes: 0xffff_ffff, maximumDepth: Number.MAX_SAFE_INTEGER,
  maximumContainerEntries: 0xffff_ffff, maximumStringBytes: 0xffff_ffff,
});

function presentationBytes(value) {
  const anchor = value.anchor && typeof value.anchor === "object"
    ? { ...value.anchor, target: value.anchor.target?.value ?? value.anchor.target } : value.anchor;
  const wire = Object.fromEntries(Object.entries({ ...value, anchor }).filter(([, axis]) => axis !== null && axis !== undefined));
  try { return encodeValueBeta(presentationValue(wire), COMPARISON_CEILINGS); } catch { return null; }
}

function samePresentation(left, right) {
  const a = presentationBytes(left), b = presentationBytes(right);
  return a !== null && b !== null && a.length === b.length && a.every((byte, index) => byte === b[index]);
}

/**
 * SetPresentation carries the owner's whole value, and an absent axis is unset (declaration-values.js).
 * A program means only the axes it says, so the value is those axes over the owner's value in
 * the fold, as a canvas gesture spells it (ui/app/renderer/edit.mjs `present`). The host answers that
 * value from the snapshot the epoch is fenced by; an owner with none starts from the defaults. A value
 * the fold already holds is no change, and no verb is sent for it.
 */
function emitPresentation(metadata) {
  const owner = metadata.owner ?? { actor: metadata.actor };
  const standing = getExecutionContext().presentationOf?.(owner) ?? null;
  const presentation = clonePresentation({ ...(standing ?? presentationDefaults()), ...metadata.presentation });
  if (standing !== null && samePresentation(presentation, standing)) return;
  emitDeclaration({
    kind: "SetPresentation",
    owner,
    presentation: Object.freeze(presentation),
  });
}

function updatePresentation(handle, update, metadata = metadataOf(handle)) {
  metadata.presentation = { ...metadata.presentation, ...update };
  emitPresentation(metadata);
  return handle;
}

function edgeDeclaration(from, to, options = {}, existingOrdinal) {
  if (options.ordinal !== undefined && (!Number.isInteger(options.ordinal) || options.ordinal < 0)) {
    throw new TypeError("edge ordinal must be a non-negative integer");
  }
  const context = getExecutionContext();
  const resolved = context.resolveEdge(from, to, options, existingOrdinal);
  if (!resolved || !Number.isInteger(resolved.ordinal) || resolved.ordinal < 0 || !resolved.attrs) {
    throw circularError("CIRCULAR_EDGE_RESOLUTION_INVALID", "context.resolveEdge must return { ordinal, attrs }");
  }
  return Object.freeze({ from, to, ordinal: resolved.ordinal, attrs: options.preprocess?.length ? { ...resolved.attrs, preprocess: options.preprocess } : resolved.attrs });
}

function refusePreprocessOption(options) {
  if (options !== null && typeof options === "object" && Object.hasOwn(options, "preprocess")) {
    throw circularError("CIRCULAR_EDGE_PREPROCESS_REPLACEMENT",
      "replaceOptions() does not replace preprocess; redeclare the edge, e.g. source.map(…).into(target, { ordinal }), to change its preprocessing");
  }
}

function createNewEdge(from, to, options) {
  let declaration = edgeDeclaration(from, to, options);
  const edge = allocateReference("edge", declaration);
  emitDeclaration({ kind: "UpsertEdge", edge, declaration });
  const handle = {};
  Object.defineProperty(handle, "replaceOptions", {
    enumerable: false,
    value(next) {
      refusePreprocessOption(next);
      declaration = edgeDeclaration(from, to, { ...next, preprocess: declaration.attrs.preprocess }, declaration.ordinal);
      emitDeclaration({ kind: "UpsertEdge", edge, declaration });
      return handle;
    },
  });
  return handle;
}

const pendingEndpoints = new WeakMap();
function connectValues(source, target, options) {
  const pending = pendingEndpoints.get(source);
  return createNewEdge(endpointOf(pending?.upstream ?? source, "source"), endpointOf(target, "target"),
    pending ? { ...options, preprocess: structuredClone(pending.preprocess) } : options);
}
function appendCombinator(upstream, kind, args) {
  requireEndpoint(upstream, "source");
  const config = constructorConfig(kind, args);
  if (!config || typeof config !== "object" || Array.isArray(config) || config instanceof Uint8Array) {
    throw new TypeError(`${kind}() requires a configuration object`);
  }
  if (kind === "flatten") {
    const issue = flattenConfigIssue(config);
    if (issue) throw new TypeError(issue);
  }
  const previous = pendingEndpoints.get(upstream);
  const value = {};
  pendingEndpoints.set(value, { upstream: previous?.upstream ?? upstream,
    preprocess: [...(previous?.preprocess ?? []), { kind, config: structuredClone(config) }] });
  attachSourceSurface(value, false);
  return Object.freeze(value);
}

function attachMountMethod(value, direction) {
  Object.defineProperty(value, "mount", {
    enumerable: false,
    value(nameInput, role) {
      if (typeof nameInput !== "string") throw new TypeError("export mount name must be a string");
      const name = nameInput.normalize("NFC");
      if (!name.length || /[\u0000-\u001f\u007f]/u.test(name)) {
        throw new TypeError("export mount name must be a non-empty canonical text value");
      }
      const writable = direction === "source" && metadataOf(value)?.writable === true;
      if (role === undefined) role = writable ? "request" : "result";
      if (!["request", "progress", "result", "error"].includes(role)) throw new TypeError("Unknown export role");
      if ((role === "request") !== writable) {
        throw circularError("authoring.export.role-direction", "authoring.export.role-direction");
      }
      const endpoint = endpointOf(value, writable ? "writable" : direction);
      let context;
      try { context = getExecutionContext(); }
      catch (error) {
        if (error.code !== "CIRCULAR_NO_EXECUTION_CONTEXT") throw error;
      }
      if (typeof context?.declareExportMount !== "function") {
        throw circularError("CIRCULAR_EXPORT_MOUNT_HOST_MISSING",
          "Export mount is not implemented by this execution host; use the installed SDK authoring host.");
      }
      context.declareExportMount({ name, role, endpoint });
      return value;
    },
  });
}

function attachSourceSurface(value, observable = true) {
  if (observable) attachMountMethod(value, "source");
  Object.defineProperty(value, "into", {
    enumerable: false,
    value(target, options) {
      if (typeof target === "string") throw new TypeError('into(inlet) was removed; pass target.in.<inlet>');
      requireEndpoint(target, "target");
      connectValues(value, target, options);
      return value;
    },
  });
  attachDownstreamMethods(value);
  return value;
}

function requireEndpoint(value, direction) {
  try { return endpointOf(direction === "source" ? pendingEndpoints.get(value)?.upstream ?? value : value, direction); }
  catch (cause) {
    const error = new TypeError(`Unknown ${direction} endpoint`, { cause });
    error.code = "CIRCULAR_PORT_UNKNOWN";
    throw error;
  }
}

function validatePortLists(ports) {
  for (const side of ["inputs", "outputs"]) {
    const list = ports?.[side];
    if (!Array.isArray(list) || list.some(p => !p || typeof p.id !== "string" || !p.id || typeof p.primary !== "boolean")
      || new Set(list.map(p => p.id)).size !== list.length) {
      throw circularError("CIRCULAR_ACTOR_RESOLUTION_INVALID", "resolveActor ports requires inputs/outputs name lists");
    }
  }
}

function portProperties(handle, direction) {
  const list = metadataOf(handle).ports[direction === "source" ? "outputs" : "inputs"];
  const ports = Object.freeze(Object.assign(Object.create(null), Object.fromEntries(list.map(({ id }) => [id, namedEndpoint(handle, direction, id)]))));
  return new Proxy(ports, {
    get: (target, name) => typeof name === "symbol" || Object.hasOwn(target, name) ? target[name] : namedEndpoint(handle, direction, name),
  });
}

function createInputBindingSet(initialBindings) {
  const bindings = [...initialBindings];
  const set = {};
  bindingMetadata.set(set, bindings);
  Object.defineProperty(set, "and", {
    enumerable: false,
    value(...others) {
      const combined = [...bindings];
      for (const other of others) {
        const otherBindings = bindingMetadata.get(other);
        if (!otherBindings) throw new TypeError("and() accepts only Circular input binding sets");
        combined.push(...otherBindings);
      }
      const names = new Set();
      for (const binding of combined) {
        if (names.has(binding.inlet)) throw circularError("CIRCULAR_INPUT_BINDING_DUPLICATE", `Duplicate input binding ${binding.inlet}`);
        names.add(binding.inlet);
      }
      return createInputBindingSet(combined);
    },
  });
  attachDownstreamMethods(set);
  return set;
}

const bindingMetadata = new WeakMap();

function namedEndpoint(handle, direction, name) {
  const metadata = metadataOf(handle);
  const ports = metadata.ports;
  const resolver = direction === "source" ? ports.output : ports.input;
  const table = direction === "source" ? ports.outputs : ports.inputs;
  const port = typeof resolver === "function" ? resolver(name) : table?.find(p => p.id === name)?.id;
  if (port === undefined || port === null) {
    throw circularError("CIRCULAR_PORT_UNKNOWN", `${metadata.spelling} has no ${direction} port ${String(name)}`);
  }
  const endpoint = {};
  registerEndpoint(endpoint, {
    actor: metadata.actor,
    port,
    direction,
    writable: direction === "source" && metadata.writable,
  });
  if (direction === "source") attachSourceSurface(endpoint);
  else attachMountMethod(endpoint, "target");
  return Object.freeze(endpoint);
}

function attachPresentationSurface(handle, metadata = metadataOf(handle)) {
  const update = value => updatePresentation(handle, value, metadata);
  const methods = {
    label(text) {
      if (typeof text !== "string") throw new TypeError("label() requires a string");
      return update({ label: text.normalize("NFC") });
    },
    at(x, y) {
      return update({ fixed: authoredPoint({ x, y }) });
    },
    size(width, height) {
      return update({ size: authoredSize({ w: width, h: height }) });
    },
    board(column, row, width, height) {
      if (!Number.isInteger(column) || column < 0 || !Number.isInteger(row) || row < 0) {
        throw new TypeError("board() column and row must be non-negative integers");
      }
      if (!Number.isInteger(width) || width < 0 || !Number.isInteger(height) || height < 0) {
        throw new TypeError("board() dimensions must be non-negative integers");
      }
      return update({ board: { col: column, row, w: width, h: height } });
    },
    group(name) {
      if (typeof name !== "string" || name.length === 0) throw new TypeError("group() requires a non-empty name");
      return update({ group: name.normalize("NFC") });
    },
    view(kind, config) {
      if (typeof kind !== "string" || kind.length === 0) {
        throw new TypeError("view() requires a non-empty kind");
      }
      validateViewConfig(config);
      return update({ view: { kind, config: config ?? null } });
    },
    collapsed(value) {
      if (typeof value !== "boolean") throw new TypeError("collapsed() requires a boolean");
      return update({ collapsed: value });
    },
    before(target, gap) {
      return relative("Before", target, gap);
    },
    after(target, gap) {
      return relative("After", target, gap);
    },
    alignHorizontal(target) {
      return align("Horizontal", target);
    },
    alignVertical(target) {
      return align("Vertical", target);
    },
    setFlags(flags) {
      if (!flags || [flags.bypass, flags.mute, flags.pause].some((value) => typeof value !== "boolean")) {
        throw new TypeError("setFlags() requires complete boolean bypass, mute, and pause fields");
      }
      const metadata = metadataOf(handle);
      metadata.declaration = { ...metadata.declaration, flags: { ...flags } };
      emitDeclaration({ kind: "SetFlags", actor: metadata.actor, flags: metadata.declaration.flags });
      return handle;
    },
  };
  function relative(relation, target, gap) {
    if (gap !== undefined) {
      throw circularError(
        "CIRCULAR_UNSUPPORTED_LAYOUT_GAP",
        "gap is not part of the canonical layout hints",
      );
    }
    return update({ anchor: { kind: "Relative", target: actorOf(target), relation } });
  }
  function align(axis, target) {
    return update({ anchor: { kind: "Align", target: actorOf(target), axis } });
  }
  for (const [name, method] of Object.entries(methods)) {
    if (name === "setFlags" && metadata.owner?.annotation) continue;
    Object.defineProperty(handle, name, { enumerable: false, value: method });
  }
}

function attachActorSurface(handle, metadata) {
  if (metadata.spelling === "match") {
    Object.defineProperties(handle, {
      ok: { enumerable: true, get: () => handle.out.ok },
      err: { enumerable: true, get: () => handle.out.err },
    });
  }
  Object.defineProperties(handle, {
    actorType: { enumerable: true, value: metadata.spelling },
    mode: { enumerable: true, value: metadata.mode },
    in: { enumerable: false, value: portProperties(handle, "target") },
    out: { enumerable: false, value: portProperties(handle, "source") },
    replaceConfig: {
      enumerable: false,
      value(config) {
        metadata.declaration = { ...metadata.declaration, config: lowerConfig(metadata.spelling, config) };
        emitDeclaration({ kind: "UpsertActor", actor: metadata.actor, declaration: metadata.declaration });
      },
    },
    replacePresentation: {
      enumerable: false,
      value(presentation) {
        metadata.presentation = normalizePublicPresentation(presentation);
        emitPresentation(metadata);
      },
    },
    remove: {
      enumerable: false,
      value() {
        for (const mount of metadata.mounts ?? []) emitDeclaration({ kind: "RetireExportMount", mount });
        emitDeclaration({ kind: "RetireActor", actor: metadata.actor });
      },
    },
    export: {
      enumerable: false,
      value(role) {
        if (!["request", "progress", "result", "error"].includes(role)) {
          throw new TypeError(`Unknown fixed export role: ${String(role)}`);
        }
        const context = getExecutionContext();
        if (typeof context.claimFixedExportRole !== "function") {
          throw circularError(
            "CIRCULAR_UNBOUND_EXPORT_ROLE",
            "export(role) requires a semantic prepass binding; use @circular/exports mount() for an explicit export mount",
          );
        }
        const endpoint = endpointOf(handle, role === "request" ? "writable" : "source");
        const commands = context.claimFixedExportRole({ role, endpoint, actor: metadata.actor });
        const sequence = Array.isArray(commands) ? commands : [commands];
        if (sequence.length === 0 || sequence.some((command) => !command)) {
          throw circularError(
            "CIRCULAR_EXPORT_ROLE_RESOLUTION_INVALID",
            "claimFixedExportRole must return one or more existing declaration commands",
          );
        }
        for (const command of sequence) emitDeclaration(command);
        return handle;
      },
    },
  });
  attachPresentationSurface(handle);
  if (metadata.ports.defaultOutput !== undefined && metadata.ports.defaultOutput !== null) {
    attachSourceSurface(handle);
  }
  return handle;
}

function constructorArguments(spelling, args) {
  const count = (NO_CONFIG.has(spelling) && spelling !== "pipeline_actor") ? 0 : spelling === "alert" ? 2 : 1;
  return { configArgs: args.slice(0, count), wiring: args[count], extra: args.length > count + 1 };
}

function appendActor(upstream, spelling, args) {
  if (COMBINATOR_NAMES.includes(spelling)) return appendCombinator(upstream, spelling, args);
  const { configArgs, wiring, extra } = constructorArguments(spelling, args);
  if (extra || (NO_CONFIG.has(spelling) && spelling !== "pipeline_actor" && wiring !== undefined)
    || (wiring !== undefined && (!wiring || typeof wiring !== "object" || Array.isArray(wiring)
      || Object.keys(wiring).some(key => key !== "at")))) throw new TypeError("chain constructors accept { at } only");
  requireEndpoint(upstream, "source");
  const resolution = resolveActor(spelling, constructorConfig(spelling, configArgs));
  validatePortLists(resolution.ports);
  if (wiring?.at !== undefined && !resolution.ports.inputs.some(p => p.id === wiring.at)) {
    const error = new TypeError(`Unknown inlet ${String(wiring.at)}`); error.code = "CIRCULAR_PORT_UNKNOWN"; throw error;
  }
  const handle = createActor(spelling, configArgs, {}, resolution);
  connectValues(upstream, wiring?.at === undefined ? handle : handle.in[wiring.at], undefined);
  return handle;
}

function detachedActor(spelling, args) {
  if (COMBINATOR_NAMES.includes(spelling)) throw circularError("authoring.prepass.detached-combinator", "a combinator is a wire's preprocessing step and needs an upstream; chain it from a handle, as source.map(...)");
  if (spelling === "match") {
    if (args.length > 1) throw new TypeError("match() accepts one upstream wire");
    if (args.length) requireEndpoint(args[0], "source");
    const handle = createActor(spelling, []);
    if (args.length) connectValues(args[0], handle.in.event);
    return handle;
  }
  const { configArgs, wiring, extra } = constructorArguments(spelling, args);
  if (extra) throw new TypeError("Too many constructor arguments");
  const resolution = resolveActor(spelling, constructorConfig(spelling, configArgs));
  validatePortLists(resolution.ports);
  const bindings = [];
  if (wiring !== undefined) {
    if (!wiring || typeof wiring !== "object" || Array.isArray(wiring)) {
      throw new TypeError("The second constructor argument is an inlet map");
    }
    for (const [inlet, sources] of Object.entries(wiring)) {
      if (!resolution.ports.inputs.some(p => p.id === inlet)) {
        const error = new TypeError(`Unknown inlet ${inlet}`); error.code = "CIRCULAR_PORT_UNKNOWN"; throw error;
      }
      for (const source of Array.isArray(sources) ? sources : [sources]) {
        requireEndpoint(source, "source");
        bindings.push({ source, inlet });
      }
    }
  }
  const handle = createActor(spelling, configArgs, {}, resolution);
  for (const binding of bindings) connectValues(binding.source, handle.in[binding.inlet]);
  return handle;
}

function attachDownstreamMethods(value) {
  for (const spelling of new Set([...DOWNSTREAM_NAMES, "assemble", "join"])) {
    if (Object.prototype.hasOwnProperty.call(value, publicName(spelling))) continue;
    Object.defineProperty(value, publicName(spelling), {
      enumerable: false,
      value(...args) {
        return appendActor(value, spelling, args);
      },
    });
  }
}

export function createActor(spelling, args, identity = {}, preparedResolution) {
  const config = constructorConfig(spelling, args);
  const resolution = preparedResolution ?? resolveActor(spelling, config);
  validatePortLists(resolution.ports);
  const actor = identity.actor ?? allocateReference("actor");
  const declaration = resolution.declaration;
  emitDeclaration({ kind: "UpsertActor", actor, declaration });
  const metadata = {
    actor,
    spelling,
    declaration,
    ports: resolution.ports,
    mode: Object.freeze({ kind: "new" }),
    writable: WRITABLE_SPELLINGS.has(spelling),
    presentation: {},
    scope: identity.scope,
  };
  const handle = registerActorHandle({}, metadata);
  return attachActorSurface(handle, metadata);
}

export function createCurrentActorHandleFromDescriptor(descriptor, spelling) {
  const resolvedSpelling = spelling ?? descriptor.spelling ?? descriptor.actorType;
  if (!resolvedSpelling || !descriptor.actor || !descriptor.declaration || !descriptor.ports) {
    throw new TypeError("Current actor descriptor requires actor, spelling, declaration, and ports");
  }
  validatePortLists(descriptor.ports);
  const metadata = {
    actor: descriptor.actor,
    spelling: resolvedSpelling,
    declaration: descriptor.declaration,
    ports: descriptor.ports,
    mode: Object.freeze({ kind: "current" }),
    writable: descriptor.writable ?? WRITABLE_SPELLINGS.has(resolvedSpelling),
    presentation: {},
    mounts: Object.freeze([...(descriptor.mounts ?? [])]),
  };
  const handle = registerActorHandle({}, metadata);
  attachActorSurface(handle, metadata);
  Object.defineProperties(handle, {
    actorId: { enumerable: true, value: descriptor.actor },
    revision: { enumerable: true, value: descriptor.revision },
  });
  return handle;
}

export function createCurrentEdgeHandleFromDescriptor(descriptor) {
  if (!descriptor.edge || !descriptor.declaration) {
    throw new TypeError("Current edge descriptor requires edge and declaration");
  }
  let declaration = descriptor.declaration;
  const handle = {};
  Object.defineProperties(handle, {
    edgeId: { enumerable: true, value: descriptor.edge },
    disconnect: {
      enumerable: false,
      value() {
        emitDeclaration({ kind: "RetireEdge", edge: descriptor.edge });
      },
    },
    replaceOptions: {
      enumerable: false,
      value(options) {
        refusePreprocessOption(options);
        declaration = edgeDeclaration(declaration["from"], declaration.to, { ...options, preprocess: declaration.attrs.preprocess }, declaration.ordinal);
        emitDeclaration({ kind: "UpsertEdge", edge: descriptor.edge, declaration });
      },
    },
  });
  return handle;
}

const constructorSpellings = new WeakMap();
export function constructorSpelling(value) { return typeof value === "function" ? constructorSpellings.get(value) : undefined; }
export function publicName(spelling) { return spelling.replace(/_([a-z0-9])/g, (_, letter) => letter.toUpperCase()); }
export function detached(spelling) {
  const constructor = (...args) => detachedActor(spelling, args);
  if (!COMBINATOR_NAMES.includes(spelling)) constructorSpellings.set(constructor, spelling);
  return constructor;
}

export function flowRow(actors, options = {}) {
  if (options.gap !== undefined) {
    throw circularError("CIRCULAR_UNSUPPORTED_LAYOUT_GAP", "gap is not part of the canonical layout hints");
  }
  for (let index = 1; index < actors.length; index += 1) actors[index].after(actors[index - 1]);
  return actors;
}

export function flowColumn(actors, options = {}) {
  if (options.gap !== undefined) {
    throw circularError("CIRCULAR_UNSUPPORTED_LAYOUT_GAP", "gap is not part of the canonical layout hints");
  }
  for (let index = 1; index < actors.length; index += 1) actors[index].after(actors[index - 1]);
  return actors;
}

export function flowGrid(rows, options = {}) {
  if (options.gap !== undefined || options.row_gap !== undefined || options.col_gap !== undefined) {
    throw circularError("CIRCULAR_UNSUPPORTED_LAYOUT_GAP", "gap is not part of the canonical layout hints");
  }
  for (const row of rows) flowRow(row);
  for (let row = 1; row < rows.length; row += 1) {
    for (let column = 0; column < Math.min(rows[row].length, rows[row - 1].length); column += 1) {
      rows[row][column].after(rows[row - 1][column]);
    }
  }
  return rows;
}

/** A note is an ordinary authored value with the same presentation methods as an actor. */
export function note(config) {
  if (!config || typeof config !== "object" || Array.isArray(config)
    || Object.keys(config).some(key => !["id", "refs", "text"].includes(key))
    || typeof config.text !== "string" || !Array.isArray(config.refs ?? [])
    || (config.id !== undefined && (typeof config.id !== "string" || !config.id))) {
    throw circularError("authoring.prepass.invalid-note", "note requires { refs, text } and an optional id");
  }
  const annotation = allocateReference("annotation", config.id);
  const refs = (config.refs ?? []).map(ref => actorOf(ref));
  emitDeclaration({ kind: "UpsertAnnotation", annotation,
    declaration: { kind: "Note", refs, body: config.text } });
  const handle = {}, metadata = { owner: { annotation }, presentation: {} };
  attachPresentationSurface(handle, metadata);
  return Object.freeze(handle);
}
