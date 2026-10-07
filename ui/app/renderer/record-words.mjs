
export const recordKinds = Object.freeze({
  actor_arrival: 'Arrival',
  http_response: 'HTTP response', file_bytes: 'File read', written_length: 'File written',
  process_result: 'Program finished', notification_delivered: 'Notification delivered',
  agent_step_result: 'Agent step', approval: 'Approval decided', schedule_armed: 'Timer set',
  peer_snapshot: 'Peer snapshot', peer_binding: 'Peer bound', submission_receipt: 'Message submitted',
  unbind_receipt: 'Peer unbound', peer_envelope: 'Peer message',
  parameter_denied: 'Parameter denied', transport_terminal: 'Connection ended', approval_required: 'Approval required',
  diverged: 'Replay diverged', endpoint_gone: 'Endpoint gone', interpreter_fault: 'Interpreter failed',
  retry_exhausted: 'Retries ran out', transport_unreached: 'Not reached', remote_deferred: 'Deferred by the other side',
  peer_adapter_unavailable: 'Peer adapter unavailable', peer_wrong_adapter: 'Wrong peer adapter',
  peer_wrong_realm: 'Wrong peer realm', peer_not_found: 'Peer not found', peer_binding_not_found: 'Peer binding not found',
  peer_binding_stale: 'Peer binding out of date', peer_address_stale: 'Peer address out of date',
  peer_inbound_refused: 'Peer refused the message', peer_duplicate_message: 'Duplicate peer message',
  peer_inbox_full: 'Peer inbox full', peer_binding_has_pending_events: 'Peer binding still has messages waiting',
  peer_unsupported_capability: 'Peer does not support this', peer_submission_unknown: 'Peer submission unknown',
  peer_name_conflict: 'Peer name already taken',
});

export const declarationVerbs = Object.freeze({
  BeginEpoch: 'Edit opened', ValidateEpoch: 'Edit checked', CommitEpoch: 'Edit committed', AbortEpoch: 'Edit discarded',
  UpsertActor: 'Actor set', RetireActor: 'Actor removed', UpsertEdge: 'Wire set', RetireEdge: 'Wire removed',
  UpsertScope: 'Scope set', RetireScope: 'Scope removed', MoveToScope: 'Moved to another scope',
  UpsertExportMount: 'Output set', RetireExportMount: 'Output removed', UpsertAnnotation: 'Note set', RetireAnnotation: 'Note removed',
  SetPresentation: 'Place or view changed', SetFlags: 'Flags changed', UpsertTemplate: 'Template set', RetireTemplate: 'Template removed',
});
export const verbText = verb => own(declarationVerbs, verb) ?? String(verb ?? '');

export const toolEffects = Object.freeze({
  spawn: 'Runs a program', file_read: 'Reads a file', file_write: 'Writes a file',
});

const own = (table, key) => typeof key === 'string' && Object.hasOwn(table, key) ? table[key] : undefined;
export const kindText = kind => own(recordKinds, kind) ?? String(kind ?? '');
export const isRecordKind = kind => own(recordKinds, kind) !== undefined;
export const effectText = effect => own(toolEffects, effect) ?? (effect == null ? null : String(effect));

const integer = value => typeof value?.value === 'bigint' ? value.value
  : typeof value === 'bigint' || Number.isInteger(value) ? BigInt(value) : null;
export const outcomeState = ({ ok, exit }) => !ok ? 'Failed'
  : exit !== null && exit !== undefined && integer(exit) !== null && integer(exit) !== 0n ? `Exit code ${integer(exit)}` : 'OK';
export function outcomeFacts(body) {
  const exit = integer(body?.exit), attempts = integer(body?.attempts);
  return [outcomeState({ ok: body?.ok === true, exit }),
    typeof body?.channel === 'string' ? `on ${body.channel}` : null,
    attempts === null ? null : `after ${attempts} ${attempts === 1n ? 'attempt' : 'attempts'}`].filter(Boolean);
}
export const causedBy = index => `caused by arrival #${index}`;
