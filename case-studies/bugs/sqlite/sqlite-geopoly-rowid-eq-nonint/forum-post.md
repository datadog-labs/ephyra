**Title:** Geopoly: `rowid = <non-integer value>` finds the row of its integer part

```sql
CREATE VIRTUAL TABLE g USING geopoly();
INSERT INTO g(rowid, _shape) VALUES
  (6, '[[0,0],[1,0],[0,1],[0,0]]'), (7, '[[0,0],[1,0],[0,1],[0,0]]');

SELECT count(*) FROM g WHERE rowid = 6.5;
SELECT count(*) FROM g WHERE rowid = '6abc';
SELECT count(*) FROM g WHERE rowid IN (6.5, 7);

CREATE TABLE t1(x);
INSERT INTO t1 VALUES (6), (6.5), ('7abc');
SELECT t1.x, g.rowid FROM t1 LEFT JOIN g ON g.rowid = t1.x;

DELETE FROM g WHERE rowid = 7.5;
SELECT group_concat(rowid) FROM g;
```

Output from trunk (check-in 185ffd27064b, 2026-10-20 13:10), built from the source tarball with --enable-geopoly. 3.46.0, 3.50.2 and 3.53.2 give the same:

```
1
1
2
6|6
6.5|6
7abc|7
6
```

Expected, and what an ordinary rowid table or an rtree table with the same rows prints:

```
0
0
1
6|6
6.5|
7abc|
6,7
```

The rowid is an integer, and by the comparison rules in datatype3.html `rowid = 6.5` and `rowid = '6abc'` are false for every row: the rowid's INTEGER affinity leaves 6.5 a REAL and '6abc' a TEXT, and neither is equal to an integer. On a geopoly table, the value is truncated to an integer instead and the row with that rowid is returned. This is a wrong result in SELECT and in joins on the rowid (a row of t1 whose x is not an integer joins the row of g whose rowid is that value truncated to an integer, if there is one), and UPDATE and DELETE with such a WHERE clause change or delete a row they should not touch: the DELETE above removes row 7.

Wrong on a geopoly table: a REAL that is not an integer (6.5, 6.000000000000001), a REAL beyond the int64 range (1e19 finds rowid 9223372036854775807, -1e19 finds -9223372036854775808, and 9223372036854775807.0, which is 2^63, finds 9223372036854775807), a text that is not a number ('6abc' finds 6, 'x' and '0x6' find 0, '9223372036854775808' finds 9223372036854775807), a BLOB (x'36' finds 6), and NULL (finds 0). The same values in an IN list and as join keys behave the same way. Integers, integral REALs (6.0, -0.0) and texts that numeric affinity turns into an integer or an integral REAL ('6', ' 6 ', '6.0', '6e0') are right. Texts in exponent form whose leading digits are not the integer ('0.6e1', '1e1') are wrong too: geopoly looks up rowids 0 and 1 instead of 6 and 10.

R-Tree gets almost all of this right. Both tables use the same xBestIndex strategy for `rowid = ?`: idxNum 1 with `aConstraintUsage[].omit = 1`, so SQLite does not check the equality again and xFilter has to find exactly the rows that are equal to the value. rtreeFilter() does this: it looks the rowid up only if `sqlite3_value_numeric_type()` is SQLITE_INTEGER, or SQLITE_FLOAT and `sqlite3IntFloatCompare()` says the REAL equals the integer (rtree1.test 24.2 tests `rid=1.005`). geopolyFilter() has the same rowid-lookup block but without the check: it calls `findLeafNode()` with `sqlite3_value_int64(argv[0])` for any value.

R-Tree's check has one small gap of its own: it takes `sqlite3_value_int64()` of the value before `sqlite3_value_numeric_type()` converts it. For the text '1e1' the integer is 1 (the digits before the 'e'), the converted value is the REAL 10.0, the two compare unequal, and `id = '1e1'` finds no row where an ordinary table finds rowid 10. Ones like '6e0' or '6.0' only work because the digits before the '.' or 'e' happen to be the integer.

The patch below moves R-Tree's check into a helper, takes the integer after the conversion, and uses the helper in both filters:

