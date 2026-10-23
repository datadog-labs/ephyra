# Bugs

The bugs the case studies found, by project. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

## sqlite

| Bug | Status | Upstream |
|---|---|---|
| [sum(b ORDER BY a) FROM t1, t2 sums a instead of b: an aggregate's ORDER BY term replaces its argument](sqlite/sqlite-aggregate-orderby-replaces-argument/) | fixed | [link](https://sqlite.org/bugs/forumpost/b53a65d1fc) |
| [In a UTF-16 database, a blob DEFAULT from ALTER TABLE ADD COLUMN is converted to text as UTF-8](sqlite/sqlite-alter-default-blob-encoding/) | fixed | [link](https://sqlite.org/bugs/forumpost/a9c654a346) |
| [UNION of four or more SELECTs drops rows: the balanced merge removes the right half's duplicates under that half's collation](sqlite/sqlite-balanced-merge-dedup-collation/) | open | SQLite bug forum, posted 2026-10-18 (thread id not recorded) |
| [A join on an INTEGER PRIMARY KEY misses the REAL key -9223372036854775808.0 when the plan has a Bloom filter](sqlite/sqlite-bloom-filter-rowid-real-minint/) | fixed | [link](https://sqlite.org/bugs/forumpost/403ca95409) |
| [Compound SELECT with ORDER BY ... COLLATE moves the last SELECT's GROUP BY, HAVING and DISTINCT to the whole compound](sqlite/sqlite-compound-orderby-collate-groupby/) | open | SQLite bug forum, posted 2026-10-16 (thread id not recorded) |
| [concat() and concat_ws() of a BLOB in a UTF-16 database copy the BLOB's size, not the text's: a NUL and stale bytes in the result](sqlite/sqlite-concat-blob-utf16-length/) | reported by others | [link](https://sqlite.org/bugs/forumpost/c621e963a9) |
| [DELETE reports "database disk image is malformed" on a database that passes integrity_check (WITHOUT ROWID, tiny key)](sqlite/sqlite-delete-corrupt/) | fixed upstream before we reported it |  |
| ["database disk image is malformed" from a write after a one-pass DELETE in the same statement (WITHOUT ROWID): the delete cursor keeps the deleted cell's CellInfo](sqlite/sqlite-delete-saveposition-stale-cellinfo/) | open | SQLite bug forum, posted 2026-10-18 (thread id not recorded) |
| [ALTER TABLE DROP CONSTRAINT on a foreign key with SET DEFAULT, NOT DEFERRABLE or MATCH fails or leaves part of the constraint behind](sqlite/sqlite-drop-constraint-fk-set-default/) | open | [link](https://sqlite.org/bugs/forumpost/5e80e94512) |
| [ALTER COLUMN DROP NOT NULL removes NOT DEFERRABLE instead of NOT NULL, or removes a DEFERRABLE clause that follows NOT NULL](sqlite/sqlite-drop-not-null-not-deferrable/) | open | [link](https://sqlite.org/bugs/forumpost/28af38d7c3) |
| [Join drops rows, or adds some, when a table has an index on an expression and a `col = constant` WHERE term](sqlite/sqlite-expr-index-constant-propagation/) | fixed | [link](https://sqlite.org/bugs/forumpost/529732a774) |
| [A UNION ALL subquery in FROM with a RIGHT JOIN after its first SELECT loses WHERE terms when flattened](sqlite/sqlite-flatten-compound-right-join/) | fixed | SQLite bug forum, bug report 2026-10-08T15:27:33Z |
| [sqlite-fts5-and-nomatch-clears-later-row](sqlite/sqlite-fts5-and-nomatch-clears-later-row/) | reported 2026-10-23 | [link](https://sqlite.org/bugs/forumpost/5cffabd8e9) |
| [FTS5: DROP TABLE and ALTER TABLE RENAME skip the %_content table of a contentless_unindexed=1 table](sqlite/sqlite-fts5-contentless-unindexed-drop-rename/) | open | SQLite bug forum, posted 2026-10-21 (thread id not recorded) |
| [FTS5: with detail=none, secure-delete of a rowid that was replaced while secure-delete was off corrupts the index](sqlite/sqlite-fts5-detail-none-secure-delete-replaced-rowid/) | open | [link](https://sqlite.org/bugs/forumpost/e380b6876c) |
| [FTS5: "no such cursor: 0" from an auxiliary function in WHERE with the `tbl = 'query'` form (constant propagation)](sqlite/sqlite-fts5-eq-constprop-aux-cursor/) | open | SQLite bug forum, posted 2026-10-20 (thread id not recorded) |
| [FTS5: INSERT into a locale=1, contentless_unindexed=1 table with an UNINDEXED column fails with "database disk image is malformed"](sqlite/sqlite-fts5-locale-contentless-unindexed-insert/) | open | SQLite bug forum, posted 2026-10-21 (thread id not recorded) |
| [FTS5: MATCH with `rowid = <non-integer value>` returns every match (regression after 3.51)](sqlite/sqlite-fts5-match-rowid-eq-nonint/) | open | SQLite bug forum, posted 2026-10-20 (thread id not recorded) |
| [FTS5: phrases of a NEAR group that does not match a row report instances for that row, some left over from an earlier row](sqlite/sqlite-fts5-near-nomatch-stale-poslist/) | open | [link](https://sqlite.org/bugs/forumpost/9fa08e2dbd) |
| [FTS5: the right operand of a NOT reports instances when the left operand matches no row](sqlite/sqlite-fts5-not-eof-rhs-instances/) | open | [link](https://sqlite.org/bugs/forumpost/5cffabd8e9) |
| [FTS5: after secure-delete removes the last rowids of a long doclist, a query with a rowid lower bound returns rows of the next term](sqlite/sqlite-fts5-secure-delete-dlidx-walks-next-term/) | fixed upstream before we reported it |  |
| [FTS5: with secure-delete on, a merge can drop a delete marker, and a later secure delete makes an old entry for the rowid match again](sqlite/sqlite-fts5-secure-delete-merge-drops-marker/) | open | [link](https://sqlite.org/bugs/forumpost/737cb495ea) |
| [FTS5 tokendata=1: a query for an absent term fails with "fts5: corruption found reading blob" after a long doclist](sqlite/sqlite-fts5-tokendata-missing-term-dlidx-corrupt/) | open | SQLite bug forum, posted 2026-10-21 (thread id not recorded) |
| [sqlite-fts5-tokendata-prefix-corrupt](sqlite/sqlite-fts5-tokendata-prefix-corrupt/) | reported 2026-10-23 | [link](https://sqlite.org/bugs/forumpost/5c53f75fbb) |
| [FTS5 trigram: a LIKE pushed down to FTS5 makes sqlite3Fts5ExprPattern() read one byte past the end of the pattern](sqlite/sqlite-fts5-trigram-like-pattern-overread/) | open | [link](https://sqlite.org/bugs/forumpost/402c7177cb) |
| [sqlite-fts5-trigram-like-unindexed](sqlite/sqlite-fts5-trigram-like-unindexed/) | reported 2026-10-23 | [link](https://sqlite.org/bugs/forumpost/be50ab5692) |
| [FTS5 detail=none/column: snippet() fails with "database disk image is malformed", highlight() and bm25() see matches in UNINDEXED columns](sqlite/sqlite-fts5-unindexed-detail-none-instances/) | open | SQLite bug forum, posted 2026-10-20 (thread id not recorded) |
| [sqlite-fts5vocab-rowid-or-right-join](sqlite/sqlite-fts5vocab-rowid-or-right-join/) | reported 2026-10-23 | [link](https://sqlite.org/bugs/forumpost/d29f636f37) |
| [FTS5: in a UTF-16 database, fts5vocab applies `term <` / `>` and ORDER BY term in UTF-8 order](sqlite/sqlite-fts5vocab-term-range-utf16-order/) | open | [link](https://sqlite.org/bugs/forumpost/b0c03988e8) |
| [An index on a VIRTUAL generated column with no type returns an integral REAL as an INTEGER](sqlite/sqlite-gencol-index-real-int/) | fixed | [link](https://sqlite.org/bugs/forumpost/6973ad169e) |
| [Geopoly: `rowid = <non-integer value>` finds the row of its integer part](sqlite/sqlite-geopoly-rowid-eq-nonint/) | open | SQLite bug forum, posted 2026-10-21 (thread id not recorded) |
| [Geopoly: an UPDATE that changes the rowid but not _shape fails with "_shape does not contain a valid polygon"](sqlite/sqlite-geopoly-update-rowid-nochange/) | open | SQLite bug forum, posted 2026-10-21 (thread id not recorded) |
| [GLOB 'é*' on an indexed untyped column misses an over-length UTF-8 blob that a scan finds (invalid UTF-8, garbage in, garbage out)](sqlite/sqlite-glob-index-blob/) | not a bug |  |
| [group_concat() window over a moving frame drops separators and text after an empty string](sqlite/sqlite-group-concat-inverse-empty-text/) | fixed | SQLite bug forum, posted 2026-10-16 (thread id not recorded) |
| [`x IN (SELECT …)` in HAVING converts the GROUP BY term x by the IN's affinity for the whole query (regression in 3.15.0)](sqlite/sqlite-group-term-in-select-affinity/) | reported by others | SQLite, reported by someone else 2026-07-29; fixed by check-in b9ecd9caf7 |
| [Rows out of order despite ORDER BY x DESC in a query with GROUP BY x over a subquery sorted by x ASC](sqlite/sqlite-groupby-subquery-orderby-direction/) | fixed | [link](https://sqlite.org/bugs/forumpost/5cd4b3b27a) |
| [x IN (SELECT y COLLATE NOCASE ...) misses rows when x has an index](sqlite/sqlite-in-select-collate-index/) | fixed | [link](https://sqlite.org/bugs/forumpost/f505c400cf) |
| [Rows missing when a UNION ALL subquery or CTE is joined with an inner join followed by a RIGHT or FULL JOIN](sqlite/sqlite-intersect-full-join-cte/) | fixed | [link](https://sqlite.org/bugs/forumpost/50dd78a44b) |
| [A correlated subquery over a three-way join counts 0 for later outer rows (JoinMiss deletes automatic-index entries)](sqlite/sqlite-joinmiss-autoindex-subquery/) | fixed | [link](https://sqlite.org/bugs/forumpost/8e5c07be86) |
| [json_valid() returns 0 for valid JSON text based on json_set call](sqlite/sqlite-json-valid-stale-cache-json5/) | fixed | [link](https://sqlite.org/bugs/forumpost/1339f93b28) |
| [LEFT JOIN with `x = NULL AND y IN (…)` on a two-column index returns a matched row as well as the NULL-extended one (regression in 3.43.0)](sqlite/sqlite-left-join-in-null/) | fixed | SQLite bug forum, posted 2026-10-07 (thread id not recorded) |
| [An OR in a LEFT JOIN's ON clause drives a loop on the left table: wrong rows, or a segfault when the joined table is omitted](sqlite/sqlite-leftjoin-on-or-omitted-table-segv/) | open | [link](https://sqlite.org/bugs/forumpost/090de3eb79) |
| [a IN (...) AND c GLOB 'A' with an index on (a,c): GROUP BY splits a group, DISTINCT repeats a row (BLOB in c)](sqlite/sqlite-like-opt-in-order/) | fixed upstream before we reported it |  |
| [NOT INDEXED is ignored on WITHOUT ROWID tables: the planner still uses secondary and UNIQUE indexes](sqlite/sqlite-notindexed-without-rowid/) | open | [link](https://sqlite.org/bugs/forumpost/cd41fceaa0) |
| [An OR term that uses a partial index WHERE a=5 and also tests a fails with "internal query planner error"](sqlite/sqlite-or-partial-index-covering-error/) | fixed | [link](https://sqlite.org/bugs/forumpost/3ede79ce42) |
| [In a UTF-16 database, a blob over 4000 bytes read from a table's overflow pages is converted as UTF-8](sqlite/sqlite-overflow-blob-encoding/) | fixed | [link](https://sqlite.org/bugs/forumpost/48c64d8bab) |
| [In a UTF-16 database, text over 4000 bytes read from a table loses a leading U+FEFF, and an UPDATE then writes it back without it](sqlite/sqlite-overflow-text-bom/) | fixed | [link](https://sqlite.org/bugs/forumpost/36a73e8c8a) |
| [A partial index WHERE col = <constant> makes col read back as the constant: a TEXT '5.0' comes back as '5'](sqlite/sqlite-partial-index-constant-affinity/) | open | [link](https://sqlite.org/bugs/forumpost/35992eec46) |
| [WHERE b = -9223372036854775808.0 AND typeof(b) = 'integer' misses a row whose INT column b holds that integer](sqlite/sqlite-propagate-const-real-minint/) | open | [link](https://sqlite.org/bugs/forumpost/9bbaa34eea) |
| [sqlite-rename-column-ipk-unique](sqlite/sqlite-rename-column-ipk-unique/) | reported 2026-10-23 | [link](https://sqlite.org/bugs/forumpost/b088eb5b02) |
| [`SELECT *` over a USING join left of a RIGHT or FULL JOIN is "ambiguous column name" when a later table has that column](sqlite/sqlite-right-join-using-star-ambiguous/) | open | [link](https://sqlite.org/bugs/forumpost/8be1215e2c) |
| [A WHERE-clause OR on a table left of a RIGHT JOIN drives a MULTI-INDEX OR and is never tested (extra rows)](sqlite/sqlite-right-join-where-or-multi-index/) | not reported upstream |  |
| [Row-value IN in a join repeats rows: the inner loop iterates the full IN list, with duplicates, for one of its columns](sqlite/sqlite-rowvalue-in-join-reuse/) | open | SQLite bug forum, posted 2026-10-16 (thread id not recorded) |
| [R-Tree: `UPDATE ... SET rowid = N` and `INSERT INTO ...(rowid, ...)` ignore the new rowid](sqlite/sqlite-rtree-update-set-rowid-ignored/) | open | SQLite bug forum, posted 2026-10-21 (thread id not recorded) |
| [ROLLBACK TO of a savepoint that dropped a table commits a stale root page in an incremental-vacuum database](sqlite/sqlite-savepoint-autovacuum/) | fixed | [link](https://sqlite.org/bugs/forumpost/3bc3c8adfa) |
| [ALTER TABLE RENAME COLUMN of a TEMP table fails when a TEMP trigger or view uses the column](sqlite/sqlite-temp-rename-column-twice/) | open | SQLite bug forum, posted 2026-10-19 (thread id not recorded) |
| [UPDATE changes REAL -9223372036854775808.0 in a column it does not assign to INTEGER, but not in the index](sqlite/sqlite-update-real-minint-stale-index/) | not reported upstream |  |
| ["SQL logic error" on the next SAVEPOINT or COMMIT, and the transaction is lost, after ROLLBACK TO undoes a CREATE VIRTUAL TABLE (fts5, fts4)](sqlite/sqlite-vtab-create-rollback-to-savepoint/) | open | SQLite bug forum, posted 2026-10-20 (thread id not recorded) |
| [With a window function, a correlated LEFT JOIN whose ON clause has a bare outer column as an AND term filters the left rows](sqlite/sqlite-window-left-join-outer-on/) | fixed | SQLite bug forum, bug report 2026-10-08T15:38:26Z |
| ["database disk image is malformed" or missing rows: ORDER BY an added column NULLS LAST on a WITHOUT ROWID table](sqlite/sqlite-wr-bignull-added-column/) | fixed | [link](https://sqlite.org/bugs/forumpost/eee345f931) |
| ["database disk image is malformed" or wrong answer: row-value IN (SELECT ...) over an added column of a WITHOUT ROWID table](sqlite/sqlite-wr-in-index-added-column/) | open | [link](https://sqlite.org/bugs/forumpost/eee345f931) |
| [sqlite-wr-multi-or-seekscan](sqlite/sqlite-wr-multi-or-seekscan/) | reported 2026-10-23 | [link](https://sqlite.org/bugs/forumpost/f554fe3656) |
| ["database disk image is malformed" or missing rows: row-value range over an added column of a WITHOUT ROWID table](sqlite/sqlite-wr-rowvalue-added-column/) | open | [link](https://sqlite.org/bugs/forumpost/eee345f931) |
| [In a UTF-16 database, a zeroblob() result used as text is converted from UTF-8, so CAST(zeroblob(4) AS TEXT) has 8 bytes and the result depends on the query plan](sqlite/sqlite-zeroblob-cast-text-utf16/) | open | [link](https://sqlite.org/bugs/forumpost/420911fdac) |

## turso

| Bug | Status | Upstream |
|---|---|---|
| [ALTER TABLE ADD COLUMN ... DEFAULT -9223372036854775808 reads as REAL for existing rows (and breaks UPDATE on a STRICT table)](turso/turso-add-column-default-min-int-real/) | open | [link](https://github.com/tursodatabase/turso/issues/10391) |
| [ANALYZE on a database without tables does not create sqlite_stat1](turso/turso-analyze-empty-schema-stat1/) | open | [link](https://github.com/tursodatabase/turso/issues/10355) |
| [ANALYZE cannot reach TEMP objects: "ANALYZE temp", "ANALYZE <temp table>" and "ANALYZE <temp index>" fail](turso/turso-analyze-temp-schema/) | fixed | [link](https://github.com/tursodatabase/turso/issues/10340) |
| [BLOB \|\| BLOB returns a BLOB; SQLite returns TEXT (closed #6089 still reproduces)](turso/turso-blob-concat-returns-blob/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/6089) |
| [CAST(text AS NUMERIC) returns an incorrect double for integer text that overflows i64](turso/turso-cast-numeric-large-integer/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/8452) |
| [Compound SELECT: ORDER BY `x ISNULL` does not match a result column written `x IS NULL` (and NOTNULL / NOT NULL / IS NOT NULL)](turso/turso-compound-order-by-isnull-spelling/) | open | [link](https://github.com/tursodatabase/turso/issues/10395) |
| [concat_ws() drops BLOB arguments and returns NULL when the separator is a BLOB](turso/turso-concat-ws-blob/) | open | [link](https://github.com/tursodatabase/turso/issues/10394) |
| [A CTE is not found in a subquery inside a JOIN ON clause](turso/turso-cte-on-clause-subquery/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/9073) |
| [Date/time functions don't stop at an embedded NUL in their text arguments; SQLite reads up to the NUL](turso/turso-date-embedded-nul/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9935) |
| [`±HH:MM` date modifier with hour 24 adds a day; SQLite adds nothing](turso/turso-date-hhmm-modifier-whole-days/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9921) |
| [date/time "NNN units" and "±YYYY-MM-DD" modifiers accept trailing whitespace and trailing text that SQLite rejects](turso/turso-date-modifier-trailing-text/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9837) |
| [Date/time functions don't accept a trailing vertical tab (0x0B) after the time value (like #8454, for dates)](turso/turso-date-vertical-tab/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9968) |
| [A runtime error in any statement rolls back the whole explicit transaction (not only JSON errors)](turso/turso-error-rolls-back-transaction/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/6864) |
| [UPDATE on a parent table fails with "foreign key mismatch" while foreign keys are off](turso/turso-fk-mismatch-parent-dml-foreign-keys-off/) | not reported upstream |  |
| [UPDATE of a child table fails with "foreign key mismatch" even when no foreign key column is assigned](turso/turso-fk-mismatch-update-unrelated-column/) | fixed | [link](https://github.com/tursodatabase/turso/issues/10401) |
| [JSON subtype lost when a result column contains an expression from GROUP BY (follow-up to #5755)](turso/turso-group-by-expression-json-subtype/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9966) |
| ["Parse error: subquery not found" for a WHERE subquery that reads a table outside a materialized hash build input](turso/turso-hash-build-input-subquery-not-found/) | open | [link](https://github.com/tursodatabase/turso/issues/10389) |
| [Inner-join "duplicate cursor key" panic through a materialized hash build input over a table-valued function subquery (wider form of #9847)](turso/turso-hash-build-input-vtab-subquery-duplicate-cursor/) | fixed upstream before we reported it | [link](https://github.com/tursodatabase/turso/issues/9847) |
| [Correlated subquery in the SELECT list reads NULL for a hash-build-side table when the probe side is a LEFT JOIN (outer-join case of #8226)](turso/turso-hash-join-left-join-correlated-subquery-null/) | open | [link](https://github.com/tursodatabase/turso/issues/10388) |
| [A REAL column read through a hash join comes back as an integer (SQLite-created file)](turso/turso-hash-join-real-affinity/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9881) |
| [Aggregates of groups removed by a GROUP BY-only HAVING term still run, so they raise errors SQLite never reaches](turso/turso-having-group-term-not-pushed/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9880) |
| [HAVING with a predicate that is itself a GROUP BY term (`GROUP BY c, a IS NULL HAVING a IS NULL`) filters on the wrong value](turso/turso-having-group-term-predicate/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9887) |
| [A HAVING subquery reads the GROUP BY column itself from the wrong row when it isn't selected](turso/turso-having-subquery-group-column/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/8221) |
| [CREATE INDEX rejects double-quoted string literals that SQLite accepts, so some SQLite databases cannot be opened or written](turso/turso-index-dqs-string-literal/) | open | [link](https://github.com/tursodatabase/turso/issues/10337) |
| [PRAGMA index_xinfo reports the rowid entry's name as '' instead of NULL](turso/turso-index-xinfo-rowid-name-null/) | open | [link](https://github.com/tursodatabase/turso/issues/10354) |
| [`->`/`->>` ignore the `'[N]'` abbreviated path form, so `'["a","b"]' ->> '[0]'` returns NULL](turso/turso-json-arrow-abbreviated-path/) | open | [link](https://github.com/tursodatabase/turso/issues/9845) |
| [`x -> x` and `x ->> x` panic (index out of bounds) when both operands are the same expression](turso/turso-json-arrow-same-operand-panic/) | open | [link](https://github.com/tursodatabase/turso/issues/9836) |
| [JSON functions take a BLOB for JSONB when json_valid(X, 8) says it isn't, and skip leading whitespace bytes before classifying it](turso/turso-json-blob-arg-jsonb-detection/) | not reported upstream |  |
| [JSON functions return the integer for a whole-number REAL when the same integer was seen just before (JSON cache key compares 1 and 1.0 as equal)](turso/turso-json-cache-integer-real-key/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9967) |
| [json_each/json_tree declare column types (fullkey TEXT, key ANY, ...), so comparisons with numbers use the wrong affinity](turso/turso-json-each-column-affinity/) | fixed | [link](https://github.com/tursodatabase/turso/issues/10342) |
| [json_each/json_tree error "number too large to fit in target type" on integers outside the i64 range](turso/turso-json-each-large-integer/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9839) |
| [json_each/json_tree with a NULL path error "root path should be text" (SQLite returns no rows)](turso/turso-json-each-null-path/) | open | [link](https://github.com/tursodatabase/turso/issues/9838) |
| [json_error_position reports a different position than SQLite for text after a complete value and for malformed numbers and literals](turso/turso-json-error-position-token/) | open | [link](https://github.com/tursodatabase/turso/issues/10378) |
| [json_error_position('') is an internal error, and whitespace before end of input shifts the position](turso/turso-json-error-position-trailing-whitespace/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9841) |
| [json() rewrites every "9.0e+999" in its output, even inside strings; json_set/json_insert write an infinite REAL as 9.0e+999 where SQLite writes 9e999](turso/turso-json-infinity-rendering/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9927) |
| [json_insert / jsonb_insert with path '$' replace the whole document; SQLite leaves it unchanged](turso/turso-json-insert-root-path/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9924) |
| [JSON functions write a negative-zero REAL as `-0.0`; SQLite writes `0.0`](turso/turso-json-negative-zero/) | open | [link](https://github.com/tursodatabase/turso/issues/9840) |
| [json_patch/jsonb_patch decode the escapes in the keys they add, and jsonb_patch writes a key with a raw control character as plain TEXT](turso/turso-json-patch-escaped-key/) | open | [link](https://github.com/tursodatabase/turso/issues/10370) |
| [json_patch/jsonb_patch with a repeated key in the patch apply members out of order, and jsonb_patch returns a malformed blob](turso/turso-json-patch-repeated-key/) | open | [link](https://github.com/tursodatabase/turso/issues/9843) |
| [json_pretty() prints an empty object or array on two lines](turso/turso-json-pretty-empty-container/) | open | [link](https://github.com/tursodatabase/turso/issues/9885) |
| [json_set/json_insert/json_replace/json_remove: a JSON `null` result becomes SQL NULL, and NULL arguments aren't handled like SQLite (follow-up to #5750)](turso/turso-json-set-null-handling/) | open | [link](https://github.com/tursodatabase/turso/issues/9844) |
| [JSON5 // comment ends only at LF; SQLite also ends it at CR, U+2028 and U+2029](turso/turso-json5-line-comment-terminator/) | fixed | [link](https://github.com/tursodatabase/turso/issues/10343) |
| [Appending to a top-level JSON array loses data (json_set/json_insert with '$[#]')](turso/turso-jsonb-root-array-append/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/8538) |
| [An ON clause with IS drops FROM-subquery materialization (quadratic LEFT JOIN subquery)](turso/turso-left-join-subquery-is-coroutine/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/8322) |
| [likelihood() rejects a float literal written with an exponent (625e-4, 1e0) or in parentheses](turso/turso-likelihood-exponent-literal/) | open | [link](https://github.com/tursodatabase/turso/issues/10338) |
| [Panic "cursor id N is None" when a materialized CTE is first read inside a nested subquery's loop and then read by the outer query](turso/turso-materialized-cte-first-filled-in-subquery-loop/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9882) |
| ["Parse error: all before-loop subqueries should have already been emitted" for a materialized CTE with a correlated subquery, read from a nested subquery](turso/turso-materialized-cte-subquery-before-loop/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9848) |
| [A result column `-9223372036854775808` becomes REAL when ORDER BY / GROUP BY has `-9.223372036854776e18` (and the other way round)](turso/turso-minint-literal-order-by-equivalence/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9925) |
| [OR over an index and the rowid misses rows: a seek on the table cursor leaves an earlier DeferredSeek pending](turso/turso-multi-index-or-stale-deferred-seek/) | not reported upstream |  |
| [Panic "column N of table tN is not stored in the index and the table cursor is not open" with a partial index and a BETWEEN that implies its WHERE](turso/turso-partial-index-between-covering-panic/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9926) |
| [Panic "column N of table tN is not stored in the index and the table cursor is not open" with a partial index, a seek on its key and a join on that key](turso/turso-partial-index-join-covering-panic/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9934) |
| [(comment on #10339) pragma_quick_check / pragma_integrity_check also reject their table-name argument](turso/turso-pragma-check-tvf-arg-comment/) | not reported upstream |  |
| [pragma_quick_check / pragma_integrity_check name their column "message" instead of "quick_check" / "integrity_check"](turso/turso-pragma-check-tvf-column-name/) | open | [link](https://github.com/tursodatabase/turso/issues/10339) |
| [quote() writes a REAL in [1, 10) that needs 16+ digits with exponent "e-00"; SQLite 3.50.4 writes "e+00"](turso/turso-quote-real-exponent-zero/) | fixed by an open pull request | [link](https://github.com/tursodatabase/turso/pull/9534) |
| [Text-to-REAL conversion through column affinity and sum()/total()/avg() gives a different double than CAST or a literal](turso/turso-real-affinity-parse/) | open | [link](https://github.com/tursodatabase/turso/issues/9849) |
| [-0.0 stored in a REAL column keeps its sign; SQLite reads it back as +0.0](turso/turso-real-column-negative-zero/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9889) |
| [replace(X, '', NULL) returns NULL instead of X](turso/turso-replace-empty-pattern-null-replacement/) | fixed | [link](https://github.com/tursodatabase/turso/issues/10426) |
| [RIGHT JOIN followed by LEFT JOIN: a subquery in the later ON clause is not re-evaluated for unmatched right rows (panic or wrong result)](turso/turso-right-join-on-subquery-unmatched-rows/) | open | [link](https://github.com/tursodatabase/turso/issues/9850) |
| [Panic "cursor id N is None" in a RIGHT JOIN's unmatched-rows pass when a WHERE subquery has a FROM subquery and a nested subquery that reads it](turso/turso-right-join-unmatched-from-subquery-cursor/) | open | [link](https://github.com/tursodatabase/turso/issues/10390) |
| [Panic "duplicate cursor key" when a RIGHT/FULL JOIN query has a WHERE subquery over a table-valued function](turso/turso-right-join-vtab-subquery-duplicate-cursor/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9847) |
| [Panic "a WITHOUT ROWID table has no secondary index to drive its loop" in a RIGHT/FULL JOIN on a SQLite-created file](turso/turso-right-join-without-rowid-secondary-index/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9852) |
| [(comment on #9051) Row-value BETWEEN also fails in a join's ON clause](turso/turso-rowvalue-between-join-comment/) | not reported upstream |  |
| [(comment on #9087) The later row-value components aren't evaluated once an earlier one is NULL, so the result uses the previous row's values](turso/turso-rowvalue-in-subquery-null-lhs-comment/) | not reported upstream |  |
| [`schema.alias.column` that refers to an outer query's table is not found in a subquery ("no such column: main.x.a")](turso/turso-schema-qualified-alias-outer-column/) | open | [link](https://github.com/tursodatabase/turso/issues/10396) |
| [sqlite_schema.sql holds a re-serialized statement instead of the original text, and keeps IF NOT EXISTS](turso/turso-schema-sql-text-reformatted/) | open | [link](https://github.com/tursodatabase/turso/issues/10398) |
| [A column qualified with sqlite_master is not found when FROM names sqlite_schema (and the reverse)](turso/turso-schema-table-alias-qualifier/) | open | [link](https://github.com/tursodatabase/turso/issues/10353) |
| [soundex() returns '?000' for any input with a non-letter, and codes H/W differently from SQLite](turso/turso-soundex-non-letters/) | open | [link](https://github.com/tursodatabase/turso/issues/10356) |
| [strftime() prints an overflowing day as is (2024-02-30); date() and SQLite normalize it to 2024-03-01](turso/turso-strftime-day-overflow/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9969) |
| [strftime() returns a value when the modifiers move the date out of range; date() and SQLite return NULL](turso/turso-strftime-out-of-range/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9884) |
| [strftime(): `%J` and a REAL format argument are printed with more digits than SQLite](turso/turso-strftime-real-formatting/) | open | [link](https://github.com/tursodatabase/turso/issues/9888) |
| [STRICT type check runs before NOT NULL, so INSERT OR IGNORE fails on a row SQLite skips](turso/turso-strict-type-check-before-not-null/) | open | [link](https://github.com/tursodatabase/turso/issues/10392) |
| [(comment on #9572) The missing comma also corrupts a SQLite-made STRICT, WITHOUT ROWID table on ALTER TABLE](turso/turso-strict-without-rowid-alter-comment/) | not reported upstream |  |
| [An uncorrelated scalar subquery runs before the scan, so it raises errors SQLite never reaches](turso/turso-subquery-hoisted-eval/) | open | [link](https://github.com/tursodatabase/turso/issues/9851) |
| [(comment on #9046) sum() overflow is also not cleared by a later BLOB or non-numeric TEXT value](turso/turso-sum-overflow-blob-text-comment/) | not reported upstream |  |
| [(comment on #7644) The TEMP trigger also blocks RENAME COLUMN on unrelated tables](turso/turso-temp-trigger-rename-column-comment/) | not reported upstream |  |
| [timediff() gives wrong (sometimes malformed) results for dates with an overflowing day such as 2023-02-29](turso/turso-timediff-day-overflow/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9886) |
| [unhex(X, Y) returns NULL when Y is an INTEGER, REAL or BLOB instead of using its text](turso/turso-unhex-non-text-ignore-arg/) | fixed | [link](https://github.com/tursodatabase/turso/issues/10400) |
| [An unused CTE is resolved, and its errors reported, when the statement has a subquery outside FROM](turso/turso-unused-cte-eager-resolve/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9842) |
| [UPDATE with a BEFORE UPDATE trigger writes an index key from a REAL column without REAL affinity, corrupting the index (SQLite-created file)](turso/turso-update-before-trigger-real-affinity-index/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9933) |
| [UPDATE through a covering index writes the row without finishing the deferred seek when a partial index WHERE reads only the rowid (panic or duplicate rowids)](turso/turso-update-partial-index-rowid-unpositioned-insert/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9932) |
| [UPSERT DO UPDATE is compiled even when the INSERT cannot conflict; errors in it fail the INSERT, SQLite skips them](turso/turso-upsert-do-update-compiled-without-conflict/) | open | [link](https://github.com/tursodatabase/turso/issues/10393) |
| [VACUUM INTO ':memory:' writes a file named ":memory:" in the current directory; VACUUM INTO '' is rejected](turso/turso-vacuum-into-memory-target/) | fixed | [link](https://github.com/tursodatabase/turso/issues/10341) |
| [PRAGMA table_info / table_list keep a view's old column list after its table changes (ADD COLUMN, DROP TABLE) until the database is reopened](turso/turso-view-columns-stale-after-alter/) | open | [link](https://github.com/tursodatabase/turso/issues/10397) |
| [(comment on #8336) Same panic when the window call's argument is non-deterministic and the call appears twice, also from first_value/nth_value](turso/turso-window-call-repeated-nondeterministic-arg-comment/) | not reported upstream |  |
| [group_concat() over a sliding window frame panics ("GroupConcat accumulator is Text after a non-NULL xStep") when the frame holds an empty string](turso/turso-window-group-concat-inverse-empty-string/) | open | [link](https://github.com/tursodatabase/turso/issues/10352) |
| ["Parse error: table not found" for ORDER BY rowid, then a min/max window column with a non-deterministic window ORDER BY](turso/turso-window-minmax-order-by-table-not-found/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9883) |
| [WITHOUT ROWID INSERT applies column affinities in schema order (false UNIQUE failures)](turso/turso-without-rowid-affinity-order/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/6749) |
| [A join into a WITHOUT ROWID table through an automatic index loses rows when no primary-key column is read](turso/turso-without-rowid-autoindex-dedup/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/9081) |
| [Secondary index read of a SQLite-made WITHOUT ROWID table returns NULL for non-indexed columns](turso/turso-without-rowid-index-null-columns/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/7273) |
| [PRAGMA index_list / index_info / index_xinfo don't show the primary-key index of a WITHOUT ROWID table](turso/turso-without-rowid-pk-index-list/) | fixed | [link](https://github.com/tursodatabase/turso/issues/10399) |
| [RENAME COLUMN of a WITHOUT ROWID table's primary-key column moves it to the end of the key: reads return permuted values and INSERT writes permuted rows](turso/turso-without-rowid-rename-pk-column-order/) | open | [link](https://github.com/tursodatabase/turso/issues/10387) |

## postgresql

| Bug | Status | Upstream |
|---|---|---|
| [ALTER COLUMN TYPE fails on an INCLUDE column of an index used by a foreign key](postgresql/pg-alter-type-include-fk-index/) | open | BUG #19791 |
| [btree_gist date_ops: inserting infinity/-infinity into a multi-page GiST index on a date fails with "cannot subtract infinite dates"](postgresql/pg-btree-gist-infinite-date/) | open | [link](https://www.postgresql.org/message-id/19761-b72cfc453e487610%40postgresql.org) |
| [CLUSTER / REPACK silently loses a row updated in a rolled-back subtransaction](postgresql/pg-cluster-subxact-row-loss/) | open | [link](https://www.postgresql.org/message-id/19760-8f9a0eba17bc8c92%40postgresql.org) |
| ["cannot start a subtransaction when there are unprocessed inval messages" after ALTER DOMAIN DROP CONSTRAINT](postgresql/pg-domain-inval-subxact/) | open | BUG #19790 |
| [Hashed set operations and recursive UNION pre-size their hash table from the planner's estimate without the hash_mem cap (PostgreSQL 19 regression)](postgresql/pg-hashsetop-out-of-memory/) | open | [link](https://www.postgresql.org/message-id/19759-acd3f33402e54f36%40postgresql.org) |
| [Nested BETWEEN grows exponentially in parse analysis and statement_timeout cannot stop it](postgresql/pg-huge-expr-timeout/) | open | [link](https://www.postgresql.org/message-id/19758-d074798b35cc9215%40postgresql.org) |
| [JSON_TABLE with a subquery in PASSING fails with "unexpected non-SELECT command in SubLink"](postgresql/pg-json-table-passing-sublink/) | open | [link](https://www.postgresql.org/message-id/19786-70758103a4b2bff2%40postgresql.org) |
| [pg-minmax-subquery-relid](postgresql/pg-minmax-subquery-relid/) |  |  |
| ["corrupt MVNDistinct entry" (XX000) from GROUP BY with grouping sets over a column and an expression of it](postgresql/pg-mvndistinct-grouping-sets/) | open | BUG #19772 |
| [EXPLAIN ANALYZE: Assert("toc->toc_entry[i].key != key") in shm_toc_insert (master only, since a241bc4fede)](postgresql/pg-parallel-material-dup-toc/) | open | [link](https://www.postgresql.org/message-id/19766-c3d105a4e0ec5487%40postgresql.org) |
| [UPDATE moving a row of a self-referencing partitioned table fails with 23503 although ON UPDATE CASCADE would fix the reference](postgresql/pg-partition-selfref-cascade-move/) | open | BUG #19781 |
| ["could not find pathkey item to sort" for a Gather Merge over a merge join after outer-join reordering](postgresql/pg-pathkey-item-norec-join/) | open | [link](https://www.postgresql.org/message-id/19787-d14d7f0f320f4495%40postgresql.org) |
| [PostgreSQL: candidate from today's round, report drafted, not reported](postgresql/pg-plans-sortagg-cte/) | open | BUG #19783 |
| [SET CONSTRAINTS in a released savepoint survives ROLLBACK TO an enclosing savepoint](postgresql/pg-set-constraints-released-savepoint/) | open | BUG #19782 |
| ["expression not found in statistics object" with MCV statistics on a domain over boolean](postgresql/pg-statext-bool-domain/) | open | BUG #19789 |
| [pg_get_viewdef prints a negated integer constant in ORDER BY / GROUP BY as a column position](postgresql/pg-viewdef-negated-sort-constant/) | open | [link](https://www.postgresql.org/message-id/19767-f1189fd6a83254b7%40postgresql.org) |

## ada

| Bug | Status | Upstream |
|---|---|---|
| [`parse("C\|", file_base)` marks the URL as having an opaque path: host/path setters fail and it cannot be a base](ada/ada-file-drive-letter-opaque-path/) | open | [link](https://github.com/ada-url/ada/issues/1278) |
| [combining-mark, bidi and normalization tables predate the Unicode 17 mapping table: leading marks and RTL letters from Unicode 14+ pass, Unicode 16 compositions are not applied](ada/ada-idna-stale-unicode-tables/) | fixed | [link](https://github.com/ada-url/idna/issues/97) |
| [`url_search_params` keeps ill-formed UTF-8 from percent-decoding instead of decoding it to U+FFFD](ada/ada-search-params-ill-formed-utf8/) | declined | [link](https://github.com/ada-url/ada/issues/1279) |
| [`url_aggregator::set_host`/`set_hostname` exceed `set_max_input_length` with IDNA-expanding hosts (regression from #1262)](ada/ada-set-host-idna-expansion-limit/) | open | [link](https://github.com/ada-url/ada/issues/1326) |
| [`set_host("host:port")` under `set_max_input_length` applies the host, drops the port, and reports success](ada/ada-set-host-port-length-limit/) | open | [link](https://github.com/ada-url/ada/issues/1282) |
| [URLPattern opaque pathname: a leading `//` is parsed as an authority and dropped (`//x/y` becomes `/y`)](ada/ada-urlpattern-opaque-pathname-authority/) | open | [link](https://github.com/ada-url/ada/issues/1284) |
| [URLPattern `generate_pattern_string` drops the braces around a part whose prefix starts with `/` but is longer, so the pathname string no longer compiles to the same pattern](ada/ada-urlpattern-pattern-string-prefix-grouping/) | fixed | [link](https://github.com/ada-url/ada/issues/1327) |
| [`canonicalize_protocol` indexes an empty string_view on a protocol pattern of an escaped colon (aborts in hardened builds)](ada/ada-urlpattern-protocol-colon-oob/) | open | [link](https://github.com/ada-url/ada/issues/1283) |
| [URLPattern search canonicalization uses the non-special query encode set: a search pattern containing `'` never matches a special URL](ada/ada-urlpattern-search-special-query-encode/) | open | [link](https://github.com/ada-url/ada/issues/1285) |
| [URLPattern: a literal backslash in a special-scheme pathname pattern is not canonicalized to "/" (spec's dummy URL is special)](ada/ada-urlpattern-special-pathname-backslash/) | fixed | [link](https://github.com/ada-url/ada/issues/1281) |
| [URLPattern test/exec/match resolve a relative string input against an empty URL when no base is given (should not match)](ada/ada-urlpattern-string-input-default-base/) | fixed | [link](https://github.com/ada-url/ada/issues/1280) |
| [x86-64 builds execute SSSE3 instructions unconditionally since #1216/#1230 (illegal instruction on CPUs without SSSE3)](ada/ada-x86-ssse3-without-dispatch/) | reported by others | [link](https://github.com/ada-url/ada/pull/1321) |

## duckdb

| Bug | Status | Upstream |
|---|---|---|
| [duckdb-abs-double-stats-inverted-at-zero-bound](duckdb/duckdb-abs-double-stats-inverted-at-zero-bound/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27711) |
| [duckdb-buffered-cte-consumer-cycle-undetected-d](duckdb/duckdb-buffered-cte-consumer-cycle-undetected-d/) | reported 2026-10-23 | [link](https://github.com/duckdb/duckdb/issues/27737) |
| [EXPLAIN (SQL) exports quantile_disc with a column reference as the quantile argument when expression_rewriter is disabled](duckdb/duckdb-cse-hoists-constant-quantile-argument/) | open | [link](https://github.com/duckdb/duckdb/issues/27776) |
| [duckdb-flush-caching-operators-continues-into-f](duckdb/duckdb-flush-caching-operators-continues-into-f/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27722) |
| [INTERNAL Error "GetPartitioningSpaceRequirement: tuple width should not be 0" for a hash join whose probe side has no columns](duckdb/duckdb-hash-join-zero-column-probe-side-partiti/) | fixed | [link](https://github.com/duckdb/duckdb/issues/27450) |
| [duckdb-in-list-treated-as-null-propagating-sing](duckdb/duckdb-in-list-treated-as-null-propagating-sing/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27712) |
| [duckdb-join-condition-stats-narrowed-by-sibling](duckdb/duckdb-join-condition-stats-narrowed-by-sibling/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27725) |
| [INTERNAL Error in join elimination (`!column_bindings.empty()`) on a correlated subquery with a never-evaluated `count(DISTINCT ...)`](duckdb/duckdb-join-elimination-asserts-on-conditionles/) | open | [link](https://github.com/duckdb/duckdb/issues/27445) |
| [INTERNAL Error "Projection map references a missing child column": join elimination leaves the parent filter's projection map stale](duckdb/duckdb-join-elimination-stale-parent-projection/) | open | [link](https://github.com/duckdb/duckdb/issues/27449) |
| [Join order optimizer drops a filter that references no column (`WHERE NULL`, `WHERE random() > 2`) when filter_pushdown is disabled](duckdb/duckdb-join-order-drops-relation-free-filter/) | fixed | [link](https://github.com/duckdb/duckdb/issues/27451) |
| [duckdb-join-order-filter-projection-map-hides-p](duckdb/duckdb-join-order-filter-projection-map-hides-p/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27723) |
| [INTERNAL Error "Failed to bind column reference" when an always-matching SEMI JOIN (`ON true`) is reordered by the join order optimizer](duckdb/duckdb-join-order-stale-projection-map-after-se/) | open | [link](https://github.com/duckdb/duckdb/issues/27452) |
| [duckdb-join-stats-clobbered-by-ungrouped-aggreg](duckdb/duckdb-join-stats-clobbered-by-ungrouped-aggreg/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27714) |
| [duckdb-lambda-in-aggregate-in-lambda-gets-outer](duckdb/duckdb-lambda-in-aggregate-in-lambda-gets-outer/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27720) |
| [duckdb-lateral-dependent-join-hoisted-cte-defin](duckdb/duckdb-lateral-dependent-join-hoisted-cte-defin/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27716) |
| [duckdb-left-join-unsatisfiable-condition-dummy-](duckdb/duckdb-left-join-unsatisfiable-condition-dummy-/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27719) |
| [Segfault in `dense_rank() OVER ()` over a RIGHT_ANTI hash join that returns its unmatched rows](duckdb/duckdb-natural-sort-empty-local-collection-dens/) | open | [link](https://github.com/duckdb/duckdb/issues/27448) |
| [duckdb-nested-loop-join-ignores-projection-map-](duckdb/duckdb-nested-loop-join-ignores-projection-map-/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27717) |
| [duckdb-nlj-ignores-projection-map-after-join-su](duckdb/duckdb-nlj-ignores-projection-map-after-join-su/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27717) |
| [duckdb-pair-dependent-full-join-cte-side-column](duckdb/duckdb-pair-dependent-full-join-cte-side-column/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27726) |
| [duckdb-pwmj-ignores-projection-map-after-join-s](duckdb/duckdb-pwmj-ignores-projection-map-after-join-s/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27717) |
| [duckdb-regex-range-blob-bounds-assert-in-string](duckdb/duckdb-regex-range-blob-bounds-assert-in-string/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27724) |
| [duckdb-semi-join-right-only-condition-no-filter-pushdown](duckdb/duckdb-semi-join-right-only-condition-no-filter-pushdown/) | reported 2026-10-21 | [link](https://github.com/duckdb/duckdb/issues/27633) |
| [INTERNAL Error `exprs.size() > 0` (expression_executor.cpp) when statistics propagation folds an aggregate with no aggregate expressions (`HAVING` without aggregates)](duckdb/duckdb-stats-folds-expressionless-aggregate-int/) | fixed | [link](https://github.com/duckdb/duckdb/issues/27446) |
| [INTERNAL Error `physical_index != DConstants::INVALID_INDEX` in propagate_get.cpp when statistics fold a pushed-down filter's column away](duckdb/duckdb-stats-table-filter-loses-column-ref/) | open | [link](https://github.com/duckdb/duckdb/issues/27447) |
| [duckdb-table-filter-true-or-null-becomes-column](duckdb/duckdb-table-filter-true-or-null-becomes-column/) | reported 2026-10-22 | [link](https://github.com/duckdb/duckdb/issues/27715) |

## cedar

| Bug | Status | Upstream |
|---|---|---|
| [`add_entities` compares a new entity's parents with the stored entity's transitive closure: a declared parent can be dropped, and identical entities are refused](cedar/cedar-add-entities-closure-duplicate/) | open | [link](https://github.com/cedar-policy/cedar/issues/2629) |
| [`upsert_entities` keeps a stale ancestor when the batch contains the same entity twice (regression in 4.11.0)](cedar/cedar-upsert-repeated-entity-stale-ancestor/) | fixed | [link](https://github.com/cedar-policy/cedar/issues/2606) |

## sqleq

| Bug | Status | Upstream |
|---|---|---|
| [sqleq-fuzz: a literal outside the generated value domain hides a boundary difference (`334 <= c` vs `334 < c` is NO-COUNTEREXAMPLE)](sqleq/sqleq-fuzz-literal-domain/) | fixed | [link](https://github.com/datadog-labs/sqleq/issues/187) |
| [sqleq-fuzz: a literal compared with an expression of a column, or behind a value-changing cast, still hides a boundary difference](sqleq/sqleq-fuzz-missed-20261021/) | open | [link](https://github.com/datadog-labs/sqleq/issues/207) |
| [frontend: a set operation or VALUES list whose branches have different types reaches the prover ill-typed when the first branch is REAL, BOOLEAN, DATE or opaque (QED panics)](sqleq/sqleq-ill-typed-in-setop/) | fixed | [link](https://github.com/datadog-labs/sqleq/issues/177) |
| [frontend: `NOT` over a demoted boolean operator (`<@`, `@>`, `&&`, `~`, `!~`) outside a WHERE reaches the prover applied to a VARBINARY term (QED panics)](sqleq/sqleq-not-demoted-boolean/) | fixed | [link](https://github.com/datadog-labs/sqleq/issues/186) |

## qed

| Bug | Status | Upstream |
|---|---|---|
| [Unsound: real constants are parsed as `f32`, so distinct literals are proved equal (and wide ones panic)](qed/qed-real-constants/) | open | [link](https://github.com/qed-solver/prover/issues/13) |
| [Panic on INTEGER vs DECIMAL in `IN (subquery)` (normal.rs:440) and `UNION` (partial.rs:185)](qed/qed-sort-panics/) | open | [link](https://github.com/qed-solver/prover/issues/14) |

## coreutils

| Bug | Status | Upstream |
|---|---|---|
| [coreutils-basenc-base64url-decode-rejects-unpadded](coreutils/coreutils-basenc-base64url-decode-rejects-unpadded/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15664) |
| [coreutils-cat-squeeze-blank-drops-blank-after-spli](coreutils/coreutils-cat-squeeze-blank-drops-blank-after-spli/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15655) |
| [coreutils-chmod-leading-dash-mode-followed-by-oper](coreutils/coreutils-chmod-leading-dash-mode-followed-by-oper/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15665) |
| [coreutils-chmod-reference-first-operand-processed-](coreutils/coreutils-chmod-reference-first-operand-processed-/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15666) |
| [comm: sorted input rejected as "not in sorted order" when a line is a prefix of the next followed by a tab](coreutils/coreutils-comm-compares-line-terminator/) | fixed | [link](https://github.com/uutils/coreutils/issues/15439) |
| [coreutils-csplit-negative-offset-regex-hangs-after](coreutils/coreutils-csplit-negative-offset-regex-hangs-after/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15653) |
| [csplit: a `/REGEX/` pattern after a line-number pattern never tests the line the split stopped at](coreutils/coreutils-csplit-regex-skips-line-after-line-numbe/) | open | [link](https://github.com/uutils/coreutils/issues/15440) |
| [coreutils-csplit-suppress-matched-line-just-past-e](coreutils/coreutils-csplit-suppress-matched-line-just-past-e/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15584) |
| [coreutils-cut-output-delimiter-repeats-last-field-](coreutils/coreutils-cut-output-delimiter-repeats-last-field-/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15654) |
| [coreutils-dd-swab-pairs-reset-per-read-block](coreutils/coreutils-dd-swab-pairs-reset-per-read-block/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15646) |
| [coreutils-expand-column-reset-at-file-and-buffer-b](coreutils/coreutils-expand-column-reset-at-file-and-buffer-b/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15656) |
| [coreutils-expand-nul-counted-as-one-column](coreutils/coreutils-expand-nul-counted-as-one-column/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15580) |
| [coreutils-expand-tabs-increment-checked-as-ascendi](coreutils/coreutils-expand-tabs-increment-checked-as-ascendi/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15579) |
| [coreutils-fmt-first-word-sentence-end-single-space](coreutils/coreutils-fmt-first-word-sentence-end-single-space/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15585) |
| [coreutils-fold-control-chars-zero-width](coreutils/coreutils-fold-control-chars-zero-width/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15571) |
| [coreutils-join-trailing-blanks-phantom-field-and-i](coreutils/coreutils-join-trailing-blanks-phantom-field-and-i/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15576) |
| [coreutils-nl-resets-logical-page-section-between-f](coreutils/coreutils-nl-resets-logical-page-section-between-f/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15586) |
| [coreutils-nl-unnumbered-line-padding-ignores-separ](coreutils/coreutils-nl-unnumbered-line-padding-ignores-separ/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15591) |
| [coreutils-numfmt-invalid-field-leaves-whole-line-u](coreutils/coreutils-numfmt-invalid-field-leaves-whole-line-u/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15575) |
| [coreutils-numfmt-rounded-to-zero-prints-negative-z](coreutils/coreutils-numfmt-rounded-to-zero-prints-negative-z/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15661) |
| [coreutils-numfmt-to-si-unscaled-value-ignores-roun](coreutils/coreutils-numfmt-to-si-unscaled-value-ignores-roun/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15574) |
| [coreutils-od-strings-hex-address-width](coreutils/coreutils-od-strings-hex-address-width/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15592) |
| [coreutils-printf-scientific-rounds-exact-ties-half](coreutils/coreutils-printf-scientific-rounds-exact-ties-half/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15662) |
| [coreutils-printf-string-star-precision-above-int-m](coreutils/coreutils-printf-string-star-precision-above-int-m/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15663) |
| [coreutils-seq-equal-width-counts-increment-digits](coreutils/coreutils-seq-equal-width-counts-increment-digits/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15572) |
| [coreutils-sort-global-i-not-overridden-by-d](coreutils/coreutils-sort-global-i-not-overridden-by-d/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15578) |
| [coreutils-sort-human-numeric-ignores-f-fold](coreutils/coreutils-sort-human-numeric-ignores-f-fold/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15577) |
| [coreutils-sort-key-own-reverse-ignored-on-whole-li](coreutils/coreutils-sort-key-own-reverse-ignored-on-whole-li/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15570) |
| [coreutils-sort-treats-form-feed-and-cr-as-blanks](coreutils/coreutils-sort-treats-form-feed-and-cr-as-blanks/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15588) |
| [coreutils-sort-version-ignores-i-d-f](coreutils/coreutils-sort-version-ignores-i-d-f/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15587) |
| [coreutils-split-lines-count-parsed-as-byte-size](coreutils/coreutils-split-lines-count-parsed-as-byte-size/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15660) |
| [coreutils-split-suffix-length-zero-ignores-default](coreutils/coreutils-split-suffix-length-zero-ignores-default/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15581) |
| [coreutils-split-suffix-start-too-large-accepted-on](coreutils/coreutils-split-suffix-start-too-large-accepted-on/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15569) |
| [coreutils-split-verbose-silent-in-number-mode](coreutils/coreutils-split-verbose-silent-in-number-mode/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15583) |
| [coreutils-tr-string2-construct-validation-mismatch](coreutils/coreutils-tr-string2-construct-validation-mismatch/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15589) |
| [coreutils-unexpand-dash-operand-taken-as-tab-short](coreutils/coreutils-unexpand-dash-operand-taken-as-tab-short/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15658) |
| [coreutils-unexpand-line-state-reset-at-file-bounda](coreutils/coreutils-unexpand-line-state-reset-at-file-bounda/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15657) |
| [coreutils-uniq-skip-fields-treats-cr-ff-as-blanks](coreutils/coreutils-uniq-skip-fields-treats-cr-ff-as-blanks/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15659) |

## datafusion

| Bug | Status | Upstream |
|---|---|---|
| [datafusion-cse-inner-alias-breaks-window-sort-key-a](datafusion/datafusion-cse-inner-alias-breaks-window-sort-key-a/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26620) |
| [datafusion-cse-intermediate-projection-drops-column](datafusion/datafusion-cse-intermediate-projection-drops-column/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26621) |
| [datafusion-equivalence-class-member-contains-own-me](datafusion/datafusion-equivalence-class-member-contains-own-me/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26623) |
| [datafusion-order-preserving-variant-pushes-topk-fet](datafusion/datafusion-order-preserving-variant-pushes-topk-fet/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26617) |
| [datafusion-projection-drops-constant-of-indirectly-](datafusion/datafusion-projection-drops-constant-of-indirectly-/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26624) |
| [datafusion-push-down-filter-aggregate-group-name-co](datafusion/datafusion-push-down-filter-aggregate-group-name-co/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26619) |
| [datafusion-push-down-filter-moves-correlated-subque](datafusion/datafusion-push-down-filter-moves-correlated-subque/) | reported 2026-10-21 | [link](https://github.com/apache/datafusion/issues/26573) |
| [datafusion-rewrite-set-comparison-renames-aggregate](datafusion/datafusion-rewrite-set-comparison-renames-aggregate/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26622) |
| [datafusion-scalar-subquery-sort-key-lost-by-project](datafusion/datafusion-scalar-subquery-sort-key-lost-by-project/) | reported 2026-10-21 | [link](https://github.com/apache/datafusion/issues/26572) |
| [`ORDER BY ... LIMIT 0` panics in TopK (`assertion failed: k > 0`) when `eliminate_limit` does not run](datafusion/datafusion-sortexec-fetch-zero-topk-panic/) | open | [link](https://github.com/apache/datafusion/issues/26431) |
| [datafusion-topk-filter-drops-negative-zero](datafusion/datafusion-topk-filter-drops-negative-zero/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26618) |
| [datafusion-union-output-ordering-drops-equivalent-s](datafusion/datafusion-union-output-ordering-drops-equivalent-s/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26625) |
| [datafusion-window-agg-exec-with-new-children-forces](datafusion/datafusion-window-agg-exec-with-new-children-forces/) | reported 2026-10-22 | [link](https://github.com/apache/datafusion/issues/26626) |
