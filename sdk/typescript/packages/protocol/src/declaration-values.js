import { flattenConfigIssue } from "./flatten-config.js";
import { PreprocessKind } from "./internal/closed-tables.js";

import {
  canonicalValueSequence,
  actorIdentityFromValue,
  actorIdentityValue,
} from "./establishment.js";

/**
 * `CurrentAuthoringRevision = Absent | At(AuthoringRevision)`.
 *
 * A closed sum: `Absent` takes no argument and is therefore the tag itself, and `At` carries the
 * revision and is a sequence headed by its tag.
 */
export const CURRENT_AUTHORING_REVISION_ARMS = Object.freeze({ Absent: 1, At: 2 });

export class DeclarationValueError extends Error {
  constructor(code, message) {
    super(`${code}: ${message}`);
    this.code = code;
  }
}

function fail(code, message) {
  throw new DeclarationValueError(code, message);
}

/**
 * The value of a `CurrentAuthoringRevision`.
 *
 * `Absent` is a real answer rather than a missing one — a genesis canvas legitimately has no
 * prior revision, and `BeginEpoch { expected_revision: Absent }` is the ordinary first epoch. So it is an arm of the sum, not an omitted key.
 */
export function currentAuthoringRevisionValue(revision) {
  if (revision === undefined) {
    fail("REVISION_NOT_SUPPLIED", "pass null for the Absent arm; undefined is a missing field");
  }
  if (revision === null) return BigInt(CURRENT_AUTHORING_REVISION_ARMS.Absent);
  if (!(revision instanceof Uint8Array)) {
    fail("REVISION_NOT_BYTES", "an AuthoringRevision is a digest identity carried as Bytes");
  }
  return [BigInt(CURRENT_AUTHORING_REVISION_ARMS.At), revision];
}

/** Reads a `CurrentAuthoringRevision` back. `Absent` returns `null`, which is what selects it. */
export function currentAuthoringRevisionFromValue(value) {
  if (value === BigInt(CURRENT_AUTHORING_REVISION_ARMS.Absent)) return null;
  if (!Array.isArray(value) || value.length !== 2
    || value[0] !== BigInt(CURRENT_AUTHORING_REVISION_ARMS.At)) {
    fail("REVISION_ARM_UNKNOWN", "a CurrentAuthoringRevision is the Absent tag or [At, revision]");
  }
  if (!(value[1] instanceof Uint8Array)) {
    fail("REVISION_NOT_BYTES", "an AuthoringRevision is a digest identity carried as Bytes");
  }
  return value[1];
}

/**
 * `AuthoringEnvironment { declaration_schema, spec_set }`.
 *
 * `versioning.md` section 1.6 fixes the two members and says why they are one value: the
 * conditions for interpreting and executing accepted state have to be atomic within
 * one authoring revision.
 *
 * **Both are Bytes.** They are opaque published identities, and neither is a name — nothing
 * about them is text, so nothing about them
 * belongs to a character grammar or a normalization form. This carried whatever the caller
 * supplied until 2026-08-18, which meant a String encoded cleanly and the daemon refused it
 * with `WrongCarrier { key: "declaration_schema" }`. Passing a value through is not the same as
 * carrying it: a carrier that does not say what kind it takes has not chosen one, it has left
 * the choice to whoever calls it, and then the wire's shape depends on the call site.
 */
export function authoringEnvironmentValue(environment) {
  if (environment === null || typeof environment !== "object") {
    fail("ENVIRONMENT_SHAPE", "an AuthoringEnvironment is an object of two axes");
  }
  const { declarationSchema, specSet } = environment;
  for (const [name, value] of [
    ["declarationSchema", declarationSchema],
    ["specSet", specSet],
  ]) {
    if (value === undefined) fail("ENVIRONMENT_MEMBER_MISSING", `AuthoringEnvironment carries ${name}`);
    if (!(value instanceof Uint8Array)) {
      fail("ENVIRONMENT_MEMBER_KIND", `AuthoringEnvironment's ${name} is an identity carried as Bytes, not text`);
    }
  }
  return {
    declaration_schema: declarationSchema,
    spec_set: specSet,
  };
}

/** Reads an `AuthoringEnvironment` back into its two named axes. */
export function authoringEnvironmentFromValue(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value) || value instanceof Uint8Array) {
    fail("ENVIRONMENT_SHAPE", "an AuthoringEnvironment is an object of two axes");
  }
  for (const name of ["declaration_schema", "spec_set"]) {
    if (!(name in value)) fail("ENVIRONMENT_MEMBER_MISSING", `AuthoringEnvironment carries ${name}`);
    if (!(value[name] instanceof Uint8Array)) {
      fail("ENVIRONMENT_MEMBER_KIND", `AuthoringEnvironment's ${name} decodes from Bytes`);
    }
  }
  return {
    declarationSchema: value.declaration_schema,
    specSet: value.spec_set,
  };
}

export function actorDeclarationValue(declaration) {
  if (declaration === null || typeof declaration !== "object") {
    fail("ACTOR_DECLARATION_SHAPE", "a ActorDecl is an object of domain and flags");
  }
  const { actorType, config, flags } = declaration;
  if (typeof actorType !== "string") fail("ACTOR_TYPE_NOT_STRING", "actor_type is carried as a String");
  if (config === undefined) fail("CONFIG_MISSING", "a ActorDecl carries a config");
  return {
    domain: { config, actor_type: actorType },
    flags: actorFlagsValue(flags),
  };
}

