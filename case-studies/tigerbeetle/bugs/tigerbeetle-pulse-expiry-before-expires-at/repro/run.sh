#!/usr/bin/env bash
# run.sh <tigerbeetle-git-checkout>
# Adds one unit test (expiry_test_*.zig, in the src/state_machine_tests.zig DSL) to a throwaway
# git worktree of the checkout and runs only that test. The checkout itself is not modified.
# Exit 1: the replica's state machine hits assert(expires_at <= timestamp_event) (bug present).
# Exit 0: the test passes. Other: could not decide (build error, etc.).
set -u
SRC=$(realpath "$1")
HERE=$(dirname "$(realpath "$0")")
WT=$(mktemp -d)/tb
git -C "$SRC" worktree add -q --detach "$WT" HEAD || exit 2
trap 'git -C "$SRC" worktree remove --force "$WT"' EXIT
cd "$WT"
# 0.17.x's test harness adds 1 ns per prepare; main's does not. The two files differ only in
# the tick offsets that make the timestamps line up.
if ! grep -q 'fn raw_request(' src/state_machine_tests.zig; then
  T="$HERE/expiry_test_0.17.zig"
else
  T="$HERE/expiry_test_main.zig"
fi
cat "$T" >> src/state_machine_tests.zig
./zig/download.sh > /dev/null 2>&1 || exit 2
./zig/zig build test:unit --summary all -Dgit-commit="$(git rev-parse HEAD)" \
  -- "expiry not before expires_at" > test.log 2>&1
rc=$?
grep -E 'assert\(expires_at <= timestamp_event\)|tests passed|passed' test.log | head -3
if grep -q 'assert(expires_at <= timestamp_event)' test.log; then exit 1; fi
if [ $rc = 0 ]; then exit 0; fi
tail -20 test.log
exit 2
