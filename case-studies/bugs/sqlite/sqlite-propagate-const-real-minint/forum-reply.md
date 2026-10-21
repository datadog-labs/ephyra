A related case at the same -2^63 boundary, with no index. It still reproduces on trunk fd68b587e9 (2026-10-14 12:36), which has the partial-index fix cb7816d72c and the OP_MustBeInt change 5e488c8da3:

```sql
CREATE TABLE t(a, b INT);
INSERT INTO t VALUES(1, -9223372036854775808);
SELECT typeof(b), b = -9223372036854775808.0 FROM t;                                -- integer|1
SELECT count(*) FROM t WHERE b = -9223372036854775808.0 AND typeof(b) = 'integer';  -- 0, expected 1
```

With `.testctrl optimizations -PropagateConst` the count is 1. Constant propagation marks the `b` in `typeof(b)` as fixed (EP_FixedCol), and it is coded as the constant followed by `OP_Affinity` with b's affinity. INT affinity leaves the REAL -2^63 a REAL (`ix>SMALLEST_INT64` in `sqlite3VdbeIntegerAffinity()`), so `typeof(b)` sees `real` for a row whose b is an integer. It is the same assumption as in the partial-index case above: that `col = constant` pins col to the constant converted with col's affinity.

It also fails with a NUMERIC column, in a STRICT table, with a bound parameter holding the REAL, and with `b || '' = '-9223372036854775808'` in place of `typeof(b)`. It fails the other way round too: a REAL -2^63 stored in an ordinary INT column is missed by `b = -9223372036854775808 AND typeof(b) = 'real'`. A REAL column is fine, and so are other values (`b = 5.0 AND typeof(b) = 'integer'` finds b = 5).

Dropping `ix>SMALLEST_INT64` fixes the count, but then tkt3922.2, tkt3922.3 and func4-3.18 fail (the text '-9223372036854775809' becomes the integer -9223372036854775808), and a REAL -2^63 already stored in a database would still be missed. So the fix probably belongs on the propagation side, for instance by not propagating a constant equal to -2^63 into a column with INT or NUMERIC affinity. We have not tried that.

Versions (official amalgamations or tarballs, default options): 3.8.2 to 3.35.0 return 1. 3.36.0, which started running constant propagation on single-table queries, through 3.54.0 and trunk return 0. With a one-row table cross-joined in front of t, it returns 0 from 3.25.0, which introduced constant propagation; 3.24.0 returns 1.
