**Title:** group_concat() window over a moving frame drops separators and text after an empty string

```sql
CREATE TABLE t1(a INTEGER PRIMARY KEY, b);
INSERT INTO t1 VALUES(1, ''), (2, ''), (3, '');
SELECT quote(group_concat(b) OVER (ORDER BY a ROWS BETWEEN CURRENT ROW AND 1 FOLLOWING)) FROM t1;

CREATE TABLE t2(a INTEGER PRIMARY KEY, b);
INSERT INTO t2 VALUES(1, 'a'), (2, ''), (3, 'b');
SELECT quote(group_concat(b, '-') OVER (ORDER BY a ROWS BETWEEN CURRENT ROW AND 1 FOLLOWING)) FROM t2;
```

Output from trunk (check-in 155b341bb4f1, 2026-10-16 13:14), built from the source tarball with default options. 3.54.0 gives the same:

```
','
''
''
'a-'
'b'
''
```

Expected:

```
','
','
''
'a-'
'-b'
'b'
```

Row 2's frame holds rows 2 and 3 (two empty strings in t1, '' and 'b' in t2), so the separator belongs between them; row 3's frame holds only row 3, so t2 should give 'b'. The same happens with string_agg(), with any separator, with ROWS, RANGE and GROUPS frames whose start moves (`ROWS 1 PRECEDING` gives 'a', 'a-', '', 'b' for 'a', '', '', 'b' instead of 'a', 'a-', '-', '-b'), and when the separator varies from row to row. It needs a row to leave the frame while the text left behind is empty but terms are still in it: here one empty string. Frames with UNBOUNDED PRECEDING (no inverse), plain group_concat() aggregates, and the same queries without empty strings are right.

**Cause, as far as we can tell.** groupConcatStep() takes a term as the first one, and writes no separator before it, when `pGCC->str.mxAlloc==0`. groupConcatInverse() sets `mxAlloc = 0` (and frees pnSepLengths) whenever `str.nChar==0` after the removal, i.e. when the text is empty, not when the frame is. In t2, removing 'a' takes 'a-' off and leaves nChar 0 with nAccum 1 (the ''), so the next step appends 'b' with no separator: 'b' instead of '-b'. When '' is removed in turn, the inverse takes off its length plus nFirstSepLength, which is the 'b' that the missing separator should have absorbed, so row 3 is ''. Resetting only when no terms are left fixes every query above:

```diff
--- a/src/func.c
+++ b/src/func.c
@@ -2349,7 +2349,7 @@
       pGCC->str.nChar -= nVS;
       memmove(pGCC->str.zText, &pGCC->str.zText[nVS], pGCC->str.nChar);
     }
-    if( pGCC->str.nChar==0 ){
+    if( pGCC->nAccum==0 ){
       pGCC->str.mxAlloc = 0;
       sqlite3_free(pGCC->pnSepLengths);
       pGCC->pnSepLengths = 0;
```

With it, the window*, windowfault, func*, aggfault, aggnested and aggorderby tests pass as before, and veryquick shows no new failures. Take it as a pointer to the cause rather than a proposed fix.

**Versions** (official builds, default options): every release with window functions that we tried, 3.25.0, 3.28.0, 3.30.0, 3.35.0, 3.37.0, 3.39.0, 3.40.0, 3.44.0, 3.50.4, 3.53.4, 3.54.0 and trunk, drops the separator the same way. Up to 3.44.0 the empty results are NULL rather than ''. The thread "Possible bug in group_concat()" (2021, fixed in 3.37.0 by tracking separator lengths in groupConcatInverse) is about the same function, but a different bug.

We found it with a property-based test generator for SQLite databases and queries: the same window query gave ' | ', '', '' on three empty strings, and ' | ', ' | ', '' with the WindowFunc optimization turned off.
