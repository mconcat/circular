import type { OwnerLocalSession } from './owner-local-session.js';
import type { Session } from './index.js';
import type { Result } from '@circular/protocol';
import type { ActorCatalogItem, ActorCreateAdmissionItem, AuthoringActorPortsItem } from '@circular/protocol/actor-query';
/** An execution-local view of the established session; it owns no socket. */
export declare function adaptAuthoringSession(session: OwnerLocalSession): Pick<Session, 'declarations'>;
export declare function actorCatalog(session: OwnerLocalSession): Promise<Result<{ readonly anchor: unknown; readonly items: readonly ActorCatalogItem[] }>>;
export declare function actorCreateAdmission(session: OwnerLocalSession, actorType: string, config: unknown, authoredActor: unknown): Promise<Result<{ readonly anchor: unknown; readonly items: readonly ActorCreateAdmissionItem[] }>>;
export declare function actorCreateInputs(session: OwnerLocalSession): Promise<Result<{ readonly anchor: readonly string[]; readonly items: readonly import('@circular/protocol').ActorCreateInputCatalogEntry[] }>>;
export declare function authoringActorPorts(session: OwnerLocalSession, scope: unknown): Promise<Result<{ readonly anchor: unknown; readonly items: readonly AuthoringActorPortsItem[] }>>;
