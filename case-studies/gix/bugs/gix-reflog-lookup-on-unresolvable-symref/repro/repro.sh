#!/bin/bash
# Repro: `<ref>@{<date>}` / `<ref>@{<n>}` on a symbolic ref that does not resolve (unborn HEAD,
# dangling refs/remotes/origin/HEAD) but has a reflog of its own. git refuses these specs
# ("fatal: Needed a single revision"); gix returns an entry from the dangling ref's reflog,
# in case B even the null id 0000000000000000000000000000000000000000.
# Usage: ./repro.sh <path-to-gitoxide-checkout>
# Exit 1: bug reproduces (gix resolves a spec that git rejects); 0: gix rejects all like git; 2: setup problem.
set -u
GIX_SRC=$(cd "${1:?usage: repro.sh <gitoxide-checkout>}" && pwd) || exit 2
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

cp -r "$HERE/revparse" "$WORK/revparse"
sed -i "s|GIX_PATH|$GIX_SRC/gix|" "$WORK/revparse/Cargo.toml"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HERE/target}" \
  cargo build -q --manifest-path "$WORK/revparse/Cargo.toml" || exit 2
BIN="${CARGO_TARGET_DIR:-$HERE/target}/debug/revparse"

export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null
export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@x GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@x
export GIT_AUTHOR_DATE='1700003600 +0000' GIT_COMMITTER_DATE='1700003600 +0000'
status=0
check() { # <repo> <spec>...
  local repo=$1; shift
  for s in "$@"; do
    if id=$(git -C "$repo" rev-parse --verify -q "$s"); then
      echo "git itself resolves $s ($id); repro setup is wrong"; exit 2
    fi
    out=$("$BIN" "$repo" "$s")
    echo "  git: $s: fatal: Needed a single revision"
    echo "  gix: $out"
    case "$out" in *": error: "*) ;; *) status=1 ;; esac
  done
}

# A: one commit on main, then HEAD is pointed at a branch that does not exist (unborn).
git init -q -b main "$WORK/a" && git -C "$WORK/a" commit -q --allow-empty -m one || exit 2
git -C "$WORK/a" symbolic-ref HEAD refs/heads/missing
echo "A: HEAD -> refs/heads/missing (unborn), HEAD has a reflog"
check "$WORK/a" 'HEAD@{0}' 'HEAD@{2023-11-15 08:23:20 +0000}'

# B: check out a branch that is a dangling symref, then delete it: HEAD's reflog only has a null-id entry.
git init -q -b main "$WORK/b" || exit 2
git -C "$WORK/b" symbolic-ref refs/heads/dev refs/heads/abcd
git -C "$WORK/b" checkout -q dev -- 2>/dev/null || git -C "$WORK/b" checkout -q -b dev
git -C "$WORK/b" branch -q -D dev
echo "B: HEAD unborn, reflog entry has the null id"
check "$WORK/b" 'HEAD@{2023-11-15 08:23:20 +0000}'

# C: refs/remotes/origin/HEAD is re-pointed at a missing remote branch; it keeps its own reflog.
git init -q -b main "$WORK/c" && git -C "$WORK/c" commit -q --allow-empty -m one || exit 2
git -C "$WORK/c" update-ref refs/remotes/origin/dev HEAD
git -C "$WORK/c" symbolic-ref refs/remotes/origin/HEAD refs/remotes/origin/dev
git -C "$WORK/c" symbolic-ref refs/remotes/origin/HEAD refs/remotes/origin/missing
echo "C: refs/remotes/origin/HEAD -> refs/remotes/origin/missing (dangling)"
check "$WORK/c" 'origin@{0}' 'origin@{2023-11-15T06:23:20Z}'
exit $status
