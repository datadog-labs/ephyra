# Bugs: gix

The bugs the case studies found in gix. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [`Commit::describe()` ignores annotated tags that point to another tag](gix-describe-ignores-annotated-tag-of-tag/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3103) |
| [`<name>@{0}` fails with "reflog entries require a ref name" when the branch name looks like a short hash](gix-hex-named-branch-reflog-rejected/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3099) |
| [1. rev-parse of a short name fails when an earlier DWIM candidate is a dangling symbolic ref](gix-hex-prefix-shadowed-by-dangling-symref/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3090) |
| [Tree merge reports a false directory/file conflict when one side replaces `d/` with a file and the other only deletes part of `d/`](gix-merge-dir-to-file-vs-partial-delete-fals/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3106) |
| [Tree merge keeps only the ancestor when one side edits a file and the other turns it into a symlink or submodule](gix-merge-distinct-types-falls-back-to-ances/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3107) |
| [1. gix-merge: same file added on both sides, executable on one side only, is missing from the merged tree](gix-merge-identical-add-add-mode-drops-path/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3097) |
| [gix-merge: moving `d/e` to `y/z` makes the tree merge treat `d` as renamed to `y`](gix-merge-infers-parent-dir-rename-from-nest/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3108) |
| [gix-merge: tree merge loses a file renamed onto a directory that the other side modified](gix-merge-rename-onto-modified-dir-drops-ent/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3109) |
| [gix-merge: debug assertion panic when one side renames and modifies a file and the other side makes the same modification](gix-merge-rename-vs-identical-modify-debug-p/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3091) |
| [1. gix-merge: same file added on both sides, executable on one side only, is missing from the merged tree](gix-merge-same-content-exec-bit-change-keeps/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3097) |
| [`HEAD^{/text}` on an unborn HEAD searches all refs instead of failing](gix-peel-regex-on-unresolvable-ref-searches-/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3100) |
| [1. rev-parse of a short name fails when an earlier DWIM candidate is a dangling symbolic ref](gix-prev-checkout-dangling-symref-falls-back/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3090) |
| [`@{-1}` peels an annotated tag that git returns unpeeled](gix-prev-checkout-peels-annotated-tag/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3095) |
| [1. `@{push}` fails when the branch has no configured remote; git falls back to `origin`](gix-push-revspec-ignores-default-remote/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3096) |
| [1. `@{push}` fails when the branch has no configured remote; git falls back to `origin`](gix-push-revspec-upstream-mapped-through-pus/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3096) |
| [1. `rev_parse_single("<ref>@{N}")` fails when the reflog has exactly N entries](gix-reflog-at-zero-uses-stale-log-entry/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3089) |
| [1. `rev_parse_single("<ref>@{N}")` fails when the reflog has exactly N entries](gix-reflog-deletion-entry-resolves-to-null-o/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3089) |
| [1. `rev_parse_single("<ref>@{N}")` fails when the reflog has exactly N entries](gix-reflog-lookup-on-unresolvable-symref/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3089) |
| [1. `rev_parse_single("<ref>@{N}")` fails when the reflog has exactly N entries](gix-reflog-nth-equal-to-entry-count-rejected/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3089) |
| [`rev_walk().with_hidden()` returns ancestors of hidden commits when commit times are skewed and there is no commit-graph](gix-rev-walk-hidden-skewed-commit-times-leak/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3104) |
| [1. rev-parse of a short name fails when an earlier DWIM candidate is a dangling symbolic ref](gix-revspec-dwim-picks-dangling-symref/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3090) |
| [`@{u}@{1}` is rejected with "unconsumed input"; git reads the upstream ref's reflog](gix-revspec-sibling-branch-reflog-rejected/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3098) |
| [`rev_parse` resolves `HEAD:file/` to the blob of `file`; git rejects a trailing slash on a non-directory](gix-revspec-tree-path-trailing-slash-accepts/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3102) |
| [SHA-256 repositories: loose objects are never found by an abbreviated id](gix-sha256-loose-object-prefix-lookup-misses/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3101) |
| [`status()` reports an executable file replaced by a symlink as `Modified` instead of `TypeChange`](gix-status-executable-replaced-by-symlink-no/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3111) |
| [`status()` fails when HEAD's branch is a symbolic ref to an unborn branch](gix-status-fails-on-head-symref-chain-to-unb/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3112) |
| [`status()` lists a file that replaced a submodule (gitlink) as untracked, in addition to the type change](gix-status-file-replacing-gitlink-listed-unt/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3113) |
| [`gix status` ignores a gitlink whose path is not listed in `.gitmodules`](gix-status-ignores-gitlink-without-gitmodule/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3114) |
| [Text merge reports no conflict when both sides append different lines at the end of a file (diff hunks placed with the indent heuristic)](gix-text-merge-indent-heuristic-hides-confli/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3110) |
| [`gix_traverse::commit::topo` returns a tip that an end hides (with `Parents::First`, or when the tip is also an end)](gix-topo-first-parent-emits-hidden-tip/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3105) |
| [`status()` / `gix_diff::index()` miss a staged deletion when the index entry is intent-to-add](gix-tree-index-diff-hides-deletion-under-int/) | fixed | [link](https://github.com/GitoxideLabs/gitoxide/issues/3115) |
