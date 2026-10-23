# Bugs: sqleq

The bugs the case studies found in sqleq. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [Unsound: real constants are parsed as `f32`, so distinct literals are proved equal (and wide ones panic)](qed-real-constants/) | open | [link](https://github.com/qed-solver/prover/issues/13) |
| [Panic on INTEGER vs DECIMAL in `IN (subquery)` (normal.rs:440) and `UNION` (partial.rs:185)](qed-sort-panics/) | open | [link](https://github.com/qed-solver/prover/issues/14) |
| [sqleq-fuzz: a literal outside the generated value domain hides a boundary difference (`334 <= c` vs `334 < c` is NO-COUNTEREXAMPLE)](sqleq-fuzz-literal-domain/) | fixed | [link](https://github.com/datadog-labs/sqleq/issues/187) |
| [sqleq-fuzz: a literal compared with an expression of a column, or behind a value-changing cast, still hides a boundary difference](sqleq-fuzz-missed-20261021/) | open | [link](https://github.com/datadog-labs/sqleq/issues/207) |
| [frontend: a set operation or VALUES list whose branches have different types reaches the prover ill-typed when the first branch is REAL, BOOLEAN, DATE or opaque (QED panics)](sqleq-ill-typed-in-setop/) | fixed | [link](https://github.com/datadog-labs/sqleq/issues/177) |
| [frontend: `NOT` over a demoted boolean operator (`<@`, `@>`, `&&`, `~`, `!~`) outside a WHERE reaches the prover applied to a VARBINARY term (QED panics)](sqleq-not-demoted-boolean/) | fixed | [link](https://github.com/datadog-labs/sqleq/issues/186) |
