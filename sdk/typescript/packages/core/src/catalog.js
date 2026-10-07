import { endpointOf } from "./internal.js";
import { tap } from "./catalog.generated.js";
export * from "./catalog.generated.js";
export { join } from "./join.js";

export function merge(...sources) {
  if (sources.length < 2) throw new TypeError("merge() requires at least two sources");

  for (const source of sources) endpointOf(source, "source");

  const joined = tap();
  for (const source of sources) source.into(joined);
  return joined;
}
