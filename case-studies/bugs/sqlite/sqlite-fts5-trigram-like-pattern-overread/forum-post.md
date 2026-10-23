**Title:** FTS5 trigram: a LIKE pushed down to FTS5 makes sqlite3Fts5ExprPattern() read one byte past the end of the pattern

The query result is not affected: this is a one-byte read past the terminator, with no write, so it is low priority.

```sql
CREATE VIRTUAL TABLE t USING fts5(a, tokenize=trigram);
SELECT * FROM t WHERE a LIKE 'abcdefg';
```

Output from trunk (check-in d6f1df8c0b, 2026-10-23 00:49 UTC), built from the source tarball with `--enable-fts5 --enable-rtree --enable-geopoly` and `CFLAGS="-g -O1 -fsanitize=address"`, run as `sqlite3 -lookaside 0 0 :memory:` (first lines of the report):

```
==43698==ERROR: AddressSanitizer: heap-buffer-overflow on address 0x602000002d98 at pc 0x00010275f250 bp 0x00016da940a0 sp 0x00016da94098
READ of size 1 at 0x602000002d98 thread T0
    #0 0x00010275f24c in sqlite3Fts5ExprPattern sqlite3.c:249524
    #1 0x00010275f24c in fts5FilterMethod sqlite3.c:264140
    #2 0x0001024a8ee4 in sqlite3VdbeExec sqlite3.c:105117
0x602000002d98 is located 0 bytes after 8-byte region [0x602000002d90,0x602000002d98)
```

The 8-byte region is the copy of the 7-byte literal plus its terminator, made by `sqlite3DbStrNDup()` from `vdbeChangeP4Full()`. We have no 3.54.0 build with FTS5, but `sqlite3Fts5ExprPattern()` is the same from 3.46.0 through trunk (see Versions).

Expected: no report. `sqlite3Fts5ExprPattern()` gets the pattern from `sqlite3_value_text()` and only the bytes up to and including the terminator belong to it. For a bound parameter that is the application's buffer: sqlite.h says that for a negative length "the length of the string is the number of bytes up to the first zero terminator", and, for SQLITE_UTF8_ZT, that "the Z array is allocated to hold at least N+1 bytes". The same query with GLOB (`a GLOB 'abcdefg'`) gives no report, and so does the LIKE with the patch below.

With `sqlite3_bind_text(st, 1, buf, -1, SQLITE_STATIC)` for `SELECT count(*) FROM t WHERE a LIKE ?1`, where `buf` is a `malloc(n+1)` string, ASan reports the read "0 bytes after" the application's `n+1`-byte region for every length we tried (0, 5, 7, 10). When the terminator of such a buffer is the last byte before a PROT_NONE page, a plain `-O2` build without ASan stops with SIGBUS (exit status 138). ASan also reports `LIKE '%bcdef%'`, `LIKE 'a_c_e_g'`, a 15-byte pattern, `LIKE CAST('abcdefg' AS BLOB)`, `detail=column`, and `detail=none` on a second column. With the default lookaside a literal of 1207 bytes is reported and one of 1199 is not. There is no report for a 6-byte literal: the read is still there, but it lands in the rounding slack of the 8-byte allocation, as it does for most literals inside a lookaside slot. There is also no report for GLOB (`'abcdefg'`, `'abc[d]e*'`), for `LIKE ... ESCAPE`, or with `case_sensitive 1` or `remove_diacritics 1`, where FTS5 does not take the LIKE. The existing fts5trigram.test and fts5trigram2.test pass on the ASan build without a report.

**Cause, as far as we can tell.** In `sqlite3Fts5ExprPattern()` (ext/fts5/fts5_expr.c), LIKE sets `aSpec[2] = 0` (GLOB sets it to `'['`). The loop runs up to `i<=nText`, and at `i==nText` it checks for a bracket expression:

```c
        if( zText[i]==aSpec[2] ){
          i += 2;
          if( zText[i-1]=='^' ) i++;
          while( i<nText && zText[i]!=']' ) i++;
        }
```

For LIKE, `zText[nText]` is the terminator, which equals `aSpec[2]`, so the branch is taken on every call. `i` becomes `nText+2` and `zText[i-1]` reads `zText[nText+1]`, one byte past the terminator (line 406, the `'^'` test). The value only decides one `i++` before the loop ends, so the generated expression and the result do not depend on it. For GLOB, a `'['` can only be at an index below `nText`, so the read stays at or before the terminator.

The patch below stops the bracket check at the end of the pattern:

```diff
--- a/ext/fts5/fts5_expr.c
+++ b/ext/fts5/fts5_expr.c
@@ -401,7 +401,7 @@
           zExpr[iOut++] = '"';
           zExpr[iOut++] = ' ';
         }
-        if( zText[i]==aSpec[2] ){
+        if( i<nText && zText[i]==aSpec[2] ){
           i += 2;
           if( zText[i-1]=='^' ) i++;
           while( i<nText && zText[i]!=']' ) i++;
```

With it, on an ASan build of trunk d6f1df8c0b, the repro and all of the cases above that were reported are clean, the PROT_NONE case runs to completion, and query results are unchanged. All 149 files in ext/fts5/test pass with no ASan report, except fts5contentless3 (1.1, 1.3, 1.6, 1.8), which fails the same way without the patch.

**Versions:** the release CLIs we have are built without FTS5, so these results come from ASan builds of the amalgamations. 3.46.0, 3.51.3, 3.53.2 and trunk d6f1df8c0b report the read for the bound-parameter case, and the function is unchanged between 3.46.0 and trunk. Earlier we also saw the report from 3.34.1, 3.35.4, 3.45.3 and 3.50.2. 3.33.0 has no trigram tokenizer. 3.34.0, which added the trigram tokenizer and its LIKE and GLOB support, is most likely the first affected release, but we have not built it. The macOS system CLI (3.51.0, with FTS5) hands this LIKE to FTS5 (`VFilter ... L0` in EXPLAIN), so the code runs there too.

We found it with a property-based test generator for SQLite databases and queries: while checking our patch for the trigram LIKE report on UNINDEXED columns ([forum post be50ab5692](https://sqlite.org/bugs/forumpost/be50ab5692)), we ran generated trigram LIKE and GLOB queries on an ASan build. That patch does not touch this code.
