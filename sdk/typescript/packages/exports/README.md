# `@circular/exports`

There is no standalone `mount()` export in this package. The old three-argument function is retired.
`handle.mount()` is available only through an authoring host implementing export declarations.
Outside that host, or with an incomplete host, it rejects with `CIRCULAR_EXPORT_MOUNT_HOST_MISSING`
and an explicit message that export mounting is not implemented by that host. Importing the package
does not install the host. Declare mounts through the installed SDK code execution host.

Handles bind roles with `.mount(name, role?)`.
`surface(name, defineExport({ roles, params?, operations?, surfaces }))` attaches a screen to that name.
The three-argument mount and its old unspecified-wire error are retired.

The fixed roles are request, progress, result, and error. Request binds a writable ingress boundary;
other roles observe ports. A surface declares exactly the roles bound under its name.

The installed prepass reads structural AST without executing authored callbacks. It accepts all thirteen
export builders, their existing cell/placeholder modifiers, and symbolic role/parameter references.
Arbitrary JavaScript is rejected. `prompt` keeps its existing textInput/placeholder normalization.
The printer emits a surface line after its handle mount lines and preserves the core Value bytes.

The third member `Export.surface` is an optional core Value. The declaration and the
compacted snapshot use the same field.
Absence removes the screen; explicit null is preserved by the wire codec. A builder program uses
`defineExport` and the existing symbolic tokens. Unprintable foreign Values are diagnosed rather
than silently dropped by the generator.

A form export mode combines textInput, button, toggle, select, prompt and composer with a
messages view. Written as `main.ts` in an attach authoring session and deployed through that
session's code host, its input boundary binds request and its tap binds result to a name such as
`team-form`. A form's harness choices are submitted data: they do not grant or bind a harness.
Dock rendering and live request/observation are verified separately.
