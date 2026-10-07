# Presentation and `.view()`

An actor's `.view(kind, config)` records `{ kind, config }` in
`presentation.view` through `SetPresentation`.
The kind is a string carried exactly as authored.
The config holds shared presentation vocabulary alongside data owned by that view.
`.label(text)` names the actor outside its body; `heading` names the body itself.

`@circular/core` exports `ViewVocabulary`, `ViewVocabularyKey`, `ViewFieldRole`,
`ViewFieldPath`, `ViewTotal`, `ViewConfig`, `ViewConfigValue`, `validateViewConfig`
and `resolveViewConfig` from one vocabulary module.

## One vocabulary, two supply points

Type registration supplies a default config value.
An actor supplies overrides with `.view(kind, config)`.
Both use these keys.

| Key | Value | Meaning |
| --- | --- | --- |
| `heading` | String or `null` | Card body heading, also used as the table title. |
| `count_label` | String or `null` | Text beside a count already obtained from recorded facts. |
| `columns` | Array of `{ path: ViewFieldPath, label: string }`, or `null` | Declares the displayed columns, their order and their labels. Each path selects a value within a row. |
| `caption` | String or `null` | Authored context below the displayed content. |
| `total` | `{ outlet: string, path: ViewFieldPath }` or `null` | Selects an already recorded value from the named outlet. Its name comes from the existing port label or id. |
| `fields` | Partial map from `title`, `status`, `value` to `ViewFieldPath`, or `null` | Assigns semantic roles to fields within a recorded body. |
| `side` | `"emitted"`, `"arrivals"`, `"both"` or `null` | Declares which recorded rows the view reads: this actor's emissions, its arrivals, or both. |
| `rows` | `"latest"`, `"outlets"` or `null` | Declares whether the rows are the latest emission's entries or the actor's declared outlets. An outlet row shows that outlet's latest emission, its time and whether the outlet is wired. It does not show a count of rows in the screen window. |
| `spark` | `"none"`, `"samples"` or `null` | Declares whether the view draws the sparkline of its inlet samples. |

The body role set is closed: `title`, `status`, `value`.
`side`, `rows` and `spark` each take one of the listed strings; any other value is rejected
through `CIRCULAR_VIEW_CONFIG_INVALID`.
The validator does not check which view kind reads a key.
A feed can use `title` for its source and `value` for its content.
Column paths bind labels to the selected values independently of object field order.
The projection array supplies the displayed column order.
Duplicate paths are rejected through `CIRCULAR_VIEW_CONFIG_INVALID`.
The caption does not duplicate the type's registered description.
No key changes a config key, port label, type label or type description.

A field path uses the same `ExactPayloadPath` representation as `join.at` and
`keyed_reduce.value`: an array of String keys and nonnegative Int (`bigint`) indices.
Indices range from `0n` through `9223372036854775807n`.
`[]` selects the whole body.
`["body", "rows", 0n, "value"]` selects a nested member.
`["a.b"]` selects the literal key `a.b`; it is not a dotted path.
Numbers such as `0` are Float values, so they are not path indices.
The vocabulary selects recorded data; `total` does not compute a sum or a count.
Missing data stays missing.

## Resolution and removal

`resolveViewConfig(actorConfig, typeDefault)` resolves each top-level key from the
actor declaration, then the type default, then absence.
An omitted key inherits.
An own `null` removes the key and suppresses its type default.
Arrays and nested objects replace as a whole; there is no deep merge.
For example, `{ fields: { value: ["body"] } }` replaces the entire default role map.
`fields: null` removes that map.
Omitting `columns` inherits the type default.
`columns: null` removes the default projection, so the interpreter derives columns.
`columns: []` explicitly displays no columns.
An empty string is an authored string, not an instruction to inherit.
A null or non-record config has no vocabulary keys.
View-specific scalar and array data remains part of the declaration.

```ts
import { resolveViewConfig } from "@circular/core";

const resolved = resolveViewConfig(
  { heading: null },
  { heading: "Weighted mean", count_label: "samples" },
);
// resolved is { count_label: "samples" }.
```

`validateViewConfig` checks the shared keys at either supply point.
`.view()` and `.replacePresentation()` call the same validator.
Invalid vocabulary throws a `TypeError` with code `CIRCULAR_VIEW_CONFIG_INVALID`.
The execution host preserves that code and the offending config path in its existing
diagnostic arguments.
Other config keys remain owned by the selected view.

## Examples

These calls use existing actor handles.
They add presentation declarations without changing the actor's computation.

Numeric view:

```ts
mean.view("trend", {
  heading: "Weighted mean",
  count_label: "samples",
  caption: "Current window",
});
```

Feed view, with a body containing `source` and `line`:

```ts
lines.view("feed", {
  heading: "Incoming lines",
  columns: [
    { path: ["source"], label: "Source" },
    { path: ["line"], label: "Value" },
  ],
  fields: { title: ["source"], value: ["line"] },
});
```

Table view, with recorded rows containing `source`, `contribution` and `updated`,
and a declared `total` outlet:

```ts
totals.view("table", {
  heading: "Contributions",
  columns: [
    { path: ["source"], label: "Source" },
    { path: ["contribution"], label: "What it contributes" },
    { path: ["updated"], label: "Updated" },
  ],
  caption: "Recorded contributions",
  total: { outlet: "total", path: [] },
});
```

Timing view:

```ts
clock.view("timing", {
  heading: "Periodic ticks",
  count_label: "ticks emitted",
});
```

Config form text belongs to type-level slots, not `.view()`.
`ConfigSlotMetadata` defines three String-or-null fields directly on each slot.
`label` names the slot, `description` supplies help, and `group` names a form group.
Null means no declared text.
The slot's existing path, shape, constraint and requirement still define the input.

```ts
import type { ConfigSlotMetadata } from "@circular/core";

const quietWindowText: ConfigSlotMetadata = {
  label: "Quiet window",
  description: "How long the inlet must stay quiet before the pending payload is emitted.",
  group: "Timing",
};
```

Response/output body roles, with `status` and `body` in the recorded response:

```ts
response.view("response", {
  heading: "Response",
  fields: { status: ["status"], value: ["body"] },
});
```

The same `fields` vocabulary applies to an `output` view.
A body with its own title can also declare `title: ["title"]`.
