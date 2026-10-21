DRAFT, not filed (low priority)

# -0.0 stored in a REAL column keeps its sign; SQLite reads it back as +0.0

## Reproduction

```sql
CREATE TABLE r(x REAL);
CREATE TABLE n(x);
INSERT INTO r VALUES (-0.0);
INSERT INTO n VALUES (-0.0);
SELECT 'real column', atan2(0, x) FROM r;
SELECT 'no affinity', atan2(0, x) FROM n;
```

Turso (`tursodb -q -m list`, main f8549ca2):
```
real column|3.14159265358979
no affinity|3.14159265358979
```

SQLite trunk f69fd5f5:
```
real column|0.0
no affinity|3.1415926535897931
```

The SQLite 3.50.4 CLI used here is built without math functions, so `atan2` fails there; reading the values through the C API (`sqlite3_column_double`) on 3.50.4 gives the same split: `SELECT x FROM r` is `0.0` and `SELECT x FROM n` is `-0.0` (Turso: `-0.0` for both). Text output is `0.0` in every case, in both engines.

## Expected

+0.0 from a REAL column. SQLite stores a REAL-affinity value that is a whole number as an integer (`MEM_IntReal`), so -0.0 is written as integer 0 and read back as +0.0. Turso writes the f64 with its sign. The difference shows in math functions (`atan2`), in JSON built from the value (see #9840), and to API users.

Tested on Turso main f8549ca2.
