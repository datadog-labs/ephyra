**Title:** FTS5: MATCH with `rowid = <non-integer value>` returns every match (regression after 3.51)

```sql
CREATE VIRTUAL TABLE ft USING fts5(a);
INSERT INTO ft(rowid, a) VALUES (1, 'apple'), (2, 'apple'), (3, 'apple pie');

SELECT count(*) FROM ft WHERE ft MATCH 'apple' AND rowid = 2.5;
SELECT count(*) FROM ft WHERE ft MATCH 'apple' AND rowid = 'x';
SELECT group_concat(rowid) FROM ft WHERE ft MATCH 'apple' AND rowid = 2.0;

CREATE TABLE t1(x);
INSERT INTO t1 VALUES (3), (2.5), ('x');
SELECT t1.x, ft.rowid FROM t1 LEFT JOIN ft ON ft.rowid = t1.x AND ft MATCH 'pie';
SELECT count(*) FROM ft WHERE ft MATCH 'apple' AND rowid IN (2.5, 'x');
```

Output from trunk (check-in 185ffd27064b, 2026-10-20 13:10), built from the source tarball with --enable-fts5. 3.53.4 gives the same; 3.51.0 is right:

```
3
3
1,2,3
3|3
2.5|3
x|3
6
```

Expected, and what 3.51.0 prints:

```
0
0
2
3|3
2.5|
x|
0
```

The rowid is an integer, and the comparison rules in datatype3.html make `rowid = 2.5` and `rowid = 'x'` false for every row: the rowid's INTEGER affinity leaves 2.5 a REAL and 'x' a TEXT, and neither is equal to an integer. 2.0 is equal to the integer 2. Without MATCH, the same constraints give 0, 0 and 2. With MATCH, a value that is not an integer (a REAL, even 2.0, a TEXT that does not look like an integer, NULL, a BLOB) matches every row the MATCH matches: in the join, every row of t1 that is not an integer joins the MATCH rows, and an IN list (or `rowid IN (SELECT ...)`) returns each match once per such value. The same happens with ORDER BY rowid DESC or ORDER BY rank, with `rowid = 2.0 AND rowid > 1.5`, with trigram LIKE/GLOB instead of MATCH, and in contentless, contentless_delete, external-content, detail=none/column and tokendata=1 tables. Integers and texts that numeric affinity turns into an integer ('2', ' 2 ') are fine ('2.0' and '2e0' are not), and so are the range constraints (`rowid <= 2.5` gives 1,2).

**Cause, as far as we can tell.** `fts5BestIndexMethod()` now sets `aConstraintUsage[].omit` for the `rowid = ?` constraint (the `bSeenEq==0 && p->op==SQLITE_INDEX_CONSTRAINT_EQ && iCol<0` case); 3.50.2's code does not (and 3.51.0 behaves as without it), so SQLite used to test the equality again on each row. With MATCH, `fts5FilterMethod()` does not look the rowid up but limits the full-text query to `iFirstRowid..iLastRowid`, both set by `fts5GetRowidLimit()`, which uses the value only if `sqlite3_value_numeric_type()` is SQLITE_INTEGER and otherwise returns the default, the whole rowid range. The comment above that code says this is fine because the omit flag stays clear for range constraints on the rowid; with `=` it is now set. (Without MATCH, the FTS5_PLAN_ROWID plan binds the value to `... WHERE rowid=?`, which compares it correctly.)

The patch below makes the `rowid = ?` limits exact, the integer the value is equal to or an empty range if there is none, and gives the expected output above:

