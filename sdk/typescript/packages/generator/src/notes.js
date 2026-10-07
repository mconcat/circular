/** Module-local Note values; scope comes from the existing module/epoch boundary. */
import { sameValue } from '@circular/protocol';
import { declarationPayloadValue, declarationCommandFromValue } from '@circular/protocol/declaration';

export function canonicalNote(id, refs, text) {
  if (typeof id !== 'string' || !id || typeof text !== 'string' || !Array.isArray(refs)
    || refs.some(ref => typeof ref !== 'string' || !ref)
    || [id, text, ...refs].some(value => value !== value.normalize('NFC'))) throw new TypeError('unsupported Note text or identity');
  const command = { kind: 'UpsertAnnotation', annotation: { arm: 'epochLocal', value: { scope: [], local: id } },
    declaration: { kind: 'Note', refs: refs.map(local => ({ scope: [], local })), body: text } };
  const { declaration } = declarationCommandFromValue(command.kind, declarationPayloadValue(command));
  return { id, refs: declaration.refs.map(ref => ref.local), text };
}
export function noteFromCommand(command, scope = []) {
  const { annotation, declaration } = command;
  if (declaration.kind !== 'Note') throw new TypeError('The published note() spelling declares Note only; no Backdrop spelling is published.');
  if (!sameValue(annotation.value.scope, scope) || declaration.refs.some(ref => !sameValue(ref.scope, scope))) {
    throw new TypeError('Note refs must name actors in the same module scope; cross-scope ref spelling is unpublished.');
  }
  return canonicalNote(annotation.value.local, declaration.refs.map(ref => ref.local), declaration.body);
}
