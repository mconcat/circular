# `@circular/authoring`

Client-side authoring contracts and the host-owned execution boundary for Circular code mode.

The root entry point owns semantic prepass, one implicit existing-command epoch, complete authored-state
snapshot acquisition, and current-state virtual-module resolution. A complete snapshot is an anchor plus
the protocol's compacted list of existing `DeclarationCommand` values. Each client folds those commands
directly into the read model it needs. `generateProgram` from `@circular/generator` reconstructs §7 TypeScript for flat programs from
a complete structural command log, without a shared graph-shaped reconstruction model.

The host supplies one fixed `current` namespace from `circular:current`. Its actor, edge, scope, export,
and annotation lookups are built eagerly from a complete snapshot and an anchor-bound identity binder.
No property access performs RPC and no snapshot-specific ESM export table is generated.

`CodeExecutionHost.execute` is called by the outer host, never by authored code. It begins an epoch before
evaluation; synchronous SDK calls immediately enqueue exact existing `DeclarationContentCommand` RPCs;
normal module completion validates and commits, while exceptions abort. The returned command trace is an
observation of those exact messages, not a second mutation or graph IR.

The first build-free ESM implementation includes fixed-current indexing, epoch orchestration, and
source-map lookup. The pinned TypeScript semantic prepass is installed. The fixed current namespace
answers its five lookups and nothing else: the prepass refuses every other use of it, and reading
newer state means taking a complete snapshot again.


Program reconstruction is published by [`@circular/generator`](../generator/README.md), which has no TypeScript dependency.
The source bundle, diagnostic, and generated-result types are imported from that package.
