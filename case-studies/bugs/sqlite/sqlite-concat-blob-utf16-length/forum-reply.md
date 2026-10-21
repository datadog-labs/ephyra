**Reply to:** https://sqlite.org/bugs/forumpost/c621e963a9 ("CONCAT produces extra trailing bytes for a BLOB argument in a UTF-16le database")

We ran into the same thing independently and can reproduce it on trunk (check-in 668541c592b8, 2026-10-16 14:31, built from the source tarball with default options) and on 3.51.0, in a fresh `sqlite3` CLI with no database file. Here is a version that doesn't depend on what is left in memory:

```sql
PRAGMA encoding = 'UTF-16le';
SELECT hex(concat(x'4100'));
SELECT hex(concat(x'0008'));
SELECT hex(concat_ws(x'2C00', 'a', 'b'));
```

Output:

```
41000000
FDFF
61002C0000006200
```

Expected:

```
4100
0008
61002C006200
```

`CAST(x'4100' AS TEXT)` and `x'4100' || ''` are 'A' (4100). concat() returns 'A' followed by a NUL, and for x'0008' (U+0800) it returns U+FFFD. (`PRAGMA encoding` only takes effect before the database is created. Run against an existing UTF-8 database file, the script reads the BLOBs as UTF-8 and shows nothing wrong.)

**Cause, as far as we can tell.** concatFuncCore() (func.c) sizes its buffer with `sqlite3_value_bytes(argv[i])`, and for each argument takes `k = sqlite3_value_bytes(argv[i])` before `v = sqlite3_value_text(argv[i])`. It then copies k bytes from v. concatwsFunc() does the same for the separator. For a BLOB, sqlite3_value_bytes() is the BLOB's size. sqlite3_value_text() then converts the BLOB, read as UTF-16, to UTF-8 in place. That halves ASCII, which is why the copy runs past the terminator into bytes the conversion never wrote (the stale bytes in the original post). It also turns U+0800 and above into three bytes from two, so the copy cuts the character short. The register keeps the converted text, so a constant BLOB argument is wrong only on the first row, and the result depends on scan order:

```sql
PRAGMA encoding = 'UTF-16be';
CREATE TABLE t1(c0);
INSERT INTO t1 VALUES (NULL), (0);
CREATE INDEX t1_i0 ON t1(c0 DESC);
SELECT hex(concat(x'0041', c0)) FROM t1 NOT INDEXED;      -- 00410000, 00410030
SELECT hex(concat(x'0041', c0)) FROM t1 INDEXED BY t1_i0; -- 004100000030, 0041
```

AddressSanitizer reports no read outside an allocation: the conversion buffer is never smaller than the BLOB. Calling sqlite3_value_text() before sqlite3_value_bytes(), as their documentation advises, in both loops and for the separator, fixes every query above:

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

The bounds check only covers a conversion that fails for lack of memory in the first loop and then succeeds in the second. With the patch, the enc*, func*, utf16align and tclsqlite tests give the same results as without it. Take it as a pointer to the cause rather than a proposed fix. Every version from 3.44.0, where concat() and concat_ws() were added, gives the wrong results.
