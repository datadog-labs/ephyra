-- Pre-triage check (sqlite-coverage/deploy/known-bugs.json): sqlite-fts5-locale-contentless-unindexed-insert
-- Run as: sqlite3 -batch -init /dev/null :memory: < check.sql (a build with FTS5)
-- Its output on a correct build (the fix applied) is the registry's "expected";
-- trunk printing exactly that means the bug was fixed upstream.
SELECT 'q1';
CREATE VIRTUAL TABLE h USING fts5(a, b UNINDEXED, locale=1, content='', contentless_unindexed=1);
INSERT INTO h(rowid, a, b) VALUES (4, 'p q', 'r');
SELECT rowid, quote(a), quote(b) FROM h;
SELECT 'q2';
INSERT INTO h(rowid, a, b) VALUES (5, fts5_locale('en', 'x y'), 's');
SELECT rowid, b FROM h('y');
SELECT 'q3';
INSERT INTO h(h) VALUES('integrity-check');
SELECT count(*) FROM h;
