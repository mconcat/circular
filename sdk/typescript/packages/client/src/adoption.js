import { openSubscription } from './subscription.js';

const OUTCOMES = new Set(['ActivationOutcome', 'RevisionAdoptionOutcome', 'RecoveryOutcome']);

/** Wait for the first recorded runtime outcome at or after an accepted commit's metadata.cursor. */
export async function waitForAdoption(session, revision) {
  let subscription;
  const closed = (code, reason) => ({ status: 'closed', revision,
    code: typeof code === 'bigint' ? Number(code) : code, reason });
  try {
    subscription = await openSubscription(session, {
      target: 'records', args: { scope: [] }, initialCredit: 1,
    });
    if (subscription.ack !== 1n) {
      const rejection = subscription.ack[1];
      return closed(rejection.code, rejection.message);
    }
    for (;;) {
      const frame = await subscription.receive(Infinity);
      if (frame === null) {
        return closed(subscription.ended?.code ?? 'SUBSCRIPTION_UNANSWERED',
          subscription.ended?.reason ?? 'records subscription ended without an outcome');
      }
      const system = frame.payload?.system;
      if (OUTCOMES.has(system?.kind) && system.revision.value >= revision) {
        return { status: system.code === null ? 'adopted' : 'failed', revision: system.revision.value, code: system.code };
      }
      const answer = await subscription.credit(1);
      if (Array.isArray(answer) && answer[0] === 2n) {
        return closed(answer[1].code, answer[1].message);
      }
    }
  } catch (error) {
    return closed(error.code ?? 'ADOPTION_SUBSCRIPTION_FAILED', error.message);
  } finally {
    subscription?.release();
  }
}
