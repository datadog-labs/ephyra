#!/bin/bash
# Builds the bug benchmark's ada variants into bench/mutants/<id>/: ada main
# 42963db with the open fixes, minus one (the bug is back), or with an
# upstream fix reverted. "control" has every fix: the suite must pass on it.
# #1283 is left out: it only shows under libc++ hardening, which this build
# does not use.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
B=$HERE/../../bugs/ada
OUT=$HERE/mutants
BASE=42963db48056403f6ec3ff6e6dbea62ed724b6ec
SRC=$(mktemp -d /tmp/adamut.XXXXXX)
git clone -q --shared "${ADA_SRC:?path to an ada clone}" "$SRC/ada"
cd "$SRC/ada"
FIXES="1278 1282 1283 1284 1285 1326"
fix_dir() {  # bash 3.2 (macOS) has no associative arrays
  case $1 in
    1278) echo ada-file-drive-letter-opaque-path ;;
    1282) echo ada-set-host-port-length-limit ;;
    1283) echo ada-urlpattern-protocol-colon-oob ;;
    1284) echo ada-urlpattern-opaque-pathname-authority ;;
    1285) echo ada-urlpattern-search-special-query-encode ;;
    1326) echo ada-set-host-idna-expansion-limit ;;
  esac
}
# Put back an upstream fix's bug by hand (reverting conflicts with later
# commits): replace exactly one occurrence, or fail.
put_back() {  # put_back <file> <new text> <old text>
  python3 - "$1" "$2" "$3" <<'PY'
import sys
path, new, old = sys.argv[1:4]
s = open(path).read()
assert s.count(new) == 1, (path, new, s.count(new))
open(path, "w").write(s.replace(new, old))
PY
}
revive_1280() {  # both declarations (in test and in match), exactly two
  python3 - <<'PY'
p = "include/ada/url_pattern-inl.h"
s = open(p).read()
new = "result<url_aggregator> base_url = tl::unexpected(errors::type_error);"
assert s.count(new) == 2, s.count(new)
open(p, "w").write(s.replace(new, "result<url_aggregator> base_url;"))
PY
}
revive_1281() {
  put_back src/url_pattern_helpers.cpp 'ada::parse<url_aggregator>("https://dummy.invalid/", nullptr);' 'ada::parse<url_aggregator>("fake://fake-url", nullptr);'
}
revive_idna97() {  # ada's bundled IDNA from before #1288 (stale tables)
  git show 473c293e~1:src/ada_idna.cpp > src/ada_idna.cpp
  git show 473c293e~1:include/ada/ada_idna.h > include/ada/ada_idna.h
}
build() {  # build <id> <fix ids to skip, space separated> <revive function or "">
  local id=$1 skip=$2 revive=$3
  git checkout -q -- . && git clean -qfd && git checkout -q "$BASE"
  for k in $FIXES; do
    case " $skip " in *" $k "*) continue ;; esac
    git apply --exclude='tests/*' "$B/$(fix_dir "$k")/fix.patch"
  done
  if [ -n "$revive" ]; then "$revive"; fi
  mkdir -p "$OUT/$id"
  AMALGAMATE_OUTPUT_PATH="$OUT/$id" python3 singleheader/amalgamate.py >/dev/null
  rm -f "$OUT/$id"/{demo.c,demo.cpp,singleheader.zip,README.md}
  git reset -q --hard
  echo "built $id"
}
build control "" ""
for k in 1278 1282 1284 1285 1326; do build "m$k" "$k" ""; done
build m1280 "" revive_1280
build m1281 "" revive_1281
build midna97 "" revive_idna97
