**Title:** FTS5: "no such cursor: 0" from an auxiliary function in WHERE with the `tbl = 'query'` form (constant propagation)

```sql
CREATE VIRTUAL TABLE ft USING fts5(a);
INSERT INTO ft(rowid, a) VALUES (1, 'one apple'), (2, 'apple pie');

SELECT rowid FROM ft WHERE ft = 'apple' AND snippet(ft, 0, '[', ']', '..', 4) IS NOT NULL;
SELECT rowid FROM ft WHERE ft = 'apple' AND bm25(ft) < 0;
SELECT count(*) FROM ft WHERE ft = 'apple' AND ft = 'pie';
```

Output from trunk (check-in 185ffd27064b, 2026-10-20 13:10), built from the source tarball with --enable-fts5. 3.53.4 and 3.51.0 give the same:

```
Error near line 4: no such cursor: 0
Error near line 5: no such cursor: 0
0
```

Expected: the same as with `ft MATCH 'apple'` (and `ft MATCH 'pie'`), which gives

```
1
2
1
2
1
```

The FTS5 documentation lists ft = 'apple' as a full-text query, like ft MATCH 'apple' and ft('apple'), and with those two forms all three statements above work. With =, any auxiliary function in the WHERE clause (snippet, highlight, bm25 or a custom one) fails, and a second ft = '...' term is false. rank = 'bm25(10.0)' is affected the same way, and a CTE that uses both forms, joined to itself, fails with "unable to use function MATCH in the requested context". Auxiliary functions in the result columns, and = on an ordinary FTS5 column, are fine.

**Cause, as far as we can tell.** WHERE-clause constant propagation (`propagateConstants()`, select.c) takes `ft = 'apple'` as a COLUMN=CONSTANT term and replaces the other references to the column `ft` in the WHERE clause with 'apple' (EP_FixedCol). FTS5 declares that column as `ft HIDDEN` (fts5_config.c), and the declared type "HIDDEN" gives it NUMERIC affinity, so `bHasAffBlob` is clear and the substitution goes into function arguments as well (EXPLAIN shows `String8 'apple'`, `Affinity C` before `Function snippet`). The first argument of an auxiliary function is that column, whose value is the FTS5 cursor id. `fts5ApiCallback()` reads it with `sqlite3_value_int64()` and gets 0 from 'apple': "no such cursor: 0". The substitution is wrong in the first place, because `ft = 'apple'` does not mean that the column holds 'apple'. xBestIndex takes it as a full-text query and sets `omit` (`fts5BestIndexMethod()`, the `SQLITE_INDEX_CONSTRAINT_EQ && iCol>=nCol` case), so the column never holds 'apple' and the comparison is never evaluated. More generally, what `=` means for a virtual table column is up to the virtual table once it sets `omit`. The affinity is incidental: with BLOB affinity the function arguments would be left alone, but `ft = 'pie'` and `rank < 0` are comparison operands and would still be rewritten.

The patch below leaves HIDDEN columns of virtual tables out of constant propagation and gives the expected output above:

```diff
--- a/src/select.c
+++ b/src/select.c
@@ -4844,6 +4844,8 @@
 **
 **    *   Do not add if pValue is a function that might return a subtype
 **
+**    *   Do not add if pColumn is a HIDDEN column of a virtual table
+**
 ** The caller guarantees the pColumn is a column.
 */
 static void constInsert(
@@ -4856,6 +4858,12 @@
   assert( pColumn->op==TK_COLUMN );
   assert( sqlite3ExprIsConstant(pConst->pParse, pValue) );
   if( ExprHasProperty(pColumn, EP_FixedCol) ) return;
+  if( ExprIsVtab(pColumn)
+   && pColumn->iColumn>=0
+   && IsHiddenColumn(&pColumn->y.pTab->aCol[pColumn->iColumn])
+  ){
+    return;
+  }
   if( sqlite3ExprAffinity(pValue)!=0 ) return;
   if( !sqlite3IsBinary(sqlite3ExprCompareCollSeq(pConst->pParse,pExpr)) ){
     return;
```

FTS5's table column and rank, and the parameters of table-valued functions, are all HIDDEN. Table-valued function arguments become WHERE terms only in `sqlite3WhereTabFuncArgs()`, after constant propagation, so nothing is lost for them, and ordinary virtual-table columns keep the optimization. Skipping all virtual-table columns is the more conservative alternative; it also fixes this, but then an `=` constraint the virtual table could use is no longer derived from `a=19 AND b=a` (whereL-1000 in the patch checks this).

**Versions**: the same output in every one we tried, 3.46.0, 3.50.2 and 3.53.2 (amalgamations built with -DSQLITE_ENABLE_FTS5), 3.51.0 (the macOS /usr/bin/sqlite3, dated 2025-06-12), 3.53.4 (Homebrew) and trunk.

We found it with a property-based test generator for SQLite databases and queries: a query over a CTE whose result column was `snippet(vt0, ...)`, with the outer WHERE term `vt0 = 'q'` flattened into it, failed with "no such cursor: 0".
