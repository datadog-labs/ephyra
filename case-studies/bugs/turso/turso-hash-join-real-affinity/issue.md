Filed as #9881 (https://github.com/tursodatabase/turso/issues/9881) (line 1 corrected 2026-10-21).

# A REAL column read through a hash join comes back as an integer (SQLite-created file)

SQLite writes a whole-number value in a REAL column (`3.0`) to disk as an integer and turns it back into a REAL when it reads the column. Turso does this for a plain scan, but not when the column is carried as payload through a hash join, so the value is an integer: `typeof` says `integer` and `a/2` does integer division.

## Reproduction

The file has to be written by SQLite (Turso writes `3.0` as an 8-byte float, so a Turso-written file does not show it):

```sh
sqlite3 r.db <<'SQL'
CREATE TABLE r(a REAL, b INT);
INSERT INTO r VALUES(3.0, 1);
CREATE TABLE u(y INT);
INSERT INTO u VALUES(1);
SQL
tursodb -q -m list r.db <<'SQL'
SELECT r.a, typeof(r.a), r.a/2 FROM r JOIN u ON u.y = r.b;
SELECT r.a, typeof(r.a) FROM r LEFT JOIN u ON u.y = r.b;
SELECT r.a, typeof(r.a), r.a/2 FROM r;
SQL
```

## Turso output

```
3|integer|1
3|integer
3.0|real|1.5
```

## SQLite 3.50.4 output

```
3.0|real|1.5
3.0|real
3.0|real|1.5
```

## Expected

`3.0|real|1.5` for all three. The same happens with a `STRICT` table and a `REAL` column. `INSERT INTO x SELECT r.a FROM r JOIN u ON u.y = r.b` stores the integer `3`, so it reaches disk.

## Likely cause

`core/translate/main_loop/hash.rs`: the hash build loop reads each payload column with `emit_column_or_rowid` and emits no `RealAffinity` for REAL columns (in `EXPLAIN`, the key column gets `RealAffinity` but the payload `Column` right after it does not). Other column reads go through `maybe_apply_affinity` (`core/translate/expr/utils.rs`).

Tested on Turso main f8549ca2.
