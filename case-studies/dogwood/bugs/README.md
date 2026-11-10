# Bugs: dogwood

The bugs the case studies found in dogwood. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [`count`/`sum` rejected with "`for`-variable does not occur in the aggregation body" when the variable is used only inside a temporal macro call](dogwood-agg-for-var-occurrence-check-ignores-mac/) | open | [link](https://github.com/dogwood-policy/dogwood/issues/21) |
| [Temporal validator types a bound variable by the last same-named binder in the condition, not the binder in scope](dogwood-binder-type-env-not-scoped/) | open | [link](https://github.com/dogwood-policy/dogwood/issues/22) |
| [`dogwood validate` drops the input/output fields of an action whose context names a top-level common type from inside a namespace](dogwood-derived-event-bare-type-ref-ignores-empt/) | open | [link](https://github.com/dogwood-policy/dogwood/issues/23) |
| [`ParsedPolicy::expanded_source` renders `-(-3)` as `--3`, which re-parses to the literal `3`](dogwood-expanded-source-renders-negated-literal-/) | open | [link](https://github.com/dogwood-policy/dogwood/issues/24) |
| [`ParsedPolicy::expanded_source` renders `!(-3)` as `!-3`, which Dogwood's parser rejects](dogwood-expanded-source-renders-not-over-negativ/) | open | [link](https://github.com/dogwood-policy/dogwood/issues/25) |
| [`dogwood explain` prints `context has input.user` and `context has "input.user"` as the same text, but the two conditions decide differently](dogwood-informalize-has-path-vs-dotted-attr-same/) | open | [link](https://github.com/dogwood-policy/dogwood/issues/26) |
| [`dogwood explain` does not parenthesize an `if` that is an operand of `&&`/`\|\|`, so two policies with different verdicts get identical prose](dogwood-informalize-if-operand-of-and-or-unfence/) | open | [link](https://github.com/dogwood-policy/dogwood/issues/27) |
| [Parser accepts Cedar reserved words (is, then, else, in, like, has, true, false) as record keys and attribute names](dogwood-parser-accepts-cedar-reserved-words-as-a/) | declined | [link](https://github.com/dogwood-policy/dogwood/issues/28) |
| [Nested positive-left `since` under the default pinned event schema takes time polynomial in the number of in-window events, with the degree growing with nesting depth](dogwood-pin-relativized-nested-since-reevaluatio/) | open | [link](https://github.com/dogwood-policy/dogwood/issues/29) |
| [Nested `previous` under the default pinned event schema takes time polynomial in the number of in-window events, with the degree growing with nesting depth](dogwood-pin-relativized-previous-superpolynomial/) | open | [link](https://github.com/dogwood-policy/dogwood/issues/30) |
| [Provider record field names that are not Cedar identifiers ("a b", "x-y", "if") are written unquoted into a synthesized schema, so lowering fails](dogwood-provider-record-field-names-not-quoted-i/) | open | [link](https://github.com/dogwood-policy/dogwood/issues/31) |
| [A `def temporal` macro whose whole body is `tp(?t)` is rejected as "macro-in-macro is not supported"](dogwood-temporal-macro-body-tp-only-parsed-as-ma/) | open | [link](https://github.com/dogwood-policy/dogwood/issues/32) |
| [A `decimal("…")` literal passed as an argument to a `def temporal` macro is rejected as "unknown macro `decimal`"](dogwood-temporal-macro-call-decimal-arg-parsed-a/) | open | [link](https://github.com/dogwood-policy/dogwood/issues/33) |