```diff
--- a/ext/rtree/rtree.c
+++ b/ext/rtree/rtree.c
@@ -1903,6 +1903,20 @@
 }
 
 int sqlite3IntFloatCompare(i64,double);
+
+/*
+** Argument pVal is the right-hand side of a "rowid = ?" constraint. If
+** there is an integer that compares equal to pVal, set *piRowid to it and
+** return 1. Otherwise - pVal is NULL, a blob, a non-numeric text value or
+** a real value that is not an integer - return 0.
+*/
+static int rtreeRowidEqValue(sqlite3_value *pVal, i64 *piRowid){
+  int eType = sqlite3_value_numeric_type(pVal);
+  *piRowid = sqlite3_value_int64(pVal);
+  return eType==SQLITE_INTEGER
+      || (eType==SQLITE_FLOAT
+          && 0==sqlite3IntFloatCompare(*piRowid, sqlite3_value_double(pVal)));
+}
 
 /* 
 ** Rtree virtual table module xFilter method.
@@ -1929,13 +1943,9 @@
     /* Special case - lookup by rowid. */
     RtreeNode *pLeaf;        /* Leaf on which the required cell resides */
     RtreeSearchPoint *p;     /* Search point for the leaf */
-    i64 iRowid = sqlite3_value_int64(argv[0]);
+    i64 iRowid;
     i64 iNode = 0;
-    int eType = sqlite3_value_numeric_type(argv[0]);
-    if( eType==SQLITE_INTEGER
-     || (eType==SQLITE_FLOAT 
-         && 0==sqlite3IntFloatCompare(iRowid,sqlite3_value_double(argv[0])))
-    ){
+    if( rtreeRowidEqValue(argv[0], &iRowid) ){
       rc = findLeafNode(pRtree, iRowid, &pLeaf, &iNode);
     }else{
       rc = SQLITE_OK;
--- a/ext/rtree/geopoly.c
+++ b/ext/rtree/geopoly.c
@@ -1389,9 +1389,13 @@
     /* Special case - lookup by rowid. */
     RtreeNode *pLeaf;        /* Leaf on which the required cell resides */
     RtreeSearchPoint *p;     /* Search point for the leaf */
-    i64 iRowid = sqlite3_value_int64(argv[0]);
+    i64 iRowid;
     i64 iNode = 0;
-    rc = findLeafNode(pRtree, iRowid, &pLeaf, &iNode);
+    if( rtreeRowidEqValue(argv[0], &iRowid) ){
+      rc = findLeafNode(pRtree, iRowid, &pLeaf, &iNode);
+    }else{
+      pLeaf = 0;
+    }
     if( rc==SQLITE_OK && pLeaf!=0 ){
       p = rtreeSearchPointNew(pCsr, RTREE_ZERO, 0);
       assert( p!=0 );  /* Always returns pCsr->sPoint */
```

geopoly.c is compiled as part of rtree.c, so the helper is visible there. Leaving `omit` clear in geopolyBestIndex(), so that SQLite checks each row again, would not be enough: a re-check can drop the wrong row but cannot add a missing one, so `rowid = '1e1'` would still miss rowid 10 because geopolyFilter() would look up 1. The value has to be converted before the lookup either way.

This is offered as a fix. The full patch, including regression tests appended to ext/rtree/rtree1.test (157 cases), is in the attached sqlite-geopoly-rowid-eq-nonint.patch.

The new tests compare `rowid = <value>`, `rowid IN (<value>, 7)` and a LEFT JOIN on a column holding the value, for 22 values, on an rtree and a geopoly table with an ordinary table holding the same rowids. Without the patch 41 of rtree1.test's 456 cases fail (37 on the geopoly table, 4 on the rtree table for '0.6e1' and '1e1'); with it none do. All 27 files in ext/rtree show no other change, and veryquick shows no new failures (the same 25 errors in json101, json501, func, literal, mjournal and types3 with and without the patch). The new tests and the example above run clean under ASan/UBSan, and the amalgamation compiles with `-Wall -Wextra` with no new warnings.

**Versions** (Geopoly builds; the macOS /usr/bin/sqlite3 3.51.0 has no Geopoly): wrong in 3.46.0, 3.50.2 and 3.53.2 (amalgamations built with -DSQLITE_ENABLE_GEOPOLY -DSQLITE_ENABLE_RTREE) and trunk. R-Tree's check is in all of these, and so is the '1e1' gap.

We reported a similar problem in FTS5 a short while ago (MATCH with `rowid = <non-integer value>`, where xBestIndex also sets omit for the rowid equality); the patch here follows the same comparison rules.

We found it with a property-based test generator for SQLite databases and queries: a join on the rowid of a geopoly table returned rows for REAL keys when the constraint was pushed into the virtual table, and none when the same query read the table from a materialized CTE.
