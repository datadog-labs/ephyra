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
| [sqlite-drop-constraint-fk-set-default](sqlite/sqlite-drop-constraint-fk-set-default/) |  |  |
| [sqlite-drop-not-null-not-deferrable](sqlite/sqlite-drop-not-null-not-deferrable/) |  |  |
| [Join drops rows, or adds some, when a table has an index on an expression and a `col = constant` WHERE term](sqlite/sqlite-expr-index-constant-propagation/) | fixed | [link](https://sqlite.org/bugs/forumpost/529732a774) |
| [A UNION ALL subquery in FROM with a RIGHT JOIN after its first SELECT loses WHERE terms when flattened](sqlite/sqlite-flatten-compound-right-join/) | fixed | SQLite bug forum, bug report 2026-10-08T15:27:33Z |
| [FTS5: DROP TABLE and ALTER TABLE RENAME skip the %_content table of a contentless_unindexed=1 table](sqlite/sqlite-fts5-contentless-unindexed-drop-rename/) | open | SQLite bug forum, posted 2026-10-21 (thread id not recorded) |
| [FTS5: with detail=none, secure-delete of a rowid that was replaced while secure-delete was off corrupts the index](sqlite/sqlite-fts5-detail-none-secure-delete-replaced-rowid/) | open | [link](https://sqlite.org/bugs/forumpost/e380b6876c) |
| [FTS5: "no such cursor: 0" from an auxiliary function in WHERE with the `tbl = 'query'` form (constant propagation)](sqlite/sqlite-fts5-eq-constprop-aux-cursor/) | open | SQLite bug forum, posted 2026-10-20 (thread id not recorded) |
| [FTS5: INSERT into a locale=1, contentless_unindexed=1 table with an UNINDEXED column fails with "database disk image is malformed"](sqlite/sqlite-fts5-locale-contentless-unindexed-insert/) | open | SQLite bug forum, posted 2026-10-21 (thread id not recorded) |
| [FTS5: MATCH with `rowid = <non-integer value>` returns every match (regression after 3.51)](sqlite/sqlite-fts5-match-rowid-eq-nonint/) | open | SQLite bug forum, posted 2026-10-20 (thread id not recorded) |
| [sqlite-fts5-secure-delete-dlidx-walks-next-term](sqlite/sqlite-fts5-secure-delete-dlidx-walks-next-term/) |  |  |
| [sqlite-fts5-secure-delete-merge-drops-marker](sqlite/sqlite-fts5-secure-delete-merge-drops-marker/) |  |  |
| [FTS5 tokendata=1: a query for an absent term fails with "fts5: corruption found reading blob" after a long doclist](sqlite/sqlite-fts5-tokendata-missing-term-dlidx-corrupt/) | open | SQLite bug forum, posted 2026-10-21 (thread id not recorded) |
| [FTS5 detail=none/column: snippet() fails with "database disk image is malformed", highlight() and bm25() see matches in UNINDEXED columns](sqlite/sqlite-fts5-unindexed-detail-none-instances/) | open | SQLite bug forum, posted 2026-10-20 (thread id not recorded) |
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
| [An OR in a LEFT JOIN's ON clause drives a loop on the left table: wrong rows, or a segfault when the joined table is omitted](sqlite/sqlite-leftjoin-on-or-omitted-table-segv/) | open | SQLite bug forum, posted 2026-10-18 (thread id not recorded) |
| [a IN (...) AND c GLOB 'A' with an index on (a,c): GROUP BY splits a group, DISTINCT repeats a row (BLOB in c)](sqlite/sqlite-like-opt-in-order/) | fixed upstream before we reported it |  |
| [NOT INDEXED is ignored on WITHOUT ROWID tables: the planner still uses secondary and UNIQUE indexes](sqlite/sqlite-notindexed-without-rowid/) | open | [link](https://sqlite.org/bugs/forumpost/cd41fceaa0) |
| [An OR term that uses a partial index WHERE a=5 and also tests a fails with "internal query planner error"](sqlite/sqlite-or-partial-index-covering-error/) | fixed | [link](https://sqlite.org/bugs/forumpost/3ede79ce42) |
| [In a UTF-16 database, a blob over 4000 bytes read from a table's overflow pages is converted as UTF-8](sqlite/sqlite-overflow-blob-encoding/) | fixed | [link](https://sqlite.org/bugs/forumpost/48c64d8bab) |
| [In a UTF-16 database, text over 4000 bytes read from a table loses a leading U+FEFF, and an UPDATE then writes it back without it](sqlite/sqlite-overflow-text-bom/) | fixed | [link](https://sqlite.org/bugs/forumpost/36a73e8c8a) |
| [A partial index WHERE col = <constant> makes col read back as the constant: a TEXT '5.0' comes back as '5'](sqlite/sqlite-partial-index-constant-affinity/) | open | [link](https://sqlite.org/bugs/forumpost/35992eec46) |
| [WHERE b = -9223372036854775808.0 AND typeof(b) = 'integer' misses a row whose INT column b holds that integer](sqlite/sqlite-propagate-const-real-minint/) | open | [link](https://sqlite.org/bugs/forumpost/9bbaa34eea) |
| [`SELECT *` over a USING join left of a RIGHT or FULL JOIN is "ambiguous column name" when a later table has that column](sqlite/sqlite-right-join-using-star-ambiguous/) | open | [link](https://sqlite.org/bugs/forumpost/8be1215e2c) |
| [Row-value IN in a join repeats rows: the inner loop iterates the full IN list, with duplicates, for one of its columns](sqlite/sqlite-rowvalue-in-join-reuse/) | open | SQLite bug forum, posted 2026-10-16 (thread id not recorded) |
| [R-Tree: `UPDATE ... SET rowid = N` and `INSERT INTO ...(rowid, ...)` ignore the new rowid](sqlite/sqlite-rtree-update-set-rowid-ignored/) | open | SQLite bug forum, posted 2026-10-21 (thread id not recorded) |
| [ROLLBACK TO of a savepoint that dropped a table commits a stale root page in an incremental-vacuum database](sqlite/sqlite-savepoint-autovacuum/) | fixed | [link](https://sqlite.org/bugs/forumpost/3bc3c8adfa) |
| [ALTER TABLE RENAME COLUMN of a TEMP table fails when a TEMP trigger or view uses the column](sqlite/sqlite-temp-rename-column-twice/) | open | SQLite bug forum, posted 2026-10-19 (thread id not recorded) |
| ["SQL logic error" on the next SAVEPOINT or COMMIT, and the transaction is lost, after ROLLBACK TO undoes a CREATE VIRTUAL TABLE (fts5, fts4)](sqlite/sqlite-vtab-create-rollback-to-savepoint/) | open | SQLite bug forum, posted 2026-10-20 (thread id not recorded) |
| [With a window function, a correlated LEFT JOIN whose ON clause has a bare outer column as an AND term filters the left rows](sqlite/sqlite-window-left-join-outer-on/) | fixed | SQLite bug forum, bug report 2026-10-08T15:38:26Z |
| ["database disk image is malformed" or missing rows: ORDER BY an added column NULLS LAST on a WITHOUT ROWID table](sqlite/sqlite-wr-bignull-added-column/) | fixed | [link](https://sqlite.org/bugs/forumpost/eee345f931) |
| ["database disk image is malformed" or wrong answer: row-value IN (SELECT ...) over an added column of a WITHOUT ROWID table](sqlite/sqlite-wr-in-index-added-column/) | open | [link](https://sqlite.org/bugs/forumpost/eee345f931) |
| ["database disk image is malformed" or missing rows: row-value range over an added column of a WITHOUT ROWID table](sqlite/sqlite-wr-rowvalue-added-column/) | open | [link](https://sqlite.org/bugs/forumpost/eee345f931) |

## turso

| Bug | Status | Upstream |
|---|---|---|
| [turso-analyze-empty-schema-stat1](turso/turso-analyze-empty-schema-stat1/) |  |  |
| [turso-analyze-temp-schema](turso/turso-analyze-temp-schema/) |  |  |
| [BLOB \|\| BLOB returns a BLOB; SQLite returns TEXT (closed #6089 still reproduces)](turso/turso-blob-concat-returns-blob/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/6089) |
| [CAST(text AS NUMERIC) returns an incorrect double for integer text that overflows i64](turso/turso-cast-numeric-large-integer/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/8452) |
| [A CTE is not found in a subquery inside a JOIN ON clause](turso/turso-cte-on-clause-subquery/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/9073) |
| [Date/time functions don't stop at an embedded NUL in their text arguments; SQLite reads up to the NUL](turso/turso-date-embedded-nul/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9935) |
| [`±HH:MM` date modifier with hour 24 adds a day; SQLite adds nothing](turso/turso-date-hhmm-modifier-whole-days/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9921) |
| [date/time "NNN units" and "±YYYY-MM-DD" modifiers accept trailing whitespace and trailing text that SQLite rejects](turso/turso-date-modifier-trailing-text/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9837) |
| [Date/time functions don't accept a trailing vertical tab (0x0B) after the time value (like #8454, for dates)](turso/turso-date-vertical-tab/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9968) |
| [A runtime error in any statement rolls back the whole explicit transaction (not only JSON errors)](turso/turso-error-rolls-back-transaction/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/6864) |
| [JSON subtype lost when a result column contains an expression from GROUP BY (follow-up to #5755)](turso/turso-group-by-expression-json-subtype/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9966) |
| [Inner-join "duplicate cursor key" panic through a materialized hash build input over a table-valued function subquery (wider form of #9847)](turso/turso-hash-build-input-vtab-subquery-duplicate-cursor/) | fixed upstream before we reported it | [link](https://github.com/tursodatabase/turso/issues/9847) |
| [A REAL column read through a hash join comes back as an integer (SQLite-created file)](turso/turso-hash-join-real-affinity/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9881) |
| [Aggregates of groups removed by a GROUP BY-only HAVING term still run, so they raise errors SQLite never reaches](turso/turso-having-group-term-not-pushed/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9880) |
| [HAVING with a predicate that is itself a GROUP BY term (`GROUP BY c, a IS NULL HAVING a IS NULL`) filters on the wrong value](turso/turso-having-group-term-predicate/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9887) |
| [A HAVING subquery reads the GROUP BY column itself from the wrong row when it isn't selected](turso/turso-having-subquery-group-column/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/8221) |
| [turso-index-dqs-string-literal](turso/turso-index-dqs-string-literal/) |  |  |
| [turso-index-xinfo-rowid-name-null](turso/turso-index-xinfo-rowid-name-null/) |  |  |
| [`->`/`->>` ignore the `'[N]'` abbreviated path form, so `'["a","b"]' ->> '[0]'` returns NULL](turso/turso-json-arrow-abbreviated-path/) | open | [link](https://github.com/tursodatabase/turso/issues/9845) |
| [`x -> x` and `x ->> x` panic (index out of bounds) when both operands are the same expression](turso/turso-json-arrow-same-operand-panic/) | open | [link](https://github.com/tursodatabase/turso/issues/9836) |
| [JSON functions return the integer for a whole-number REAL when the same integer was seen just before (JSON cache key compares 1 and 1.0 as equal)](turso/turso-json-cache-integer-real-key/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9967) |
| [turso-json-each-column-affinity](turso/turso-json-each-column-affinity/) |  |  |
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
| [turso-json5-line-comment-terminator](turso/turso-json5-line-comment-terminator/) |  |  |
| [Appending to a top-level JSON array loses data (json_set/json_insert with '$[#]')](turso/turso-jsonb-root-array-append/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/8538) |
| [An ON clause with IS drops FROM-subquery materialization (quadratic LEFT JOIN subquery)](turso/turso-left-join-subquery-is-coroutine/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/8322) |
| [turso-likelihood-exponent-literal](turso/turso-likelihood-exponent-literal/) |  |  |
| [Panic "cursor id N is None" when a materialized CTE is first read inside a nested subquery's loop and then read by the outer query](turso/turso-materialized-cte-first-filled-in-subquery-loop/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9882) |
| ["Parse error: all before-loop subqueries should have already been emitted" for a materialized CTE with a correlated subquery, read from a nested subquery](turso/turso-materialized-cte-subquery-before-loop/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9848) |
| [A result column `-9223372036854775808` becomes REAL when ORDER BY / GROUP BY has `-9.223372036854776e18` (and the other way round)](turso/turso-minint-literal-order-by-equivalence/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9925) |
| [Panic "column N of table tN is not stored in the index and the table cursor is not open" with a partial index and a BETWEEN that implies its WHERE](turso/turso-partial-index-between-covering-panic/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9926) |
| [Panic "column N of table tN is not stored in the index and the table cursor is not open" with a partial index, a seek on its key and a join on that key](turso/turso-partial-index-join-covering-panic/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9934) |
| [turso-pragma-check-tvf-arg-comment](turso/turso-pragma-check-tvf-arg-comment/) |  |  |
| [turso-pragma-check-tvf-column-name](turso/turso-pragma-check-tvf-column-name/) |  |  |
| [quote() writes a REAL in [1, 10) that needs 16+ digits with exponent "e-00"; SQLite 3.50.4 writes "e+00"](turso/turso-quote-real-exponent-zero/) | fixed by an open pull request | [link](https://github.com/tursodatabase/turso/pull/9534) |
| [Text-to-REAL conversion through column affinity and sum()/total()/avg() gives a different double than CAST or a literal](turso/turso-real-affinity-parse/) | open | [link](https://github.com/tursodatabase/turso/issues/9849) |
| [-0.0 stored in a REAL column keeps its sign; SQLite reads it back as +0.0](turso/turso-real-column-negative-zero/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9889) |
| [RIGHT JOIN followed by LEFT JOIN: a subquery in the later ON clause is not re-evaluated for unmatched right rows (panic or wrong result)](turso/turso-right-join-on-subquery-unmatched-rows/) | open | [link](https://github.com/tursodatabase/turso/issues/9850) |
| [Panic "duplicate cursor key" when a RIGHT/FULL JOIN query has a WHERE subquery over a table-valued function](turso/turso-right-join-vtab-subquery-duplicate-cursor/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9847) |
| [Panic "a WITHOUT ROWID table has no secondary index to drive its loop" in a RIGHT/FULL JOIN on a SQLite-created file](turso/turso-right-join-without-rowid-secondary-index/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9852) |
| [turso-rowvalue-between-join-comment](turso/turso-rowvalue-between-join-comment/) |  |  |
| [turso-schema-table-alias-qualifier](turso/turso-schema-table-alias-qualifier/) |  |  |
| [turso-soundex-non-letters](turso/turso-soundex-non-letters/) |  |  |
| [strftime() prints an overflowing day as is (2024-02-30); date() and SQLite normalize it to 2024-03-01](turso/turso-strftime-day-overflow/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9969) |
| [strftime() returns a value when the modifiers move the date out of range; date() and SQLite return NULL](turso/turso-strftime-out-of-range/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9884) |
| [strftime(): `%J` and a REAL format argument are printed with more digits than SQLite](turso/turso-strftime-real-formatting/) | open | [link](https://github.com/tursodatabase/turso/issues/9888) |
| [turso-strict-without-rowid-alter-comment](turso/turso-strict-without-rowid-alter-comment/) |  |  |
| [An uncorrelated scalar subquery runs before the scan, so it raises errors SQLite never reaches](turso/turso-subquery-hoisted-eval/) | open | [link](https://github.com/tursodatabase/turso/issues/9851) |
| [turso-sum-overflow-blob-text-comment](turso/turso-sum-overflow-blob-text-comment/) |  |  |
| [turso-temp-trigger-rename-column-comment](turso/turso-temp-trigger-rename-column-comment/) |  |  |
| [timediff() gives wrong (sometimes malformed) results for dates with an overflowing day such as 2023-02-29](turso/turso-timediff-day-overflow/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9886) |
| [An unused CTE is resolved, and its errors reported, when the statement has a subquery outside FROM](turso/turso-unused-cte-eager-resolve/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9842) |
| [UPDATE with a BEFORE UPDATE trigger writes an index key from a REAL column without REAL affinity, corrupting the index (SQLite-created file)](turso/turso-update-before-trigger-real-affinity-index/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9933) |
| [UPDATE through a covering index writes the row without finishing the deferred seek when a partial index WHERE reads only the rowid (panic or duplicate rowids)](turso/turso-update-partial-index-rowid-unpositioned-insert/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9932) |
| [turso-vacuum-into-memory-target](turso/turso-vacuum-into-memory-target/) |  |  |
| [turso-window-call-repeated-nondeterministic-arg-comment](turso/turso-window-call-repeated-nondeterministic-arg-comment/) |  |  |
| [turso-window-group-concat-inverse-empty-string](turso/turso-window-group-concat-inverse-empty-string/) |  |  |
| ["Parse error: table not found" for ORDER BY rowid, then a min/max window column with a non-deterministic window ORDER BY](turso/turso-window-minmax-order-by-table-not-found/) | fixed | [link](https://github.com/tursodatabase/turso/issues/9883) |
| [WITHOUT ROWID INSERT applies column affinities in schema order (false UNIQUE failures)](turso/turso-without-rowid-affinity-order/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/6749) |
| [A join into a WITHOUT ROWID table through an automatic index loses rows when no primary-key column is read](turso/turso-without-rowid-autoindex-dedup/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/9081) |
| [Secondary index read of a SQLite-made WITHOUT ROWID table returns NULL for non-indexed columns](turso/turso-without-rowid-index-null-columns/) | reported by others | [link](https://github.com/tursodatabase/turso/issues/7273) |

## postgresql

| Bug | Status | Upstream |
|---|---|---|
| [btree_gist date_ops: inserting infinity/-infinity into a multi-page GiST index on a date fails with "cannot subtract infinite dates"](postgresql/pg-btree-gist-infinite-date/) | open | [link](https://www.postgresql.org/message-id/19761-b72cfc453e487610%40postgresql.org) |
| [CLUSTER / REPACK silently loses a row updated in a rolled-back subtransaction](postgresql/pg-cluster-subxact-row-loss/) | open | [link](https://www.postgresql.org/message-id/19760-8f9a0eba17bc8c92%40postgresql.org) |
| [Hashed set operations and recursive UNION pre-size their hash table from the planner's estimate without the hash_mem cap (PostgreSQL 19 regression)](postgresql/pg-hashsetop-out-of-memory/) | open | [link](https://www.postgresql.org/message-id/19759-acd3f33402e54f36%40postgresql.org) |
| [Nested BETWEEN grows exponentially in parse analysis and statement_timeout cannot stop it](postgresql/pg-huge-expr-timeout/) | open | [link](https://www.postgresql.org/message-id/19758-d074798b35cc9215%40postgresql.org) |
| ["corrupt MVNDistinct entry" (XX000) from GROUP BY with grouping sets over a column and an expression of it](postgresql/pg-mvndistinct-grouping-sets/) | open | BUG #19772 |
| [EXPLAIN ANALYZE: Assert("toc->toc_entry[i].key != key") in shm_toc_insert (master only, since a241bc4fede)](postgresql/pg-parallel-material-dup-toc/) | open | [link](https://www.postgresql.org/message-id/19766-c3d105a4e0ec5487%40postgresql.org) |
| [UPDATE moving a row of a self-referencing partitioned table fails with 23503 although ON UPDATE CASCADE would fix the reference](postgresql/pg-partition-selfref-cascade-move/) | open | BUG #19781 |
| [PostgreSQL: candidate from today's round, report drafted, not reported](postgresql/pg-plans-sortagg-cte/) | open | BUG #19783 |
| [SET CONSTRAINTS in a released savepoint survives ROLLBACK TO an enclosing savepoint](postgresql/pg-set-constraints-released-savepoint/) | open | BUG #19782 |
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
| [INTERNAL Error "GetPartitioningSpaceRequirement: tuple width should not be 0" for a hash join whose probe side has no columns](duckdb/duckdb-hash-join-zero-column-probe-side-partiti/) | fixed | [link](https://github.com/duckdb/duckdb/issues/27450) |
| [INTERNAL Error in join elimination (`!column_bindings.empty()`) on a correlated subquery with a never-evaluated `count(DISTINCT ...)`](duckdb/duckdb-join-elimination-asserts-on-conditionles/) | open | [link](https://github.com/duckdb/duckdb/issues/27445) |
| [INTERNAL Error "Projection map references a missing child column": join elimination leaves the parent filter's projection map stale](duckdb/duckdb-join-elimination-stale-parent-projection/) | open | [link](https://github.com/duckdb/duckdb/issues/27449) |
| [Join order optimizer drops a filter that references no column (`WHERE NULL`, `WHERE random() > 2`) when filter_pushdown is disabled](duckdb/duckdb-join-order-drops-relation-free-filter/) | fixed | [link](https://github.com/duckdb/duckdb/issues/27451) |
| [INTERNAL Error "Failed to bind column reference" when an always-matching SEMI JOIN (`ON true`) is reordered by the join order optimizer](duckdb/duckdb-join-order-stale-projection-map-after-se/) | open | [link](https://github.com/duckdb/duckdb/issues/27452) |
| [Segfault in `dense_rank() OVER ()` over a RIGHT_ANTI hash join that returns its unmatched rows](duckdb/duckdb-natural-sort-empty-local-collection-dens/) | open | [link](https://github.com/duckdb/duckdb/issues/27448) |
| [duckdb-semi-join-right-only-condition-no-filter-pushdown](duckdb/duckdb-semi-join-right-only-condition-no-filter-pushdown/) |  |  |
| [INTERNAL Error `exprs.size() > 0` (expression_executor.cpp) when statistics propagation folds an aggregate with no aggregate expressions (`HAVING` without aggregates)](duckdb/duckdb-stats-folds-expressionless-aggregate-int/) | fixed | [link](https://github.com/duckdb/duckdb/issues/27446) |
| [INTERNAL Error `physical_index != DConstants::INVALID_INDEX` in propagate_get.cpp when statistics fold a pushed-down filter's column away](duckdb/duckdb-stats-table-filter-loses-column-ref/) | open | [link](https://github.com/duckdb/duckdb/issues/27447) |

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
| [comm: sorted input rejected as "not in sorted order" when a line is a prefix of the next followed by a tab](coreutils/coreutils-comm-compares-line-terminator/) | fixed | [link](https://github.com/uutils/coreutils/issues/15439) |
| [csplit: a `/REGEX/` pattern after a line-number pattern never tests the line the split stopped at](coreutils/coreutils-csplit-regex-skips-line-after-line-numbe/) | open | [link](https://github.com/uutils/coreutils/issues/15440) |

## datafusion

| Bug | Status | Upstream |
|---|---|---|
| [`ORDER BY ... LIMIT 0` panics in TopK (`assertion failed: k > 0`) when `eliminate_limit` does not run](datafusion/datafusion-sortexec-fetch-zero-topk-panic/) | open | [link](https://github.com/apache/datafusion/issues/26431) |
