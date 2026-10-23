# ephyra-sqleq

Equivalent PostgreSQL queries from an e-graph, checked with
[sqleq](https://github.com/datadog-labs/sqleq). Seeds (a schema and a query)
come from [ephyra-postgresql](../../postgresql/generator); variants come from
[egg](https://github.com/egraphs-good/egg) over its typed query language;
each pair goes through sqleq's frontend, the
[QED](https://github.com/qed-solver/prover) prover, `sqleq-solver` and
`sqleq-fuzz`, and PostgreSQL itself judges on the case's data. The SQLite
counterpart of the sampling is [egg-demo](../../egg/demo).

## Setup

- **sqleq**: `tools::bin` runs `sqleq-frontend`, `sqleq-fuzz` and
  `sqleq-solver` from `$SQLEQ_BIN` (a directory, e.g. a sqleq checkout's
  `target/release`), else from PATH. `sqleq-solver` compiles Z3 from source
  (cmake).

  ```bash
  git clone https://github.com/datadog-labs/sqleq && (cd sqleq && cargo build --release)
  export SQLEQ_BIN=$PWD/sqleq/target/release
  ```

- **QED**: `qed-prover` from `$QED_PROVER` (the binary), else from PATH;
  built from [qed-solver/prover](https://github.com/qed-solver/prover) (with
  Nix here; findings were checked on main 9e9c262). The prover takes a
  directory and has no options.
- **PostgreSQL**: ephyra-postgresql's cluster (`../ephyra-postgresql/.pg`,
  its README's setup), via `pgcheck`.
- Runs write under `out/` (gitignored).

## What exists

- [lang.rs](src/lang.rs): the language, a query as one term (a child per
  expression slot, subqueries' slots too), leaves as payload symbols
  carrying their type, the `Types` analysis, expression rules sound in
  three-valued logic. GROUP BY keys are shared opaque leaves.
- [variants.rs](src/variants.rs): e-graph saturation and per-slot uniform
  sampling ([sample.rs](src/sample.rs), generic port of egg-demo's);
  [onestep.rs](src/onestep.rs): one rule at one place (egg's own search,
  single substitution) plus [relational.rs](src/relational.rs) (ON/WHERE
  moves, join commutation, LEFT to inner, DISTINCT/GROUP BY, IN to EXISTS),
  all one-step variants named by rule.
- [fragment.rs](src/fragment.rs): seeds within sqleq's fragment.
  [mutate.rs](src/mutate.rs): one-operator mutants.
  [pgcheck.rs](src/pgcheck.rs): PostgreSQL as judge.
  [check.rs](src/check.rs): the batch checker and finding classes
  (claims-not-equivalent, proves-refuted, unproved, qed-panic, and
  `known-qed-13-*` / `known-qed-14-panic` for causes already filed).
  [minimize.rs](src/minimize.rs): slot reversion, seed shrinking keeping an
  unproved (or any predicate) one-step variant by the same rule.
- Examples: `run` (end to end, `--onestep`), `stats`, `onestep`, `pgcheck`,
  `relcheck`, `selfcheck`, `minimize`, `minimize_rules`, `minimize_panics`,
  `probe`.

## Findings (2026-10-14)

- QED: [qed-solver/prover#13](https://github.com/qed-solver/prover/issues/13)
  (real constants read as f32: unsound proofs and panics at
  normal.rs:655/658/659) and
  [qed-solver/prover#14](https://github.com/qed-solver/prover/issues/14)
  (INTEGER vs DECIMAL in IN/UNION: panics at normal.rs:440, partial.rs:185).
  Repros: [bugs/qed](../bugs).
- sqleq: [datadog-labs/sqleq#177](https://github.com/datadog-labs/sqleq/issues/177)
  (set-operation/VALUES typing: opaque functions typed VARBINARY, no casts),
  [#186](https://github.com/datadog-labs/sqleq/issues/186) (`NOT` over
  demoted booleans) and
  [#187](https://github.com/datadog-labs/sqleq/issues/187) (sqleq-fuzz
  literal domain). Repros: [bugs/sqleq](../bugs).
- No equivalent pair refuted by sqleq-fuzz, no PostgreSQL-refuted mutant
  proved, outside #13.
- Expression rules: 8778 variants agree on PostgreSQL; relational rules:
  2135 agree (1500 inputs).
- Prover incompleteness: about 18% of one-step pairs unproved (many through
  the panics above).
