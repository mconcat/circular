# `@circular/core`

Build-free ESM runtime and TypeScript declarations for Circular v2 pipeline authoring. Tooling and
canonical executable bundles import this package directly as `@circular/core`.

Topology reads upstream to downstream. A source endpoint appends a downstream catalog actor, connects to an
existing target with `into(target.in.port)`. Detached constructors accept inlet maps for
cycles and forward references. New handles, anchored current handles, projected GUI values, and runtime actor
identities are separate domains; this package declares only the first two authored domains.

The public catalog contains every currently named v2 surface, preserves the exact child-boundary spellings,
and exposes both registered replicator spellings. It does not copy v1-only actors. Config and named-port
surfaces whose generated v2 registration is incomplete remain nominal unresolved types supplied by
`@circular/specs`; the declarations never widen them to arbitrary dictionaries.

`match(source)` (or `match()` followed by an incoming wire) is a captured constructor.
Its `ok` endpoint carries the input value and `err` carries the existing failure reason; the
Ok/Err tag stays outside the payload. Both endpoints can be wired with `into`, and `match`
is not an implicit chain method. Catalog generation owns the constructor and its specialized type.

Every constructor and fluent mutation is synchronous. It emits an existing protocol
`DeclarationContentCommand` into the host-installed execution epoch and returns a symbolic handle; it does
not wait for an RPC response and does not build a second mutation IR. `@circular/core/internal` is a
host-adapter-only subpath that installs the command sink, pinned constructor resolver, reference allocator,
and snapshot-bound current handles. It is deliberately absent from the public package entry point.

```ts
import { json } from "@circular/core";

export const smoothed = json({ value: 10 })
  .ema({ halfLife: "30s" })
  .label("smoothed value");

export const samples = smoothed
  .tap()
  .counter()
  .label("sample count");
```

Registered string view kinds lower to `SetPresentation`. Structural static view values and explicit layout
gaps currently fail before emission because the existing protocol payload cannot round-trip them. The same
payload must be extended before those calls can be implemented without lossy side data.
