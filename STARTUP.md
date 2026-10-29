# Testing a project with ephyra

This page is for a coding agent asked to test a project with ephyra. The
goal is real bugs, each with a small input that shows it, plus tests that
keep them fixed. This page is about how to work. For the API, read
[the library's README](crates/ephyra/README.md), then [API.md](API.md) (every
generator, combinator, setting and loop step, and what does not exist);
`ephyra guide` prints the guide to writing generators.

## 1. Set up

```toml
[dev-dependencies]
ephyra = "0.2"
```

`cargo install ephyra-cli` installs the `ephyra` command. Put property tests where the project keeps
its tests, or in a separate crate if the user prefers; don't change the
project's own manifests or lock files without asking.

Build tests with `debug-assertions` and `overflow-checks` on, also in
release (`[profile.release]` in the test crate): many bugs show up first as
a failed `debug_assert!` or an overflow.

For a system that isn't Rust (a database, a compiler, a CLI), write a small
Rust test crate that drives it through its API, its command line or a
connection.

## 2. Decide what correct means before you generate anything

Read the docs, the types, the existing tests and fixtures. Expected
behaviour must come from the project: a sentence in its docs, a type, a
test, a standard it says it follows, or a universal rule (no crash, no
hang, no data loss, no wrong answer to a well-defined question). Write
down, for the 3 to 5 places where a bug would hurt most, the property you
will check and where it comes from. Show the user this list; if no one is
there to see it, write it to a notes file and go on.

Oracles that find bugs, roughly in order of effort:

| Oracle | Example |
|---|---|
| Crash | the code panics, aborts, hangs or trips an assertion on valid input |
| Round trip | `parse(print(x)) == x`, `decode(encode(x)) == x` |
| Reference model | a simple, obviously correct implementation, an older version, or another implementation of the same spec |
| Invariant | sortedness, balance, no lost or duplicated elements, totals conserved, after every operation |
| Metamorphic | two queries that must return the same rows (with and without an index, an optimizer flag, a rewrite) |

## 3. Generators

- **Valid by construction.** Build inputs that are valid, from the
  grammar or the types, instead of generating anything and filtering. If
  most inputs are discarded, the filter is too strict.
- **Start from the project's own inputs.** Its tests, fixtures and examples
  are good seeds; mix them with generated values.
- **Sequences for state.** For anything stateful, generate sequences of
  operations, not single calls, and check the state after each step.
- **Mark and verify.** Give each generator a support predicate and mark
  both with `#[ephyra::generator]`; give structured values a shrinker
  (`with_shrink`, `#[ephyra::shrinker]`, or `#[derive(Arbitrary)]`). Run
  `ephyra check` and `check_shrinker` before trusting them: a generator
  that produces invalid inputs or barely varies finds nothing.

## 4. Run

Run properties through ephyra's runner: `#[ephyra::test]`, `check`, or a
`Run` loop when you need your own search (API.md, "Your own loop"). The
runner gives you shrinking, exact replay (`EPHYRA_REPLAY`), remembered
regressions and reports; a hand-written loop with its own minimizer loses
all of them.

To find many bugs in one long run instead of stopping at the first, set
`Config::failures(n)` (or `EPHYRA_FAILURES=n`): the run goes on until it has
`n` distinct failures, shrinks each one and reports each with its own replay
line. Failures are told apart by where the property panicked; name them
yourself with `ephyra::signature("display not reparsable: SHOW")` inside the
property, or with `Config::signature`, when one assertion covers many causes.

Short runs find shallow bugs. Once the properties pass a quick run, run
them longer and in the background, with several seeds (`EPHYRA_SEED`) and a
time budget (`EPHYRA_BUDGET_MS`). With the `coverage` feature
(`RUSTFLAGS="-C instrument-coverage"`, one test thread), reports say which
functions and lines were never reached: that is where to aim next. From a
separate test crate, set `EPHYRA_COVERAGE_FILTER=<crate under test>` (the
default tracks the test crate itself), and check the report's `coverage:`
line names that crate; `.ephyra/coverage/<property>.html` shows files and
lines.

## 5. When a property fails

1. **Replay** it with the `EPHYRA_REPLAY` value the report prints, and put
   the shrunk counterexample in a plain test that fails on the unmodified
   code.
2. **Triage** it:

   | Verdict | Signs | Action |
   |---|---|---|
   | Real bug | The input is valid by the docs, and the result contradicts a sentence you can quote | Keep the test; report it |
   | Test bug | The model, the generator or the oracle is wrong | Fix the test and rerun |
   | Intended | Docs, a `# Panics` section or an existing test say so | Exclude it narrowly, with a comment |
   | Environment | Version, platform, locale, time, a missing tool | Note it; don't report |

   A new oracle that fails on more than about 1% of inputs is usually the
   oracle's fault: reread the docs before the code.
3. **Check it is new and current.** Compare with earlier findings (the
   failure site, the shrunk input) and with the project's open and closed
   issues (GitHub's search API works without a token, rate-limited:
   `https://api.github.com/search/issues?q=repo:OWNER/REPO+<words>`).
   Reproduce it on the latest version of the project before reporting it.

## 6. When nothing fails

A clean run is a reason to change the test, not to stop.

- **Evolve the generator** against what it never produces: shapes,
  options, enum variants and API calls it misses, and functions coverage
  never reached. Fill one gap at a time and measure before and after.
- **Add an oracle on the same code.** In our case studies a new oracle
  often failed on its first run; an old oracle given more time rarely did.
- **Check what the oracles skip.** Count the inputs each oracle discards or
  excludes. An exclusion added for one known bug can hide most inputs.
- **Pivot.** After two rounds with no failure and flat coverage, move to
  another part of the project.

## 7. Rules

- Never edit or delete an existing test, and never weaken a property to
  make it pass.
- Don't change the code under test to get a pass. If you are asked to fix a
  bug, the fix needs a failing test first: no reproducer, no fix.
- Don't file anything upstream yourself unless the user asks. When they
  do, follow the project's CONTRIBUTING, issue templates and any AI policy
  (some projects want no AI-written text in issues), and send anything
  that may be a security problem through the project's private channel.
- A report has the version tested, the minimal input, the commands, the
  expected and the actual output, and the sentence that says what is
  expected.

## 8. Report back

Give the user: the properties written and what each checks, how many cases
each ran, what failed and your verdict, the reproducing tests, and what you
would test next.

The [case studies](case-studies) show this on real systems: generators,
oracles, harnesses and the bugs they found.