export function actorFlagsValue(flags) {
  if (flags === null || typeof flags !== "object") fail("FLAGS_SHAPE", "ActorFlags is an object");
  const value = {};
  for (const name of ["bypass", "mute", "pause"]) {
    if (typeof flags[name] !== "boolean") fail("FLAG_NOT_BOOL", `${name} is a Bool`);
    value[name] = flags[name];
  }
  for (const name of Object.keys(flags)) {
    if (!["bypass", "mute", "pause"].includes(name)) {
      fail("FLAG_UNKNOWN", `ActorFlags is closed and carries no ${name}`);
    }
  }
  return value;
}

/** Reads a `ActorDecl` back into the flat shape `actorDeclarationValue` was given. */
export function actorDeclarationFromValue(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value) || value instanceof Uint8Array) {
    fail("ACTOR_DECLARATION_SHAPE", "a ActorDecl is an object of domain and flags");
  }
  const { domain, flags } = value;
  if (domain === null || typeof domain !== "object" || Array.isArray(domain)) {
    fail("ACTOR_DECLARATION_SHAPE", "a ActorDecl's domain is an object of config and actor_type");
  }
  if (typeof domain.actor_type !== "string") fail("ACTOR_TYPE_NOT_STRING", "actor_type is carried as a String");
  if (!("config" in domain)) fail("CONFIG_MISSING", "a ActorDecl carries a config");
  return { actorType: domain.actor_type, config: domain.config, flags: actorFlagsValue(flags) };
}

export const DELIVERY_ARMS = Object.freeze({ BestEffort: 1, Lossless: 2, Durable: 3 });

export const DEFAULT_EDGE_ATTRS = Object.freeze({
  delay: Object.freeze({ num: 1n, den: 2n }),
  policy: Object.freeze({ delivery: "Lossless", capacity: 64n }),
});
export const SHED_ARMS = Object.freeze({ DropNewest: 1, DropOldest: 2 });

/**
 * `EdgeDecl { from: (PlanActorKey, PortId), to: (PlanActorKey, PortId), ordinal: u16, attrs }`.
 *
 * An endpoint is always a port pair, and that is what the tuple shape records. A tuple's order is part of its
 * identity, so the pair is a sequence and is not sorted.
 *
 * **`attrs` is required, and so are both of its members.** The delay's value space is
 * `NonNegativeSeconds { num: u64, den: NonZero }`: the rational is two integers and
 * irreducibility is what makes it canonical. An optional `attrs` would be wrong on its own
 * terms, since `UpsertEdge` commits a full normalized `EdgeAttrs`.
 */
export function edgeDeclarationValue(declaration) {
  if (declaration === null || typeof declaration !== "object") {
    fail("EDGE_DECLARATION_SHAPE", "an EdgeDecl is an object");
  }
  const { from, to, ordinal, attrs } = declaration;
  if (!Number.isSafeInteger(ordinal) || ordinal < 0 || ordinal > 0xffff) {
    fail("ORDINAL_WIDTH", "an ordinal is a u16");
  }
  if (attrs === undefined) {
    fail("EDGE_ATTRS_MISSING", "UpsertEdge commits a full normalized EdgeAttrs, so it is not optional");
  }
  return {
    attrs: edgeAttributesValue(attrs),
    from: endpointValue(from, "from"),
    ordinal: BigInt(ordinal),
    to: endpointValue(to, "to"),
  };
}

/**
 * `Presentation` — the owner's complete value.
 *
 * **An absent axis is unset, not unchanged.** Every optional member is carried by the presence
 * of its key, so leaving one out says "this actor has no group" rather than "leave the group
 * alone". Each command replaces the whole value: a partial update would make two
 * commands with the same bytes mean different things depending on what stood before.
 *
 * `collapsed` is the one required member, because a boolean has no absent state that differs
 * from a value — omitting it would be a third state for a two-state thing.
 */
const ANCHOR_ARMS = Object.freeze({ Flow: 1, Relative: 2, Align: 3 });
const RELATION_ARMS = Object.freeze({ Before: 1, After: 2 });
const AXIS_ARMS = Object.freeze({ Horizontal: 1, Vertical: 2 });

function anchorValue(anchor) {
  if (anchor === "Flow") return BigInt(ANCHOR_ARMS.Flow);
  if (anchor === null || typeof anchor !== "object") fail("ANCHOR_SHAPE", "an anchor is Flow or an arm object");
  if (anchor.kind === "Relative") {
    const relation = RELATION_ARMS[anchor.relation];
    if (relation === undefined) fail("RELATION_UNKNOWN", `${anchor.relation} is not a relation`);
    return [BigInt(ANCHOR_ARMS.Relative), actorIdentityValue(anchor.target), BigInt(relation)];
  }
  if (anchor.kind === "Align") {
    const axis = AXIS_ARMS[anchor.axis];
    if (axis === undefined) fail("AXIS_UNKNOWN", `${anchor.axis} is not an axis`);
    return [BigInt(ANCHOR_ARMS.Align), actorIdentityValue(anchor.target), BigInt(axis)];
  }
  return fail("ANCHOR_ARM_UNKNOWN", `${String(anchor.kind)} is not an anchor arm`);
}

