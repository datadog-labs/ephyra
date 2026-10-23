# coreutils harness: uudiff

Differential search, uutils coreutils (dev profile, opt-level 1: debug assertions,
overflow checks, panic=unwind) against GNU coreutils 9.12 (`work/ref/gnu-9.12/bin`),
the version uutils targets (`util/fetch-gnu.sh`). The contract: README.md:42-43
"Differences with GNU are treated as bugs", README.md:46 "Matching GNU's output
(stdout and error code) exactly", CONTRIBUTING.md:70 "should never unexpectedly fail".

- `build.sh <src> <build>`: cargo build of coreutils into `<build>/debug/coreutils`,
  `<build>/bin/<util>` symlinks (argv0 dispatch, so both sides print `<util>:`), and the
  search driver `driver/` (Rust, ephyra) into `<build>/uudiff`.
- `build.sh <src> <build> coverage`: same, with `-C instrument-coverage` on the target only
  (build-script profiles go to `<build>/build-profraw`), and `<build>/coverage.json`
  naming `debug/coreutils` and the toolchain's llvm-profdata/llvm-cov.
- `run.sh <build> <seed> <secs> <out>`: one `uudiff` process. `UUDIFF_UTILS=sort,cut`
  restricts the utilities. `UUDIFF_GNU` overrides the reference bin dir.
- `driver/src/gen.rs`: generators. `driver/src/known.rs`: masks. `driver/src/repro.sh`:
  template for each finding's `repro.sh`.

## Case

One command line: a utility, argv built from that utility's GNU-documented options
(full long names, each option at most once), stdin, and files `f1`, `f2` (sometimes `d1/`)
in a fresh working directory. 35 utilities: sort uniq cut tr head tail wc nl fold expand
unexpand paste comm join tac od numfmt seq printf expr test basename dirname base64 base32
basenc cksum(+md5sum sha1sum sha256sum sha512sum b2sum sum) factor split csplit truncate
cat echo fmt dd. Text comes from small pools (fields, delimiters, duplicated lines) so keys
collide; ASCII plus rare control bytes (non-ASCII bytes in the C locale are known: #9712, #7600).
Both sides run at once with `env -i PATH=<bindir> LC_ALL=C TZ=UTC0`, stdin from a regular
file, RLIMIT_FSIZE 16 MB, 5 s timeout (process group killed).

## Oracles (signature `<oracle>__<util>_<aspect>`)

1. `crash`: `panic` (uutils stderr has `panicked at`, or exit 101), `signal` (uutils killed
   by a signal GNU was not), `timeout` (uutils hangs, GNU finished).
2. `diff`, in this order: `exit` (exit codes differ), `stdout` (bytes differ), `files`
   (working tree after the run: path, type, mode, content/link target), `stderr`
   (empty vs non-empty only; error text is not compared, clap's messages differ by design).
   Cases where GNU times out or dies by a signal are skipped (counted in `skipped`).

Each aspect panics at its own line in the property, so ephyra's shrinker (drop an
argument group, drop lines/bytes of stdin and files) keeps the failure kind. A finding is
written only after its `repro.sh` re-records GNU's behaviour (`repro.sh --record <gnu-bin>`
writes `expected/`) and then exits 1 against the build; otherwise it is counted in
`repro_unconfirmed`. `repro.sh` needs only its own directory and the build dir: `case/args`
(NUL-separated), `case/stdin`, `case/files/`, `expected/`. `observed/` keeps both sides'
outputs for triage.

## Known differences masked (`known.rs`, counted in `masked_known`)

Each fires only when the outputs are equal after removing exactly that difference:

| mask | issue |
|---|---|
| wc_column_width | #15254 `wc -l -c`: column width for regular-file stdin |
| cksum_dash_name | #15254 `cksum -`: name `-` not printed |
| basenc_decode_error_prefix | #15254 `basenc --base64 -d`: GNU writes the decoded prefix before "invalid input" |
| join_unpaired_trailing_space | #15254 `join -a 1`: trailing separator |
| fmt_goal | #15254 `fmt -g 1` |
| sort_incompatible_opts | #15254 `sort --month-sort -k 2 --hu`: GNU rejects incompatible orderings |
| unexpand_first_only_dash | #15254 `unexpand --first-only -` |
| od_strings_with_type | #15254 `od --strings=2 -o` |
| fold_bytes_and_characters | #15254 `fold --bytes --ch` |

