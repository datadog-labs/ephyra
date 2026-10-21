**Title:** An OR term that uses a partial index WHERE a=5 and also tests a fails with "internal query planner error"

```sql
CREATE TABLE t(a INT, b);
CREATE INDEX i ON t(b) WHERE a=5;
INSERT INTO t VALUES(5,0),(1,0),(5,1);
SELECT rowid, a, b FROM t WHERE b=0 AND a=5 AND a>1 OR rowid=2;
```

Output from trunk (check-in dc388e83b05d, 2026-10-10 19:19), built from the source tarball with default options. 3.54.0 and 3.53.4 give the same:

```
Parse error near line 4: internal query planner error (2)
```

Expected `1|5|0` and `2|1|0`: row 1 satisfies the first OR term, and row 2 satisfies `rowid=2`. The same query with `FROM t NOT INDEXED` returns exactly that, and so does 3.46.0.

The table can be empty: without the INSERT, `SELECT * FROM t WHERE b=0 AND a=5 AND a=1 OR rowid=0` fails the same way. UPDATE and DELETE with these WHERE clauses fail too. Turning off every optimization with `.testctrl optimizations -All` does not help.

It fails whenever the OR term that uses `i` reads `a` again besides `a=5`: `a>1`, `a=1`, a bare `a` and `a IN (1,5)` all fail. It also fails with `b=0` outside the OR (`WHERE b=0 AND (a=5 AND a>1 OR rowid=2)`), with `a` declared TEXT, REAL or NUMERIC, in a STRICT table, and in a join. It does not fail, and gives the right rows, in these cases:
- `a` has no declared type;
- `a` is in the index (`ON t(b,a) WHERE a=5`);
- the OR term reads nothing but `a=5`;
- the statement also reads a column of `t` that is not in `i`;
- `t` is a WITHOUT ROWID table.

So it seems to need `i` to look like a covering index for the whole statement.

**Cause, as far as we can tell.** `whereLoopAddBtree()` plans the OR term as a sub-WHERE with WHERE_OR_SUBCLAUSE. There it calls `wherePartIdxExpr(..., &m, 0, 0)`, which clears `a` from the mask of columns that `i` lacks, because the index's WHERE implies `a=5`. That substitution was added in 3.44.0 (check-in 8d4160910d). The loop therefore gets WHERE_IDX_ONLY. But `sqlite3WhereBegin()` registers the matching a→5 substitution in `Parse.pIdxPartExpr` only in its last branch, the one that opens a new index cursor. An OR term takes the `iAuxArg && (wctrlFlags & WHERE_OR_SUBCLAUSE)!=0` branch instead (OP_ReopenIdx on the shared cursor). So `a>1` is coded as an OP_Column on the table cursor. `sqlite3WhereEnd()` cannot move that column to the index cursor, and since check-in f0b671183f (3.46.1) it reports this as "internal query planner error". `whereIsCoveringIndex()` already returns 0 for an OR term (`pSelect==0` there). The partial-index path has no such guard.

In 3.44.0 to 3.46.0 these statements return the right rows. An OR term seeks the table row anyway, so the column is read from there. EXPLAIN QUERY PLAN in those releases already shows the OR term as `SEARCH t USING COVERING INDEX i (b=?)`.

The patch below skips the substitution for OR terms when computing the mask. With it, every statement above gives the right answer. The OR term's plan becomes `SEARCH t USING INDEX i (b=?)`, while `SELECT a, b FROM t WHERE b=0 AND a=5` still uses `COVERING INDEX i`:

```diff
--- src/where.c
+++ src/where.c
@@ -4195,9 +4195,11 @@
       }else{
         m = pSrc->colUsed & pProbe->colNotIdxed;
-        if( pProbe->pPartIdxWhere ){
+        if( pProbe->pPartIdxWhere
+         && (pWInfo->wctrlFlags & WHERE_OR_SUBCLAUSE)==0
+        ){
           wherePartIdxExpr(
               pWInfo->pParse, pProbe, pProbe->pPartIdxWhere, &m, 0, 0
           );
         }
```

The veryquick tests give the same results with the patch as without it. We have only checked it against these statements and some generated ones, so take it as a pointer to the cause rather than a proposed fix.

A separate report, "A partial index WHERE col = <constant> makes col read back as the constant: a TEXT '5.0' comes back as '5'", covers wrong values from the same substitution.

**Versions** (official amalgamations or tarballs, default options):
- 3.8.0 (the first with partial indexes), 3.25.0, 3.35.0, 3.40.0, 3.42.0 and 3.43.0 return the right rows.
- 3.44.0, 3.45.0, 3.45.3 and 3.46.0 also return the right rows, with the COVERING INDEX plan.
- 3.46.1, 3.47.0, 3.49.0, 3.50.0, 3.51.0, 3.53.4, 3.54.0 and trunk dc388e83b05d fail with the error.

We found it with a property-based test generator for SQLite databases and queries: a generated join on an empty table, with a partial index on `c0+1` `WHERE c0 = 3.5`, failed to prepare with this error.