function anchorFromValue(value) {
  if (typeof value === "bigint") {
    if (value !== BigInt(ANCHOR_ARMS.Flow)) fail("ANCHOR_ARM_UNKNOWN", `anchor tag ${value} takes an argument`);
    return "Flow";
  }
  if (!Array.isArray(value) || value.length !== 3) fail("ANCHOR_SHAPE", "an anchor arm is a three-part sequence");
  const target = actorIdentityFromValue(value[1]);
  const nameFor = (table, tag) => Object.keys(table).find((name) => BigInt(table[name]) === tag);
  if (value[0] === BigInt(ANCHOR_ARMS.Relative)) {
    const relation = nameFor(RELATION_ARMS, value[2]);
    if (relation === undefined) fail("RELATION_UNKNOWN", `relation tag ${value[2]} is unassigned`);
    return { kind: "Relative", target, relation };
  }
  if (value[0] === BigInt(ANCHOR_ARMS.Align)) {
    const axis = nameFor(AXIS_ARMS, value[2]);
    if (axis === undefined) fail("AXIS_UNKNOWN", `axis tag ${value[2]} is unassigned`);
    return { kind: "Align", target, axis };
  }
  return fail("ANCHOR_ARM_UNKNOWN", `anchor tag ${value[0]} is unassigned`);
}

function signed32(value, key) {
  const number = typeof value === "bigint" ? value : BigInt(value);
  if (number < -(2n ** 31n) || number > 2n ** 31n - 1n) fail("OUT_OF_RANGE", `${key} does not fit an i32`);
  return number;
}

function unsigned32(value, key) {
  const number = typeof value === "bigint" ? value : BigInt(value);
  if (number < 0n || number > 2n ** 32n - 1n) fail("OUT_OF_RANGE", `${key} does not fit a u32`);
  return number;
}

/** Builds the complete presentation value. Optional members are keys, present or absent. */
export function presentationValue(presentation) {
  if (presentation === null || typeof presentation !== "object") {
    fail("PRESENTATION_SHAPE", "a Presentation is an object");
  }
  const known = ["label", "group", "anchor", "fixed", "size", "board", "view", "collapsed"];
  for (const name of Object.keys(presentation)) {
    if (!known.includes(name)) fail("PRESENTATION_UNEXPECTED", `a Presentation carries no \`${name}\``);
  }
  if (typeof presentation.collapsed !== "boolean") {
    fail("PRESENTATION_COLLAPSED", "collapsed is required; a boolean has no absent state");
  }
  const value = { collapsed: presentation.collapsed };
  const present = (name) => presentation[name] !== undefined && presentation[name] !== null;
  if (present("anchor")) value.anchor = anchorValue(presentation.anchor);
  if (present("board")) {
    const board = presentation.board;
    value.board = {
      col: unsigned32(board.col, "col"),
      h: unsigned32(board.h, "h"),
      row: unsigned32(board.row, "row"),
      w: unsigned32(board.w, "w"),
    };
  }
  if (present("fixed")) {
    value.fixed = { x: signed32(presentation.fixed.x, "x"), y: signed32(presentation.fixed.y, "y") };
  }
  if (present("group")) value.group = presentation.group;
  if (present("label")) value.label = presentation.label;
  if (present("size")) {
    value.size = { h: unsigned32(presentation.size.h, "h"), w: unsigned32(presentation.size.w, "w") };
  }
  if (present("view")) {
    const view = presentation.view;
    if (typeof view.kind !== "string" || view.kind.length === 0) {
      fail("VIEW_KIND", "a view kind is a non-empty String");
    }
    value.view = { config: view.config === undefined ? null : view.config, kind: view.kind };
  }
  return value;
}

/** Reads a presentation back into the shape it was built from. */
export function presentationFromValue(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail("PRESENTATION_SHAPE", "a Presentation is an object");
  }
  if (typeof value.collapsed !== "boolean") fail("PRESENTATION_COLLAPSED", "collapsed is required");
  const out = { collapsed: value.collapsed };
  if (value.anchor !== undefined) out.anchor = anchorFromValue(value.anchor);
  if (value.board !== undefined) out.board = { ...value.board };
  if (value.fixed !== undefined) out.fixed = { x: value.fixed.x, y: value.fixed.y };
  if (value.group !== undefined) out.group = value.group;
  if (value.label !== undefined) out.label = value.label;
  if (value.size !== undefined) out.size = { h: value.size.h, w: value.size.w };
  if (value.view !== undefined) out.view = { kind: value.view.kind, config: value.view.config };
  return out;
}

/**
 * The two scope roles, as the closed sum they are.
 *
 * An arm that takes no argument **is** its tag, so these encode as bare
 * integers rather than as a one-element sequence.
 */
const SCOPE_ROLE_TAGS = Object.freeze({ Concrete: 1n, Template: 2n });

