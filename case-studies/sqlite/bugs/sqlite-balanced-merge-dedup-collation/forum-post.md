**Title:** UNION of four or more SELECTs drops rows: the balanced merge removes the right half's duplicates under that half's collation

```sql
CREATE TABLE t0(c0);
CREATE TABLE t1(c0 COLLATE RTRIM);
INSERT INTO t1 VALUES (''), (' ');
.mode quote
SELECT c0 FROM t0 UNION SELECT 1 UNION SELECT 2 UNION SELECT c0 FROM t1;
SELECT c0 FROM t0 UNION SELECT 1 UNION SELECT 2 UNION SELECT c0 FROM t1
 ORDER BY 1;
SELECT 1 COLLATE BINARY UNION SELECT 2 UNION SELECT '' UNION SELECT ' ' COLLATE RTRIM
 EXCEPT SELECT 9;
```

Output from trunk (check-in 5bd093e261a7, 2026-10-16 18:11), built from the source tarball with default options. 3.54.0 gives the same:

```
1
2
''
1
2
''
1
2
''
```

Expected:

```
1
2
''
' '
1
2
''
' '
1
2
''
' '
```

The leftmost SELECT's column, t0.c0, has the default BINARY collation, so the compound's column is BINARY and `''` and `' '` are two rows. With `.testctrl optimizations -BalancedMerge` all three queries give the expected rows. It takes a run of at least four SELECTs joined by UNION (three are not split), with two values that the whole compound keeps apart in its right half, and a collation in that half (from its first SELECT that has one) under which they are equal. The UNION run can be the left operand of an EXCEPT or INTERSECT, as in the third query. If the two values are in different halves (`SELECT '' COLLATE BINARY UNION SELECT 1 UNION SELECT 2 UNION SELECT ' ' COLLATE RTRIM`), or the right half has no UNION (`... UNION SELECT '' COLLATE RTRIM UNION ALL SELECT ' '`), the result is right.

**Cause, as far as we can tell.** `multiSelectByMerge()` (select.c) splits a run of UNION (or UNION ALL) SELECTs into two halves when there are more than three. Before the split, `multiSelectByMergeKeyInfo()` attaches the compound's collation (from `multiSelectCollSeq()` on the whole compound) to the ORDER BY terms, and that ORDER BY is copied to the left half. The right half is coded by `sqlite3Select(pParse, p, &destB)` as a compound of its own, so it goes through `multiSelectByMerge()` again, and there its `pKeyDup` (the KeyInfo for removing duplicates in the output subroutine) is built from `multiSelectCollSeq(pParse, p, i)`. With `pSplit->pPrior` set to 0, that is the collation of the right half's own SELECTs: RTRIM from t1.c0 here (`SELECT 2` has none). The right half's rows come out in BINARY order (its ORDER BY has the COLLATE BINARY attached above), `''` is followed by `' '`, which is equal under RTRIM and is dropped. The outer merge never sees it.

Before 3.53 the first and third queries were right in releases we have (3.50.4, 3.51.0): a UNION, EXCEPT or INTERSECT without ORDER BY was computed with temporary b-trees ("UNION USING TEMP B-TREE"), whose KeyInfo comes from the whole compound. Trunk now codes every compound other than UNION ALL with the merge (`multiSelect()`, "then also always use the merge algorithm", with an invented `ORDER BY 1`), so the balanced merge applies without ORDER BY too. With ORDER BY, the second query has been wrong since the balanced merge appeared (3.38.0).

The same `pKeyDup` makes the left operand of a merge remove its duplicates under its own collation while the merge itself compares under the compound's. In every release we tried (3.20.0 to trunk), continuing the session above:

```sql
CREATE TABLE t(c);
INSERT INTO t VALUES ('a'), ('A');
SELECT 'a' UNION SELECT 'A' UNION ALL SELECT 'x' COLLATE NOCASE ORDER BY 1;
SELECT c || '' FROM t UNION SELECT 'b' UNION ALL SELECT 'x' COLLATE NOCASE ORDER BY 1;
```

gives `'A', 'x'` and then `'a', 'A', 'b', 'x'`. The compound is NOCASE (its first SELECT with a collation is the last one), and `'a'` and `'A'` are merged when they come from different SELECTs but not when they come from one.

Taking the duplicate-removal collation from the ORDER BY terms, which carry the enclosing compound's collation into both halves, gives the expected rows for all five queries (the fourth is already right):

```diff
--- a/src/select.c
+++ b/src/select.c
@@ -3607,9 +3607,25 @@
     pKeyDup = sqlite3KeyInfoAlloc(db, nExpr, 1);
     if( pKeyDup ){
       assert( sqlite3KeyInfoIsWriteable(pKeyDup) );
-      for(i=0; i<nExpr; i++){
-        pKeyDup->aColl[i] = multiSelectCollSeq(pParse, p, i);
+      /* Use the collation of the ORDER BY term on each column: it is the
+      ** collation of the whole compound, also when p is only a part of
+      ** it (one half of a balanced merge, or the left operand of a merge),
+      ** whose ORDER BY was copied from the enclosing compound with the
+      ** compound's collation attached by multiSelectByMergeKeyInfo(). */
+      for(i=0; i<nExpr; i++){
+        pKeyDup->aColl[i] = 0;
         pKeyDup->aSortFlags[i] = 0;
+      }
+      for(i=0; pKeyMerge && i<nOrderBy; i++){
+        int iCol = pOrderBy->a[i].u.x.iOrderByCol - 1;
+        if( iCol<nExpr && pKeyDup->aColl[iCol]==0 ){
+          pKeyDup->aColl[iCol] = pKeyMerge->aColl[i];
+        }
+      }
+      for(i=0; i<nExpr; i++){
+        if( pKeyDup->aColl[i]==0 ){
+          pKeyDup->aColl[i] = multiSelectCollSeq(pParse, p, i);
+        }
       }
     }
   }
```

(An ORDER BY with an explicit COLLATE never reaches this point when the operator is not UNION ALL, since `convertCompoundSelectToSubquery()` moves it to an outer query.) With it, the select*, e_select*, collate* and union* tests pass as before, and veryquick shows no new failures. Take it as a pointer to the cause rather than a proposed fix.

A related change that the diff does not touch: without ORDER BY, the left operand of a UNION ALL is now computed with its own collation, so `SELECT 'a' UNION SELECT 'A' UNION ALL SELECT 'x' COLLATE NOCASE` gives `'A', 'a', 'x'` on trunk, 3.53.4 and 3.54.0 but `'A', 'x'` on 3.51.0 and with `ORDER BY 1`.

**Versions** (official builds, default options):
- First and third queries: right in 3.20.0 to 3.51.0, wrong in 3.53.4, 3.54.0 and trunk.
- Second query (ORDER BY): right in 3.20.0 to 3.37.0, wrong from 3.38.0 to trunk. 3.37.0 merges a four-SELECT UNION as a left-deep chain and 3.38.0 as two halves (EXPLAIN QUERY PLAN). In 3.38.0 to 3.44.0 there is no option to turn the split off; 3.50.4 and later have `-BalancedMerge`.
- The `t` queries: the same output in every release we tried, 3.20.0 to trunk.

We found it with a property-based test generator for SQLite databases and queries: while fixing our own model of a compound's collation (after a finding of its set-operation check), a four-SELECT UNION whose leftmost column was BINARY and whose last column was RTRIM lost a row that differed only in trailing spaces.
