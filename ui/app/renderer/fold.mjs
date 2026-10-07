import { canonicalValueSequence } from '@circular/protocol/establishment';
import { declarationPayloadValue } from '@circular/protocol/declaration';
import { OWNER_LOCAL_RESOURCE_CEILINGS } from '@circular/client';
import { identity as key } from './query.mjs';
import { verbOf, LENS } from './verbs.mjs';

const fault = code => Object.assign(new Error(code), { code });

export const emptyScene = (root = []) => ({ root, actors: new Map(),
  edges: new Map(), scopes: new Map([[key(root), { address: root }]]), annotations: new Map(), mounts: new Map(), templates: new Map() });

function row(command) {
  const read = verbOf(command.kind);
  if (!read?.fold) throw fault('DECLARATION_ARM');
  return read;
}
/** Folds one declaration row (a snapshot item or a `delta` row) into the scene and returns the next scene. */
export const foldCommand = (scene, command) => row(command).fold(scene, command);

export const sceneFromSnapshot = ({ anchor, commands }) =>
  commands.reduce((scene, command) => row(command).fold(scene, command, anchor.scope), emptyScene(anchor.scope));
export const foldEpoch = (scene, commit) => commit.delta.reduce(foldCommand, scene);

export function sceneCommands(scene) {
  const groups = [['templates', 'template'], ['scopes', 'scope'], ['actors', 'actor'], ['edges', 'edge'],
    ['mounts', 'mount'], ['annotations', 'annotation'], ['actors', 'presentation', 'actor'],
    ['annotations', 'presentation', 'annotation']];
  return groups.flatMap(([collection, kind, owner]) => {
    const rows = new Map();
    for (const { address, declaration } of scene[collection].values()) {
      if (declaration === undefined) continue;
      const at = owner ? { [owner]: address } : address;
      const value = LENS[kind].view(scene, at);
      if (kind === 'presentation' && !value) continue;
      for (const command of LENS[kind].verbs(at, value, declaration.flags)) {
        rows.set(declarationPayloadValue(command, { includeKind: true }), command);
      }
    }
    return canonicalValueSequence([...rows.keys()], OWNER_LOCAL_RESOURCE_CEILINGS).map(value => rows.get(value));
  });
}