/**
 * One boundary binding — `{ inner: [actor, port], outer }`.
 *
 * The inner position reuses the endpoint carrier `UpsertEdge` publishes, unchanged. Spelling
 * "an actor's port" two ways would let the two drift, and the moment they drift one target has two
 * byte strings.
 */
function scopeBindingValue(binding, label) {
  if (binding === null || typeof binding !== "object") fail("BINDING_SHAPE", `${label} is a binding`);
  const { inner, outer } = binding;
  if (typeof outer !== "string") fail("BINDING_OUTER", `${label}.outer is carried as a String`);
  return { inner: endpointValue(inner, `${label}.inner`), outer };
}

function scopeBindingFromValue(value, label) {
  if (value === null || typeof value !== "object") fail("BINDING_SHAPE", `${label} is a binding`);
  if (typeof value.outer !== "string") fail("BINDING_OUTER", `${label}.outer is carried as a String`);
  return { inner: endpointFromValue(value.inner, `${label}.inner`), outer: value.outer };
}

/**
 * One direction of a boundary, ordered by the outer name's raw UTF-8 bytes.
 *
 * **Not by the encoded bytes of the whole binding.** Each direction is a map keyed by the outer
 * port name, so it is ordered by that key; a length prefix compares first in the encoded
 * form, which would order `tick` before `event` while any receiver rebuilding the map hands back
 * `event` before `tick` — a round trip that does not close. Sorting here rather than trusting the
 * caller's order is deliberate: canonical order is a property of the value, and a caller that
 * built the list in another order still means the same boundary.
 */
/** Lexicographic order on two byte strings, so the rule is about the bytes themselves. */
function compareBytes(left, right) {
  for (let index = 0; index < Math.min(left.length, right.length); index += 1) {
    if (left[index] !== right[index]) return left[index] - right[index];
  }
  return left.length - right.length;
}

function boundaryDirectionValue(bindings, label) {
  if (!Array.isArray(bindings)) fail("BOUNDARY_DIRECTION", `${label} is a sequence of bindings`);
  const values = bindings.map((binding, index) => scopeBindingValue(binding, `${label}[${index}]`));
  const seen = new Set();
  for (const value of values) {
    if (seen.has(value.outer)) {
      fail("BOUNDARY_DUPLICATE", `${label} carries the outer name ${value.outer} twice`);
    }
    seen.add(value.outer);
  }
  const encoder = new TextEncoder();
  return values.sort((left, right) => compareBytes(encoder.encode(left.outer), encoder.encode(right.outer)));
}

/**
 * `ScopeDeclaration { role, boundary }` — what `UpsertScope` carries.
 *
 * The scope's contents are not in it. Actors and edges under a prototype are ordinary
 * declarations whose scope address happens to sit beneath it, which is the whole reason stage
 * one of self-modification needs no mutation machinery of its own.
 */
export function scopeDeclarationValue(declaration) {
  if (declaration === null || typeof declaration !== "object") {
    fail("SCOPE_DECLARATION_SHAPE", "a ScopeDeclaration is an object");
  }
  for (const name of Object.keys(declaration)) {
    if (name !== "role" && name !== "boundary") {
      fail("SCOPE_DECLARATION_UNEXPECTED", `a ScopeDeclaration carries no \`${name}\``);
    }
  }
  const tag = SCOPE_ROLE_TAGS[declaration.role];
  if (tag === undefined) fail("SCOPE_ROLE_UNKNOWN", `no scope role is spelled ${String(declaration.role)}`);
  const boundary = declaration.boundary;
  if (boundary === null || typeof boundary !== "object") {
    fail("SCOPE_BOUNDARY_SHAPE", "a ScopeBoundary is an object of two directions");
  }
  return {
    boundary: {
      inlets: boundaryDirectionValue(boundary.inlets, "inlets"),
      outlets: boundaryDirectionValue(boundary.outlets, "outlets"),
    },
    role: tag,
  };
}

/** Reads a scope declaration back into the shape it was built from. */
export function scopeDeclarationFromValue(value) {
  if (value === null || typeof value !== "object") {
    fail("SCOPE_DECLARATION_SHAPE", "a ScopeDeclaration is an object");
  }
  const role = Object.keys(SCOPE_ROLE_TAGS).find((name) => SCOPE_ROLE_TAGS[name] === value.role);
  if (role === undefined) fail("SCOPE_ROLE_UNKNOWN", `scope role tag ${String(value.role)} is unassigned`);
  const boundary = value.boundary;
  if (boundary === null || typeof boundary !== "object") {
    fail("SCOPE_BOUNDARY_SHAPE", "a ScopeBoundary is an object of two directions");
  }
  const direction = (list, label) => {
    if (!Array.isArray(list)) fail("BOUNDARY_DIRECTION", `${label} is a sequence of bindings`);
    const bindings = list.map((binding, index) => scopeBindingFromValue(binding, `${label}[${index}]`));
    const encoder = new TextEncoder();
    let previous = null;
    for (const binding of bindings) {
      const current = encoder.encode(binding.outer);
      if (previous !== null && compareBytes(previous, current) >= 0) {
        fail(
          "BOUNDARY_NOT_CANONICAL",
          `${label} is not in the outer name's UTF-8 order, or repeats one: ${binding.outer}`,
        );
      }
      previous = current;
    }
    return bindings;
  };
  return {
    role,
    boundary: {
      inlets: direction(boundary.inlets, "inlets"),
      outlets: direction(boundary.outlets, "outlets"),
    },
  };
}

