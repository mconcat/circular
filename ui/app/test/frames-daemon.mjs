import { envelope, wireEnvelopeCodec } from '@circular/protocol';
import { OWNER_LOCAL_RESOURCE_CEILINGS } from '@circular/client';

export const codec = wireEnvelopeCodec(OWNER_LOCAL_RESOURCE_CEILINGS);
export const helloAck = request => envelope('SessionMechanics', 'HelloAck', request.correlation,
  { features: request.payload.features, protocol_version: 1n, roles: [], token: new Uint8Array(32), trust: 1n });

/**
 * `respond(request)` answers one request envelope with an envelope, a promise of one, or nothing.
 * Hello is answered with HelloAck unless `respond` answers it. `sent` keeps every request frame's
 * bytes; `end()` ends the stream as the shell does when its socket closes.
 * The fixture is one attachment of the bridge, `attachment`. Its frames are handed with it, and
 * a frame the document sends for any other attachment is refused as the shell's relay refuses it.
 */
export function framesDaemon(respond = () => undefined, { attachment = 1 } = {}) {
  const handlers = [], sent = [];
  const deliver = bytes => { for (const handler of handlers) handler(attachment, bytes); };
  return {
    attachment, sent, closed: 0,
    async send(named, bytes) {
      if (named !== attachment) return { code: 'STREAM_CLOSED' };
      sent.push(Uint8Array.from(bytes));
      const { envelope: request } = codec.decode(bytes);
      const answer = await (respond(request) ?? (request.kind.verb === 'Hello' ? helloAck(request) : undefined));
      if (answer) deliver(codec.encode(answer));
      return undefined;
    },
    onFrame(handler) { handlers.push(handler); },
    async close() { this.closed += 1; },
    end() { deliver(null); },
  };
}
