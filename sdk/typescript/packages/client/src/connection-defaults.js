import { Ceilings } from '@circular/protocol/tables';

/** crates/transport/src/owner_local_invocation.rs::OWNER_LOCAL_SOCKET_NAME. */
export const OWNER_LOCAL_SOCKET_NAME = 'daemon.sock';

export const OWNER_LOCAL_RESOURCE_CEILINGS = Object.freeze({
  maximumBytes: Ceilings.Wire.max_bytes,
  maximumDepth: Ceilings.Wire.max_depth,
  maximumContainerEntries: Ceilings.Wire.max_container_entries,
  maximumStringBytes: Ceilings.Wire.max_string_bytes,
});
