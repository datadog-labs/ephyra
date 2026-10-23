Filed as #9925 (https://github.com/tursodatabase/turso/issues/9925) (line 1 corrected 2026-10-21).

Title: A result column `-9223372036854775808` becomes REAL when ORDER BY / GROUP BY has `-9.223372036854776e18` (and the other way round)

```sql
CREATE TABLE t(x);
INSERT INTO t VALUES (1);
SELECT -9223372036854775808 FROM t ORDER BY -9.223372036854776e18;
SELECT -9223372036854775808 FROM t GROUP BY -9.223372036854776e18;
SELECT -9.223372036854776e18 FROM t ORDER BY -9223372036854775808;
SELECT -9223372036854775808 AS m, typeof(-9223372036854775808) FROM t ORDER BY -9.223372036854776e18;
```

Turso (`tursodb -q -m list`):

```
-9.22337203685478e+18
-9.22337203685478e+18
-9223372036854775808
-9.22337203685478e+18|integer
```

SQLite 3.50.4 (trunk is the same apart from printing the REAL with 17 digits):

```
-9223372036854775808
-9223372036854775808
-9.22337203685478e+18
-9223372036854775808|integer
```

Expected: each result column keeps its own value and type: the INTEGER -2^63 for `-9223372036854775808` and the REAL for `-9.223372036854776e18`. The last line shows the problem: `typeof()` of the same literal says integer, but the column itself comes back as a REAL.

Likely cause: Turso reuses the ORDER BY / GROUP BY value for a result column when `exprs_are_equivalent` says the two expressions are the same (core/translate/order_by.rs ~771). For numeric literals that calls `cmp_numeric_strings` (core/util.rs:325). That function keeps integers and floats apart, but it parses `9223372036854775808` (the operand of the unary minus) as an f64 because it doesn't fit in i64. So it compares equal to `9.223372036854776e18`, and `-9223372036854775808` matches `-9.223372036854776e18`. Other values (`5` vs `5.0`, `-9223372036854775807` vs `-9.223372036854776e18`) aren't affected.

Tested on Turso main f8549ca2.
