# Wire preprocessing: map, filter, bang, parse, flatten

`map`, `filter`, `bang`, `parse` and `flatten` are chain methods on an outlet or a handle.
They are **not actors**. Each one adds a step to the receiving inlet of the wire it is
written on, so the steps belong to the destination actor. They do not appear as nodes,
they hold no state, and no value sits between two of them.

```ts
import { input } from "@circular/core";

export let events = input({ label: "events" });
export let failures = events.filter("event.level == 'error'").tap();
events.mount("events", "request");
failures.mount("failures", "result");
```

Here the filter runs on the inlet of `failures`. Another wire from `events` with a
different filter is a different branch, and neither branch sees the other's steps.

Steps on one wire run in the order they are written. A step that fails stops the rest of
that wire's steps for that value.

## The name of the arriving value

Inside a `map` or `filter` string the arriving value is named **`event`**. That is the only
declared name, whatever the destination inlet is called: a map on the wire into a `notify`
actor's `notification` inlet still reads `event`, not `notification`. Any other name is
refused when the program is admitted:

```text
ConfigRejected: actor `failures`; preprocess[0].config.predicate = "value.level == 'error'"; predicate was rejected: …
```

Map and filter accept literal CEL strings or supported single-expression arrow functions.
The authoring prepass lowers those functions to CEL and rejects unsupported callback syntax.

## Functions inside an expression

The functions an expression may call are a closed set. This page does not copy that set.
A call to a function outside it is refused when the program is admitted. The refusal
lists the function names that the admission check accepts:

```text
ConfigRejected: actor `joined`; preprocess[0].config.transform = "['a', 'b'].join(',')"; transform was rejected: `join` is not an available function; available functions: …
```

Numbers that arrive as text are read with `double(...)`, `int(...)` or `uint(...)`. The
Prometheus HTTP API is one such source: every sample value is a string, as in
`"value": [1727000000.5, "0.16"]`. `double(r.value[1]) > 0.05` compares that sample as a
number. Text that does not read as a number is an evaluation error. It is not read as `0`.

`NaN` is a floating-point value with no order. When both operands are floating-point
numbers and one is `NaN`, `==` answers `false` and `!=` answers `true`, while `<`, `<=`, `>`
and `>=` are evaluation errors, not `false`, because CEL does not order `NaN`. Comparing a
floating-point number with an integer has no CEL overload. Admission rejects the comparison
when both operand types are known, including `double('NaN') == 1`; otherwise it fails at
evaluation. A Prometheus `0/0` ratio arrives as the text
`"NaN"`. Test for it before comparing:
`r.value[1] != 'NaN' && double(r.value[1]) > 0.05`.

## The five steps

| Step | Config | What reaches the actor |
| --- | --- | --- |
| `map(transform)` | A CEL string or supported single-expression arrow function | The value of the expression. It replaces the arriving value. |
| `filter(predicate)` | A CEL string or supported single-expression arrow function | The arriving value unchanged when the predicate is `true`; nothing when it is `false`. |
| `bang()` | None | `null`. The content of the arriving value is dropped; only the fact that something arrived remains. |
| `parse({ decoder, field, arguments? })` | `decoder` is `"json"`, `"kv"` or `"regex"`; `field` names the text field to read (for `json`, a Bytes field holding UTF-8 text also reads) | The arriving object with `field` removed and the decoded fields merged in at the top level. A decoded field with the same name as an existing one wins. |
| `flatten({ at })` | `at` is a payload path to an array of objects | One value per array element: the arriving value with the array at `at` replaced by that element. An empty array produces nothing. |

### map

```ts
export let doubled = source.map("{'doubled': event.a * 2, 'kept': event}").tap();
```

`{ a: 21 }` arrives at `doubled` as `{ doubled: 42, kept: { a: 21 } }`. The result of a map
is whatever the expression builds; nothing of the old value survives unless the expression
includes it.

### filter

The predicate must produce a Bool. `false` drops the value on that wire. A non-Bool result
or an evaluation error is a failure, not a `false` (see "When a step fails").

### bang

