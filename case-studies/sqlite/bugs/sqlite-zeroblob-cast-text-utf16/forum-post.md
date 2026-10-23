**Title:** In a UTF-16 database, a zeroblob() result used as text is converted from UTF-8, so CAST(zeroblob(4) AS TEXT) has 8 bytes and the result depends on the query plan

```sql
PRAGMA encoding = 'UTF-16le';
CREATE TABLE t1(b);
INSERT INTO t1 VALUES (zeroblob(4));
SELECT hex(CAST(zeroblob(4) AS TEXT));
SELECT hex(CAST(x'00000000' AS TEXT));
SELECT hex(CAST(b AS TEXT)) FROM t1;
WITH x AS MATERIALIZED (SELECT zeroblob(4) AS b FROM t1) SELECT hex(CAST(b AS TEXT)) FROM x;
CREATE TABLE t2(c);
INSERT INTO t2 VALUES ('ab'), ('abc');
WITH x AS (SELECT zeroblob(length(c)) AS z FROM t2) SELECT count(*) FROM (SELECT 1 FROM x GROUP BY upper(z));
WITH x AS MATERIALIZED (SELECT zeroblob(length(c)) AS z FROM t2) SELECT count(*) FROM (SELECT 1 FROM x GROUP BY upper(z));
```

Output from trunk (check-in d6f1df8c0b, 2026-10-23 00:49 UTC), built from the source tarball with `--enable-fts5 --enable-rtree --enable-geopoly`. 3.54.0 gives the same:

```
0000000000000000
00000000
00000000
00000000
2
1
```

Expected:

```
00000000
00000000
00000000
00000000
1
1
```

sqlite3.h says that `sqlite3_result_zeroblob(C,N)` sets the result to "a BLOB containing all zero bytes and N bytes in size", and a BLOB cast to TEXT is read as text in the database encoding. So zeroblob(4) should give the same 4 bytes (2 UTF-16 NUL characters) as the literal `x'00000000'`, the same value stored in a table, and the same value read back from a MATERIALIZED CTE. Only the value taken straight from zeroblob() becomes 8 bytes. upper() sees the same difference: zeroblob(2) and zeroblob(3) become 2 and 3 NUL characters (2 groups) straight from zeroblob(), but 1 NUL character each after going through the CTE's ephemeral table (1 group). With all optimizations off (`.testctrl optimizations 0xffffffff`), the first GROUP BY query also gives 1.

The same happens with UTF-16be. `upper(zeroblob(4))` and `lower(zeroblob(4))` give 8 bytes where `upper(x'00000000')` gives 4, `CAST(zeroblob(4) AS TEXT) = CAST(x'00000000' AS TEXT)` is 0, octet_length() gives 8 and 4, and `count(DISTINCT CAST(z AS TEXT))` over zeroblob(4) and x'00000000' is 2. `sqlite3_bind_zeroblob()` has the same problem: in a small C program, `SELECT hex(CAST(?1 AS TEXT))` gives 0000000000000000 with `sqlite3_bind_zeroblob(st, 1, 4)` and 00000000 with `sqlite3_bind_blob()` of four zero bytes. Not affected: UTF-8 databases, `CAST(unhex('41004200') AS TEXT)` (41004200, 'AB'), `CAST(randomblob(4) AS TEXT)` (4 bytes), and concatenating zeroblob(4) with an empty string (4 bytes).

**Cause, as far as we can tell.** `sqlite3VdbeMemSetZeroBlob()` (src/vdbemem.c) tags the value as UTF-8:

```c
  pMem->u.nZero = n;
  pMem->enc = SQLITE_UTF8;
```

and `sqlite3_result_zeroblob64()` and `sqlite3_bind_zeroblob()` (src/vdbeapi.c) leave it at that. The other places that make a BLOB set `enc` to the database encoding afterwards: `OP_Blob` does `pOut->enc = encoding;` after the same call, `setResultStrOrError()` calls `sqlite3VdbeChangeEncoding(pOut, pCtx->enc)`, which only sets `enc` for a non-string, and `bindText()` does `if( encoding==0 ) pVar->enc = ENC(p->db);`. When the zeroblob value is later used as text, `sqlite3VdbeMemCast()` sets MEM_Str and calls `sqlite3VdbeChangeEncoding(pMem, encoding)`, and `sqlite3_value_text()` in upper() does the same conversion. Both see `enc` UTF-8 and translate the bytes from UTF-8, so each zero byte becomes one UTF-16 character. A value that went through a record (a table or the CTE's ephemeral table) is read back with the database encoding, which is why the plan changes the result.

The patch below sets `enc` in both functions:

```diff
--- a/src/vdbeapi.c
+++ b/src/vdbeapi.c
@@ -691,6 +691,7 @@
   }
 #ifndef SQLITE_OMIT_INCRBLOB
   sqlite3VdbeMemSetZeroBlob(pCtx->pOut, (int)n);
+  pOut->enc = pCtx->enc;
   return SQLITE_OK;
 #else
   return sqlite3VdbeMemSetZeroBlob(pCtx->pOut, (int)n);
@@ -2017,6 +2018,7 @@
 #else
     rc = sqlite3VdbeMemSetZeroBlob(&p->aVar[i-1], n);
 #endif
+    p->aVar[i-1].enc = ENC(p->db);
     sqlite3_mutex_leave(p->db->mutex);
   }
   return rc;
```

With it, the repro gives the expected output, and every variant above, including the bound zeroblob, gives the same result as the literal. These test files gave the same results with and without the patch: zeroblob, zeroblobfault, func, func2, func4, cast, enc, enc2, enc3, enc4, utf16align, bind, bind2, bindxfer, capi3, capi3b to capi3e, incrblob, incrblob2, incrblob3, incrblob_err, e_blobopen, misc1, misc8, tkt3762, tkt3918, distinct, values, table, basexx1 and sqllimits1. All pass except 9 cases in func.test (func-38.100, func-39.120 to func-39.190), which fail in the same way without the patch: our Tcl 8.5 prints "inf" where the test expects "Inf". We have not run the full test suite, and the SQLITE_OMIT_INCRBLOB branch of `sqlite3_result_zeroblob64()` is not changed.

The bytes are all zero, so only the number of NUL characters differs. The documentation for NUL Characters In Strings says that "the use of NUL within strings can lead to surprising behaviors", so this may not be worth changing. What seemed worth reporting is that the same expression gives a different result depending on the query plan (the two GROUP BY queries above), which is not one of the behaviors listed there.

**Versions:** trunk d6f1df8c0b, 3.54.0, 3.53.4, 3.51.0, 3.50.4, 3.40.0, 3.39.0, 3.38.0, 3.37.0, 3.35.0, 3.33.0 and 3.31.0 give 0000000000000000 for the first query. 3.20.0, 3.25.0, 3.28.0 and 3.30.0 give 00000000, but `upper(zeroblob(4))` gives 8 bytes on every version we tried, back to 3.20.0. The GROUP BY pair gives 2 and 1 from 3.39.0. In 3.35.0, 3.37.0 and 3.38.0 the MATERIALIZED CTE also gives 0000000000000000 and both GROUP BY queries give 2. We have not looked at what changed in 3.31.0.

We found it with a property-based test generator for SQLite databases and queries, in UTF-16 databases: the same query gave different results with a CTE inlined and materialized, and with all optimizations on and off.
