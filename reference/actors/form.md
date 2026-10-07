# form

> **Alpha:** On the canvas, the New Form dialog takes `fields` as a list of rows, each a
> field name and a type (`string`, `int`, `float`, `bool`, `bytes`, `uint` or `null`), and
> writes the value the daemon expects. A value that list cannot show, such as a nested
> shape, stays in the generic value editor. The SDK accepts a field-name to base-type map:
> `form({ fields: { question: "string", count: "int" } })`. The raw value `fields: []` is refused with
> `ConfigRejected`. The canvas submits zero rows as `fields: [1n, [4n, [], false]]`.
> The daemon accepts that type expression as a form with no fields.

A place where a person fills in a typed draft and commits it into the graph. `form` does not
produce values on its own: it is an injection boundary, like `input`, with one difference —
it carries the type of that boundary in its own configuration.

While someone is filling the form in, the draft is local to the client. The graph sees one
thing: the commit.

## Ports

| Direction | Name | Payload | Required |
| --- | --- | --- | --- |
| Outlet (inside the scope) | derived boundary port **(primary)** | Exactly the type carried by `fields` | — |

The registration declares one injectable inlet; like `input`, the wiring face of that
declaration inside the scope is an outlet. There are **no authored inlets** and no `_error`
outlet.

Having exactly one port is a statement of the type, not a check performed on you.

## Configuration

| Key | Type | Required | Meaning |
| --- | --- | --- | --- |
| `fields` | In the SDK, a map of field name to base type (`{ question: "string" }`); a shape the map cannot spell, such as a nested one, as the array carrying the canonical type expression | **Yes — no default** | The fields a person fills in, each with its type; a committed form carries exactly these fields. |

`fields` reuses the published canonical type-expression carrier, so there is no second
schema language for form fields: the object-shaped field list **is** the form's fields. A
structure the client cannot draw falls back to raw entry on the client side; that is a
presentation choice, not part of this contract.

In SDK source, `fields` accepts a plain object of field names to `BaseShape` spellings from
`@circular/protocol`. Object key order becomes field order. The SDK lowers the map to the
same canonical array as the canvas. An empty map, `fields: {}`, declares a form with no
fields. Canonical arrays remain accepted for shapes the map cannot express.

Generated source (`current.ts`) uses the map for a Stream of a closed Object containing
only Base fields when object key order preserves the fields. Other values retain their
canonical array spelling.

`fields` is required: a declaration without it is refused. The SDK throws
`authoring.prepass.config-not-literal`, and the daemon answers `ConfigRejected`.

## State

Form runs as a stateless boundary actor that forwards an accepted commit. It has no
checkpointed state.

The draft, the cursor and the validation markers all belong to the client and are not
pipeline values.

## Rejections

| Code / name | When |
| --- | --- |
| `authoring.prepass.config-not-literal` | The SDK receives a non-object `fields` value other than a canonical array, or a map value outside the published `BaseShape` vocabulary. |
| `ConfigRejected` | `fields` is missing, or is not a canonical type-expression carrier. A free-form string is refused here, because accepting one would turn `fields` into a second schema language. |
| Format fault (injection) | A committed value outside the type the boundary declares. **No arrival is produced.** This is a fault in the value, not a failed mount lookup, so it is reported with a format-fault code and the next step is to resend a value that fits. Accepting it quietly would make the form's schema decorative. |

Reasons a form may be unusable at a given moment — no fields configured yet, no injectable
input, an unbound field input, an unconnected commit provider — are runtime and topology
facts, not failures of this actor.

## Example

```ts
import { form } from "@circular/core";

export let request = form({ fields: { question: "string", count: "int" } });
```

On the canvas, New Form with the rows `question: string` and `count: int` declares the same
fields in the same order.
