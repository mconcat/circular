# tap

Passes every payload through unchanged and gives that point in the graph a name you can
observe. Use it when you want to watch what is actually flowing on an edge, or when you
need a stable named place to attach an observation mount.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Inlet | `event` **(primary)** | Any value | Yes |
| Outlet | `event` **(primary)** | The same payload, unchanged | — |

The registration uses the same item type and flow kind for the inlet and outlet. There is
no `_error` outlet.

Pass-through means payload equality, not event reuse. The emission is a new emission with
its own identity and stamp; what is preserved is the payload and the type.

## Configuration

None. `tap` takes no configuration. Its configuration value is `null`, and that is what
the published constructor sends. Any other value, including the empty object `{}`, is
refused at admission (see Rejections).

## State

Stateless. It does not retain the last input, the last output, or any observation snapshot.

## Rejections

`ConfigRejected` (code 1) at admission when the configuration is anything other than `null`:
the empty object `{}`, an object with keys, or any other value. The value is not normalised to
`null`, and the refusal points at no key because the schema has none.

At runtime there is nothing to refuse: `tap` does not inspect or transform the payload.

## Example

```ts
import { tap } from "@circular/core";

export const observed = upstream.tap();
observed.mount("what-is-flowing-here");
```
