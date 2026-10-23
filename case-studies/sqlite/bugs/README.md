# Bugs: sqlite

The bugs the case studies found in sqlite. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [sum(b ORDER BY a) FROM t1, t2 sums a instead of b: an aggregate's ORDER BY term replaces its argument](sqlite-aggregate-orderby-replaces-argument/) | fixed | [link](https://sqlite.org/bugs/forumpost/b53a65d1fc) |
| [In a UTF-16 database, a blob DEFAULT from ALTER TABLE ADD COLUMN is converted to text as UTF-8](sqlite-alter-default-blob-encoding/) | fixed | [link](https://sqlite.org/bugs/forumpost/a9c654a346) |
| [UNION of four or more SELECTs drops rows: the balanced merge removes the right half's duplicates under that half's collation](sqlite-balanced-merge-dedup-collation/) | open | SQLite bug forum, posted 2026-10-18 (thread id not recorded) |
| [A join on an INTEGER PRIMARY KEY misses the REAL key -9223372036854775808.0 when the plan has a Bloom filter](sqlite-bloom-filter-rowid-real-minint/) | fixed | [link](https://sqlite.org/bugs/forumpost/403ca95409) |
| [Compound SELECT with ORDER BY ... COLLATE moves the last SELECT's GROUP BY, HAVING and DISTINCT to the whole compound](sqlite-compound-orderby-collate-groupby/) | open | SQLite bug forum, posted 2026-10-16 (thread id not recorded) |
| [concat() and concat_ws() of a BLOB in a UTF-16 database copy the BLOB's size, not the text's: a NUL and stale bytes in the result](sqlite-concat-blob-utf16-length/) | reported by others | [link](https://sqlite.org/bugs/forumpost/c621e963a9) |
| [DELETE reports "database disk image is malformed" on a database that passes integrity_check (WITHOUT ROWID, tiny key)](sqlite-delete-corrupt/) | fixed upstream before we reported it |  |
| ["database disk image is malformed" from a write after a one-pass DELETE in the same statement (WITHOUT ROWID): the delete cursor keeps the deleted cell's CellInfo](sqlite-delete-saveposition-stale-cellinfo/) | open | SQLite bug forum, posted 2026-10-18 (thread id not recorded) |
| [ALTER TABLE DROP CONSTRAINT on a foreign key with SET DEFAULT, NOT DEFERRABLE or MATCH fails or leaves part of the constraint behind](sqlite-drop-constraint-fk-set-default/) | open | [link](https://sqlite.org/bugs/forumpost/5e80e94512) |
| [ALTER COLUMN DROP NOT NULL removes NOT DEFERRABLE instead of NOT NULL, or removes a DEFERRABLE clause that follows NOT NULL](sqlite-drop-not-null-not-deferrable/) | open | [link](https://sqlite.org/bugs/forumpost/28af38d7c3) |
| [Join drops rows, or adds some, when a table has an index on an expression and a `col = constant` WHERE term](sqlite-expr-index-constant-propagation/) | fixed | [link](https://sqlite.org/bugs/forumpost/529732a774) |
| [A UNION ALL subquery in FROM with a RIGHT JOIN after its first SELECT loses WHERE terms when flattened](sqlite-flatten-compound-right-join/) | fixed | SQLite bug forum, bug report 2026-10-08T15:27:33Z |
| [sqlite-fts5-and-nomatch-clears-later-row](sqlite-fts5-and-nomatch-clears-later-row/) | reported 2026-10-23 | [link](https://sqlite.org/bugs/forumpost/5cffabd8e9) |
| [FTS5: DROP TABLE and ALTER TABLE RENAME skip the %_content table of a contentless_unindexed=1 table](sqlite-fts5-contentless-unindexed-drop-rename/) | open | SQLite bug forum, posted 2026-10-21 (thread id not recorded) |
| [FTS5: with detail=none, secure-delete of a rowid that was replaced while secure-delete was off corrupts the index](sqlite-fts5-detail-none-secure-delete-replaced-rowid/) | open | [link](https://sqlite.org/bugs/forumpost/e380b6876c) |
| [FTS5: "no such cursor: 0" from an auxiliary function in WHERE with the `tbl = 'query'` form (constant propagation)](sqlite-fts5-eq-constprop-aux-cursor/) | open | SQLite bug forum, posted 2026-10-20 (thread id not recorded) |
| [FTS5: INSERT into a locale=1, contentless_unindexed=1 table with an UNINDEXED column fails with "database disk image is malformed"](sqlite-fts5-locale-contentless-unindexed-insert/) | open | SQLite bug forum, posted 2026-10-21 (thread id not recorded) |
| [FTS5: MATCH with `rowid = <non-integer value>` returns every match (regression after 3.51)](sqlite-fts5-match-rowid-eq-nonint/) | open | SQLite bug forum, posted 2026-10-20 (thread id not recorded) |
| [FTS5: phrases of a NEAR group that does not match a row report instances for that row, some left over from an earlier row](sqlite-fts5-near-nomatch-stale-poslist/) | open | [link](https://sqlite.org/bugs/forumpost/9fa08e2dbd) |
| [FTS5: the right operand of a NOT reports instances when the left operand matches no row](sqlite-fts5-not-eof-rhs-instances/) | open | [link](https://sqlite.org/bugs/forumpost/5cffabd8e9) |
| [FTS5: after secure-delete removes the last rowids of a long doclist, a query with a rowid lower bound returns rows of the next term](sqlite-fts5-secure-delete-dlidx-walks-next-term/) | fixed upstream before we reported it |  |
| [FTS5: with secure-delete on, a merge can drop a delete marker, and a later secure delete makes an old entry for the rowid match again](sqlite-fts5-secure-delete-merge-drops-marker/) | open | [link](https://sqlite.org/bugs/forumpost/737cb495ea) |
| [FTS5 tokendata=1: a query for an absent term fails with "fts5: corruption found reading blob" after a long doclist](sqlite-fts5-tokendata-missing-term-dlidx-corrupt/) | open | SQLite bug forum, posted 2026-10-21 (thread id not recorded) |
| [sqlite-fts5-tokendata-prefix-corrupt](sqlite-fts5-tokendata-prefix-corrupt/) | reported 2026-10-23 | [link](https://sqlite.org/bugs/forumpost/5c53f75fbb) |
| [FTS5 trigram: a LIKE pushed down to FTS5 makes sqlite3Fts5ExprPattern() read one byte past the end of the pattern](sqlite-fts5-trigram-like-pattern-overread/) | open | [link](https://sqlite.org/bugs/forumpost/402c7177cb) |
| [sqlite-fts5-trigram-like-unindexed](sqlite-fts5-trigram-like-unindexed/) | reported 2026-10-23 | [link](https://sqlite.org/bugs/forumpost/be50ab5692) |
| [FTS5 detail=none/column: snippet() fails with "database disk image is malformed", highlight() and bm25() see matches in UNINDEXED columns](sqlite-fts5-unindexed-detail-none-instances/) | open | SQLite bug forum, posted 2026-10-20 (thread id not recorded) |
| [sqlite-fts5vocab-rowid-or-right-join](sqlite-fts5vocab-rowid-or-right-join/) | reported 2026-10-23 | [link](https://sqlite.org/bugs/forumpost/d29f636f37) |
| [FTS5: in a UTF-16 database, fts5vocab applies `term <` / `>` and ORDER BY term in UTF-8 order](sqlite-fts5vocab-term-range-utf16-order/) | open | [link](https://sqlite.org/bugs/forumpost/b0c03988e8) |
| [An index on a VIRTUAL generated column with no type returns an integral REAL as an INTEGER](sqlite-gencol-index-real-int/) | fixed | [link](https://sqlite.org/bugs/forumpost/6973ad169e) |
| [Geopoly: `rowid = <non-integer value>` finds the row of its integer part](sqlite-geopoly-rowid-eq-nonint/) | open | SQLite bug forum, posted 2026-10-21 (thread id not recorded) |
| [Geopoly: an UPDATE that changes the rowid but not _shape fails with "_shape does not contain a valid polygon"](sqlite-geopoly-update-rowid-nochange/) | open | SQLite bug forum, posted 2026-10-21 (thread id not recorded) |
| [GLOB 'é*' on an indexed untyped column misses an over-length UTF-8 blob that a scan finds (invalid UTF-8, garbage in, garbage out)](sqlite-glob-index-blob/) | not a bug |  |
| [group_concat() window over a moving frame drops separators and text after an empty string](sqlite-group-concat-inverse-empty-text/) | fixed | SQLite bug forum, posted 2026-10-16 (thread id not recorded) |
| [`x IN (SELECT …)` in HAVING converts the GROUP BY term x by the IN's affinity for the whole query (regression in 3.15.0)](sqlite-group-term-in-select-affinity/) | reported by others | SQLite, reported by someone else 2026-07-29; fixed by check-in b9ecd9caf7 |
| [Rows out of order despite ORDER BY x DESC in a query with GROUP BY x over a subquery sorted by x ASC](sqlite-groupby-subquery-orderby-direction/) | fixed | [link](https://sqlite.org/bugs/forumpost/5cd4b3b27a) |
| [x IN (SELECT y COLLATE NOCASE ...) misses rows when x has an index](sqlite-in-select-collate-index/) | fixed | [link](https://sqlite.org/bugs/forumpost/f505c400cf) |
| [Rows missing when a UNION ALL subquery or CTE is joined with an inner join followed by a RIGHT or FULL JOIN](sqlite-intersect-full-join-cte/) | fixed | [link](https://sqlite.org/bugs/forumpost/50dd78a44b) |
| [A correlated subquery over a three-way join counts 0 for later outer rows (JoinMiss deletes automatic-index entries)](sqlite-joinmiss-autoindex-subquery/) | fixed | [link](https://sqlite.org/bugs/forumpost/8e5c07be86) |
| [json_valid() returns 0 for valid JSON text based on json_set call](sqlite-json-valid-stale-cache-json5/) | fixed | [link](https://sqlite.org/bugs/forumpost/1339f93b28) |
| [LEFT JOIN with `x = NULL AND y IN (…)` on a two-column index returns a matched row as well as the NULL-extended one (regression in 3.43.0)](sqlite-left-join-in-null/) | fixed | SQLite bug forum, posted 2026-10-07 (thread id not recorded) |
| [An OR in a LEFT JOIN's ON clause drives a loop on the left table: wrong rows, or a segfault when the joined table is omitted](sqlite-leftjoin-on-or-omitted-table-segv/) | open | [link](https://sqlite.org/bugs/forumpost/090de3eb79) |
| [a IN (...) AND c GLOB 'A' with an index on (a,c): GROUP BY splits a group, DISTINCT repeats a row (BLOB in c)](sqlite-like-opt-in-order/) | fixed upstream before we reported it |  |
| [NOT INDEXED is ignored on WITHOUT ROWID tables: the planner still uses secondary and UNIQUE indexes](sqlite-notindexed-without-rowid/) | open | [link](https://sqlite.org/bugs/forumpost/cd41fceaa0) |
| [An OR term that uses a partial index WHERE a=5 and also tests a fails with "internal query planner error"](sqlite-or-partial-index-covering-error/) | fixed | [link](https://sqlite.org/bugs/forumpost/3ede79ce42) |
| [In a UTF-16 database, a blob over 4000 bytes read from a table's overflow pages is converted as UTF-8](sqlite-overflow-blob-encoding/) | fixed | [link](https://sqlite.org/bugs/forumpost/48c64d8bab) |
| [In a UTF-16 database, text over 4000 bytes read from a table loses a leading U+FEFF, and an UPDATE then writes it back without it](sqlite-overflow-text-bom/) | fixed | [link](https://sqlite.org/bugs/forumpost/36a73e8c8a) |
| [A partial index WHERE col = <constant> makes col read back as the constant: a TEXT '5.0' comes back as '5'](sqlite-partial-index-constant-affinity/) | open | [link](https://sqlite.org/bugs/forumpost/35992eec46) |
| [WHERE b = -9223372036854775808.0 AND typeof(b) = 'integer' misses a row whose INT column b holds that integer](sqlite-propagate-const-real-minint/) | open | [link](https://sqlite.org/bugs/forumpost/9bbaa34eea) |
| [sqlite-rename-column-ipk-unique](sqlite-rename-column-ipk-unique/) | reported 2026-10-23 | [link](https://sqlite.org/bugs/forumpost/b088eb5b02) |
| [`SELECT *` over a USING join left of a RIGHT or FULL JOIN is "ambiguous column name" when a later table has that column](sqlite-right-join-using-star-ambiguous/) | open | [link](https://sqlite.org/bugs/forumpost/8be1215e2c) |
| [A WHERE-clause OR on a table left of a RIGHT JOIN drives a MULTI-INDEX OR and is never tested (extra rows)](sqlite-right-join-where-or-multi-index/) | not reported upstream |  |
| [Row-value IN in a join repeats rows: the inner loop iterates the full IN list, with duplicates, for one of its columns](sqlite-rowvalue-in-join-reuse/) | open | SQLite bug forum, posted 2026-10-16 (thread id not recorded) |
| [R-Tree: `UPDATE ... SET rowid = N` and `INSERT INTO ...(rowid, ...)` ignore the new rowid](sqlite-rtree-update-set-rowid-ignored/) | open | SQLite bug forum, posted 2026-10-21 (thread id not recorded) |
| [ROLLBACK TO of a savepoint that dropped a table commits a stale root page in an incremental-vacuum database](sqlite-savepoint-autovacuum/) | fixed | [link](https://sqlite.org/bugs/forumpost/3bc3c8adfa) |
| [ALTER TABLE RENAME COLUMN of a TEMP table fails when a TEMP trigger or view uses the column](sqlite-temp-rename-column-twice/) | open | SQLite bug forum, posted 2026-10-19 (thread id not recorded) |
| [UPDATE changes REAL -9223372036854775808.0 in a column it does not assign to INTEGER, but not in the index](sqlite-update-real-minint-stale-index/) | not reported upstream |  |
| ["SQL logic error" on the next SAVEPOINT or COMMIT, and the transaction is lost, after ROLLBACK TO undoes a CREATE VIRTUAL TABLE (fts5, fts4)](sqlite-vtab-create-rollback-to-savepoint/) | open | SQLite bug forum, posted 2026-10-20 (thread id not recorded) |
| [With a window function, a correlated LEFT JOIN whose ON clause has a bare outer column as an AND term filters the left rows](sqlite-window-left-join-outer-on/) | fixed | SQLite bug forum, bug report 2026-10-08T15:38:26Z |
| ["database disk image is malformed" or missing rows: ORDER BY an added column NULLS LAST on a WITHOUT ROWID table](sqlite-wr-bignull-added-column/) | fixed | [link](https://sqlite.org/bugs/forumpost/eee345f931) |
| ["database disk image is malformed" or wrong answer: row-value IN (SELECT ...) over an added column of a WITHOUT ROWID table](sqlite-wr-in-index-added-column/) | open | [link](https://sqlite.org/bugs/forumpost/eee345f931) |
| [sqlite-wr-multi-or-seekscan](sqlite-wr-multi-or-seekscan/) | reported 2026-10-23 | [link](https://sqlite.org/bugs/forumpost/f554fe3656) |
| ["database disk image is malformed" or missing rows: row-value range over an added column of a WITHOUT ROWID table](sqlite-wr-rowvalue-added-column/) | open | [link](https://sqlite.org/bugs/forumpost/eee345f931) |
| [In a UTF-16 database, a zeroblob() result used as text is converted from UTF-8, so CAST(zeroblob(4) AS TEXT) has 8 bytes and the result depends on the query plan](sqlite-zeroblob-cast-text-utf16/) | open | [link](https://sqlite.org/bugs/forumpost/420911fdac) |
