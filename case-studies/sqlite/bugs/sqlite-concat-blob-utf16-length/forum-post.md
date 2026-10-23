**Title:** concat() and concat_ws() of a BLOB in a UTF-16 database copy the BLOB's size, not the text's: a NUL and stale bytes in the result

```sql
PRAGMA encoding = 'UTF-16be';
SELECT hex(concat(x'0041'));
SELECT hex(concat(x'00410042'));
SELECT hex(concat_ws(x'002C', 'a', 'b'));
SELECT hex(concat(x'0800'));
```

Output from trunk (check-in 668541c592b8, 2026-10-16 14:31), built from the source tarball with default options. 3.44.0 and 3.51.0 give the same, apart from the last byte of the second line, which varies:

```
00410000
0041004200000097
0061002C00000062
FFFD
```

Expected:

```
0041
00410042
0061002C0062
0800
```

In a UTF-16 database a BLOB converted to text is read as UTF-16 text in the database's encoding, so x'0041' is 'A', x'00410042' is 'AB' and x'0800' is U+0800 (as `CAST(x'0041' AS TEXT)` and `x'0041' || ''` show). concat() gives 'A' followed by a NUL, 'AB' followed by a NUL and a byte that was never written (it changes from run to run: 0x97, 0x57, 0xB2, 0x8A, ...), and for x'0800' only the first two of the three UTF-8 bytes, which read back as U+FFFD. concat_ws() does the same with a BLOB separator or BLOB arguments. UTF-8 databases are not affected.

Because the function converts its argument register in place, a constant BLOB argument is only wrong on the first row, so the result depends on the scan order:

```sql
PRAGMA encoding = 'UTF-16be';
CREATE TABLE t1(c0);
INSERT INTO t1 VALUES (NULL), (0);
CREATE INDEX t1_i0 ON t1(c0 DESC);
SELECT hex(concat(x'0041', c0)) FROM t1 NOT INDEXED;   -- 00410000, 00410030
SELECT hex(concat(x'0041', c0)) FROM t1 INDEXED BY t1_i0; -- 004100000030, 0041
```

Expected `0041, 00410030` and `00410030, 0041`.

**Cause, as far as we can tell.** concatFuncCore() (func.c) sums `sqlite3_value_bytes(argv[i])` to size its buffer and, per argument, takes `k = sqlite3_value_bytes(argv[i])` before `v = sqlite3_value_text(argv[i])`, then copies k bytes from v; concatwsFunc() does the same for the separator. For a BLOB, sqlite3_value_bytes() returns the BLOB's size without converting it. sqlite3_value_text() then converts it to UTF-8 (UTF-16 to UTF-8 halves ASCII, and makes U+0800 and up three bytes from two), in the register itself. So k is the BLOB's size and the copy runs past the text's terminator into the rest of the 2n+1 byte conversion buffer, which sqlite3VdbeMemTranslate() never wrote, or stops short of the text's end. On the next row the register already holds the UTF-8 text, so the sizes agree and the result is right. We did not see a read outside an allocation: AddressSanitizer on a debug build reports nothing, as the conversion buffer is always at least as large as the BLOB. Calling sqlite3_value_text() before sqlite3_value_bytes(), as the documentation of those functions advises, in both loops and for the separator, fixes every query above:

```diff
--- a/src/func.c
+++ b/src/func.c
@@ -1713,6 +1713,9 @@
   int bNotNull = 0;   /* True after at least NOT NULL argument seen */
   char *z;
   for(i=0; i<argc; i++){
+    /* Convert to text first: for a BLOB in a UTF-16 database the size of
+    ** the UTF-8 text is not the size of the BLOB. */
+    (void)sqlite3_value_text(argv[i]);
     n += sqlite3_value_bytes(argv[i]);
   }
   n += (argc-1)*(i64)nSep;
@@ -1724,9 +1727,14 @@
   j = 0;
   for(i=0; i<argc; i++){
     if( sqlite3_value_type(argv[i])!=SQLITE_NULL ){
-      int k = sqlite3_value_bytes(argv[i]);
       const char *v = (const char*)sqlite3_value_text(argv[i]);
+      int k = sqlite3_value_bytes(argv[i]);
       if( v!=0 ){
+        if( j + (bNotNull ? nSep : 0) + k > n ){
+          sqlite3_free(z);
+          sqlite3_result_error_nomem(context);
+          return;
+        }
         if( bNotNull && nSep>0 ){
           memcpy(&z[j], zSep, nSep);
           j += nSep;
@@ -1766,8 +1774,8 @@
   int argc,
   sqlite3_value **argv
 ){
-  int nSep = sqlite3_value_bytes(argv[0]);
   const char *zSep = (const char*)sqlite3_value_text(argv[0]);
+  int nSep = sqlite3_value_bytes(argv[0]);
   if( zSep==0 ) return;
   concatFuncCore(context, argc-1, argv+1, nSep, zSep);
 }
```

The added bounds check is only there for the case where a conversion in the first loop failed for lack of memory and a later one succeeds. With the patch, enc*, func*, utf16align and tclsqlite tests give the same results as without it. Take it as a pointer to the cause rather than a proposed fix.

**Versions:** 3.44.0, where concat() and concat_ws() were added, 3.51.0 and trunk give the wrong results above.

We found it with a property-based test generator for SQLite databases and queries: in a UTF-16be database, `SELECT concat(x'3f', c0) FROM t1` returned a different set of rows from the same query with `INDEXED BY t1_i0`.
