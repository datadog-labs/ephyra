#!/bin/bash
# gix resolves `HEAD:file/` (trailing slash on a file path) to the file's blob; git rejects it.
#   git rev-parse --verify HEAD:file/   # fatal: Needed a single revision
#   gix rev parse HEAD:file/            # prints the blob id of `file`
# Usage: ./repro.sh <path-to-gix-binary> [git-binary]
# Exit 1: bug reproduces; 0: gix agrees with git; 2: setup problem.
set -u
GIX=$(realpath "${1:?usage: repro.sh <gix-binary> [git-binary]}") || exit 2
GIT=${2:-git}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null HOME="$WORK"
export GIT_AUTHOR_NAME=a GIT_AUTHOR_EMAIL=a@x GIT_COMMITTER_NAME=a GIT_COMMITTER_EMAIL=a@x
export GIT_AUTHOR_DATE='1700000000 +0000' GIT_COMMITTER_DATE='1700000000 +0000'
cd "$WORK" && "$GIT" init -q -b main repo && cd repo || exit 2
mkdir dir && echo hi > file && echo x > dir/f
"$GIT" add -A && "$GIT" commit -q -m init || exit 2

status=0
for spec in HEAD:file/ HEAD:dir/ HEAD:dir//f HEAD:dir/./f; do
  g=$("$GIT" rev-parse -q --verify "$spec" 2>/dev/null) || g="error"
  x=$("$GIX" rev parse "$spec" 2>/dev/null) || x="error"
  echo "$spec"
  echo "  # git: $g"
  echo "  # gix: $x"
  [ "$g" = "$x" ] || status=1
done
exit $status