const EXPORT_ROLES = Object.freeze(["error", "progress", "request", "result"]);

export function exportDeclarationValue(declaration) {
  if (declaration === null || typeof declaration !== "object") {
    fail("EXPORT_DECLARATION_SHAPE", "an Export is an object of roles, operations and surface");
  }
  for (const name of Object.keys(declaration)) {
    if (name !== "roles" && name !== "operations" && name !== "surface") {
      fail("EXPORT_DECLARATION_UNEXPECTED", `Export is closed to roles, operations and surface; it carries no \`${name}\``);
    }
  }
  const roles = declaration.roles;
  if (roles === null || typeof roles !== "object" || Array.isArray(roles)) {
    fail("EXPORT_ROLES_SHAPE", "roles is a typed partial record");
  }
  const built = {};
  for (const name of Object.keys(roles)) {
    if (!EXPORT_ROLES.includes(name)) fail("EXPORT_ROLE_UNKNOWN", `${name} is not one of the four export roles`);
  }
  for (const name of EXPORT_ROLES) {
    if (roles[name] !== undefined) built[name] = endpointValue(roles[name], `roles.${name}`);
  }
  const value = { roles: built };
  if (declaration.operations !== undefined) value.operations = declaration.operations;
  if (declaration.surface !== undefined) value.surface = declaration.surface;
  return value;
}

/** Reads an export declaration back into the shape it was built from. */
export function exportDeclarationFromValue(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail("EXPORT_DECLARATION_SHAPE", "an Export is an object of roles, operations and surface");
  }
  for (const name of Object.keys(value)) {
    if (!["roles", "operations", "surface"].includes(name)) fail("EXPORT_DECLARATION_UNEXPECTED", `Export carries no \`${name}\``);
  }
  const roles = value.roles;
  if (roles === null || typeof roles !== "object" || Array.isArray(roles)) {
    fail("EXPORT_ROLES_SHAPE", "roles is a typed partial record");
  }
  const read = {};
  for (const name of Object.keys(roles)) {
    if (!EXPORT_ROLES.includes(name)) fail("EXPORT_ROLE_UNKNOWN", `${name} is not one of the four export roles`);
    read[name] = endpointFromValue(roles[name], `roles.${name}`);
  }
  const declaration = { roles: read };
  if (value.operations !== undefined) declaration.operations = value.operations;
  if (value.surface !== undefined) declaration.surface = value.surface;
  return declaration;
}

/**
 * `Annotation { kind, refs, body }` — what `UpsertAnnotation` carries.
 *
 * **There is no `placement` axis, and its absence is deliberate rather than pending.** The value
 * space has not come down from the graph-model spec (`plan`'s `AnnotationPlacement` is
 * `unimplemented!`, which is the same fact said twice). The position is not filled with opaque
 * bytes or a free-form value: a field whose shape is undecided is one for
 * which a vector proving two languages read it alike **cannot be written at all**. So the axis is
 * left out rather than invented, and a body carrying a `placement` key is refused as an
 * unpublished key.
 *
 * `refs` is a set, so its canonical order is the elements' encoded bytes and a
 * repeated ref is a caller's mistake rather than something to collapse.
 */
const ANNOTATION_KIND_TAGS = Object.freeze({ Note: 1, Backdrop: 2 });

/**
 * The ceilings the ref ordering is derived under, when a caller supplies none.
 *
 * **The order does not depend on them.** Ceilings bound what may be encoded; they never change
 * what a given value encodes *to*, so two peers with different limits either agree on the order
 * or one of them refuses outright. The far end hardcodes its provisional ceilings at exactly this
 * position for the same reason. The parameter exists only so a caller's own limit is honoured
 * where one is at hand, not because the answer varies with it.
 */
const ORDERING_CEILINGS = Object.freeze({
  maximumBytes: 1 << 24,
  maximumDepth: 64,
  maximumContainerEntries: 4096,
  maximumStringBytes: 1 << 16,
});

export function annotationDeclarationValue(declaration, resourceCeilings = ORDERING_CEILINGS) {
  if (declaration === null || typeof declaration !== "object") {
    fail("ANNOTATION_DECLARATION_SHAPE", "an Annotation is an object of kind, refs and body");
  }
  for (const name of Object.keys(declaration)) {
    if (name !== "kind" && name !== "refs" && name !== "body") {
      fail("ANNOTATION_DECLARATION_UNEXPECTED", `an Annotation carries no \`${name}\`; the placement axis is unpublished, not omitted here`);
    }
  }
  const tag = ANNOTATION_KIND_TAGS[declaration.kind];
  if (tag === undefined) fail("ANNOTATION_KIND_UNKNOWN", `${String(declaration.kind)} is not an annotation kind`);
  if (typeof declaration.body !== "string") fail("ANNOTATION_BODY", "an annotation body is text");
  if (!Array.isArray(declaration.refs)) fail("ANNOTATION_REFS_SHAPE", "refs is a set of plan actor keys");
  return {
    body: declaration.body,
    kind: BigInt(tag),
    refs: canonicalValueSequence(declaration.refs.map((ref) => actorIdentityValue(ref)), resourceCeilings),
  };
}

