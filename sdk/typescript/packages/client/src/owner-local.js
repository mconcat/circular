/**
 * The OwnerLocal transport: a Unix-domain socket to a daemon owned by the same user.
 *
 * This is the `transport` half of `establish(transport, options)`. It moves complete
 * frames and nothing else — it does not decode envelopes, choose a codec, or interpret a
 * session. `crates/transport/src/owner_local_socket.rs` is the counterpart, and it draws the
 * same line from the other side: "It does not encode protocol envelopes, choose a frame
 * profile, open a network listener, or invent remote authentication."
 *
 * ## What is verified before any byte is read
 *
 * The Rust side proves three things before exposing bytes, and this reproduces the two that
 * Actor can prove:
 *
 *   1. the socket root is the current effective user's non-symlink `0700` directory
 *   2. the socket pathname is that user's non-symlink socket with mode `0600`
 *   3. the connected peer has the same effective UID  ← **not reproducible here**
 *
 * **Actor cannot read peer credentials.** `SO_PEERCRED` and `LOCAL_PEERCRED` are not exposed by
 * `net.Socket`, and reaching them needs a native addon. So this client cannot confirm who is on
 * the other end, and saying otherwise would be the dangerous kind of wrong.
 *
 * What carries the weight instead is check 1. A `0700` directory owned by the effective user
 * cannot be written by anyone else, so no other user can create or replace the socket inside
 * it — an impostor would already need this user's privileges, at which point peer credentials
 * would not have helped either. Check 3 remains real defence in depth on the daemon side, where
 * it is implemented; it is absent on this side and that asymmetry is stated rather than hidden.
 *
 * The socket's identity is re-checked after connecting, because the path could have been
 * replaced between the check and the connect. Rust does the same, for the same reason.
 *
 * ## Framing
 *
 * **Envelopes do not sit on the stream directly.** They are carried inside the OwnerLocal
 * canonical frame — see `canonical-frame.js` — and this module moves whole envelopes over that
 * layer without looking inside either one.
 *
 * It did read envelopes straight off the socket once, using the twelve-byte head's own declared
 * length. That is self-consistent and was wrong: the daemon wraps every envelope in an
 * eleven-byte transport frame, so the first thing this side read would have been a transport
 * head interpreted as an envelope head. Nothing sent could have been read, and no test on
 * either side could see it, because both sides of *this* stream agreed with themselves — the
 * gap was only visible in the other implementation's source.
 */

import { lstat } from "node:fs/promises";
import net from "node:net";
import process from "node:process";

import {
  OWNER_LOCAL_MAX_MESSAGE_BYTES,
  OwnerLocalFrameReader,
  encodeMessageFrames,
} from "./canonical-frame.js";

/**
 * How many complete frames may wait before the transport stops reading.
 *
 * A default rather than a required value, unlike the frame size and the codec ceilings: those
 * bound what one peer can make this side allocate, and a caller who did not choose them would
 * inherit a limit against a hostile sender. This one bounds a backlog the *caller* creates by
 * not reading, so a default is a reasonable place to start and the caller can raise it.
 */
export const DEFAULT_MAXIMUM_PENDING_FRAMES = 1024;

const OWNER_ROOT_MODE = 0o700;
const OWNER_SOCKET_MODE = 0o600;
const PERMISSION_AND_SPECIAL_BITS = 0o7777;

export class OwnerLocalTransportError extends Error {
  constructor(code, message, options) {
    super(`${code}: ${message}`, options);
    this.code = code;
  }
}

function fail(code, message) {
  throw new OwnerLocalTransportError(code, message);
}

/**
 * The identity of a path, as the pair that says "still the same file".
 *
 * A path is not an identity: it can be unlinked and recreated between two looks. Device and
 * inode together are what makes the second look comparable to the first.
 */
function identityOf(stats) {
  return `${stats.dev}:${stats.ino}`;
}

async function inspectOwnerRoot(root, ownerUid) {
  let stats;
  try {
    stats = await lstat(root);
  } catch (error) {
    if (error.code === "ENOENT") fail("ROOT_MISSING", `${root} does not exist`);
    fail("ROOT_UNREADABLE", `${root} could not be inspected: ${error.code}`);
  }
  if (stats.isSymbolicLink()) fail("ROOT_IS_SYMLINK", `${root} is a symbolic link`);
  if (!stats.isDirectory()) fail("ROOT_NOT_DIRECTORY", `${root} is not a directory`);
  if (stats.uid !== ownerUid) {
    fail("ROOT_OWNER_MISMATCH", `${root} is owned by uid ${stats.uid}, not ${ownerUid}`);
  }
  const mode = stats.mode & PERMISSION_AND_SPECIAL_BITS;
  if (mode !== OWNER_ROOT_MODE) {
    fail("ROOT_MODE_MISMATCH", `${root} has mode ${mode.toString(8)}, not 700`);
  }
  return stats;
}

async function inspectSocket(socketPath, ownerUid) {
  let stats;
  try {
    stats = await lstat(socketPath);
  } catch (error) {
    if (error.code === "ENOENT") fail("SOCKET_MISSING", `${socketPath} does not exist`);
    fail("SOCKET_UNREADABLE", `${socketPath} could not be inspected: ${error.code}`);
  }
  if (stats.isSymbolicLink()) fail("SOCKET_IS_SYMLINK", `${socketPath} is a symbolic link`);
  if (!stats.isSocket()) fail("SOCKET_NOT_SOCKET", `${socketPath} is not a socket`);
  if (stats.uid !== ownerUid) {
    fail("SOCKET_OWNER_MISMATCH", `${socketPath} is owned by uid ${stats.uid}, not ${ownerUid}`);
  }
  const mode = stats.mode & PERMISSION_AND_SPECIAL_BITS;
  if (mode !== OWNER_SOCKET_MODE) {
    fail("SOCKET_MODE_MISMATCH", `${socketPath} has mode ${mode.toString(8)}, not 600`);
  }
  return stats;
}

