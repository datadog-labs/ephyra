#!/bin/sh
# Creates the libsqlite3-sys crates the SQLite case studies patch in
# through [patch.crates-io]: libsqlite3-sys 0.35.0 (the version rusqlite
# 0.37 uses) from crates.io, with its bundled amalgamation (sqlite3.c,
# sqlite3.h, sqlite3ext.h) replaced. The prebuilt bindings stay 3.50.2's,
# which these versions' ABI keeps.
#
#   libsqlite3-sys-trunk           SQLite trunk at $TRUNK, with Geopoly enabled
#                                  (ephyra-sqlite, ephyra-sqlite-pinned,
#                                  sqlite-coverage, diversity-probe)
#   libsqlite3-sys-trunk-bfe473ab  the same check-in, build options unchanged
#                                  (turso-diff/trunk-oracle)
#   libsqlite3-sys-3.50.4          the 3.50.4 release (turso-diff)
#
# sqlite-coverage's sections.tsv is generated from the trunk amalgamation
# (deploy/sections.py); regenerate it after moving TRUNK.
#
# Needs curl, tar, unzip, a C compiler, make and tclsh (to build sqlite3.c
# from the source tree). An existing directory is left alone; delete it to
# fetch it again.
set -eu

TRUNK=${TRUNK:-bfe473ab60f666449b819f0a55e936cef5359e32fd61683437d2ee3e0f3ff6c6}
CRATE=libsqlite3-sys-0.35.0
RELEASE_ZIP=https://sqlite.org/2025/sqlite-amalgamation-3500400.zip

here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

crate() { # crate DIR: a fresh copy of the crates.io crate at DIR
	if [ ! -d "$tmp/$CRATE" ]; then
		curl -fsSL "https://static.crates.io/crates/libsqlite3-sys/$CRATE.crate" | tar -xz -C "$tmp"
	fi
	cp -R "$tmp/$CRATE" "$1"
}

trunk_amalgamation() { # builds sqlite3.c and sqlite3.h of $TRUNK once, in $tmp/sqlite
	if [ ! -f "$tmp/sqlite/sqlite3.c" ]; then
		curl -fsSL "https://sqlite.org/src/tarball/$TRUNK/sqlite.tar.gz" | tar -xz -C "$tmp"
		(cd "$tmp/sqlite" && ./configure >/dev/null && make sqlite3.c >/dev/null)
	fi
}

put_trunk() { # put_trunk DIR
	trunk_amalgamation
	cp "$tmp/sqlite/sqlite3.c" "$tmp/sqlite/sqlite3.h" "$1/sqlite3/"
	cp "$tmp/sqlite/src/sqlite3ext.h" "$1/sqlite3/"
}

d="$here/libsqlite3-sys-trunk"
if [ ! -d "$d" ]; then
	crate "$d.tmp"
	put_trunk "$d.tmp"
	# Geopoly is off in libsqlite3-sys's bundled build; the SQLite case studies test it.
	sed 's|^\( *\)\.flag("-DSQLITE_ENABLE_FTS5")$|&\
\1.flag("-DSQLITE_ENABLE_GEOPOLY")|' "$d.tmp/build.rs" > "$d.tmp/build.rs.new"
	mv "$d.tmp/build.rs.new" "$d.tmp/build.rs"
	grep -q 'SQLITE_ENABLE_GEOPOLY' "$d.tmp/build.rs" || { echo "fetch.sh: could not enable Geopoly in build.rs" >&2; exit 1; }
	mv "$d.tmp" "$d"
	echo "$d: trunk ${TRUNK%"${TRUNK#????????????}"}, Geopoly"
fi

d="$here/libsqlite3-sys-trunk-bfe473ab"
if [ ! -d "$d" ]; then
	crate "$d.tmp"
	put_trunk "$d.tmp"
	mv "$d.tmp" "$d"
	echo "$d: trunk ${TRUNK%"${TRUNK#????????????}"}"
fi

# sqlite-coverage's refresh-trunk.sh moves libsqlite3-sys-trunk to newer
# check-ins in place and records which one in this note.
if [ ! -f "$here/README.md" ]; then
	cat > "$here/README.md" <<EOF
# vendor/libsqlite3-sys-trunk

\`libsqlite3-sys\` 0.35.0 (the version rusqlite 0.37 uses) from crates.io, with its bundled
amalgamation (\`sqlite3/sqlite3.c\`, \`sqlite3.h\`, \`sqlite3ext.h\`) replaced by SQLite trunk:
check-in ${TRUNK%"${TRUNK#????????????}"} ($(sed -n 's/^#define SQLITE_SOURCE_ID *"\([0-9-]* [0-9:]*\) .*/\1/p' "$here/libsqlite3-sys-trunk/sqlite3/sqlite3.h")), built from https://sqlite.org/src/tarball/trunk/sqlite.tar.gz with
\`./configure && make sqlite3.c\`, and Geopoly enabled in build.rs. Created by fetch.sh.
EOF
fi

d="$here/libsqlite3-sys-3.50.4"
if [ ! -d "$d" ]; then
	crate "$d.tmp"
	curl -fsSL -o "$tmp/release.zip" "$RELEASE_ZIP"
	unzip -q -o "$tmp/release.zip" -d "$tmp/release"
	cp "$tmp"/release/sqlite-amalgamation-*/sqlite3.c "$tmp"/release/sqlite-amalgamation-*/sqlite3.h \
		"$tmp"/release/sqlite-amalgamation-*/sqlite3ext.h "$d.tmp/sqlite3/"
	mv "$d.tmp" "$d"
	echo "$d: 3.50.4"
fi
