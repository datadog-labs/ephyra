# brush harness (brushfuzz)

`build.sh <src> <build>` builds brush from `<src>` as an optimised build that keeps debug assertions,
overflow checks and `panic=unwind`, so panics show up as messages. It also builds `fuzzer/` (the `brushfuzz` binary,
which uses `ephyra::Source` for seeded generation) and copies the reference bash 5.3.20
(`work/ref/bash-5.3/bin/bash`) to `<build>/bash`. `run.sh <build> <seed> <secs> <out>` runs
`BRUSHFUZZ_THREADS` workers (default 3).

## What a case is
Each case is a generated bash script. It is a tree of statements, so the shrinker can delete subtrees or
unwrap them. Every script starts with a fixed prelude: `shopt -s extglob`, plus 6 files in a fresh directory.
Next comes a variable initialisation node that the shrinker never deletes: 3 scalars, 2 non-empty arrays,
`declare -i n`, an assoc `A`, and sometimes `set --`. Then come 3–12 statements: assignments, `printf '<%s>'`
observers, parameter expansions (`# ## % %% / // /# /% ^ ^^ , ,, :off:len :- := + @U/L/E/a`),
arrays and slices, arithmetic (bases, shifts, ternary, assignment ops), brace expansion, globs,
`if`/`for`/`for ((;;))`/`case` (`;;`, `;&`, `;;&`), functions with `local`/`return`, subshells,
`$( )`, pipelines, here-docs, here-strings, `read`, `mapfile`, redirections to files, `set --` and
`shift`. The script ends by printing every variable.
The script runs as a file argument (stdin is /dev/null), with the environment cleared except
`LC_ALL=C PATH=/usr/bin:/bin HOME=/nonexistent-home`. brush and bash each get their own fresh
directory, and each has a 5 s timeout.

## Oracles / signatures
- `crash__panic`: brush stderr contains `panicked at`, or brush exits 101, or brush is killed by a signal.
- `hang__timeout`: brush runs past 5 s while bash finishes.
- `diff__stdout`, `diff__status`, `diff__files`, `diff__stderr`: differential against bash 5.3. They
  compare stdout bytes, the exit code, the files left in the directory (names and contents), and whether
  stderr is empty, never its text. brush targets bash 5.3+ (docs/reference/compatibility.md).
  The checks run in that order, and the first one that differs decides the signature.
  A `diff__*` signature gets a third part, the feature area of the shrunk script
  (`diff__stdout__param_patsub`, `diff__files__redirection`, ...): `area()` in main.rs names the most
  specific construct among the statements left after shrinking (node 0 and the final printf excluded).
  The shrinker and `repro` compare only `oracle__kind`; the area is a property of the input, not the build.
  At most `12*cap` shrinks per `oracle__kind` per run; later failures are only counted.

Skips (counted in stats.json as `skip_*`): brush exit 99 or a "not yet implemented" message, which is
brush's documented "unimplemented" signal (`skip_unimplemented`). Also skipped: a bash timeout or a bash signal.

## Round-trip statements (steering round 1)
About 7% of simple statements are round-trips. Each one checks a printer whose output bash.1 documents as reusable input,
and the output is still diffed against bash:
- `roundtrip_func` (area `roundtrip_func`): `eval "$(declare -f f)"` at top level or in a subshell followed by a call, the
  idempotence check `[[ "$(declare -f f)" == "$(eval "$(declare -f f)"; declare -f f)" ]]`, and `declare -F`. Function
  definitions now also use `function f {`, `function f() {`, `f() ( ... )` and (rarely) `f() { ...; } 2>/dev/null`.
- `roundtrip_quote` (area `roundtrip_quote`): `${v@Q}`, `printf %q` / `'%q '`, `declare -p`, `${v@A}` / `${a[@]@A}`,
  `${a[@]@Q}` and `${@@Q}` are each re-`eval`ed and the value printed. Before the round-trip a scalar or `a[0]` may be set
  to a quoting-hostile value (`#a`, `~`, `` `x` ``, `$'\x01'`, `$'\x7f'`, `a&b|c`, ...). Non-ASCII and `\xff` are left
  out: case conversion under LC_ALL=C and lossy UTF-8 (#1115) differ for other reasons.
Bugs the oracle found at once on the base build: `declare -f` indents here-doc bodies and terminators, and the continuation
lines of multi-line quoted literals and `$( )` bodies (re-indented on every print). It also prints a compound command's redirection without a space (`}2> /dev/null`,
`]]2>`). Separately, `read` drops `\x01`/`\x7f` bytes, and `patsub_replacement` (`&` in `${v/p/r}`) is ignored.
To keep the failure rate under 1%, a function whose body hits one of the three printer triggers is "fragile"
(`gen.rs` `fragile`, `compound_redir`). Its round-trip is generated only 10% of the time.

