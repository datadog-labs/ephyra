# Tree merge keeps only the ancestor when one side edits a file and the other turns it into a symlink or submodule

Posted as https://github.com/GitoxideLabs/gitoxide/issues/3107 on 2026-10-23. The text below is the issue as posted.

---

### Current behavior 😯

Our branch replaces the regular file `a` with a symlink, and their branch edits `a`. `gix merge commit` reports a
conflict (`resolution: Err(Unknown)`). The merged tree then holds only the base version of `a`. Neither the symlink
nor the edited file is in it.

```
$ gix merge commit main theirs        # then: git ls-tree <printed tree id>
# gix: 100644 blob d00491fd…  a        (the base content "1")
```

The same happens when our side turns `a` into a gitlink (submodule) instead of a symlink.

### Expected behavior 🤔

Both sides' versions should end up in the merged tree, as `git merge-tree` does. `gix-merge/src/tree/function/resolve.rs`
says: "Git's `merge-ort` is the behavioral reference". Dropping both sides also loses their content from the result tree.

The `Unknown` variant says it is for "an untested state" that is "considered a conflict without adding our *or* their
side to the resulting tree". The code calls the `_unknown` arm a "Defensive, best-effort fallback" so that "a tested
combination can gain an explicit arm". A file edited on one side and replaced by a symlink or submodule on the other
is an ordinary case that git handles explicitly.

### Git behavior

```
$ git merge-tree --write-tree main theirs   # then: git ls-tree <tree>
# git 2.56: 120000 blob 1de56593…  a          (our symlink)
# git 2.56: 100644 blob 0cfbf088…  a~theirs   (their edited file)
# git 2.56: CONFLICT (distinct types): a had different types on each side; renamed one of them so each can be recorded somewhere.
$ gix merge commit main theirs              # then: git ls-tree <tree>
# gix: 100644 blob d00491fd…  a                (base file only)
```

With a gitlink instead of a symlink, git keeps the gitlink at `a` and the edited file at `a~theirs`. gix again keeps
only the base file.

### Steps to reproduce 🕹

```sh
git init -q -b main repo && cd repo
echo 1 > a && git add a && git commit -q -m base
git branch theirs
git rm -q a && ln -s target a && git add a && git commit -q -m "a becomes a symlink"
git checkout -q theirs && echo 2 > a && git commit -q -am "edit a" && git checkout -q main

git ls-tree $(git merge-tree --write-tree main theirs | head -1)   # a (symlink) and a~theirs (edited file)
git ls-tree $(gix merge commit main theirs | cut -d' ' -f1)        # only a (base file)
```

The same thing through the library: `repo.merge_commits(main, theirs, labels, repo.tree_merge_options()?.into())`.
Its `tree_merge.tree` contains only the base `a`.

### Notes

- Seen on gitoxide `main` at e87b6eefb (2026-10-22, macOS aarch64; the current main f984c56f0 only changes a test) and earlier on 59ba38b89 (2026-10-21, Linux), compared with git 2.56.0.
- Where in the code: `gix-merge/src/tree/function/resolve.rs`. The modify/modify arms only handle blob modes that
  `merge_modes` can combine, or gitlink vs gitlink. The add/add type-mismatch arm excludes submodules. So these pairs
  reach the `_unknown =>` fallback, which leaves the ancestor in place:
  - modification vs modification with blob vs symlink or gitlink;
  - addition vs addition with blob or symlink vs gitlink (e.g. one side adds a file `d/f`, the other a gitlink at
    `d/f`). git keeps the gitlink at `d/f` and the file at `d/f~main`. gix's tree has nothing at either path.
- When neither side is a regular file (symlink vs gitlink), git moves both, to `a~main` and `a~theirs`.
- This is not only the documented "submodule merges count as conflicts" limitation (crate-status.md). It also happens
  with a plain symlink, and git reports a conflict here too. The difference is the content of the merged tree.
- Related: #2968 / #2969 (identical add/add reaching the `Unknown` fallback, now fixed). This is a different input.
- The `Unknown` fallback has been there since the first tree-merge release, so released versions are affected.

This issue was drafted and submitted by an AI assistant (Claude) on behalf of @alpaylan, at their request, from an automated differential test (gix vs git 2.56).
