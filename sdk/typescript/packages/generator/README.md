# `@circular/generator`

Prints compacted declaration commands as ordinary SDK source without a TypeScript dependency.
The journal remains the authority; generated source is a reconstruction for reading and editing.

```js
import { generateProgram, generatorEnvironment } from '@circular/generator';

const result = generateProgram(commands, { ...generatorEnvironment, specSet, catalog, admissions });
```

The result is complete source or structured rejection diagnostics. The package owns
`ProgramGeneratorOptions`, `GeneratedStructure`, `SourceProgramBundle`, and their shared
source and diagnostic types. Use `@circular/authoring` to prepare and execute source.


`generateProgram(commands, { bindings, sdkVersion, specSet, catalog, admissions })` reconstructs
flat programs. Catalog rows supply static primary ports; admission rows,
keyed by binding name, supply config-dependent ports. Unknown ports are emitted explicitly.
Concrete scopes reconstruct as `main.ts` plus `scopes/<name>.ts` modules, recursively. The installed host validates the whole bundle and merges parent topic name lists with child project boundaries. Template scopes use `templates/<name>.ts` with one project input and zero or more project outputs. `replicator` carries exactly `at`, `ttl`, and `capacity`; the installed host tries create-admission and records any container rejection before using the catalog path with the validated policy. This implements authoring round trips; cell execution remains outside this host.
The root entry exports `generateProgram` and `generatorEnvironment`; the latter supplies the installed SDK version and constructor bindings.

Reconstruction preserves mounts at their authored scope and resolves descendant endpoints relative
to that module. Presentation `size`/`board` zero extents and every existing wire `view.config` Value can be
printed and executed without loss. This is a codec round trip, not an admission promise: the current
engine rejects zero-extent boards and non-Null view configs. `operations` without an export
surface is outside the cross-edit set shared by the SDK and the canvas; no new spelling is introduced, and closing daemon
admission belongs to a separate engine change. Unknown commands, raw moves and combinator actor declarations remain
outside the compacted-state reconstruction domain; combinators are printed as edge preprocessing.