## Differences avoided in the generator, not masked
No output normalisation is applied. Instead, the generator avoids constructs that hit known or frequent
differences. Each one is listed with its evidence in `work/agents/harness-004/notes.md`:
- Non-whitespace or null IFS (documented known IFS edge cases).
- Error-producing arithmetic: brush aborts the script on a `(( ))` error.
- The shortest match with `*` in `${v#*}` / `${v%*}`.
- `*` or empty patterns in `${v/p/r}`.
- `case` inside `$( )`, `break n` with n > depth, `for x in ;`, `set -u`, `${v:?}`.
- `[@]` parts in multi-part quoted words.
- extglob patterns that can match the empty string.
- `@Q`/`%q` (two valid quotings), `@u`, nocasematch, negative substring lengths, side effects in
  here-docs and nested words, and braces in `declare` arguments.
- Shell constructs that bash 5.3 itself mis-parses inside `$( )`.
Many of these are real brush bugs. They are listed as leads in the notes, so later steering rounds can
re-enable them one at a time.
`shift` past `$#`: only `$? > 0` is printed, because bash.1 says only "greater than zero".

## Throughput and base failure rate
Each case is 2 process spawns. A 4×240 s gate run on 16 CPUs (4 run.sh, 3 threads each) gave ~70 cases/s
per run.sh and a 0.24–0.31% failure rate. Most failures were the `n+=` overflow panic
(variables.rs:276), plus rare real stdout/stderr differences.
A known flake source: a group redirected to a file whose inner pipeline also `cat`s that file is a race.

## Findings / repro
`findings/<sig>/<n>/` contains `script.sh` (shrunk), `expected.json` (the bash outcome),
`brush-observed.json` and `finding.json`. `repro.sh <build>` runs `<build>/brushfuzz repro`. That re-runs
brush on the script up to 3 times and compares the result with the stored bash outcome; bash is not
needed. Exit 1 means the same signature reproduced, 0 means it did not, and 2 means it cannot decide.

Tools: `brushfuzz gen <seed> [n]` prints scripts. `brushfuzz one <build> <file>` runs both shells and
prints the verdict. `BRUSHFUZZ_CAP=N` raises the per-signature cap, for triage only.

## Mutants
In `mutants/`: `case-fallthrough-dropped` (`;&` treated like `;;`), `substring-length-cap-off-by-one`,
`arith-ge-strict` (`>=` evaluated as `>`), `arith-modulo-sign` (`%` result made non-negative).

## Coverage build
`build.sh <src> <build> coverage` builds brush with `-C instrument-coverage` in `<build>/target-brush-cov-<hash of src path>`.
The target dirs are keyed by source path. Otherwise cargo treats a different tree with older mtimes as fresh and keeps a binary whose coverage map points at the old tree; that failed the first coverage gate.
Build-script profiles go to `<build>/build-profraw`. The fuzzer and bash are not instrumented. It also writes
`<build>/coverage.json`, which lists `<build>/brush` and the toolchain's own `llvm-profdata`/`llvm-cov`
(from `rustc --print sysroot`). brushfuzz clears the environment for the shells, but it passes
`LLVM_PROFILE_FILE` through, made absolute because each shell runs in a scratch directory.
A `%p` in the pattern becomes `<pid>-<worker thread>`, and `%m` is added if missing. The LLVM runtime then
merges all runs of one worker into one file. With one file per process, a 30 s run wrote 10 GB; this way it
writes about 10 MB, with the same coverage.
Checked: 30 s at ~85 cases/s, a 0.16% failure rate, and the merged report covers
brush-core expansion.rs at 78%, arithmetic.rs at 79% and interp.rs at 60% of lines.
