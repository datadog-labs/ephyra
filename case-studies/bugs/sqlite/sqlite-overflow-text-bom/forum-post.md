**Title:** In a UTF-16 database, text over 4000 bytes read from a table loses a leading U+FEFF, and an UPDATE then writes it back without it

```sql
PRAGMA encoding = 'UTF-16le';
CREATE TABLE t(n, c);
CREATE INDEX i ON t(c);
INSERT INTO t VALUES (1000, char(0xFEFF) || printf('%.*c', 1000, 'a'));
INSERT INTO t VALUES (5000, char(0xFEFF) || printf('%.*c', 5000, 'a'));
SELECT n, length(c), unicode(c) FROM t NOT INDEXED;
SELECT n, length(c), unicode(c) FROM t INDEXED BY i;
PRAGMA integrity_check;
```

Output from trunk (check-in f69fd5f5c301, 2026-10-12 23:38), built from the source tarball with default options. 3.54.0 and 3.53.4 give the same:

```
1000|1001|65279
5000|5000|97
1000|1001|65279
5000|5001|65279
row 2 missing from index i
```

Expected `5000|5001|65279` on the second line, as from the index, and `ok`. Both strings start with U+FEFF (ZERO WIDTH NO-BREAK SPACE), an ordinary character. Read from the table, the 5000-character one comes back without it; read from the index, it comes back with it. So `WHERE c = <the inserted value>` finds the row through the index and not with NOT INDEXED. The file is still correct at this point, and integrity_check's complaint is about the read, not the index.

It becomes real data loss on the next write of that row. An UPDATE of another column reads `c` the same way and stores the shortened value:

```sql
UPDATE t SET n = n + 1;
```

Opening the file with 3.42.0, which reads it correctly, `length(c)` for that row is 5001 before this UPDATE and 5000 after, and 3.42.0's integrity_check now reports the index entry as missing. A REINDEX on trunk then rebuilds the index from the shortened value, and integrity_check says `ok`. UTF-16be behaves the same way. UTF-8 databases are not affected.

If the text starts with U+FFFE instead, the first character is dropped and the rest is read in the opposite byte order:

```sql
PRAGMA encoding = 'UTF-16be';
CREATE TABLE t(c);
INSERT INTO t VALUES (char(0xFFFE) || 'ab' || printf('%.*c', 2500, 'b'));
SELECT length(c), unicode(c), hex(substr(c, 1, 3)) FROM t;
-- 2502|24832|610062006200   expected 2503|65534|FFFE00610062
```

This also happens to intermediate results stored in an ephemeral table, such as a materialized CTE. So one aggregate result can come out differently depending on the query around it.

**Cause, as far as we can tell.** In `vdbeColumnFromOverflow()`, a TEXT value over 4000 bytes read through a table cursor takes the RCStr cache path:

```c
rc = sqlite3VdbeMemSetStr(pDest, pBuf, len, encoding, sqlite3RCStrUnref);
```

For a UTF-16 encoding, `sqlite3VdbeMemSetStr()` calls `sqlite3VdbeMemHandleBom()`, which treats a leading FE FF / FF FE as a byte-order mark. It removes those bytes and sets `pMem->enc` from them. That makes sense for strings passed in through the API, but not for text read from the database. The other read path, `sqlite3VdbeMemFromBtree()`, doesn't do it. The blob branch just below already passes 0 and then sets `pDest->enc = encoding;`. Doing the same in the text branch fixes both examples above:

```diff
--- a/src/vdbe.c
+++ b/src/vdbe.c
@@ -796,8 +796,9 @@
     assert( t>=12 );
     sqlite3RCStrRef(pBuf);
     if( t&1 ){
-      rc = sqlite3VdbeMemSetStr(pDest, pBuf, len, encoding,
+      rc = sqlite3VdbeMemSetStr(pDest, pBuf, len, SQLITE_UTF8,
                                 sqlite3RCStrUnref);
+      pDest->enc = encoding;
       pDest->flags |= MEM_Term;
     }else{
       rc = sqlite3VdbeMemSetStr(pDest, pBuf, len, 0,
```

We checked the patch against these examples, our original query, and round-trips of long text in all three encodings. We have not run the test suite with it, so take it as a pointer to the cause rather than a proposed fix.

**Versions** (official amalgamations or tarballs, default options): 3.39.0, 3.40.0, 3.41.0 and 3.42.0 are correct. Every minor release from 3.43.0 to 3.51.0, plus 3.53.4, 3.54.0 and trunk f69fd5f5c301, shows the bug. 3.43.0 is when the overflow cache arrived. This is the text branch of the function whose blob branch was fixed for [forum post 48c64d8bab](https://sqlite.org/bugs/forumpost/48c64d8bab) (check-in a8895cb047, a blob over 4000 bytes converted as UTF-8 in a UTF-16 database).

We found it with a property-based test generator for SQLite databases and queries. In a UTF-16be database, a 4060-byte `group_concat()` result starting with U+FFFE came out byte-swapped under `SELECT DISTINCT`, and swapped back but missing its first character under the equivalent INTERSECT form.
