#!/bin/sh
# Make the file with SQLite (it stores the whole-number REAL 3.0 as an integer on disk), read it with Turso.
SQLITE=${SQLITE:-sqlite3}  # a SQLite 3.50.4 CLI
TURSO=${TURSO:-$HOME/Projects/turso/target/release/tursodb}
rm -f /tmp/hj-real.db
$SQLITE /tmp/hj-real.db <<'SQL'
CREATE TABLE r(a REAL, b INT);
INSERT INTO r VALUES(3.0, 1);
CREATE TABLE u(y INT);
INSERT INTO u VALUES(1);
SQL
Q="SELECT r.a, typeof(r.a), r.a/2 FROM r JOIN u ON u.y = r.b;
SELECT r.a, typeof(r.a) FROM r LEFT JOIN u ON u.y = r.b;
SELECT r.a, typeof(r.a), r.a/2 FROM r;"
echo "== SQLite"; echo "$Q" | $SQLITE /tmp/hj-real.db
echo "== Turso";  echo "$Q" | $TURSO -q -m list /tmp/hj-real.db
