#!/bin/sh
# Turso can't CREATE INDEX on a WITHOUT ROWID table, so SQLite makes the file.
SQLITE=${SQLITE:-sqlite3}  # a SQLite 3.50.4 CLI
TURSO=${TURSO:-$HOME/Projects/turso/target/release/tursodb}
rm -f /tmp/wr-idx.db
$SQLITE /tmp/wr-idx.db <<'SQL'
CREATE TABLE t(a, b, c, PRIMARY KEY(c)) WITHOUT ROWID;
CREATE INDEX t_a ON t(a);
INSERT INTO t VALUES(1,'x',10),(1,'y',20);
SQL
Q="SELECT * FROM t WHERE a = 1;
SELECT DISTINCT * FROM t WHERE a = 1;
SELECT count(DISTINCT c) FROM t WHERE a = 1;"
echo "== SQLite"; echo "$Q" | $SQLITE /tmp/wr-idx.db
echo "== Turso";  echo "$Q" | $TURSO -q -m list --experimental-without-rowid /tmp/wr-idx.db