/** Reads an annotation declaration back into the shape it was built from. */
export function annotationDeclarationFromValue(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    fail("ANNOTATION_DECLARATION_SHAPE", "an Annotation is an object of kind, refs and body");
  }
  const kind = Object.keys(ANNOTATION_KIND_TAGS).find((name) => BigInt(ANNOTATION_KIND_TAGS[name]) === value.kind);
  if (kind === undefined) fail("ANNOTATION_KIND_UNKNOWN", `annotation kind tag ${value.kind} is unassigned`);
  if (typeof value.body !== "string") fail("ANNOTATION_BODY", "an annotation body is text");
  if (!Array.isArray(value.refs)) fail("ANNOTATION_REFS_SHAPE", "refs is a set of plan actor keys");
  return { kind, refs: value.refs.map((ref) => actorIdentityFromValue(ref)), body: value.body };
}

export function edgeKeyFromDeclaration(declaration) {
  if (declaration === null || typeof declaration !== "object") {
    fail("EDGE_DECLARATION_SHAPE", "an EdgeDecl is an object");
  }
  const { from, to, ordinal } = declaration;
  return { from, to, ordinal };
}

/** The value of an edge key: the same three members, in the same carriers as the declaration. */
export function edgeKeyValue(key) {
  if (key === null || typeof key !== "object") fail("EDGE_KEY_SHAPE", "an EdgeKey is an object");
  const { from, to, ordinal } = key;
  for (const name of Object.keys(key)) {
    if (!["from", "to", "ordinal"].includes(name)) {
      fail("EDGE_KEY_UNEXPECTED", `an EdgeKey carries no \`${name}\`; attrs in particular is not in the key`);
    }
  }
  if (!Number.isSafeInteger(ordinal) || ordinal < 0 || ordinal > 0xffff) {
    fail("ORDINAL_WIDTH", "an ordinal is a u16");
  }
  return [1n, identityEndpointValue(from, "from"), identityEndpointValue(to, "to"), BigInt(ordinal)];
}

/** Reads an edge key back. */
export function edgeKeyFromValue(value) {
  if (!Array.isArray(value)) fail("EDGE_KEY_SHAPE", "an EdgeKey is a tagged sequence");
  if (value[0] === 2n) fail("EDGE_KEY_ARM", "an authored edge address never names the Outcome arm");
  if (value[0] !== 1n || value.length !== 4) {
    fail("EDGE_KEY_SHAPE", "an EdgeKey is a tagged sequence");
  }
  return {
    from: identityEndpointFromValue(value[1], "from"),
    to: identityEndpointFromValue(value[2], "to"),
    ordinal: readOrdinal(value[3]),
  };
}

function readOrdinal(raw) {
  if (typeof raw !== "bigint" || raw < 0n || raw > 0xffffn) fail("ORDINAL_WIDTH", "an ordinal is a u16");
  return Number(raw);
}

/**
 * One endpoint: the `(PlanActorKey, PortId)` pair, in that order, never sorted.
 *
 * **The actor is the plan key, with no arm tag in front of it.** The pair is defined over
 * `PlanActorKey` rather than over an address, so an endpoint names an actor directly.
 * That costs nothing: a plan actor key is author-chosen (`{local, scope}`), so an actor declared
 * earlier in the same epoch already has its key and never needs an epoch-local forward
 * reference. The SDK's `EdgeDeclaration<D>` parameterizes endpoints by the address domain and
 * so says otherwise; the surface is what is behind here, not the wire.
 */
function endpointValue(endpoint, label) {
  if (endpoint === null || typeof endpoint !== "object") fail("ENDPOINT_SHAPE", `${label} is a port pair`);
  const { actor, port } = endpoint;
  if (typeof port !== "string") fail("ENDPOINT_PORT", `${label}.port is carried as a String`);
  return [actorIdentityValue(actor), port];
}

function endpointFromValue(value, label) {
  if (!Array.isArray(value) || value.length !== 2) fail("ENDPOINT_SHAPE", `${label} is a port pair`);
  if (typeof value[1] !== "string") fail("ENDPOINT_PORT", `${label}.port is carried as a String`);
  return { actor: actorIdentityFromValue(value[0]), port: value[1] };
}

function identityEndpointValue(endpoint, label) {
  if (endpoint === null || typeof endpoint !== "object") fail("ENDPOINT_SHAPE", `${label} is a port pair`);
  const { actor, port } = endpoint;
  if (typeof port !== "string") fail("ENDPOINT_PORT", `${label}.port is carried as a String`);
  return { actor: actorIdentityValue(actor), port };
}

function identityEndpointFromValue(value, label) {
  if (value === null || typeof value !== "object" || Array.isArray(value) || value instanceof Uint8Array) {
    fail("ENDPOINT_SHAPE", `${label} is a port pair`);
  }
  if (typeof value.port !== "string") fail("ENDPOINT_PORT", `${label}.port is carried as a String`);
  return { actor: actorIdentityFromValue(value.actor), port: value.port };
}

export const PREPROCESS_KINDS = PreprocessKind;

