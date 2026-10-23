**Title:** ROLLBACK TO of a savepoint that dropped a table commits a stale root page in an incremental-vacuum database

The statements below build a file database with SQL only and leave it corrupt: in a new connection, `PRAGMA integrity_check` reports a bad pointer-map entry and a second reference to a free page. Every statement, the `COMMIT` included, returns OK; the corruption is silent and persists after commit.

```sql
PRAGMA auto_vacuum = INCREMENTAL;
CREATE TABLE a(x);
CREATE TABLE f(x);
INSERT INTO f VALUES (zeroblob(4200));
DROP TABLE f;
BEGIN;
CREATE TABLE b(x);
SAVEPOINT s;
DROP TABLE a;
ROLLBACK TO s;
COMMIT;
```

Output from the 3.53.4 CLI, built from the release amalgamation with default options (`rm -f t.db; sqlite3 t.db < repro.sql`, then `sqlite3 t.db 'PRAGMA integrity_check'` in a new connection):

```
*** in database main ***
Tree 4 page 4 cell 0: Bad ptr map entry key=5 expected=(3,4) got=(2,0)
Tree 4 page 4 cell 0: 2nd reference to page 5
```

It also reproduces on the macOS system `sqlite3` (3.51.0, which defaults to `secure_delete = FAST` and reserves 12 bytes per page), with no pragmas added. The unreduced cases from our generator failed the same way on 3.46.0, 3.50.2 and 3.53.2; the code involved is unchanged between those versions.

**What happens.** Default 4096-byte pages; the page numbers below are this script's. Page 1 is the schema page and page 2 the pointer-map. `a` gets root page 3. `f`'s root is page 4 and its overflow page is 5; `DROP TABLE f` frees both, leaving a free-list with trunk 4 and leaf 5. The largest-root-page header (meta[3]) becomes 3.

1. `CREATE TABLE b` inside the transaction: `btreeCreate` puts the new root at meta[3]+1 = 4, and `allocateBtreePage` removes that exact page from the free-list — here the trunk page itself, whose leaf 5 is re-linked as the new trunk. b's root content now exists only in the cache: it is dirty and has not been spilled to the file.
2. `DROP TABLE a` inside the savepoint: `a`'s root (3) is not the largest root (4), so `btreeDropTable` takes its relocation branch, used only in pointer-map databases: `relocatePage` moves the largest root page, b's, into the freed page 3. `sqlite3PagerMovepage` saves page 4's current content to the sub-journal first (it is dirty and not saved since the latest savepoint), then re-keys its cache entry as page 3. The vacated page 4 is then freed by `freePage2`, which appends it as a new leaf under trunk 5. Because `BTS_SECURE_DELETE` is not set, `freePage2` also calls `sqlite3PagerDontWrite` on it — the optimization that says a freed page's content never needs to reach the file. Page 4 is now flagged "do not write".
3. `ROLLBACK TO s`: `pagerPlaybackSavepoint` replays the sub-journal. The record for page 4 restores b's root content into the cache. The in-cache path of `pager_playback_one_page` only memcpy's the data; it does not call `sqlite3PcacheMakeDirty`, which is the only code that clears `PGHDR_DONT_WRITE`. The sibling not-in-cache path — whose comment describes exactly this scenario, a page on the free-list at the start of the transaction, populated, then moved by `sqlite3PagerMovepage` — refetches the page and does call it. Page 4 is left flagged "do not write", its correct content only in the cache.
4. `COMMIT`: `pager_write_pagelist` skips pages with `PGHDR_DONT_WRITE`. The file keeps the pre-transaction image of page 4 — `f`'s old root, whose cell references overflow page 5 — while the committed schema, free-list and pointer-map all say page 4 is b's root. A new connection parses `f`'s old root as b's root, producing both errors above: cell 0 points at page 5, which the pointer-map records as a free-list page (type 2, parent 0) instead of page 4's overflow page (type 3).

The session itself sees nothing: all statements return OK, and an `integrity_check` before the `COMMIT` passes, because the cache still holds the correct image of page 4. Evicting it first (`PRAGMA cache_size = -1` or `PRAGMA shrink_memory` after the `ROLLBACK TO`) makes the same session's `integrity_check` fail — that is how our generator caught it — but the flush is only needed to see it in-session; the corruption commits without it.

What it needs, each point verified by removing it:

- `auto_vacuum = INCREMENTAL`. `FULL` vacuums the free-list away at every commit, and `NONE` has no pointer-map, so neither relocates.
- A free-list of at least two pages when the `DROP TABLE` runs, so the vacated page becomes a free-list leaf: `freePage2`'s `sqlite3PagerDontWrite` runs only there. An empty free-list makes the vacated page a new trunk, which is written normally. (Dropping an empty table before the transaction leaves a single free page, which the `CREATE TABLE` inside the transaction consumes; the script then passes `integrity_check`.)
- A root page created inside the transaction, before the inner savepoint, that is the largest root and has not been spilled to the file. A page created in an earlier, committed transaction is already correct on disk, so the flag does not matter. `INSERT`, `UPDATE` and `DELETE` do not allocate a root, which is why they do not trigger it.
- The dropped table's root must not be the largest root, or `btreeDropTable` takes the plain freePage path and nothing is relocated.
- `secure_delete` not ON. OFF and FAST both leave `BTS_SECURE_DELETE` clear — FAST sets `BTS_OVERWRITE` instead (btreeSetFlags, sqlite3.c line 74732 of the 3.50.2 amalgamation) — so both reproduce; only ON does not.
- `journal_mode` DELETE, TRUNCATE, PERSIST or MEMORY. WAL is not affected: there the playback always refetches the page through the path that marks it dirty.
- A file database. `:memory:` is a temp-file pager, where `sqlite3PagerMovepage` calls `sqlite3PagerWrite` and the flag is never set.

Suggested fix: in `pager_playback_one_page`'s in-cache restore path, call `sqlite3PcacheMakeDirty` (or at least clear `PGHDR_DONT_WRITE`) for savepoint records, as the not-in-cache path does. Function names are from the 3.50.2 amalgamation; the code is identical in 3.53.4.

We found this with a property-based test generator for SQLite database files and SQL statements, which checks `integrity_check` after every write. Two generated cases in about 3 million failed this way. Both needed a cache flush to be visible before commit, which is why an earlier draft of this report required `PRAGMA cache_size = -1`; the reduced script above shows the corruption commits without it.
