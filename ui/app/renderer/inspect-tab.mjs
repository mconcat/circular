import { actorIdentityFromValue } from '@circular/protocol/establishment';
import { key } from './scene.mjs';
import { codeText, reason, reasonText } from './reasons.mjs';

export function actorAccess(page, address) {
  return page?.items.find(({ actor }) => actor?.[0] === 1n
    && key(actorIdentityFromValue(actor[1])) === key(address));
}
const capabilityLabel = name => name.replace(/([a-z])([A-Z])/g, '$1 $2').replaceAll('_', ' ').toLowerCase()
  .replace(/^fs\b/i, 'file').replace(/^./, c => c.toUpperCase());
const reading = code => ({ code: codeText(code),
  value: reasonText(reason(code).code === 'UNRECOGNIZED_REASON' ? 'READ_UNAVAILABLE' : code) });

export function permissionsView(node, page, error) {
  if (error) return { ...reading(error), groups: [] };
  const access = actorAccess(page, node?.address);
  if (!access) return null;
  const requirements = access.requirements.map(requirement => ({
    label: capabilityLabel(requirement.capability),
    subject: requirement.subject?.[0] === 1n ? requirement.subject[1] : null,
    ...reading(requirement.decision[0] === 1n ? 'PERMISSION_ALLOWED' : 'PERMISSION_DENIED'),
    detail: requirement.decision[0] === 2n ? requirement.decision[1] : null,
  }));
  const policies = Object.entries(node.declaration.config?.capabilities ?? {}).map(([name, policy]) => ({
    label: capabilityLabel(name),
    ...reading(({ required: 'APPROVAL_POLICY_REQUIRED', none: 'APPROVAL_POLICY_NONE' })[policy.approval]
      ?? 'APPROVAL_POLICY_UNREADABLE'),
  }));
  return { groups: [
    { title: 'Capability access', rows: requirements,
      ...(requirements.length ? {} : { note: reasonText('CAPABILITIES_UNREQUIRED'), noteCode: 'CAPABILITIES_UNREQUIRED' }) },
    ...(policies.length ? [{ title: 'Declared approval policy', rows: policies }] : []),
  ] };
}
export function heldPermissions(node, access) {
  return access ? permissionsView(node, access.page, access.error)
    : { note: 'Reading this actor’s access from the daemon…' };
}

export function portTitle([row, slot]) {
  const code = reason(slot);
  return code.code === slot ? `${row} · ${code.label}` : `${row} · ${slot}`;
}