function preprocessSteps(value) {
  if (!Array.isArray(value)) fail("EDGE_PREPROCESS_SHAPE", "preprocess must be an array");
  return value.map(step => {
    if (!step || typeof step !== "object" || Array.isArray(step)
      || Object.keys(step).length !== 2 || !Object.hasOwn(step, "kind") || !Object.hasOwn(step, "config")) {
      fail("EDGE_PREPROCESS_STEP", "preprocess step requires kind and config");
    }
    if (!PREPROCESS_KINDS.includes(step.kind)) {
      fail("EDGE_PREPROCESS_STEP", `unknown preprocess kind ${String(step.kind)}`);
    }
    if (!step.config || typeof step.config !== "object" || Array.isArray(step.config) || step.config instanceof Uint8Array) {
      fail("EDGE_PREPROCESS_CONFIG", "preprocess config must be an object");
    }
    if (step.kind === "flatten") {
      const issue = flattenConfigIssue(step.config);
      if (issue) fail("EDGE_PREPROCESS_CONFIG", issue);
    }
    return { config: step.config, kind: step.kind };
  });
}
function edgeAttributeKeys(attrs) {
  if (!attrs || typeof attrs !== "object" || Array.isArray(attrs) || attrs instanceof Uint8Array) fail("EDGE_ATTRS_SHAPE", "EdgeAttrs is an object");
  for (const name of Object.keys(attrs)) if (!["delay", "policy", "preprocess"].includes(name)) {
    fail("EDGE_ATTRS_UNEXPECTED", `EdgeAttrs carries no ${name}`);
  }
}
function edgeAttributesValue(attrs) {
  edgeAttributeKeys(attrs);
  const preprocess = Object.hasOwn(attrs, "preprocess") ? preprocessSteps(attrs.preprocess) : [];
  return { delay: delayValue(attrs.delay), policy: policyValue(attrs.policy), ...(preprocess.length ? { preprocess } : {}) };
}
function edgeAttributesFromValue(value) {
  edgeAttributeKeys(value);
  const preprocess = Object.hasOwn(value, "preprocess") ? preprocessSteps(value.preprocess) : [];
  return { delay: delayFromValue(value.delay), policy: policyFromValue(value.policy), preprocess };
}

/**
 * `NonNegativeSeconds { num: u64, den: NonZero }`, irreducible.
 *
 * **Irreducibility is refused, not normalized.** The reason is a byte reason rather than an
 * equality one: `UpsertEdge` commits a *full normalized*
 * `EdgeAttrs`, so if `1/2` and `2/4` encoded differently that word would be false. Reducing
 * silently would also mean the value a caller sent and the value the wire carries differ, which
 * is the thing normalization is supposed to prevent.
 *
 * `num = 0` is allowed — a zero-delay edge is the ordinary case — and then only `den = 1` is
 * irreducible, since every other denominator names the same duration.
 */
/**
 * A reduced non-negative rational: `{ num: u64, den: NonZero }`.
 *
 * **Exported because the wire has one such carrier, not one per position.** The edge delay and
 * the replay multiplier are the same `NonNegativeSeconds`-shaped pair with the same canonical
 * form, and the far end reads both through one `decode_ratio`. Two copies here would let the two
 * positions drift while both files' comments went on claiming they were the same carrier — the
 * kind of agreement that holds right up until one side is edited.
 *
 * `key` names the position only so a rejection says which one reduced.
 */
export function reducedRatioValue(ratio, key) {
  if (ratio === null || typeof ratio !== "object") {
    fail("RATIO_SHAPE", `${key} is a ratio { num, den }`);
  }
  const num = readUnsigned(ratio.num, "num");
  const den = readUnsigned(ratio.den, "den");
  if (den === 0n) fail("RATIO_DENOMINATOR_ZERO", `den is NonZero; ${key} over zero names no quantity`);
  const divisor = num === 0n ? den : greatestCommonDivisor(num, den);
  if (divisor !== 1n) {
    fail("RATIO_NOT_IRREDUCIBLE", `${num}/${den} reduces, and a reducible pair would give one ${key} two byte strings`);
  }
  return { den, num };
}

function delayValue(delay) {
  if (delay === null || typeof delay !== "object") {
    fail("DELAY_SHAPE", "a declared delay is NonNegativeSeconds { num, den }");
  }
  return reducedRatioValue(delay, "delay");
}

function delayFromValue(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value) || value instanceof Uint8Array) {
    fail("DELAY_SHAPE", "a declared delay is NonNegativeSeconds { num, den }");
  }
  return delayValue({ num: value.num, den: value.den });
}

function greatestCommonDivisor(left, right) {
  let a = left;
  let b = right;
  while (b !== 0n) {
    const next = a % b;
    a = b;
    b = next;
  }
  return a;
}

function readUnsigned(raw, label) {
  if (typeof raw !== "bigint" || raw < 0n) fail("DELAY_MEMBER_KIND", `${label} is a non-negative Int`);
  return raw;
}

