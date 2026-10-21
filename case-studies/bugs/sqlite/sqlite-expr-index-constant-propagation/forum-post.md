**Title:** Join drops rows, or adds some, when a table has an index on an expression and a `col = constant` WHERE term

```sql
CREATE TABLE t(a INT, b);
INSERT INTO t VALUES(0, 'old');
CREATE INDEX i ON t(a+1);
SELECT count(*) FROM (SELECT 1), t WHERE a+1 = (SELECT 1) AND a = 0;
UPDATE t SET b = 'new' FROM (SELECT 1) WHERE a+1 = (SELECT 1) AND a = 0;
SELECT b FROM t;
```

Output from trunk (check-in dc388e83b05d, 2026-10-10 19:19), built from the source tarball with default options:

```
0
old
```

Expected `1` and `new`. The one row has a = 0, so a+1 = 1 and both WHERE terms are true. Trunk does give `1` and `new` with `t NOT INDEXED` in both statements, or with `.testctrl optimizations -PropagateConst` or `-IndexedExpr`. 3.54.0 and 3.53.4 give the same wrong results.

Rows can be added as well as lost. Here INDEXED BY forces a covering scan of the index:

```sql
CREATE TABLE u(a INTEGER PRIMARY KEY);
INSERT INTO u VALUES(1),(2);
CREATE INDEX ui ON u(a+1);
SELECT * FROM u AS x CROSS JOIN u AS y INDEXED BY ui WHERE x.a <> y.a+1 AND y.a = 1;
SELECT * FROM u AS x CROSS JOIN u AS y INDEXED BY ui WHERE x.a + (y.a+1) IS NULL AND y.a = 1;
```

The first query returns no rows, but it should return `1|1` (x.a <> 2). The second returns `1|1` and `2|1`, but it should return none (x.a + 2 is never NULL). With `NOT INDEXED` in place of `INDEXED BY ui`, both are right.

Other forms that fail: any indexed expression of the column (we also tried `CAST(c AS INTEGER)` on a TEXT column holding `''`, `abs(a)`, `lower(a)` and `a->>'k'`); `JOIN`, `CROSS JOIN` or `JOIN ... ON <both terms>` in place of the comma; `IN (SELECT 1)` in place of `= (SELECT 1)`; a VALUES list in place of the subquery; and a table in place of the subquery when INDEXED BY forces the index. Forms that give the right answer: a single-table query, a constant right side (`a+1 = 1`, `a+1 IN (1,2)`), a column declared without a type, and a LEFT JOIN with both terms in its ON clause.

**Cause, as far as we can tell.** `propagateConstants()` sees `a = 0` and tags the `a` inside `a+1` as EP_FixedCol. `sqlite3WhereExprUsage()` ignores EP_FixedCol columns, so the term `a+1 = (SELECT 1)` depends on no table and is coded in the outer loop (over the subquery). The same term also drives the SEARCH on index `i` in the inner loop, so `a+1` is in `pParse->pIdxEpr`. When the outer-loop copy of the term is coded, `sqlite3IndexedExprLookup()` matches `a+1` with the index column, because `sqlite3ExprCompare()` does not compare the pLeft of an EP_FixedCol node. It then reads the value from the index cursor, which the inner loop has not positioned yet:

```
21  Yield    1   43  0   next row of (subquery-1)
22  Column   2   0   10  r[10]=i expr-column 0      <- index i, not positioned yet
23  IsNull   10  42  0   if r[10]==NULL goto 42
...
35  SeekGE   2   42  15  key=r[15]                  <- index i is positioned here
```

The value read is NULL. That makes the term false, or true in the IS NULL query. In the second example, `x.a <> y.a+1` depends only on x and is coded in x's loop, before the covering scan of `ui` starts. The first example fails from check-in 44200596aa943963 on. Before it, `exprMightBeIndexed()` gave up on a term that references no table, and the plan was a plain `SCAN t`. From that check-in, the plan is `SEARCH t USING INDEX i (<expr>=?)`.

If `sqlite3IndexedExprLookup()` skips the substitution when the expression contains an EP_FixedCol column, every query above gives the expected answer:

```c
static int exprNodeIsFixedCol(Walker *pWalker, Expr *pExpr){
  if( pExpr->op==TK_COLUMN && ExprHasProperty(pExpr, EP_FixedCol) ){
    pWalker->eCode = 1;
    return WRC_Abort;
  }
  return WRC_Continue;
}
static int exprHasFixedCol(Expr *pExpr){
  Walker w;
  memset(&w, 0, sizeof(w));
  w.xExprCallback = exprNodeIsFixedCol;
  sqlite3WalkExpr(&w, pExpr);
  return w.eCode;
}
...
    if( sqlite3ExprCompare(0, pExpr, p->pExpr, iDataCur)!=0 ) continue;
    if( exprHasFixedCol(pExpr) ) continue;    /* new */
```

The veryquick test files we could run show no new failures (our Tcl is 8.5, so mjournal.test and a few others could not run). But this gives up the index value even where the cursor is positioned, so it is probably broader than needed. Take it as a pointer to the cause rather than a proposed fix.

**Versions** (official amalgamations or tarballs, default options):

- The first example gives `0` and `old` in 3.41.0, 3.42.0, 3.43.0, 3.44.0, 3.45.0, 3.45.3, 3.46.0, 3.46.1, 3.47.0, 3.48.0, 3.49.0, 3.50.0, 3.51.0, 3.53.4, 3.54.0 and trunk dc388e83b05d.
- It gives `1` and `new` in 3.35.0, 3.36.0, 3.37.0, 3.38.0, 3.39.0, 3.39.4, 3.40.0 and 3.40.1, and the SELECT alone gives `1` in every older release we tried, back to 3.9.0.
- Built from source, check-in 44200596aa943963 (2023-02-10) gives the wrong answer, and its parent 00714b39b3 the right one.
- The second example is wrong from 3.40.0 to trunk. It is right in 3.35.0 to 3.39.4. The older releases we tried (3.31.0 and before) reject the INDEXED BY with "no query solution".
- On 3.40.0 and 3.41.0, turning off the IndexedExpr optimization (`.testctrl optimizations 0x01000000`) makes both examples right.

We found it with a property-based test generator for SQLite databases and queries: a join with a condition in its WHERE clause returned fewer rows than a count of the rows of the same join where that condition is true, and adding INDEXED BY on the expression index changed another query's rows.
