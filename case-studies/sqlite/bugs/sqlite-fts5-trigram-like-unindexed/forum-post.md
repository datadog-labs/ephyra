**Title:** FTS5 trigram: LIKE and GLOB on an UNINDEXED column return no rows

```sql
CREATE VIRTUAL TABLE t1 USING fts5(a, b UNINDEXED, tokenize=trigram);
INSERT INTO t1 VALUES('xyz', 'abcdef');
SELECT count(*) FROM t1 WHERE b LIKE '%bcd%';
SELECT count(*) FROM t1 WHERE b GLOB '*bcd*';
SELECT count(*) FROM t1 WHERE +b LIKE '%bcd%';
```

Output from trunk (check-in d6f1df8c0bbc, 2026-10-23) built with `--enable-fts5`:

```
0
0
1
```

Expected: 1 for all three. The third query, where the unary `+` keeps the LIKE away from the virtual table, shows that the row matches.

`fts5BestIndexMethod()` in ext/fts5/fts5_main.c passes a LIKE or GLOB on any column to xFilter as a trigram lookup (`L1` in `EXPLAIN QUERY PLAN`) without checking whether the column is UNINDEXED. An UNINDEXED column has no tokens in the index, so the lookup matches no row.

The patch below leaves LIKE and GLOB on an UNINDEXED column to SQLite:

```diff
--- a/ext/fts5/fts5_main.c
+++ b/ext/fts5/fts5_main.c
@@ -675,7 +675,9 @@
         pInfo->aConstraintUsage[i].omit = 1;
       }
     }else if( p->usable ){
-      if( iCol>=0 && iCol<nCol && fts5UsePatternMatch(pConfig, p) ){
+      if( iCol>=0 && iCol<nCol && fts5UsePatternMatch(pConfig, p)
+       && pConfig->abUnindexed[iCol]==0
+      ){
         assert( p->op==FTS5_PATTERN_LIKE || p->op==FTS5_PATTERN_GLOB );
         idxStr[iIdxStr++] = p->op==FTS5_PATTERN_LIKE ? 'L' : 'G';
         sqlite3_snprintf(6, &idxStr[iIdxStr], "%d", iCol);
```

**Versions:** trunk, 3.53.2, 3.51.0, 3.50.2 and 3.46.0 (the oldest build with FTS5 that we tried).
