/**
 * The closed tables inside this package, frozen, and the two readings several modules share.
 *
 * The tables are `@circular/protocol/tables` (`../tables.generated.js`): data the Rust declarations
 * generate (`cargo run --locked -p engine --example closed_tables`), checked in, and held equal to
 * the declarations by one engine test. Each SDK check that a value belongs to one of these tables
 * takes the table from there. So do these types: `DaemonHealthItem` `state` and `reason` (client `index.d.ts`),
 * `PreprocessStep` `kind`, `DeadLetterReasonCode` and `BaseShape` (protocol `index.d.ts`). The
 * `records`, `actor.events` and `authoring-commits` registrations (`record-values.js`) take their
 * name, paging form and delivery discipline from the `QueryId` and `SubscriptionTarget` rows.
 * Their `anchorKind` is not in those rows and is spelled in `record-values.js`.
 *
 * Lists, unions and per-arm branches that spell these tables' values by hand remain here:
 * - type unions: `Partition`, `StableVerb`, `PartitionVerb`, `IncarnationPhase`,
 *   `ConfigInputSnippet` `mode`, `TimelineMarkKind` and the payload arms of `DeadLetterReason`
 *   (protocol `index.d.ts`); the frame arm and origin unions (`subscription-values.d.ts`, client
 *   `subscription.d.ts`) and the ending reason union (client `subscription.d.ts`);
 *   `DaemonHealthAnchor` `lifecycle` (client `index.d.ts`); the registration keys, their paging
 *   and discipline type arguments, the `system` kinds and code numbers and the `approval`
 *   decision arms (`record-values.d.ts`);
 * - sums whose arms carry their own shapes: the command, frame and ending types in protocol
 *   `index.d.ts`, and the decoders that branch on each arm (`subscription-values.js`,
 *   `declaration.js`, `observation-rows.js`, `record-values.js`, client `subscription.js`), and the
 *   per-combinator config keys (core `runtime.js`, authoring `prepass.js`, generator `generator.js`);
 * - subsets that no Rust declaration names: `RECONSTRUCTIVE_KINDS` and `DELTA_ROWS`
 *   (`declaration.js`), `UNIT_REASONS` (`observation-rows.js`), the health states that carry a
 *   reason (`record-values.js`), `CONTENT` and `ENVELOPE` (generator `generator.js`), and the
 *   reasons a `match` `err` outlet receives: its type (core `catalog.generated.d.ts`) admits every
 *   `DeadLetterReason`, while each product `Err` envelope is built with `processing` and
 *   `match` passes an upstream reason on (crates `match_actor.rs`);
 * - the other query registrations' names, paging and anchors (`record-values.js`,
 *   `observation-values.js`, `timeline-values.js`, client `index.js`).
 * Code that names one arm or one registration in order to use it, such as sending a `Query` verb
 * or querying `actor.catalog`, is not listed.
 */
import * as generated from '../tables.generated.js';

function frozen(value) {
  if (value !== null && typeof value === 'object') {
    for (const child of Object.values(value)) frozen(child);
    Object.freeze(value);
  }
  return value;
}

for (const table of Object.values(generated)) frozen(table);

export * from '../tables.generated.js';

/** One boundary's codec ceilings (`Ceilings::for_boundary`), in the SDK's option names. */
export function resourceCeilings(boundary) {
  const limits = generated.Ceilings[boundary];
  return Object.freeze({
    maximumBytes: limits.max_bytes,
    maximumDepth: limits.max_depth,
    maximumContainerEntries: limits.max_container_entries,
    maximumStringBytes: limits.max_string_bytes,
  });
}

/**
 * The number a rejection reason carries in answers outside Lifecycle — the first of
 * `RejectionReason::numbers`. A reason the table does not hold is a failure here, not an
 * `undefined` that compares false.
 */
export function rejectionCode(reason) {
  const row = generated.RejectionReason.find(candidate => candidate.name === reason);
  if (row === undefined || row.numbers[0] === null) {
    throw new Error(`closed tables: ${reason} has no number outside Lifecycle`);
  }
  return row.numbers[0];
}
