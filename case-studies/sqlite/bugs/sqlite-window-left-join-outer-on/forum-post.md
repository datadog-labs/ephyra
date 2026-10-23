**Title:** With a window function, a correlated LEFT JOIN whose ON clause has a bare outer column as an AND term filters the left rows

```sql
CREATE TABLE t(c);
INSERT INTO t VALUES (NULL);
SELECT count(*) OVER (), (SELECT count(*) FROM t AS b LEFT JOIN t AS d ON a.c) FROM t AS a;
```

Output from trunk (check-in 07095b76c1f0, 2026-10-08 13:26), built from the source tarball with default options:

```
1|0
```

Expected `1|1`: a LEFT JOIN keeps every row of its left operand `b` whatever its ON clause, so the subquery counts one row (`b`'s, NULL-extended). Without the window function the same subquery gives 1:

```sql
SELECT (SELECT count(*) FROM t AS b LEFT JOIN t AS d ON a.c) FROM t AS a;  -- 1
```

It needs the outer column alone as the ON clause or as one of its AND terms: `ON a.c`, `ON NULL AND a.c`, `ON a.c AND 1` and `ON a.c AND d.c IS NULL` give 0; `ON NOT a.c`, `ON a.c OR 0`, `ON a.c IS 1`, `ON a.c = 5`, `ON abs(a.c)`, `ON a.c > 0`, `ON d.c = a.c` and `ON 0` all give 1. Any window function in the outer query will do (`row_number() OVER ()`, `max(c) OVER ()`), and so will EXISTS in place of `count(*)` (`EXISTS (SELECT 1 FROM t AS b LEFT JOIN t AS d ON a.c)` is 0 with the window, 1 without). So the bare outer column, which the window rewrite turns into a reference to the window's input, seems to stop being an ON term of the LEFT JOIN and filter `b`'s row like a WHERE term.

**Cause, as far as we can tell.** `selectWindowRewriteExprCb` rewrites each reference to the outer query's columns, including `a.c` inside the subquery, into a TK_COLUMN on the window's ephemeral table. It clears the node and keeps only `EP_Collate`:

```c
int f = pExpr->flags & EP_Collate;
...
memset(pExpr, 0, sizeof(Expr));
...
pExpr->flags = f;
```

That also drops `EP_OuterON` and `w.iJoin`. When the bare column is a whole AND term of the ON clause, it is that term's root, so the term stops being an ON term of the LEFT JOIN and acts as a WHERE term. In `a.c = 5`, `abs(a.c)` and the like, the ON marking is on the term's root (`=`, the function), which the rewrite leaves alone, so those are correct.

Keeping the two fields makes every failing form above give the right answer (`1|1`, and 1 for the EXISTS form), and leaves the correct ones unchanged:

```c
int f = pExpr->flags & (EP_Collate|EP_OuterON|EP_InnerON);
int iJoin = ExprHasProperty(pExpr, EP_OuterON|EP_InnerON) ? pExpr->w.iJoin : 0;
...
pExpr->flags = f;
if( f & (EP_OuterON|EP_InnerON) ) pExpr->w.iJoin = iJoin;
```

We have only checked it against these queries, so take it as a pointer to the cause rather than a proposed fix.

**Versions** (official amalgamations or tarballs, default options): 3.25.0 (the first with window functions), 3.30.0, 3.35.0, 3.39.0, 3.42.0, 3.45.0, 3.53.4 and trunk eaec2f7f3744 and 07095b76c1f0 all give `1|0`.

We found it with a property-based test generator for SQLite databases and queries: the window function's input, computed without the window, disagreed with the window query on the subquery's value.
