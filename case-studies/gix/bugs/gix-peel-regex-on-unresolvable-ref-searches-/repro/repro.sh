#!/bin/bash
# Repro: `<rev>^{/<regex>}` where <rev> is a symbolic ref that cannot be peeled (unborn HEAD, dangling
# symref). git refuses the spec ("fatal: Needed a single revision"); gix ignores the anchor and searches
# commits reachable from *all* references, like `:/<regex>`, and returns a commit.
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
export GIT_AUTHOR_DATE='1700000000 +0000' GIT_COMMITTER_DATE='1700000000 +0000'
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

# A: one commit "wip" on main, then an orphan checkout: HEAD -> refs/heads/unborn (unborn).
git init -q -b main "$WORK/a" && git -C "$WORK/a" commit -q --allow-empty -m wip || exit 2
git -C "$WORK/a" checkout -q --orphan unborn
echo "A: HEAD -> refs/heads/unborn (unborn); main has commit 'wip'"
check "$WORK/a" 'HEAD^{/wip}' '@^{/wip}'

# B: a branch that is a symref to a branch that does not exist.
git init -q -b main "$WORK/b" && git -C "$WORK/b" commit -q --allow-empty -m wip || exit 2
git -C "$WORK/b" symbolic-ref refs/heads/alias refs/heads/missing
echo "B: refs/heads/alias -> refs/heads/missing (dangling)"
check "$WORK/b" 'alias^{/wip}' 'alias^{/!-x}'
exit $status
