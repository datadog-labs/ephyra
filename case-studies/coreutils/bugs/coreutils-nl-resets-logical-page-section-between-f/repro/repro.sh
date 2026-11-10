#!/usr/bin/env bash
# nl must keep the current logical-page section (header/body/footer) across input files.
# Usage: repro.sh [NL]   NL = nl binary to test (default: "nl" on PATH; for a uutils
# multicall build pass e.g. "/path/to/target/debug/coreutils nl").
# Exit 1 = bug reproduces, 0 = output matches GNU coreutils 9.12, 2 = could not run.
NL=${1:-nl}
T=$(mktemp -d) || exit 2
trap 'rm -rf "$T"' EXIT
cd "$T" || exit 2
printf '\\:\n' > a       # a footer delimiter line
printf 'x\n'   > b       # one line, read while the footer section is in effect

# Footer lines are not numbered by default (-f n), so x stays unnumbered.
# GNU coreutils 9.12 prints an empty line for the delimiter, then x padded to the
# number column (6 spaces + separator TAB is replaced by 7 spaces for unnumbered lines).
printf '\n       x\n' > expected

# shellcheck disable=SC2086
LC_ALL=C $NL a b > actual 2> err || { echo "nl failed:"; cat err; exit 2; }
echo "--- expected (GNU coreutils 9.12)"; od -c expected
echo "--- actual"; od -c actual
if cmp -s expected actual; then echo "OK: section kept across files"; exit 0; fi
echo "BUG: nl restarted the body section at the second file"; exit 1
