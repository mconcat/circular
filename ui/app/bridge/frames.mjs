import { connectOwnerLocal, OWNER_LOCAL_SOCKET_NAME } from '@circular/client/owner-local';

const fail = code => Object.assign(new Error(code), { code });

export async function connect({ state, socketName = OWNER_LOCAL_SOCKET_NAME } = {}) {
  if (!state) throw fail('STATE_REQUIRED');
  return connectOwnerLocal({ root: state, socketName });
}

/**
 * One attachment's transport at a time. Each attachment is its own frame path: `deliver(attachment,
 * bytes)` hands the document each frame that arrives on that attachment's socket, and
 * `deliver(attachment, null)` once it ended. The document's frames name the attachment its
 * session stands on, and only that attachment's socket takes them; an attachment that is no longer the
 * one standing has closed, so a frame for it is refused, never written on the socket that replaced it.
 * The attachment a frame names is what says whose it is, never the order it arrives in: the shell's answer
 * to a new attachment and the last frames of the one it replaced reach the document by two separate ways,
 * and the answer was measured arriving first.
 */
export function frameRelay(deliver) {
  let current, issued = 0, pumping = Promise.resolve();
  const pump = async (attachment, transport) => {
    try { for await (const bytes of transport.incoming) deliver(attachment, bytes); }
    catch (error) { console.error('frames', error?.code ?? 'READ_UNAVAILABLE'); }
    finally { deliver(attachment, null); }
  };
  const finish = async previous => {
    if (!previous) return;
    let transport;
    try { transport = await previous.opening; } catch { return; }
    await transport.close();
    await pumping;
  };
  const end = async () => {
    const previous = current;
    current = undefined;
    await finish(previous);
  };
  return {
    get attached() { return current !== undefined; },
    async connection() {
      try {
        if (!current) throw fail('STATE_REQUIRED');
        const { attachment, opening } = current;
        await opening;
        return { connection: 'connected', attachment };
      } catch (error) { return { connection: error.code ?? 'READ_UNAVAILABLE' }; }
    },
    async attach(open) {
      await end().catch(error => console.error('frames-end', error?.code ?? 'READ_UNAVAILABLE'));
      const attachment = ++issued, opening = Promise.resolve().then(open);
      current = { attachment, opening };
      opening.then(transport => { pumping = pump(attachment, transport); }, () => {});
      return opening;
    },
    async send(attachment, bytes) {
      try {
        if (attachment === undefined || current?.attachment !== attachment)
          throw fail(attachment === undefined ? 'STATE_REQUIRED' : 'STREAM_CLOSED');
        await (await current.opening).send(bytes);
        return undefined;
      } catch (error) { return { code: error.code ?? 'READ_UNAVAILABLE' }; }
    },
    async close(attachment) {
      if (attachment === undefined || current?.attachment !== attachment) return;
      await end();
    },
    end,
  };
}
