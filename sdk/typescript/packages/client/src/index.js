/** Established-session client for Circular's stable owner-local byte wire. */

export { DEFAULT_MAX_IN_FLIGHT, establish } from "./owner-local-session.js";
export { OWNER_LOCAL_SOCKET_NAME, OWNER_LOCAL_RESOURCE_CEILINGS } from './connection-defaults.js';

/** Protocol-owned descriptor for reconstructing current authored state. */
export const authoringSnapshot = Object.freeze({
  name: "authoring-snapshot",
  paging: "cursor",
  anchorKind: "AuthoringSnapshot",
});

/** Protocol-owned credit-controlled continuation stream after an authoring snapshot. */
export const authoringCommits = recordRegistrations['authoring-commits'].subscription;

export { adaptAuthoringSession, actorCatalog, actorCreateAdmission, actorCreateInputs, authoringActorPorts } from "./authoring-adapter.js";

import { recordRegistrations } from '../../protocol/src/internal/record-values.js';
/** actor.events query (non-durable QueryId 21). */
export const actorEvents = recordRegistrations['actor.events'].query;
export const actorEventsSubscription = recordRegistrations['actor.events'].subscription;
/** records query (non-durable QueryId 22). */
export const records = recordRegistrations.records.query;
/** Credit subscription continuing the records query's last applied cursor. */
export const recordsSubscription = recordRegistrations.records.subscription;
/** Credit subscription asking every standing actor its inlet depths now — one frame per
 * row as each actor answers, `Complete` once every asked actor answered or ended. */
export const edgeDepths = recordRegistrations['edge.depths'].subscription;
export { edgeDepthsItemFromValue } from '../../protocol/src/internal/record-values.js';

export { daemonHealthPageFromValue } from './daemon-health.js';
export { timelinePageFromValue, recordsPageFromValue,
  actorEventsPageFromValue, runtimeApprovalsPageFromValue, timeline, runtimeApprovals,
  queryCatalogPageFromValue, queryCatalog,
  deadLettersPageFromValue, deadLetters, instanceTransitionsPageFromValue, instanceTransitions,
  agentHarnessCandidatesPageFromValue, agentHarnessCandidates,
  agentHarnessesPageFromValue, agentHarnesses,
} from '../../protocol/src/internal/observation-values.js';
import { daemonHealthRegistration } from '../../protocol/src/internal/record-values.js';
import { RejectionReason } from '@circular/protocol/tables';
export const daemonHealth = daemonHealthRegistration.query;
export { decideApproval } from './approval.js';
export { setAgentHarness } from './harness.js';
export { waitForAdoption } from './adoption.js';
export { actorEventParents } from './actor-event-parents.js';

/** Daemon rejection codes; Rust owns their numeric assignments (`RejectionReason::numbers`, first column,
 * read from `@circular/protocol/tables`). */
export const reasons = Object.freeze({
  RevisionConflict: RejectionReason.find(reason => reason.name === 'RevisionConflict').numbers[0],
});