Generator-side avoidance (not masks): repeated/overriding options (bulk class in #15254),
tab-separated numfmt input, tabs in fmt input (mostly), `f1 -` for expand/unexpand (bug
found here: stdin after a file is dropped), rare weights for `expand -t N,+M`/`N,/M`,
`nl -s` of length != 1, `split -l 1k`, `split -u`, `dd seek=5B`, `expr +1`. Utilities with
many open differences (fmt, comm, join, tr, expand, unexpand) have weight 1.
Re-enable these in later rounds once the classes are triaged.

## Steering round 1 (2026-10-21): utilities the search never entered

18 more utilities, at low weights (31 of ~230): date (`-d`/`-f` strings built from
absolute dates, times, zones, `TZ="..."` and relative items; `+FORMAT` with flags and
widths; `-I`/`-R`/`--rfc-3339`), pr, ptx, tsort, env (`-i`, `-S` strings with quotes,
escapes and `${VAR}`), and file-tree utilities chmod mkdir rmdir rm ln cp mv install stat
ls du realpath readlink over a tree with nested dirs and symlinks (`name@` in
`Case::files` is a symlink whose data is the target; `-i` answers come from stdin).
Things that differ by construction are pinned (`Case::keep`: the shrinker never drops the
first `keep` groups): `date -d` (else "now"), `pr -t`/`--date-format` (header dates),
`ls --time-style=+T`, `du --apparent-size|-b|--inodes`, `stat --printf`,
`realpath --relative-to` (absolute paths), `env -i` (the inherited PATH), `ptx -G`.
Signatures get an area per option family (date file/rfc/epoch/relative/zone/format,
pr merge/columns/pages/page_size/numbering_tabs/plain, ls long/quoting/recursive/layout, ...).
Made rare because one class fires on nearly every case: pr multi-column/-m (tab padding),
pr -W, -v, -i; ptx untrimmed/multi-blank lines (GNU keeps blanks in contexts, uutils marks
truncation), GNU-mode ptx with more than one line (#9335); ls grids (-C/-x); `-B K`
style unit-only block sizes; POSIX TZ strings; signed relative items right after a time
(GNU reads a zone); env options after NAME=VALUE and `-u ''`.

## Coverage runs

The driver clears the target's environment but passes `LLVM_PROFILE_FILE` through.
A `%p` in it is replaced by LLVM online merging (`%m`, or `%4m` if the pattern has no `%m`)
in the same directory: one instrumented process writes ~5 MB, so `%p` made 17 GB in 30 s;
with merging a 30 s run leaves one 5 MB file with the same counts. ~90 cases/s on the
coverage build.

## Numbers (base build, 2026-10-16)

- ~128-136 cases/s per process (4 processes in parallel on 16 CPUs, no slowdown).
- 4 x 240 s: ~30.6k cases each, failure rate 0.55%, 0 unconfirmed repros, 0 flaky.
- Raw rate before masks and generator fixes was ~7%.

## Mutants (`mutants/*.diff`)

- `uniq-skip-fields-tab`: `uniq -f` skips only spaces, not tabs, before a field.
- `head-lines-drops-terminator`: `head -n K` drops the newline of the K-th line.
- `ranges-complement-single-gap`: `--complement` loses one-element gaps (cut).
- `sort-numeric-sign-order`: `sort -n` orders negative before positive the wrong way round.

Own check (1 x 240 s each, 120 s for sort): head 3, cut 2, uniq 6, sort 4 findings that
reproduce on the mutant and not on base. A first sort mutant (`-k1f` ignoring `f`) was
not caught in 240 s and was replaced; key modifiers are a coverage gap.
