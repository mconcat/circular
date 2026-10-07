# @circular/specs

The daemon owns actor types, configuration admission, and ports. The SDK consumes
`actor.catalog`, `actor.create-inputs`, and `actor.create-admission` responses.
`createProviderBindingRegistry` maps SDK imports to constructor names; it does not
supply spec identity or validate the daemon's configuration rules.

The captured catalog/create-inputs pair is a test golden and input to declaration
code generation. It is not the connected daemon's current catalog or a spec_set.
Attach reads the connected daemon and writes its authoring reference beside the
reconstructed SDK program. The execution host consumes daemon admission before
opening a mutation epoch, including approval policy rejections.

For effect actors, the `capabilities` slot in `actor.create-inputs` carries a
`policies` object. Its keys name the capabilities declared by that actor; each
value is an array of input slots with full config paths. The `approval` slot
publishes the closed values `"none"` and `"required"`. Filesystem policies also
publish `roots` as an array of strings; admission requires absolute paths.
These inputs are mandatory within a demanded policy. The `capabilities` slot's
own requirement says which policies are demanded. When it is mandatory
(`request`, `notify`, `file`, `listener`), every policy it lists is demanded, and
the draft's `missing` list names `capabilities`. When it is optional
(`tool_executor`), only the effects of the authored tools demand their
corresponding policies. The policy metadata adds no defaults to the draft.
Supply explicit approval and roots for the demanded policies, then submit the
config to `actor.create-admission`.

For example, a request's metadata names `HttpFetch` and the path
`capabilities.HttpFetch.approval`. Author
`capabilities: { HttpFetch: { approval: "required" } }` to request approval for
that authority. Select `"none"` only when the intended policy requires no approval.

The artifact registry/loader and the local config and port interpreters are retired.
Some legacy structural declarations are still present; whether they are removed is
not yet decided.