```diff
--- a/ext/fts5/fts5_main.c
+++ b/ext/fts5/fts5_main.c
@@ -1298,6 +1298,31 @@
 }
 
 /*
+** Argument pVal is the right-hand side of a "rowid = ?" constraint. If
+** there is an integer that compares equal to pVal, set *piRowid to it and
+** return 1. Otherwise - pVal is NULL, a blob, a non-numeric text value or
+** a real value that is not an integer - return 0.
+*/
+static int fts5RowidEqValue(sqlite3_value *pVal, i64 *piRowid){
+  int eType = sqlite3_value_numeric_type(pVal);
+  if( eType==SQLITE_INTEGER ){
+    *piRowid = sqlite3_value_int64(pVal);
+    return 1;
+  }
+  if( eType==SQLITE_FLOAT ){
+    double r = sqlite3_value_double(pVal);
+    if( r>=-9223372036854775808.0 && r<9223372036854775808.0 ){
+      i64 iVal = (i64)r;
+      if( (double)iVal==r ){
+        *piRowid = iVal;
+        return 1;
+      }
+    }
+  }
+  return 0;
+}
+
+/*
 ** Set the error message on the virtual table passed as the first argument.
 */
 static void fts5SetVtabError(Fts5FullTable *p, const char *zFormat, ...){
@@ -1572,11 +1597,18 @@
   /* Set the cursor upper and lower rowid limits. Only some strategies 
   ** actually use them. This is ok, as the xBestIndex() method leaves the
   ** sqlite3_index_constraint.omit flag clear for range constraints
-  ** on the rowid field.  */
+  ** on the rowid field. It sets the omit flag for "rowid = ?" though, so
+  ** in that case the limits must select exactly the rowid equal to the
+  ** value, or no rowid at all if no integer is equal to it.  */
   if( pRowidEq ){
-    pRowidLe = pRowidGe = pRowidEq;
-  }
-  if( bDesc ){
+    i64 iEq;
+    if( fts5RowidEqValue(pRowidEq, &iEq) ){
+      pCsr->iFirstRowid = pCsr->iLastRowid = iEq;
+    }else{
+      pCsr->iFirstRowid = bDesc ? SMALLEST_INT64 : LARGEST_INT64;
+      pCsr->iLastRowid = bDesc ? LARGEST_INT64 : SMALLEST_INT64;
+    }
+  }else if( bDesc ){
     pCsr->iFirstRowid = fts5GetRowidLimit(pRowidLe, LARGEST_INT64);
     pCsr->iLastRowid = fts5GetRowidLimit(pRowidGe, SMALLEST_INT64);
   }else{
```

It keeps `omit` set, which looks deliberate (see below), so the comparison has to follow SQLite's rules: `sqlite3_value_numeric_type()` applies the same numeric affinity to texts, and a REAL is equal to an integer only if it is integral and within [-2^63, 2^63). The one-line alternative is to drop `omit = 1` for `rowid =` again and let SQLite re-check each row; that is also correct, at the cost of the re-check and of a full MATCH scan for values like 2.0.

This is offered as a fix. The full patch, including regression tests appended to ext/fts5/test/fts5plan.test (396 new cases), is in the attached sqlite-fts5-match-rowid-eq-nonint.patch.

Without the patch 230 of fts5plan.test's 403 cases fail; with it none do, and 195 files (all of ext/fts5/test including the fault-injection tests, plus test/vtab\*, tabfunc\* and rowid\*) and veryquick show no new failures (only the pre-existing fts5contentless3, fts5faultI and, in veryquick, the same 29 errors as without the patch); check.sql and the new tests run clean under ASan/UBSan. All four fixes from this batch were also applied together to trunk 185ffd27064b; every new test passes and veryquick shows no new failures.

**Versions** (FTS5 builds; our older release CLIs are built without FTS5): right in 3.46.0, 3.50.2 (amalgamations built with -DSQLITE_ENABLE_FTS5) and 3.51.0 (the macOS /usr/bin/sqlite3, dated 2025-06-12); wrong in 3.53.2 (amalgamation), 3.53.4 (Homebrew) and trunk. The `omit` assignment is in 3.53.2's fts5BestIndexMethod() and not in 3.50.2's, so it came in after 3.51, together with the rewrite of that function's cost model (the new estimatedCost values, `fts5SetEstimatedRows()`, and `fts5SetUniqueFlag()` now also with MATCH); we have no build in between to narrow it down.

We found it with a property-based test generator for SQLite databases and queries: a FULL JOIN on rowid between `SELECT rowid AS c0 ... FROM vt0('zeta')` and a table whose c0 held reals and texts returned 394 rows instead of 42.
