**Title:** OP_SeekRowid converts a key that has both a number and its text from the text: assertion, and the wrong row with a short fp_digits (since 3.54.0)

```sql
CREATE TABLE t(a);
SELECT * FROM t WHERE rowid = sqlar_uncompress(1.0, 3);
```

Output from the CLI of trunk (check-in 8a675a4b2b, 2026-11-05 00:01), built from the source tarball with zlib and -DSQLITE_DEBUG. 3.54.1 gives the same:

```
Assertion failed: ((pRec->flags & (MEM_Str|MEM_Int|MEM_Real|MEM_IntReal))==MEM_Str), function applyNumericAffinity
```

Expected: no output and no assertion. As in https://sqlite.org/bugs/forumpost/45bcbf3580, sqlar_uncompress(X, SZ) returns X unchanged after reading its byte length, so sqlar_uncompress(1.0, 3) is the real 1.0 with the text "1.0" attached; an application function that calls sqlite3_value_text() on its argument and returns it with sqlite3_result_value() gives the same value (repro.c).

A release build gives the wrong row when the text is not exact, for example with fp_digits 4, where the text of 12345.0 is 1.235e+04:

```sql
CREATE TABLE t(a);
INSERT INTO t(rowid, a) VALUES(12340, 'row 12340'), (12345, 'row 12345'), (12350, 'row 12350');
.dbconfig fp_digits 4
SELECT a FROM t WHERE rowid = sqlar_uncompress(12345.0, 9);
SELECT a FROM t WHERE +rowid = sqlar_uncompress(12345.0, 9);
```

```
row 12350
row 12345
```

Expected: row 12345 twice.

**Cause, as far as we can tell.** Since check-in 987ba5a3dc (2026-07-19, in 3.54.0), OP_SeekRowid calls applyNumericAffinity() on a copy of the key whenever MEM_Str is set. applyNumericAffinity() expects a value that is text alone; here the key is already MEM_Real (or MEM_Int) with cached text, so it asserts, and in a release build it converts the text and ignores the number that is already there. 3.50.4 uses applyAffinity() with NUMERIC affinity in OP_SeekRowid and has neither problem.

The patch below calls applyNumericAffinity() only for a key that is text alone, and gives the expected output above for both scripts:

```diff
--- a/src/vdbe.c
+++ b/src/vdbe.c
@@ -5678,7 +5678,7 @@
     ** changing the datatype of pIn3, however, as it is used by other
     ** parts of the prepared statement. */
     Mem x = pIn3[0];
-    if( x.flags & MEM_Str ){
+    if( (x.flags & (MEM_Str|MEM_Int|MEM_Real|MEM_IntReal))==MEM_Str ){
       applyNumericAffinity(&x, 1);
     }
     if( x.flags & MEM_Int ){
```

**Versions**: wrong in trunk (8a675a4b2b) and 3.54.1; 3.50.4 is not affected (and has no fp_digits setting).

We found it with a property-based test generator for SQLite that builds VDBE programs directly and calls application-defined functions that read their arguments through the sqlite3_value interfaces.