function policyValue(policy) {
  if (policy === null || typeof policy !== "object") fail("POLICY_SHAPE", "a WirePolicy is an object");
  for (const name of Object.keys(policy)) {
    if (name !== "delivery" && name !== "capacity") {
      fail("POLICY_UNEXPECTED", `a WirePolicy carries delivery and capacity, not \`${name}\``);
    }
  }
  const value = {};
  if (policy.capacity !== undefined && policy.capacity !== null) {
    const capacity = readUnsigned(policy.capacity, "capacity");
    if (capacity === 0n) {
      fail("CAPACITY_NOT_POSITIVE", "PositiveCapacity has no zero constructor; an absent capacity is an absent key");
    }
    value.capacity = capacity;
  }
  value.delivery = deliveryValue(policy.delivery);
  return value;
}

function policyFromValue(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value) || value instanceof Uint8Array) {
    fail("POLICY_SHAPE", "a WirePolicy is an object");
  }
  const policy = { delivery: deliveryFromValue(value.delivery) };
  if ("capacity" in value) policy.capacity = readUnsigned(value.capacity, "capacity");
  return policy;
}

/** One delivery mode: the tag alone, or `[BestEffort, shed]`. */
function deliveryValue(delivery) {
  if (typeof delivery === "string") {
    const tag = DELIVERY_ARMS[delivery];
    if (tag === undefined) fail("DELIVERY_ARM_UNKNOWN", `${delivery} is not a delivery mode`);
    if (delivery === "BestEffort") {
      fail("DELIVERY_SHED_MISSING", "BestEffort carries a Shed; the other two have no Shed field at all");
    }
    return BigInt(tag);
  }
  if (delivery === null || typeof delivery !== "object") {
    fail("DELIVERY_SHAPE", "a delivery mode is a name, or BestEffort with its Shed");
  }
  if (delivery.mode !== "BestEffort") {
    fail("DELIVERY_SHED_UNEXPECTED", "only BestEffort carries a Shed");
  }
  const shed = SHED_ARMS[delivery.onFull];
  if (shed === undefined) fail("SHED_ARM_UNKNOWN", `${delivery.onFull} is not a Shed`);
  return [BigInt(DELIVERY_ARMS.BestEffort), BigInt(shed)];
}

function deliveryFromValue(value) {
  const nameFor = (table, tag) => Object.keys(table).find((name) => BigInt(table[name]) === tag);
  if (typeof value === "bigint") {
    const name = nameFor(DELIVERY_ARMS, value);
    if (name === undefined || name === "BestEffort") {
      fail("DELIVERY_ARM_UNKNOWN", `delivery tag ${value} is not an argument-free mode`);
    }
    return name;
  }
  if (!Array.isArray(value) || value.length !== 2) fail("DELIVERY_SHAPE", "a delivery mode is a tag or [tag, shed]");
  if (value[0] !== BigInt(DELIVERY_ARMS.BestEffort)) {
    fail("DELIVERY_SHED_UNEXPECTED", `delivery tag ${value[0]} carries no Shed`);
  }
  const onFull = nameFor(SHED_ARMS, value[1]);
  if (onFull === undefined) fail("SHED_ARM_UNKNOWN", `shed tag ${value[1]} is unassigned`);
  return { mode: "BestEffort", onFull };
}

/** Reads an `EdgeDecl` back into the shape `edgeDeclarationValue` was given. */
export function edgeDeclarationFromValue(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value) || value instanceof Uint8Array) {
    fail("EDGE_DECLARATION_SHAPE", "an EdgeDecl is an object");
  }
  return {
    from: endpointFromValue(value.from, "from"),
    to: endpointFromValue(value.to, "to"),
    ordinal: readOrdinal(value.ordinal),
    attrs: edgeAttributesFromValue(value.attrs),
  };
}

/** Re-exported so a caller building a role or scope set uses one ordering, not two. */
export { canonicalValueSequence };

export const COMMAND_RESULT_ARMS = Object.freeze({ Accepted: 1, Rejected: 2 });

/** The fields a rejection carries. `reason` is deliberately not among them. */
const REJECTION_FIELDS = Object.freeze(["code", "message", "hint", "at"]);

/**
 * The value of an accepted result.
 *
 * `fact` is `undefined` for the commands that produce none, and then the value is the bare tag.
 */
export function acceptedResultValue(fact) {
  if (fact === undefined) return BigInt(COMMAND_RESULT_ARMS.Accepted);
  return [BigInt(COMMAND_RESULT_ARMS.Accepted), fact];
}

/**
 * The value of a rejected result.
 *
 * `reason` is refused rather than dropped: a caller that supplied one believes the wire carries
 * it, and silently discarding it would leave that belief intact.
 */
export function rejectedResultValue(rejection) {
  if (rejection === null || typeof rejection !== "object") {
    fail("REJECTION_SHAPE", "a rejection is an object with the published rejection fields");
  }
  if ("reason" in rejection) {
    fail(
      "REJECTION_CARRIES_REASON",
      "the reason is computed from the code, so the wire carries the other four; a reason on the wire gives a derived value an independent position",
    );
  }
  if (rejection.code === undefined) fail("REJECTION_CODE_MISSING", "a rejection names its code");

  const value = {};
  for (const field of REJECTION_FIELDS) {
    if (rejection[field] !== undefined) value[field] = rejection[field];
  }
  return [BigInt(COMMAND_RESULT_ARMS.Rejected), value];
}
