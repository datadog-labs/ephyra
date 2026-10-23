# gix harness (gixfuzz)

`build.sh <src> <build>` builds `<build>/bin/gixfuzz`, a Rust binary that links gix from `<src>`
(release profile with `debug-assertions` and `overflow-checks` on) and ephyra (generation), and
symlinks the reference git 2.56.0 (`work/ref/git-2.56.0/bin/git`) as `<build>/bin/gitbin/git`.

`build.sh <src> <build> coverage` builds the same binary with `-C instrument-coverage` (explicit
`--target`, so build scripts are not instrumented) and writes `<build>/coverage.json` (the binary,
plus the toolchain's llvm-profdata/llvm-cov). run.sh keeps the environment, so `LLVM_PROFILE_FILE`
reaches gixfuzz and its `check`/`eval` children.

`run.sh <build> <seed> <seconds> <out>` runs `gixfuzz search`, restarting it after a timeout finding
or an abort until the time is up. `stats.json` is rewritten every 5 s.

## Oracles

### 1. `revspec__*`: gix `Repository::rev_parse_single` vs `git rev-parse --verify`

Contract: `gix-revision/src/spec/parse/function.rs:10` "Parse a git revspec", AGENTS.md "Use git
itself as reference implementation".

Each case (one ephyra draw):
1. A random history is rendered as a bash script driving the reference git (commits over a small
   file pool, branches, checkouts (for `@{-n}`), detached HEADs, `-s ours` merges incl. octopus,
   light/annotated tags incl. tag-of-tag/tree/blob, resets, upstream config (local and
   `origin/*`), remote-tracking refs, `push.default`/`remote.pushDefault`, `pack-refs`, `repack`,
   symbolic refs, branches named after short hashes, `update-ref -m`). SHA-1 or SHA-256 (30%).
   Small name pools so names collide (`v1`/`main` are both branch and tag, `origin` is a branch
   and the remote, `abcd` looks like hex). Fixed author/committer identities; dates tick 1 h per
   step. Environment copied from gix-testtools (no system/global config, etc.).
2. 10..60 specs from a grammar: `base [@{n}|@{-n}|@{u}|@{push}|@{date}] (~n|^n|^{type}|^{/re})* [:path]`,
   plus `:/re`, `:path`, `:n:path`; bases are HEAD, `@`, branch/tag names in all spellings
   (`refs/heads/x`, `heads/x`, `tags/x`, `origin/x`, `remotes/origin/x`, `origin`), and hex
   prefixes (4..40 chars) of real objects.
3. gix opens the repo (`open::Options::isolated()`), resolves every spec. git answers via one
   `git cat-file --batch-check` (fast path); every disagreement is re-decided by
   `git rev-parse --verify <spec>`, the answer of record.
4. Signatures: `revspec__{wrong_object|gix_rejects|gix_accepts}_<construct>` where construct is
   the most specific feature of the spec (`msgsearch`, `peel_regex`, `prev_checkout`, `push`,
   `upstream`, `reflog`, `date`, `index_path`, `tree_path`, `peel`, `nav`, `hex`, `ref`).
   Error messages are not compared.

Failing cases are shrunk greedily on the exact signature (spec list → single spec, ops removed,
spec simplified), then written as `case.json` (setup script + specs), with `setup.sh`/`specs.txt`
copies for reading. `repro.sh <build>` runs `gixfuzz check case.json`, which evaluates the case in a
child process (so aborts/hangs are classified too).

### 2. `crash__*`: panics, hangs, aborts in gix

- `crash__panic_<crate>`: a panic inside gix while opening the repo or resolving a spec (crate from
  the panic location). debug-assertions and overflow checks are on.
- `crash__timeout`: one `gix_answers` call over 20 s (watchdog thread; the process exits 3 and
  run.sh restarts it).
- `crash__abort`: the search process died (stack overflow, abort); run.sh turns `current.json`
  (the case being evaluated) into a finding.

### 3. `graph__*`: commit-graph algorithms (merge-base, rev-walk, topo order, describe)

Same repositories (the generator also writes commit-graph files, `git commit-graph write
--reachable [--split|--changed-paths]`, and moves the clock back sometimes so commit times repeat
or go backwards). Each case also carries 0..20 queries, rendered into the spec list as
`#<kind> <words>` with full object ids (any commit in the repo, reachable or not):

| query | gix | git (answer of record) |
|---|---|---|
| `#merge_base A B` | `Repository::merge_base` | one of `git merge-base --all A B` |
| `#merge_bases_many A B..` | `Repository::merge_bases_many` | same set as `git merge-base --all A B..` |
| `#octopus A B..` | `Repository::merge_base_octopus` | one of `git merge-base --octopus --all` |
| `#walk <bfs/newest/oldest> <fp/all> <graph/nograph> T.. ^H..` | `rev_walk(T).with_hidden(H)` | same set as `git rev-list [--first-parent] T.. ^H..` |
| `#topo <topo/date> <fp/all> T.. ^E..` | `gix_traverse::commit::topo` | same order as `git rev-list --topo-order/--date-order` ("This is equivalent to `git rev-list --topo-order`", topo/mod.rs) |
| `#describe <annotated/tags/all> <fp/all> <n> C` | `Commit::describe()` | same name and depth as `git describe [--tags/--all] --candidates=n --long` |

Signatures `graph__{wrong|gix_rejects|gix_accepts}__<area>`, area = `merge_base`,
`merge_bases_many`, `octopus`, `revwalk`, `revwalk_first_parent`, `topo_order`, `date_order`,
`describe_{annotated,tags,all}`. Shrinking also drops query arguments.

`git_revlist_skew_limiting`: with skewed commit dates and no commit-graph, `git rev-list T ^H` stops
painting early and lists ancestors of `H`; on a walk/topo disagreement the exact set
`rev-list T` minus `rev-list H` (no negative revisions; hidden commits hide all their ancestors,
also with `--first-parent`) decides, and an order difference then has no reference. Hidden/end
commits equal to a tip are dropped except in 0.3-0.4% of walk/topo queries (`overlap`), since that bug
(G1) fires on most such queries.

Graph masks: `describe_name_tie` (same depth, both names peel to the same commit, neither an
annotated tag: which equal name wins is undocumented, gix sorts by name); `git_describe_external_name`
(git prints a tag object's internal name, "tag 'main' is externally known as 'v2'"). Known graph bugs
(`known_topo_tip_is_end`, `known_describe_tag_chain`) are masked only after the search process wrote
`KNOWN_CAP` (1) finding for that signature, so every run still reports them; any other
disagreement of a query oracle (graph, treediff, merge) on a signature with a finding, or once the
process has a finding of that oracle family, is counted as `graph_capped`. These oracles have many
frequent real bugs; without the family cap every process reporting each once was above 1% of a
4-minute run's cases. Other processes and windows report the others.

### 4. `treediff__*`: tree-to-tree diff vs `git diff-tree`

Queries `#diff <nore|exact> <A> <B>` (B any commit; A any commit, or `B~1`, B's first parent, half
the time). gix `Repository::diff_tree_to_tree` without rewrites vs `git diff-tree -r -z --no-abbrev
--no-renames`; with exact renames only (`Rewrites { percentage: None, copies: None }`) vs
`-M100%`. Lines `<old mode> <new mode> <old id> <new id> <path>` (renames `R ... src -> dst`),
trees left out, sorted. Exact renames are only compared when unambiguous (a blob id in at most one
deletion and one addition; `diff_ambiguous_renames` otherwise): gix-diff/src/rewrites/tracker.rs:1-7
documents a different candidate choice. Signature area `<mode>_<feature>` with feature the most
specific of the differing lines: `rename`, `gitlink`, `symlink`, `mode`, `add_delete`, `modify`.

### 5. `merge__*`: commit merges vs `git merge-tree --write-tree`

Queries `#merge <ours> <theirs>`: gix `Repository::merge_commits` (options from
`tree_merge_options()`, object memory so nothing is written) vs `git merge-tree --write-tree
--no-messages`. Contract gix-merge/src/tree/function/resolve.rs:68-70 "Git's `merge-ort` is the
behavioral reference"; the upstream baseline compares merged tree ids. Compared: verdict (gix
`has_unresolved_conflicts(TreatAsUnresolved::git())` vs exit 1), clean tree id, and for conflicted
merges the listing `<mode> <id> <path>` of the result tree where blobs with conflict markers count as
`MARKERS` (marker placement is a documented deviation, text/function.rs:116). Areas: `gix_clean`,
`gix_conflict` (verdicts differ), `clean_tree`, `conflict_tree`, `conflict_tree_typechange`
(a symlink/gitlink among the differing entries).

### 6. `status__*` / `index__*`: worktree status and the index vs git

Queries `#status`, `#index`, `#index_rt` (no arguments; `crate/src/wt.rs`):

| query | gix | git (answer of record) |
|---|---|---|
| `#status` | `Repository::status()`, untracked files one by one, no renames, submodules `ignore=dirty` | `git --no-optional-locks status --porcelain=v2 -z -uall --no-renames --ignore-submodules=dirty` |
| `#index` | `Repository::open_index()` entries | `git ls-files -s -v -z` (+ intent-to-add bit from `--debug`) |
| `#index_rt` | the index written back with `gix_index::File::write_to` (default options) to `.git/gixfuzz-index-rt`, and re-read by gix | `git ls-files -s -v --debug` of the original and of gix's copy (`GIT_INDEX_FILE`), on-disk flag bits only |

Contracts: `status()` is "most similar to `git status --ignored=no`" (gix/src/status/mod.rs);
gix-index decodes V2..V4 and writes V2/V3 "depending on the content of the entries" (write.rs).
Status is compared as lines `I <X> path` (HEAD vs index), `W <Y> path` (index vs worktree),
`U path` (unmerged; nothing else for that path), `? path`. Areas: `unmerged`, `intent_to_add`,
`staged_add`, `untracked`, `worktree_typechange`, `staged_typechange`, `worktree_deleted`,
`staged_delete`, `worktree_modified`, `staged_modify`, `staged_delete_ita` (git's staged deletion
under an intent-to-add entry of a tracked path), `unresolvable_head` (gix errors, HEAD is a
dangling symref chain; git treats it as unborn). `wt_head_cycle`: HEAD through a symref cycle
(broken repo, no reference) is counted, not compared. `git_eloop_deletion`: a tracked path under a symlink
loop (`d -> d/c`, `d/c` tracked) is gone from the worktree, but git's lstat gets ELOOP and git
reports nothing; gix's `W D` alone is not a failure. Index areas `entries_stage`,
`entries_ita`, `entries_flags`, `entries`, `roundtrip`. These two families are capped per
signature and at `WT_FAMILY_CAP` (2) findings per process instead of 1 per family.

Generator: `Op::Dirty` (in the history, picked up by the next commit's `git add -A`, and 1..4 of
them at the end of half the histories): unstaged edits (also same-size, racy), staged edits
(also edited again), untracked files/dirs/empty dirs, `add -N` (also over a tracked path),
`rm --cached`, worktree deletion / replacement by a directory or symlink, `--skip-worktree`,
`--assume-unchanged` (and edits after), `.gitignore`/`info/exclude` rules with ignored and
re-included files, `--index-version 2|3|4`, `--split-index`/`--no-split-index`,
`--force-untracked-cache` + status, `--refresh`, a real (default-strategy) merge that may stop
with conflicts, `--chmod=±x`, nested repos, `core.autocrlf`/`eol=crlf`, unmerged stages by
`--index-info`, `git reset -- p`. The revspec `:path`/`:n:path` lookups see those indexes too.

### Generator: tree shapes

`Op::Shape` changes the worktree before the next commit: toggle +x, symlink, `git mv` (mostly to a
fresh `<path>.mv`), replace by a directory, multi-line content, copy, truncate, commit a gitlink
(`update-index --cacheinfo 160000,HEAD,<p>`), `rm -rf`, over `a b d/c d/e/f d.x d0 e/a d/e x/y/z d`
(`d.x` < `d/` < `d0` in git's tree order). The revspec `:path` lookups see those shapes too.

## Masked differences (each counted in stats.json)

Reference-side corrections (git deviating from its own docs):
- `skip_ambiguous_prefix`: gix resolves a short id git calls ambiguous. Documented superset
  (`gix/tests/gix/revision/spec/from_bytes/util.rs:139` "Git can't do that, but we can").
- `git_peel_error_but_ok`: `X@{n}^{tag}` on a non-tag makes git print "error: ... expected tag type"
  yet exit 0 with X@{n}; gitrevisions(7) says the peel must fail, so it counts as an error.
- `git_suffix_after_at_inconsistent`: after `ref@{...}`, a failing `^{/re}` makes git return the
  ref's current value. Suffixes apply to the object the prefix names, so the reference answer is
  git's answer for `<oid of prefix><suffix>` (computed only on disagreement).
- `git_full_refname_sibling`: git says "no such branch" for `refs/heads/x@{u}` / `heads/x@{push}`
  (`<branchname>@{upstream}`); gix accepts the longer spelling.
- `git_youngest_tie`: `:/re` / `^{/re}` ("the youngest matching commit") where gix's commit is at least
  as young as git's (repeated or skewed commit times from the generator's clock moves).
- `fastpath_overruled`: cat-file fast path disagreed with gix but rev-parse agreed (mostly specs
  that make cat-file die). Not a mask, just a counter.

Known gix bugs found by this harness, masked narrowly so they don't hide everything else
(`GIXFUZZ_NO_MASK=1` turns them off; use it to check a fix). Details in
`work/agents/harness-001/notes.md`:
- `known_sha256_short_hash` (B1): SHA-256 repos: every abbreviated id is rejected ("couldn't parse
  revision"). Masked for `gix_rejects` with that message in SHA-256 repos.
- `known_msgsearch_detached_head` (B2): `:/re` does not search from a detached HEAD.
- `known_detached_bare_reflog` (B5): bare `@{n}`/`@{date}` with a detached HEAD.
- `known_at_at` (B7): `@@{n}` does not parse.
- `known_prev_then_at` (B6): `@{-n}@{...}` "unconsumed input".
- `known_tag_branch_ambiguous` (B9/B10): a name that is both tag and branch with any `@{..}`:
  git's dwim_log / `@{-n}` use the branch, gix the tag.
- `known_local_upstream` (B12): upstream is a local branch (`branch.x.remote = .`).
- `known_branch_symref` (B13): symbolic refs under refs/heads (dangling or looping), specs with
  `@{` or `^{/`.
- `known_dangling_symref_peel_regex` (B11): `dangling^{/re}` searches all refs in gix.
- `known_trailing_slash_non_tree` (B14): `<rev>:<path>/` where `<path>` is a blob or symlink: git
  fails ("Needed a single revision"), gix returns the entry. Always signature
  `revspec__gix_accepts_tree_path` (whatever else the spec has); masked after `KNOWN_CAP` findings.

Not generated: approxidate-only date forms (`2023-11-15.21:23:20`, `... UTC`, `2023.11.15`) that
gix-date rejects (B3, left for a date oracle); `@{..}` after navigation (`HEAD^{tree}@{push}`),
where git's behaviour is odd.

Triage helpers: `gixfuzz resolve <case.json> [spec...]` prints gix's and git's answer
(`GIXFUZZ_KEEP=1` keeps the repo); `GIXFUZZ_TALLY_ONLY=1` skips shrinking and appends every
failure to `<out>/failures.tsv` (for looking at the failure distribution).

## Mutants

`mutants/*.diff` (patches against `work/src`, all in `gix/src/revision/spec/parse/delegate/`):
01 `~n` walks all parents instead of first-parent, 02 `@{date}` returns the entry's previous id,
03 `@{-n}` off-by-one, 04 `@{n}` off-by-one. Each was caught in 2 × 60 s (repros exit 1 on the
mutant build, 0 on base).

## Throughput

About 4 cases/s per process with 8 processes in parallel (one case = one generated repo plus
10..60 specs, ≈ 135 spec comparisons/s per process). The repo setup (bash + git) dominates.
Failure rate after masks ≈ 0.3% of cases (revspec only). With the treediff/merge queries and
tree shapes: ≈ 1.9 cases/s per process with 4 processes, 0.65% failures in 4 × 240 s (real bugs:
graph G1-G6, merge conflicted trees, B14, a gix-merge debug-assert panic). Shrinking stops after
20 s per finding.
