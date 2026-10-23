**Title:** In a UTF-16 database, a blob over 4000 bytes read from a table's overflow pages is converted as UTF-8

```sql
PRAGMA encoding = 'UTF-16le';
CREATE TABLE t(n, c);
INSERT INTO t VALUES (1000, CAST(printf('%.*c', 1000, 'a') AS BLOB));
INSERT INTO t VALUES (5000, CAST(printf('%.*c', 5000, 'a') AS BLOB));
SELECT n, length(c), length(CAST(c AS TEXT)) FROM t;
```

Output from the 3.53.4 CLI, built from the release amalgamation with default options:

```
1000|2000|1000
5000|10000|1
```

Both blobs are the UTF-16LE bytes of a string of `a`s, so casting each back to text should give that string, as it does for the first. Per the documentation, a BLOB cast to TEXT is "interpreted as text encoded using the database encoding". The second blob is stored on overflow pages, and it is read as UTF-8: `a`, then a NUL, so the text is 1 character long. The same also happens with 3.46.0, 3.50.2, 3.53.2, the macOS system `sqlite3` (3.51.0), and current trunk (check-in 2662a68f0d5c, 2026-10-07). 3.42.0 and earlier are correct: the code below arrived in 3.43.0.

It affects any conversion of such a blob to text or a number (CAST, `upper()`, arithmetic, `WHERE c`). Because only the table path is affected, results can depend on the plan: through an index on `c` the value converts correctly.

**Cause.** In `vdbeColumnFromOverflow`, a value longer than 4000 bytes read through a table cursor (`pC->pKeyInfo==0`) takes the reference-counted cache path, and a blob there is set with

```c
rc = sqlite3VdbeMemSetStr(pDest, pBuf, len, 0, sqlite3RCStrUnref);
```

The encoding argument 0 marks a blob, and `sqlite3VdbeMemSetStr` then sets `pDest->enc` to SQLITE_UTF8, where other paths leave the database encoding. Adding `pDest->enc = encoding;` after that call (`encoding` is saved from `pDest->enc` at the top of the function) gives `5000|10000|5000`. Trunk has the same code.

Found with a property-based test generator for SQLite database files, using SQLancer-style query-plan and TLP oracles.
