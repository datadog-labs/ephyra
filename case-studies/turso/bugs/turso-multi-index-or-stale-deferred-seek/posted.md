# OR over an index and the rowid misses rows: a seek on the table cursor leaves an earlier DeferredSeek pending

Posted as https://github.com/tursodatabase/turso/issues/10908 on 2026-10-27. The text below is the issue as posted.

---

```sql
CREATE TABLE t(a INTEGER PRIMARY KEY, b);
CREATE INDEX i ON t(b);
INSERT INTO t VALUES (3, NULL), (4, 0), (5, 1);
SELECT a FROM t WHERE (b = 0 AND a IS NULL) OR a >= 0;
DELETE FROM t WHERE (b = 0 AND a < 0) OR a >= 0;
SELECT a FROM t;
```

Turso (`tursodb -q -m list :memory: < repro.sql`):
```
4
3
5
```

(`4` is the SELECT's only row; `3` and `5` are the rows the DELETE left behind.)

SQLite 3.50.4 (trunk gives the same):
```
3
4
5
```

(The SELECT returns all three rows, and the DELETE empties the table.)

Expected: `a >= 0` holds for every row, so the SELECT returns 3, 4 and 5 and the DELETE empties the table. Turso returns only row 4, and the DELETE removes only row 4. The same happens with `a < 0` or `a IS NULL` in the first branch, and with `a = 3` as the second branch (no rows at all). When the first branch reads a non-rowid column (`(b = 0 AND c = 9) OR a >= 0`), the result is right.

Likely cause: the plan is a multi-index OR (`MULTI-INDEX OR t (i, PRIMARY KEY)`). The first branch scans `i` and emits `DeferredSeek i -> t` for each entry, then evaluates `a IS NULL` with `RowId t`. `op_row_id` (core/vdbe/execute.rs) answers from the pending deferred seek and, like SQLite, leaves it pending. The second branch then runs `SeekGE` on `t` and loops with `RowId t` / `Next t`. But `op_seek` (and `op_rewind` / `op_last`) do not clear `state.deferred_seeks[t]`, so every `RowId t` in the second branch still returns the rowid of the last index entry from the first branch (4), and the RowSet ends up holding only that rowid. In SQLite, OP_SeekGE/GT/LE/LT, OP_SeekRowid, OP_Rewind and OP_Last all set `pC->deferredMoveto = 0`. Calling `invalidate_deferred_seeks_for_cursor` for the table cursor in those ops (as `op_seek_rowid` and `OpenRead` already do) should fix it. #9243 is a different path to a stale deferred seek (after NullRow).

Tested on Turso main 222c39296 (also 0f5b651c and db441331).
