**Title:** In a UTF-16 database, a blob DEFAULT from ALTER TABLE ADD COLUMN is converted to text as UTF-8

```sql
PRAGMA encoding = 'UTF-16le';
CREATE TABLE t1(a);
INSERT INTO t1 VALUES(1);
ALTER TABLE t1 ADD COLUMN b DEFAULT x'9e2e';
CREATE TABLE t2(a, b);
INSERT INTO t2 VALUES(1, x'9e2e');
SELECT unicode(CAST(b AS TEXT)), length(CAST(b AS TEXT)) FROM t1;
SELECT unicode(CAST(b AS TEXT)), length(CAST(b AS TEXT)) FROM t2;
SELECT unicode(min(CAST(b AS TEXT))) FROM t1;
SELECT unicode(min(CAST(b AS TEXT))) FROM t1 GROUP BY a;
SELECT CAST(b AS TEXT) = CAST(x'9e2e' AS TEXT) FROM t1;
```

Output from trunk (check-in dabff5c1cde6, 2026-10-14 16:11), built from the source tarball with default options. 3.54.0 and 3.53.4 give the same:

```
158|2
11934|1
158
11934
0
```

Expected:

```
11934|1
11934|1
11934
11934
1
```

lang_expr.html says: "To cast a BLOB value to TEXT, the sequence of bytes that make up the BLOB is interpreted as text encoded using the database encoding." x'9e2e' in UTF-16LE is U+2E9E, which is what t2's stored copy of the same blob gives. For t1's row, written before the ALTER, b's value is the column default, and that is decoded as UTF-8 instead: U+009E followed by '.'. `upper(b)` and `b LIKE '⺞'` are wrong the same way. `hex(b)`, `quote(b)` and `b || ''` are right. The GROUP BY query is right because the value has passed through a sorter, so the answer depends on the plan. A row inserted after the ALTER is right, and so is t1's row once something writes it (`UPDATE t1 SET a = a`). UTF-16BE is affected too (x'0b0b' comes back as U+000B, not U+0B0B). A UTF-8 database is consistent.

**Cause, as far as we can tell.** The column default is made by `valueFromExpr()` in vdbemem.c. Its TK_BLOB branch calls

```c
sqlite3VdbeMemSetStr(pVal, sqlite3HexToBlob(db, zVal, nVal), nVal/2,
                     0, SQLITE_DYNAMIC);
```

With encoding 0, `sqlite3VdbeMemSetStr()` sets `pVal->enc` to SQLITE_UTF8, not to the `enc` that `valueFromExpr()` was given. When a record is shorter than the table, OP_Column copies that default without changing `enc`, and `sqlite3VdbeMemCast()` then converts the bytes from UTF-8. Setting the encoding after the call fixes every query above:

```diff
--- a/src/vdbemem.c
+++ b/src/vdbemem.c
@@ -1930,6 +1930,7 @@
     assert( zVal[nVal]=='\'' );
     sqlite3VdbeMemSetStr(pVal, sqlite3HexToBlob(db, zVal, nVal), nVal/2,
                          0, SQLITE_DYNAMIC);
+    pVal->enc = enc;
   }
 #endif
 #ifdef SQLITE_ENABLE_STAT4
```

With it, the alter*, altercol, enc*, cast, func2, func3, blob, utf16align and default tests we ran pass. Take it as a pointer to the cause rather than a proposed fix. It looks like the same kind of problem as the overflow-page blob in [forum post 48c64d8bab](https://sqlite.org/bugs/forumpost/48c64d8bab), fixed by check-in a8895cb047.

**Versions** (official amalgamations or tarballs, default options): 3.20.0, 3.25.0, 3.28.0 and 3.30.0 give the wrong results above. 3.31.0, 3.33.0, 3.35.0, 3.37.0 and 3.38.0 give the expected ones. 3.39.0, 3.40.0, 3.53.4, 3.54.0 and trunk dabff5c1cde6 give the wrong ones again, so it seems to have come back in 3.39.0.

We found it with a property-based test generator for SQLite databases and queries: the same query over a UTF-16 table with an added column returned different text depending on whether it used an index or an ORDER BY.
