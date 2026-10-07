import {
  actorIdentityFromValue,
  actorIdentityValue,
  scopeIdentityFromValue,
  scopeIdentityValue,
} from "./establishment.js";
import { edgeKeyFromValue, edgeKeyValue } from "./declaration-values.js";

const ARM_TAGS = Object.freeze({ absolute: 1n, epochLocal: 2n, relative: 3n });
const ARM_BY_TAG = new Map(Object.entries(ARM_TAGS).map(([name, tag]) => [tag, name]));
const PLAN_KEYS = new Set(["actor", "exportMount", "annotation"]);

function fail(code, message) {
  const error = new TypeError(`${code}: ${message}`);
  error.code = code;
  throw error;
}

function identityValue(entity, value) {
  if (PLAN_KEYS.has(entity)) return actorIdentityValue(value);
  if (entity === "scope") return scopeIdentityValue(value);
  if (entity === "edge") return edgeKeyValue(value);
  return fail("ADDRESS_ENTITY", `${String(entity)} is not an address entity`);
}

function identityFromValue(entity, value) {
  if (PLAN_KEYS.has(entity)) return actorIdentityFromValue(value);
  if (entity === "scope") return scopeIdentityFromValue(value);
  if (entity === "edge") return edgeKeyFromValue(value);
  return fail("ADDRESS_ENTITY", `${String(entity)} is not an address entity`);
}

function ownerValue(owner, context, codec) {
  if (!owner || typeof owner !== "object" || Array.isArray(owner) || Object.keys(owner).length !== 1
    || !["actor", "annotation"].includes(Object.keys(owner)[0])) {
    fail("PRESENTATION_OWNER_SHAPE", 'owner is exactly one of { actor: address } or { annotation: address }');
  }
  const kind = Object.keys(owner)[0];
  return Object.freeze({ [kind]: codec(owner[kind], kind, context) });
}

export function declarationAddressValue(address, entity, context) {
  if (entity === "presentationOwner") return ownerValue(address, context, declarationAddressValue);
  if (address === null || typeof address !== "object") {
    fail("ADDRESS_SHAPE", "a declaration address is { arm, value }");
  }
  const admitted = context === "mutation"
    ? new Set(["absolute", "epochLocal"])
    : context === "acceptedHistory"
      ? new Set(["absolute"])
      : context === "snapshot"
        ? new Set(["relative"])
        : null;
  if (admitted === null) fail("ADDRESS_CONTEXT", `${String(context)} is not an address context`);
  if (!admitted.has(address.arm)) {
    fail("ADDRESS_ARM_NOT_ADMITTED", `${String(address.arm)} is not admitted in ${context}`);
  }
  const tag = ARM_TAGS[address.arm];
  if (tag === undefined) fail("ADDRESS_ARM", `${String(address.arm)} is not an address arm`);
  return [tag, identityValue(entity, address.value)];
}

export function declarationAddressFromValue(value, entity, context) {
  if (entity === "presentationOwner") return ownerValue(value, context, declarationAddressFromValue);
  if (!Array.isArray(value) || value.length !== 2 || typeof value[0] !== "bigint") {
    fail("ADDRESS_SHAPE", "a declaration address is [arm-tag, identity]");
  }
  const arm = ARM_BY_TAG.get(value[0]);
  if (arm === undefined) fail("ADDRESS_ARM", `address tag ${String(value[0])} is unassigned`);
  const address = Object.freeze({ arm, value: identityFromValue(entity, value[1]) });
  declarationAddressValue(address, entity, context);
  return address;
}