`bang()` turns any arrival into `null`. Use it to count or trigger on arrivals whose content
does not matter.

### parse

`field` names a **String** field of the arriving object, or UTF-8 **Bytes** for `json`.
The three decoders read it as follows.

| `decoder` | `arguments` | Decoded fields |
| --- | --- | --- |
| `"json"` | none | The members of a JSON object. Integers become Int, numbers with a fraction or an exponent become Float. A top-level array, number or string is a failure, because there are no fields to merge. |
| `"kv"` | `{ pair_separator, value_separator }`, both non-empty strings | One String field per `key<value_separator>value` piece. Pieces without the value separator are skipped. |
| `"regex"` | `{ pattern }`, a non-empty RE2 pattern | One String field per named capture group that took part in the match. Unnamed groups produce nothing. No match is a failure. |

For example, with `field: "body"`:

| Arriving value | Step | Value that reaches the actor |
| --- | --- | --- |
| `{ status: 200, body: "{\"status\":\"success\",\"data\":{\"x\":1}}", other: "kept" }` | `parse({ decoder: "json", field: "body" })` | `{ status: "success", data: { x: 1 }, other: "kept" }` |
| `{ line: "a=1;b=two;junk" }` | `parse({ decoder: "kv", field: "line", arguments: { pair_separator: ";", value_separator: "=" } })` | `{ a: "1", b: "two" }` |
| `{ line: "GET /api/products 200" }` | `parse({ decoder: "regex", field: "line", arguments: { pattern: "^(?P<method>[A-Z]+) (?P<path>[^ ]+)" } })` | `{ method: "GET", path: "/api/products" }` |

In the first row the decoded `status` replaced the HTTP status, and a predicate downstream
reads `event.data`, not `event.body.data`.

**Bytes bodies.** `json` reads a String field, or a Bytes field holding UTF-8 text; invalid
UTF-8 fails the step. `kv` and `regex` read only a String field. A `request` response body
is Bytes, while a webhook body arrives as a UTF-8 String. JSON parse reads either directly;
`kv` and `regex` need `map("{'body': string(event.body)}")` first for a Bytes body.

```ts
export let parsed = probe.out.response
  .parse({ decoder: "json", field: "body" })
  .tap();
```

### flatten

`at` is a payload path: string elements select object keys and non-negative integer
elements select array indices. It must select an array whose elements are all objects.

```ts
export let perItem = source.flatten({ at: ["items"] }).tap();
```

`{ svc: "cart", items: [{ n: 1 }, { n: 2 }] }` arrives at `perItem` as two values,
`{ svc: "cart", items: { n: 1 } }` and `{ svc: "cart", items: { n: 2 } }`. Every element is
checked before any value is produced, so one non-object element fails the whole arrival.

## When a step fails

A step fails when its input is outside what it accepts: a map or filter expression that
does not evaluate, a filter result that is not a Bool, a `parse` field that is absent, not
a String or not in the decoder's grammar, a regex with no match, or a `flatten` path that
does not select an array of objects.

The failed value does not reach the actor and is not dropped silently. When the receiving
inlet is an ordinary inlet, the value becomes a **dead letter**. The `dead.letters` query
lists it with reason code `processing`, and its `detail` names the processing cause
and the failure point: the wire, the step's `index` in that wire, and its `kind`
(`"map"`, `"filter"`, `"parse"`, `"flatten"`). A cause of `3` means the value was outside
the domain of that step.

When the receiving inlet is the `event` inlet of a [`match`](actors/match.md) actor, the
failure is not a dead letter: `match` emits its reason on `err`, and a value that passed
every step leaves on `ok`. Wire through `match` when a failure should become data your
pipeline acts on.

## Observing a step's output

`arrival.scan` takes a registered mount name and reads recorded arrivals **before** inlet
preprocessing, so a value that a filter drops or a parse rejects still appears there. To
see what the steps produced, put a `tap` after the actor that carries them and read that tap:

```ts
export let parsed = raw.parse({ decoder: "json", field: "body" }).tap();
export let parsedOut = parsed.tap();
parsedOut.mount("parsed", "result");
```