/**
 * Connects to an OwnerLocal daemon and returns a `Transport`.
 *
 * `maximumMessageBytes` defaults to the profile's own bound rather than being required. The
 * value ceilings are still required and the reasons differ: nothing publishes a ceiling for a
 * decoded value, so a caller who did not choose one would inherit a limit they cannot reason
 * about, while the transport profile publishes both of its bounds and a caller cannot raise
 * them anyway — a larger message is refused at the other end. Lowering stays the caller's.
 */
export async function connectOwnerLocal(options) {
  const { root, socketName } = options;
  if (typeof root !== "string" || !root.startsWith("/")) {
    fail("ROOT_NOT_ABSOLUTE", "the socket root must be an absolute path");
  }
  if (typeof socketName !== "string" || socketName.length === 0 || socketName.includes("/")) {
    fail("SOCKET_NAME_INVALID", "the socket name must be a single path component");
  }

  const ownerUid = process.getuid?.();
  if (ownerUid === undefined) fail("UID_UNAVAILABLE", "this platform reports no effective uid");

  const socketPath = `${root}/${socketName}`;
  await inspectOwnerRoot(root, ownerUid);
  const before = identityOf(await inspectSocket(socketPath, ownerUid));

  const socket = await new Promise((resolve, reject) => {
    const pending = net.connect(socketPath);
    pending.once("connect", () => resolve(pending));
    pending.once("error", (error) => reject(new OwnerLocalTransportError("CONNECT_FAILED", error.message, { cause: error })));
  });

  try {
    const after = identityOf(await inspectSocket(socketPath, ownerUid));
    if (after !== before) fail("SOCKET_IDENTITY_CHANGED", "the socket was replaced while connecting");
  } catch (error) {
    socket.destroy();
    throw error;
  }

  return frameTransport(socket, options);
}

/**
 * Wraps a connected byte stream as an envelope transport.
 *
 * Exported so a test can drive the framing over any stream pair, and so the socket's
 * verification and the framing stay separable.
 *
 * What crosses this boundary is a **whole envelope**, in both directions. The transport frames
 * on the way out and reassembles on the way in, and neither side of it interprets an envelope:
 * the caller does not see the frame layer and the frame layer does not see the envelope.
 */
export function frameTransport(socket, options = {}) {
  const channel = options.channel ?? 0;
  const maximumMessageBytes = options.maximumMessageBytes ?? OWNER_LOCAL_MAX_MESSAGE_BYTES;
  const reader = new OwnerLocalFrameReader({ maximumMessageBytes });
  const maximumPendingFrames = options.maximumPendingFrames ?? DEFAULT_MAXIMUM_PENDING_FRAMES;
  if (!Number.isSafeInteger(maximumPendingFrames) || maximumPendingFrames <= 0) {
    fail("MAXIMUM_PENDING_FRAMES_INVALID", "maximumPendingFrames must be a positive integer");
  }
  const ready = [];
  let paused = false;
  let waiting = null;
  let closed = false;
  let failure = null;

  const settle = () => {
    if (waiting === null) return;
    const resume = waiting;
    waiting = null;
    resume();
  };

  const applyBackpressure = () => {
    if (!paused && ready.length >= maximumPendingFrames) {
      paused = true;
      socket.pause();
    } else if (paused && ready.length === 0) {
      paused = false;
      socket.resume();
    }
  };

  socket.on("data", (chunk) => {
    if (failure !== null) return;
    try {
      ready.push(...reader.push(chunk));
    } catch (error) {
      failure = error;
      socket.destroy();
    }
    applyBackpressure();
    settle();
  });
  socket.on("error", (error) => {
    if (failure === null) failure = new OwnerLocalTransportError("STREAM_FAILED", error.message);
    settle();
  });
  socket.on("close", () => {
    closed = true;
    settle();
  });

  return Object.freeze({
    /**
     * Sends one whole envelope.
     *
     * Every frame of a message is written before the promise settles. A partial write left to
     * the caller would put half a message on the stream, and the peer reading it cannot tell a
     * half-sent message from a malformed one.
     */
    send(envelope) {
      const frames = encodeMessageFrames(channel, envelope, { maximumMessageBytes });
      return new Promise((resolve, reject) => {
        if (failure !== null) return reject(failure);
        if (closed) return reject(new OwnerLocalTransportError("STREAM_CLOSED", "the transport is closed"));
        let remaining = frames.length;
        for (const frame of frames) {
          socket.write(frame, (error) => {
            if (error) return reject(new OwnerLocalTransportError("SEND_FAILED", error.message));
            remaining -= 1;
            if (remaining === 0) resolve();
          });
        }
      });
    },

    /** How many complete envelopes are waiting to be read. */
    get pending() {
      return ready.length;
    },

    incoming: {
      async *[Symbol.asyncIterator]() {
        for (;;) {
          while (ready.length > 0) {
          const frame = ready.shift();
          applyBackpressure();
          yield frame;
        }
          if (failure !== null) throw failure;
          if (closed) {
            if (reader.pending > 0 || reader.collecting) {
              throw new OwnerLocalTransportError("STREAM_TRUNCATED", "the stream ended inside a message");
            }
            return;
          }
          await new Promise((resolve) => {
            waiting = resolve;
          });
        }
      },
    },

    close() {
      return new Promise((resolve) => {
        if (closed) return resolve();
        socket.end(() => resolve());
      });
    },
  });
}
export { OWNER_LOCAL_SOCKET_NAME, OWNER_LOCAL_RESOURCE_CEILINGS } from './connection-defaults.js';
