DRAFT, not filed. Duplicate of open issue #6749 ("WITHOUT ROWID insert applies affinity in schema order"; fix PR #6675). Not worth a new issue. A comment with the UNIQUE false positive below may help show the impact.

Title: WITHOUT ROWID insert applies column affinities to the wrong columns when the PRIMARY KEY is not first, causing false UNIQUE failures

Reproduction (`tursodb -q -m list --experimental-without-rowid x.db < repro.sql`):

```sql
CREATE TABLE t(a TEXT, b INTEGER, PRIMARY KEY(b)) WITHOUT ROWID;
INSERT INTO t(a, b) VALUES (5, '7');
SELECT typeof(a), typeof(b) FROM t;
CREATE TABLE u(c0 TEXT, c1, PRIMARY KEY(c1)) WITHOUT ROWID;
INSERT INTO u(c1) VALUES (1.5);
INSERT INTO u(c1) VALUES (1.5000000000000002);
SELECT typeof(c1), quote(c1) FROM u;
```

Turso (7641fad2):
```
integer|text
Runtime error: UNIQUE constraint failed: u.c1 (19)
text|'1.5'
```

SQLite 3.50.4:
```
text|integer
real|1.5
real|1.500000000000000223e+00
```

Column a gets b's INTEGER affinity and b gets a's TEXT affinity. In `u`, c1 gets c0's TEXT affinity, so 1.5000000000000002 becomes '1.5' and collides with the first row. With `c0 DOUBLE PRECISION`, a big integer key becomes REAL and collides with a neighbouring integer (1346429344266934015 vs ...016).

Likely cause: in `core/translate/insert.rs` (translate_insert, the "apply column affinity to the values early" block), the affinity string comes from `col_mappings` in table-column order. It is then applied to the contiguous registers starting at `first_col_register()`, and for WITHOUT ROWID those registers are in record order (PK first). EXPLAIN shows `Affinity r[3..5] = B, D` with r[3] = b.

Tested on Turso main 7641fad2 (2026-10-13) with --experimental-without-rowid.
