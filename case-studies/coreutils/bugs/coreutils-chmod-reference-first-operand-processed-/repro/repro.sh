#!/bin/sh
# usage: repro.sh [CHMOD]   (default: chmod on PATH)
# With --reference, chmod should process FILE operands in command-line order.
CHMOD=${1:-chmod}
T=$(mktemp -d) || exit 2
cd "$T" || exit 2

echo '## 1: -v output order'
touch ref a b
chmod 644 ref; chmod 600 a b
"$CHMOD" -v --reference=ref a b
echo "exit=$?"

echo '## 2: directory listed after a file inside it'
mkdir dir; touch dir/file
chmod 755 dir; chmod 600 dir/file
"$CHMOD" --reference=ref dir/file dir
echo "exit=$?"

cd /; chmod -R u+rwx "$T"; rm -rf "$T"
