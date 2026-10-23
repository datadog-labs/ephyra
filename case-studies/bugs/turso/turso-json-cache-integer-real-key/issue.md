Filed as #9967 (https://github.com/tursodatabase/turso/issues/9967) (line 1 corrected 2026-10-21).
Title: JSON functions return the integer for a whole-number REAL when the same integer was seen just before (JSON cache key compares 1 and 1.0 as equal)

When a JSON function gets the REAL `1.0` right after it got the INTEGER `1` (for example on consecutive rows), Turso treats `1.0` as the integer `1`. Alone, `1.0` works.

```sql
CREATE TABLE t(c);
INSERT INTO t VALUES (1), (1.0), (0), (0.0);
SELECT c, json_extract(c, '$'), c ->> '$', json_set(c, '$.a', 1) FROM t;
```

Turso (`tursodb -q -m list`):

```
1|1|1|1
1.0|1|1|1
0|0|0|0
0.0|0|0|0
```

SQLite 3.50.4 (trunk is the same):

```
1|1|1|1
1.0|1.0|1.0|1.0
0|0|0|0
0.0|0.0|0.0|0.0
```

Expected: the SQLite output. With only the `1.0` row, Turso returns `1.0`. The order matters: `1.0` first and then `1` gives `1.0` for both rows, so the integer row is wrong then.

Likely cause: `JsonCache::lookup` (core/json/cache.rs ~70) finds a cached parse with `key == *stored_key`. `ValueRef` equality counts `Integer(1)` and `Float(1.0)` as equal, so the second value reuses the first value's parsed JSON. The cache key needs to compare the value's type as well.

Tested on Turso main abf885b2 (also on f8549ca2).
