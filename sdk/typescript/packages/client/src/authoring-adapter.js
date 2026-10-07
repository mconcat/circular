import { actorCatalogItemFromValue, actorCreateAdmissionItemFromValue, actorCreateAdmissionArgumentsValue,
  authoringActorPortsItemFromValue, authoringActorPortsArgumentsValue, actorQueryResultFromValue,
} from '@circular/protocol/actor-query';
import { actorCreateInputCatalogEntryFromValue, decodeActorCreateInputs } from '@circular/protocol';
import { recordedCommit } from './internal/commit-outcome.js';

async function query(session, name, args, decode) {
  const answer = await session.exchange('Query', 'Query', { name, args });
  if (answer.kind.verb !== 'QueryResult') throw new TypeError('authoring.query.unexpected-result');
  return actorQueryResultFromValue(answer.payload, decode);
}
const catalogs = new WeakMap();
/** One catalog read per established session, including across host executions. */
export function actorCatalog(session) {
  if (!catalogs.has(session)) catalogs.set(session, query(session, 'actor.catalog', null, actorCatalogItemFromValue));
  return catalogs.get(session);
}
export function actorCreateAdmission(session, actorType, config, authoredActor) {
  return query(session, 'actor.create-admission', actorCreateAdmissionArgumentsValue(actorType, config, authoredActor), actorCreateAdmissionItemFromValue);
}
export async function actorCreateInputs(session) {
  const result = await query(session, 'actor.create-inputs', null, actorCreateInputCatalogEntryFromValue);
  if (result.status === 'accepted') decodeActorCreateInputs(result.value.anchor, result.value.items);
  return result;
}
export function authoringActorPorts(session, scope) {
  return query(session, 'authoring.actor-ports', authoringActorPortsArgumentsValue(scope), authoringActorPortsItemFromValue);
}
export function adaptAuthoringSession(session) {
  let issued = 0;
  const epochs = new Map();
  const send = (command) => {
    const correlation = `authoring-${++issued}`;
    const completion = session.declare(command);
    return { correlation, completion };
  };
  const terminal = (command) => {
    const epoch = epochs.get(command.epoch);
    if (epoch === undefined) throw new TypeError('authoring.session.unknown-epoch');
    const request = send({ ...command, epoch });
    if (command.kind !== 'ValidateEpoch') request.completion = request.completion.finally(() => epochs.delete(command.epoch));
    return request;
  };
  return Object.freeze({
    declarations: Object.freeze({
      begin(command) {
        const request = send({ ...command, scope: { arm: 'absolute', value: command.scope } });
        request.completion = request.completion.then(result => {
          if (result.status !== 'accepted') return result;
          const token = request.correlation;
          epochs.set(token, result.value.epoch);
          return { status: 'accepted', value: { epoch: token } };
        });
        return request;
      },
      apply({ epoch, command }) {
        if (!epochs.has(epoch)) throw new TypeError('authoring.session.unknown-epoch');
        return send(command);
      },
      validate: terminal, commit: terminal, abort: terminal,
      /** The durable commit record for one CommitId — read when a CommitEpoch answer is lost. */
      recorded(commitId) { return recordedCommit(session, commitId); },
    }),
  });
}
