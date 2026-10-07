# `@circular/protocol`

Runtime and TypeScript surface for Circular's existing RPC vocabulary. The package exposes envelopes,
address-domain-parametric declaration commands, query pages, subscription frames, session negotiation,
interaction payloads, replay messages, and transport/codec ports.

It deliberately does not expose a graph-shaped pipeline snapshot. Current authored state is represented only
as a compacted, reconstructive list of the same `DeclarationCommand` union used by mutation and accepted
history.

Small runtime constructors cover result values, descriptors, envelopes, and a zero-copy in-process codec.
There is deliberately no JSON wire codec: remote transports supply their negotiated codec (for example,
generated Cap'n Proto bindings) to `@circular/client`.

`valueKey(value): string` returns the canonical Value codec bytes as lowercase hexadecimal for
Map keys, and `sameValue(a, b): boolean` compares those same bytes. Both are root exports of
`@circular/protocol`; NaN and null, -0 and 0, and `1n` and `{integer:"1"}` are distinct.
`valueKey` throws for values the codec cannot encode. `sameValue` preserves the SDK's absence
rule (`undefined` equals only `undefined`); other unencodable values compare false.

Open implementation decisions remain visible in the declarations: physical tag and digest encodings,
`InjectionKey` and token width limits, codec bindings, and several replay-control mappings. None of those open
decisions creates a second reconstruction format.

## Integers are 64-bit, and JavaScript cannot hold all of them in a `number`

The value model has a signed 64-bit integer kind, `Int`. This package carries it as a
JavaScript `bigint`. The unsigned 64-bit kind, `UInt`, is carried by `CircularUInt`. A
JavaScript `number` is encoded as a `Float`, even when its value is a whole number. This
section describes what that means for TypeScript callers.

**The problem in one line.** A JavaScript `number` holds every integer up to
`Number.MAX_SAFE_INTEGER`, which is 9007199254740991 — about 2⁵³. An `i64` goes to
9223372036854775807. The gap is not a rounding detail: values in it convert *silently*.

```js
Number(9007199254740993n)   // 9007199254740992   — off by one, no error
Number(2n ** 63n - 1n)      // 9223372036854776000 — off by 193, no error
9007199254740993            // 9007199254740992   — the source literal already lost it
```

The third line is the one to notice. **A TypeScript source file cannot express the value**, so
the loss happens before any codec is involved.

**This is not hypothetical.** Nanoseconds since the Unix epoch passed 2⁵³ years ago and are
now about 1.7 × 10¹⁸, and the arrival record carries a u64 wall clock. Any code reading such a
field through a plain `number` is already wrong.

### What that means for you

**Reading an integer that may exceed 2⁵³ requires `BigInt`.** There is no third option: a
`number` cannot represent the value, and a string moves the problem rather than solving it.

`BigInt` carries three sharp edges, all verified against this runtime:

| | Behaviour |
|---|---|
| `JSON.stringify(1n)` | **throws** `TypeError: Do not know how to serialize a BigInt` — and so does any object containing one |
| `1n + 1` | **throws** `TypeError: Cannot mix BigInt and other types` — mixed arithmetic needs an explicit conversion |
| `structuredClone(1n)` | works |
| `1n == 1` / `1n === 1` | `true` / `false` — loose equality crosses the types, strict equality does not |

The first two matter most. **A `BigInt` crossing a JSON boundary throws rather than degrading**,
so the failure is loud, which is the good case. **Mixed arithmetic also throws**, so an
expression that worked while a field was a `number` breaks when that field becomes an integer —
loudly again, but at runtime.

The asymmetry worth remembering: `structuredClone` accepts `BigInt` and `JSON.stringify` does
not, so a value can cross the code-mode host boundary and fail at a JSON boundary in the same
program.

### Why the SDK does not simply cap integers at 2⁵³

This track proposed exactly that during the audit, and it was rejected. The argument against
it is worth knowing, because it explains why the awkwardness above is deliberate:

- A 2⁵³ ceiling is not a fact about the value model. It is an artifact of one host language's
  number type, and choosing it would rebuild the precision cliff the audit had just judged
  artificial.
- Canonical bytes outlive every implementation. Binding them to JavaScript's number model
  would make every future language's consumer emulate JavaScript's arithmetic permanently.
- The epoch-nanosecond case above is a live counterexample rather than a hypothetical: at
  a 2⁵³ ceiling that value has to be carried as a string, which is the workaround the value
  model was changed to remove.

So the boundary is real and stays. This section exists so that it is met in documentation
rather than in a silently wrong timestamp.

### Authoring expressions are unaffected

Numeric literals in authored callbacks stay reals, so `4` in an authored
expression is a `Float` and `x.length / 4` keeps evaluating in floating point exactly as it
does today. Integers arrive from CEL standard-library results, from external data, and from
positions declared integral; they are not something the authoring surface produces by accident.
