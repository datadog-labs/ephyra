**Title:** WHERE b = -9223372036854775808.0 AND typeof(b) = 'integer' misses a row whose INT column b holds that integer

```sql
CREATE TABLE t(a, b INT);
INSERT INTO t VALUES(1, -9223372036854775808);
SELECT typeof(b), b = -9223372036854775808.0 FROM t;
SELECT count(*) FROM t WHERE b = -9223372036854775808.0 AND typeof(b) = 'integer';
```

Output from trunk (check-in fd68b587e9, 2026-10-14 12:36, which includes the partial-index fix cb7816d72c and the OP_MustBeInt change 5e488c8da3), built from the source tarball with default options:

```
integer|1
0
```

Expected `1` from the second query: the first query shows that the row's `b` is an integer and equals -9223372036854775808.0 (INTEGER and REAL compare exactly). With `.testctrl optimizations -PropagateConst` it returns 1.

It also happens with a bound parameter (`b = ?1`, with ?1 the REAL -2^63), with `b || '' = '-9223372036854775808'` in place of `typeof(b)`, with a NUMERIC column, and in a STRICT table. It happens the other way round too: a REAL -9223372036854775808.0 stored in an ordinary INT column (INT affinity leaves it a REAL) is missed by `WHERE b = -9223372036854775808 AND typeof(b) = 'real'`. REAL columns are fine, and so are other values (`b = 5.0 AND typeof(b) = 'integer'` finds b = 5).

**Cause, as far as we can tell.** Constant propagation marks the `b` in `typeof(b)` as fixed (EP_FixedCol). `sqlite3ExprCodeTarget()` then codes it as the constant followed by `OP_Affinity` with b's affinity. That gives b's value only if b's affinity turns every value equal to the constant into the same value. -9223372036854775808.0 is the exception, because `sqlite3VdbeIntegerAffinity()` leaves it a REAL:

```c
    if( pMem->u.r==ix && ix>SMALLEST_INT64 && ix<LARGEST_INT64 ){
```

So an INT column can hold either the INTEGER or the REAL -2^63, both equal the constant, and the propagated value has the type of only one of them. The exclusion is deliberate (ticket #3922). Dropping `ix>SMALLEST_INT64` fixes the query above, but then tkt3922.test and func4-3.18 fail, because the text '-9223372036854775809' becomes the integer -9223372036854775808. A REAL -2^63 already stored in a database would still be missed. So the fix probably belongs on the propagation side, for instance by not propagating a constant equal to -2^63 into a column with INT or NUMERIC affinity. We have not tried that.

A partial index `WHERE b = -9223372036854775808.0` goes wrong on the same value ([forum post 35992eec46](https://sqlite.org/bugs/forumpost/35992eec46)), and so do a Bloom filter before a rowid lookup and a foreign-key check on an INTEGER PRIMARY KEY parent ([forum post 403ca95409](https://sqlite.org/bugs/forumpost/403ca95409)).

**Versions** (official amalgamations or tarballs, default options): 3.8.2, 3.20.0, 3.28.0 and 3.35.0 return 1. 3.36.0, which started running constant propagation on single-table queries, returns 0, as do 3.39.0, 3.44.0, 3.49.0, 3.53.4, 3.54.0 and trunk fd68b587e9. With a join (`FROM o CROSS JOIN t`, where `o` has one row), it returns 0 from 3.25.0, which introduced constant propagation; 3.24.0 returns 1.

We found it with a property-based test generator for SQLite databases and queries, while reducing a related case with the same constant: the query gave a different count with constant propagation on and off.
