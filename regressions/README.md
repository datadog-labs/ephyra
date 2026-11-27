# Regressions: fixed bugs, re-injected

Every bug fixed in Ephyra comes with a property that catches its **class**, not only the
case that was reported. Each bug also gets an entry here that proves the property catches
it: the fix, reversed, as a patch. The layout follows
[etna-workloads](https://github.com/datadog-labs/etna-workloads), adapted to a code base
that changes every day instead of a pinned release. The method is described in
[CONTRIBUTING.md](../CONTRIBUTING.md#bugs-a-property-for-the-class-and-a-regression-entry).

```
python3 regressions/run.py               # fix soundness, then bug soundness, for every entry
python3 regressions/run.py ID ...        # only some entries
python3 regressions/run.py --matrix      # every oracle against every injection
python3 regressions/run.py --check       # only check that every injection applies and reverses
```

The runner copies the library to a scratch directory, so your working tree is never
touched and uncommitted changes are tested as they are. It builds in
`target/regressions`. CI runs it on every pull request, and nightly with `--matrix`
([`.github/workflows/regressions.yml`](../.github/workflows/regressions.yml)).

## What it checks

- **The injection is exact.** `injection.patch` applies to the current tree and reverses
  back to it byte for byte. It touches implementation files only.
- **Fix soundness.** On the clean tree every oracle and regression test passes.
- **Bug soundness.** With the injection applied, every oracle fails under its fixed seed.
  An oracle that passes on its injection is too weak for its class.

## The entries

| entry | bug | class | reported by | fixed by |
|---|---|---|---|---|
| [`f32-rounds-up-to-one`](f32-rounds-up-to-one) | #40 | generator-domain | Doğaç Eldenk | Yusuf Eren Bozkurt (#137) |
| [`f64-range-width-overflow`](f64-range-width-overflow) | #41 | generator-spread | Doğaç Eldenk | Bruno Volpato (#69) |
| [`dependent-shrink-ignores-discard`](dependent-shrink-ignores-discard) | #44 | shrink-support | Doğaç Eldenk | |
| [`f32-bits-infinity-shrinks-to-itself`](f32-bits-infinity-shrinks-to-itself) | #47 | shrink-progress | Doğaç Eldenk | |
| [`any-float-shrink-cycle`](any-float-shrink-cycle) | | shrink-termination | ephyra-agent | ephyra-agent |
| [`names-sharing-a-file-replay-each-other`](names-sharing-a-file-replay-each-other) | #48 | replay-isolation | Doğaç Eldenk | Yusuf Eren Bozkurt (#136) |
| [`hash-collection-replays-stale`](hash-collection-replays-stale) | #46 | replay-fidelity | Doğaç Eldenk | Bruno Volpato (#73) |
| [`save-glues-onto-a-line-without-newline`](save-glues-onto-a-line-without-newline) | #59 | replay-durability | Doğaç Eldenk | |
| [`a-bad-line-drops-the-file`](a-bad-line-drops-the-file) | #59 | replay-durability | Doğaç Eldenk | |
| [`result-ok-value-ignored`](result-ok-value-ignored) | | verdict-return-value | (described in #63) | Bruno Volpato (#63) |
| [`run-of-discards-passes`](run-of-discards-passes) | | verdict-evidence | (described in #65) | Bruno Volpato (#65) |
| [`panic-after-discard-aborts`](panic-after-discard-aborts) | | verdict-discard | (described in #76) | Bruno Volpato (#76) |
| [`failures-n-never-ends`](failures-n-never-ends) | #42 | verdict-termination | Doğaç Eldenk | Bruno Volpato (#66) |
| [`targeted-phase-ignores-discard-limit`](targeted-phase-ignores-discard-limit) | #102 | verdict-evidence | David R. MacIver | |
| [`a-choice-runs-other-alternatives-hooks`](a-choice-runs-other-alternatives-hooks) | | shrink-support | (described in #70) | |
| [`handled-panic-becomes-the-message`](handled-panic-becomes-the-message) | #58 | report-message | Doğaç Eldenk | |
| [`resumed-payload-message-lost`](resumed-payload-message-lost) | #101 | report-message | David R. MacIver | |
| [`two-panics-on-a-line-are-one-failure`](two-panics-on-a-line-are-one-failure) | #100 | report-failure-kept | David R. MacIver | |
| [`replay-of-a-generator-panic-passes`](replay-of-a-generator-panic-passes) | #97 | report-replay | David R. MacIver | |
| [`replay-reports-a-random-seed`](replay-reports-a-random-seed) | #98 | report-replay | David R. MacIver | |
| [`checker-counterexample-outside-support`](checker-counterexample-outside-support) | #113 | report-counterexample | David R. MacIver | |
| [`shrink-panic-escapes-check`](shrink-panic-escapes-check) | | report-survives | (described in #72) | Bruno Volpato (#72) |

Four properties catch them all:

- [`every_builtin_generator_keeps_its_contract`](../crates/ephyra/tests/gen_contract.rs) catches
  the generator bugs (the first five rows, and `a-choice-runs-other-alternatives-hooks`).
- [`a_saved_failure_is_replayed_for_its_own_property_only`](../crates/ephyra/tests/replay_contract.rs)
  catches the replay bugs (the next four).
- [`a_runs_verdict_matches_what_its_inputs_did`](../crates/ephyra/tests/verdict_contract.rs)
  catches the verdict bugs (the next five).
- [`what_a_failing_report_says_is_true`](../crates/ephyra/tests/report_contract.rs) catches the
  report bugs (the last seven).

## An entry

`regressions/<id>/` holds:

- `injection.patch`: the fix, reversed. Applying it puts the bug back. It is a unified diff
  from the repository root and touches implementation code only, not the regression test.
- `meta.json`, with these fields:

| field | meaning |
|---|---|
| `id` | the directory's name: what went wrong, in a few words |
| `title` | one line, as an issue title |
| `class` | the contract the bug breaks, shared by its siblings: `generator-domain`, `generator-spread`, `shrink-support`, `shrink-progress`, `shrink-termination`, `replay-isolation`, `replay-fidelity`, `replay-durability`, `verdict-return-value`, `verdict-evidence`, `verdict-discard`, `verdict-termination`, `report-message`, `report-failure-kept`, `report-replay`, `report-counterexample`, `report-survives`, ... |
| `symptom` | `wrong-result`, `soundness-false-pass`, `false-failure`, `crash`, `non-termination`, `data-loss`, ... |
| `issue` | the GitHub issue, if there is one |
| `cause` | the root cause, in a sentence |
| `reported_by`, `fixed_by` | credit |
| `oracles` | the class properties that must fail with the injection: `{"target": "test:gen_contract", "name": "..."}` (`target` is `lib` or `test:<integration test>`; `package` defaults to `ephyra`; `features` is optional) |
| `regression` | the narrow test pinning the reported case; it must pass on the clean tree |

## Adding a bug

1. Find the property for the bug's class, or write it, against the buggy code. See it fail.
2. Fix the bug. The property passes.
3. Write the narrow regression test for the reported case.
4. Make the entry. `injection.patch` is the fix reversed, restricted to implementation
   code. For a fix commit `F`: `git diff F F^ -- crates/ephyra/src`, without the hunks that
   add tests. Then write `meta.json`.
5. Run `python3 regressions/run.py <id> --matrix`.

## When a patch stops applying

The library changes and the patches don't. `run.py --check` lists the ones that no longer
apply. Re-derive such an entry on the current tree: write the change that puts the bug
back, by hand if the code moved. Then check it again. Don't delete an entry because its
patch went stale, because the bug it records can come back.
