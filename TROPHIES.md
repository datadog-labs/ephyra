# Trophies

Bugs found with ephyra and reported upstream: 514 in 27 projects, 318 of them fixed or partly fixed. Each links to its report; the status is as last recorded here, so follow the link for the current state. Bugs still under embargo or not yet reported publicly are not listed. The [case studies](case-studies) hold the harnesses, and for many targets the reports as filed.

| Project | Bugs | Fixed |
|---|---:|---:|
| [Turso](#turso) | 86 | 61 |
| [SQLite](#sqlite) | 71 | 63 |
| [uutils coreutils](#uutils-coreutils) | 38 | 29 |
| [gitoxide](#gitoxide) | 31 | 31 |
| [DuckDB](#duckdb) | 26 | 14 |
| [brush](#brush) | 24 | 8 |
| [Lean 4](#lean-4) | 24 | 14 |
| [ty](#ty) | 22 | 13 |
| [Bun](#bun) | 19 | 0 |
| [FastMCP](#fastmcp) | 19 | 17 |
| [PostgreSQL](#postgresql) | 18 | 4 |
| [sqlparser-rs](#sqlparser-rs) | 17 | 0 |
| [LiteLLM](#litellm) | 16 | 13 |
| [TorchSpec](#torchspec) | 15 | 15 |
| [Apache DataFusion](#apache-datafusion) | 13 | 7 |
| [OxCaml](#oxcaml) | 13 | 3 |
| [Dogwood](#dogwood) | 13 | 0 |
| [Racket](#racket) | 11 | 3 |
| [Ada URL parser](#ada-url-parser) | 11 | 10 |
| [Elixir](#elixir) | 9 | 9 |
| [OCaml](#ocaml) | 5 | 2 |
| [bbtree](#bbtree) | 4 | 0 |
| [Envoy](#envoy) | 2 | 0 |
| [FoundationDB](#foundationdb) | 2 | 1 |
| [Cedar](#cedar) | 2 | 1 |
| [QED prover](#qed-prover) | 2 | 0 |
| [TigerBeetle](#tigerbeetle) | 1 | 0 |

## Turso

[github.com/tursodatabase/turso](https://github.com/tursodatabase/turso)

- [tursodatabase/turso#10337](https://github.com/tursodatabase/turso/issues/10337): CREATE INDEX rejects double-quoted string literals that SQLite accepts, so some SQLite databases cannot be opened or written (open)
- [tursodatabase/turso#10338](https://github.com/tursodatabase/turso/issues/10338): likelihood() rejects a float literal written with an exponent (625e-4, 1e0) or in parentheses (open)
- [tursodatabase/turso#10339](https://github.com/tursodatabase/turso/issues/10339): pragma_quick_check / pragma_integrity_check name their column "message" instead of "quick_check" / "integrity_check" (fixed)
- [tursodatabase/turso#10340](https://github.com/tursodatabase/turso/issues/10340): ANALYZE cannot reach TEMP objects: "ANALYZE temp", "ANALYZE <temp table>" and "ANALYZE <temp index>" fail (fixed)
- [tursodatabase/turso#10341](https://github.com/tursodatabase/turso/issues/10341): VACUUM INTO ':memory:' writes a file named ":memory:" in the current directory; VACUUM INTO '' is rejected (fixed)
- [tursodatabase/turso#10342](https://github.com/tursodatabase/turso/issues/10342): json_each/json_tree declare column types (fullkey TEXT, key ANY, ...), so comparisons with numbers use the wrong affinity (fixed)
- [tursodatabase/turso#10343](https://github.com/tursodatabase/turso/issues/10343): JSON5 // comment ends only at LF; SQLite also ends it at CR, U+2028 and U+2029 (fixed)
- [tursodatabase/turso#10352](https://github.com/tursodatabase/turso/issues/10352): group_concat() over a sliding window frame panics ("GroupConcat accumulator is Text after a non-NULL xStep") when the frame holds an empty string (fixed)
- [tursodatabase/turso#10353](https://github.com/tursodatabase/turso/issues/10353): A column qualified with sqlite_master is not found when FROM names sqlite_schema (and the reverse) (fixed)
- [tursodatabase/turso#10354](https://github.com/tursodatabase/turso/issues/10354): PRAGMA index_xinfo reports the rowid entry's name as '' instead of NULL (open)
- [tursodatabase/turso#10355](https://github.com/tursodatabase/turso/issues/10355): ANALYZE on a database without tables does not create sqlite_stat1 (fixed)
- [tursodatabase/turso#10356](https://github.com/tursodatabase/turso/issues/10356): soundex() returns '?000' for any input with a non-letter, and codes H/W differently from SQLite (fixed)
- [tursodatabase/turso#10370](https://github.com/tursodatabase/turso/issues/10370): json_patch/jsonb_patch decode the escapes in the keys they add, and jsonb_patch writes a key with a raw control character as plain TEXT (fixed)
- [tursodatabase/turso#10378](https://github.com/tursodatabase/turso/issues/10378): json_error_position reports a different position than SQLite for text after a complete value and for malformed numbers and literals (fixed)
- [tursodatabase/turso#10387](https://github.com/tursodatabase/turso/issues/10387): RENAME COLUMN of a WITHOUT ROWID table's primary-key column moves it to the end of the key: reads return permuted values and INSERT writes permuted rows (fixed)
- [tursodatabase/turso#10388](https://github.com/tursodatabase/turso/issues/10388): Correlated subquery in the SELECT list reads NULL for a hash-build-side table when the probe side is a LEFT JOIN (outer-join case of #8226) (fixed)
- [tursodatabase/turso#10389](https://github.com/tursodatabase/turso/issues/10389): Parse error: subquery not found for a WHERE subquery that reads a table outside a materialized hash build input (fixed)
- [tursodatabase/turso#10390](https://github.com/tursodatabase/turso/issues/10390): Panic "cursor id N is None" in a RIGHT JOIN's unmatched-rows pass when a WHERE subquery has a FROM subquery and a nested subquery that reads it (fixed)
- [tursodatabase/turso#10391](https://github.com/tursodatabase/turso/issues/10391): ALTER TABLE ADD COLUMN ... DEFAULT -9223372036854775808 reads as REAL for existing rows (and breaks UPDATE on a STRICT table) (fixed)
- [tursodatabase/turso#10392](https://github.com/tursodatabase/turso/issues/10392): STRICT type check runs before NOT NULL, so INSERT OR IGNORE fails on a row SQLite skips (fixed)
- [tursodatabase/turso#10393](https://github.com/tursodatabase/turso/issues/10393): UPSERT DO UPDATE is compiled even when the INSERT cannot conflict; errors in it fail the INSERT, SQLite skips them (fixed)
- [tursodatabase/turso#10394](https://github.com/tursodatabase/turso/issues/10394): concat_ws() drops BLOB arguments and returns NULL when the separator is a BLOB (fixed)
- [tursodatabase/turso#10395](https://github.com/tursodatabase/turso/issues/10395): Compound SELECT: ORDER BY `x ISNULL` does not match a result column written `x IS NULL` (and NOTNULL / NOT NULL / IS NOT NULL) (open)
- [tursodatabase/turso#10396](https://github.com/tursodatabase/turso/issues/10396): `schema.alias.column` that refers to an outer query's table is not found in a subquery ("no such column: main.x.a") (fixed)
- [tursodatabase/turso#10397](https://github.com/tursodatabase/turso/issues/10397): PRAGMA table_info / table_list keep a view's old column list after its table changes (ADD COLUMN, DROP TABLE) until the database is reopened (open)
- [tursodatabase/turso#10398](https://github.com/tursodatabase/turso/issues/10398): sqlite_schema.sql holds a re-serialized statement instead of the original text, and keeps IF NOT EXISTS (open)
- [tursodatabase/turso#10399](https://github.com/tursodatabase/turso/issues/10399): PRAGMA index_list / index_info / index_xinfo don't show the primary-key index of a WITHOUT ROWID table (fixed)
- [tursodatabase/turso#10400](https://github.com/tursodatabase/turso/issues/10400): unhex(X, Y) returns NULL when Y is an INTEGER, REAL or BLOB instead of using its text (fixed)
- [tursodatabase/turso#10401](https://github.com/tursodatabase/turso/issues/10401): UPDATE of a child table fails with "foreign key mismatch" even when no foreign key column is assigned (fixed)
- [tursodatabase/turso#10426](https://github.com/tursodatabase/turso/issues/10426): replace(X, '', NULL) returns NULL instead of X (fixed)
- [tursodatabase/turso#10496](https://github.com/tursodatabase/turso/issues/10496): UPDATE on a parent table fails with "foreign key mismatch" while foreign keys are off (open)
- [tursodatabase/turso#10906](https://github.com/tursodatabase/turso/issues/10906): group_concat() over a sliding window frame splits a multi-byte UTF-8 character when removing rows (follow-up to #10352) (fixed)
- [tursodatabase/turso#10907](https://github.com/tursodatabase/turso/issues/10907): JSON functions take a BLOB for JSONB when json_valid(X, 8) says it isn't, and skip leading whitespace bytes before classifying it (fixed)
- [tursodatabase/turso#10907](https://github.com/tursodatabase/turso/issues/10907): A TEXT value that is not valid UTF-8 is rejected as JSONB (filed with turso-json-blob-arg-jsonb-detection) (fixed)
- [tursodatabase/turso#10908](https://github.com/tursodatabase/turso/issues/10908): OR over an index and the rowid misses rows: a seek on the table cursor leaves an earlier DeferredSeek pending (fixed)
- [tursodatabase/turso#11536](https://github.com/tursodatabase/turso/issues/11536): CREATE ... IF NOT EXISTS / DROP ... IF EXISTS that does nothing fails with "database is locked" while another connection writes (open)
- [tursodatabase/turso#11537](https://github.com/tursodatabase/turso/issues/11537): "Database schema changed" from a statement stepped after ROLLBACK TO, when another connection changed the schema (open)
- [tursodatabase/turso#11538](https://github.com/tursodatabase/turso/issues/11538): "Database schema changed" in a read transaction after another connection changes the schema of an attached database (open)
- [tursodatabase/turso#11539](https://github.com/tursodatabase/turso/issues/11539): DDL on an attached database takes a write transaction on main: "database snapshot is stale" after another connection wrote to main, and writers of main get Busy (open)
- [tursodatabase/turso#11540](https://github.com/tursodatabase/turso/issues/11540): A completed INSERT ... RETURNING is rolled back when a statement prepared before a schema change is stepped while the INSERT is suspended (open)
- [tursodatabase/turso#11541](https://github.com/tursodatabase/turso/issues/11541): A failed statement inside BEGIN rolls back a virtual table's earlier writes, but the transaction still commits (open)
- [tursodatabase/turso#11542](https://github.com/tursodatabase/turso/issues/11542): Resetting a SELECT that was started before BEGIN rolls back the explicit transaction (open)
- [tursodatabase/turso#11543](https://github.com/tursodatabase/turso/issues/11543): Panic "shared autocommit transaction needed full rollback outside MVCC" when a window-function SELECT is reset while an INSERT ... RETURNING is suspended (open)
- [tursodatabase/turso#11544](https://github.com/tursodatabase/turso/issues/11544): Writing an extension virtual table from a connection other than the one that created it panics "Could not find virtual table for VBegin"; DROP TABLE from it fails (open)
- [tursodatabase/turso#11545](https://github.com/tursodatabase/turso/issues/11545): UPDATE that sets the rowid of an extension virtual table panics "NotExists on unexpected cursor" (open)
- [tursodatabase/turso#11546](https://github.com/tursodatabase/turso/issues/11546): A statement that got Busy on an attached database keeps returning Busy after the lock is released, until it is reset (open)
- [tursodatabase/turso#11547](https://github.com/tursodatabase/turso/issues/11547): changes() / total_changes() differ from SQLite: CREATE INDEX resets changes(), INSERT OR FAIL inside BEGIN, and virtual-table writes (fixed)
- [tursodatabase/turso#9836](https://github.com/tursodatabase/turso/issues/9836): `x -> x` and `x ->> x` panic (index out of bounds) when both operands are the same expression (fixed)
- [tursodatabase/turso#9837](https://github.com/tursodatabase/turso/issues/9837): date/time "NNN units" and "±YYYY-MM-DD" modifiers accept trailing whitespace and trailing text that SQLite rejects (fixed)
- [tursodatabase/turso#9838](https://github.com/tursodatabase/turso/issues/9838): json_each/json_tree with a NULL path error "root path should be text" (SQLite returns no rows) (fixed)
- [tursodatabase/turso#9839](https://github.com/tursodatabase/turso/issues/9839): json_each/json_tree error "number too large to fit in target type" on integers outside the i64 range (fixed)
- [tursodatabase/turso#9840](https://github.com/tursodatabase/turso/issues/9840): JSON functions write a negative-zero REAL as `-0.0`; SQLite writes `0.0` (open)
- [tursodatabase/turso#9841](https://github.com/tursodatabase/turso/issues/9841): json_error_position('') is an internal error, and whitespace before end of input shifts the position (fixed)
- [tursodatabase/turso#9842](https://github.com/tursodatabase/turso/issues/9842): An unused CTE is resolved, and its errors reported, when the statement has a subquery outside FROM (fixed)
- [tursodatabase/turso#9843](https://github.com/tursodatabase/turso/issues/9843): json_patch/jsonb_patch with a repeated key in the patch apply members out of order, and jsonb_patch returns a malformed blob (fixed)
- [tursodatabase/turso#9844](https://github.com/tursodatabase/turso/issues/9844): json_set/json_insert/json_replace/json_remove: a JSON `null` result becomes SQL NULL, and NULL arguments aren't handled like SQLite (follow-up to #5750) (fixed)
- [tursodatabase/turso#9845](https://github.com/tursodatabase/turso/issues/9845): `->`/`->>` ignore the `'[N]'` abbreviated path form, so `'["a","b"]' ->> '[0]'` returns NULL (open)
- [tursodatabase/turso#9847](https://github.com/tursodatabase/turso/issues/9847): Panic "duplicate cursor key" when a RIGHT/FULL JOIN query has a WHERE subquery over a table-valued function (open)
- [tursodatabase/turso#9848](https://github.com/tursodatabase/turso/issues/9848): Parse error: all before-loop subqueries should have already been emitted for a materialized CTE with a correlated subquery, read from a nested subquery (fixed)
- [tursodatabase/turso#9849](https://github.com/tursodatabase/turso/issues/9849): Text-to-REAL conversion through column affinity and sum()/total()/avg() gives a different double than CAST or a literal (open)
- [tursodatabase/turso#9850](https://github.com/tursodatabase/turso/issues/9850): RIGHT JOIN followed by LEFT JOIN: a subquery in the later ON clause is not re-evaluated for unmatched right rows (panic or wrong result) (open)
- [tursodatabase/turso#9851](https://github.com/tursodatabase/turso/issues/9851): An uncorrelated scalar subquery runs before the scan, so it raises errors SQLite never reaches (open)
- [tursodatabase/turso#9852](https://github.com/tursodatabase/turso/issues/9852): Panic "a WITHOUT ROWID table has no secondary index to drive its loop" in a RIGHT/FULL JOIN on a SQLite-created file (fixed)
- [tursodatabase/turso#9880](https://github.com/tursodatabase/turso/issues/9880): Aggregates of groups removed by a GROUP BY-only HAVING term still run, so they raise errors SQLite never reaches (fixed)
- [tursodatabase/turso#9881](https://github.com/tursodatabase/turso/issues/9881): A REAL column read through a hash join comes back as an integer (SQLite-created file) (fixed)
- [tursodatabase/turso#9882](https://github.com/tursodatabase/turso/issues/9882): Panic "cursor id N is None" when a materialized CTE is first read inside a nested subquery's loop and then read by the outer query (fixed)
- [tursodatabase/turso#9883](https://github.com/tursodatabase/turso/issues/9883): Parse error: table not found for ORDER BY rowid, then a min/max window column with a non-deterministic window ORDER BY (fixed)
- [tursodatabase/turso#9884](https://github.com/tursodatabase/turso/issues/9884): strftime() returns a value when the modifiers move the date out of range; date() and SQLite return NULL (fixed)
- [tursodatabase/turso#9885](https://github.com/tursodatabase/turso/issues/9885): json_pretty() prints an empty object or array on two lines (fixed)
- [tursodatabase/turso#9886](https://github.com/tursodatabase/turso/issues/9886): timediff() gives wrong (sometimes malformed) results for dates with an overflowing day such as 2023-02-29 (fixed)
- [tursodatabase/turso#9887](https://github.com/tursodatabase/turso/issues/9887): HAVING with a predicate that is itself a GROUP BY term (`GROUP BY c, a IS NULL HAVING a IS NULL`) filters on the wrong value (fixed)
- [tursodatabase/turso#9888](https://github.com/tursodatabase/turso/issues/9888): strftime(): `%J` and a REAL format argument are printed with more digits than SQLite (fixed)
- [tursodatabase/turso#9889](https://github.com/tursodatabase/turso/issues/9889): -0.0 stored in a REAL column keeps its sign; SQLite reads it back as +0.0 (fixed)
- [tursodatabase/turso#9921](https://github.com/tursodatabase/turso/issues/9921): `±HH:MM` date modifier with hour 24 adds a day; SQLite adds nothing (fixed)
- [tursodatabase/turso#9924](https://github.com/tursodatabase/turso/issues/9924): json_insert / jsonb_insert with path '$' replace the whole document; SQLite leaves it unchanged (fixed)
- [tursodatabase/turso#9925](https://github.com/tursodatabase/turso/issues/9925): A result column `-9223372036854775808` becomes REAL when ORDER BY / GROUP BY has `-9.223372036854776e18` (and the other way round) (fixed)
- [tursodatabase/turso#9926](https://github.com/tursodatabase/turso/issues/9926): Panic "column N of table tN is not stored in the index and the table cursor is not open" with a partial index and a BETWEEN that implies its WHERE (fixed)
- [tursodatabase/turso#9927](https://github.com/tursodatabase/turso/issues/9927): json() rewrites every "9.0e+999" in its output, even inside strings; json_set/json_insert write an infinite REAL as 9.0e+999 where SQLite writes 9e999 (fixed)
- [tursodatabase/turso#9932](https://github.com/tursodatabase/turso/issues/9932): UPDATE through a covering index writes the row without finishing the deferred seek when a partial index WHERE reads only the rowid (panic or duplicate rowids) (fixed)
- [tursodatabase/turso#9933](https://github.com/tursodatabase/turso/issues/9933): UPDATE with a BEFORE UPDATE trigger writes an index key from a REAL column without REAL affinity, corrupting the index (SQLite-created file) (fixed)
- [tursodatabase/turso#9934](https://github.com/tursodatabase/turso/issues/9934): Panic "column N of table tN is not stored in the index and the table cursor is not open" with a partial index, a seek on its key and a join on that key (fixed)
- [tursodatabase/turso#9935](https://github.com/tursodatabase/turso/issues/9935): Date/time functions don't stop at an embedded NUL in their text arguments; SQLite reads up to the NUL (fixed)
- [tursodatabase/turso#9966](https://github.com/tursodatabase/turso/issues/9966): JSON subtype lost when a result column contains an expression from GROUP BY (follow-up to #5755) (fixed)
- [tursodatabase/turso#9967](https://github.com/tursodatabase/turso/issues/9967): JSON functions return the integer for a whole-number REAL when the same integer was seen just before (JSON cache key compares 1 and 1.0 as equal) (fixed)
- [tursodatabase/turso#9968](https://github.com/tursodatabase/turso/issues/9968): Date/time functions don't accept a trailing vertical tab (0x0B) after the time value (like #8454, for dates) (fixed)
- [tursodatabase/turso#9969](https://github.com/tursodatabase/turso/issues/9969): strftime() prints an overflowing day as is (2024-02-30); date() and SQLite normalize it to 2024-03-01 (open)

## SQLite

[sqlite.org](https://sqlite.org)

- [SQLite bug forum](https://sqlite.org/bugs/forumpost/64c52890a4): Two pending UPDATE ... RETURNING statements finished oldest-first close their statement transactions out of order (since 3.35.0) (open)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/38492ae5dc): R-Tree and Geopoly: if a node is longer than SQLITE_LIMIT_LENGTH, an INSERT succeeds and stores NULL in its place (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/f712b4e116): FTS5: if a leaf is longer than SQLITE_LIMIT_LENGTH, an INSERT succeeds and stores NULL in its place (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/2fb4734a27): SQLite: weak candidate, not filed (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/53f21c7fcf): A statement that gets SQLITE_BUSY from an attached database keeps its statement transaction open (open)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/250f323f45): FTS5, FTS3/4, R-Tree, Geopoly: a single-row write that fails inside a transaction leaves the module's partial shadow-table writes, and COMMIT keeps them (open)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/bc159f8204): FTS5: ROLLBACK frees the index structure under an open MATCH query in another statement (heap use-after-free) (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/45bcbf3580): A function result that carries a number and its text keeps the old text after REAL affinity: wrong RETURNING output and FOREIGN KEY failures (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/7c7dc3813e): OP_SeekRowid converts a key that has both a number and its text from the text: assertion, and the wrong row with a short fp_digits (since 3.54.0) (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/b53a65d1fc): sum(b ORDER BY a) FROM t1, t2 sums a instead of b: an aggregate's ORDER BY term replaces its argument (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/a9c654a346): In a UTF-16 database, a blob DEFAULT from ALTER TABLE ADD COLUMN is converted to text as UTF-8 (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/73f9f9a7fd): UNION of four or more SELECTs drops rows: the balanced merge removes the right half's duplicates under that half's collation (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/403ca95409): A join on an INTEGER PRIMARY KEY misses the REAL key -9223372036854775808.0 when the plan has a Bloom filter (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/e2d2dd7c5b): Compound SELECT with ORDER BY ... COLLATE moves the last SELECT's GROUP BY, HAVING and DISTINCT to the whole compound (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/85d3969644): database disk image is malformed from a write after a one-pass DELETE in the same statement (WITHOUT ROWID): the delete cursor keeps the deleted cell's CellInfo (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/5e80e94512): ALTER TABLE DROP CONSTRAINT on a foreign key with SET DEFAULT, NOT DEFERRABLE or MATCH fails or leaves part of the constraint behind (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/28af38d7c3): ALTER COLUMN DROP NOT NULL removes NOT DEFERRABLE instead of NOT NULL, or removes a DEFERRABLE clause that follows NOT NULL (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/529732a774): Join drops rows, or adds some, when a table has an index on an expression and a `col = constant` WHERE term (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/b6e0cf3df3): A UNION ALL subquery in FROM with a RIGHT JOIN after its first SELECT loses WHERE terms when flattened (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/5cffabd8e9): FTS5: wrong instances for phrases below a non-matching AND, and on the right of a NOT whose left side matches no row (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/9f84b69d92): FTS5: DROP TABLE and ALTER TABLE RENAME skip the %_content table of a contentless_unindexed=1 table (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/e380b6876c): FTS5: with detail=none, secure-delete of a rowid that was replaced while secure-delete was off corrupts the index (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/a27cfbee6c): FTS5: "no such cursor: 0" from an auxiliary function in WHERE with the `tbl = 'query'` form (constant propagation) (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/c7ab2c9250): FTS5: INSERT into a locale=1, contentless_unindexed=1 table with an UNINDEXED column fails with "database disk image is malformed" (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/49d216c497): FTS5: MATCH with `rowid = <non-integer value>` returns every match (regression after 3.51) (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/9fa08e2dbd): FTS5: phrases of a NEAR group that does not match a row report instances for that row, some left over from an earlier row (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/5cffabd8e9): FTS5: the right operand of a NOT reports instances when the left operand matches no row (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/737cb495ea): FTS5: with secure-delete on, a merge can drop a delete marker, and a later secure delete makes an old entry for the rowid match again (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/7b3cda966f): FTS5 tokendata=1: a query for an absent term fails with "fts5: corruption found reading blob" after a long doclist (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/5c53f75fbb): FTS5 tokendata=1: a prefix query followed by a term query in an open transaction reads freed memory (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/402c7177cb): FTS5 trigram: a LIKE pushed down to FTS5 makes sqlite3Fts5ExprPattern() read one byte past the end of the pattern (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/be50ab5692): FTS5 trigram: LIKE and GLOB on an UNINDEXED column return no rows (open)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/a3eee51a91): FTS5 detail=none/column: snippet() fails with "database disk image is malformed", highlight() and bm25() see matches in UNINDEXED columns (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/d29f636f37): FTS5: fts5vocab rowids restart in every scan, so `term = ? OR term < ?` and RIGHT JOIN return wrong rows (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/b0c03988e8): FTS5: in a UTF-16 database, fts5vocab applies `term <` / `>` and ORDER BY term in UTF-8 order (open)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/d29f636f37): fts5vocab: col IS NULL returns no rows since the WITHOUT ROWID schema change (reply in d29f636f37) (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/26213fdfb9): FULL JOIN USING column reads back as REAL through a subquery that is not flattened (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/6973ad169e): An index on a VIRTUAL generated column with no type returns an integral REAL as an INTEGER (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/6d40716f60): Geopoly: `rowid = <non-integer value>` finds the row of its integer part (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/1b9d323951): Geopoly: an UPDATE that changes the rowid but not _shape fails with "_shape does not contain a valid polygon" (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/ef313c69fc): group_concat() window over a moving frame drops separators and text after an empty string (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/5cd4b3b27a): Rows out of order despite ORDER BY x DESC in a query with GROUP BY x over a subquery sorted by x ASC (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/f505c400cf): x IN (SELECT y COLLATE NOCASE ...) misses rows when x has an index (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/50dd78a44b): Rows missing when a UNION ALL subquery or CTE is joined with an inner join followed by a RIGHT or FULL JOIN (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/8e5c07be86): A correlated subquery over a three-way join counts 0 for later outer rows (JoinMiss deletes automatic-index entries) (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/1339f93b28): json_valid() returns 0 for valid JSON text based on json_set call (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/41325cafc0): LEFT JOIN with `x = NULL AND y IN (…)` on a two-column index returns a matched row as well as the NULL-extended one (regression in 3.43.0) (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/090de3eb79): An OR in a LEFT JOIN's ON clause drives a loop on the left table: wrong rows, or a segfault when the joined table is omitted (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/74bf8db543): LIKE optimization reads an uninitialized counter under a LEFT JOIN (reply in 74bf8db543) (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/77efe1e826): a IN (...) AND c GLOB 'A' with an index on (a,c): GROUP BY splits a group, DISTINCT repeats a row (BLOB in c) (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/cd41fceaa0): NOT INDEXED is ignored on WITHOUT ROWID tables: the planner still uses secondary and UNIQUE indexes (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/3ede79ce42): An OR term that uses a partial index WHERE a=5 and also tests a fails with "internal query planner error" (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/48c64d8bab): In a UTF-16 database, a blob over 4000 bytes read from a table's overflow pages is converted as UTF-8 (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/36a73e8c8a): In a UTF-16 database, text over 4000 bytes read from a table loses a leading U+FEFF, and an UPDATE then writes it back without it (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/35992eec46): A partial index WHERE col = <constant> makes col read back as the constant: a TEXT '5.0' comes back as '5' (partly fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/0d4c9e9e99): HAVING-to-WHERE and window push-down drop rows when a group holds both 1 and 1.0 (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/9bbaa34eea): WHERE b = -9223372036854775808.0 AND typeof(b) = 'integer' misses a row whose INT column b holds that integer (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/b088eb5b02): RENAME COLUMN fails on an INTEGER PRIMARY KEY column named in a UNIQUE table constraint (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/8be1215e2c): `SELECT *` over a USING join left of a RIGHT or FULL JOIN is "ambiguous column name" when a later table has that column (open)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/19d2bc80bf): Row-value IN in a join repeats rows: the inner loop iterates the full IN list, with duplicates, for one of its columns (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/1783a82ae4): R-Tree: `UPDATE ... SET rowid = N` and `INSERT INTO ...(rowid, ...)` ignore the new rowid (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/3bc3c8adfa): ROLLBACK TO of a savepoint that dropped a table commits a stale root page in an incremental-vacuum database (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/c75ba2bcd0): ALTER TABLE RENAME COLUMN of a TEMP table fails when a TEMP trigger or view uses the column (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/1f92d7105b): SQL logic error on the next SAVEPOINT or COMMIT, and the transaction is lost, after ROLLBACK TO undoes a CREATE VIRTUAL TABLE (fts5, fts4) (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/069f778165): With a window function, a correlated LEFT JOIN whose ON clause has a bare outer column as an AND term filters the left rows (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/eee345f931): database disk image is malformed or missing rows: ORDER BY an added column NULLS LAST on a WITHOUT ROWID table (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/eee345f931): database disk image is malformed or wrong answer: row-value IN (SELECT ...) over an added column of a WITHOUT ROWID table (open)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/f554fe3656): Multi-index OR on a WITHOUT ROWID table misses rows when an OR-term uses OP_SeekScan (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/eee345f931): database disk image is malformed or missing rows: row-value range over an added column of a WITHOUT ROWID table (open)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/420911fdac): In a UTF-16 database, a zeroblob() result used as text is converted from UTF-8, so CAST(zeroblob(4) AS TEXT) has 8 bytes and the result depends on the query plan (fixed)
- [SQLite bug forum](https://sqlite.org/bugs/forumpost/9d039d4ca7): DELETE reports "database disk image is malformed" on a database that passes integrity_check (WITHOUT ROWID, tiny key) (fixed)

## uutils coreutils

[github.com/uutils/coreutils](https://github.com/uutils/coreutils)

- [uutils/coreutils#15439](https://github.com/uutils/coreutils/issues/15439): comm: sorted input rejected as "not in sorted order" when a line is a prefix of the next followed by a tab (fixed)
- [uutils/coreutils#15440](https://github.com/uutils/coreutils/issues/15440): csplit: a `/REGEX/` pattern after a line-number pattern never tests the line the split stopped at (open)
- [uutils/coreutils#15569](https://github.com/uutils/coreutils/issues/15569): split: a suffix start value too large for the suffix length is accepted when input is empty (regression on main) (fixed)
- [uutils/coreutils#15570](https://github.com/uutils/coreutils/issues/15570): sort: a key's own `r` is ignored when the key is the whole line (`-k1r`) (fixed)
- [uutils/coreutils#15571](https://github.com/uutils/coreutils/issues/15571): fold: control characters such as `\f`, ESC and DEL take no column, so long lines are not folded (open)
- [uutils/coreutils#15572](https://github.com/uutils/coreutils/issues/15572): seq: `-w` pads to the width of the increment, which is never printed (fixed)
- [uutils/coreutils#15574](https://github.com/uutils/coreutils/issues/15574): numfmt: `--to=si`/`--to=iec` ignores `--round` for values from 10 to 999 that get no suffix (open)
- [uutils/coreutils#15575](https://github.com/uutils/coreutils/issues/15575): numfmt: with `--invalid=fail/warn/ignore`, one invalid field leaves the whole line unconverted (fixed)
- [uutils/coreutils#15576](https://github.com/uutils/coreutils/issues/15576): join: with `-i`, every line of a matching group is printed with the first line's join field (fixed)
- [uutils/coreutils#15577](https://github.com/uutils/coreutils/issues/15577): sort: `-h` ignores `-f`, so lowercase suffixes such as `m`, `g`, `e` are not folded (fixed)
- [uutils/coreutils#15578](https://github.com/uutils/coreutils/issues/15578): sort: global `-i` is not overridden by `-d` (fixed)
- [uutils/coreutils#15579](https://github.com/uutils/coreutils/issues/15579): expand: `--tabs=2,/2` and `--tabs=4,+2` are rejected with "tab sizes must be ascending" (fixed)
- [uutils/coreutils#15580](https://github.com/uutils/coreutils/issues/15580): expand: a NUL byte takes one column, so the next tab stop comes one column early (fixed)
- [uutils/coreutils#15581](https://github.com/uutils/coreutils/issues/15581): split: `-a 0` with `-n N` uses a 1-character suffix instead of the default length 2 (fixed)
- [uutils/coreutils#15583](https://github.com/uutils/coreutils/issues/15583): split: `--verbose` prints nothing with `-n`/`--number` (open)
- [uutils/coreutils#15584](https://github.com/uutils/coreutils/issues/15584): csplit: with `--suppress-matched`, a line number one past the end of input is rejected as out of range (fixed)
- [uutils/coreutils#15585](https://github.com/uutils/coreutils/issues/15585): fmt: one space instead of two after a sentence-ending first word at the end of a line (fixed)
- [uutils/coreutils#15586](https://github.com/uutils/coreutils/issues/15586): nl: header/footer section is reset to body at the start of each input file (fixed)
- [uutils/coreutils#15587](https://github.com/uutils/coreutils/issues/15587): sort: `-V` ignores `-d` and `-i` (follow-up to #15463) (open)
- [uutils/coreutils#15588](https://github.com/uutils/coreutils/issues/15588): sort: `-b` and `-M` still skip leading form feed and carriage return (follow-up to #14119) (fixed)
- [uutils/coreutils#15589](https://github.com/uutils/coreutils/issues/15589): tr: `[c*]` accepted with `-d -s`, `[=c=]` accepted in string2 when translating, and a case class past the end of string1 rejected (partly fixed)
- [uutils/coreutils#15591](https://github.com/uutils/coreutils/issues/15591): nl: unnumbered lines are padded with width+1 spaces, ignoring the length of `-s` (fixed)
- [uutils/coreutils#15592](https://github.com/uutils/coreutils/issues/15592): od: `-S` with `-Ax` prints 7-digit hex offsets instead of 6 (fixed)
- [uutils/coreutils#15646](https://github.com/uutils/coreutils/issues/15646): dd: `conv=swab` pairs bytes within each input block, so the result depends on `bs=` and on how a pipe delivers data (open)
- [uutils/coreutils#15653](https://github.com/uutils/coreutils/issues/15653): csplit: hangs on a `/REGEX/-N` pattern after a line-number pattern that names the line a regex stopped at (open)
- [uutils/coreutils#15654](https://github.com/uutils/coreutils/issues/15654): cut: with `--output-delimiter`, the last field is printed twice when the line has no terminator (fixed)
- [uutils/coreutils#15655](https://github.com/uutils/coreutils/issues/15655): cat: `-s` drops a single blank line when the line before it was continued from an earlier file or read (fixed)
- [uutils/coreutils#15656](https://github.com/uutils/coreutils/issues/15656): expand: tab stops are misplaced after a file without a trailing newline, and on lines longer than 8 KiB (fixed)
- [uutils/coreutils#15657](https://github.com/uutils/coreutils/issues/15657): unexpand: a line that continues from one FILE into the next is handled as two lines (fixed)
- [uutils/coreutils#15658](https://github.com/uutils/coreutils/issues/15658): expand/unexpand: a bare `-` operand is taken as an obsolete `-N` tab option, so standard input is skipped (fixed)
- [uutils/coreutils#15659](https://github.com/uutils/coreutils/issues/15659): uniq: `-f` treats carriage return and form feed as field separators (fixed)
- [uutils/coreutils#15660](https://github.com/uutils/coreutils/issues/15660): split: the `-l` count is parsed like a byte SIZE (`-l 010` means 8 lines, `-l 1k` is accepted) (open)
- [uutils/coreutils#15661](https://github.com/uutils/coreutils/issues/15661): numfmt: a negative number that rounds to zero is printed as `-0` (fixed)
- [uutils/coreutils#15662](https://github.com/uutils/coreutils/issues/15662): printf: `%e` and `%g` round an exact halfway value up instead of to even (`%.0e` of 2.5 gives `3e+00`) (open)
- [uutils/coreutils#15663](https://github.com/uutils/coreutils/issues/15663): printf: `%.*s` accepts a `*` precision above 2147483647 instead of failing with "invalid precision" (fixed)
- [uutils/coreutils#15664](https://github.com/uutils/coreutils/issues/15664): basenc: `--base64url -d` rejects input whose last group has no `=` padding (fixed)
- [uutils/coreutils#15665](https://github.com/uutils/coreutils/issues/15665): chmod: modes like `-=r`, `-+x` and `-,u+x` are rejected as unknown options (fixed)
- [uutils/coreutils#15666](https://github.com/uutils/coreutils/issues/15666): chmod --reference: the first FILE operand is processed last (fixed)

## gitoxide

[github.com/GitoxideLabs/gitoxide](https://github.com/GitoxideLabs/gitoxide)

- [GitoxideLabs/gitoxide#3089](https://github.com/GitoxideLabs/gitoxide/issues/3089): Reflog specs (`<ref>@{N}`, `@{0}`, `HEAD@{n}`) disagree with git in four cases (follow-up to #3040 / #3047) (fixed)
- [GitoxideLabs/gitoxide#3089](https://github.com/GitoxideLabs/gitoxide/issues/3089): Reflog specs (`<ref>@{N}`, `@{0}`, `HEAD@{n}`) disagree with git in four cases (follow-up to #3040 / #3047) (fixed)
- [GitoxideLabs/gitoxide#3089](https://github.com/GitoxideLabs/gitoxide/issues/3089): Reflog specs (`<ref>@{N}`, `@{0}`, `HEAD@{n}`) disagree with git in four cases (follow-up to #3040 / #3047) (fixed)
- [GitoxideLabs/gitoxide#3089](https://github.com/GitoxideLabs/gitoxide/issues/3089): Reflog specs (`<ref>@{N}`, `@{0}`, `HEAD@{n}`) disagree with git in four cases (follow-up to #3040 / #3047) (fixed)
- [GitoxideLabs/gitoxide#3090](https://github.com/GitoxideLabs/gitoxide/issues/3090): Dangling symbolic refs win DWIM lookup: short names, short hex prefixes and `@{-1}` fail or resolve to the wrong object (fixed)
- [GitoxideLabs/gitoxide#3090](https://github.com/GitoxideLabs/gitoxide/issues/3090): Dangling symbolic refs win DWIM lookup: short names, short hex prefixes and `@{-1}` fail or resolve to the wrong object (fixed)
- [GitoxideLabs/gitoxide#3090](https://github.com/GitoxideLabs/gitoxide/issues/3090): Dangling symbolic refs win DWIM lookup: short names, short hex prefixes and `@{-1}` fail or resolve to the wrong object (fixed)
- [GitoxideLabs/gitoxide#3091](https://github.com/GitoxideLabs/gitoxide/issues/3091): gix-merge: debug assertion panic when one side renames and modifies a file and the other side makes the same modification (fixed)
- [GitoxideLabs/gitoxide#3095](https://github.com/GitoxideLabs/gitoxide/issues/3095): `@{-1}` peels an annotated tag that git returns unpeeled (fixed)
- [GitoxideLabs/gitoxide#3096](https://github.com/GitoxideLabs/gitoxide/issues/3096): `@{push}` resolution ignores git's fallbacks: no default `origin`, and `push.default=upstream` maps through the push remote (fixed)
- [GitoxideLabs/gitoxide#3096](https://github.com/GitoxideLabs/gitoxide/issues/3096): `@{push}` resolution ignores git's fallbacks: no default `origin`, and `push.default=upstream` maps through the push remote (fixed)
- [GitoxideLabs/gitoxide#3097](https://github.com/GitoxideLabs/gitoxide/issues/3097): gix-merge: same blob on both sides with different modes falls through to `Unknown` (path dropped, or ancestor kept) (fixed)
- [GitoxideLabs/gitoxide#3097](https://github.com/GitoxideLabs/gitoxide/issues/3097): gix-merge: same blob on both sides with different modes falls through to `Unknown` (path dropped, or ancestor kept) (fixed)
- [GitoxideLabs/gitoxide#3098](https://github.com/GitoxideLabs/gitoxide/issues/3098): `@{u}@{1}` is rejected with "unconsumed input"; git reads the upstream ref's reflog (fixed)
- [GitoxideLabs/gitoxide#3099](https://github.com/GitoxideLabs/gitoxide/issues/3099): `<name>@{0}` fails with "reflog entries require a ref name" when the branch name looks like a short hash (fixed)
- [GitoxideLabs/gitoxide#3100](https://github.com/GitoxideLabs/gitoxide/issues/3100): `HEAD^{/text}` on an unborn HEAD searches all refs instead of failing (fixed)
- [GitoxideLabs/gitoxide#3101](https://github.com/GitoxideLabs/gitoxide/issues/3101): SHA-256 repositories: loose objects are never found by an abbreviated id (fixed)
- [GitoxideLabs/gitoxide#3102](https://github.com/GitoxideLabs/gitoxide/issues/3102): `rev_parse` resolves `HEAD:file/` to the blob of `file`; git rejects a trailing slash on a non-directory (fixed)
- [GitoxideLabs/gitoxide#3103](https://github.com/GitoxideLabs/gitoxide/issues/3103): `Commit::describe()` ignores annotated tags that point to another tag (fixed)
- [GitoxideLabs/gitoxide#3104](https://github.com/GitoxideLabs/gitoxide/issues/3104): `rev_walk().with_hidden()` returns ancestors of hidden commits when commit times are skewed and there is no commit-graph (fixed)
- [GitoxideLabs/gitoxide#3105](https://github.com/GitoxideLabs/gitoxide/issues/3105): `gix_traverse::commit::topo` returns a tip that an end hides (with `Parents::First`, or when the tip is also an end) (fixed)
- [GitoxideLabs/gitoxide#3106](https://github.com/GitoxideLabs/gitoxide/issues/3106): Tree merge reports a false directory/file conflict when one side replaces `d/` with a file and the other only deletes part of `d/` (fixed)
- [GitoxideLabs/gitoxide#3107](https://github.com/GitoxideLabs/gitoxide/issues/3107): Tree merge keeps only the ancestor when one side edits a file and the other turns it into a symlink or submodule (fixed)
- [GitoxideLabs/gitoxide#3108](https://github.com/GitoxideLabs/gitoxide/issues/3108): gix-merge: moving `d/e` to `y/z` makes the tree merge treat `d` as renamed to `y` (fixed)
- [GitoxideLabs/gitoxide#3109](https://github.com/GitoxideLabs/gitoxide/issues/3109): gix-merge: tree merge loses a file renamed onto a directory that the other side modified (fixed)
- [GitoxideLabs/gitoxide#3110](https://github.com/GitoxideLabs/gitoxide/issues/3110): Text merge reports no conflict when both sides append different lines at the end of a file (diff hunks placed with the indent heuristic) (fixed)
- [GitoxideLabs/gitoxide#3111](https://github.com/GitoxideLabs/gitoxide/issues/3111): `status()` reports an executable file replaced by a symlink as `Modified` instead of `TypeChange` (fixed)
- [GitoxideLabs/gitoxide#3112](https://github.com/GitoxideLabs/gitoxide/issues/3112): `status()` fails when HEAD's branch is a symbolic ref to an unborn branch (fixed)
- [GitoxideLabs/gitoxide#3113](https://github.com/GitoxideLabs/gitoxide/issues/3113): `status()` lists a file that replaced a submodule (gitlink) as untracked, in addition to the type change (fixed)
- [GitoxideLabs/gitoxide#3114](https://github.com/GitoxideLabs/gitoxide/issues/3114): `gix status` ignores a gitlink whose path is not listed in `.gitmodules` (fixed)
- [GitoxideLabs/gitoxide#3115](https://github.com/GitoxideLabs/gitoxide/issues/3115): `status()` / `gix_diff::index()` miss a staged deletion when the index entry is intent-to-add (fixed)

## DuckDB

[github.com/duckdb/duckdb](https://github.com/duckdb/duckdb)

- [duckdb/duckdb#27445](https://github.com/duckdb/duckdb/issues/27445): INTERNAL Error in join elimination (`!column_bindings.empty()`) on a correlated subquery with a never-evaluated `count(DISTINCT ...)` (open)
- [duckdb/duckdb#27446](https://github.com/duckdb/duckdb/issues/27446): INTERNAL Error `exprs.size() > 0` (expression_executor.cpp) when statistics propagation folds an aggregate with no aggregate expressions (`HAVING` without aggregates) (fixed)
- [duckdb/duckdb#27447](https://github.com/duckdb/duckdb/issues/27447): INTERNAL Error `physical_index != DConstants::INVALID_INDEX` in propagate_get.cpp when statistics fold a pushed-down filter's column away (fixed)
- [duckdb/duckdb#27448](https://github.com/duckdb/duckdb/issues/27448): Segfault in `dense_rank() OVER ()` over a RIGHT_ANTI hash join that returns its unmatched rows (fixed)
- [duckdb/duckdb#27449](https://github.com/duckdb/duckdb/issues/27449): INTERNAL Error "Projection map references a missing child column": join elimination leaves the parent filter's projection map stale (open)
- [duckdb/duckdb#27450](https://github.com/duckdb/duckdb/issues/27450): INTERNAL Error "GetPartitioningSpaceRequirement: tuple width should not be 0" for a hash join whose probe side has no columns (fixed)
- [duckdb/duckdb#27451](https://github.com/duckdb/duckdb/issues/27451): Join order optimizer drops a filter that references no column (`WHERE NULL`, `WHERE random() > 2`) when filter_pushdown is disabled (fixed)
- [duckdb/duckdb#27452](https://github.com/duckdb/duckdb/issues/27452): INTERNAL Error "Failed to bind column reference" when an always-matching SEMI JOIN (`ON true`) is reordered by the join order optimizer (open)
- [duckdb/duckdb#27633](https://github.com/duckdb/duckdb/issues/27633): SEMI JOIN with a right-side-only condition returns no rows when filter_pushdown is disabled (open)
- [duckdb/duckdb#27711](https://github.com/duckdb/duckdb/issues/27711): Wrong result: float/double `abs()` statistics are backwards when the column's max is 0 and its min is negative (fixed)
- [duckdb/duckdb#27712](https://github.com/duckdb/duckdb/issues/27712): Wrong result: WHERE (1 IN ((SELECT max(1) WHERE b), 1)) <> b drops a qualifying row (regression since v1.5.6) (fixed)
- [duckdb/duckdb#27714](https://github.com/duckdb/duckdb/issues/27714): Wrong result: LIMIT 1 returns 3 rows when joined with an ungrouped aggregate over an empty table (fixed)
- [duckdb/duckdb#27715](https://github.com/duckdb/duckdb/issues/27715): Wrong result: WHERE contains(CASE WHEN b THEN 'a' END, 'a') keeps rows where the predicate is NULL (fixed)
- [duckdb/duckdb#27716](https://github.com/duckdb/duckdb/issues/27716): INTERNAL Error !ContainsDependentJoin in planner.cpp: lateral reference in FULL JOIN ON subquery, join input with correlated subquery (open)
- [duckdb/duckdb#27717](https://github.com/duckdb/duckdb/issues/27717): Nested loop / piecewise merge joins ignore the left projection map above an outer join with a subquery in its ON clause (INTERNAL errors, wrong columns) (fixed)
- [duckdb/duckdb#27717](https://github.com/duckdb/duckdb/issues/27717): Nested loop / piecewise merge joins ignore the left projection map above an outer join with a subquery in its ON clause (INTERNAL errors, wrong columns) (fixed)
- [duckdb/duckdb#27717](https://github.com/duckdb/duckdb/issues/27717): Nested loop / piecewise merge joins ignore the left projection map above an outer join with a subquery in its ON clause (INTERNAL errors, wrong columns) (fixed)
- [duckdb/duckdb#27719](https://github.com/duckdb/duckdb/issues/27719): INTERNAL Error "Failed to bind column reference" for LEFT JOIN ... ON false AND x IN (SELECT ...) WHERE true (open)
- [duckdb/duckdb#27720](https://github.com/duckdb/duckdb/issues/27720): INTERNAL Error "BoundReferenceExpression should not be used here yet!" for a lambda inside an aggregate inside a lambda (open)
- [duckdb/duckdb#27722](https://github.com/duckdb/duckdb/issues/27722): INTERNAL Error: assertion in join_hashtable.cpp:1474 for join query with WHERE NOT EXISTS (SELECT 1) (assertion builds) (open)
- [duckdb/duckdb#27723](https://github.com/duckdb/duckdb/issues/27723): INTERNAL Error: Assertion "normalized" in query_graph_manager.cpp on CROSS JOIN + LEFT JOIN ON (SELECT true) + WHERE (open)
- [duckdb/duckdb#27724](https://github.com/duckdb/duckdb/issues/27724): INTERNAL Error (assertion string_stats.cpp:715) for SIMILAR TO filter above FULL JOIN (open)
- [duckdb/duckdb#27725](https://github.com/duckdb/duckdb/issues/27725): INTERNAL Error "Statistics mismatch: vector labeled as not having NULL values" with debug_verify_stats on a join with `a < b AND a = b - 1` (open)
- [duckdb/duckdb#27726](https://github.com/duckdb/duckdb/issues/27726): INTERNAL Error "Vector::Reference used on vector of different type" for FULL JOIN with a subquery in ON over a CROSS JOIN, projection_pullup disabled (open)
- [duckdb/duckdb#27737](https://github.com/duckdb/duckdb/issues/27737): Hang: correlated NOT EXISTS (... EXCEPT ALL ...) query never finishes on v2.0.0 (works on v1.5.6) (fixed)
- [duckdb/duckdb#27776](https://github.com/duckdb/duckdb/issues/27776): EXPLAIN (SQL) exports quantile_disc with a column reference as the quantile argument when expression_rewriter is disabled (fixed)

## brush

[github.com/reubeno/brush](https://github.com/reubeno/brush)

- [reubeno/brush#1568](https://github.com/reubeno/brush/issues/1568): `"${a[@]+x}"` with an empty array gives one empty word instead of no words (fixed)
- [reubeno/brush#1569](https://github.com/reubeno/brush/issues/1569): Brace expansion is not applied to assignment arguments of `declare`/`local`/`export` (open)
- [reubeno/brush#1570](https://github.com/reubeno/brush/issues/1570): Negative subscript on a sparse array counts from the number of elements, not from the highest index (fixed)
- [reubeno/brush#1576](https://github.com/reubeno/brush/issues/1576): Command substitution hangs when a builtin writes more than 64 KiB after a nested `$( )` (open)
- [reubeno/brush#1577](https://github.com/reubeno/brush/issues/1577): `+=` on a `declare -i` variable panics with "attempt to add with overflow" in builds with overflow checks (fixed)
- [reubeno/brush#1578](https://github.com/reubeno/brush/issues/1578): A syntax error later in a script file or -c string stops every earlier command from running (open)
- [reubeno/brush#1579](https://github.com/reubeno/brush/issues/1579): read drops control characters from a pipe, file or here-string, and treats ^C/^D in the data as keystrokes (open)
- [reubeno/brush#1580](https://github.com/reubeno/brush/issues/1580): declare -f indents lines inside multi-line strings and here-documents, changing the function (open)
- [reubeno/brush#1581](https://github.com/reubeno/brush/issues/1581): `shopt -s globstar` is accepted, but `**` does not match recursively (open)
- [reubeno/brush#1582](https://github.com/reubeno/brush/issues/1582): nullglob removes a word like `[a/b]` that is not a pattern (regression since #1153) (open)
- [reubeno/brush#1583](https://github.com/reubeno/brush/issues/1583): Pathname expansion: backslash from a variable is kept in a glob path component that has no glob characters (fixed)
- [reubeno/brush#1584](https://github.com/reubeno/brush/issues/1584): Glob `"".*` (empty quotes before the leading dot) does not match dot files (fixed)
- [reubeno/brush#1585](https://github.com/reubeno/brush/issues/1585): `${v:=word}` expands to `word` instead of the new value of `v` when `v` has `declare -u/-l/-c` (open)
- [reubeno/brush#1586](https://github.com/reubeno/brush/issues/1586): `${var/pattern/string}`: unquoted `&` in the replacement is not replaced by the match (patsub_replacement is on but not implemented) (open)
- [reubeno/brush#1587](https://github.com/reubeno/brush/issues/1587): `[[ -z == x ]]` is accepted as a string comparison; bash reads `-z` as the unary operator and rejects it (fixed)
- [reubeno/brush#1588](https://github.com/reubeno/brush/issues/1588): `echo -n -n hi` fails with a usage error, and `-e` wins over a later `-E` (fixed)
- [reubeno/brush#1589](https://github.com/reubeno/brush/issues/1589): printf: "invalid number" message ignores `2>` redirection, and the exit status is 0 (open)
- [reubeno/brush#1590](https://github.com/reubeno/brush/issues/1590): printf: a large `*` width aborts printf with "formatting width too large" and drops the rest of the output (open)
- [reubeno/brush#1591](https://github.com/reubeno/brush/issues/1591): `~-` expands to an empty string when OLDPWD is not set (for example at startup) (fixed)
- [reubeno/brush#1592](https://github.com/reubeno/brush/issues/1592): Unexpanded `~01` loses its leading zero and prints as `~1` (open)
- [reubeno/brush#1593](https://github.com/reubeno/brush/issues/1593): An error on one line of a multi-line `$( )` does not end the command substitution (open)
- [reubeno/brush#1594](https://github.com/reubeno/brush/issues/1594): An assignment error inside a function does not abandon the rest of the calling line (open)
- [reubeno/brush#1595](https://github.com/reubeno/brush/issues/1595): `break`/`continue` outside a loop, or with a count larger than the loop depth, skip the rest of the line or fail the function (open)
- [reubeno/brush#1596](https://github.com/reubeno/brush/issues/1596): Error inside a compound command with `2>` ignores that redirect (open)

## Lean 4

[github.com/leanprover/lean4](https://github.com/leanprover/lean4)

- [leanprover/lean4#16160](https://github.com/leanprover/lean4/issues/16160): With `compiler.extract_closed false`, borrow inference treats string and big `Nat` literals as borrowed: use after free and wrong results (open)
- [leanprover/lean4#16168](https://github.com/leanprover/lean4/issues/16168): `grind` builds a proof the kernel rejects for `match` conditions where a literal meets a pattern variable (`decide ¬3 = a`) (fixed)
- [leanprover/lean4#16197](https://github.com/leanprover/lean4/issues/16197): PANIC "`getOptions` called inside a computation recording its dependencies" when `logAt` warns during instance synthesis (`decide` on `BitVec.twoPow 276`) (open)
- [leanprover/lean4#16198](https://github.com/leanprover/lean4/issues/16198): `match` on `Float`/`Float32` literal patterns treats equal-valued literals as different (`16777216` vs `16777217`, `5` vs `5.0`), so a matching alternative is skipped (fixed)
- [leanprover/lean4#16201](https://github.com/leanprover/lean4/issues/16201): With `compiler.postponeCompile`, `leanir` fails with ``Unknown constant `_private.Init.Data.Array.Basic.0.Array.mapMUnsafe.map` `` on any use of `Array.map` (open)
- [leanprover/lean4#16228](https://github.com/leanprover/lean4/issues/16228): `lia`/`grind`: kernel rejects the proof of `x % 3 ≤ 2 * x % 3 → 4 * x % 3 ≤ 2 * x % 3` (`eq_of_dvd_ediv` type mismatch) (fixed)
- [leanprover/lean4#16229](https://github.com/leanprover/lean4/issues/16229): `grind.debug`: PANIC in `checkEqc` on `-x = -1` for `Int8` (and `Int16`/`Int32`/`Int64`) after `x = 1` (fixed)
- [leanprover/lean4#16230](https://github.com/leanprover/lean4/issues/16230): Kernel refuses `2 ^ 16777217` as over the numeral size limit, though the result is 2 MB of the 128 MB budget (open)
- [leanprover/lean4#16231](https://github.com/leanprover/lean4/issues/16231): `grind (maxChar := 1)` with the `Sym.simp` normalizer builds a proof the kernel rejects for `(0 : Fin 1) = x ^ 2` (fixed)
- [leanprover/lean4#16242](https://github.com/leanprover/lean4/issues/16242): Kernel rejects `structure J : Type (u+1) where x : Sort (imax (u+1) u)` as "too big", though the elaborator accepts it (open)
- [leanprover/lean4#16246](https://github.com/leanprover/lean4/issues/16246): Kernel rejects the equation lemma `f.eq_1`: unknown constant `f.match_1.eq_1` (regression in 4.36.0-rc1) (fixed)
- [leanprover/lean4#16247](https://github.com/leanprover/lean4/issues/16247): Std.Time: `aaaa` formats the AM/PM marker as "PM" but parses only "post meridiem", so it cannot read back its own output (fixed)
- [leanprover/lean4#16248](https://github.com/leanprover/lean4/issues/16248): Std.Time: pattern `B` formats 22:00 as "10:00 at night" but parses it back as 10:00 (fixed)
- [leanprover/lean4#16257](https://github.com/leanprover/lean4/issues/16257): Std.Time: `Duration.toMilliseconds` rounds negative durations down (`-1ns` gives `-1ms`), unlike the other unit conversions (open)
- [leanprover/lean4#16259](https://github.com/leanprover/lean4/issues/16259): Equation lemmas fail to generate for `match id a, a with ...`: the hypothesis `id 0 = n + 1` is dropped from the statement (open)
- [leanprover/lean4#16267](https://github.com/leanprover/lean4/issues/16267): Segfault on the second `withImportModules` in one process: `findOLean`'s root cache keeps a `Name` from a freed `.olean` region (regression in 4.36.0-rc1) (fixed)
- [leanprover/lean4#16268](https://github.com/leanprover/lean4/issues/16268): Std.Http: a URI with no authority and a path starting with `//` prints as `https://a`, which parses back with host `a` (fixed)
- [leanprover/lean4#16269](https://github.com/leanprover/lean4/issues/16269): Std.Http: `URI.Builder.setUserInfo` does not percent-encode `:` in the username, so the printed URI parses back with different credentials (fixed)
- [leanprover/lean4#16285](https://github.com/leanprover/lean4/issues/16285): `contradiction` fails with the internal error "mkNoConfusion: No manifest constructors" on `h : 0 + 0 = 1` (regression since 4.22) (fixed)
- [leanprover/lean4#16286](https://github.com/leanprover/lean4/issues/16286): `grind_linarith`: internal error "linarith term has not been internalized" on negative `Int8`..`Int64` literals (regression after 4.36) (fixed)
- [leanprover/lean4#16287](https://github.com/leanprover/lean4/issues/16287): `grind_linarith`: internal error "structure is not a preorder" on `<` / `≤` over `Fin n` and `BitVec w` (fixed)
- [leanprover/lean4#16299](https://github.com/leanprover/lean4/issues/16299): `NameTrie.findLongestPrefix?` returns `none` when the lookup passes through a key-less node before leaving the trie (open)
- [leanprover/lean4#16300](https://github.com/leanprover/lean4/issues/16300): "failed to generate equational theorem" when a `match` discriminant such as `1 - a` becomes a closed term after unfolding (open)
- [leanprover/lean4#16305](https://github.com/leanprover/lean4/issues/16305): `String.Slice.revSplit` drops the empty piece at the start of the slice (open)

## ty

[github.com/astral-sh/ty](https://github.com/astral-sh/ty)

- [astral-sh/ty#4825](https://github.com/astral-sh/ty/issues/4825): Narrowing lost in a comprehension inside a nested function (open)
- [astral-sh/ty#4826](https://github.com/astral-sh/ty/issues/4826): Literal string compared with LiteralString narrows to Never in the != branch (fixed)
- [astral-sh/ty#4827](https://github.com/astral-sh/ty/issues/4827): `x == S.A` narrows a `str` to the StrEnum, although `"a" == S.A` (fixed)
- [astral-sh/ty#4828](https://github.com/astral-sh/ty/issues/4828): False positive when a generic call over `list[T]` is passed to a bounded TypeVar parameter (open)
- [astral-sh/ty#4829](https://github.com/astral-sh/ty/issues/4829): Dict literal is rejected for a `TypedDict \| dict[str, list[float]]` parameter: inner list inferred without context (fixed)
- [astral-sh/ty#4830](https://github.com/astral-sh/ty/issues/4830): Generic alias parameter `A[T]` (`type A[T] = list[T]`) gets no context from sibling arguments in a generic call (open)
- [astral-sh/ty#4831](https://github.com/astral-sh/ty/issues/4831): Mapping-pattern capture is `object` when the subject was narrowed to an intersection (e.g. after `if d:`) (fixed)
- [astral-sh/ty#4835](https://github.com/astral-sh/ty/issues/4835): Lambda gets no type context when the expected type is a union of two or more `Callable`s (open)
- [astral-sh/ty#4836](https://github.com/astral-sh/ty/issues/4836): Conditional of list literals passed to `list[T]` is rejected when a sibling argument also binds `T`, depending on branch order (open)
- [astral-sh/ty#4837](https://github.com/astral-sh/ty/issues/4837): [panic] Chained assignment with a missing value (debug assertions) (fixed)
- [astral-sh/ty#4839](https://github.com/astral-sh/ty/issues/4839): [panic] `--update-baseline` aborts: "user-provided comparison function does not correctly implement a total order" (fixed)
- [astral-sh/ty#4840](https://github.com/astral-sh/ty/issues/4840): `--fix` for `unused-ignore-comment` after a backslash continuation introduces a syntax error, so no fixes in the file are applied (fixed)
- [astral-sh/ty#4854](https://github.com/astral-sh/ty/issues/4854): `TypeAliasType("A", value=int)` is rejected: "Wrong number of arguments … expected 2, found 1" (fixed)
- [astral-sh/ty#4855](https://github.com/astral-sh/ty/issues/4855): Accepting a call-argument-name inlay hint can produce a syntax error (`f((1))` → `f((x=1))`, `f(1, 2)` → `f(x=1, 2)`) (fixed)
- [astral-sh/ty#4855](https://github.com/astral-sh/ty/issues/4855): Accepting a call-argument-name inlay hint can produce a syntax error (`f((1))` → `f((x=1))`, `f(1, 2)` → `f(x=1, 2)`) (fixed)
- [astral-sh/ty#4856](https://github.com/astral-sh/ty/issues/4856): Rename / find references of an attribute skip accesses in platform-dependent unreachable code (open)
- [astral-sh/ty#4858](https://github.com/astral-sh/ty/issues/4858): Call-argument-name inlay hint on a union callee uses the first element's parameter name (open)
- [astral-sh/ty#4859](https://github.com/astral-sh/ty/issues/4859): Invalid edit offer on variable-type inlay hint (fixed)
- [astral-sh/ty#4860](https://github.com/astral-sh/ty/issues/4860): Rename skips calls of a method of a `@final` generic class through a specialized instance (open)
- [astral-sh/ty#4865](https://github.com/astral-sh/ty/issues/4865): `unknown-argument` for extra keyword arguments to a pydantic-settings `BaseSettings` model with `extra="allow"` (fixed)
- [astral-sh/ty#4866](https://github.com/astral-sh/ty/issues/4866): Accepting the variable-type inlay hint on a subclass override of a `ClassVar` gives `invalid-attribute-override` (open)
- [astral-sh/ty#4868](https://github.com/astral-sh/ty/issues/4868): Pydantic `Field(...)` metadata in a quoted `Annotated[...]` field annotation is ignored (fixed)

## Bun

[github.com/oven-sh/bun](https://github.com/oven-sh/bun)

- [oven-sh/bun#48361](https://github.com/oven-sh/bun/issues/48361): `bun build` lowering of `using` in a loop rethrows an earlier iteration's error (open)
- [oven-sh/bun#48401](https://github.com/oven-sh/bun/issues/48401): `Bun.XML.parse` drops a `&#xA;` at the end of an `NMTOKEN` attribute value (open)
- [oven-sh/bun#48505](https://github.com/oven-sh/bun/issues/48505): `bun fmt --no-semi` drops the `;` after a TypeScript overload signature, and the next block becomes the function body (open)
- [oven-sh/bun#48506](https://github.com/oven-sh/bun/issues/48506): `bun fmt` puts a trailing comma after a setter's parameter, which JavaScriptCore rejects (open)
- [oven-sh/bun#48507](https://github.com/oven-sh/bun/issues/48507): `bun fmt` is not idempotent on an object key that it prints differently from the source (`\u{61}`, `[.5]`, `[ 5 ]`, `['\'']`) (open)
- [oven-sh/bun#48509](https://github.com/oven-sh/bun/issues/48509): `bun fmt` puts a comma inside an empty YAML flow mapping or sequence that it breaks, and the file no longer parses (open)
- [oven-sh/bun#48510](https://github.com/oven-sh/bun/issues/48510): `bun fmt` adds a space to indented code in a Markdown list item when it renumbers `99.` to `100.` (open)
- [oven-sh/bun#48601](https://github.com/oven-sh/bun/issues/48601): `bun fmt` adds or drops an empty line between Markdown blocks, and the empty line moves where a block ends (open)
- [oven-sh/bun#48601](https://github.com/oven-sh/bun/issues/48601): `bun fmt` adds or drops an empty line between Markdown blocks, and the empty line moves where a block ends (open)
- [oven-sh/bun#48601](https://github.com/oven-sh/bun/issues/48601): `bun fmt` adds or drops an empty line between Markdown blocks, and the empty line moves where a block ends (open)
- [oven-sh/bun#48601](https://github.com/oven-sh/bun/issues/48601): `bun fmt` adds or drops an empty line between Markdown blocks, and the empty line moves where a block ends (open)
- [oven-sh/bun#48602](https://github.com/oven-sh/bun/issues/48602): `bun fmt` adds an empty line to an empty fenced code block in Markdown, so the code is no longer empty (open)
- [oven-sh/bun#48603](https://github.com/oven-sh/bun/issues/48603): `bun fmt` shortens a Markdown code span from two backticks to one after a lone backtick, and the span then starts at that backtick (open)
- [oven-sh/bun#48604](https://github.com/oven-sh/bun/issues/48604): `bun fmt` moves a comment inside a YAML mapping value to column 0 when the first key has a tag or an anchor (open)
- [oven-sh/bun#48605](https://github.com/oven-sh/bun/issues/48605): `bun fmt` adds line breaks to a YAML `\|+` block scalar when an indented comment follows it, on every run (open)
- [oven-sh/bun#48606](https://github.com/oven-sh/bun/issues/48606): `bun fmt --prose-wrap always` breaks a YAML double-quoted string at an escaped space `\ `, which deletes the space (open)
- [oven-sh/bun#48607](https://github.com/oven-sh/bun/issues/48607): `bun fmt` splits a Markdown paragraph in two when emphasis holds a link followed by `[[]]` (open)
- [oven-sh/bun#48608](https://github.com/oven-sh/bun/issues/48608): `bun fmt --prose-wrap always` breaks a Markdown task mark `[ ]` across lines when a heading follows it, and the item loses its checkbox (open)
- [oven-sh/bun#48609](https://github.com/oven-sh/bun/issues/48609): `bun fmt` escapes the `*` that opens emphasis in Markdown `_*a *b** c_`, and `**a *b** c*` changes on the second run (open)

## FastMCP

[github.com/PrefectHQ/fastmcp](https://github.com/PrefectHQ/fastmcp)

- [PrefectHQ/fastmcp#6121](https://github.com/PrefectHQ/fastmcp/issues/6121): json_schema_to_type: const/enum accept true for 1, and an object-valued const in a union raises TypeError (fixed)
- [PrefectHQ/fastmcp#6121](https://github.com/PrefectHQ/fastmcp/issues/6121): json_schema_to_type: const/enum accept true for 1, and an object-valued const in a union raises TypeError (fixed)
- [PrefectHQ/fastmcp#6122](https://github.com/PrefectHQ/fastmcp/issues/6122): json_schema_to_type applies items to the prefixItems positions (fixed)
- [PrefectHQ/fastmcp#6123](https://github.com/PrefectHQ/fastmcp/issues/6123): json_schema_to_type rejects a valid object when a property's default doesn't match its own type (fixed)
- [PrefectHQ/fastmcp#6124](https://github.com/PrefectHQ/fastmcp/issues/6124): json_schema_to_type drops additionalProperties keys when the schema has properties but no type (fixed)
- [PrefectHQ/fastmcp#6125](https://github.com/PrefectHQ/fastmcp/issues/6125): tools/list fails for the whole server when a tool's root $ref points to a boolean schema (fixed)
- [PrefectHQ/fastmcp#6135](https://github.com/PrefectHQ/fastmcp/issues/6135): Mutually recursive Pydantic return types are hydrated as the wrong class in result.data (fixed)
- [PrefectHQ/fastmcp#6136](https://github.com/PrefectHQ/fastmcp/issues/6136): result.data is built from an anyOf branch that coerces the value (0 becomes False) (open)
- [PrefectHQ/fastmcp#6149](https://github.com/PrefectHQ/fastmcp/issues/6149): from_openapi drops a query value when an object query parameter has a property named like another parameter (fixed)
- [PrefectHQ/fastmcp#6150](https://github.com/PrefectHQ/fastmcp/issues/6150): from_openapi lists resource templates for non-identifier path parameters (e.g. {$top}) that can never be read (fixed)
- [PrefectHQ/fastmcp#6295](https://github.com/PrefectHQ/fastmcp/issues/6295): Redirect URI helpers accept raw-string URIs that resolve to a different host or path (backslash, trailing space/C0, IPv4 shorthand, percent-encoded or IDNA hosts) (fixed)
- [PrefectHQ/fastmcp#6296](https://github.com/PrefectHQ/fastmcp/issues/6296): OAuth client redirect turns a registered `myapp://` redirect_uri into `myapp:?...` (fixed)
- [PrefectHQ/fastmcp#6297](https://github.com/PrefectHQ/fastmcp/issues/6297): OAuth client redirect carries two `iss` parameters when the registered redirect_uri already repeats `iss` (fixed)
- [PrefectHQ/fastmcp#6298](https://github.com/PrefectHQ/fastmcp/issues/6298): `Client(mode="legacy", cache=CacheConfig(default_ttl_ms=...))` serves cached listings, although the cache is documented as inert on legacy connections (fixed)
- [PrefectHQ/fastmcp#6299](https://github.com/PrefectHQ/fastmcp/issues/6299): `logging/setLevel` is ignored for `ctx.log()` calls from tools of a mounted server (fixed)
- [PrefectHQ/fastmcp#6300](https://github.com/PrefectHQ/fastmcp/issues/6300): `ctx.elicit()` multi-select with `[[...]]` sends `items` without `"type": "string"`, which the 2025-11-25 schema requires (fixed)
- [PrefectHQ/fastmcp#6301](https://github.com/PrefectHQ/fastmcp/issues/6301): Versionless lookup through a mount does not fall back to the enabled version when the child server has list-filtering middleware (open)
- [PrefectHQ/fastmcp#6302](https://github.com/PrefectHQ/fastmcp/issues/6302): A supporting file named `SKILL.md` in a subfolder is listed as a separate skill and makes `sync_skills()` fail (fixed)
- [PrefectHQ/fastmcp#6303](https://github.com/PrefectHQ/fastmcp/issues/6303): A proxy served over stdio forwards only some of the backend's `ctx.log()` messages (fixed)

## PostgreSQL

[www.postgresql.org](https://www.postgresql.org)

- [PostgreSQL BUG #19758](https://www.postgresql.org/message-id/19758-d074798b35cc9215%40postgresql.org): Nested BETWEEN grows exponentially in parse analysis and statement_timeout cannot stop it (open)
- [PostgreSQL BUG #19759](https://www.postgresql.org/message-id/19759-acd3f33402e54f36%40postgresql.org): Hashed set operations and recursive UNION pre-size their hash table from the planner's estimate without the hash_mem cap (PostgreSQL 19 regression) (fixed)
- [PostgreSQL BUG #19760](https://www.postgresql.org/message-id/19760-8f9a0eba17bc8c92%40postgresql.org): CLUSTER / REPACK silently loses a row updated in a rolled-back subtransaction (partly fixed)
- [PostgreSQL BUG #19761](https://www.postgresql.org/message-id/19761-b72cfc453e487610%40postgresql.org): btree_gist date_ops: inserting infinity/-infinity into a multi-page GiST index on a date fails with "cannot subtract infinite dates" (open)
- [PostgreSQL BUG #19766](https://www.postgresql.org/message-id/19766-c3d105a4e0ec5487%40postgresql.org): EXPLAIN ANALYZE: Assert("toc->toc_entry[i].key != key") in shm_toc_insert (master only, since a241bc4fede) (open)
- [PostgreSQL BUG #19767](https://www.postgresql.org/message-id/19767-f1189fd6a83254b7%40postgresql.org): pg_get_viewdef prints a negated integer constant in ORDER BY / GROUP BY as a column position (open)
- [PostgreSQL BUG #19772](https://www.postgresql.org/message-id/19772-d8c886b4db230132%40postgresql.org): corrupt MVNDistinct entry (XX000) from GROUP BY with grouping sets over a column and an expression of it (open)
- [PostgreSQL BUG #19781](https://www.postgresql.org/message-id/19781-a8a20b6a904d9050%40postgresql.org): UPDATE moving a row of a self-referencing partitioned table fails with 23503 although ON UPDATE CASCADE would fix the reference (open)
- [PostgreSQL BUG #19782](https://www.postgresql.org/message-id/19782-02b66fb19c9ce85b%40postgresql.org): SET CONSTRAINTS in a released savepoint survives ROLLBACK TO an enclosing savepoint (open)
- [PostgreSQL BUG #19783](https://www.postgresql.org/message-id/19783-070e4bf0c73c1076%40postgresql.org): PostgreSQL: candidate from today's round, report drafted, not reported (fixed)
- [PostgreSQL BUG #19784](https://www.postgresql.org/message-id/19784-93f76236decd0656%40postgresql.org): no relation entry for relid from min/max aggregate optimization with = ANY over an aggregate (open)
- [PostgreSQL BUG #19786](https://www.postgresql.org/message-id/19786-70758103a4b2bff2%40postgresql.org): JSON_TABLE with a subquery in PASSING fails with "unexpected non-SELECT command in SubLink" (fixed)
- [PostgreSQL BUG #19787](https://www.postgresql.org/message-id/19787-d14d7f0f320f4495%40postgresql.org): could not find pathkey item to sort for a Gather Merge over a merge join after outer-join reordering (open)
- [PostgreSQL BUG #19789](https://www.postgresql.org/message-id/19789-b6592eeba8c6f6e1%40postgresql.org): expression not found in statistics object with MCV statistics on a domain over boolean (open)
- [PostgreSQL BUG #19790](https://www.postgresql.org/message-id/19790-fda013085342fe7f%40postgresql.org): cannot start a subtransaction when there are unprocessed inval messages after ALTER DOMAIN DROP CONSTRAINT (open)
- [PostgreSQL BUG #19791](https://www.postgresql.org/message-id/19791-ebe77f384f92b31b%40postgresql.org): ALTER COLUMN TYPE fails on an INCLUDE column of an index used by a foreign key (open)
- [PostgreSQL BUG #19798](https://www.postgresql.org/message-id/19798-5a5da856f5f71118%40postgresql.org): BUG #19798: "failed to build any 4-way joins" with ROLLUP over a LATERAL subquery (open)
- [PostgreSQL BUG #19799](https://www.postgresql.org/message-id/19799-dab86d769b08dc53%40postgresql.org): BUG #19799: Self-join elimination: "failed to build any 3-way joins" with LATERAL (open)

## sqlparser-rs

[github.com/apache/datafusion-sqlparser-rs](https://github.com/apache/datafusion-sqlparser-rs)

- [apache/datafusion-sqlparser-rs#2759](https://github.com/apache/datafusion-sqlparser-rs/issues/2759): `SHOW ... ILIKE '<pattern>'` is displayed without quotes (open)
- [apache/datafusion-sqlparser-rs#2760](https://github.com/apache/datafusion-sqlparser-rs/issues/2760): MySQL: `REGEXP`/`RLIKE` followed by `NULL` is rejected or misparsed (regression in 0.59.0) (open)
- [apache/datafusion-sqlparser-rs#2763](https://github.com/apache/datafusion-sqlparser-rs/issues/2763): PostgreSQL: `INTERVAL (p) '<value>'` loses the precision (open)
- [apache/datafusion-sqlparser-rs#2764](https://github.com/apache/datafusion-sqlparser-rs/issues/2764): MySQL: `FLUSH TABLES` consumes the `;`, so the next statement is rejected (open)
- [apache/datafusion-sqlparser-rs#2765](https://github.com/apache/datafusion-sqlparser-rs/issues/2765): Snowflake: `CREATE DATABASE` and `COPY INTO` consume the `;`, so the next statement is rejected (open)
- [apache/datafusion-sqlparser-rs#2801](https://github.com/apache/datafusion-sqlparser-rs/issues/2801): Spark/Databricks: `CACHE TABLE t;` is rejected (only EOF ends the optional query) (open)
- [apache/datafusion-sqlparser-rs#2802](https://github.com/apache/datafusion-sqlparser-rs/issues/2802): `x = ANY(SELECT ...) <op> y`: the operator after `)` is parsed into the ANY/ALL operand (open)
- [apache/datafusion-sqlparser-rs#2803](https://github.com/apache/datafusion-sqlparser-rs/issues/2803): `._name` after `)` or `]` fails with "Unexpected character '_'" (open)
- [apache/datafusion-sqlparser-rs#2804](https://github.com/apache/datafusion-sqlparser-rs/issues/2804): Table version clause (`AT(...)`, `FOR SYSTEM_TIME AS OF`, `VERSION AS OF`) is displayed after the alias and doesn't reparse (open)
- [apache/datafusion-sqlparser-rs#2805](https://github.com/apache/datafusion-sqlparser-rs/issues/2805): `SHOW TABLES ... LIKE 'p' LIMIT n` is displayed with the `LIKE` after `LIMIT` (open)
- [apache/datafusion-sqlparser-rs#2806](https://github.com/apache/datafusion-sqlparser-rs/issues/2806): `ROLLUP`/`CUBE`/`GROUPING SETS`: an element starting with `(` is always read as a sublist (`(SELECT 1)` rejected, `((b))` displayed as `(b)`) (open)
- [apache/datafusion-sqlparser-rs#2807](https://github.com/apache/datafusion-sqlparser-rs/issues/2807): Slice with a stride and no upper bound (`arr[1: :4]`) is displayed as `arr[1::4]`, which doesn't parse (open)
- [apache/datafusion-sqlparser-rs#2808](https://github.com/apache/datafusion-sqlparser-rs/issues/2808): DuckDB: `f(a == b)` is displayed as `f(a = b)`, which reparses as a named argument (open)
- [apache/datafusion-sqlparser-rs#3000](https://github.com/apache/datafusion-sqlparser-rs/issues/3000): MySQL: exponential parse time on chains of `BINARY` operands that fail to parse (regression from #2146) (open)
- [apache/datafusion-sqlparser-rs#3000](https://github.com/apache/datafusion-sqlparser-rs/issues/3000): MySQL: exponential parse time on chains of `BINARY` operands that fail to parse (regression from #2146) (open)
- [apache/datafusion-sqlparser-rs#3001](https://github.com/apache/datafusion-sqlparser-rs/issues/3001): Exponential parse time on nested qualified CEIL, FLOOR, SUBSTRING or TRIM calls (regression from #2685) (open)
- [apache/datafusion-sqlparser-rs#3002](https://github.com/apache/datafusion-sqlparser-rs/issues/3002): DuckDB: exponential parse time on unclosed nested qualified calls with a named argument (regression from #2685) (open)

## LiteLLM

[github.com/BerriAI/litellm](https://github.com/BerriAI/litellm)

- [BerriAI/litellm#48715](https://github.com/BerriAI/litellm/issues/48715): Gemini streaming reports finish_reason "length" for a tool call that ends with MAX_TOKENS, non-streaming reports "tool_calls" (open)
- [BerriAI/litellm#48716](https://github.com/BerriAI/litellm/issues/48716): OpenAI-compatible streaming keeps finish_reason "stop" when the response has tool calls, non-streaming reports "tool_calls" (fixed)
- [BerriAI/litellm#48831](https://github.com/BerriAI/litellm/issues/48831): Gemini conversion drops a parallel tool call when two calls have the same name and arguments, leaving more functionResponse than functionCall parts (fixed)
- [BerriAI/litellm#48832](https://github.com/BerriAI/litellm/issues/48832): responses() sends a function_call_output of empty text parts to the provider as the JSON text of the list (fixed)
- [BerriAI/litellm#49063](https://github.com/BerriAI/litellm/issues/49063): responses() with Gemini fails with "Missing corresponding tool call" when a reasoning item sits between two function_call items (fixed)
- [BerriAI/litellm#49925](https://github.com/BerriAI/litellm/issues/49925): [Bug]: Cached responses() call on an Anthropic model fails with "provider returned a response with no 'choices'" (fixed)
- [BerriAI/litellm#49927](https://github.com/BerriAI/litellm/issues/49927): [Bug]: responses() sends temperature to OpenAI o-series models even with drop_params=True (fixed)
- [BerriAI/litellm#49928](https://github.com/BerriAI/litellm/issues/49928): [Bug]: max_messages is counted on the translated chat messages for responses() and anthropic.messages on bridged providers (fixed)
- [BerriAI/litellm#49929](https://github.com/BerriAI/litellm/issues/49929): [Bug]: A cache hit returns finish_reason "tool_calls" for a stored Anthropic response that had "stop" (fixed)
- [BerriAI/litellm#49931](https://github.com/BerriAI/litellm/issues/49931): [Bug]: anthropic.messages.acreate(stream=True) caches an empty reply from a non-Anthropic model and replays it on every repeat (fixed)
- [BerriAI/litellm#49932](https://github.com/BerriAI/litellm/issues/49932): [Bug]: /engines/{model}/chat/completions and /openai/deployments/{model}/chat/completions return the provider model name without streaming (fixed)
- [BerriAI/litellm#49933](https://github.com/BerriAI/litellm/issues/49933): [Bug]: Router stream=True with a Gemini deployment raises the raw VertexAIError (async) or a TypeError (sync) when the stream fails to open (fixed)
- [BerriAI/litellm#49935](https://github.com/BerriAI/litellm/issues/49935): [Bug]: anthropic.messages.acreate(stream=True) on Bedrock raises the raw BedrockError for an exception event in the stream (open)
- [BerriAI/litellm#49936](https://github.com/BerriAI/litellm/issues/49936): [Bug]: OpenAI HTTP 403 raises APIError or BadRequestError instead of PermissionDeniedError (fixed)
- [BerriAI/litellm#49938](https://github.com/BerriAI/litellm/issues/49938): [Bug]: Sync Router.responses() ignores the Router's num_retries and fallbacks, Router.aresponses() honours them (fixed)
- [BerriAI/litellm#49940](https://github.com/BerriAI/litellm/issues/49940): [Bug]: A stream that fails after it opens logs the raw provider exception class (e.g. AnthropicError) in error_information (open)

## TorchSpec

[github.com/lightseekorg/TorchSpec](https://github.com/lightseekorg/TorchSpec)

- [lightseekorg/TorchSpec#204](https://github.com/lightseekorg/TorchSpec/issues/204): SMG binding silently rounds integers outside the i64/u64 range in tool-call arguments to floats (fixed)
- [lightseekorg/TorchSpec#205](https://github.com/lightseekorg/TorchSpec/issues/205): SmgRenderer with minimax-m2 ends the assistant span inside the content when the content ends in "[e~" (fixed)
- [lightseekorg/TorchSpec#206](https://github.com/lightseekorg/TorchSpec/issues/206): compute_assistant_loss_mask(last_turn_only=True) masks an earlier assistant turn when the last one is empty (fixed)
- [lightseekorg/TorchSpec#207](https://github.com/lightseekorg/TorchSpec/issues/207): build_rotary_embedding crashes with TypeError on a YaRN rope_scaling that omits beta_fast/beta_slow (fixed)
- [lightseekorg/TorchSpec#208](https://github.com/lightseekorg/TorchSpec/issues/208): build_rotary_embedding ignores rope_theta for linear, dynamic and mrope RoPE scaling (drafts use base 10000) (fixed)
- [lightseekorg/TorchSpec#209](https://github.com/lightseekorg/TorchSpec/issues/209): generate_vocab_mapping crashes with ValueError when a dataset row has exactly one token (fixed)
- [lightseekorg/TorchSpec#210](https://github.com/lightseekorg/TorchSpec/issues/210): EAGLE3 -> EAGLE-4 attn_norm warm start fails with shape_mismatch when stacked q/k/v is narrower than hidden_size (fixed)
- [lightseekorg/TorchSpec#211](https://github.com/lightseekorg/TorchSpec/issues/211): KimiK25Parser renders a message with `content: null` as the text "None" and trains on it (fixed)
- [lightseekorg/TorchSpec#212](https://github.com/lightseekorg/TorchSpec/issues/212): KimiK25Parser writes dict tool-call arguments as a Python repr ({'q': 'a'}) instead of JSON (fixed)
- [lightseekorg/TorchSpec#213](https://github.com/lightseekorg/TorchSpec/issues/213): DFlashConfig / DSparkConfig crash with AttributeError when given a YaRN rope_scaling (transformers 5.x) (fixed)
- [lightseekorg/TorchSpec#214](https://github.com/lightseekorg/TorchSpec/issues/214): DFlashConfig / DSparkConfig raise KeyError on a Llama-3.1-style rope_theta + llama3 rope_scaling (transformers 5.x) (fixed)
- [lightseekorg/TorchSpec#215](https://github.com/lightseekorg/TorchSpec/issues/215): DFlash attention ignores rope_scaling: drafts train with unscaled RoPE while the config declares scaling (fixed)
- [lightseekorg/TorchSpec#216](https://github.com/lightseekorg/TorchSpec/issues/216): MiniMaxParser.format splits assistant content at "</think>" differently from the MiniMax-M2.5 chat template (fixed)
- [lightseekorg/TorchSpec#217](https://github.com/lightseekorg/TorchSpec/issues/217): K3Renderer.get_assistant_token_ids() does not match named assistant turns, so the dynamic loss mask drops them (fixed)
- [lightseekorg/TorchSpec#218](https://github.com/lightseekorg/TorchSpec/issues/218): AsyncPutManager raises an async put failure twice: once from wait_for_buffer, again from the next check_last_error (fixed)

## Apache DataFusion

[github.com/apache/datafusion](https://github.com/apache/datafusion)

- [apache/datafusion#26431](https://github.com/apache/datafusion/issues/26431): `ORDER BY ... LIMIT 0` panics in TopK (`assertion failed: k > 0`) when `eliminate_limit` does not run (fixed)
- [apache/datafusion#26572](https://github.com/apache/datafusion/issues/26572): Regression: `ORDER BY col, (scalar subquery) LIMIT` over a `RIGHT JOIN` fails in `SanityCheckPlan` (fixed)
- [apache/datafusion#26573](https://github.com/apache/datafusion/issues/26573): `= ANY (subquery)` in a `JOIN ... ON` over an aliased table fails: "No field named x.a" (open)
- [apache/datafusion#26617](https://github.com/apache/datafusion/issues/26617): Wrong result: `count(*) OVER ()` sees only the LIMIT rows when ORDER BY ... LIMIT sits on top of another window (target_partitions > 1) (fixed)
- [apache/datafusion#26618](https://github.com/apache/datafusion/issues/26618): Wrong results: `ORDER BY <double>, ... LIMIT n` drops `-0.0` rows when 0.0 arrives in an earlier batch (TopK dynamic filter) (open)
- [apache/datafusion#26619](https://github.com/apache/datafusion/issues/26619): `push_down_filter` substitutes the wrong expression when `col` and `CAST(col AS ...)` share a schema name (HAVING and correlated IN give type errors) (fixed)
- [apache/datafusion#26620](https://github.com/apache/datafusion/issues/26620): Panic "all window expressions shall have the same sort keys" when two window functions share `ORDER BY a + 1` (debug builds) (open)
- [apache/datafusion#26621](https://github.com/apache/datafusion/issues/26621): `GROUP BY CAST(a AS VARCHAR), a` with a common subexpression fails in `optimize_projections` ("No field named t.a", sometimes a panic) (open)
- [apache/datafusion#26622](https://github.com/apache/datafusion/issues/26622): `x > ALL (subquery)` in the SELECT list fails with an internal schema error unless it is aliased (open)
- [apache/datafusion#26623](https://github.com/apache/datafusion/issues/26623): SanityCheckPlan error with sort-merge join when a filter says `a + b = a` (fixed)
- [apache/datafusion#26624](https://github.com/apache/datafusion/issues/26624): SanityCheckPlan error with sort-merge join when a filter makes a sort or join key constant (constant lost in `EquivalenceGroup` projection) (fixed)
- [apache/datafusion#26625](https://github.com/apache/datafusion/issues/26625): SanityCheckPlan error for `UNION ALL ... ORDER BY` when each arm selects the same column twice (fixed)
- [apache/datafusion#26626](https://github.com/apache/datafusion/issues/26626): SanityCheckPlan error for two window functions with `target_partitions = 1` (`WindowAggExec` loses `can_repartition` when its children are replaced) (open)

## OxCaml

[github.com/oxcaml/oxcaml](https://github.com/oxcaml/oxcaml)

- [oxcaml/oxcaml#7668](https://github.com/oxcaml/oxcaml/issues/7668): `ocamlopt -Oclassic` crashes on a division by constant zero inside a `try` (fixed)
- [oxcaml/oxcaml#7683](https://github.com/oxcaml/oxcaml/issues/7683): `ocamlopt -Oclassic` crashes on a `float#` field read in an unreachable match arm (open)
- [oxcaml/oxcaml#7684](https://github.com/oxcaml/oxcaml/issues/7684): with `-flambda2-reaper -g -flambda2-expert-phantom-lets`, an unused variable in a closure breaks compilation (open)
- [oxcaml/oxcaml#7700](https://github.com/oxcaml/oxcaml/issues/7700): x86-64 binary emitter encodes 32-bit `bswap` of r8d-r15d without REX.B, so it swaps eax-edi instead (open)
- [oxcaml/oxcaml#7701](https://github.com/oxcaml/oxcaml/issues/7701): `-use-ssa`: SSA validator rejects `x <= 1` against `x < 2` for the same switch, compilation aborts with "CFG comparison MISMATCH" (open)
- [oxcaml/oxcaml#7702](https://github.com/oxcaml/oxcaml/issues/7702): `-use-ssa`: a call followed by an unused pure operation becomes a tail call only in the SSA pipeline, validator aborts with "CFG comparison MISMATCH" (open)
- [oxcaml/oxcaml#7704](https://github.com/oxcaml/oxcaml/issues/7704): `-Oclassic -flambda2-expert-inline-effects-in-cmm`: `let f x = let y = sin x in (y, y)` fails with "Duplicate order for bindings when flushing" (open)
- [oxcaml/oxcaml#7705](https://github.com/oxcaml/oxcaml/issues/7705): `-Oclassic -flambda2-expert-inline-effects-in-cmm`: a call used as the argument of another call inside a `try` crashes ocamlopt with `Invalid_argument("Array.iter2 ...")` (declined)
- [oxcaml/oxcaml#7879](https://github.com/oxcaml/oxcaml/issues/7879): `ocamlopt` fails with "Assertion failed" in amd64 `emit.ml` when linking a curried function whose first parameter is an unboxed tuple of 254 or more words (open)
- [oxcaml/oxcaml#7888](https://github.com/oxcaml/oxcaml/issues/7888): amd64: `caml_int128_add` / `caml_int128_sub` builtins compute a wrong high word when the low half of `x` is the high half of `y` (fixed)
- [oxcaml/oxcaml#7889](https://github.com/oxcaml/oxcaml/issues/7889): amd64: `Atomic.compare_and_set` on an `int Atomic.t` succeeds after an earlier failed CAS with the same expected value (open)
- [oxcaml/oxcaml#7918](https://github.com/oxcaml/oxcaml/issues/7918): Bytecode: `Effect.perform` segfaults `ocamlrun` when a memprof callback grows the fiber stack (open)
- [oxcaml/oxcaml#7919](https://github.com/oxcaml/oxcaml/issues/7919): amd64: `ocamlopt -fno-avx` fails with `Fatal error: exception Not_found` on a store to a float32 Bigarray (fixed)

## Dogwood

[github.com/dogwood-policy/dogwood](https://github.com/dogwood-policy/dogwood)

- [dogwood-policy/dogwood#21](https://github.com/dogwood-policy/dogwood/issues/21): `count`/`sum` rejected with "`for`-variable does not occur in the aggregation body" when the variable is used only inside a temporal macro call (open)
- [dogwood-policy/dogwood#22](https://github.com/dogwood-policy/dogwood/issues/22): Temporal validator types a bound variable by the last same-named binder in the condition, not the binder in scope (open)
- [dogwood-policy/dogwood#23](https://github.com/dogwood-policy/dogwood/issues/23): `dogwood validate` drops the input/output fields of an action whose context names a top-level common type from inside a namespace (open)
- [dogwood-policy/dogwood#24](https://github.com/dogwood-policy/dogwood/issues/24): `ParsedPolicy::expanded_source` renders `-(-3)` as `--3`, which re-parses to the literal `3` (open)
- [dogwood-policy/dogwood#25](https://github.com/dogwood-policy/dogwood/issues/25): `ParsedPolicy::expanded_source` renders `!(-3)` as `!-3`, which Dogwood's parser rejects (open)
- [dogwood-policy/dogwood#26](https://github.com/dogwood-policy/dogwood/issues/26): `dogwood explain` prints `context has input.user` and `context has "input.user"` as the same text, but the two conditions decide differently (open)
- [dogwood-policy/dogwood#27](https://github.com/dogwood-policy/dogwood/issues/27): `dogwood explain` does not parenthesize an `if` that is an operand of `&&`/`\|\|`, so two policies with different verdicts get identical prose (open)
- [dogwood-policy/dogwood#28](https://github.com/dogwood-policy/dogwood/issues/28): Parser accepts Cedar reserved words (is, then, else, in, like, has, true, false) as record keys and attribute names (declined)
- [dogwood-policy/dogwood#29](https://github.com/dogwood-policy/dogwood/issues/29): Nested positive-left `since` under the default pinned event schema takes time polynomial in the number of in-window events, with the degree growing with nesting depth (open)
- [dogwood-policy/dogwood#30](https://github.com/dogwood-policy/dogwood/issues/30): Nested `previous` under the default pinned event schema takes time polynomial in the number of in-window events, with the degree growing with nesting depth (open)
- [dogwood-policy/dogwood#31](https://github.com/dogwood-policy/dogwood/issues/31): Provider record field names that are not Cedar identifiers ("a b", "x-y", "if") are written unquoted into a synthesized schema, so lowering fails (open)
- [dogwood-policy/dogwood#32](https://github.com/dogwood-policy/dogwood/issues/32): A `def temporal` macro whose whole body is `tp(?t)` is rejected as "macro-in-macro is not supported" (open)
- [dogwood-policy/dogwood#33](https://github.com/dogwood-policy/dogwood/issues/33): A `decimal("…")` literal passed as an argument to a `def temporal` macro is rejected as "unknown macro `decimal`" (open)

## Racket

[github.com/racket/racket](https://github.com/racket/racket)

- [racket/racket#5688](https://github.com/racket/racket/issues/5688): schemify: inlined function loses `set!` of a `let`-bound copy of its argument (fixed)
- [racket/racket#5689](https://github.com/racket/racket/issues/5689): schemify: cross-module inlining loses `set!` of the inlined function's argument (fixed)
- [racket/racket#5690](https://github.com/racket/racket/issues/5690): Re-entering a continuation captured inside `fl+` mutates the flonum returned the first time (fixed)
- [racket/racket#5692](https://github.com/racket/racket/issues/5692): Compiled `even?`/`odd?` on a non-integer flonum reports `fleven?`/`flodd?` instead of `even?`/`odd?` (open)
- [racket/racket#5695](https://github.com/racket/racket/issues/5695): `=` raises "no exact representation" for a fixnum above 2^53 and an inexact complex with an infinite or NaN part (open)
- [racket/racket#5697](https://github.com/racket/racket/issues/5697): `get-output-bytes` of 4096+ bytes in a future: "internal-error: attempting to suspend a future in uninterruptible mode", then the process hangs (open)
- [racket/racket#5698](https://github.com/racket/racket/issues/5698): A nested future that captures or aborts to a prompt fails with "no corresponding prompt" when the outer future was already started by a worker (open)
- [racket/racket#5699](https://github.com/racket/racket/issues/5699): Racket CS: `vector-fill!` rejects a chaperoned or impersonated mutable vector (open)
- [racket/racket#5700](https://github.com/racket/racket/issues/5700): `regexp-match*` on a byte string with start-pos > 0: look-behind in later matches sees bytes before start-pos (open)
- [racket/racket#5702](https://github.com/racket/racket/issues/5702): `regexp-match-positions*` skips an empty match after a non-ASCII character when the pattern looks behind (open)
- [racket/racket#5703](https://github.com/racket/racket/issues/5703): `regexp-replace*` skips a non-empty match that starts right after an empty match (open)

## Ada URL parser

[github.com/ada-url/ada](https://github.com/ada-url/ada)

- [ada-url/ada#1278](https://github.com/ada-url/ada/issues/1278): `parse("C\|", file_base)` marks the URL as having an opaque path: host/path setters fail and it cannot be a base (fixed)
- [ada-url/ada#1279](https://github.com/ada-url/ada/issues/1279): `url_search_params` keeps ill-formed UTF-8 from percent-decoding instead of decoding it to U+FFFD (declined)
- [ada-url/ada#1280](https://github.com/ada-url/ada/issues/1280): URLPattern test/exec/match resolve a relative string input against an empty URL when no base is given (should not match) (fixed)
- [ada-url/ada#1281](https://github.com/ada-url/ada/issues/1281): URLPattern: a literal backslash in a special-scheme pathname pattern is not canonicalized to "/" (spec's dummy URL is special) (fixed)
- [ada-url/ada#1282](https://github.com/ada-url/ada/issues/1282): `set_host("host:port")` under `set_max_input_length` applies the host, drops the port, and reports success (fixed)
- [ada-url/ada#1283](https://github.com/ada-url/ada/issues/1283): `canonicalize_protocol` indexes an empty string_view on a protocol pattern of an escaped colon (aborts in hardened builds) (fixed)
- [ada-url/ada#1284](https://github.com/ada-url/ada/issues/1284): URLPattern opaque pathname: a leading `//` is parsed as an authority and dropped (`//x/y` becomes `/y`) (fixed)
- [ada-url/ada#1285](https://github.com/ada-url/ada/issues/1285): URLPattern search canonicalization uses the non-special query encode set: a search pattern containing `'` never matches a special URL (fixed)
- [ada-url/ada#1326](https://github.com/ada-url/ada/issues/1326): `url_aggregator::set_host`/`set_hostname` exceed `set_max_input_length` with IDNA-expanding hosts (regression from #1262) (fixed)
- [ada-url/ada#1327](https://github.com/ada-url/ada/issues/1327): URLPattern `generate_pattern_string` drops the braces around a part whose prefix starts with `/` but is longer, so the pathname string no longer compiles to the same pattern (fixed)
- [ada-url/idna#97](https://github.com/ada-url/idna/issues/97): combining-mark, bidi and normalization tables predate the Unicode 17 mapping table: leading marks and RTL letters from Unicode 14+ pass, Unicode 16 compositions are not applied (fixed)

## Elixir

[github.com/elixir-lang/elixir](https://github.com/elixir-lang/elixir)

- [elixir-lang/elixir#16101](https://github.com/elixir-lang/elixir/issues/16101): False "clause will never match" for a list pattern whose tail is a cons pattern with variables (`[y \| [_]]`) (fixed)
- [elixir-lang/elixir#16102](https://github.com/elixir-lang/elixir/issues/16102): Type checker crashes (MatchError in `Of.refine_body_var/5`) when a `for` generator uses a variable bound in `into:` or `reduce:` (fixed)
- [elixir-lang/elixir#16105](https://github.com/elixir-lang/elixir/issues/16105): `Map.from_keys/2` with mixed keys drops the atom keys from the inferred type (fixed)
- [elixir-lang/elixir#16106](https://github.com/elixir-lang/elixir/issues/16106): False "incompatible types in binary construction" for a `::bits` pattern variable used as `::binary` (fixed)
- [elixir-lang/elixir#16107](https://github.com/elixir-lang/elixir/issues/16107): A clause head that cannot match is subtracted from the next clause's domain (fixed)
- [elixir-lang/elixir#16108](https://github.com/elixir-lang/elixir/issues/16108): False "pattern will never match" on a def head when a nested fn guard refines a head variable (fixed)
- [elixir-lang/elixir#16109](https://github.com/elixir-lang/elixir/issues/16109): Guard on a `<<x::bitstring>>` clause is ignored when typing the next clause (fixed)
- [elixir-lang/elixir#16110](https://github.com/elixir-lang/elixir/issues/16110): A call argument's expected type refines the variable for later arguments of the same call (fixed)
- [elixir-lang/elixir#16111](https://github.com/elixir-lang/elixir/issues/16111): `min/2` with a dynamic argument returns a non-dynamic type (fixed)

## OCaml

[github.com/ocaml/ocaml](https://github.com/ocaml/ocaml)

- [ocaml/ocaml#15185](https://github.com/ocaml/ocaml/issues/15185): non-exhaustive integer match crashes instead of raising Match_failure (fixed)
- [ocaml/ocaml#15189](https://github.com/ocaml/ocaml/issues/15189): `ocamlopt -linscan` miscompiles `Atomic.compare_and_set` on amd64 (open)
- [ocaml/ocaml#15205](https://github.com/ocaml/ocaml/issues/15205): amd64 `Atomic.exchange` stores a wrong value (5.6 & trunk regression) (open)
- [ocaml/ocaml#15212](https://github.com/ocaml/ocaml/issues/15212): flambda `ocamlopt` stops with "Bad definition for float array member" on a GADT match with a dead float-array branch (fixed)
- [ocaml/ocaml#15214](https://github.com/ocaml/ocaml/issues/15214): toplevel "Fatal error: n unbound at toplevel" on a redundant match case whose or-pattern binds a variable (open)

## bbtree

[github.com/XiangpengHao/bbtree](https://github.com/XiangpengHao/bbtree)

- [XiangpengHao/bbtree#23](https://github.com/XiangpengHao/bbtree/issues/23): WalWriter::open reports Corrupted on a healthy WAL when another writer fills the last file during the scan (open)
- [XiangpengHao/bbtree#25](https://github.com/XiangpengHao/bbtree/issues/25): BBTreeMap::get_or_insert: a panicking closure during a leaf split aborts the process (debug) or leaks the reserved pages (release); closed by documenting that user code must not panic (declined)
- [XiangpengHao/bbtree#29](https://github.com/XiangpengHao/bbtree/issues/29): BBTreeMap: dropping the map while a key's or value's Drop panics leaks every page on its free lists; closed by documenting that user code must not panic (declined)
- [XiangpengHao/bbtree#31](https://github.com/XiangpengHao/bbtree/issues/31): BBTreeMapFile::create writes the writer's heap addresses into the file's root page, so the same entries give different files (open)

## Envoy

[github.com/envoyproxy/envoy](https://github.com/envoyproxy/envoy)

- [envoyproxy/envoy#48807](https://github.com/envoyproxy/envoy/issues/48807): oghttp2 accepts a pseudo-header after a regular header (RFC 9113 8.3), in requests and responses (open)
- [envoyproxy/envoy#48808](https://github.com/envoyproxy/envoy/issues/48808): UHV builds: oghttp2 accepts duplicate pseudo-headers and joins them (:path becomes /public,/admin) (open)

## FoundationDB

[github.com/apple/foundationdb](https://github.com/apple/foundationdb)

- [apple/foundationdb#14477](https://github.com/apple/foundationdb/issues/14477): Read-your-writes: `get_range` with a key selector counts a key that `compare_and_clear` left absent (fixed)
- [apple/foundationdb#14478](https://github.com/apple/foundationdb/issues/14478): `read_your_writes_disable`: spurious conflicts with system-key writes after reading past the last key (open)

## Cedar

[github.com/cedar-policy/cedar](https://github.com/cedar-policy/cedar)

- [cedar-policy/cedar#2606](https://github.com/cedar-policy/cedar/issues/2606): `upsert_entities` keeps a stale ancestor when the batch contains the same entity twice (regression in 4.11.0) (fixed)
- [cedar-policy/cedar#2629](https://github.com/cedar-policy/cedar/issues/2629): `add_entities` compares a new entity's parents with the stored entity's transitive closure: a declared parent can be dropped, and identical entities are refused (open)

## QED prover

[github.com/qed-solver/prover](https://github.com/qed-solver/prover)

- [qed-solver/prover#13](https://github.com/qed-solver/prover/issues/13): Unsound: real constants are parsed as `f32`, so distinct literals are proved equal (and wide ones panic) (open)
- [qed-solver/prover#14](https://github.com/qed-solver/prover/issues/14): Panic on INTEGER vs DECIMAL in `IN (subquery)` (normal.rs:440) and `UNION` (partial.rs:185) (open)

## TigerBeetle

[github.com/tigerbeetle/tigerbeetle](https://github.com/tigerbeetle/tigerbeetle)

- [tigerbeetle/tigerbeetle#3960](https://github.com/tigerbeetle/tigerbeetle/issues/3960): State machine bug when two transfers share the same expiry nanosecond (open)

_Generated by `public-export/trophies.py` on 2026-11-27._
