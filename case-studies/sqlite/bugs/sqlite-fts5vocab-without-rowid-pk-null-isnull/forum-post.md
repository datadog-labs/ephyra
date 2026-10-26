# Reply (bug forum thread d29f636f37)

Posted to the SQLite bug forum as https://sqlite.org/bugs/forumpost/2d3320849d on 2026-10-26, as a reply in thread https://sqlite.org/bugs/forumpost/d29f636f37. The text below is the post as written.

---

Hi Dan, I think there might be a regression?

Since 25d3a17413 the fts5vocab tables are declared WITHOUT ROWID, and their PRIMARY KEY includes columns that fts5vocab returns as NULL. `IS NULL` on those columns in a WHERE clause now gives the wrong answer:

```sql
CREATE VIRTUAL TABLE ft USING fts5(a, detail=none);
INSERT INTO ft VALUES('x');
CREATE VIRTUAL TABLE v USING fts5vocab(ft, 'col');
SELECT term, quote(col) FROM v;
SELECT count(*) FROM v WHERE col IS NULL;
SELECT count(*) FROM v WHERE col IS NOT NULL;
```

Trunk 75cb77338bd7 (2026-10-26) prints:

```
x|NULL
0
1
```

Expected, and printed by trunk 03f699544e6b (2026-10-21), before the change:

```
x|NULL
1
0
```

The PRIMARY KEY columns of a WITHOUT ROWID table are marked NOT NULL (`PRAGMA table_xinfo(v)` now shows notnull=1 for term and col), so `col IS NULL` is folded to false. On a detail=none table, fts5vocab returns NULL for col in 'col' and 'instance' tables. On a detail=none or detail=column table, it returns NULL for offset in 'instance' tables. All of these columns are affected in the same way.
