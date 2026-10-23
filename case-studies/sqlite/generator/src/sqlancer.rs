// Unless explicitly stated otherwise all files in this repository are licensed under the MIT License.
// This product includes software developed at Datadog (https://www.datadoghq.com/) Copyright 2026-Present Datadog, Inc.

//! SQLancer-style metamorphic oracles: SQLite compared with itself on
//! queries ([`crate::expr::Query`]) over the state a [`Case`] leaves. Test
//! support for `tests/sqlancer.rs` and `examples/sqlancer.rs` (which run
//! exactly these functions); the generators never call it.
//!
//! Each oracle opens the case's database (writable in memory unless the
//! thread's storage says otherwise, see Storage below), runs its
//! statements (as `tests/cases.rs` does), then runs its queries. Each
//! returns what it compared ([`Tally`], with comparisons per variant) or a
//! description of the disagreement. Below, `F` is the query's FROM clause
//! as written (`t0 AS a0 [LEFT|CROSS] JOIN t1 AS a1 [ON e] ..`), `p` its
//! WHERE clause (`1` without one), `K` its GROUP BY terms, `h` its HAVING
//! clause and `I` its items.
//!
//! - [`norec`], Non-optimizing Reference Engine Construction (Rigger and
//!   Su, ESEC/FSE 2020), in the form SQLancer uses for SQLite: the
//!   optimized `SELECT count(*) FROM F WHERE p` against the unoptimized
//!   `SELECT sum(ref0) FROM (SELECT (p) IS TRUE AS ref0 FROM F)`, whose
//!   predicate is a value computed for every row of the join, not a filter
//!   the planner can use (a NULL sum, over no rows, is 0). Joins keep
//!   their ON clauses on both sides: WHERE filters the joined rows, LEFT
//!   JOIN's NULL rows included, so the counts agree for every join kind.
//!   Variants `norec`, `norec-join` (with a join).
//! - [`tlp`], Ternary Logic Partitioning (Rigger and Su, OOPSLA 2020): a
//!   query `Q` against the union of three partitions of it on a predicate
//!   `x`, each row (or group) falling in exactly one: `x`, `NOT x`, `x IS
//!   NULL`. By the query's shape:
//!   - `tlp-where`: `SELECT I FROM F` against `SELECT I FROM F WHERE (p)`,
//!     `.. WHERE NOT (p)`, `.. WHERE (p) IS NULL`, rows as a multiset
//!     (their concatenation). With joins the partitions filter the joined
//!     rows, as the original's WHERE would; ON clauses are never
//!     partitioned (for a LEFT JOIN, a row that fails ON is still a row,
//!     NULL-extended, so ON is not a filter).
//!   - `tlp-distinct`: the same with `SELECT DISTINCT`, as sets (the union
//!     of the partitions, then distinct).
//!   - `tlp-agg`: one aggregate, `SELECT agg(e) FROM F` against the
//!     partitions' aggregates combined in SQL, as SQLancer does: `SELECT
//!     sum(ref0) FROM (SELECT count(..) AS ref0 FROM F WHERE (p) UNION
//!     ALL ..)`; `sum` for `count` and `sum`, `min` for `min`, `max` for
//!     `max`.
//!   - `tlp-group`: GROUP BY with no aggregate and no HAVING: `SELECT I FROM
//!     F GROUP BY K` against the partitions on `p`, each with `GROUP BY K`,
//!     as sets (a group split over partitions shows in each).
//!   - `tlp-having`: GROUP BY with HAVING: `SELECT I FROM F WHERE p GROUP
//!     BY K` (no HAVING) against `.. GROUP BY K HAVING (h)`, `.. HAVING NOT
//!     (h)`, `.. HAVING (h) IS NULL`, rows as a multiset (a set with
//!     DISTINCT): HAVING keeps or drops whole groups.
//!   - `tlp-outer`: an aggregate query with neither (its WHERE would
//!     split groups and change their aggregates), partitioned on its
//!     result rows as `tlp-window` partitions a window query (below,
//!     [`tlp_window`]): `Q` against `WITH tw(r0, ..) AS (Q) SELECT * FROM
//!     tw WHERE (x)`, .., which SQLite may push down into `Q`'s HAVING
//!     (and on a GROUP BY term on into its WHERE, `havingToWhere`); `x` is
//!     over a GROUP BY term half of the time when an item is one, else
//!     half of the time over a GROUP BY term added as an item of its own.
//!     With `tlp-outer-union`, the same partitions as one compound.
//!   - `tlp-window`: a SELECT with window items, whatever its shape: the
//!     SELECT `Q` as written (WHERE, GROUP BY, HAVING; no ORDER BY, LIMIT)
//!     against `WITH tw(r0, ..) AS (Q) SELECT * FROM tw WHERE (x)`, `..
//!     WHERE NOT (x)`, `.. WHERE (x) IS NULL`, `x` over a result column
//!     (see [`tlp_window`]): the windows are computed before the
//!     partition, which SQLite may push down into `Q` only on a PARTITION
//!     BY term of every window (`x` is over such a term often, an item
//!     of its own when none is one, see [`tlp_window`]). Rows as a
//!     multiset (a set with DISTINCT).
//!     With `tlp-window-union`, the same partitions as one compound.
//!   - `tlp-union`, for every variant but `tlp-agg`: its partitions as one
//!     compound, joined by UNION ALL (by UNION when compared as sets).
//!   - `tlp-intersect`, for `tlp-where` and `tlp-distinct`, as sets:
//!     `SELECT I FROM F WHERE (p) INTERSECT SELECT I FROM F` against
//!     `SELECT DISTINCT I FROM F WHERE (p)`.
//!   - `tlp-except`, for the same, as sets: `SELECT I FROM F WHERE NOT (p)
//!     UNION SELECT I FROM F WHERE (p) IS NULL EXCEPT SELECT I FROM F
//!     WHERE (p)` against `SELECT I FROM F EXCEPT SELECT I FROM F WHERE
//!     (p)`.
//!
//!   On a compound query, NoREC and TLP check each of its SELECTs.
//! - [`order`], ORDER BY and LIMIT, for queries with a total ORDER BY:
//!   `order-multiset`, `order-limit`, `order-sorted` (see [`order`]); a
//!   query without ORDER BY the same under one by every result column
//!   ([`added_order`]): `order-all-multiset`, `order-all-limit`,
//!   `order-all-sorted`.
//! - [`windows`], window functions, for queries with window items:
//!   `window-def`, the windows' values against their definition computed
//!   from the rows they read (see [`window_values`]). TLP partitions such
//!   queries on their result rows (`tlp-window`: a partition of the WHERE
//!   clause would change the windows' partitions); NoREC, the plan
//!   variants and ORDER BY apply as to any query. Any other query (and a
//!   window query in a UTF-16 database or compared folded) is wrapped,
//!   each of its SELECTs in a CTE read by `count(*)` windows over its
//!   rows: `window-wrap`, `window-count` (see [`window_wrap`]).
//! - [`ctes`], common table expressions, for queries with a WITH clause:
//!   `cte-inline`, every CTE read as a subquery in FROM; `cte-materialized`
//!   and `cte-not-materialized`, every CTE with that hint. Every other
//!   oracle runs a query's statements under its WITH clause.
//! - [`setops`], set operations, for compound queries: `setops-def`, the
//!   compound against its definition computed from its SELECTs' rows;
//!   `setops-swap`, the operands of a commutative operator swapped (see
//!   [`setops`]); for any other query, `setops-self`, its SELECT as both
//!   members of a compound against its rows (see [`setops_self`]).
//! - [`plans`], differential query plans: the query as written against
//!   variants a correct planner must answer alike, compared as the query's
//!   results are (in order with ORDER BY, which every variant keeps):
//!   - `plans-noindex`: `NOT INDEXED` on every source, its subqueries'
//!     included;
//!   - `plans-index`: `INDEXED BY i` on one source, for each index `i` of
//!     its table (the others unhinted); `plans-index-partial` for a partial
//!     index and `plans-index-expr` for an expression index, skipped when
//!     SQLite finds no plan ("no query solution": the query's WHERE clause
//!     does not imply the partial index's, or the expression index is on
//!     the right of a RIGHT or FULL JOIN, where SQLite never scans one),
//!     counted in [`Tally::hint_skips`];
//!   - with joins all inner (with ON) or cross (inner and cross joins are
//!     commutative and associative, ON and WHERE terms interchangeable):
//!     `plans-cross`, every join a `CROSS JOIN` with its ON clause (SQLite
//!     keeps the order of a CROSS JOIN's operands as written);
//!     `plans-comma`, `FROM t0 AS a0, t1 AS a1 .. WHERE (on1) AND .. AND
//!     (p)`; `plans-reorder`, the sources in reverse order as CROSS JOINs
//!     with the ON clauses in the WHERE, `*` written as `a0.*, a1.*, ..` to
//!     keep the column order;
//!   - `plans-right`: a two-source LEFT JOIN `t0 AS a0 LEFT JOIN t1 AS a1
//!     ON e` as `t1 AS a1 RIGHT JOIN t0 AS a0 ON e`, `*` as `a0.*, a1.*`;
//!     `plans-left`, a two-source RIGHT JOIN the other way round;
//!     `plans-full-swap`, a two-source FULL JOIN with its operands swapped;
//!   - `plans-full-union`: a plain two-source FULL JOIN as a LEFT JOIN UNION
//!     ALL the right source's rows that match nothing (see [`full_union`]),
//!     against the query without ORDER BY;
//!   - `plans-on`: a USING or NATURAL join as ON equalities of its columns,
//!     `*` written out (a USING column once: the left source's for an
//!     inner or LEFT join, the right's for a RIGHT join, the two coalesced
//!     for a FULL join, as SQLite resolves its name).
//!   - `plans-noopt`: the query as written, on the same connection with
//!     every optimization of SQLite's off (`SQLITE_TESTCTRL_OPTIMIZATIONS`
//!     with mask `0xffffffff`, [`Optimizations`]; the query prepared under
//!     the mask, which is reset to 0 at once; Coroutines stays on under
//!     `PRAGMA reverse_unordered_selects`, a known SQLite bug, see
//!     [`noopt_mask`]). On a disagreement each of
//!     the 31 optimizations ([`OPTIMIZATIONS`]) is turned off alone: one
//!     that alone reproduces it names the variant, `plans-noopt-<name>`
//!     (`plans-noopt-BloomFilter`), else it stays `plans-noopt`; the
//!     message's last line is the mask of the run it shows (see [`noopt`]).
//!     The other variants rewrite the SQL; this one keeps it and changes
//!     the plan, as SQLite's own test programs do.
//!
//!   RIGHT and FULL joins, USING and NATURAL take part in every other
//!   oracle as any join does: TLP partitions the WHERE of the joined rows,
//!   NoREC counts them.
//! - [`storage`], storage: the query as written on the case's database in
//!   memory (the original) against the same query on it in a WAL file (the
//!   variant), compared as `plans` compares (in order with a total ORDER
//!   BY, else as the query's results are; ambiguous groups skipped).
//!   Variant `storage`.
//!
//! Storage, per thread like the deadline ([`set_storage`], [`Storage`]).
//! By default ([`Storage::Memory`]) a case's database is its file
//! deserialized into memory, a WAL header presented as rollback mode: the
//! pager's file paths, the WAL (`wal.c`) and the unix VFS (`os_unix.c`)
//! never run. Under [`Storage::Wal`] every [`open`] writes the case's bytes
//! as they are to a fresh file in the thread's directory (best a tmpfs),
//! opens it with the unix VFS in WAL mode (`PRAGMA journal_mode=WAL` unless
//! its header says WAL), with a small page cache and a small autocheckpoint
//! (so that pages spill to the WAL and commits checkpoint) and, for about
//! half the cases, closes it after the statements and opens it again (its
//! WAL kept, so that the new connection recovers the WAL index from it):
//! each choice from a hash of the case alone ([`wal_settings`]), so a replay
//! is the same. The next open on the thread, or [`clear_storage`], deletes
//! the files of the last (the database, `-wal`, `-shm`, `-journal`). Every
//! oracle runs unchanged on such a database; [`storage`] compares the two.
//!
//! How results compare: rows as a multiset, values exactly (reals to the
//! bit); with DISTINCT as a set of values as SQLite compares them; a
//! compound as below; with ORDER BY in order (below); one
//! aggregate as SQLite compares values (`count`, `min`, `max`) or exactly
//! (`sum`, see below); with GROUP BY as a multiset, values exactly but
//! `-0.0` and `0.0` taken as one (see below).
//!
//! Errors: the original (first) query of a comparison must not fail
//! except as below, or the generator wrote an invalid query; when it
//! fails, every variant must fail (any message); when it succeeds, every
//! variant must succeed.
//!
//! Exclusions, each narrow:
//!
//! - "integer overflow" errors. The only run-time error the expression
//!   language can raise is `abs()` of -2^63 (a JSON function's arguments
//!   are well formed by their form, `crate::expr::json_args_ok`, so it
//!   raises no "malformed JSON" or "bad JSON path" on any row); and
//!   `sum()` raises it when any partial sum overflows ("Sum() will throw
//!   an 'integer overflow' exception if all inputs are integers or NULL
//!   and an integer overflow occurs at any point during the
//!   computation", lang_aggfunc.html), so
//!   it depends on the order rows are summed in, and on how partial sums
//!   are grouped. Which rows and which terms of a WHERE clause SQLite
//!   evaluates depends on the plan (it reorders AND terms, short-circuits
//!   them, and skips rows an index excludes); and a TLP partition
//!   evaluates the predicate, which the original does not. So when exactly
//!   one side fails with "integer overflow", the comparison is skipped
//!   (counted in [`Tally::overflow_skips`]). Any other error mismatch is a
//!   failure.
//! - SQLite's size and depth limits ([`limit`]: subqueries nested too
//!   deep, an expression tree too large, too many terms in a compound,
//!   ...). A variant is a larger query than its original (a TLP union
//!   nests the original's subqueries one level deeper), so it can pass a
//!   limit the original stays under; when exactly one side fails so, the
//!   comparison is skipped. An original that passes a limit itself (a
//!   mutant stacked deep enough, a LIKE pattern computed longer than the
//!   pattern length limit) is skipped with its query. Both counted in
//!   [`Tally::limit_skips`].
//! - The host's storage failing ([`io_error`]: "database or disk is full",
//!   "disk I/O error"): a WAL file on a full disk is not SQLite's logic.
//!   [`storage`] skips a comparison where either side fails so, and an
//!   input whose WAL file does not open so (as every oracle does under
//!   [`Storage::Wal`]); counted in [`Tally::io_skips`]. Under
//!   [`Storage::Memory`] such errors are failures, as before.
//! - JSON functions refusing an argument value ([`json_error`]: "malformed
//!   JSON", "JSON cannot hold BLOB values", "bad JSON path", "json_object()
//!   labels must be TEXT"). Queries draw JSON arguments well formed by
//!   their form (`expr::JsonArg`), but a mutant or a value that reaches a
//!   JSON argument through a column, a cast or an aggregate can still be
//!   refused, and whether a row's argument is evaluated at all depends on
//!   the plan as for "integer overflow". An original refused so is skipped
//!   with its query, a comparison where exactly one side is refused so is
//!   skipped, and so are the setops and window oracles' own queries; all
//!   counted in [`Tally::json_skips`], which the generator keeps rare.
//! - A case SQLite refuses for a date and time function reading the
//!   current time where it must be deterministic ([`nondeterministic`]:
//!   "non-deterministic use of julianday() in an index", or in a CHECK
//!   constraint or a generated column): a statement of the case writes, or
//!   indexes, a value `date()`, `julianday()`, .. read as `now`
//!   (`crate::model::reads_as_now`: `now`, `subsec` or `subsecond` in any
//!   case, a blob of those bytes) under an expression index of one, which
//!   SQLite raises on purpose (deterministic.html). The generators write
//!   no such value (`crate::model::not_now`), but a case from before that
//!   rule, or a mutant of one, may hold one. Its input is skipped
//!   (counted in [`Tally::nondeterministic_skips`]); only this message,
//!   with a date and time function and one of those three places, is.
//! - Floating-point sums. A top-level `sum()` whose result is a REAL on
//!   both sides is not compared: summation order changes the rounding
//!   (SQLite sums reals with Kahan-Babushka-Neumaier compensation, which
//!   is still not order-independent, and overflow to infinity depends on
//!   order). Integer sums are compared exactly; a REAL against an INTEGER
//!   is a failure. A sum of reals that reaches NaN (`+inf` plus `-inf`) is
//!   NULL, and whether it does depends on how the partial sums group the
//!   infinities: a NULL sum against a non-NULL one is skipped when the
//!   NULL side had a non-NULL input (`count(e)` over its rows is not 0),
//!   so its NULL is a NaN. Counted in [`Tally::real_sum_skips`]. Nested
//!   aggregates (in GROUP BY queries and scalar subqueries) avoid both by
//!   construction: `sum` only of a `CAST(.. AS INTEGER)`
//!   (`crate::expr::Query::valid_in`).
//! - Equal values that are not identical. Where SQLite picks one of
//!   several equal values (`DISTINCT` keeps the first of rows that compare
//!   equal; `min()` and `max()` return the first of equal extremes), the
//!   one it picks depends on row order: `1`, `1.0` and `-0.0`... compare
//!   equal as numbers. Those results are compared as SQLite compares
//!   values (an integer and a real are equal when they are the same
//!   number; text and blobs by their bytes); every other result exactly,
//!   to the bit. Nested `min()` and `max()` take a `CAST` to INTEGER or
//!   TEXT, which maps equal values to identical ones.
//! - GROUP BY puts equal values in one group, and SQLite evaluates a
//!   GROUP BY term in the items or HAVING "against a single arbitrarily
//!   chosen row from within the group" (lang_select.html), so a group
//!   holding `1` and `1.0` shows either. A GROUP BY query is compared only
//!   when no group's terms hold values of different types or texts
//!   ([`ambiguous_groups`]; else counted in [`Tally::group_skips`]); and
//!   since `-0.0` and `0.0` have the same type and text, a real zero in a
//!   GROUP BY query's rows is compared without its sign. Columns outside
//!   GROUP BY terms and aggregates are never generated in such queries.
//! - Known SQLite behaviour, skipped per query ([`known_issue`], counted
//!   in [`Tally::known_skips`]), and one query in three rewritten around
//!   it ([`as_checked`]; all of them cost the runner's throughput past its
//!   budget):
//!   - a LIKE or GLOB whose pattern is a text literal with a non-ASCII
//!     character before its first wildcard, in a query reading a table
//!     that holds a value that is not valid text in the database's
//!     encoding ([`invalid_text_pattern`]). SQLite's decoder accepts
//!     over-length UTF-8 (`x'fba9'` reads as `é`), so a scan matches such a
//!     value, but the LIKE optimization's range scan compares bytes and
//!     never visits it: "SQLite follows a policy of Garbage-In,
//!     Garbage-Out", and invalid UTF includes "using more bytes of UTF-8
//!     than necessary" (invalidutf.html; case-studies/sqlite/bugs/sqlite-glob-index-blob).
//!     One such query in three is checked with each such pattern `'p'` written `('p'
//!     || '')` ([`patterns_computed`], counted in [`Tally::known_rewrites`]):
//!     the same pattern, but the LIKE optimization takes only a literal,
//!     so every plan matches each row alike.
//!
//!   The last two before it, a
//!   flattened compound CTE with a RIGHT join losing WHERE terms
//!   (case-studies/sqlite/bugs/sqlite-flatten-compound-right-join) and a window query's
//!   correlated outer join ON a bare outer column filtering its left rows
//!   (case-studies/sqlite/bugs/sqlite-window-left-join-outer-on), were fixed on trunk by
//!   check-ins cfa69ea48a and c50d7f682c.
//!
//!   Rewritten around known SQLite behaviour, counted in
//!   [`Tally::known_rewrites`]: in a UTF-16 database, a `concat()` or
//!   `concat_ws()` argument that may be a BLOB ([`concat_blob_utf16`]) is
//!   checked as `CAST(x AS TEXT)` ([`concat_as_text`], the same value):
//!   SQLite copies as many bytes of a BLOB argument's UTF-8 text as the
//!   BLOB has, a NUL and stale bytes or a truncated character, and only on
//!   the first row for a constant (case-studies/sqlite/bugs/sqlite-concat-blob-utf16-length;
//!   valid UTF-16 too, not garbage in, garbage out).
//! - Collating sequences. Where a table the query reads (in its sources,
//!   its subqueries', its compound members' or its CTEs') has a column
//!   under NOCASE or RTRIM, or the query puts an expression under one with
//!   `COLLATE` ([`folds`], decided per query: no other comparison in a
//!   query is under either, as an index's own `COLLATE` serves only
//!   comparisons under its sequence), texts that differ only in the case
//!   of ASCII letters or in trailing spaces may compare equal, and SQLite
//!   picks one of them by the plan where it keeps one of equal values
//!   (DISTINCT, GROUP BY, a compound, `min()` and `max()`, an ordered
//!   aggregate's ties). Every comparison of such a query is made on text
//!   folded as both sequences would compare it ([`Cell::folded`]: ASCII
//!   capitals in lower case, trailing spaces gone, nothing past a NUL),
//!   and the checks computed in Rust under BINARY take each column's or
//!   term's sequence instead: `order-sorted` checks the order of each term
//!   under its own (not on a compound or `*` over a join, which it skips),
//!   `setops-def` tells rows apart and sorts them under the sequence each
//!   column of the compound takes (from its first member with one,
//!   `multiSelectCollSeq`: a column's, a CTE's, a view's or a subquery's
//!   included, BINARY where none is declared; skipped where the model does
//!   not tell, `*` over a join or a subquery item, and where a group of
//!   the members that SQLite computes as a compound of its own tells rows
//!   apart under other sequences, counted in [`Tally::coll_skips`]; for
//!   that reason `order-multiset` skips such a compound too, where it
//!   differs with ORDER BY or without), and `window-def` skips it; so does
//!   `setops-swap` where swapping the members changes the sequence a
//!   column of the compound takes. The queries never expose the pick
//!   otherwise: a `CAST(x AS TEXT)` keeps `x`'s sequence, so aggregates'
//!   arguments and tie terms never cast a collated value to TEXT
//!   (`crate::expr::Query::valid_in`), and [`ambiguous_groups`] tells a
//!   group's texts apart by their bytes.
//! - JSONB bytes. SQLite may encode one value as JSONB in several ways
//!   ("The header for an element does not need to be in its simplest
//!   form", jsonb.html; a string as TEXT or TEXTRAW, by its per-statement
//!   JSON parse cache, so by the order rows are visited), and
//!   "Applications should access JSONB only through the JSON SQL
//!   functions, not by looking at individual bytes". So a result BLOB that
//!   is valid JSONB (`json_valid(x, 8)`) is compared as its JSON text,
//!   `json(x)`, tagged apart from text ([`Cell::Jsonb`]; x'00' is not
//!   'null'); ordered by its bytes, as SQLite orders it. And no query
//!   shows the bytes otherwise: a `jsonb` function's value is only an
//!   argument of a JSON function (`crate::expr::jsonb_read`), never of
//!   `lower`, `length`, `||`, a comparison, a cast, an aggregate, a sort,
//!   a group, a set or a result column, which would see an encoding (the
//!   runner's tlp-where of `lower(jsonb_replace(..))`, run 20261012-0317).
//!   Such a result BLOB is then one of a column's.
//! - ORDER BY only at the top level and only a total order
//!   ([`Query::total_order`]), LIMIT only with one: LIMIT without a total
//!   order may return any of the rows that tie. Ordered results are
//!   compared in order: exactly when the order is by every source's unique
//!   key (rowid or primary key, so no two rows tie), else as SQLite
//!   compares values (rows that tie in every result column may differ in
//!   values that compare equal, `1` and `1.0`). No ORDER BY in subqueries.
//!   A scalar subquery is one aggregate without GROUP BY, so exactly one
//!   row: of several rows SQLite would take the first, which depends on
//!   the plan.
//! - [`order`] checks sortedness, and [`setops`] computes set operations,
//!   in Rust only in UTF-8 databases: text in a UTF-16 database compares
//!   by its UTF-16 bytes, and the rows read back as UTF-8, where distinct
//!   ill-formed UTF-16 texts may read back alike.
//! - `*` over a USING or NATURAL join that is left of a RIGHT or FULL join
//!   is never generated when a later source has a column of a USING
//!   column's name: SQLite rejects it as "ambiguous column name" (a bug,
//!   case-studies/sqlite/bugs/sqlite-right-join-using-star-ambiguous; see
//!   `crate::expr::Query::using_ok`).
//! - Window values are determined by construction (`crate::expr::Window`):
//!   `row_number`, `ntile`, the functions that read one row (`first_value`,
//!   `last_value`, `nth_value`, `lag`, `lead`), the concatenating
//!   aggregates (`group_concat`, `json_group_array`) and ROWS frames only
//!   with an ORDER BY that ends with every source's key (so no two rows are
//!   peers); `rank`, `dense_rank`, `percent_rank`, `cume_dist` depend on
//!   peers only; aggregates of the same arguments as nested aggregates (a
//!   CAST to INTEGER or TEXT for `min` and `max`, to INTEGER for `sum` and
//!   `json_group_array`), `avg` and `total` of `CAST(x AS INTEGER) % n`
//!   (n <= 1000: the integer sum is exact, so its real does not depend on
//!   the order rows are added in), so no value depends on the order of
//!   peers; RANGE frames with offsets only over one integer ORDER BY term
//!   (a rowid or a `CAST(.. AS INTEGER)`). [`windows`] computes them all
//!   (FILTER, EXCLUDE and named windows included) only in UTF-8 databases,
//!   and skips a RANGE frame with offsets over values beyond 2^62 in magnitude (near the
//!   ends of 64 bits, where SQLite's bound arithmetic rounds).
//! - A CTE's query is never DISTINCT, grouped or one [`Items::Agg`], and
//!   has a LIMIT only with an order by every source's key
//!   ([`crate::expr::Cte::info`]): its rows are then determined exactly,
//!   so a query reading it is compared as one reading tables is.
//! - Compounds: rows of a UNION, INTERSECT or EXCEPT are one of each set of
//!   rows equal as SQLite compares values, as with DISTINCT, so a compound
//!   is compared as a multiset of values as SQLite compares them, unless
//!   every operator is UNION ALL and every SELECT compares exactly. Neither
//!   a compound's first SELECT nor a member is one aggregate `sum()`, whose
//!   real sum depends on the order of rows.

use std::{
    borrow::Cow,
    cell::{Cell as Slot, RefCell},
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
    time::Instant,
};

use rusqlite::{types::ValueRef, Connection};

use crate::{
    cases::Case,
    expr::{
        alias, never_blob, table_name, Agg, BinOp, Bound, By, CastType, CteBody, Exclude, Expr, FrameMode, Func, Items, JoinKind, JoinOp, Limit, Order,
        OrderKey, Query, RowKey, Schema, SetOp, TableInfo, Vtab, WinFunc, Window, PRAGMAS,
    },
    model::{fold, int_real, Coll, Encoding, Model, Value},
    probes::final_model,
    statements::Statement,
};

/// A result value, exactly: reals by their bits, text by its bytes (a
/// cast of a blob to text need not be UTF-8).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Cell {
    Null,
    Int(i64),
    Real(u64),
    Text(Vec<u8>),
    Blob(Vec<u8>),
    /// A BLOB that is valid JSONB, compared as its JSON text (`json(x)`):
    /// SQLite encodes one value as JSONB in more than one way. Its bytes
    /// order it as SQLite does ([`cmp_cells`]).
    Jsonb(Vec<u8>, Raw),
}

/// A JSONB value's bytes, which no comparison of cells sees but
/// [`cmp_cells`]: equal, and in order, to every other.
#[derive(Clone, Debug)]
pub struct Raw(pub Vec<u8>);

impl PartialEq for Raw {
    fn eq(&self, _: &Raw) -> bool {
        true
    }
}

impl Eq for Raw {}

impl PartialOrd for Raw {
    fn partial_cmp(&self, other: &Raw) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Raw {
    fn cmp(&self, _: &Raw) -> Ordering {
        Ordering::Equal
    }
}

impl std::hash::Hash for Raw {
    fn hash<H: std::hash::Hasher>(&self, _: &mut H) {}
}

impl fmt::Debug for Cell {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Cell::Null => write!(f, "NULL"),
            Cell::Int(i) => write!(f, "{i}"),
            Cell::Real(b) => write!(f, "{:?}", f64::from_bits(*b)),
            Cell::Text(t) => write!(f, "{:?}", String::from_utf8_lossy(t)),
            Cell::Blob(b) => write!(f, "x'{}'", b.iter().map(|x| format!("{x:02x}")).collect::<String>()),
            Cell::Jsonb(t, _) => write!(f, "jsonb({:?})", String::from_utf8_lossy(t)),
        }
    }
}

impl Cell {
    /// The value as SQLite compares it: a real that is an integer in
    /// range is that integer (`-0.0` is 0), so equal numbers are equal.
    pub fn equiv(&self) -> Cell {
        if let Cell::Real(b) = self {
            let x = f64::from_bits(*b);
            if x.fract() == 0.0 && (-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&x) {
                return Cell::Int(x as i64);
            }
        }
        self.clone()
    }
}

impl Cell {
    /// The value as SQLite tells values apart in a set or a partition:
    /// [`Cell::equiv`], a JSONB value by its bytes.
    pub fn distinct_key(&self) -> Cell {
        match self {
            Cell::Jsonb(_, Raw(b)) => Cell::Blob(b.clone()),
            c => c.equiv(),
        }
    }
}

impl Cell {
    /// The value with text folded as both NOCASE and RTRIM would compare
    /// it ([`crate::model::fold`]: ASCII capitals in lower case, trailing
    /// spaces gone, nothing past a NUL); a JSONB value's JSON text
    /// likewise.
    pub fn folded(&self) -> Cell {
        match self {
            Cell::Text(t) => Cell::Text(fold(t).into_owned()),
            Cell::Jsonb(t, raw) => Cell::Jsonb(fold(t).into_owned(), raw.clone()),
            c => c.clone(),
        }
    }
}

impl Cell {
    /// The value with a real `-0.0` as `0.0`.
    pub fn unsigned_zero(&self) -> Cell {
        match self {
            Cell::Real(b) if f64::from_bits(*b) == 0.0 => Cell::Real(0.0f64.to_bits()),
            _ => self.clone(),
        }
    }
}

type Rows = Vec<Vec<Cell>>;
/// A query's rows, or its error message.
pub type Outcome = Result<Rows, String>;

fn cell(v: ValueRef) -> Cell {
    match v {
        ValueRef::Null => Cell::Null,
        ValueRef::Integer(i) => Cell::Int(i),
        ValueRef::Real(x) => Cell::Real(x.to_bits()),
        ValueRef::Text(t) => Cell::Text(t.to_vec()),
        ValueRef::Blob(b) => Cell::Blob(b.to_vec()),
    }
}

/// Run one query. A BLOB that is valid JSONB (`json_valid(x, 8)`) is
/// [`Cell::Jsonb`], its JSON text (see the module documentation).
pub fn run(db: &Connection, sql: &str) -> Outcome {
    jsonb_cells(db, query_rows(db, sql)?)
}

/// The rows of one query, its statement prepared afresh (never from
/// rusqlite's statement cache: SQLite reads the optimizations it may use,
/// [`Optimizations`], when it prepares a statement), its BLOBs as they are.
fn query_rows(db: &Connection, sql: &str) -> Outcome {
    let go = || -> rusqlite::Result<Rows> {
        let mut stmt = db.prepare(sql)?;
        let n = stmt.column_count();
        let rows = stmt.query_map([], |r| (0..n).map(|k| r.get_ref(k).map(cell)).collect::<Result<Vec<_>, _>>())?;
        rows.collect()
    };
    go().map_err(|e| e.to_string())
}

/// `rows` with every BLOB that is valid JSONB as [`Cell::Jsonb`] (see
/// [`run`]).
fn jsonb_cells(db: &Connection, mut rows: Rows) -> Outcome {
    if rows.iter().flatten().any(|c| matches!(c, Cell::Blob(_))) {
        let mut text = db.prepare_cached("SELECT json(?1) WHERE json_valid(?1, 8)").map_err(|e| e.to_string())?;
        for c in rows.iter_mut().flatten() {
            if let Cell::Blob(b) = c {
                if let Ok(Some(t)) = text.query_row([&b[..]], |r| r.get::<_, Option<String>>(0)) {
                    *c = Cell::Jsonb(t.into_bytes(), Raw(std::mem::take(b)));
                }
            }
        }
    }
    Ok(rows)
}

/// One optimization of SQLite's: its one-based bit in the mask of
/// `SQLITE_TESTCTRL_OPTIMIZATIONS`, its name, and the name of the
/// `plans-noopt` variant it alone explains.
macro_rules! optimizations {
    ($(($bit:literal, $name:ident)),* $(,)?) => {
        [$(($bit, stringify!($name), concat!("plans-noopt-", stringify!($name)))),*]
    };
}

/// SQLite's optimizations that `SQLITE_TESTCTRL_OPTIMIZATIONS` can turn
/// off, by bit and name as trunk's sqliteInt.h numbers them and its
/// ext/misc/optname.c names them (bit `n` is `1 << (n - 1)` in the mask;
/// bit 21 is unused; the aliases `Flatten` and `OrderBySubquery` left out).
/// Bit 4 (`JoinMiss`) is unused before trunk f69fd5f5.
pub const OPTIMIZATIONS: [(u32, &str, &str); 31] = optimizations![
    (1, QueryFlattener),
    (2, WindowFunc),
    (3, GroupByOrder),
    (4, JoinMiss),
    (5, DistinctOpt),
    (6, CoverIdxScan),
    (7, OrderByIdxJoin),
    (8, Transitive),
    (9, OmitNoopJoin),
    (10, CountOfView),
    (11, CursorHints),
    (12, Stat4),
    (13, PushDown),
    (14, SimplifyJoin),
    (15, SkipScan),
    (16, PropagateConst),
    (17, MinMaxOpt),
    (18, SeekScan),
    (19, OmitOrderBy),
    (20, BloomFilter),
    (22, BalancedMerge),
    (23, ReleaseReg),
    (24, FlttnUnionAll),
    (25, IndexedExpr),
    (26, Coroutines),
    (27, NullUnusedCols),
    (28, OnePass),
    (29, OrderBySubq),
    (30, StarQuery),
    (31, ExistsToJoin),
    (32, UnionLimit),
];

/// The mask that turns every optimization off (`SQLITE_AllOpts`).
pub const ALL_OPTIMIZATIONS: u32 = 0xffff_ffff;

/// SQLite's optimizations of `mask` turned off on a connection
/// (`sqlite3_test_control(SQLITE_TESTCTRL_OPTIMIZATIONS, db, mask)`, the
/// mask read as a u32), until this is dropped, which turns them all back on
/// (mask 0, the default), on every path out (an error, a panic). SQLite
/// reads the mask when it prepares a statement: one prepared before keeps
/// its plan, so a query run under the mask must be prepared under it (as
/// [`run`] prepares every query).
pub struct Optimizations<'a>(&'a Connection);

impl<'a> Optimizations<'a> {
    pub fn off(db: &'a Connection, mask: u32) -> Optimizations<'a> {
        set_optimizations(db, mask);
        Optimizations(db)
    }
}

impl Drop for Optimizations<'_> {
    fn drop(&mut self) {
        set_optimizations(self.0, 0);
    }
}

fn set_optimizations(db: &Connection, mask: u32) {
    // SAFETY: the handle is this open connection's; the test control reads
    // a connection and a u32 from its variadic arguments.
    unsafe {
        rusqlite::ffi::sqlite3_test_control(rusqlite::ffi::SQLITE_TESTCTRL_OPTIMIZATIONS, db.handle(), mask as std::ffi::c_uint);
    }
}

/// [`run`] with the optimizations of `mask` off: the query prepared and
/// run under the mask, which is reset before anything else (the JSONB
/// conversion's cached statement included) runs.
pub fn run_without(db: &Connection, sql: &str, mask: u32) -> Outcome {
    let rows = {
        let _off = Optimizations::off(db, mask);
        query_rows(db, sql)
    };
    jsonb_cells(db, rows?)
}

thread_local! {
    static DEADLINE: Slot<Option<Instant>> = const { Slot::new(None) };
    /// Whether the query being checked is compared folded ([`folds`], set
    /// by [`each_on`]): its comparisons fold text ([`Cell::folded`]).
    static FOLD: Slot<bool> = const { Slot::new(false) };
}

/// Whether `q` (its compound members and CTEs, its subqueries included)
/// puts an expression under NOCASE or RTRIM with `COLLATE`.
pub fn collates(q: &Query) -> bool {
    let mut found = false;
    each_expr(q, &mut |e| found |= matches!(e, Expr::Collate(_, k) if *k != Coll::Binary));
    found
}

/// Whether `q`, on the tables of `schema`, is compared folded and checked
/// under each column's or term's collating sequence where the checks are
/// computed in Rust (see the module documentation): a table it reads
/// ([`Query::tables`]: its sources, its subqueries', its compound members'
/// and its CTEs') has a column under NOCASE or RTRIM
/// ([`TableInfo::collated`]), or it puts an expression under one with
/// `COLLATE` ([`collates`]).
fn folds(schema: &Schema, q: &Query) -> bool {
    q.tables().iter().any(|&t| schema.info(t).is_some_and(TableInfo::collated)) || collates(q)
}

/// Whether the query being checked is compared folded ([`folds`]).
fn folding() -> bool {
    FOLD.with(|f| f.get())
}

/// From `deadline` on, SQLite work on the connections this thread opens
/// with [`open`] is interrupted (statements and queries then fail with
/// "interrupted"), until it is set back to `None`: so that a loop does not
/// stall on one slow input.
pub fn set_deadline(deadline: Option<Instant>) {
    DEADLINE.with(|d| d.set(deadline));
}

/// Whether this thread's deadline ([`set_deadline`]) has passed.
pub fn past_deadline() -> bool {
    DEADLINE.with(|d| d.get().is_some_and(|t| Instant::now() >= t))
}

/// Where [`open`] keeps a case's database, set per thread like the
/// deadline ([`set_storage`]; see the module documentation).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Storage {
    /// In memory, deserialized from the case's bytes, a WAL header
    /// presented as rollback mode.
    #[default]
    Memory,
    /// A fresh file in `dir` (best a tmpfs) in WAL mode, through the unix
    /// VFS, with the settings [`wal_settings`] derives from the case.
    Wal { dir: PathBuf },
}

thread_local! {
    static STORAGE: RefCell<Storage> = const { RefCell::new(Storage::Memory) };
    /// The database file of this thread's last WAL open, whose files the
    /// next open (or [`clear_storage`]) deletes.
    static LAST_FILE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Numbers WAL files apart, with the process id.
static SERIAL: AtomicU64 = AtomicU64::new(0);

/// The storage of the connections this thread opens with [`open`] from now
/// on (the oracles' and [`selectivity`]'s; [`storage`] opens both kinds).
pub fn set_storage(storage: Storage) {
    STORAGE.with(|s| *s.borrow_mut() = storage);
}

/// This thread's storage ([`set_storage`]).
pub fn thread_storage() -> Storage {
    STORAGE.with(|s| s.borrow().clone())
}

/// Delete the files of this thread's last WAL open (the database and its
/// `-wal`, `-shm` and `-journal`), as its next WAL open does: for a loop's
/// exit. Its connection should be closed first.
pub fn clear_storage() {
    if let Some(path) = LAST_FILE.with(|f| f.borrow_mut().take()) {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let mut p = path.clone().into_os_string();
            p.push(suffix);
            let _ = std::fs::remove_file(p);
        }
    }
}

/// How a WAL open sets its connection up, derived from the case alone (a
/// hash of its file's bytes and its statements' SQL), so that a replay of
/// the case is the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WalSettings {
    /// `PRAGMA cache_size`, in pages: small, so that a transaction's dirty
    /// pages spill to the WAL before it commits (unless the case sets its
    /// own).
    pub cache_size: u32,
    /// `PRAGMA wal_autocheckpoint`, in frames: small, so that commits
    /// checkpoint.
    pub autocheckpoint: u32,
    /// `PRAGMA synchronous`: `NORMAL` or `FULL` (the WAL synced at every
    /// commit).
    pub synchronous: &'static str,
    /// Whether the connection is closed after the statements, its WAL kept
    /// (`SQLITE_FCNTL_PERSIST_WAL`), and the file opened again, so that the
    /// new connection rebuilds the WAL index from the WAL
    /// (`walIndexRecover`); only when no transaction is open, and never for
    /// a case that attaches `aux` or makes a TEMP table, trigger or
    /// statistics ([`in_connection`]): those live in the connection, not in
    /// the file, and a new connection has none:
    /// three quarters of the other cases, about half of all as before.
    pub reopen: bool,
}

/// Whether statement `s` makes something the connection keeps and a new
/// one would not have: an attached database, a TEMP table, a TEMP trigger
/// (or one of a TEMP table), TEMP statistics (`ANALYZE temp`); a query
/// reading the TEMP schema's table (`sqlite_temp_schema`) sees them.
fn in_connection(s: &Statement) -> bool {
    use crate::model::Db;
    match s {
        Statement::Attach(_) | Statement::AnalyzeDb(Db::Temp) => true,
        Statement::CreateTable { def, .. } => def.db != Db::Main,
        Statement::CreateRule(g) => g.temp || g.db != Db::Main,
        _ => false,
    }
}

/// The [`WalSettings`] of `case`.
pub fn wal_settings(case: &Case) -> WalSettings {
    // FNV-1a, then SplitMix64's finalizer to spread it over the bits.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    eat(&case.file.0);
    for s in &case.statements {
        eat(s.to_string().as_bytes());
        eat(b";");
    }
    h = (h ^ (h >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h = (h ^ (h >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^= h >> 31;
    WalSettings {
        cache_size: [4, 10, 32, 100][(h & 3) as usize],
        autocheckpoint: [1, 4, 16, 100][(h >> 2 & 3) as usize],
        synchronous: ["NORMAL", "FULL"][(h >> 4 & 1) as usize],
        reopen: h >> 5 & 3 != 0 && !case.statements.iter().any(in_connection),
    }
}

/// Whether `e` is the host's storage failing (the disk full, an I/O error,
/// a file that cannot be written), not SQLite's logic.
fn io_error(e: &str) -> bool {
    ["database or disk is full", "disk I/O error"].iter().any(|m| e.contains(m))
}

/// Whether `e`, a case's statement failing (`statement k: sql: e`), is
/// SQLite refusing a date and time function the current time where its
/// value must be deterministic: "non-deterministic use of X() in Y", X
/// one of `crate::expr::Func::DATE` and Y an index, a CHECK constraint or a
/// generated column (`sqlite3NotPureFunc`), at the end of the message.
pub fn nondeterministic(e: &str) -> bool {
    let Some((_, what)) = e.rsplit_once("non-deterministic use of ") else {
        return false;
    };
    crate::expr::Func::DATE.iter().any(|f| {
        ["an index", "a CHECK constraint", "a generated column"].iter().any(|y| what == format!("{}() in {y}", f.name()))
    })
}

/// The case's database, as this thread's storage says ([`set_storage`];
/// see [`open_with`]).
pub fn open(case: &Case) -> Result<Connection, String> {
    open_with(case, &thread_storage())
}

/// The case's database in `storage`, with its statements run; or the
/// statement that failed. Interrupted past this thread's deadline
/// ([`set_deadline`]).
///
/// [`Storage::Memory`]: the file opened writable in memory (a WAL header
/// presented as rollback mode, as the predicate does). [`Storage::Wal`]:
/// the files of this thread's last WAL open deleted, the case's bytes
/// written as they are to a fresh file in the directory (header and all),
/// opened with the unix VFS, put in WAL mode unless its header says so
/// (`PRAGMA journal_mode=WAL`; a file SQLite will not put in WAL mode stays
/// in its own), set up as [`wal_settings`] says, the statements run, and the
/// connection maybe closed and opened again ([`WalSettings::reopen`]). A
/// file that cannot be written fails as a "disk I/O error".
pub fn open_with(case: &Case, storage: &Storage) -> Result<Connection, String> {
    match storage {
        Storage::Memory => open_memory(case),
        Storage::Wal { dir } => open_wal(case, dir),
    }
}

/// Run statement `k` of a case, `s`, on `db`; or say how it failed. A
/// `PRAGMA wal_checkpoint` that fails with `SQLITE_LOCKED` ("database table
/// is locked") counts as run: a checkpoint changes no row, and SQLite
/// refuses it so on a connection whose own work it cannot rule out, not
/// only inside a write transaction. On a database whose WAL was not opened
/// yet (a file of no page put in WAL mode with no transaction since: the
/// case's `PRAGMA journal_mode = WAL` after its `= DELETE`, or the open's
/// own on an empty file), `sqlite3PagerCheckpoint` first runs
/// `PRAGMA table_list` from inside the checkpoint to open it, and that
/// nested statement leaves every database's b-tree in a read transaction
/// (`btreeEndTransaction` with another statement active); a TEMP database
/// the connection opened before (an integrity check of every schema, a
/// TEMP table) then refuses its own checkpoint, `sqlite3BtreeCheckpoint`'s
/// `SQLITE_LOCKED`. The statement's end rolls those transactions back, and
/// the WAL is open after it. A memory database is never in WAL mode, so
/// only a WAL file fails so: the storage oracle would fail a case that
/// opens in memory and not in the file. Found as run 20261018-1734 storage
/// w13-91071 and cte w13-54229 (`gen+wal`: `PRAGMA journal_mode = DELETE;
/// PRAGMA integrity_check; PRAGMA journal_mode = WAL; PRAGMA
/// wal_checkpoint(PASSIVE)` on an empty file), an SQLite quirk.
fn run_statement(db: &Connection, k: usize, s: &Statement) -> Result<(), String> {
    match db.execute_batch(&s.to_string()) {
        Err(rusqlite::Error::SqliteFailure(e, _)) if matches!(s, Statement::WalCheckpoint(_)) && e.extended_code & 0xff == rusqlite::ffi::SQLITE_LOCKED => Ok(()),
        r => r.map_err(|e| format!("statement {k}: {s}: {e}")),
    }
}

fn open_wal(case: &Case, dir: &Path) -> Result<Connection, String> {
    clear_storage();
    let io = |what: &str, e: std::io::Error| format!("disk I/O error: {what} {}: {e}", dir.display());
    if !dir.is_dir() {
        std::fs::create_dir_all(dir).map_err(|e| io("making", e))?;
    }
    let path = dir.join(format!("ephyra-{}-{}.db", std::process::id(), SERIAL.fetch_add(1, AtomicOrdering::Relaxed)));
    LAST_FILE.with(|f| *f.borrow_mut() = Some(path.clone()));
    let bytes = &case.file.0;
    std::fs::write(&path, bytes).map_err(|e| io("writing a database in", e))?;
    let settings = wal_settings(case);
    let connect = || -> Result<Connection, String> {
        let db = Connection::open(&path).map_err(|e| e.to_string())?;
        db.progress_handler(10_000, Some(past_deadline));
        let s = &settings;
        let pragmas = format!("PRAGMA cache_size = {}; PRAGMA synchronous = {}; PRAGMA wal_autocheckpoint = {};", s.cache_size, s.synchronous, s.autocheckpoint);
        db.execute_batch(&pragmas).map_err(|e| format!("{pragmas}: {e}"))?;
        Ok(db)
    };
    let mut db = connect()?;
    if bytes.len() < 100 || bytes[19] != 2 {
        db.query_row("PRAGMA journal_mode=WAL", [], |r| r.get::<_, String>(0)).map_err(|e| format!("PRAGMA journal_mode=WAL: {e}"))?;
    }
    for (k, s) in case.statements.iter().enumerate() {
        run_statement(&db, k, s)?;
    }
    if settings.reopen && db.is_autocommit() {
        let mut persist: std::ffi::c_int = 1;
        // SAFETY: the handle is this open connection's; the file control
        // reads and writes the one int it is given.
        let rc = unsafe {
            rusqlite::ffi::sqlite3_file_control(
                db.handle(),
                c"main".as_ptr(),
                rusqlite::ffi::SQLITE_FCNTL_PERSIST_WAL,
                (&mut persist as *mut std::ffi::c_int).cast(),
            )
        };
        if rc != rusqlite::ffi::SQLITE_OK {
            return Err(format!("SQLITE_FCNTL_PERSIST_WAL: error {rc}"));
        }
        db.close().map_err(|(_, e)| format!("closing to reopen: {e}"))?;
        db = connect()?;
    }
    Ok(db)
}

fn open_memory(case: &Case) -> Result<Connection, String> {
    let mut db = Connection::open_in_memory().map_err(|e| e.to_string())?;
    db.progress_handler(10_000, Some(past_deadline));
    let bytes = &case.file.0;
    if !bytes.is_empty() {
        let mut bytes = bytes.to_vec();
        for at in [18, 19] {
            if bytes[at] == 2 {
                bytes[at] = 1;
            }
        }
        db.deserialize_read_exact(rusqlite::MAIN_DB, &bytes[..], bytes.len(), false).map_err(|e| e.to_string())?;
    }
    for (k, s) in case.statements.iter().enumerate() {
        run_statement(&db, k, s)?;
    }
    Ok(db)
}

/// What an oracle compared, and what it skipped (see the module
/// documentation).
#[derive(Clone, Debug, Default)]
pub struct Tally {
    /// Queries the oracle ran.
    pub queries: u64,
    /// Comparisons made (each variant against its original).
    pub comparisons: u64,
    /// Comparisons where both sides failed.
    pub both_failed: u64,
    pub overflow_skips: u64,
    /// Comparisons skipped because one side passed a SQLite limit
    /// ([`limit`]), and queries whose original passed one.
    pub limit_skips: u64,
    /// Comparisons skipped because one side's JSON function refused an
    /// argument ([`json_error`]), and queries whose original or own query
    /// was refused so.
    pub json_skips: u64,
    pub real_sum_skips: u64,
    /// Queries skipped for known SQLite behaviour ([`known_issue`]).
    pub known_skips: u64,
    /// Queries checked as rewritten around known SQLite behaviour
    /// ([`concat_as_text`]).
    pub known_rewrites: u64,
    /// GROUP BY queries skipped: a group's terms hold equal values that
    /// are not identical ([`ambiguous_groups`]).
    pub group_skips: u64,
    /// Comparisons skipped because a side's rows hold text with a NUL
    /// character ([`nul_text`]).
    pub nul_skips: u64,
    /// `plans-index-partial` and `plans-index-expr` variants skipped:
    /// `INDEXED BY` such an index finds no plan where the planner may not
    /// use it.
    pub hint_skips: u64,
    /// `setops-def` and `order-multiset` comparisons skipped: a group of
    /// the compound's members that SQLite computes as a compound of its own
    /// tells rows apart under sequences other than the compound's
    /// ([`steps_share_colls`]), with or without ORDER BY.
    pub coll_skips: u64,
    /// Queries the oracle has no formulation for (ctes on a query without
    /// a WITH clause, setops on one `sum` that is not a compound, windows
    /// on a RANGE frame with offsets over values near the ends).
    pub inapplicable: u64,
    /// The host's storage failing, not SQLite's logic ([`io_error`]: the
    /// disk full, an I/O error): [`storage`]'s comparisons where a side
    /// failed so, and inputs whose WAL file did not open so (in
    /// [`storage`], or in any oracle under [`Storage::Wal`]).
    pub io_skips: u64,
    /// Inputs whose case SQLite refused for a date function reading the
    /// current time in an index, a CHECK constraint or a generated column
    /// ([`nondeterministic`]).
    pub nondeterministic_skips: u64,
    /// Queries left out by a draw for throughput ([`thinned`]): one in
    /// two of those [`order`], [`setops`] and [`windows`] check with a
    /// formulation of their own ([`added_order`], [`setops_self`],
    /// [`window_wrap`]), which run each a query the oracle ran none for.
    pub thinned: u64,
    /// Comparisons per variant (see the module documentation).
    pub variants: BTreeMap<&'static str, u64>,
}

impl Tally {
    pub fn add(&mut self, o: &Tally) {
        self.queries += o.queries;
        self.comparisons += o.comparisons;
        self.both_failed += o.both_failed;
        self.overflow_skips += o.overflow_skips;
        self.limit_skips += o.limit_skips;
        self.json_skips += o.json_skips;
        self.real_sum_skips += o.real_sum_skips;
        self.known_skips += o.known_skips;
        self.known_rewrites += o.known_rewrites;
        self.group_skips += o.group_skips;
        self.nul_skips += o.nul_skips;
        self.hint_skips += o.hint_skips;
        self.coll_skips += o.coll_skips;
        self.inapplicable += o.inapplicable;
        self.io_skips += o.io_skips;
        self.nondeterministic_skips += o.nondeterministic_skips;
        self.thinned += o.thinned;
        for (k, v) in &o.variants {
            *self.variants.entry(k).or_insert(0) += v;
        }
    }
}

/// How two results are compared.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    /// Rows as a multiset, values exactly.
    Multiset,
    /// Rows as a set of values as SQLite compares them (DISTINCT).
    Set,
    /// One value, as SQLite compares it (count, min, max).
    Scalar,
    /// One `sum()`: integers exactly, reals skipped.
    Sum,
    /// Rows of a GROUP BY query as a multiset, values exactly but the
    /// sign of a real zero.
    Groups,
    /// Rows as a multiset of values as SQLite compares them (a compound with
    /// set operators, whose rows are one of equal values).
    Equiv,
    /// Rows in order: values exactly (`true`), or as SQLite compares them
    /// (`false`: rows that tie in the ORDER BY may differ in values that
    /// compare equal, see [`Query::total_order`]).
    Ordered(bool),
}

/// How the results of `q` compare: in order when it has an ORDER BY,
/// else as [`unordered_mode`] says.
fn mode(q: &Query, schema: &Schema) -> Mode {
    match q.total_order(schema) {
        Some(exact) => Mode::Ordered(exact),
        None => unordered_mode(q),
    }
}

/// How the results of `q` without its ORDER BY compare: a compound of
/// UNION ALL whose SELECTs all compare exactly as a multiset, so; any other
/// compound as a multiset of values as SQLite compares them.
fn unordered_mode(q: &Query) -> Mode {
    if !q.compound.is_empty() {
        let exact =
            q.compound.iter().all(|(op, _)| *op == SetOp::UnionAll) && q.cores().iter().all(|c| unordered_mode(c) == Mode::Multiset);
        return if exact { Mode::Multiset } else { Mode::Equiv };
    }
    match &q.items {
        Items::Agg(Agg::Sum, _) => Mode::Sum,
        Items::Agg(..) => Mode::Scalar,
        _ if q.distinct => Mode::Set,
        _ if !q.group.is_empty() => Mode::Groups,
        _ => Mode::Multiset,
    }
}

/// Rows with every value as SQLite compares it ([`Cell::equiv`]).
fn equiv_rows(r: &Rows) -> Rows {
    r.iter().map(|row| row.iter().map(Cell::equiv).collect()).collect()
}

fn overflow(e: &str) -> bool {
    e.contains("integer overflow")
}

/// Whether `e` is SQLite refusing a query for one of its size or depth
/// limits (SQLITE_LIMIT_EXPR_DEPTH, _COMPOUND_SELECT, _COLUMN,
/// _FUNCTION_ARG, _LIKE_PATTERN_LENGTH, _LENGTH, the FROM clause's and the
/// parser's), with its message.
fn limit(e: &str) -> bool {
    [
        "nested too deep",
        "Expression tree is too large",
        "too many terms in compound SELECT",
        "too many columns in result set",
        "too many arguments on function",
        "too many FROM clause terms",
        "too many references to",
        "LIKE or GLOB pattern too complex",
        "string or blob too big",
        "parser stack overflow",
    ]
    .iter()
    .any(|m| e.contains(m))
}

/// Whether `e` is a JSON function refusing an argument value: text that is
/// not JSON, a BLOB that is not JSONB, a path that is not one, an object
/// label that is not text.
fn json_error(e: &str) -> bool {
    ["malformed JSON", "JSON cannot hold BLOB values", "bad JSON path", "json_object() labels must be TEXT"].iter().any(|m| e.contains(m))
}

/// The skip an error one side of a comparison, or an oracle's own query,
/// fails with is counted in, if it is one.
fn skip(t: &mut Tally, e: &str) -> Option<()> {
    let n = if overflow(e) {
        &mut t.overflow_skips
    } else if limit(e) {
        &mut t.limit_skips
    } else if json_error(e) {
        &mut t.json_skips
    } else {
        return None;
    };
    *n += 1;
    Some(())
}

/// Whether a row holds text with a NUL character: SQLite leaves the
/// result of most operations on such text undefined (nulinstr.html), and
/// it came out by the plan (NOCASE stops at the NUL, `sqlite3StrNICmp`):
/// `c0 || c0` over two BLOBs that differ after a NUL byte gave one row of
/// an EXCEPT under NOCASE by definition and two from SQLite (run
/// 20261018-0438 setops-def w1-179561), `max(CAST(zeroblob(40) AS TEXT))`
/// 20 NULs from a materialized CTE and 40 inline in a UTF-16 database
/// (run 20261016-2205 cte-inline w5-1083). Such comparisons are skipped
/// ([`Tally::nul_skips`]).
fn nul_text(rows: &Rows) -> bool {
    rows.iter().flatten().any(|c| matches!(c, Cell::Text(t) if t.contains(&0)))
}

fn scalar(rows: &Rows) -> Option<&Cell> {
    match rows.as_slice() {
        [row] if row.len() == 1 => Some(&row[0]),
        _ => None,
    }
}

/// Compare `variant` (from `vsql`) with `orig` (from `osql`) under `mode`,
/// counted as a comparison of variant `name`. For a sum, `nan(original)`
/// tells whether the original's (or the variant's) NULL sum had non-NULL
/// inputs, so is a NaN.
#[allow(clippy::too_many_arguments)]
fn agree(
    t: &mut Tally,
    name: &'static str,
    mode: Mode,
    osql: &str,
    orig: &Outcome,
    vsql: &str,
    variant: &Outcome,
    nan: &dyn Fn(bool) -> bool,
) -> Result<(), String> {
    t.comparisons += 1;
    *t.variants.entry(name).or_insert(0) += 1;
    let fail = |why: &str| Err(format!("{why} ({name})\n  original: {osql}\n    -> {orig:?}\n  variant:  {vsql}\n    -> {variant:?}"));
    let (a, b) = match (orig, variant) {
        (Err(_), Err(_)) => {
            t.both_failed += 1;
            return Ok(());
        },
        (Err(e), Ok(_)) | (Ok(_), Err(e)) if skip(t, e).is_some() => return Ok(()),
        (Err(_), Ok(_)) => return fail("the original fails, the variant does not"),
        (Ok(_), Err(_)) => return fail("the variant fails, the original does not"),
        (Ok(a), Ok(b)) => (a, b),
    };
    if nul_text(a) || nul_text(b) {
        t.nul_skips += 1;
        return Ok(());
    }
    // Under a collating sequence other than BINARY, SQLite picks one of
    // texts that compare equal (DISTINCT, GROUP BY, compounds, min and
    // max), by the plan: such texts are compared folded.
    let folded;
    let (a, b) = if folding() {
        let fold = |r: &Rows| -> Rows { r.iter().map(|row| row.iter().map(Cell::folded).collect()).collect() };
        folded = (fold(a), fold(b));
        (&folded.0, &folded.1)
    } else {
        (a, b)
    };
    let same = match mode {
        Mode::Multiset => {
            let (mut a, mut b) = (a.clone(), b.clone());
            a.sort();
            b.sort();
            a == b
        },
        Mode::Groups => {
            let unsigned = |r: &Rows| {
                let mut r: Rows = r.iter().map(|row| row.iter().map(Cell::unsigned_zero).collect()).collect();
                r.sort();
                r
            };
            unsigned(a) == unsigned(b)
        },
        Mode::Set => {
            let set = |r: &Rows| r.iter().map(|row| row.iter().map(Cell::equiv).collect::<Vec<_>>()).collect::<BTreeSet<_>>();
            set(a) == set(b)
        },
        Mode::Equiv => {
            let classes = |r: &Rows| {
                let mut r = equiv_rows(r);
                r.sort();
                r
            };
            classes(a) == classes(b)
        },
        Mode::Ordered(true) => a == b,
        Mode::Ordered(false) => equiv_rows(a) == equiv_rows(b),
        Mode::Scalar => matches!((scalar(a), scalar(b)), (Some(x), Some(y)) if x.equiv() == y.equiv()),
        Mode::Sum => match (scalar(a), scalar(b)) {
            (Some(Cell::Real(_)), Some(Cell::Real(_))) => {
                t.real_sum_skips += 1;
                true
            },
            (Some(Cell::Null), Some(y)) if *y != Cell::Null && nan(true) => {
                t.real_sum_skips += 1;
                true
            },
            (Some(x), Some(Cell::Null)) if *x != Cell::Null && nan(false) => {
                t.real_sum_skips += 1;
                true
            },
            (Some(x), Some(y)) => x == y,
            _ => false,
        },
    };
    if same {
        Ok(())
    } else {
        fail("results differ")
    }
}

/// The original query must not fail but with "integer overflow" (see the
/// module documentation; compared as an outcome), for a SQLite limit
/// ([`limit`]) or with a JSON argument refused ([`json_error`]): then it is
/// skipped, `None` (counted in [`Tally::limit_skips`] or
/// [`Tally::json_skips`]). Any other error is an invalid query.
fn original(db: &Connection, sql: &str, t: &mut Tally) -> Result<Option<Outcome>, String> {
    judge_original(run(db, sql), sql, t)
}

/// [`original`] of an outcome already run.
fn judge_original(o: Outcome, sql: &str, t: &mut Tally) -> Result<Option<Outcome>, String> {
    match &o {
        Err(e) if overflow(e) => Ok(Some(o)),
        Err(e) if skip(t, e).is_some() => Ok(None),
        Err(e) => Err(format!("the original query fails: {sql}\n  -> {e}")),
        _ => Ok(Some(o)),
    }
}

/// Whether a `sum(e)` query's rows (filtered by its WHERE when `filtered`)
/// have a non-NULL `e`, so that a NULL sum is a NaN.
fn sum_inputs(db: &Connection, q: &Query, filtered: bool) -> bool {
    let Items::Agg(Agg::Sum, Some(e)) = &q.items else {
        return false;
    };
    let w = if filtered { q.where_sql(0) } else { String::new() };
    let sql = format!("{}SELECT count({}) FROM {}{w}", q.with_sql(), q.e(e, 0), q.from_sql(0, &[]));
    matches!(run(db, &sql).ok().as_ref().and_then(scalar), Some(Cell::Int(n)) if *n > 0)
}

/// Whether some group of a GROUP BY query (over its rows filtered by its
/// WHERE when `filtered`) holds, in some GROUP BY term, equal values that
/// are not identical: values of two types or two texts (`CAST(k AS
/// TEXT)`, told apart by their bytes, `COLLATE BINARY`, as the cast keeps
/// a column's NOCASE or RTRIM). An integer and a real that are equal differ in both; equal
/// texts and equal blobs are identical; `-0.0` and `0.0` differ in
/// neither (see the module documentation). True when the check fails. For
/// a compound, whether any of its SELECTs has such a group.
pub fn ambiguous_groups(db: &Connection, q: &Query, filtered: bool) -> bool {
    if own_ambiguous_groups(db, q, filtered, "", false) {
        return true;
    }
    // A grouped subquery or CTE body picks its bare columns' values from
    // an arbitrary row of each group as the top level does, and the query
    // reads them (run 20261014-0121, plans-noindex w3-69281: `c0 IN
    // (SELECT c2 .. GROUP BY c2 HAVING c2 GLOB ..)` over 0 and 0.0, where
    // the index scan's group holds 0.0 and the table scan's 0). A
    // correlated subquery does not run alone, which counts as ambiguous.
    let with = q.with_sql();
    let mut found = false;
    each_expr(q, &mut |e| {
        if let Some(s) = e.subquery() {
            found = found || own_ambiguous_groups(db, s, true, &with, true);
        }
    });
    found
        || q.with.iter().any(|cte| match &cte.body {
            CteBody::Select(b) => own_ambiguous_groups(db, b, true, &with, true),
            _ => false,
        })
}

/// [`ambiguous_groups`] of `q`'s own GROUP BY (its compound members'
/// too), run after `with` (the enclosing query's WITH clause, so that a
/// nested query's references to its CTEs resolve). A `nested` query's
/// DISTINCT (or its compound's UNION, INTERSECT or EXCEPT) is grouping
/// too: it keeps one of the rows that compare equal, which one unspecified
/// (run 20261015-1745, plans-noindex w0-79984 and plans-noopt w8-33112:
/// `x IN (SELECT DISTINCT a % b ..)` keeping 0 or 0.0 by scan order, which
/// a TEXT x compares as '0' = 0 true and '0' = 0.0 false); a top-level
/// one the oracles compare as SQLite compares values. A compound's
/// members are checked one at a time, not against each other; with a
/// UNION, INTERSECT or EXCEPT, a member's `min()` or `max()` too
/// ([`extreme_ambiguous`]).
fn own_ambiguous_groups(db: &Connection, q: &Query, filtered: bool, with: &str, nested: bool) -> bool {
    if !q.compound.is_empty() {
        let dedups = q.compound.iter().any(|(op, _)| *op != SetOp::UnionAll);
        let dedup = nested && dedups;
        return q.cores().iter().any(|c| {
            own_ambiguous_groups(db, c, filtered, with, nested)
                || (dedup && distinct_ambiguous(db, c, filtered, with))
                || (dedups && extreme_ambiguous(db, c, filtered, with))
        });
    }
    if q.group.is_empty() {
        return nested && q.distinct && distinct_ambiguous(db, q, filtered, with);
    }
    let terms: Vec<String> = q
        .group
        .iter()
        .map(|k| {
            let k = q.e(k, 0);
            format!("count(DISTINCT typeof({k})) > 1 OR count(DISTINCT CAST({k} AS TEXT) COLLATE BINARY) > 1")
        })
        .collect();
    let w = if filtered { q.where_sql(0) } else { String::new() };
    let own = q.with_sql();
    let with = if q.with.is_empty() { with } else { own.as_str() };
    let sql = format!("{with}SELECT 1 FROM {}{w}{} HAVING {} LIMIT 1", q.from_sql(0, &[]), q.group_sql(0), terms.join(" OR "));
    !matches!(run(db, &sql), Ok(rows) if rows.is_empty())
}

/// Whether a compound member's one aggregate, `min(e)` or `max(e)` (over
/// its rows filtered by its WHERE when `filtered`), ranges over values
/// that compare equal under `e`'s collating sequence but differ (`'-1'`
/// and `'-1  '` under RTRIM, `'a'` and `'A'` under NOCASE): it returns
/// whichever of equal extremes its plan visits first, and the aggregate's
/// result has no collating sequence, so the compound's UNION, INTERSECT or
/// EXCEPT tells the two apart (run 20261018-0438 plans-noindex w3-186496:
/// `SELECT min(c3) FROM t3 WHERE c1 < x'' EXCEPT SELECT min(c3) FROM t3`,
/// c3 `TEXT COLLATE RTRIM` holding both, gave '-1  ' through the index on
/// c1 and no row NOT INDEXED). Values equal under BINARY are identical
/// texts or equal numbers, which the compound takes for equal, so a check
/// grouping under `e`'s sequence and counting under BINARY finds only
/// such pairs. Aggregates elsewhere take an argument cast to INTEGER or
/// to TEXT of a BINARY value, which maps equal values to identical ones
/// (`crate::expr` `agg_arg_ok`), as do window functions' `min` and `max`;
/// two-argument `min(a, b)` and `max(a, b)` pick by argument position
/// among equal ones (`minmaxFunc`), not by plan.
fn extreme_ambiguous(db: &Connection, q: &Query, filtered: bool, with: &str) -> bool {
    let Items::Agg(Agg::Min | Agg::Max, Some(e)) = &q.items else {
        return false;
    };
    let k = q.e(e, 0).to_string();
    let w = if filtered { q.where_sql(0) } else { String::new() };
    let own = q.with_sql();
    let with = if q.with.is_empty() { with } else { own.as_str() };
    // `+(k)`: never a result column's number, and of `k`'s sequence.
    let sql = format!("{with}SELECT 1 FROM {}{w} GROUP BY +({k}) HAVING count(DISTINCT ({k}) COLLATE BINARY) > 1 LIMIT 1", q.from_sql(0, &[]));
    !matches!(run(db, &sql), Ok(rows) if rows.is_empty())
}

/// Whether an ungrouped core's result rows include values that compare
/// equal but are not the same (types or text differ: 0 and 0.0, 'a' and
/// 'A' under NOCASE), grouping by its result expressions as its DISTINCT
/// would; `*` and aggregate items count as not ambiguous.
fn distinct_ambiguous(db: &Connection, q: &Query, filtered: bool, with: &str) -> bool {
    let Items::Exprs(items) = &q.items else {
        return false;
    };
    if items.is_empty() || !q.group.is_empty() {
        return false;
    }
    let keys: Vec<String> = items.iter().map(|k| q.e(k, 0).to_string()).collect();
    let terms: Vec<String> =
        keys.iter().map(|k| format!("count(DISTINCT typeof({k})) > 1 OR count(DISTINCT CAST({k} AS TEXT) COLLATE BINARY) > 1")).collect();
    let w = if filtered { q.where_sql(0) } else { String::new() };
    let own = q.with_sql();
    let with = if q.with.is_empty() { with } else { own.as_str() };
    let sql = format!("{with}SELECT 1 FROM {}{w} GROUP BY {} HAVING {} LIMIT 1", q.from_sql(0, &[]), keys.join(", "), terms.join(" OR "));
    !matches!(run(db, &sql), Ok(rows) if rows.is_empty())
}

/// Whether `q`, on the final model `m`, meets known SQLite behaviour that
/// the oracles would report (see the module documentation); one query in
/// three of those [`as_checked`] rewrites around it.
pub fn known_issue(m: &Model, q: &Query) -> bool {
    invalid_text_pattern(m, q)
}

/// Every expression of `q`: its own, its compound members' and its CTE
/// bodies' (subqueries' through [`Expr::walk`]).
fn each_expr(q: &Query, f: &mut dyn FnMut(&Expr)) {
    for e in q.exprs() {
        e.walk(f);
    }
    for (_, m) in &q.compound {
        each_expr(m, f);
    }
    for cte in &q.with {
        if let CteBody::Select(b) = &cte.body {
            each_expr(b, f);
        }
    }
}

/// Whether `q` has a LIKE or GLOB whose pattern is a text literal with a
/// non-ASCII character before its first wildcard (where SQLite may scan an
/// index range for its prefix), and reads a table holding a value that is
/// not valid text in `m`'s encoding: a blob that is not UTF-8 (over-length
/// encodings included), or in a UTF-16 database not UTF-16.
pub fn invalid_text_pattern(m: &Model, q: &Query) -> bool {
    let mut pattern = false;
    each_expr(q, &mut |e| {
        if let Expr::Bin(op, _, rhs) = e {
            if let Expr::Lit(Value::Text(p)) = &**rhs {
                pattern |= non_ascii_prefix(*op, p);
            }
        }
    });
    if !pattern {
        return false;
    }
    let invalid = |b: &[u8]| match m.encoding {
        Encoding::Utf8 => std::str::from_utf8(b).is_err(),
        Encoding::Utf16le | Encoding::Utf16be => {
            let le = m.encoding == Encoding::Utf16le;
            let units: Vec<u16> = b.chunks(2).map(|c| if c.len() < 2 { 0xd800 } else if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
            String::from_utf16(&units).is_err()
        },
    };
    q.tables().iter().filter_map(|t| m.tables.get(*t)).any(|t| t.rows.iter().any(|r| r.values.iter().any(|v| matches!(v, Value::Blob(b) if invalid(b)))))
}

/// Whether `op` is LIKE or GLOB (or NOT) and its pattern `p` has a
/// non-ASCII character before its first wildcard.
fn non_ascii_prefix(op: BinOp, p: &str) -> bool {
    let glob = matches!(op, BinOp::Glob | BinOp::NotGlob);
    let wild = |c: char| if glob { matches!(c, '*' | '?' | '[') } else { matches!(c, '%' | '_') };
    (glob || matches!(op, BinOp::Like | BinOp::NotLike)) && p.chars().take_while(|&c| !wild(c)).any(|c| !c.is_ascii())
}

/// Whether `m` is a UTF-16 database and `q` (or a query below it) has a
/// `concat()` or `concat_ws()` argument that may be a BLOB, that is not
/// [`never_blob`] by its form. SQLite takes such an argument's size before
/// converting it to text, so the result holds the BLOB's size in bytes of
/// the UTF-8 text: a NUL and stale bytes past it, or a truncated
/// character; and as the argument is converted in place, a constant BLOB
/// is wrong only on the first row, so the answer depends on the plan
/// (case-studies/sqlite/bugs/sqlite-concat-blob-utf16-length, every SQLite since
/// 3.44.0). The oracles check such a query as [`concat_as_text`] writes it.
pub fn concat_blob_utf16(m: &Model, q: &Query) -> bool {
    let mut arg = false;
    if m.encoding != Encoding::Utf8 {
        each_expr_deep(q, &mut |e| {
            if let Expr::Call(Func::Concat | Func::ConcatWs, args) = e {
                arg |= args.iter().any(|a| !never_blob(a));
            }
        });
    }
    arg
}

/// `q` with every `concat()` and `concat_ws()` argument that may be a BLOB
/// (not [`never_blob`]) as `CAST(x AS TEXT)`, in it and every query below
/// it: the same query, as these functions read every argument as text
/// (a BLOB as text in the database's encoding, as `CAST` does), without the
/// SQLite bug of [`concat_blob_utf16`], which a TEXT argument avoids.
pub fn concat_as_text(q: &Query) -> Query {
    fn query(q: &mut Query) {
        for e in q.exprs_mut() {
            expr(e);
        }
        for (_, m) in &mut q.compound {
            query(m);
        }
        for cte in &mut q.with {
            if let CteBody::Select(b) = &mut cte.body {
                query(b);
            }
        }
    }
    fn expr(e: &mut Expr) {
        if let Expr::Call(Func::Concat | Func::ConcatWs, args) = e {
            for a in args.iter_mut().filter(|a| !never_blob(a)) {
                *a = Expr::Cast(Box::new(a.clone()), CastType::Text);
            }
        }
        for c in e.children_mut() {
            expr(c);
        }
        if let Some(s) = e.subquery_mut() {
            query(s);
        }
    }
    let mut q = q.clone();
    query(&mut q);
    q
}

/// [`each_expr`] of `q` and of every query below it (subqueries'
/// compound members and CTE bodies too).
fn each_expr_deep(q: &Query, f: &mut dyn FnMut(&Expr)) {
    each_expr(q, &mut |e| {
        f(e);
        if let Some(s) = e.subquery() {
            each_expr_deep(s, f);
        }
    });
}

/// Run `check` on every query of the case's final state but those with a
/// known issue, on the database [`open`] gives (under [`Storage::Wal`], a
/// file that does not open for the host's storage is one
/// [`Tally::io_skips`]).
fn each(
    case: &Case,
    queries: &[Query],
    check: impl FnMut(&Connection, &Model, &Schema, &Query, &mut Tally) -> Result<(), String>,
) -> Result<Tally, String> {
    let db = match open(case) {
        Err(e) if io_error(&e) && thread_storage() != Storage::Memory => return Ok(Tally { io_skips: 1, ..Tally::default() }),
        Err(e) if nondeterministic(&e) => return Ok(Tally { nondeterministic_skips: 1, ..Tally::default() }),
        db => db?,
    };
    each_on(&db, case, queries, check)
}

/// [`each`] on `db`, the case's database.
fn each_on(
    db: &Connection,
    case: &Case,
    queries: &[Query],
    mut check: impl FnMut(&Connection, &Model, &Schema, &Query, &mut Tally) -> Result<(), String>,
) -> Result<Tally, String> {
    let m = final_model(case).ok_or("the file's model does not read back")?;
    let schema = Schema::of(&m);
    let mut tally = Tally::default();
    for drawn in queries {
        let (q, scoped) = as_checked(&m, &schema, drawn)?;
        if let Cow::Owned(_) = q {
            tally.known_rewrites += 1;
        }
        FOLD.with(|f| f.set(folds(&schema, &q)));
        tally.queries += 1;
        if known_issue(&m, &q) {
            tally.known_skips += 1;
            continue;
        }
        check(db, &m, &scoped, &q, &mut tally)?;
    }
    Ok(tally)
}

/// The query the oracles check for `drawn` in `m` ([`concat_as_text`] of it
/// when [`concat_blob_utf16`], then [`patterns_computed`] of that when
/// [`invalid_text_pattern`], for one such query in three, owned then), and the schema with its CTEs as
/// sources. That schema is of the query as drawn: the rewrite is the same
/// query, CTE columns of the same names, values, affinities and collating
/// sequences (`CAST` passes its operand's collating sequence on, and a
/// concat() argument's sets nothing), but not always one [`Query::valid`]
/// accepts: a `CAST(x AS TEXT)` of a column with another collating
/// sequence than BINARY inside an aggregate's or a window's argument
/// (`max(CAST(concat_ws(',', c0) AS INTEGER))` over a NOCASE c0) fails its
/// `casts_binary` rule, which is about the aggregate's comparisons, not a
/// concat() argument's. Scoping the rewrite made "a CTE of the query is not
/// valid" of such CTE bodies (run 20261016-1828, every oracle).
pub fn as_checked<'a>(m: &Model, schema: &Schema, drawn: &'a Query) -> Result<(Cow<'a, Query>, Schema), String> {
    let scoped = drawn.scope(schema).ok_or_else(|| format!("a CTE of the query is not valid: {}", drawn.sql()))?;
    let q = if concat_blob_utf16(m, drawn) { Cow::Owned(concat_as_text(drawn)) } else { Cow::Borrowed(drawn) };
    // One such query in three, for throughput (the others stay
    // [`known_issue`]s, skipped).
    let q = if invalid_text_pattern(m, &q) && (fnv(&drawn.sql()) >> 37).is_multiple_of(3) { Cow::Owned(patterns_computed(&q)) } else { q };
    Ok((q, scoped))
}

/// `q` with the pattern of every LIKE and GLOB that is a text literal with
/// a non-ASCII character before its first wildcard ([`non_ascii_prefix`])
/// written `('p' || '')`, in it and every query below it: the same query
/// (the concatenation is the same text, and a pattern's collating sequence
/// plays no part in LIKE and GLOB), but one the LIKE optimization does not
/// apply to, as it takes only a literal pattern (`isLikeOrGlob`): every
/// plan then reads each row and matches it alike, an over-length UTF-8
/// value included, the known SQLite behaviour of
/// [`invalid_text_pattern`].
pub fn patterns_computed(q: &Query) -> Query {
    fn query(q: &mut Query) {
        for e in q.exprs_mut() {
            expr(e);
        }
        for (_, m) in &mut q.compound {
            query(m);
        }
        for cte in &mut q.with {
            if let CteBody::Select(b) = &mut cte.body {
                query(b);
            }
        }
    }
    fn expr(e: &mut Expr) {
        if let Expr::Bin(op, _, rhs) = e {
            if let Expr::Lit(Value::Text(p)) = &**rhs {
                if non_ascii_prefix(*op, p) {
                    let computed = Expr::Bin(BinOp::Concat, rhs.clone(), Box::new(Expr::Lit(Value::Text(String::new()))));
                    **rhs = computed;
                }
            }
        }
        for c in e.children_mut() {
            expr(c);
        }
        if let Some(s) = e.subquery_mut() {
            query(s);
        }
    }
    let mut q = q.clone();
    query(&mut q);
    q
}

/// The query's WHERE clause, `1` without one.
fn predicate(q: &Query) -> String {
    q.filter.as_ref().map_or("1".to_string(), |p| q.e(p, 0).to_string())
}

/// The three partitions on `x` as `keyword` clauses: ` WHERE (x)`,
/// ` WHERE NOT (x)`, ` WHERE (x) IS NULL`.
fn partitions(keyword: &str, x: &str) -> [String; 3] {
    [format!(" {keyword} ({x})"), format!(" {keyword} NOT ({x})"), format!(" {keyword} ({x}) IS NULL")]
}

/// The rows of every query, concatenated; or the first error.
fn concat(db: &Connection, sqls: &[String]) -> Outcome {
    let mut rows = Vec::new();
    for sql in sqls {
        rows.extend(run(db, sql)?);
    }
    Ok(rows)
}

/// NoREC (see the module documentation), on every query's FROM and WHERE
/// clauses.
pub fn norec(case: &Case, queries: &[Query]) -> Result<Tally, String> {
    each(case, queries, |db, _, _, q, tally| q.cores().iter().try_for_each(|c| norec_core(db, c, tally)))
}

/// NoREC on one SELECT (a query's core or a compound member).
fn norec_core(db: &Connection, q: &Query, tally: &mut Tally) -> Result<(), String> {
    let (p, from) = (predicate(q), q.from_sql(0, &[]));
    let w = q.with_sql();
    let osql = format!("{w}SELECT count(*) FROM {from} WHERE {p}");
    let vsql = format!("{w}SELECT sum(ref0) FROM (SELECT ({p}) IS TRUE AS ref0 FROM {from})");
    let Some(orig) = original(db, &osql, tally)? else {
        return Ok(());
    };
    // A sum over no rows is NULL: no row is selected.
    let variant = run(db, &vsql).map(|rows| if rows == [vec![Cell::Null]] { vec![vec![Cell::Int(0)]] } else { rows });
    let name = if q.joins.is_empty() { "norec" } else { "norec-join" };
    agree(tally, name, Mode::Scalar, &osql, &orig, &vsql, &variant, &|_| false)
}

/// TLP (see the module documentation): the variant the query's shape
/// allows.
pub fn tlp(case: &Case, queries: &[Query]) -> Result<Tally, String> {
    each(case, queries, |db, _, schema, q, tally| q.cores().iter().try_for_each(|c| tlp_core(db, schema, c, tally)))
}

/// TLP on one SELECT (a query's core or a compound member).
fn tlp_core(db: &Connection, schema: &Schema, q: &Query, tally: &mut Tally) -> Result<(), String> {
    // A window's values depend on every row of its partition, which a
    // partition of the WHERE clause splits: the query is partitioned
    // outside, on its result rows.
    if q.has_window() {
        return tlp_window(db, q, TLP_WINDOW, tally);
    }
    let (p, from, select) = (predicate(q), q.from_sql(0, &[]), q.select_sql(0));
    let has_agg = q.items.exprs().iter().any(|e| e.has_agg());
    let (name, osql, parts): (&'static str, String, Vec<String>) = match &q.items {
        Items::Agg(a, _) => {
            let outer = match a {
                Agg::Count | Agg::Sum => "sum",
                Agg::Min => "min",
                Agg::Max => "max",
            };
            let parts: Vec<String> =
                partitions("WHERE", &p).iter().map(|w| format!("SELECT {} AS ref0 FROM {from}{w}", q.items_sql(0))).collect();
            // `ref0` is `agg(e)`, a function's result, which has no
            // collating sequence (datatype3.html 7.1): an outer `min` or
            // `max` would compare the partitions' results under BINARY
            // where the original compares under `e`'s sequence, and pick
            // another text where they differ (NOCASE: 'B' and 'a'; RTRIM
            // in a UTF-16 database, which converts to UTF-8 to compare
            // where BINARY compares UTF-16 bytes: 'ж' and 'k'). The
            // outer aggregate takes `e`'s sequence explicitly.
            let arg = match (a, &q.items) {
                (Agg::Min | Agg::Max, Items::Agg(_, Some(e))) => match crate::expr::expr_coll(e, schema, &[q.sources()]) {
                    Coll::Binary => "ref0".to_string(),
                    k => format!("ref0 COLLATE {}", k.name()),
                },
                _ => "ref0".to_string(),
            };
            let vsql = format!("SELECT {outer}({arg}) FROM ({})", parts.join(" UNION ALL "));
            ("tlp-agg", format!("{select} FROM {from}"), vec![vsql])
        },
        _ if q.group.is_empty() && !has_agg => {
            let osql = format!("{select} FROM {from}");
            let parts = partitions("WHERE", &p).iter().map(|w| format!("{osql}{w}")).collect();
            (if q.distinct { "tlp-distinct" } else { "tlp-where" }, osql, parts)
        },
        _ if !q.group.is_empty() && q.having.is_some() => {
            if ambiguous_groups(db, q, true) {
                tally.group_skips += 1;
                return Ok(());
            }
            let osql = format!("{select} FROM {from}{}{}", q.where_sql(0), q.group_sql(0));
            let h = q.e(q.having.as_ref().unwrap(), 0).to_string();
            let parts = partitions("HAVING", &h).iter().map(|x| format!("{osql}{x}")).collect();
            ("tlp-having", osql, parts)
        },
        _ if !q.group.is_empty() && !has_agg => {
            if ambiguous_groups(db, q, false) {
                tally.group_skips += 1;
                return Ok(());
            }
            let group = q.group_sql(0);
            let parts = partitions("WHERE", &p).iter().map(|w| format!("{select} FROM {from}{w}{group}")).collect();
            ("tlp-group", format!("{select} FROM {from}{group}"), parts)
        },
        // An aggregate query with neither one aggregate nor HAVING nor an
        // aggregate-free GROUP BY: a partition of its WHERE clause would
        // split groups and change their aggregates; partitioned outside,
        // on its result rows, as a window query is.
        _ => return tlp_window(db, q, TLP_OUTER, tally),
    };
    // Every statement run starts with the query's WITH clause.
    let with = q.with_sql();
    let pre = |s: &str| format!("{with}{s}");
    // The partitions are compared without the query's ORDER BY and LIMIT.
    let mode = if name == "tlp-group" { Mode::Set } else { unordered_mode(q) };
    let stmts: Vec<String> = parts.iter().map(|s| pre(s)).collect();
    let variant = concat(db, &stmts);
    let osql = pre(&osql);
    let Some(orig) = original(db, &osql, tally)? else {
        return Ok(());
    };
    // Both sides of a TLP-aggregate sum over every row of the join.
    agree(tally, name, mode, &osql, &orig, &stmts.join(";\n             "), &variant, &|_| sum_inputs(db, q, false))?;
    if name == "tlp-agg" {
        return Ok(());
    }
    // The partitions as one compound: UNION ALL, or UNION for a set.
    let usql = pre(&parts.join(if mode == Mode::Set { " UNION " } else { " UNION ALL " }));
    agree(tally, "tlp-union", mode, &osql, &orig, &usql, &run(db, &usql), &|_| false)?;
    let osql = format!("{select} FROM {from}");
    if !matches!(name, "tlp-where" | "tlp-distinct") {
        return Ok(());
    }
    // Q(p) INTERSECT Q is Q(p) as a set; Q EXCEPT Q(p) is (Q(NOT p)
    // UNION Q(p IS NULL)) EXCEPT Q(p).
    let [yes, no, null] = partitions("WHERE", &p).map(|w| format!("{osql}{w}"));
    let distinct = pre(&format!("SELECT DISTINCT {} FROM {from} WHERE ({p})", q.items_sql(0)));
    let isql = pre(&format!("{yes} INTERSECT {osql}"));
    let Some(dorig) = original(db, &distinct, tally)? else {
        return Ok(());
    };
    agree(tally, "tlp-intersect", Mode::Set, &distinct, &dorig, &isql, &run(db, &isql), &|_| false)?;
    let esql = pre(&format!("{osql} EXCEPT {yes}"));
    let vsql = pre(&format!("{no} UNION {null} EXCEPT {yes}"));
    let Some(eorig) = original(db, &esql, tally)? else {
        return Ok(());
    };
    agree(tally, "tlp-except", Mode::Set, &esql, &eorig, &vsql, &run(db, &vsql), &|_| false)
}

/// TLP on a SELECT with window items (`tlp-window`, see the module
/// documentation): the SELECT as written (its WINDOW clause too) but for
/// its ORDER BY and LIMIT,
/// `Q`, against `WITH tw(r0, ..) AS (Q) SELECT * FROM tw WHERE (x)`, `..
/// WHERE NOT (x)`, `.. WHERE (x) IS NULL` (after the query's own CTEs),
/// `x` over one result column `rk` ([`window_partition`]). The windows'
/// values are computed in `Q` before any partition, so each result row
/// of `Q` falls in exactly one; SQLite may push `x` down into `Q` only
/// where the windows cannot see it (`pushDownWhereTerms`: a PARTITION BY
/// term of every window), and that is what this checks: `x` is over such
/// an item half of the time, and in a query neither grouped nor aggregate
/// with no such item a term of every window's PARTITION BY is half of the
/// time an item of its own after the query's (`Q` then has it). Also
/// `tlp-window-union`, the partitions as one compound. The same for an
/// aggregate query TLP has no other formulation for (`names`
/// [`TLP_OUTER`]), a GROUP BY term in place of a PARTITION BY term: SQLite
/// pushes `x` down into `Q`'s HAVING, and from there into its WHERE on a
/// GROUP BY term (`havingToWhere`). Each group's values are those of `Q`
/// (its aggregates' arguments are such that the plan does not change
/// them, as the plans oracle checks; ambiguous groups are skipped), so
/// each of its rows falls in exactly one partition.
fn tlp_window(db: &Connection, q: &Query, names: [&'static str; 2], tally: &mut Tally) -> Result<(), String> {
    // A group's (or a nested query's) bare columns come from a row of it
    // the plan picks, which pushing `x` down may change.
    if ambiguous_groups(db, q, true) {
        tally.group_skips += 1;
        return Ok(());
    }
    let items = q.items.exprs();
    // The items SQLite may push a partition on down into `Q`: a term of
    // every window's PARTITION BY.
    let windows: Vec<&Window> = items.iter().filter_map(|e| if let Expr::Window(w) = e { Some(&**w) } else { None }).collect();
    // Without windows (`tlp-outer`, an aggregate query), the items SQLite
    // moves from the pushed-down HAVING to the WHERE (`havingToWhere`): a
    // GROUP BY term.
    let pushable: Vec<usize> = match windows[..] {
        [] => (0..items.len()).filter(|&k| q.group.contains(items[k])).collect(),
        _ => (0..items.len()).filter(|&k| windows.iter().all(|w| w.partition.contains(items[k]))).collect(),
    };
    // The tail of the core, without ORDER BY and LIMIT: its WINDOW clause.
    let tail = q.core().tail_sql(0);
    let (from, filter, group, having) = (q.from_sql(0, &[]), q.where_sql(0), q.group_sql(0), q.having_sql(0));
    let with = q.with_sql();
    let h = fnv(&format!("{with}{} FROM {from}{filter}{group}{having}{tail}", q.select_sql(0)));
    // Without such an item, half of the time a term of every window's
    // PARTITION BY is an item of its own after the query's, in a query
    // neither grouped nor aggregate (where it is a value of each row).
    // Without windows, a GROUP BY term (a value of each group).
    let common: Vec<&Expr> = match windows[..] {
        [] => q.group.iter().collect(),
        [w, ..] => w.partition.iter().filter(|t| windows.iter().all(|w| w.partition.contains(t))).collect(),
    };
    let extra = match common[..] {
        [_, ..] if pushable.is_empty() && (windows.is_empty() || !q.aggregate()) && (h >> 58).is_multiple_of(2) => {
            Some(common[(h % common.len() as u64) as usize])
        },
        _ => None,
    };
    let (n, pushable) = match extra {
        Some(_) => (items.len() + 1, vec![items.len()]),
        None => (items.len(), pushable),
    };
    let select = q.select_sql(0) + &extra.map_or(String::new(), |t| format!(", {}", q.e(t, 0)));
    let core = format!("{select} FROM {from}{filter}{group}{having}{tail}");
    let osql = format!("{with}{core}");
    let Some(orig) = original(db, &osql, tally)? else {
        return Ok(());
    };
    let (x, materialized) = window_partition(h, n, &pushable, extra.is_some(), &orig);
    let columns: Vec<String> = (0..n).map(|k| format!("r{k}")).collect();
    let prefix = if with.is_empty() { "WITH ".to_string() } else { format!("{}, ", with.trim_end()) };
    let hint = if materialized { "MATERIALIZED " } else { "" };
    let base = format!("{prefix}tw({}) AS {hint}({core}) ", columns.join(", "));
    let mode = unordered_mode(q);
    let parts = partitions("WHERE", &x).map(|w| format!("SELECT * FROM tw{w}"));
    let stmts: Vec<String> = parts.iter().map(|s| format!("{base}{s}")).collect();
    agree(tally, names[0], mode, &osql, &orig, &stmts.join(";\n             "), &concat(db, &stmts), &|_| false)?;
    let usql = format!("{base}{}", parts.join(if mode == Mode::Set { " UNION " } else { " UNION ALL " }));
    agree(tally, names[1], mode, &osql, &orig, &usql, &run(db, &usql), &|_| false)
}

/// The variant names of [`tlp_window`] for a query with window items.
const TLP_WINDOW: [&str; 2] = ["tlp-window", "tlp-window-union"];
/// The variant names of [`tlp_window`] for an aggregate query TLP has no
/// other formulation for (`tlp-outer`).
const TLP_OUTER: [&str; 2] = ["tlp-outer", "tlp-outer-union"];

/// `window-wrap`, for a query [`windows`] does not compute the windows of
/// (no window items, or in a UTF-16 database, or compared folded), on
/// each of its SELECTs `Q` (as TLP, [`window_wrap_core`]):
/// `WITH tw(r0, ..) AS (Q) SELECT *, count(*) OVER (), count(*) OVER
/// (PARTITION BY rk) FROM tw` (after the query's own CTEs; MATERIALIZED
/// one time in four), one time in eight against `Q`, its first columns
/// compared as `Q`'s rows are ([`unordered_mode`]); and always
/// `window-count`, each row's counts against the variant's own rows: the
/// first the number of rows, the second the number whose `rk` SQLite
/// tells apart from the row's no more than in a partition
/// ([`Cell::distinct_key`]: numbers by value, text and blobs by their
/// bytes, NULLs alike). `rk` is a column drawn from [`fnv`] of `Q`'s
/// text (deterministic). The partition is left out in a UTF-16 database
/// (text read back in UTF-8) and where the query is compared folded (`rk`
/// may be under NOCASE or RTRIM). Ambiguous groups are skipped (counted in
/// [`Tally::group_skips`]).
fn window_wrap(db: &Connection, m: &Model, schema: &Schema, q: &Query, tally: &mut Tally) -> Result<(), String> {
    if thinned(fnv(&q.sql()), tally) {
        return Ok(());
    }
    if ambiguous_groups(db, q, true) {
        tally.group_skips += 1;
        return Ok(());
    }
    q.cores().iter().try_for_each(|c| window_wrap_core(db, m, schema, c, tally))
}

/// [`window_wrap`] on one SELECT (a query's core or a compound member): a
/// compound in a CTE gives its columns the affinity of its first member's,
/// which converts another member's values (an integer 0 under a REAL
/// column reads as 0.0).
fn window_wrap_core(db: &Connection, m: &Model, schema: &Schema, body: &Query, tally: &mut Tally) -> Result<(), String> {
    let osql = body.sql();
    let h = fnv(&osql);
    // `Q` itself one time in eight (`window-wrap`); the counts always.
    let orig = match (h >> 44).is_multiple_of(8) {
        true => match original(db, &osql, tally)? {
            Some(o) => Some(o),
            None => return Ok(()),
        },
        false => None,
    };
    let with = body.with_sql();
    let Some(inner) = osql.strip_prefix(&with) else {
        return Err(format!("the query does not start with its WITH clause: {osql}"));
    };
    let n = body.width(schema);
    let k = (h % n.max(1) as u64) as usize;
    let partitioned = m.encoding == Encoding::Utf8 && !folding();
    let columns: Vec<String> = (0..n).map(|k| format!("r{k}")).collect();
    let prefix = if with.is_empty() { "WITH ".to_string() } else { format!("{}, ", with.trim_end()) };
    let hint = if (h >> 50).is_multiple_of(4) { "MATERIALIZED " } else { "" };
    let windows = if partitioned { format!("count(*) OVER (), count(*) OVER (PARTITION BY r{k})") } else { "count(*) OVER ()".to_string() };
    let vsql = format!("{prefix}tw({}) AS {hint}({inner}) SELECT *, {windows} FROM tw", columns.join(", "));
    let variant = match &orig {
        Some(orig) => {
            let variant = run(db, &vsql);
            let rows = variant.as_ref().map(|r| r.iter().map(|row| row[..n.min(row.len())].to_vec()).collect()).map_err(|e| e.clone());
            agree(tally, "window-wrap", unordered_mode(body), &osql, orig, &vsql, &rows, &|_| false)?;
            variant
        },
        // The variant alone is the original: it fails only as `Q` may.
        None => match original(db, &vsql, tally)? {
            Some(v) => v,
            None => return Ok(()),
        },
    };
    let Ok(rows) = variant else {
        return Ok(());
    };
    tally.comparisons += 1;
    *tally.variants.entry("window-count").or_insert(0) += 1;
    let mut parts: BTreeMap<Cell, i64> = BTreeMap::new();
    if partitioned {
        for row in &rows {
            *parts.entry(row[k].distinct_key()).or_insert(0) += 1;
        }
    }
    for row in &rows {
        let all = Cell::Int(rows.len() as i64);
        let part = partitioned.then(|| Cell::Int(parts[&row[k].distinct_key()]));
        if row.len() != n + 1 + usize::from(partitioned) || row[n] != all || part.as_ref().is_some_and(|p| row[n + 1] != *p) {
            return Err(format!(
                "a row's window counts are not its rows' (window-count: {} rows, {} in its partition on r{k})\n  query: {vsql}\n    -> {row:?}",
                rows.len(),
                part.map_or("-".to_string(), |p| format!("{p:?}"))
            ));
        }
    }
    Ok(())
}

/// Whether the query whose text's [`fnv`] is `h` is left out of
/// [`added_order`], [`setops_self`] and [`window_wrap`] (counted in
/// [`Tally::thinned`]): one query in two. Each runs queries its oracle
/// ran none of before, for every query of the runner's inputs it is given,
/// so the runner's throughput would fall past its budget.
fn thinned(h: u64, tally: &mut Tally) -> bool {
    let out = (h >> 37).is_multiple_of(2);
    tally.thinned += u64::from(out);
    out
}

/// FNV-1a of `s`.
fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The partitioning predicate of [`tlp_window`] for a query of `n`
/// result columns, and whether its CTE is MATERIALIZED, both drawn from
/// `h` (the query's [`fnv`]; deterministic): a column `rk` (one of the
/// `pushable` ones always when `push`, half of the time else, when there
/// are), and `rk` itself (its truth), or `rk` compared (`<`, `=`, `>=`)
/// with its value in one of the query's result rows `orig`, so that each
/// partition tends to hold rows. A NULL, a JSONB value or a text that is
/// not UTF-8 (or holds a NUL) is not written as a literal: `rk` then.
fn window_partition(h: u64, n: usize, pushable: &[usize], push: bool, orig: &Outcome) -> (String, bool) {
    let k = match pushable {
        [_, ..] if push || (h >> 56).is_multiple_of(2) => pushable[(h % pushable.len() as u64) as usize],
        _ => (h % n.max(1) as u64) as usize,
    };
    let column = format!("r{k}");
    let value = match orig {
        Ok(rows) if !rows.is_empty() => rows[((h >> 20) % rows.len() as u64) as usize].get(k).and_then(cell_literal),
        _ => None,
    };
    let x = match (value, (h >> 40) % 4) {
        (Some(v), 1) => format!("{column} < {v}"),
        (Some(v), 2) => format!("{column} = {v}"),
        (Some(v), 3) => format!("{column} >= {v}"),
        _ => column,
    };
    (x, (h >> 50).is_multiple_of(4))
}

/// A result value as an SQL literal of the same value, where one is
/// written ([`crate::statements::literal`]).
fn cell_literal(c: &Cell) -> Option<String> {
    let v = match c {
        Cell::Int(i) => Value::Int(*i),
        Cell::Real(b) => Value::Real(f64::from_bits(*b)),
        Cell::Text(t) => Value::Text(std::str::from_utf8(t).ok().filter(|s| !s.contains('\0'))?.to_string()),
        Cell::Blob(b) => Value::Blob(b.clone()),
        Cell::Null | Cell::Jsonb(..) => return None,
    };
    Some(crate::statements::literal(&v))
}

/// The join-order variants of the plans oracle (see the module
/// documentation): none without a join.
fn join_variants(q: &Query, schema: &Schema) -> Vec<(&'static str, String)> {
    let n = q.joins.len() + 1;
    if n == 1 {
        return Vec::new();
    }
    let sources = q.sources();
    let src = |k: usize| format!("{} AS {}", table_name(sources[k]), alias(0, k));
    let rest = format!("{}{}{}", q.group_sql(0), q.having_sql(0), q.tail_sql(0));
    // `*` with the columns in the order the query as written has them.
    let select = match &q.items {
        Items::Star => {
            let stars: Vec<String> = (0..n).map(|k| format!("{}.*", alias(0, k))).collect();
            format!("SELECT {}{}", if q.distinct { "DISTINCT " } else { "" }, stars.join(", "))
        },
        _ => q.select_sql(0),
    };
    let mut out = Vec::new();
    if q.joins.iter().all(|j| matches!(j.kind, JoinKind::Inner(_) | JoinKind::Cross)) {
        let mut cross = src(0);
        for (k, j) in q.joins.iter().enumerate() {
            cross += &format!(" CROSS JOIN {}", src(k + 1));
            if let Some(on) = j.on() {
                cross += &format!(" ON {}", q.e(on, 0));
            }
        }
        out.push(("plans-cross", format!("{} FROM {cross}{}{rest}", q.select_sql(0), q.where_sql(0))));
        let terms: Vec<String> =
            q.joins.iter().filter_map(|j| j.on()).chain(q.filter.iter()).map(|e| format!("({})", q.e(e, 0))).collect();
        let w = if terms.is_empty() { String::new() } else { format!(" WHERE {}", terms.join(" AND ")) };
        let comma: Vec<String> = (0..n).map(src).collect();
        out.push(("plans-comma", format!("{} FROM {}{w}{rest}", q.select_sql(0), comma.join(", "))));
        let reversed: Vec<String> = (0..n).rev().map(src).collect();
        out.push(("plans-reorder", format!("{select} FROM {}{w}{rest}", reversed.join(" CROSS JOIN "))));
    } else if let (2, JoinKind::Left(on)) = (n, &q.joins[0].kind) {
        let from = format!("{} RIGHT JOIN {} ON {}", src(1), src(0), q.e(on, 0));
        out.push(("plans-right", format!("{select} FROM {from}{}{rest}", q.where_sql(0))));
    } else if let (2, JoinKind::Right(on)) = (n, &q.joins[0].kind) {
        let from = format!("{} LEFT JOIN {} ON {}", src(1), src(0), q.e(on, 0));
        out.push(("plans-left", format!("{select} FROM {from}{}{rest}", q.where_sql(0))));
    } else if let (2, JoinKind::Full(on)) = (n, &q.joins[0].kind) {
        let from = format!("{} FULL JOIN {} ON {}", src(1), src(0), q.e(on, 0));
        out.push(("plans-full-swap", format!("{select} FROM {from}{}{rest}", q.where_sql(0))));
    }
    // USING or NATURAL as ON equalities of the columns, `*` written out:
    // a USING column once, the left source's for an inner or LEFT join, the
    // right source's for a RIGHT join, the two coalesced for a FULL join,
    // as SQLite resolves the column's name (`lookupName`). SQLite's own
    // coalesce() there is marked `SQLITE_AFF_DEFER`, so it has the left
    // column's collating sequence (`sqlite3ExprCollSeq`; 3.53.2 on, 3580d166)
    // where a written coalesce() has none (BINARY): the left column's
    // sequence is written out with COLLATE, or ORDER BY and DISTINCT on it
    // differ (an RTRIM column in a UTF-16 database, run 1959).
    if let (Some(op), JoinKind::Using(..) | JoinKind::Natural(_)) = (q.joins[0].kind.op(), &q.joins[0].kind) {
        let columns: Vec<usize> = sources.iter().map(|t| schema.info(*t).map_or(0, |i| i.columns)).collect();
        let using = q.joins[0].using(columns[0], columns[1]);
        let on: Vec<String> = using.iter().map(|c| format!("(a0.c{c} = a1.c{c})")).collect();
        // The query's expressions with every column qualified: a USING
        // column's name written bare (`At::using`) is ambiguous under ON.
        let mut qualified = q.clone();
        qualified.joins[0].kind = JoinKind::Cross;
        let q = &qualified;
        let rest = format!("{}{}{}", q.group_sql(0), q.having_sql(0), q.tail_sql(0));
        // The later joins as written.
        let from = format!("{} {} {} ON {}{}", src(0), op.keyword(), src(1), on.join(" AND "), q.joins_sql(1));
        let items = match &q.items {
            Items::Star => {
                let mut cols = Vec::new();
                for c in 0..columns[0] {
                    cols.push(match op {
                        _ if !using.contains(&c) => format!("a0.c{c}"),
                        JoinOp::Inner | JoinOp::Left => format!("a0.c{c}"),
                        JoinOp::Right => format!("a1.c{c}"),
                        JoinOp::Full => match schema.info(sources[0]).map_or(Coll::Binary, |i| i.coll_of(c)) {
                            Coll::Binary => format!("coalesce(a0.c{c}, a1.c{c})"),
                            k => format!("coalesce(a0.c{c}, a1.c{c}) COLLATE {}", k.name()),
                        },
                    });
                }
                cols.extend((0..columns[1]).filter(|c| !using.contains(c)).map(|c| format!("a1.c{c}")));
                cols.extend((2..n).map(|k| format!("{}.*", alias(0, k))));
                format!("SELECT {}{}", if q.distinct { "DISTINCT " } else { "" }, cols.join(", "))
            },
            _ => q.select_sql(0),
        };
        out.push(("plans-on", format!("{items} FROM {from}{}{rest}", q.where_sql(0))));
    }
    let with = q.with_sql();
    out.into_iter().map(|(name, sql)| (name, format!("{with}{sql}"))).collect()
}

/// The FULL JOIN identity of the plans oracle (see the module
/// documentation): for a plain two-source query `SELECT I FROM t0 AS a0
/// FULL JOIN t1 AS a1 ON e [WHERE p]` (no window, no compound) whose first
/// source has a key, `(SELECT I FROM t0 AS a0 LEFT JOIN t1 AS a1 ON e
/// WHERE p) UNION ALL (SELECT I FROM t1 AS a1 LEFT JOIN t0 AS a0 ON e
/// WHERE a0.k IS NULL AND p)`, the second part the rows of `t1` no row of
/// `t0` matches (`k` its rowid or a column of its primary key, never NULL
/// in a row), `*` written `a0.*, a1.*`. The query (without ORDER BY and
/// LIMIT) and the variant, or `None`.
fn full_union(q: &Query, schema: &Schema) -> Option<(String, String)> {
    let [j] = q.joins.as_slice() else {
        return None;
    };
    let JoinKind::Full(on) = &j.kind else {
        return None;
    };
    if !q.plain() || q.has_window() {
        return None;
    }
    let key = match &schema.info(q.table)?.key {
        RowKey::Rowid => "a0.rowid".to_string(),
        RowKey::Columns(pk) => format!("a0.c{}", pk.first()?),
        RowKey::None => return None,
    };
    let select = match &q.items {
        Items::Star => "SELECT a0.*, a1.*".to_string(),
        _ => q.select_sql(0),
    };
    let (t0, t1, on) = (format!("{} AS a0", table_name(q.table)), format!("{} AS a1", table_name(j.table)), q.e(on, 0));
    let p = q.filter.as_ref().map(|p| format!(" AND ({})", q.e(p, 0))).unwrap_or_default();
    let left = format!("{select} FROM {t0} LEFT JOIN {t1} ON {on}{}", q.where_sql(0));
    let right = format!("{select} FROM {t1} LEFT JOIN {t0} ON {on} WHERE ({key} IS NULL){p}");
    Some((q.core().sql(), format!("{}{left} UNION ALL {right}", q.with_sql())))
}

/// Differential query plans (see the module documentation).
pub fn plans(case: &Case, queries: &[Query]) -> Result<Tally, String> {
    each(case, queries, |db, m, schema, q, tally| {
        if ambiguous_groups(db, q, true) {
            tally.group_skips += 1;
            return Ok(());
        }
        let osql = q.sql();
        let Some(orig) = original(db, &osql, tally)? else {
            return Ok(());
        };
        let mut variants = vec![("plans-noindex", q.sql_not_indexed())];
        for (c, core) in q.cores().iter().enumerate() {
            for (k, t) in core.sources().iter().enumerate() {
                for (i, index) in m.tables.get(*t).map_or(&[][..], |t| &t.indexes[..]).iter().enumerate() {
                    let mut hints = vec![Vec::new(); c + 1];
                    hints[c] = vec![String::new(); k + 1];
                    hints[c][k] = format!(" INDEXED BY t{t}_i{i}");
                    let name = match index {
                        _ if index.filter.is_some() => "plans-index-partial",
                        _ if index.expr.is_some() => "plans-index-expr",
                        _ => "plans-index",
                    };
                    variants.push((name, q.sql_hinted(&hints)));
                }
            }
        }
        if q.compound.is_empty() {
            variants.extend(join_variants(q, schema));
        }
        if let Some((usql, vsql)) = full_union(q, schema) {
            if let Some(uorig) = original(db, &usql, tally)? {
                agree(tally, "plans-full-union", unordered_mode(q), &usql, &uorig, &vsql, &run(db, &vsql), &|_| false)?;
            }
        }
        for (name, vsql) in variants {
            let variant = run(db, &vsql);
            // A partial index the query's WHERE clause does not imply
            // (`whereUsablePartialIndex`), or an expression index on the
            // right of a RIGHT or FULL JOIN (never scanned there, see
            // `whereLoopAddBtree`), gives no plan to be forced to.
            let restricted = matches!(name, "plans-index-partial" | "plans-index-expr");
            if restricted && matches!(&variant, Err(e) if e.contains("no query solution")) {
                tally.hint_skips += 1;
                continue;
            }
            agree(tally, name, mode(q, schema), &osql, &orig, &vsql, &variant, &|_| sum_inputs(db, q, true))?;
        }
        let all = noopt_mask(db);
        noopt(tally, all, mode(q, schema), &osql, &orig, &|mask| run_without(db, &osql, mask), &|_| sum_inputs(db, q, true))
    })
}

/// The optimizations `plans-noopt` turns off on `db`: all, but Coroutines
/// while `PRAGMA reverse_unordered_selects` is on. Then SQLite (3.53.4 to
/// trunk 74a6182a) materializes the sorted input of a window function and
/// scans it in reverse, so computes the window over its rows in reverse
/// order (`SELECT c0, sum(c0) OVER (ORDER BY c0)` over 1, 2, 3 is 3|3,
/// 2|5, 1|6), a known SQLite bug (61 of
/// 20000 drawn inputs of the plans oracle, every one of them with that
/// pragma), see the test
/// `sqlite_reverses_materialized_window_input_without_coroutines`.
fn noopt_mask(db: &Connection) -> u32 {
    let coroutines = OPTIMIZATIONS.iter().find(|o| o.1 == "Coroutines").map_or(0, |o| 1u32 << (o.0 - 1));
    let reverse = db.query_row("PRAGMA reverse_unordered_selects", [], |r| r.get::<_, i64>(0)).unwrap_or(0) != 0;
    if reverse {
        ALL_OPTIMIZATIONS & !coroutines
    } else {
        ALL_OPTIMIZATIONS
    }
}

/// How a `plans-noopt` message names a mask of several optimizations:
/// `all`, or `all but X, Y` (those of [`OPTIMIZATIONS`] it leaves on).
fn mask_name(mask: u32) -> String {
    let on: Vec<&str> = OPTIMIZATIONS.iter().filter(|o| mask & (1u32 << (o.0 - 1)) == 0).map(|o| o.1).collect();
    if on.is_empty() {
        "all".to_string()
    } else {
        format!("all but {}", on.join(", "))
    }
}

/// The `plans-noopt` variant (see the module documentation): the query as
/// written, run with the optimizations of `all` off (every one but as
/// [`noopt_mask`] says; `run_with(mask)` runs it with those of `mask` off),
/// compared with `orig` as `agree` compares. On a disagreement, the query
/// is run with each optimization of `all` alone off: when exactly one
/// reproduces it, the failure is that run's, named
/// `plans-noopt-<optimization>` (so the runner's signatures tell culprits
/// apart); else it is the run with `all` off, named `plans-noopt`. The
/// message ends with the mask of the run it shows (`  optimizations off:
/// 0x00080000 (BloomFilter)`; `0xffffffff (all; ..)` says which
/// optimizations reproduce it alone, if any), for a replay.
fn noopt(
    tally: &mut Tally,
    all: u32,
    mode: Mode,
    osql: &str,
    orig: &Outcome,
    run_with: &dyn Fn(u32) -> Outcome,
    nan: &dyn Fn(bool) -> bool,
) -> Result<(), String> {
    let variant = run_with(all);
    let Err(message) = agree(tally, "plans-noopt", mode, osql, orig, osql, &variant, nan) else {
        return Ok(());
    };
    let several = mask_name(all);
    let mut culprits = Vec::new();
    for &(bit, name, variant) in &OPTIMIZATIONS {
        let mask = 1u32 << (bit - 1);
        if all & mask == 0 {
            continue;
        }
        if past_deadline() {
            // Every run from now on is interrupted, so tells nothing.
            return Err(format!("{message}\n  optimizations off: {all:#010x} ({several})"));
        }
        if let Err(m) = agree(&mut Tally::default(), variant, mode, osql, orig, osql, &run_with(mask), nan) {
            culprits.push((mask, name, m));
        }
    }
    Err(match culprits.as_slice() {
        [(mask, name, m)] => format!("{m}\n  optimizations off: {mask:#010x} ({name})"),
        [] => format!("{message}\n  optimizations off: {all:#010x} ({several}; none alone)"),
        culprits => {
            let names: Vec<String> = culprits.iter().map(|(mask, name, _)| format!("{name} {mask:#010x}")).collect();
            format!("{message}\n  optimizations off: {all:#010x} ({several}; alone, each of {})", names.join(", "))
        },
    })
}

/// Storage (see the module documentation): every query on the case's
/// database in memory ([`Storage::Memory`], the original) against the same
/// query on it in a WAL file ([`Storage::Wal`], the variant: in this
/// thread's WAL directory, or the system's temporary directory when the
/// thread's storage is memory), compared as the plans oracle compares a
/// query with its variants ([`mode`]: in order with a total ORDER BY, else
/// as the query's results are; GROUP BY queries with ambiguous groups
/// skipped). A side failing with the host's storage ([`io_error`]) skips
/// the comparison, and a WAL file that does not open so the input; both
/// counted in [`Tally::io_skips`]. A case whose statements run in memory
/// but not in the file (or the other way round) is a failure.
pub fn storage(case: &Case, queries: &[Query]) -> Result<Tally, String> {
    let dir = match thread_storage() {
        Storage::Wal { dir } => dir,
        Storage::Memory => std::env::temp_dir(),
    };
    let memory = open_with(case, &Storage::Memory);
    if matches!(&memory, Err(e) if nondeterministic(e)) {
        return Ok(Tally { nondeterministic_skips: 1, ..Tally::default() });
    }
    let file = match (open_with(case, &Storage::Wal { dir }), &memory) {
        (Err(e), _) if io_error(&e) => return Ok(Tally { io_skips: 1, ..Tally::default() }),
        (Ok(f), Ok(_)) => f,
        (Err(_), Err(e)) => return Err(e.clone()),
        (Err(e), Ok(_)) => return Err(format!("the case opens in memory but not in a WAL file: {e}")),
        (Ok(_), Err(e)) => return Err(format!("the case opens in a WAL file but not in memory: {e}")),
    };
    let memory = memory?;
    let functions_differ = wal_settings(case).reopen && case.statements.iter().any(registers_functions);
    each_on(&memory, case, queries, |db, _, schema, q, tally| {
        if ambiguous_groups(db, q, true) {
            tally.group_skips += 1;
            return Ok(());
        }
        if reads_layout(q) || functions_differ && reads_function_list(q) {
            tally.inapplicable += 1;
            return Ok(());
        }
        let osql = q.sql();
        let (orig, variant) = (run(db, &osql), run(&file, &osql));
        if [&orig, &variant].iter().any(|o| matches!(o, Err(e) if io_error(e))) {
            tally.io_skips += 1;
            return Ok(());
        }
        let Some(orig) = judge_original(orig, &osql, tally)? else {
            return Ok(());
        };
        agree(tally, "storage", mode(q, schema), &osql, &orig, &osql, &variant, &|_| sum_inputs(db, q, true))
    })
}

/// Whether `q` reads the database's layout rather than its rows: `dbstat`,
/// or `pragma_page_count` / `pragma_freelist_count`, whose values the
/// storage oracle's two databases need not share (the WAL file is written
/// anew: with no table in the case it holds no page yet, so `dbstat` has
/// no row where the deserialized copy has page 1). Found as run
/// 20261016-2113 storage, `gen+wal` and `novel+wal`. And
/// `pragma_table_list`, whose `ncol` of a view is the connection's: it
/// computes the views' columns in its schema hash's order, which follows
/// the order the connection read the schema in, and a view that does not
/// prepare (its table dropped) keeps it from reaching the views after it,
/// whose `ncol` stays 0 unless a query read them (an SQLite quirk). Found as run
/// 20261017-0438 storage w13-317839 (`gen+wal`: v0 of 1 column on one
/// connection, 0 on the other). Every other virtual-table form the
/// generator draws (`draw_vtab`: the table, index and foreign key pragmas,
/// `sqlite_schema`, the PRAGMAS but `page_count` and `freelist_count`)
/// gave the same rows on both connections over the cases of 1500 inputs,
/// and on one connection before and after the inputs' queries.
fn reads_layout(q: &Query) -> bool {
    q.with.iter().any(|c| match &c.body {
        CteBody::Vtab(Vtab::Dbstat { .. } | Vtab::TableList) => true,
        CteBody::Vtab(Vtab::Pragma { k, .. }) => matches!(PRAGMAS[*k % PRAGMAS.len()].0, "page_count" | "freelist_count"),
        _ => false,
    })
}

/// Whether statement `s` registers SQL functions on its connection:
/// `PRAGMA case_sensitive_like` (a read-back of [`Statement::Synchronous`])
/// registers `like` and `glob` again (`sqlite3RegisterLikeFunctions`), as
/// functions of the connection, and `pragma_function_list` lists them
/// besides the built-in ones (`like` of 2 and 3 arguments, `builtin` 0);
/// a connection that did not run it lists only the built-in ones.
fn registers_functions(s: &Statement) -> bool {
    s.to_string().contains("case_sensitive_like")
}

/// Whether `q` reads `pragma_function_list`, the functions of the
/// connection: the storage oracle leaves it out when the WAL file's
/// connection is a new one ([`WalSettings::reopen`]) and the case's
/// statements registered functions on the first ([`registers_functions`]),
/// which the memory database's connection keeps and the new one has not.
/// Found as run 20261022-0422 storage w13-4647 (`gen+wal`: `PRAGMA
/// synchronous = EXTRA; ..; PRAGMA case_sensitive_like = 0; ..` on an empty
/// file, two `like` rows more in memory).
fn reads_function_list(q: &Query) -> bool {
    q.with.iter().any(|c| matches!(&c.body, CteBody::Vtab(Vtab::Pragma { k, .. }) if PRAGMAS[*k % PRAGMAS.len()].0 == "function_list"))
}

/// SQLite's order of two result values (BINARY collation, UTF-8 text):
/// NULL, numbers by value, text by bytes, blobs by bytes.
pub fn cmp_cells(a: &Cell, b: &Cell) -> Ordering {
    let rank = |c: &Cell| match c {
        Cell::Null => 0,
        Cell::Int(_) | Cell::Real(_) => 1,
        Cell::Text(_) => 2,
        Cell::Blob(_) | Cell::Jsonb(..) => 3,
    };
    match (a, b) {
        (Cell::Int(x), Cell::Int(y)) => x.cmp(y),
        (Cell::Real(x), Cell::Real(y)) => f64::from_bits(*x).partial_cmp(&f64::from_bits(*y)).unwrap_or(Ordering::Equal),
        (Cell::Int(x), Cell::Real(y)) => int_real(*x, f64::from_bits(*y)),
        (Cell::Real(x), Cell::Int(y)) => int_real(*y, f64::from_bits(*x)).reverse(),
        (Cell::Text(x), Cell::Text(y))
        | (Cell::Blob(x) | Cell::Jsonb(_, Raw(x)), Cell::Blob(y) | Cell::Jsonb(_, Raw(y))) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// [`cmp_cells`] with two texts under `coll` ([`Coll::key`]).
fn cmp_cells_coll(a: &Cell, b: &Cell, coll: Coll) -> Ordering {
    match (a, b) {
        (Cell::Text(x), Cell::Text(y)) => coll.key(x).cmp(&coll.key(y)),
        _ => cmp_cells(a, b),
    }
}

/// Two rows' order under ORDER BY terms `terms`, each the column of the
/// rows holding its key and the term.
fn cmp_rows(a: &[Cell], b: &[Cell], terms: &[(usize, &Order)]) -> Ordering {
    cmp_rows_coll(a, b, terms, &[])
}

/// [`cmp_rows`] with each term's text under its collating sequence
/// (`colls`, by term; BINARY past its end).
fn cmp_rows_coll(a: &[Cell], b: &[Cell], terms: &[(usize, &Order)], colls: &[Coll]) -> Ordering {
    for (i, (k, o)) in terms.iter().enumerate() {
        let coll = colls.get(i).copied().unwrap_or(Coll::Binary);
        let x = match (&a[*k], &b[*k]) {
            (Cell::Null, Cell::Null) => Ordering::Equal,
            (Cell::Null, _) => if o.nulls_first() { Ordering::Less } else { Ordering::Greater },
            (_, Cell::Null) => if o.nulls_first() { Ordering::Greater } else { Ordering::Less },
            (x, y) if o.desc => cmp_cells_coll(x, y, coll).reverse(),
            (x, y) => cmp_cells_coll(x, y, coll),
        };
        if x != Ordering::Equal {
            return x;
        }
    }
    Ordering::Equal
}

/// The collating sequence of each ORDER BY term of a query that is not a
/// compound (`crate::expr::expr_coll`: a result column's expression's, a
/// rowid's BINARY), `None` where the model does not tell (a compound, `*`
/// over a join).
fn term_colls(q: &Query, schema: &Schema) -> Option<Vec<Coll>> {
    if !q.compound.is_empty() {
        return None;
    }
    let levels = [q.sources()];
    q.order
        .iter()
        .map(|o| match &o.key {
            OrderKey::Rowid(_) => Some(Coll::Binary),
            OrderKey::Expr(e) => Some(crate::expr::expr_coll(e, schema, &levels)),
            OrderKey::Column(k) => match &q.items {
                Items::Exprs(items) => items.get(*k).map(|e| crate::expr::expr_coll(e, schema, &levels)),
                Items::Star if q.joins.is_empty() => schema.info(levels[0][0]).map(|i| i.coll_of(*k)),
                _ => None,
            },
        })
        .collect()
}

/// ORDER BY and LIMIT (see the module documentation): for a query `Q
/// ORDER BY o [LIMIT n OFFSET k]` with a total order `o`, `R` the rows of
/// `Q ORDER BY o`:
///
/// - `order-multiset`: `R` against `Q`'s rows (no ORDER BY), compared as
///   `Q`'s are;
/// - `order-limit`: `Q ORDER BY o LIMIT n OFFSET k` against rows `k..k +
///   n` of `R`, in order;
/// - `order-sorted` (UTF-8 databases): `SELECT I, x1, .. FROM ..`, with
///   the keys `xi` of the terms that are not result columns appended as
///   columns, against `R` in its first columns, in order; and its rows in
///   order by the terms, as [`cmp_rows`] orders them (strictly, without
///   ties, when the order is by every source's key).
///
/// A query `Q` without ORDER BY is checked so as `Q ORDER BY o [LIMIT n
/// OFFSET k]`, `o` every result column ([`added_order`], a total order),
/// the variants named `order-all-multiset` (`Q` itself against `R`, one
/// time in eight, or where the order is not checked), `order-all-limit`
/// and `order-all-sorted`.
pub fn order(case: &Case, queries: &[Query]) -> Result<Tally, String> {
    each(case, queries, |db, m, schema, q, tally| {
        let Some(exact) = q.total_order(schema) else {
            // Without ORDER BY: the query ordered by every result column,
            // a total order ([`added_order`]).
            let h = fnv(&q.sql());
            if thinned(h, tally) {
                return Ok(());
            }
            let Some(o) = added_order(q, schema, h) else {
                tally.inapplicable += 1;
                return Ok(());
            };
            if ambiguous_groups(db, q, true) {
                tally.group_skips += 1;
                return Ok(());
            }
            // Its rows against the query's own one time in eight, or where
            // their order is not checked.
            let full = (h >> 44).is_multiple_of(8);
            return order_on(db, m, schema, &o, false, ORDER_ADDED, full, tally);
        };
        if ambiguous_groups(db, q, true) {
            tally.group_skips += 1;
            return Ok(());
        }
        order_on(db, m, schema, q, exact, ORDER_WRITTEN, true, tally)
    })
}

/// The variant names of [`order`] for a query's own ORDER BY.
const ORDER_WRITTEN: [&str; 3] = ["order-multiset", "order-limit", "order-sorted"];
/// The variant names of [`order`] for the ORDER BY of [`added_order`].
const ORDER_ADDED: [&str; 3] = ["order-all-multiset", "order-all-limit", "order-all-sorted"];

/// A query without ORDER BY, `Q`, ordered by every result column (by its
/// number, in an order rotated by a draw), each term ASC or DESC with
/// NULLS FIRST, NULLS LAST or neither, one time in four with a LIMIT of up
/// to 4 rows and an OFFSET of up to 2: a total order
/// ([`Query::total_order`] is `Some(false)`: rows that tie are equal as
/// SQLite compares them) of a form the generator writes, so that
/// [`order`] checks it as it checks the query's own. Each draw is from
/// `h`, [`fnv`] of `Q`'s text (deterministic). `None` for a query with an
/// ORDER BY.
fn added_order(q: &Query, schema: &Schema, h: u64) -> Option<Query> {
    if !q.order.is_empty() {
        return None;
    }
    let n = q.width(schema);
    if n == 0 {
        return None;
    }
    let start = (h % n as u64) as usize;
    let order = (0..n)
        .map(|i| {
            let bits = h.rotate_right((3 * i % 61) as u32 + 3);
            let nulls = match (bits >> 1) % 4 {
                0 => Some(true),
                1 => Some(false),
                _ => None,
            };
            Order { key: OrderKey::Column((start + i) % n), desc: bits % 2 == 1, nulls, by: By::Number }
        })
        .collect();
    let limit = (h >> 61).is_multiple_of(4).then_some(Limit { count: ((h >> 32) % 5) as i64, offset: ((h >> 40) % 3) as i64, comma: false });
    let o = Query { order, limit, ..q.clone() };
    (o.total_order(schema) == Some(false)).then_some(o)
}

/// [`order`] on `q`, whose ORDER BY is a total order (`exact` as
/// [`Query::total_order`] says), its variants named `names`; the
/// multiset only when `full` or where the order is not checked (a UTF-16
/// database, terms whose collating sequences the model does not tell).
#[allow(clippy::too_many_arguments)]
fn order_on(
    db: &Connection,
    m: &Model,
    schema: &Schema,
    q: &Query,
    exact: bool,
    names: [&'static str; 3],
    full_check: bool,
    tally: &mut Tally,
) -> Result<(), String> {
    let [multiset, limited, sorted_name] = names;
    let full = Query { limit: None, ..q.clone() };
    let fsql = full.sql();
    let Some(rows) = original(db, &fsql, tally)? else {
        return Ok(());
    };
    let unordered = Query { order: Vec::new(), ..full.clone() };
    let usql = unordered.sql();
    // A compound SQLite computes in parts under sequences other than
    // its own, which ORDER BY changes (`steps_share_colls`), gives other
    // rows with it than without: known, as setops-def skips it.
    let sorted_checked = m.encoding == Encoding::Utf8 && (!folding() || term_colls(q, schema).is_some());
    if folding() && !q.compound.is_empty() && !(steps_share_colls(&full, schema) && steps_share_colls(&unordered, schema)) {
        tally.coll_skips += 1;
    } else if full_check || !sorted_checked {
        agree(tally, multiset, unordered_mode(q), &fsql, &rows, &usql, &run(db, &usql), &|_| false)?;
    }
    let mode = Mode::Ordered(exact);
    if let Some(l) = q.limit {
        let lsql = q.sql();
        let expected = rows.as_ref().map(|r| l.slice(r)).map_err(|e| e.clone());
        let what = format!("{fsql} (its rows {}.., {} of them)", l.offset.max(0), l.count);
        agree(tally, limited, mode, &what, &expected, &lsql, &run(db, &lsql), &|_| false)?;
    }
    if m.encoding != Encoding::Utf8 {
        return Ok(());
    }
    // Under NOCASE or RTRIM, each term sorts under its own sequence.
    let colls = if folding() { term_colls(q, schema) } else { Some(vec![Coll::Binary; q.order.len()]) };
    let Some(colls) = colls else {
        return Ok(());
    };
    let width = q.width(schema);
    let mut extra = Vec::new();
    let mut terms = Vec::new();
    for o in &q.order {
        match o.key {
            OrderKey::Column(c) => terms.push((c, o)),
            _ => {
                terms.push((width + extra.len(), o));
                extra.push(q.key_sql(&o.key));
            },
        }
    }
    let ssql = if extra.is_empty() {
        fsql.clone()
    } else {
        let (w, s) = (q.with_sql(), q.select_sql(0));
        format!("{w}{s}, {} FROM {}{}{}", extra.join(", "), q.from_sql(0, &[]), q.where_sql(0), full.tail_sql(0))
    };
    // Without keys appended, the query is the one already run.
    let sorted = if extra.is_empty() { rows.clone() } else { run(db, &ssql) };
    let prefix = sorted.as_ref().map(|r| r.iter().map(|row| row[..width.min(row.len())].to_vec()).collect()).map_err(|e| e.clone());
    agree(tally, sorted_name, mode, &fsql, &rows, &ssql, &prefix, &|_| false)?;
    if let Ok(r) = &sorted {
        for (k, w) in r.windows(2).enumerate() {
            let o = cmp_rows_coll(&w[0], &w[1], &terms, &colls);
            if o == Ordering::Greater || (exact && o == Ordering::Equal) {
                return Err(format!(
                    "rows {k} and {} out of order ({sorted_name})\n  query: {ssql}\n    -> {:?}\n    -> {:?}",
                    k + 1,
                    w[0],
                    w[1]
                ));
            }
        }
    }
    Ok(())
}

/// `left op right` by the definition of `op`, on rows compared exactly
/// (pass rows as SQLite compares them, [`equiv_rows`], for SQLite's
/// equality): UNION ALL concatenates; UNION is the distinct rows of both;
/// INTERSECT the distinct rows of `left` also in `right`; EXCEPT those not
/// in `right`.
pub fn set_op(op: SetOp, left: Rows, right: Rows) -> Rows {
    // SQLite tells rows apart by their bytes: two JSONB encodings of one
    // value are two rows ([`Cell::Jsonb`] compares them alike).
    let bytes = |r: &[Cell]| -> Vec<Cell> { r.iter().map(|c| if let Cell::Jsonb(_, Raw(b)) = c { Cell::Blob(b.clone()) } else { c.clone() }).collect() };
    let distinct = |rows: Rows| {
        let mut seen = BTreeSet::new();
        rows.into_iter().filter(|r| seen.insert(bytes(r))).collect::<Rows>()
    };
    match op {
        SetOp::UnionAll => [left, right].concat(),
        SetOp::Union => distinct([left, right].concat()),
        SetOp::Intersect | SetOp::Except => {
            let right: BTreeSet<Vec<Cell>> = right.iter().map(|r| bytes(r)).collect();
            distinct(left).into_iter().filter(|r| right.contains(&bytes(r)) == (op == SetOp::Intersect)).collect()
        },
    }
}

/// Set operations (see the module documentation), for compound queries `S0
/// op1 S1 .. [ORDER BY o [LIMIT ..]]`:
///
/// - `setops-def` (UTF-8 databases): each SELECT `Si` run alone, the
///   compound computed from their rows by [`set_op`], left to right (on
///   rows as SQLite compares values unless every operator is UNION ALL
///   and every SELECT compares exactly), then sorted by `o` ([`cmp_rows`])
///   and cut by the LIMIT; against the compound;
/// - `setops-swap`: two SELECTs and a commutative operator (not EXCEPT):
///   `S1 op S0 [ORDER BY o [LIMIT ..]]` against `S0 op S1 ..`.
///
/// A query that is not a compound is checked as [`setops_self`] says.
pub fn setops(case: &Case, queries: &[Query]) -> Result<Tally, String> {
    each(case, queries, |db, m, schema, q, tally| {
        if q.compound.is_empty() {
            return setops_self(db, schema, q, tally);
        }
        if ambiguous_groups(db, q, true) {
            tally.group_skips += 1;
            return Ok(());
        }
        let osql = q.sql();
        let Some(orig) = original(db, &osql, tally)? else {
            return Ok(());
        };
        let mode = mode(q, schema);
        if let [(op, member)] = q.compound.as_slice() {
            // Swapped, the compound takes its collating sequences from the
            // other member first (`multiSelectCollSeq`): compared only where
            // they are the same either way.
            if *op != SetOp::Except && (!folding() || swap_keeps_colls(&q.core(), member, schema)) {
                let first = Query { with: Vec::new(), ..q.core() };
                let (order, limit) = (q.order.clone(), q.limit);
                let swapped = Query { with: q.with.clone(), compound: vec![(*op, first)], order, limit, ..member.clone() };
                let ssql = swapped.sql();
                agree(tally, "setops-swap", mode, &osql, &orig, &ssql, &run(db, &ssql), &|_| false)?;
            }
        }
        if m.encoding != Encoding::Utf8 {
            return Ok(());
        }
        // Under NOCASE or RTRIM, each column of the compound tells rows
        // apart, and sorts, under its own sequence.
        let colls = if folding() { compound_colls(q, schema) } else { Some(vec![Coll::Binary; q.width(schema)]) };
        let Some(colls) = colls else {
            return Ok(());
        };
        if folding() && !steps_share_colls(q, schema) {
            tally.coll_skips += 1;
            return Ok(());
        }
        let equiv = unordered_mode(q) != Mode::Multiset;
        let mut results = Vec::new();
        for c in q.cores() {
            match run(db, &c.sql()) {
                Ok(r) => results.push(if equiv { equiv_rows(&r) } else { r }),
                Err(e) if skip(tally, &e).is_some() => return Ok(()),
                Err(e) => return Err(format!("a SELECT of the compound fails: {}\n  -> {e}", c.sql())),
            }
        }
        let mut results = results.into_iter();
        let mut rows = results.next().unwrap_or_default();
        for ((op, _), r) in q.compound.iter().zip(results) {
            rows = set_op_coll(*op, rows, r, &colls);
        }
        if !q.order.is_empty() {
            let terms: Vec<(usize, &Order)> =
                q.order.iter().filter_map(|o| if let OrderKey::Column(c) = o.key { Some((c, o)) } else { None }).collect();
            let term_colls: Vec<Coll> = terms.iter().map(|(c, _)| colls.get(*c).copied().unwrap_or(Coll::Binary)).collect();
            rows.sort_by(|a, b| cmp_rows_coll(a, b, &terms, &term_colls));
            if let Some(l) = q.limit {
                rows = l.slice(&rows);
            }
        }
        let what = format!("{osql} (by definition, from its SELECTs' rows)");
        agree(tally, "setops-def", mode, &what, &Ok(rows), &osql, &orig, &|_| false)
    })
}

/// `setops-self`, for a query `Q` that is not a compound: its SELECT
/// (without ORDER BY and LIMIT), `S`, as both members of a compound `S op
/// S`, `op` drawn from [`fnv`] of `Q`'s text (deterministic), half of the
/// time ordered by every result column (not under NOCASE or RTRIM, where
/// ORDER BY changes how SQLite computes a compound, see [`order`]); seven
/// times in eight its rows against themselves (`setops-self-own`,
/// [`self_compound_rows`]), else against `S`'s rows `R`: `R` twice for UNION ALL (compared as a
/// compound of `S` is, [`unordered_mode`]), `R` as a set for UNION and
/// INTERSECT (each row of either member is in the other), none for
/// EXCEPT (each is in the right member). `S` is deterministic as any
/// query compared with itself (ambiguous groups skipped, counted in
/// [`Tally::group_skips`]), so its two runs give the same rows. One
/// aggregate `sum`, whose REAL sum may round apart by the order its rows
/// are summed in, is counted in [`Tally::inapplicable`].
fn setops_self(db: &Connection, schema: &Schema, q: &Query, tally: &mut Tally) -> Result<(), String> {
    let h = fnv(&q.sql());
    if thinned(h, tally) {
        return Ok(());
    }
    let core = q.core();
    if unordered_mode(&core) == Mode::Sum {
        tally.inapplicable += 1;
        return Ok(());
    }
    if ambiguous_groups(db, q, true) {
        tally.group_skips += 1;
        return Ok(());
    }
    let osql = core.sql();
    let op = [SetOp::UnionAll, SetOp::Union, SetOp::Intersect, SetOp::Except][(h % 4) as usize];
    let order = match (h >> 8).is_multiple_of(2) && !folding() {
        true => {
            let terms: Vec<String> = (0..q.width(schema)).map(|k| format!("{}{}", k + 1, if (h >> (16 + k % 40)) % 2 == 1 { " DESC" } else { "" })).collect();
            format!(" ORDER BY {}", terms.join(", "))
        },
        false => String::new(),
    };
    // Written out: `Query::sql` puts a compound's WINDOW clause once, at
    // its end, where it names the last member's windows; each member
    // here has its own.
    let with = core.with_sql();
    let Some(s) = osql.strip_prefix(&with) else {
        return Err(format!("the query does not start with its WITH clause: {osql}"));
    };
    let vsql = format!("{with}{s} {} {s}{order}", op.sql());
    // Seven times in eight, the compound alone: its rows against
    // themselves.
    if !(h >> 44).is_multiple_of(8) {
        let Some(Ok(rows)) = original(db, &vsql, tally)? else {
            return Ok(());
        };
        return self_compound_rows(tally, op, &vsql, &rows);
    }
    let Some(orig) = original(db, &osql, tally)? else {
        return Ok(());
    };
    let (mode, expected) = match (op, &orig) {
        (SetOp::UnionAll, Ok(r)) => (unordered_mode(&Query { compound: vec![(op, core.clone())], ..core.clone() }), Ok([r.clone(), r.clone()].concat())),
        (SetOp::Except, Ok(_)) => (Mode::Set, Ok(Vec::new())),
        _ => (Mode::Set, orig.clone()),
    };
    let what = format!("{osql} (its rows, as {op:?} of it with itself has them)");
    agree(tally, "setops-self", mode, &what, &expected, &vsql, &run(db, &vsql), &|_| false)
}

/// `setops-self-own`: the rows of `S op S` ([`setops_self`]) against
/// themselves: under UNION ALL each row as often as an equal one in the
/// other member, so every class of rows SQLite cannot tell apart (folded
/// where the query is compared folded, then [`Cell::distinct_key`]) an
/// even number of times; under UNION and INTERSECT no two rows SQLite
/// tells apart under no collating sequence ([`Cell::distinct_key`], equal
/// under every one); under EXCEPT none.
fn self_compound_rows(tally: &mut Tally, op: SetOp, vsql: &str, rows: &Rows) -> Result<(), String> {
    tally.comparisons += 1;
    *tally.variants.entry("setops-self-own").or_insert(0) += 1;
    if nul_text(rows) {
        tally.nul_skips += 1;
        return Ok(());
    }
    let fold = folding();
    let key = |r: &[Cell]| -> Vec<Cell> { r.iter().map(|c| if fold { c.folded().distinct_key() } else { c.distinct_key() }).collect() };
    let mut classes: BTreeMap<Vec<Cell>, usize> = BTreeMap::new();
    for r in rows {
        *classes.entry(if op == SetOp::UnionAll { key(r) } else { r.iter().map(Cell::distinct_key).collect() }).or_insert(0) += 1;
    }
    let odd = classes.iter().find(|(_, &n)| match op {
        SetOp::UnionAll => n % 2 == 1,
        SetOp::Union | SetOp::Intersect => n > 1,
        SetOp::Except => true,
    });
    match odd {
        Some((row, n)) => Err(format!("{op:?} of a query with itself gives {row:?} {n} times (setops-self-own)\n  query: {vsql}\n    -> {rows:?}")),
        None => Ok(()),
    }
}

/// [`set_op`] with each column's text told apart under its collating
/// sequence (`colls`, by its key, [`Coll::key`]); the first of equal rows
/// is kept, which a comparison folded, as under such sequences, takes for
/// whichever SQLite keeps.
pub fn set_op_coll(op: SetOp, left: Rows, right: Rows, colls: &[Coll]) -> Rows {
    if colls.iter().all(|k| *k == Coll::Binary) {
        return set_op(op, left, right);
    }
    let key = |r: &[Cell]| -> Vec<Cell> {
        r.iter()
            .enumerate()
            .map(|(k, c)| match (c, colls.get(k).copied().unwrap_or(Coll::Binary)) {
                (Cell::Jsonb(_, Raw(b)), _) => Cell::Blob(b.clone()),
                (Cell::Text(t), coll) => Cell::Text(coll.key(t).into_owned()),
                (c, _) => c.clone(),
            })
            .collect()
    };
    let distinct = |rows: Rows| {
        let mut seen = BTreeSet::new();
        rows.into_iter().filter(|r| seen.insert(key(r))).collect::<Rows>()
    };
    match op {
        SetOp::UnionAll => [left, right].concat(),
        SetOp::Union => distinct([left, right].concat()),
        SetOp::Intersect | SetOp::Except => {
            let right: BTreeSet<Vec<Cell>> = right.iter().map(|r| key(r)).collect();
            distinct(left).into_iter().filter(|r| right.contains(&key(r)) == (op == SetOp::Intersect)).collect()
        },
    }
}

/// The collating sequence each column of a compound takes (from its first
/// member whose item has one, BINARY when none has; `multiSelectCollSeq`),
/// `None` where the model does not tell ([`member_colls`]).
fn compound_colls(q: &Query, schema: &Schema) -> Option<Vec<Coll>> {
    let members = members_colls(q, schema)?;
    Some(first_colls(&members))
}

/// Each member's [`member_colls`], all of one width; `None` where the
/// model does not tell.
fn members_colls(q: &Query, schema: &Schema) -> Option<Vec<Vec<Option<Coll>>>> {
    let members: Vec<Vec<Option<Coll>>> = q.cores().iter().map(|c| member_colls(c, schema)).collect::<Option<_>>()?;
    let width = members.first()?.len();
    members.iter().all(|m| m.len() == width).then_some(members)
}

/// The sequence of each column of a compound of `members`: the first
/// member's that has one, BINARY when none has.
fn first_colls(members: &[Vec<Option<Coll>>]) -> Vec<Coll> {
    let width = members.first().map_or(0, Vec::len);
    (0..width).map(|k| members.iter().find_map(|m| m[k]).unwrap_or(Coll::Binary)).collect()
}

/// Whether SQLite tells rows of compound `q` apart, and matches them,
/// under the compound's sequences ([`compound_colls`]) at every step: false
/// where it computes a first part of the members, `S0 op1 .. opk Sk` with
/// `opk` not UNION ALL, as a compound of its own whose duplicates it tells
/// apart under the sequences of those members alone (`multiSelectCollSeq`
/// of the part, which its `pKeyDup` and its invented ORDER BY terms take),
/// and they are not the whole compound's. SQLite computes `S0 op1 .. opn
/// Sn` as a merge (`multiSelectByMerge`, an ORDER BY of every column
/// invented where the compound has none and `opn` is not UNION ALL) of a
/// left part and a right one, each computed alike with the ORDER BY copied
/// (its terms under the whole compound's sequences): the right part `Sn`
/// alone, or for a run of four or more of one operator at the end (UNION or
/// UNION ALL, the balanced merge) its second half. Without ORDER BY and with
/// `opn` UNION ALL, it runs `S0 .. op(n-1) S(n-1)` as a compound of its
/// own, with its own sequences, then `Sn`. So `c || '' UNION NULL UNION ALL
/// 'x' COLLATE NOCASE` keeps both 'a' and 'A' of `c` (its UNION under
/// BINARY), and with `ORDER BY 1` the UNION's two sides are merged under
/// NOCASE but the duplicates of one side told apart under BINARY: neither
/// is the compound's NOCASE (nor are the rows the same with and without the
/// ORDER BY, as in SQLite 3.51.0, which used NOCASE without it).
/// A right part that SQLite computes under its own members' sequences (a
/// balanced merge's second half) is not the model's but an SQLite bug of
/// trunk (3.51.0 used the compound's), compared as the compound's:
/// case-studies/sqlite/bugs/sqlite-balanced-merge-dedup-collation.
fn steps_share_colls(q: &Query, schema: &Schema) -> bool {
    let Some(members) = members_colls(q, schema) else {
        return false;
    };
    let whole = first_colls(&members);
    // `ops[k]`: the operator before member k (`ops[0]` unused).
    let ops: Vec<SetOp> = std::iter::once(SetOp::UnionAll).chain(q.compound.iter().map(|(op, _)| *op)).collect();
    // The first part `S0 .. S{hi}`, computed by a merge or not.
    let (mut hi, mut merge) = (members.len() - 1, !q.order.is_empty());
    while hi > 0 {
        let op = ops[hi];
        if !merge && op == SetOp::UnionAll {
            hi -= 1;
            continue;
        }
        if op != SetOp::UnionAll && first_colls(&members[..=hi]) != whole {
            return false;
        }
        // `pSplit`: the right part starts at `split`.
        let mut n = 1;
        if matches!(op, SetOp::Union | SetOp::UnionAll) {
            while hi + 1 > n && ops[hi + 1 - n] == op {
                n += 1;
            }
        }
        hi = if n <= 3 { hi - 1 } else { hi - (n - 1) / 2 - 1 };
        merge = true;
    }
    true
}

/// The collating sequence a compound may take from each item of a member
/// ([`crate::expr::member_coll`]), `None` where the model does not tell
/// (`*` over a join, a subquery item). A source the member reads through a
/// subquery in FROM ([`Query::wrapped`]) has a sequence for each column.
fn member_colls(q: &Query, schema: &Schema) -> Option<Vec<Option<Coll>>> {
    let levels = [q.sources()];
    let wrapped = q.wrapped(0);
    match &q.items {
        Items::Exprs(items) => items.iter().map(|e| crate::expr::member_coll(e, schema, &levels, wrapped)).collect(),
        // `*` of one source: its columns (an INTEGER PRIMARY KEY with none,
        // unless read through a subquery).
        Items::Star if q.joins.is_empty() => {
            let info = schema.info(levels[0][0])?;
            Some((0..info.columns).map(|c| if wrapped & 1 == 1 { Some(info.coll_of(c)) } else { info.seq_of(c) }).collect())
        },
        Items::Agg(a, arg) => {
            let e = Expr::Agg(*a, arg.clone().map(Box::new));
            Some(vec![crate::expr::member_coll(&e, schema, &levels, wrapped)?])
        },
        Items::Star => None,
    }
}

/// Whether a compound of `a` and `b` takes the same collating sequence for
/// every column as one of `b` and `a` ([`crate::expr::member_coll`]); false
/// where the model does not tell (items that are not expressions, a
/// subquery).
fn swap_keeps_colls(a: &Query, b: &Query, schema: &Schema) -> bool {
    // A column with no declared sequence still has one, BINARY, and as the
    // leftmost member's it decides the compound's (only an expression with
    // none, a literal, defers to the next member): swapped members are
    // compared only where both give the same sequence, a missing one read
    // as BINARY (run 20261016-0317, setops-swap w6-47: a CTE's column
    // first sorted BINARY, t0's `c0 COLLATE RTRIM` first sorted RTRIM, which
    // order '0' and '中cc' differently in a UTF-16 database).
    match (member_colls(a, schema), member_colls(b, schema)) {
        (Some(x), Some(y)) if x.len() == y.len() => x.iter().zip(&y).all(|(p, q)| p.unwrap_or(Coll::Binary) == q.unwrap_or(Coll::Binary)),
        _ => false,
    }
}

/// Common table expressions (see the module documentation), for queries
/// `WITH x0 AS (..), .. Q` as drawn, compared as `Q`'s results are:
///
/// - `cte-inline`: every reference to a CTE replaced by its query as a
///   subquery in FROM (a recursive one's integers as a UNION ALL of
///   literals), no WITH clause ([`Query::sql_inline`]);
/// - `cte-materialized`, `cte-not-materialized`: every CTE `MATERIALIZED`,
///   every CTE `NOT MATERIALIZED`.
///
/// Queries without a WITH clause are counted in [`Tally::inapplicable`].
pub fn ctes(case: &Case, queries: &[Query]) -> Result<Tally, String> {
    each(case, queries, |db, _, schema, q, tally| {
        if q.with.is_empty() {
            tally.inapplicable += 1;
            return Ok(());
        }
        if ambiguous_groups(db, q, true) {
            tally.group_skips += 1;
            return Ok(());
        }
        let osql = q.sql();
        let Some(orig) = original(db, &osql, tally)? else {
            return Ok(());
        };
        let mode = mode(q, schema);
        let hinted = |h: bool| {
            let mut v = q.clone();
            for c in &mut v.with {
                c.hint = Some(h);
            }
            v.sql()
        };
        for (name, vsql) in [("cte-inline", q.sql_inline()), ("cte-materialized", hinted(true)), ("cte-not-materialized", hinted(false))] {
            agree(tally, name, mode, &osql, &orig, &vsql, &run(db, &vsql), &|_| sum_inputs(db, q, true))?;
        }
        Ok(())
    })
}

/// A row of a window's input: its PARTITION BY values, its ORDER BY
/// values, its argument (`None` without one; for `group_concat` as TEXT),
/// its default (`lag` and `lead`), and whether its FILTER holds (true
/// without one).
#[derive(Clone, Debug)]
pub struct WindowRow {
    pub partition: Vec<Cell>,
    pub order: Vec<Cell>,
    pub arg: Option<Cell>,
    pub default: Option<Cell>,
    pub filter: bool,
}

impl WindowRow {
    pub fn new(partition: Vec<Cell>, order: Vec<Cell>, arg: Option<Cell>) -> WindowRow {
        WindowRow { partition, order, arg, default: None, filter: true }
    }
}

/// The values of window `w` over `rows`, in their order, by the definition
/// of window functions (UTF-8 text, as [`cmp_cells`]): rows in one
/// partition when their PARTITION BY values are equal as SQLite compares
/// values; in a partition, ordered by the ORDER BY terms ([`cmp_rows`]),
/// peers when equal in all of them (all are, without ORDER BY);
/// `row_number` the position, `rank` 1 + the rows before the peers,
/// `dense_rank` 1 + the peer groups before, `percent_rank` (rank - 1) /
/// (rows - 1) (0.0 for one row), `cume_dist` the rows up to the last peer
/// / rows, `ntile(n)` the bucket of the position (the first rows % n
/// buckets one row larger); `lag` and `lead` the argument `n` rows before
/// or after, else the default (NULL without one). The others over the
/// frame: by default from the partition's start to the last peer (without
/// ORDER BY, the partition); ROWS by position, GROUPS by peer group, RANGE
/// to and from the peers, or with offsets to the first and last rows whose
/// (integer) value is within the offset of the current row's (a NULL
/// row's bounds its NULL peers; NULLs before every value when they sort
/// first, after when last); less the rows
/// EXCLUDE takes. `first_value`, `last_value`, `nth_value` the argument of
/// that row of the frame (NULL without it); the aggregates of the frame's
/// rows whose FILTER holds: `count` counts rows (`count(*)`) or non-NULL
/// values; `sum` adds integers (NULL without any; an error when the sum
/// overflows); `total` and `avg` the integer sum as a real (`total` 0.0
/// without any, `avg` divided by the count, NULL without any); `min` and
/// `max` of non-NULL values (NULL without any); `group_concat` the non-NULL
/// texts in order with the separator between (NULL without any);
/// `json_group_array` the integers and NULLs as a JSON array.
pub fn window_values(w: &Window, rows: &[WindowRow]) -> Result<Vec<Cell>, String> {
    let mut parts: BTreeMap<Vec<Cell>, Vec<usize>> = BTreeMap::new();
    for (i, r) in rows.iter().enumerate() {
        parts.entry(r.partition.iter().map(Cell::distinct_key).collect()).or_default().push(i);
    }
    let terms: Vec<(usize, &Order)> = w.order.iter().enumerate().collect();
    let mut out = vec![Cell::Null; rows.len()];
    let real = |r: f64| Cell::Real(r.to_bits());
    for idx in parts.values() {
        let mut idx = idx.clone();
        idx.sort_by(|&a, &b| cmp_rows(&rows[a].order, &rows[b].order, &terms));
        let n = idx.len();
        // The peer group of each position, and where each group starts.
        let mut group = vec![0usize; n];
        let mut starts = vec![0usize];
        for p in 1..n {
            let new = cmp_rows(&rows[idx[p - 1]].order, &rows[idx[p]].order, &terms) != Ordering::Equal;
            group[p] = group[p - 1] + usize::from(new);
            if new {
                starts.push(p);
            }
        }
        let groups = starts.len() as i64;
        let first = |g: i64| if g < 0 { 0 } else if g >= groups { n as i64 } else { starts[g as usize] as i64 };
        let last = |g: i64| if g < 0 { -1 } else if g + 1 >= groups { n as i64 - 1 } else { starts[g as usize + 1] as i64 - 1 };
        // A RANGE offset bound's row over the one integer ORDER BY term.
        let int_of = |q: usize| match rows[idx[q]].order.first() {
            Some(Cell::Int(i)) => Ok(Some(i128::from(*i))),
            Some(Cell::Null) => Ok(None),
            other => Err(format!("a RANGE offset over a value that is not an integer: {other:?}")),
        };
        let desc = w.order.first().is_some_and(|o| o.desc);
        let nulls_first = w.order.first().is_some_and(|o| o.nulls.unwrap_or(!o.desc));
        let range_at = |p: usize, b: Bound, start: bool| -> Result<i64, String> {
            let off = match b {
                Bound::Preceding(k) => -i128::from(k),
                Bound::Following(k) => i128::from(k),
                _ => unreachable!(),
            };
            let Some(v) = int_of(p)? else {
                // A NULL row: its NULL peers.
                return Ok(if start { first(group[p] as i64) } else { last(group[p] as i64) });
            };
            // Exact: values near the ends of 64 bits are left out (see
            // `windows`).
            let bound = if desc { v - off } else { v + off };
            let inside = |x: i128| if start == desc { x <= bound } else { x >= bound };
            let mut found: Option<usize> = None;
            // The first row inside for a start, the last for an end (inside
            // is monotone over the rows in order): a NULL row is before every
            // value when NULLs sort first, so inside an end and not a start,
            // and the other way round when they sort last (window seeds
            // 5600332, 5601487: `UNBOUNDED PRECEDING AND 1 PRECEDING` over a
            // NULL and a 0 counts the NULL for the 0).
            for q in 0..n {
                let ok = match int_of(q)? {
                    Some(x) => inside(x),
                    None => start != nulls_first,
                };
                if ok {
                    found = Some(q);
                    if start {
                        break;
                    }
                }
            }
            Ok(match (found, start) {
                (Some(q), _) => q as i64,
                (None, true) => n as i64,
                (None, false) => -1,
            })
        };
        for p in 0..n {
            let g = group[p] as i64;
            let pi = p as i64;
            let at_row = |q: i64| if (0..n as i64).contains(&q) { rows[idx[q as usize]].arg.clone().unwrap_or(Cell::Null) } else { Cell::Null };
            let value = match w.func {
                WinFunc::RowNumber => Cell::Int(pi + 1),
                WinFunc::Rank => Cell::Int(starts[group[p]] as i64 + 1),
                WinFunc::DenseRank => Cell::Int(g + 1),
                WinFunc::PercentRank if n > 1 => real(starts[group[p]] as f64 / (n - 1) as f64),
                WinFunc::PercentRank => real(0.0),
                WinFunc::CumeDist => real((last(g) + 1) as f64 / n as f64),
                WinFunc::Ntile(k) => {
                    let k = usize::from(k.max(1));
                    let (size, big) = (n / k, n % k);
                    let bucket = if size == 0 { p } else if p < big * (size + 1) { p / (size + 1) } else { big + (p - big * (size + 1)) / size };
                    Cell::Int(bucket as i64 + 1)
                },
                WinFunc::Lag(k) | WinFunc::Lead(k) => {
                    let q = if matches!(w.func, WinFunc::Lag(_)) { pi - i64::from(k) } else { pi + i64::from(k) };
                    if (0..n as i64).contains(&q) {
                        at_row(q)
                    } else {
                        rows[idx[p]].default.clone().unwrap_or(Cell::Null)
                    }
                },
                func => {
                    let (lo, hi) = match w.frame {
                        None if w.order.is_empty() => (0, n as i64 - 1),
                        None => (0, last(g)),
                        Some(f) => {
                            let at = |b: Bound, start: bool| -> Result<i64, String> {
                                Ok(match (f.mode, b) {
                                    (_, Bound::UnboundedPreceding) => 0,
                                    (_, Bound::UnboundedFollowing) => n as i64 - 1,
                                    (FrameMode::Rows, Bound::Preceding(k)) => pi - i64::from(k),
                                    (FrameMode::Rows, Bound::CurrentRow) => pi,
                                    (FrameMode::Rows, Bound::Following(k)) => pi + i64::from(k),
                                    (FrameMode::Range, Bound::Preceding(_) | Bound::Following(_)) => range_at(p, b, start)?,
                                    (_, Bound::Preceding(k)) if start => first(g - i64::from(k)),
                                    (_, Bound::Preceding(k)) => last(g - i64::from(k)),
                                    (_, Bound::CurrentRow) if start => first(g),
                                    (_, Bound::CurrentRow) => last(g),
                                    (_, Bound::Following(k)) if start => first(g + i64::from(k)),
                                    (_, Bound::Following(k)) => last((g + i64::from(k)).min(groups - 1)),
                                })
                            };
                            (at(f.start, true)?, at(f.end, false)?)
                        },
                    };
                    let exclude = w.frame.and_then(|f| f.exclude).unwrap_or(Exclude::NoOthers);
                    let frame: Vec<usize> = (lo.max(0)..=hi.min(n as i64 - 1))
                        .map(|q| q as usize)
                        .filter(|&q| match exclude {
                            Exclude::NoOthers => true,
                            Exclude::CurrentRow => q != p,
                            Exclude::Group => group[q] != group[p],
                            Exclude::Ties => q == p || group[q] != group[p],
                        })
                        .collect();
                    let nth = |k: usize| frame.get(k).map_or(Cell::Null, |&q| at_row(q as i64));
                    let args: Vec<Option<&Cell>> = frame.iter().filter(|&&q| rows[idx[q]].filter).map(|&q| rows[idx[q]].arg.as_ref()).collect();
                    let values: Vec<&Cell> = args.iter().flatten().copied().filter(|c| **c != Cell::Null).collect();
                    let int_sum = |values: &[&Cell]| -> Result<i128, String> {
                        let mut sum = 0i128;
                        for v in values {
                            match v {
                                Cell::Int(i) => sum += i128::from(*i),
                                other => return Err(format!("a sum of a value that is not an integer: {other:?}")),
                            }
                        }
                        Ok(sum)
                    };
                    match func {
                        WinFunc::FirstValue => nth(0),
                        WinFunc::LastValue => frame.last().map_or(Cell::Null, |&q| at_row(q as i64)),
                        WinFunc::NthValue(k) => nth(usize::from(k.max(1)) - 1),
                        WinFunc::Count if w.arg.is_none() => Cell::Int(args.len() as i64),
                        WinFunc::Count => Cell::Int(values.len() as i64),
                        WinFunc::Sum | WinFunc::Avg if values.is_empty() => Cell::Null,
                        WinFunc::Sum => Cell::Int(i64::try_from(int_sum(&values)?).map_err(|_| "integer overflow (computed)".to_string())?),
                        WinFunc::Total => real(int_sum(&values)? as f64),
                        WinFunc::Avg => real(int_sum(&values)? as f64 / values.len() as f64),
                        WinFunc::GroupConcat(_) if values.is_empty() => Cell::Null,
                        WinFunc::GroupConcat(k) => {
                            let sep = crate::expr::GROUP_SEPS[k as usize % crate::expr::GROUP_SEPS.len()].as_bytes();
                            let mut text = Vec::new();
                            for (i, v) in values.iter().enumerate() {
                                if i > 0 {
                                    text.extend_from_slice(sep);
                                }
                                match v {
                                    Cell::Text(t) => text.extend_from_slice(t),
                                    other => return Err(format!("a group_concat of a value read as not text: {other:?}")),
                                }
                            }
                            Cell::Text(text)
                        },
                        WinFunc::JsonGroupArray => {
                            let mut items = Vec::new();
                            for a in &args {
                                match a {
                                    Some(Cell::Int(i)) => items.push(i.to_string()),
                                    Some(Cell::Null) | None => items.push("null".to_string()),
                                    Some(other) => return Err(format!("a json_group_array of a value that is not an integer: {other:?}")),
                                }
                            }
                            Cell::Text(format!("[{}]", items.join(",")).into_bytes())
                        },
                        WinFunc::Min => values.iter().min_by(|a, b| cmp_cells(a, b)).map_or(Cell::Null, |c| (*c).clone()),
                        _ => values.iter().max_by(|a, b| cmp_cells(a, b)).map_or(Cell::Null, |c| (*c).clone()),
                    }
                },
            };
            out[idx[p]] = value;
        }
    }
    Ok(out)
}

/// Window functions (see the module documentation), for queries with
/// window function items, in UTF-8 databases: `window-def`, the query
/// (without its ORDER BY and LIMIT) against its rows computed from `SELECT
/// I', x1, .. FROM F WHERE p`, where `I'` is the items with each window
/// NULL and `x1, ..` are each window's PARTITION BY terms, ORDER BY terms
/// and argument: the windows' values by [`window_values`], as a multiset,
/// exactly. Other queries, and queries in UTF-16 databases or compared
/// folded, are checked as [`window_wrap`] says.
pub fn windows(case: &Case, queries: &[Query]) -> Result<Tally, String> {
    each(case, queries, |db, m, schema, q, tally| {
        let Items::Exprs(items) = &q.items else {
            return window_wrap(db, m, schema, q, tally);
        };
        if !q.has_window() || m.encoding != Encoding::Utf8 || folding() {
            return window_wrap(db, m, schema, q, tally);
        }
        let osql = q.core().sql();
        let Some(orig) = original(db, &osql, tally)? else {
            return Ok(());
        };
        let (mut cols, mut extra, mut layout) = (Vec::new(), Vec::new(), Vec::new());
        for e in items {
            match e {
                Expr::Window(w) => {
                    cols.push("NULL".to_string());
                    layout.push(Some((&**w, extra.len())));
                    let terms = w.partition.iter().chain(w.keys());
                    extra.extend(terms.map(|x| q.e(x, 0).to_string()));
                    // group_concat reads its argument as text.
                    let text = matches!(w.func, WinFunc::GroupConcat(_));
                    extra.extend(w.arg.iter().map(|x| if text { format!("CAST({} AS TEXT)", q.e(x, 0)) } else { q.e(x, 0).to_string() }));
                    extra.extend(w.default.iter().map(|x| q.e(x, 0).to_string()));
                    extra.extend(w.filter.iter().map(|x| format!("(({}) IS TRUE)", q.e(x, 0))));
                },
                _ => {
                    cols.push(q.e(e, 0).to_string());
                    layout.push(None);
                },
            }
        }
        let width = cols.len();
        cols.extend(extra);
        let bsql = format!("{}SELECT {} FROM {}{}", q.with_sql(), cols.join(", "), q.from_sql(0, &[]), q.where_sql(0));
        let base = match run(db, &bsql) {
            Ok(r) => r,
            Err(e) if skip(tally, &e).is_some() => return Ok(()),
            Err(e) => return Err(format!("the window's input fails: {bsql}\n  -> {e}")),
        };
        // SQLite adds a RANGE offset to a value in 64 bits and past them in
        // a double, whose rounding puts -2^63 within 1 PRECEDING of itself
        // or not by how the bound is tested (window seeds 4376, 9145): a
        // RANGE frame with offsets over values near the ends is not
        // computed.
        let far = |c: &Cell| matches!(c, Cell::Int(i) if i.unsigned_abs() > 1 << 62);
        for (w, at) in layout.iter().flatten() {
            if w.frame.is_some_and(|f| f.mode == FrameMode::Range && f.offsets()) {
                let o = width + at + w.partition.len();
                if base.iter().any(|r| far(&r[o])) {
                    tally.inapplicable += 1;
                    return Ok(());
                }
            }
        }
        let mut expected: Outcome = Ok(base.iter().map(|r| r[..width].to_vec()).collect());
        for (k, l) in layout.iter().enumerate() {
            let (Some((w, at)), Ok(rows)) = (l, &mut expected) else {
                continue;
            };
            let (np, nk) = (w.partition.len(), w.order.len());
            let (p, o) = (width + at, width + at + np);
            let a = o + nk;
            let d = a + usize::from(w.arg.is_some());
            let fl = d + usize::from(w.default.is_some());
            let input: Vec<WindowRow> = base
                .iter()
                .map(|r| WindowRow {
                    partition: r[p..o].to_vec(),
                    order: r[o..a].to_vec(),
                    arg: w.arg.as_ref().map(|_| r[a].clone()),
                    default: w.default.as_ref().map(|_| r[d].clone()),
                    filter: w.filter.is_none() || r[fl] == Cell::Int(1),
                })
                .collect();
            match window_values(w, &input) {
                Ok(values) => {
                    for (row, v) in rows.iter_mut().zip(values) {
                        row[k] = v;
                    }
                },
                Err(e) => expected = Err(e),
            }
        }
        let what = format!("{bsql} (the windows computed from its rows)");
        agree(tally, "window-def", Mode::Multiset, &osql, &orig, &what, &expected, &|_| false)
    })
}

/// For metrics: how many queries fail, and the mean share of the rows of
/// a query's FROM clause (its joined rows) that its filter selects (over
/// queries with a filter and some rows; 0 without any). (0, 0) when the
/// case does not run.
pub fn selectivity(case: &Case, queries: &[Query]) -> (usize, f64) {
    let Ok(db) = open(case) else {
        return (0, 0.0);
    };
    let (mut errors, mut shares) = (0, Vec::new());
    for q in queries {
        if run(&db, &q.sql()).is_err() {
            errors += 1;
        }
        if q.filter.is_none() {
            continue;
        }
        let count = |sql: String| match run(&db, &sql).ok().as_ref().and_then(scalar) {
            Some(Cell::Int(n)) => Some(*n as f64),
            _ => None,
        };
        let from = q.from_sql(0, &[]);
        let w = q.with_sql();
        let all = count(format!("{w}SELECT count(*) FROM {from}"));
        let some = count(format!("{w}SELECT count(*) FROM {from}{}", q.where_sql(0)));
        if let (Some(all), Some(some)) = (all, some) {
            if all > 0.0 {
                shares.push(some / all);
            }
        }
    }
    let share = if shares.is_empty() { 0.0 } else { shares.iter().sum::<f64>() / shares.len() as f64 };
    (errors, share)
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::{window_values, Cell, SetOp};
    use crate::expr::{Bound, By, Exclude, Expr, Frame, FrameMode, Items, Order, OrderKey, Query, WinFunc, Window};

    /// A grouped subquery whose group holds values that compare equal but
    /// differ (0 and 0.0) makes the query ambiguous, as its own GROUP BY
    /// would (run 20261014-0121, plans-noindex w3-69281); the same
    /// subquery over values that do not is not.
    /// Swapping a member whose column has no declared sequence with one
    /// whose column is RTRIM changes the compound's sequence (a column's
    /// BINARY decides as the leftmost member's), so setops-swap leaves it
    /// out (run 20261016-0317, setops-swap w6-47); members with the same
    /// sequences are still swapped.
    #[test]
    fn swap_needs_the_same_sequences_with_a_missing_one_read_as_binary() {
        use crate::{expr::{RowKey, Schema, TableInfo}, model::Coll};
        let table = |coll: Vec<Option<Coll>>| TableInfo {
            columns: 2, rows: 1, samples: vec![], by_column: vec![vec![]; 2], shared: vec![], key: RowKey::Rowid,
            exprs: vec![], partial: vec![], leading: vec![], index_cols: vec![], coll, aff: Vec::new(),
        };
        let schema = Schema { tables: vec![table(vec![Some(Coll::Rtrim), None]), table(vec![None, None]), table(vec![None, None])], ctes: vec![], catalog: Default::default() };
        let member = |t: usize| Query::simple(t, false, Items::Exprs(vec![Expr::col(0, 0), Expr::col(0, 1)]), None);
        assert!(!super::swap_keeps_colls(&member(0), &member(1), &schema));
        assert!(super::swap_keeps_colls(&member(1), &member(2), &schema));
    }

    /// A compound takes BINARY from its leftmost member's column of no
    /// declared sequence, a CTE's (the recursive counter's too), a
    /// view's or a subquery's included (an INTEGER PRIMARY KEY read through
    /// a subquery in FROM is a column of it); only an expression without a
    /// column (`c0 || ''`) or a table's INTEGER PRIMARY KEY defers to the
    /// next member's RTRIM. Run 20261018-1734 setops-def w3-8985: `WITH
    /// RECURSIVE x0(c0) AS (SELECT 0 AS c0 UNION ALL ..) SELECT * FROM x0
    /// AS a0 UNION SELECT * FROM t1 AS a0`, t1's column RTRIM holding ''
    /// and ' ', which SQLite keeps both of. And a first part of the members
    /// that SQLite computes as a compound of its own, under its own
    /// sequences, is told apart from the compound (`steps_share_colls`).
    #[test]
    fn a_compound_takes_binary_from_any_column() {
        use crate::{
            expr::{Cte, CteBody, RowKey, Schema, TableInfo, CTE},
            model::{Coll, Value},
        };
        let table = |coll: Vec<Option<Coll>>| TableInfo {
            columns: coll.len(), rows: 2, samples: vec![], by_column: vec![vec![]; coll.len()], shared: vec![], key: RowKey::Rowid,
            exprs: vec![], partial: vec![], leading: vec![], index_cols: vec![], coll, aff: Vec::new(),
        };
        let schema = Schema { tables: vec![table(vec![Some(Coll::Binary)]), table(vec![Some(Coll::Rtrim)]), table(vec![None, Some(Coll::Binary)])], ctes: vec![], catalog: Default::default() };
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0); CREATE TABLE t1(c0 COLLATE RTRIM); INSERT INTO t1 VALUES (''), (' '); CREATE TABLE t2(c0 INTEGER PRIMARY KEY, c1); INSERT INTO t2 VALUES (1, 'a');").unwrap();
        let star = |t: usize| Query::simple(t, false, Items::Star, None);
        let item = |t: usize, e: Expr| Query::simple(t, false, Items::Exprs(vec![e]), None);
        let rtrim = star(1);
        let count = Cte { body: CteBody::Count { start: 0, count: 1, form: 0 }, hint: None, columns: false };
        let concat = Expr::bin(crate::expr::BinOp::Concat, Expr::col(0, 0), Expr::Lit(Value::Text(String::new())));
        // (first member, its CTEs, the compound's sequence, the rows SQLite gives).
        let cases = [
            (star(CTE), vec![count], Coll::Binary, 3),
            (star(0), vec![], Coll::Binary, 2),
            (item(0, concat), vec![], Coll::Rtrim, 1),
            (item(2, Expr::col(0, 0)), vec![], Coll::Rtrim, 2),
        ];
        for (first, with, coll, n) in cases {
            let q = Query { with, compound: vec![(SetOp::Union, rtrim.clone())], ..first };
            let scoped = q.scope(&schema).unwrap();
            assert_eq!(super::compound_colls(&q, &scoped), Some(vec![coll]), "{}", q.sql());
            assert!(super::steps_share_colls(&q, &scoped));
            let rows = super::run(&db, &q.sql()).unwrap();
            assert_eq!(rows.len(), n, "{} gives {rows:?}", q.sql());
            let mut def = Vec::new();
            for c in q.cores() {
                def = super::set_op_coll(SetOp::Union, def, super::run(&db, &c.sql()).unwrap(), &[coll]);
            }
            assert_eq!(def.len(), n, "{}", q.sql());
        }
        // An INTEGER PRIMARY KEY has none; read through a subquery, BINARY.
        let levels = [vec![2]];
        assert_eq!(crate::expr::member_coll(&Expr::col(0, 0), &schema, &levels, 0), Some(None));
        assert_eq!(crate::expr::member_coll(&Expr::col(0, 0), &schema, &levels, 1), Some(Some(Coll::Binary)));
        assert_eq!(super::run(&db, "SELECT a0.c0 FROM (SELECT * FROM t2) AS a0 UNION SELECT c0 FROM t1").unwrap().len(), 3);
        // `c0 || '' UNION NULL UNION ALL 'x' COLLATE NOCASE`: the UNION
        // under BINARY, the compound NOCASE.
        let nul = item(0, Expr::Lit(Value::Null));
        let x = item(0, Expr::Collate(Box::new(Expr::Lit(Value::Text("x".into()))), Coll::Nocase));
        let concat = item(0, Expr::bin(crate::expr::BinOp::Concat, Expr::col(0, 0), Expr::Lit(Value::Text(String::new()))));
        let q = Query { compound: vec![(SetOp::Union, nul.clone()), (SetOp::UnionAll, x.clone())], ..concat.clone() };
        assert_eq!(super::compound_colls(&q, &schema), Some(vec![Coll::Nocase]));
        assert!(!super::steps_share_colls(&q, &schema));
        // With the UNION ALL first, every part's sequence is the compound's.
        let q = Query { compound: vec![(SetOp::UnionAll, nul), (SetOp::Union, x)], ..concat };
        assert!(super::steps_share_colls(&q, &schema));
    }

    /// `order-multiset` skips a compound that SQLite computes in parts under
    /// other sequences than its own, ORDER BY or not (`steps_share_colls`):
    /// `c0 || '' UNION 9 UNION ALL NULL COLLATE RTRIM` over '' and ' ' keeps
    /// both without ORDER BY (the UNION under BINARY) and one with `ORDER BY
    /// 1` on trunk 3769f60be38b (3.51.0 the other way round; the bundled
    /// 3.55.0 both ways), the known item 2 of
    /// case-studies/sqlite/bugs/sqlite-balanced-merge-dedup-collation. Run 20261022-0005
    /// order-multiset w4-16054. With the UNION ALL first it is compared.
    #[test]
    fn order_multiset_skips_a_compound_whose_parts_take_other_sequences() {
        use crate::{
            cases::Case,
            model::{normalize, Coll, Encoding, Key, Layout, Model, Row, Session, Table, Value},
            SqliteFile,
        };
        let mut t = Table::new(1, Key::Rowid);
        t.rows = ["", " "].iter().enumerate().map(|(k, v)| Row { rowid: k as i64 + 1, values: vec![Value::Text(v.to_string())] }).collect();
        let mut m = Model { encoding: Encoding::Utf8, tables: vec![t], views: Vec::new(), triggers: Vec::new(), savepoints: Vec::new(), foreign_keys: true, rules: Vec::new(), recursive_triggers: false, session: Session::default(), vtabs: Vec::new() };
        let mut l = Layout::SIMPLE;
        normalize(&mut m, &mut l);
        let case = Case { file: SqliteFile(crate::writer::write(&m, &l)), statements: Vec::new() };
        let item = |e: Expr| Query::simple(0, false, Items::Exprs(vec![e]), None);
        let concat = item(Expr::bin(crate::expr::BinOp::Concat, Expr::col(0, 0), Expr::Lit(Value::Text(String::new()))));
        let nul = item(Expr::Lit(Value::Int(9)));
        let x = item(Expr::Collate(Box::new(Expr::Lit(Value::Null)), Coll::Rtrim));
        let order = vec![Order { key: OrderKey::Column(0), desc: false, nulls: None, by: By::Number }];
        let q = Query { compound: vec![(SetOp::Union, nul.clone()), (SetOp::UnionAll, x.clone())], order: order.clone(), ..concat.clone() };
        let t = super::order(&case, std::slice::from_ref(&q)).unwrap();
        assert_eq!((t.queries, t.coll_skips), (1, 1));
        let q = Query { compound: vec![(SetOp::UnionAll, nul), (SetOp::Union, x)], order, ..concat };
        let t = super::order(&case, std::slice::from_ref(&q)).unwrap();
        assert_eq!((t.queries, t.coll_skips), (1, 0));
    }

    /// A DISTINCT subquery keeps one of the rows that compare equal
    /// (0 and 0.0), which one by scan order, so a query reading it is
    /// ambiguous (run 20261015-1745, plans-noindex w0-79984); over values
    /// that do not compare equal it is not, and neither is a top-level
    /// DISTINCT.
    #[test]
    fn distinct_subqueries_are_ambiguous_over_equal_values() {
        let sub = Query::simple(2, true, Items::Exprs(vec![Expr::col(0, 2)]), None);
        let q = Query::simple(0, false, Items::Exprs(vec![Expr::col(0, 0)]), Some(Expr::InSelect(Box::new(Expr::col(0, 0)), Box::new(sub), false)));
        let top = Query::simple(2, true, Items::Exprs(vec![Expr::col(0, 2)]), None);
        for (rows, ambiguous) in [("(1, 1, 0), (2, 2, 0.0)", true), ("(1, 1, 0), (2, 2, 1)", false)] {
            let db = Connection::open_in_memory().unwrap();
            db.execute_batch(&format!("CREATE TABLE t0(c0); INSERT INTO t0 VALUES ('0'); CREATE TABLE t2(c0, c1, c2); INSERT INTO t2 VALUES {rows};")).unwrap();
            assert_eq!(super::ambiguous_groups(&db, &q, true), ambiguous, "{} over {rows}", q.sql());
            assert!(!super::ambiguous_groups(&db, &top, true), "{}", top.sql());
        }
    }

    /// A compound member's `min()` or `max()` over values equal under its
    /// column's RTRIM but not identical returns either (run 20261018-0438
    /// plans-noindex w3-186496), which an EXCEPT tells apart: ambiguous;
    /// over values that differ under RTRIM too, or under a UNION ALL, not;
    /// nor over equal numbers of two types, which the compound takes for
    /// equal.
    #[test]
    fn compound_extremes_are_ambiguous_over_equal_texts() {
        use crate::expr::{Agg, BinOp};
        let member = |filter: Option<Expr>| Query::simple(3, false, Items::Agg(Agg::Min, Some(Expr::col(0, 3))), filter);
        let filter = Expr::bin(BinOp::Lt, Expr::col(0, 1), Expr::Lit(crate::model::Value::Blob(vec![])));
        let compound = |op| Query { compound: vec![(op, member(None))], ..member(Some(filter.clone())) };
        for (rows, ambiguous) in [("('-1'), ('-1  ')", true), ("('-1'), ('-2  ')", false), ("(1), (1.0)", false)] {
            let db = Connection::open_in_memory().unwrap();
            db.execute_batch(&format!(
                "CREATE TABLE t0(c0); CREATE TABLE t1(c0); CREATE TABLE t2(c0); CREATE TABLE t3(c0, c1, c2); CREATE INDEX t3_i0 ON t3(c1); \
                 ALTER TABLE t3 ADD COLUMN c3 TEXT COLLATE RTRIM; INSERT INTO t3(c3) VALUES {rows};"
            ))
            .unwrap();
            let q = compound(SetOp::Except);
            assert_eq!(super::ambiguous_groups(&db, &q, true), ambiguous, "{} over {rows}", q.sql());
            assert!(!super::ambiguous_groups(&db, &compound(SetOp::UnionAll), true));
            assert!(!super::ambiguous_groups(&db, &member(None), true));
        }
    }

    #[test]
    fn grouped_subqueries_are_ambiguous_as_the_top_level_is() {
        let sub = Query { group: vec![Expr::col(0, 2)], ..Query::simple(2, false, Items::Exprs(vec![Expr::col(0, 2)]), None) };
        let q = Query::simple(0, false, Items::Exprs(vec![Expr::col(0, 0)]), Some(Expr::InSelect(Box::new(Expr::col(0, 0)), Box::new(sub), false)));
        for (rows, ambiguous) in [("(1, 1, 0), (2, 2, 0.0)", true), ("(1, 1, 0), (2, 2, 1)", false)] {
            let db = Connection::open_in_memory().unwrap();
            db.execute_batch(&format!("CREATE TABLE t0(c0); INSERT INTO t0 VALUES (0); CREATE TABLE t2(c0, c1, c2); INSERT INTO t2 VALUES {rows};")).unwrap();
            assert_eq!(super::ambiguous_groups(&db, &q, true), ambiguous, "{} over {rows}", q.sql());
        }
    }

    /// Past the thread's deadline, a connection from [`super::open`]
    /// interrupts a long query; without one the same query runs.
    #[test]
    fn a_passed_deadline_interrupts_queries() {
        use ephyra::{Gen, Source};
        let case = crate::probes::case_queries().generate(&mut Source::new(0, 0)).0;
        let db = super::open(&case).unwrap();
        let long = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 100000000) SELECT count(*) FROM n";
        super::set_deadline(Some(std::time::Instant::now()));
        let e = super::run(&db, long).unwrap_err();
        assert!(e.contains("interrupted"), "{e}");
        super::set_deadline(None);
        let short = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1000) SELECT count(*) FROM n";
        assert_eq!(super::run(&db, short).unwrap(), vec![vec![Cell::Int(1000)]]);
    }

    /// Cases drawn from seeds `0..n`, with their queries.
    fn drawn(n: u64) -> Vec<(crate::cases::Case, Vec<crate::expr::Query>)> {
        use ephyra::{Gen, Source};
        let gen = crate::probes::case_queries();
        (0..n).map(|s| gen.generate(&mut Source::new(s, (s % 101) as usize))).collect()
    }

    /// A fresh directory for one test's WAL files.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ephyra-sqlancer-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn files_in(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    /// A thread's storage is memory until it is set: the database is an
    /// in-memory one, and no file is made.
    #[test]
    fn memory_is_the_default_storage() {
        let (case, _) = drawn(1).remove(0);
        assert_eq!(super::thread_storage(), super::Storage::Memory);
        let db = super::open(&case).unwrap();
        let mode: String = db.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "memory");
        assert!(super::LAST_FILE.with(|f| f.borrow().is_none()));
    }

    /// Under WAL storage each open makes a fresh WAL-mode file in the
    /// directory, the next open deletes the last one's files (`-wal` and
    /// `-shm` too), and `clear_storage` the rest.
    #[test]
    fn wal_files_are_made_and_cleaned() {
        let dir = scratch("files");
        super::set_storage(super::Storage::Wal { dir: dir.clone() });
        let cases = drawn(3);
        let db = super::open(&cases[1].0).unwrap();
        let mode: String = db.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "wal");
        let first = files_in(&dir);
        assert!(first.len() >= 2 && first.iter().any(|f| f.ends_with(".db-wal")), "{first:?}");
        drop(db);
        let db = super::open(&cases[2].0).unwrap();
        let second = files_in(&dir);
        assert!(second.iter().all(|f| !first.contains(f)), "{first:?} then {second:?}");
        drop(db);
        super::clear_storage();
        assert_eq!(files_in(&dir), Vec::<String>::new());
        super::set_storage(super::Storage::Memory);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The settings of a WAL open are the case's alone (so a replay is the
    /// same), and spread over every choice.
    #[test]
    fn wal_settings_are_deterministic() {
        let cases = drawn(300);
        let all: Vec<super::WalSettings> = cases.iter().map(|(c, _)| super::wal_settings(c)).collect();
        for ((c, _), s) in cases.iter().zip(&all) {
            assert_eq!(super::wal_settings(&c.clone()), *s);
        }
        let reopen = all.iter().filter(|s| s.reopen).count();
        assert!((90..=210).contains(&reopen), "{reopen} of 300 reopen");
        for k in [4, 10, 32, 100] {
            assert!(all.iter().any(|s| s.cache_size == k), "cache_size {k}");
        }
        for k in [1, 4, 16, 100] {
            assert!(all.iter().any(|s| s.autocheckpoint == k), "autocheckpoint {k}");
        }
        assert!(all.iter().any(|s| s.synchronous == "FULL") && all.iter().any(|s| s.synchronous == "NORMAL"));
    }

    /// A case that reopens its file hands over a new connection whose WAL
    /// index was recovered from the WAL the first one kept (frames in the
    /// log), and the storage oracle agrees on it.
    #[test]
    fn a_reopened_wal_file_is_recovered() {
        let dir = scratch("reopen");
        super::set_storage(super::Storage::Wal { dir: dir.clone() });
        let mut recovered = 0;
        for (case, queries) in drawn(120).into_iter().filter(|(c, _)| super::wal_settings(c).reopen) {
            let db = super::open(&case).unwrap();
            // A case that ends in a transaction keeps its first connection.
            if !db.is_autocommit() {
                continue;
            }
            let (busy, log, _): (i64, i64, i64) = db.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
            assert_eq!(busy, 0);
            recovered += usize::from(log > 0);
            drop(db);
            super::storage(&case, &queries).unwrap();
        }
        assert!(recovered >= 10, "{recovered} reopened files had frames in their WAL");
        super::clear_storage();
        super::set_storage(super::Storage::Memory);
        assert_eq!(files_in(&dir), Vec::<String>::new());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A checkpoint SQLite refuses with `SQLITE_LOCKED` on a WAL file whose
    /// WAL was not opened yet, the connection's TEMP database open, counts
    /// as run (`run_statement`): the case opens in both storages, and the
    /// storage oracle agrees on it. Run 20261018-1734 storage w13-91071.
    #[test]
    fn a_locked_checkpoint_counts_as_run() {
        use crate::{
            cases::Case,
            statements::{Checkpoint, Journal, Statement},
            SqliteFile,
        };
        let dir = scratch("locked-checkpoint");
        let check = Statement::Check { quick: false, table: None };
        let statements = vec![Statement::JournalMode(Journal::Delete), check, Statement::JournalMode(Journal::Wal), Statement::WalCheckpoint(Some(Checkpoint::Passive))];
        let case = Case { file: SqliteFile(Vec::new()), statements };
        // The quirk itself, on a file the case's statements leave so.
        let path = dir.join("quirk.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA journal_mode = DELETE; PRAGMA integrity_check; PRAGMA journal_mode = WAL;").unwrap();
        let e = db.execute_batch("PRAGMA wal_checkpoint(PASSIVE)").unwrap_err();
        assert!(e.to_string().contains("database table is locked"), "{e}");
        db.execute_batch("PRAGMA wal_checkpoint(PASSIVE)").unwrap();
        drop(db);
        super::set_storage(super::Storage::Wal { dir: dir.clone() });
        let db = super::open(&case).unwrap();
        let mode: String = db.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "wal");
        drop(db);
        super::storage(&case, &[]).unwrap();
        super::open_with(&case, &super::Storage::Memory).unwrap();
        // Any other failure of a statement still fails the open.
        let bad = Case { file: SqliteFile(Vec::new()), statements: vec![Statement::Commit] };
        assert!(super::open(&bad).is_err());
        super::clear_storage();
        super::set_storage(super::Storage::Memory);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The storage oracle skips, and counts, a side that fails for the
    /// host's storage, and no other error.
    #[test]
    fn io_errors_are_the_hosts() {
        assert!(super::io_error("database or disk is full"));
        assert!(super::io_error("disk I/O error: writing a database in /x: No space left on device (os error 28)"));
        assert!(!super::io_error("database disk image is malformed"));
        let (case, queries) = drawn(40).into_iter().find(|(_, q)| !q.is_empty()).unwrap();
        // A directory that cannot be made: the input is skipped, counted.
        super::set_storage(super::Storage::Wal { dir: "/dev/null/ephyra".into() });
        let t = super::storage(&case, &queries).unwrap();
        assert_eq!((t.io_skips, t.queries), (1, 0));
        let t = super::norec(&case, &queries).unwrap();
        assert_eq!(t.io_skips, 1);
        super::set_storage(super::Storage::Memory);
    }

    /// The SQLite behaviour behind [`super::invalid_text_pattern`]: an
    /// over-length UTF-8 blob (`x'fba9'`, read as `é`) matches `GLOB 'é*'`
    /// in a scan but not in the LIKE optimization's range scan (invalid
    /// UTF-8 is garbage in, garbage out, invalidutf.html). When this fails,
    /// SQLite treats such values alike in both plans and the exclusion can
    /// go.
    /// An original refused for a limit or a JSON argument is skipped with
    /// its query and counted; so is a comparison where only the variant is;
    /// any other error of the original is a failure. The messages are
    /// SQLite's own (run-0446's findings: a LIKE pattern from
    /// `json_pretty`, text that is not JSON).
    #[test]
    fn limit_and_json_errors_are_skipped_and_counted() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t(c); INSERT INTO t VALUES ('{]'), (x'01'), (printf('%.60000c', 'x'));").unwrap();
        let mut t = super::Tally::default();
        for sql in [
            "SELECT count(*) FROM t WHERE 'a' LIKE c",
            "SELECT json_extract(c, '$') FROM t",
            "SELECT json('{]')",
            "SELECT json_array(x'ff')",
            "SELECT json_extract('{}', 'a')",
            "SELECT json_object(1, 2)",
        ] {
            assert!(matches!(super::original(&db, sql, &mut t), Ok(None)), "{sql}: {:?}", super::run(&db, sql));
        }
        assert_eq!((t.limit_skips, t.json_skips), (1, 5));
        let e = super::original(&db, "SELECT abs(-9223372036854775808)", &mut t);
        assert!(matches!(e, Ok(Some(Err(_)))), "integer overflow is an outcome");
        let e = super::original(&db, "SELECT no_such_function(1)", &mut t).unwrap_err();
        assert!(e.starts_with("the original query fails"), "{e}");
        let orig = super::run(&db, "SELECT 1");
        let variant = super::run(&db, "SELECT json('{]')");
        super::agree(&mut t, "norec", super::Mode::Scalar, "SELECT 1", &orig, "SELECT json('{]')", &variant, &|_| false).unwrap();
        assert_eq!(t.json_skips, 6);
        let variant = super::run(&db, "SELECT no_such_function(1)");
        assert!(super::agree(&mut t, "norec", super::Mode::Scalar, "", &orig, "", &variant, &|_| false).is_err());
    }

    /// A comparison whose rows hold text with a NUL character is skipped
    /// and counted, whichever side holds it (run 20261018-0438 setops-def
    /// w1-179561: `c0 || c0` of BLOBs that differ after a NUL byte, one
    /// row by definition and two from SQLite); text without one, and a
    /// BLOB with NUL bytes, are compared.
    #[test]
    fn text_with_a_nul_is_not_compared() {
        let db = Connection::open_in_memory().unwrap();
        let mut t = super::Tally::default();
        let one = super::run(&db, "SELECT CAST(x'000084' AS TEXT) UNION ALL SELECT NULL");
        let two = super::run(&db, "SELECT CAST(x'000090' AS TEXT) UNION ALL SELECT CAST(x'000084' AS TEXT) UNION ALL SELECT NULL");
        for (a, b) in [(&one, &two), (&two, &one)] {
            super::agree(&mut t, "setops-def", super::Mode::Equiv, "", a, "", b, &|_| false).unwrap();
        }
        assert_eq!(t.nul_skips, 2);
        let (x, y) = (super::run(&db, "SELECT 'a'"), super::run(&db, "SELECT 'b'"));
        assert!(super::agree(&mut t, "setops-def", super::Mode::Equiv, "", &x, "", &y, &|_| false).is_err());
        let (x, y) = (super::run(&db, "SELECT x'0001'"), super::run(&db, "SELECT x'0002'"));
        assert!(super::agree(&mut t, "setops-def", super::Mode::Equiv, "", &x, "", &y, &|_| false).is_err());
        assert_eq!(t.nul_skips, 2);
    }

    /// `pragma_table_list` gives a view's `ncol` as the connection got
    /// to it: a view that does not prepare keeps the views after it in the
    /// schema's hash order at 0, so the same schema read in another order
    /// lists another `ncol` (run 20261017-0438 storage w13-317839); the
    /// storage oracle leaves it out with dbstat, and keeps the other
    /// virtual tables.
    #[test]
    fn table_list_is_left_out_of_storage() {
        use crate::expr::{Cte, CteBody, Vtab};
        let ncol = |order: &str| {
            let db = Connection::open_in_memory().unwrap();
            db.execute_batch(&format!("CREATE TABLE t(a); {order} DROP TABLE t;")).unwrap();
            let bytes = db.serialize(rusqlite::MAIN_DB).unwrap().to_vec();
            let mut fresh = Connection::open_in_memory().unwrap();
            fresh.deserialize_read_exact(rusqlite::MAIN_DB, &bytes[..], bytes.len(), true).unwrap();
            super::run(&fresh, "SELECT ncol FROM pragma_table_list WHERE name = 'v0'").unwrap()
        };
        let (good, broken) = ("CREATE VIEW v0 AS SELECT 1 AS x;", "CREATE VIEW vb AS SELECT a FROM t;");
        assert_ne!(ncol(&format!("{good} {broken}")), ncol(&format!("{broken} {good}")));
        let with = |v: Vtab| Query { with: vec![Cte { body: CteBody::Vtab(v), hint: None, columns: false }], ..Query::simple(0, false, Items::Star, None) };
        assert!(super::reads_layout(&with(Vtab::TableList)));
        assert!(super::reads_layout(&with(Vtab::Dbstat { aggregate: false, arg: crate::expr::VtabArg::Call })));
        assert!(!super::reads_layout(&with(Vtab::SqliteSchema { form: 0 })));
        assert!(!super::reads_layout(&with(Vtab::Pragma { k: 4, table: None })));
    }

    /// `PRAGMA case_sensitive_like` registers `like` again on its
    /// connection, and `pragma_function_list` lists it there (two rows of
    /// `builtin` 0) and not on a new connection: a WAL file reopened after
    /// the case's statements lists fewer functions than the memory
    /// database. The storage oracle leaves such queries out when it
    /// reopens a case that runs the pragma (run 20261022-0422 storage
    /// w13-4647, replayed: it failed before), and compares them otherwise.
    #[test]
    fn function_list_is_left_out_of_a_reopened_storage() {
        use crate::{
            cases::Case,
            expr::{set_spellings, Cte, CteBody, Vtab, PRAGMAS},
            statements::Statement,
            SqliteFile,
        };
        let likes = |pragma: &str| {
            let db = Connection::open_in_memory().unwrap();
            db.execute_batch(pragma).unwrap();
            super::run(&db, "SELECT count(*) FROM pragma_function_list WHERE name = 'like' AND NOT builtin").unwrap()
        };
        assert_ne!(likes("PRAGMA case_sensitive_like = 0"), likes("PRAGMA synchronous = EXTRA"));
        let k = PRAGMAS.iter().position(|p| p.0 == "function_list").unwrap();
        let query = Query { with: vec![Cte { body: CteBody::Vtab(Vtab::Pragma { k, table: None }), hint: None, columns: false }], ..Query::simple(crate::expr::CTE, false, Items::Star, None) };
        let dir = scratch("function-list");
        set_spellings(true);
        let case = Case { file: SqliteFile(Vec::new()), statements: vec![Statement::Synchronous(3)] };
        assert!(case.statements[0].to_string().contains("PRAGMA case_sensitive_like = 0"), "{}", case.statements[0]);
        assert!(super::wal_settings(&case).reopen);
        super::set_storage(super::Storage::Wal { dir: dir.clone() });
        let t = super::storage(&case, std::slice::from_ref(&query));
        // Without the pragma the two connections list the same functions.
        let plain = Case { file: SqliteFile(Vec::new()), statements: vec![Statement::CellSizeCheck(true)] };
        assert!(super::wal_settings(&plain).reopen);
        let u = super::storage(&plain, std::slice::from_ref(&query));
        super::clear_storage();
        super::set_storage(super::Storage::Memory);
        set_spellings(false);
        let _ = std::fs::remove_dir_all(&dir);
        let t = t.unwrap();
        assert_eq!((t.inapplicable, t.comparisons), (1, 0));
        let u = u.unwrap();
        assert_eq!((u.inapplicable, u.comparisons), (0, 1));
    }

    /// A case whose expression index of a date function reads a value
    /// SQLite takes for the current time fails in its statements with
    /// "non-deterministic use of julianday() in an index" (deterministic.html;
    /// run 20261012-0317: 152 of 257 findings, all of worker w8, profile
    /// rare); every oracle skips such an input and counts it, and no other
    /// error. The generators' values never read so (`model::not_now`).
    #[test]
    fn nondeterministic_date_indexes_are_skipped_and_counted() {
        use crate::{
            cases::Case,
            model::{not_now, reads_as_now, Index, KeyExpr, Value},
            statements::{Statement, Update},
        };
        let utf16 = |s: &str, le: bool| -> Vec<u8> { s.encode_utf16().flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() }).collect() };
        for v in [
            Value::Text("now".into()),
            Value::Text("NoW".into()),
            Value::Text("SUBSEC".into()),
            Value::Blob(b"now".to_vec()),
            Value::Blob(b"NOW\0x".to_vec()),
            Value::Blob(utf16("now", true)),
            Value::Blob(utf16("subsecond", false)),
        ] {
            assert!(reads_as_now(&v), "{v:?}");
            assert!(!reads_as_now(&not_now(v.clone())), "{v:?}");
        }
        for v in [Value::Text(" now".into()), Value::Text("now ".into()), Value::Blob(b"no".to_vec()), Value::Int(0), Value::Null] {
            assert!(!reads_as_now(&v), "{v:?}");
            assert_eq!(not_now(v.clone()), v);
        }
        // A case whose first table has rows and a column an UPDATE may set.
        let settable = |t: &crate::model::Table| {
            t.strict.is_none() && !t.rows.is_empty() && t.indexes.iter().all(|i| !i.unique) && (0..t.columns).any(|c| !crate::semantics::is_key(t, c))
        };
        let (case, queries) =
            drawn(200).into_iter().find(|(c, q)| !q.is_empty() && crate::probes::final_model(c).is_some_and(|m| m.tables.first().is_some_and(settable))).unwrap();
        let m = crate::probes::final_model(&case).unwrap();
        let t = &m.tables[0];
        let mut statements = case.statements.clone();
        let index = Index { expr: Some(KeyExpr::Julianday), ..Index::of(vec![(0, false)]) };
        statements.push(Statement::CreateIndex { table: 0, index: t.indexes.len(), def: index, db: crate::model::Db::Main });
        let column = (0..t.columns).find(|&c| !crate::semantics::is_key(t, c)).unwrap();
        let index = Index { expr: Some(KeyExpr::Julianday), ..Index::of(vec![(column, false)]) };
        statements.pop();
        statements.push(Statement::CreateIndex { table: 0, index: t.indexes.len(), def: index, db: crate::model::Db::Main });
        let insert = |v: Value| Statement::Update(Update { table: 0, sets: vec![(column, v)], filter: None, ..Default::default() });
        assert!(!crate::semantics::valid(&m, &insert(Value::Blob(b"now".to_vec()))));
        assert!(crate::semantics::valid(&m, &insert(Value::Blob(b"nos".to_vec()))));
        // Written as the generators never write it (text: a blob reads as
        // text in the database's encoding, which may be UTF-16).
        let with = |v: Value| {
            let mut s = statements.clone();
            s.push(insert(v));
            Case { file: case.file.clone(), statements: s }
        };
        let bad = with(Value::Text("nOw".into()));
        let Err(e) = super::open(&bad) else {
            panic!("SQLite takes `now` under a julianday() index");
        };
        assert!(super::nondeterministic(&e), "{e}");
        for oracle in [super::norec, super::tlp, super::plans, super::order, super::setops, super::ctes, super::windows, super::storage] {
            let t = oracle(&bad, &queries).unwrap();
            assert_eq!((t.nondeterministic_skips, t.queries), (1, 0));
        }
        let good = with(not_now(Value::Text("nOw".into())));
        super::open(&good).unwrap();
        for e in ["non-deterministic use of julianday() in a CHECK constraint", "statement 3: INSERT ..: non-deterministic use of strftime() in a generated column"] {
            assert!(super::nondeterministic(e), "{e}");
        }
        for e in ["non-deterministic functions prohibited in index expressions", "non-deterministic use of random() in an index", "non-deterministic use of date() in an index, and more", "UNIQUE constraint failed: t0.c0"] {
            assert!(!super::nondeterministic(e), "{e}");
        }
    }

    #[test]
    fn sqlite_matches_over_length_utf8_only_without_the_like_optimization() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t(c); CREATE INDEX ti ON t(c); INSERT INTO t VALUES (x'fba9'), (x'c3a9');").unwrap();
        let indexed = super::run(&db, "SELECT count(*) FROM t WHERE c GLOB 'é*'").unwrap();
        let scanned = super::run(&db, "SELECT count(*) FROM t NOT INDEXED WHERE c GLOB 'é*'").unwrap();
        // The valid x'c3a9' is found both ways; the over-length x'fba9' only by the scan.
        assert_eq!((indexed, scanned), (vec![vec![Cell::Int(1)]], vec![vec![Cell::Int(2)]]), "SQLite may have changed: remove the exclusion");
    }

    /// The exclusion of JSONB bytes: SQLite encodes one string as JSONB
    /// TEXT or TEXTRAW depending on its JSON parse cache, so on row order
    /// (a plans-index finding of the runner). The
    /// bytes differ with and without the index; the cells do not.
    #[test]
    fn jsonb_compares_as_its_json_text() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0); CREATE INDEX t0_i0 ON t0(c0); INSERT INTO t0 VALUES (printf('%.120c', 'x')), (NULL), (printf('%.120c', 'x'));").unwrap();
        let cols = "jsonb(json_quote(c0)), length(json('[1]')), length(json_replace('[]', '$', c0))";
        let heads = |from: &str| super::run(&db, &format!("SELECT hex(substr(jsonb(json_quote(c0)), 1, 1)), length(json('[1]')), length(json_replace('[]', '$', c0)) FROM {from}")).unwrap();
        let mut a = heads("t0 NOT INDEXED");
        let mut b = heads("t0 INDEXED BY t0_i0");
        a.sort();
        b.sort();
        assert_ne!(a, b, "SQLite may encode JSONB alike now: the exclusion may go");
        let mut x = super::run(&db, &format!("SELECT {cols} FROM t0 NOT INDEXED")).unwrap();
        let mut y = super::run(&db, &format!("SELECT {cols} FROM t0 INDEXED BY t0_i0")).unwrap();
        x.sort();
        y.sort();
        assert_eq!(x, y);
        assert!(x.iter().flatten().any(|c| matches!(c, Cell::Jsonb(..))));
        // Two encodings of one value are one value to the oracles, but two
        // rows to SQLite's sets (bugs: seed 2010528 of the setops burn).
        let two = super::run(&db, "SELECT x'07' UNION ALL SELECT x'0a'").unwrap();
        assert_eq!(two[0], two[1]);
        assert_eq!(super::set_op(SetOp::Union, two.clone(), Vec::new()).len(), 2);
        assert_eq!(super::run(&db, "SELECT x'07' UNION SELECT x'0a'").unwrap().len(), 2);
        // A blob that is no JSONB stays a blob; x'00' (JSONB null) is not the text 'null'.
        assert_eq!(super::run(&db, "SELECT x'ff', x'00', 'null'").unwrap(), vec![vec![Cell::Blob(vec![0xff]), Cell::Jsonb(b"null".to_vec(), super::Raw(vec![0])), Cell::Text(b"null".to_vec())]]);
    }

    /// The behaviour behind the skips of `plans-index-partial` and
    /// `plans-index-expr`: `INDEXED BY` a partial index the WHERE clause
    /// does not imply, or an expression index right of a RIGHT JOIN, finds
    /// no plan; the same hints elsewhere do. And the planner matches the
    /// terms the queries draw to the indexes the statements create.
    #[test]
    fn sqlite_finds_no_plan_for_restricted_index_hints() {
        use crate::{
            expr::{key_expr, pred_expr, At, BinOp, Sql},
            model::{Index, KeyExpr, Value},
            statements::{CmpOp, Pred, Statement},
        };
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0, c1); CREATE TABLE t1(c0); INSERT INTO t0 VALUES (1, 2), ('A', NULL); INSERT INTO t1 VALUES (1);").unwrap();
        let partial = Index { filter: Some(Pred::IsNull(1, true)), ..Index::of(vec![(0, false)]) };
        let expr = Index { expr: Some(KeyExpr::Lower), ..Index::of(vec![(0, false)]) };
        for (i, def) in [partial, expr].into_iter().enumerate() {
            db.execute_batch(&Statement::CreateIndex { table: 0, index: i, def, db: crate::model::Db::Main }.to_string()).unwrap();
        }
        for sql in ["SELECT * FROM t0 INDEXED BY t0_i0", "SELECT * FROM t1 RIGHT JOIN t0 INDEXED BY t0_i1 ON 1"] {
            let e = super::run(&db, sql).unwrap_err();
            assert!(e.contains("no query solution"), "SQLite may plan it now: remove the skip ({sql}: {e})");
        }
        for sql in ["SELECT * FROM t0 INDEXED BY t0_i0 WHERE c1 = 2", "SELECT * FROM t1 LEFT JOIN t0 INDEXED BY t0_i1 ON 1"] {
            super::run(&db, sql).unwrap();
        }
        let plan = |w: &Expr| {
            let sql = format!("EXPLAIN QUERY PLAN SELECT * FROM t0 AS a0 CROSS JOIN t1 AS a1 WHERE {}", Sql(w, At { sources: 2, ..At::TOP }));
            let rows = super::run(&db, &sql).unwrap();
            format!("{rows:?}")
        };
        let lower = Expr::bin(BinOp::Eq, key_expr(KeyExpr::Lower, 0), Expr::Lit(Value::Text("a".into())));
        assert!(plan(&lower).contains("t0_i1"), "{}", plan(&lower));
        let implied = Expr::bin(BinOp::And, pred_expr(&Pred::Cmp(0, CmpOp::Eq, Value::Int(1))), pred_expr(&Pred::IsNull(1, true)));
        assert!(plan(&implied).contains("t0_i0"), "{}", plan(&implied));
    }

    /// The SQLite bug behind the `*` exclusion of `Query::using_ok`: `*`
    /// over a USING join left of a RIGHT or FULL join is "ambiguous" when a
    /// later source has a column of a USING column's name. When this
    /// fails, SQLite has fixed it and the exclusion can go.
    #[test]
    fn sqlite_rejects_star_over_right_join_using_as_ambiguous() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0, c1); CREATE TABLE t1(c0, c1); INSERT INTO t0 VALUES (1, 2); INSERT INTO t1 VALUES (1, 3);").unwrap();
        for sql in [
            "SELECT * FROM t0 FULL JOIN t1 USING (c0) CROSS JOIN t0 AS t2",
            "SELECT * FROM t0 JOIN t1 USING (c0) RIGHT JOIN t0 AS t2 ON 1",
        ] {
            let e = super::run(&db, sql).unwrap_err();
            assert!(e.contains("ambiguous column name: c0"), "SQLite may have fixed the bug: remove the exclusion ({sql}: {e})");
        }
        // Written out, the same query is fine.
        let rows = super::run(&db, "SELECT coalesce(t0.c0, t1.c0), t0.c1, t1.c1, t2.* FROM t0 FULL JOIN t1 ON t0.c0 = t1.c0 CROSS JOIN t0 AS t2").unwrap();
        assert_eq!(rows.len(), 1);
    }

    /// The window computation against SQLite on a small table: `t(c0, c1)`
    /// with rows (1, 1), (1, 1), (1, 2), (2, NULL), (1, NULL), partitioned by `c0`,
    /// ordered by `c1`.
    #[test]
    fn windows_compute_as_sqlite_does() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t(c0, c1); INSERT INTO t VALUES (1, 1), (1, 1), (1, 2), (2, NULL), (1, NULL);").unwrap();
        let rows: Vec<super::WindowRow> = [(1, Some(1)), (1, Some(1)), (1, Some(2)), (2, None), (1, None)]
            .iter()
            .map(|&(p, k)| {
                let k = k.map_or(Cell::Null, Cell::Int);
                super::WindowRow::new(vec![Cell::Int(p)], vec![k.clone()], Some(k))
            })
            .collect();
        let order = vec![Order { key: OrderKey::Expr(Expr::col(0, 1)), desc: false, nulls: None, by: By::Number }];
        let frames = [
            None,
            Some(Frame { mode: FrameMode::Groups, start: Bound::Preceding(1), end: Bound::CurrentRow, exclude: Some(Exclude::Ties), short: false }),
            Some(Frame { mode: FrameMode::Range, start: Bound::CurrentRow, end: Bound::UnboundedFollowing, exclude: Some(Exclude::Group), short: false }),
            Some(Frame { mode: FrameMode::Range, start: Bound::Preceding(1), end: Bound::Following(0), exclude: None, short: false }),
            Some(Frame { mode: FrameMode::Range, start: Bound::Following(1), end: Bound::UnboundedFollowing, exclude: Some(Exclude::CurrentRow), short: false }),
            Some(Frame { mode: FrameMode::Range, start: Bound::UnboundedPreceding, end: Bound::Preceding(1), exclude: None, short: false }),
        ];
        let funcs = [
            WinFunc::Rank,
            WinFunc::DenseRank,
            WinFunc::PercentRank,
            WinFunc::CumeDist,
            WinFunc::Count,
            WinFunc::Sum,
            WinFunc::Max,
            WinFunc::Avg,
            WinFunc::Total,
            WinFunc::JsonGroupArray,
        ];
        for func in funcs {
            for frame in frames {
                if frame.is_some() && !func.aggregate() {
                    continue;
                }
                let arg = (func != WinFunc::Count && func.aggregate()).then(|| Expr::col(0, 1));
                let w = Window { func, arg, default: None, filter: None, partition: vec![Expr::col(0, 0)], order: order.clone(), frame, named: Default::default() };
                let sql = format!("SELECT {} FROM t ORDER BY rowid", crate::expr::Sql(&Expr::Window(Box::new(w.clone())), crate::expr::At::TOP));
                let sqlite = super::run(&db, &sql).unwrap().into_iter().map(|r| r[0].clone()).collect::<Vec<_>>();
                assert_eq!(window_values(&w, &rows).unwrap(), sqlite, "{sql}");
            }
        }
    }

    /// The functions that read rows in order, against SQLite over a total
    /// order (rowid) on `t(c0, c1)`: two partitions, a NULL argument, a
    /// FILTER, a lag default, frames of each mode with EXCLUDE.
    #[test]
    fn ordered_windows_compute_as_sqlite_does() {
        use crate::{expr::{BinOp, Named, ROWID}, model::Value};
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0, c1); INSERT INTO t0 VALUES (1, 'a'), (1, NULL), (1, 'c'), (2, 'd'), (1, 'e'), (2, 'f'), (1, 'g');").unwrap();
        let base = super::run(&db, "SELECT c0, rowid, c1, CAST(c1 AS TEXT), rowid % 3 = 0, rowid - 10 FROM t0 ORDER BY rowid").unwrap();
        let order = vec![Order { key: OrderKey::Expr(Expr::col(0, ROWID)), desc: true, nulls: None, by: By::Number }];
        let frames = [
            None,
            Some(Frame { mode: FrameMode::Rows, start: Bound::Preceding(1), end: Bound::Following(2), exclude: Some(Exclude::CurrentRow), short: false }),
            Some(Frame { mode: FrameMode::Range, start: Bound::Preceding(2), end: Bound::Following(1), exclude: None, short: false }),
            Some(Frame { mode: FrameMode::Groups, start: Bound::CurrentRow, end: Bound::UnboundedFollowing, exclude: Some(Exclude::Ties), short: false }),
        ];
        let funcs = [
            WinFunc::Ntile(3),
            WinFunc::FirstValue,
            WinFunc::LastValue,
            WinFunc::NthValue(2),
            WinFunc::Lag(1),
            WinFunc::Lead(2),
            WinFunc::GroupConcat(2),
            WinFunc::Count,
        ];
        for func in funcs {
            for frame in frames {
                for filter in [false, true] {
                    if (frame.is_some() && !func.framed()) || (filter && !func.aggregate()) {
                        continue;
                    }
                    let arg = (!matches!(func, WinFunc::Ntile(_))).then(|| Expr::col(0, 1));
                    let int = |i| Expr::Lit(Value::Int(i));
                    let default = matches!(func, WinFunc::Lag(_)).then(|| Expr::bin(BinOp::Sub, Expr::col(0, ROWID), int(10)));
                    let filter = filter.then(|| Expr::bin(BinOp::Eq, Expr::bin(BinOp::Rem, Expr::col(0, ROWID), int(3)), int(0)));
                    let w = Window { func, arg, default, filter, partition: vec![Expr::col(0, 0)], order: order.clone(), frame, named: Named::Partition };
                    let rows: Vec<super::WindowRow> = base
                        .iter()
                        .map(|r| super::WindowRow {
                            partition: vec![r[0].clone()],
                            order: vec![r[1].clone()],
                            arg: Some(if matches!(func, WinFunc::GroupConcat(_)) { r[3].clone() } else { r[2].clone() }),
                            default: w.default.as_ref().map(|_| r[5].clone()),
                            filter: w.filter.is_none() || r[4] == Cell::Int(1),
                        })
                        .collect();
                    let q = Query::simple(0, false, Items::Exprs(vec![Expr::Window(Box::new(w.clone()))]), None);
                    let sql = q.sql() + " ORDER BY rowid";
                    let sqlite = super::run(&db, &sql).unwrap().into_iter().map(|r| r[0].clone()).collect::<Vec<_>>();
                    assert_eq!(window_values(&w, &rows).unwrap(), sqlite, "{sql}");
                }
            }
        }
    }

    /// TLP of a window query partitions its result rows, not its WHERE
    /// clause: `count(c1) OVER w` (`WINDOW w AS (PARTITION BY c0)`) over
    /// the rows a partition of the WHERE clause keeps counts other rows
    /// than over all of them, so the in-place identity fails; partitioned
    /// on a result column outside the windows (`tlp_window`) it holds, on
    /// the PARTITION BY column (which SQLite pushes down) or on the window.
    #[test]
    fn tlp_partitions_a_window_query_outside() {
        use crate::expr::{BinOp, Named};
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0, c1); INSERT INTO t0 VALUES (1, 1), (1, 5), (2, 3), (2, NULL), (3, 7);").unwrap();
        let w = Window {
            func: WinFunc::Count,
            arg: Some(Expr::col(0, 1)),
            default: None,
            filter: None,
            partition: vec![Expr::col(0, 0)],
            order: Vec::new(),
            frame: None,
            named: Named::Partition,
        };
        let items = Items::Exprs(vec![Expr::col(0, 0), Expr::Window(Box::new(w))]);
        let q = Query::simple(0, false, items, None);
        let sorted = |mut r: Vec<Vec<Cell>>| {
            r.sort();
            r
        };
        let all = sorted(super::run(&db, &q.sql()).unwrap());
        let in_place = q.sql().replace(" WINDOW", " WHERE {} WINDOW");
        let parts: Vec<String> = super::partitions("", "c1 > 2").iter().map(|p| in_place.replace(" WHERE {}", &format!(" WHERE{p}"))).collect();
        assert!(parts[0].starts_with("SELECT c0, count(c1) OVER (w") && parts[0].contains(" FROM t0 AS a0 WHERE  (c1 > 2) WINDOW w"), "{}", parts[0]);
        let split = sorted(super::concat(&db, &parts).unwrap());
        assert_eq!(all.len(), split.len());
        assert_ne!(all, split, "the windows count the rows of each WHERE partition");
        // The query partitioned outside: on every result column (`r0` is
        // the PARTITION BY column, `r1` the window) and every shape.
        for (x, materialized) in [("r0", false), ("r0 >= 2", true), ("r1 = 1", false), ("r1 < 2", true)] {
            let base = format!("WITH tw(r0, r1) AS {}({}) ", if materialized { "MATERIALIZED " } else { "" }, q.sql());
            let parts: Vec<String> = super::partitions("WHERE", x).iter().map(|p| format!("{base}SELECT * FROM tw{p}")).collect();
            assert_eq!(sorted(super::concat(&db, &parts).unwrap()), all, "{x}");
        }
        let mut tally = super::Tally::default();
        super::tlp_window(&db, &q, super::TLP_WINDOW, &mut tally).unwrap();
        assert_eq!((tally.comparisons, tally.variants.get("tlp-window"), tally.variants.get("tlp-window-union")), (2, Some(&1), Some(&1)));
        // The window alone: its PARTITION BY term `c0` an item of its own
        // (`r1`), which SQLite pushes down into `Q`, when drawn so.
        let alone = Query::simple(0, false, Items::Exprs(vec![q.items.exprs()[1].clone()]), None);
        let base = format!("WITH tw(r0, r1) AS ({}) ", alone.sql().replace(" FROM", ", c0 FROM"));
        let plan = super::run(&db, &format!("EXPLAIN QUERY PLAN {base}SELECT * FROM tw WHERE r1 = 2")).unwrap();
        assert!(format!("{plan:?}").contains("subquery"), "{plan:?}");
        let mut tally = super::Tally::default();
        super::tlp_window(&db, &alone, super::TLP_WINDOW, &mut tally).unwrap();
        assert_eq!(tally.comparisons, 2);
        // A WHERE clause stays in the partitioned query.
        let q = Query { filter: Some(Expr::bin(BinOp::Gt, Expr::col(0, 1), Expr::Lit(crate::model::Value::Int(2)))), ..q };
        let mut tally = super::Tally::default();
        super::tlp_window(&db, &q, super::TLP_WINDOW, &mut tally).unwrap();
        assert_eq!(tally.comparisons, 2);
    }

    /// A model of one rowid table of `columns` columns in a UTF-8 database.
    fn one_table(columns: usize) -> (crate::model::Model, crate::expr::Schema) {
        use crate::model::{Encoding, Key, Model, Table};
        let m = Model {
            encoding: Encoding::Utf8,
            tables: vec![Table::new(columns, Key::Rowid)],
            views: Vec::new(),
            triggers: Vec::new(),
            savepoints: Vec::new(),
            foreign_keys: true,
            rules: Vec::new(),
            recursive_triggers: false,
            session: crate::model::Session::default(),
            vtabs: Vec::new(),
        };
        let schema = crate::expr::Schema::of(&m);
        (m, schema)
    }

    /// `order` checks a query without ORDER BY under one by every result
    /// column (`added_order`), a total order: a LIMIT under an order by
    /// some columns only cuts among rows that tie, which differ (the same
    /// rows inserted in another order give another first row), so that is
    /// not checked; by every column the rows that tie are equal.
    #[test]
    fn order_checks_a_query_without_one_under_every_column() {
        let (m, schema) = one_table(2);
        let first = |rows: &str| {
            let db = Connection::open_in_memory().unwrap();
            db.execute_batch(&format!("CREATE TABLE t0(c0, c1); INSERT INTO t0 VALUES {rows};")).unwrap();
            super::run(&db, "SELECT c0, c1 FROM t0 ORDER BY c0 LIMIT 1").unwrap()
        };
        assert_ne!(first("(1, 'a'), (1, 'b')"), first("(1, 'b'), (1, 'a')"), "a partial order's LIMIT depends on the rows' order");
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0, c1); INSERT INTO t0 VALUES (1, 'b'), (NULL, 'a'), (1, 'a'), (2, NULL), (1, 'b'), (0, 'z');").unwrap();
        let q = Query::simple(0, false, Items::Exprs(vec![Expr::col(0, 0), Expr::col(0, 1)]), None);
        let o = super::added_order(&q, &schema, super::fnv(&q.sql())).unwrap();
        assert_eq!((o.order.len(), o.total_order(&schema)), (2, Some(false)));
        assert!(super::added_order(&o, &schema, 0).is_none(), "a query with an ORDER BY keeps its own");
        let mut tally = super::Tally::default();
        super::order_on(&db, &m, &schema, &o, false, super::ORDER_ADDED, true, &mut tally).unwrap();
        assert_eq!((tally.variants.get("order-all-multiset"), tally.variants.get("order-all-sorted")), (Some(&1), Some(&1)), "{}", o.sql());
        // Every LIMIT and term direction, with and without NULLS.
        for (desc, nulls, count, offset) in [(false, None, 2, 0), (true, Some(true), 3, 1), (true, None, 1, 2), (false, Some(false), 4, 0)] {
            let order = (0..2).map(|k| Order { key: OrderKey::Column(1 - k), desc: desc ^ (k == 1), nulls, by: By::Number }).collect();
            let o = Query { order, limit: Some(crate::expr::Limit { count, offset, comma: false }), ..q.clone() };
            let mut tally = super::Tally::default();
            super::order_on(&db, &m, &schema, &o, false, super::ORDER_ADDED, true, &mut tally).unwrap();
            assert_eq!(tally.variants.get("order-all-limit"), Some(&1), "{}", o.sql());
        }
    }

    /// TLP of an aggregate query with neither one aggregate nor HAVING nor
    /// an aggregate-free GROUP BY (`tlp-outer`): `SELECT c0, count(c1) ..
    /// GROUP BY c0` partitioned in place, on `c1 > 2` in each part's WHERE,
    /// splits the groups and counts each part's rows (the in-place identity
    /// fails); partitioned on its result rows outside (`tlp_window`) it
    /// holds, on the GROUP BY term (which SQLite pushes down into the
    /// HAVING, then the WHERE) or on the count. So for two aggregates
    /// without GROUP BY, which `tlp_core` sends there too.
    #[test]
    fn tlp_partitions_an_aggregate_query_outside() {
        use crate::expr::{Agg, BinOp};
        use crate::model::Value;
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0, c1); INSERT INTO t0 VALUES (1, 1), (1, 5), (2, 3), (2, NULL), (3, 7), (NULL, 4);").unwrap();
        let count = Expr::Agg(Agg::Count, Some(Box::new(Expr::col(0, 1))));
        let q = Query { group: vec![Expr::col(0, 0)], ..Query::simple(0, false, Items::Exprs(vec![Expr::col(0, 0), count.clone()]), None) };
        let sorted = |mut r: Vec<Vec<Cell>>| {
            r.sort();
            r
        };
        let all = sorted(super::run(&db, &q.sql()).unwrap());
        let parts: Vec<String> = super::partitions("WHERE", "c1 > 2").iter().map(|p| q.sql().replace(" GROUP BY", &format!("{p} GROUP BY"))).collect();
        assert!(parts[1].contains("FROM t0 AS a0 WHERE NOT (c1 > 2) GROUP BY"), "{}", parts[1]);
        assert_ne!(sorted(super::concat(&db, &parts).unwrap()), all, "a WHERE partition splits the groups");
        for (x, materialized) in [("r0", false), ("r0 >= 2", true), ("r0 = 1", false), ("r1 < 2", true), ("r1", false)] {
            let base = format!("WITH tw(r0, r1) AS {}({}) ", if materialized { "MATERIALIZED " } else { "" }, q.sql());
            let parts: Vec<String> = super::partitions("WHERE", x).iter().map(|p| format!("{base}SELECT * FROM tw{p}")).collect();
            assert_eq!(sorted(super::concat(&db, &parts).unwrap()), all, "{x}");
        }
        let plan = super::run(&db, &format!("EXPLAIN QUERY PLAN WITH tw(r0, r1) AS ({}) SELECT * FROM tw WHERE r0 = 1", q.sql())).unwrap();
        assert!(!format!("{plan:?}").contains("MATERIALIZE"), "pushed down, not materialized: {plan:?}");
        let mut tally = super::Tally::default();
        super::tlp_window(&db, &q, super::TLP_OUTER, &mut tally).unwrap();
        assert_eq!((tally.comparisons, tally.variants.get("tlp-outer"), tally.variants.get("tlp-outer-union")), (2, Some(&1), Some(&1)));
        // Two aggregates without GROUP BY, through `tlp_core`, with a WHERE
        // clause kept inside.
        let (_, schema) = one_table(2);
        let max = Expr::Agg(Agg::Max, Some(Box::new(Expr::Cast(Box::new(Expr::col(0, 0)), crate::expr::CastType::Integer))));
        let filter = Some(Expr::bin(BinOp::Gt, Expr::col(0, 1), Expr::Lit(Value::Int(2))));
        let two = Query::simple(0, false, Items::Exprs(vec![count, max]), filter);
        let mut tally = super::Tally::default();
        super::tlp_core(&db, &schema, &two, &mut tally).unwrap();
        assert_eq!((tally.inapplicable, tally.variants.get("tlp-outer"), tally.variants.get("tlp-outer-union")), (0, Some(&1), Some(&1)));
    }

    /// A LIKE or GLOB pattern with a non-ASCII prefix written `('p' ||
    /// '')` (`patterns_computed`) is matched against every row in every
    /// plan, so an over-length UTF-8 value (the known SQLite behaviour of
    /// `invalid_text_pattern`) matches alike with the index and without,
    /// where the literal pattern's LIKE optimization skips it; a pattern
    /// whose prefix is ASCII is left as written.
    #[test]
    fn computed_patterns_match_over_length_utf8_in_every_plan() {
        use crate::expr::{Agg, BinOp};
        use crate::model::Value;
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0); CREATE INDEX t0_i0 ON t0(c0); INSERT INTO t0 VALUES (x'fba9'), (x'c3a9'), ('é!'), ('x');").unwrap();
        let glob = |p: &str, op| Query::simple(0, false, Items::Agg(Agg::Count, None), Some(Expr::bin(op, Expr::col(0, 0), Expr::Lit(Value::Text(p.into())))));
        let q = glob("é*", BinOp::Glob);
        let count = |sql: &str| super::run(&db, sql).unwrap();
        assert_ne!(count(&q.sql()), count(&q.sql_not_indexed()), "the literal's range scan skips x'fba9'");
        let c = super::patterns_computed(&q);
        assert!(c.sql().contains("('é*' || '')"), "{}", c.sql());
        assert_eq!(count(&c.sql()), vec![vec![Cell::Int(3)]]);
        assert_eq!(count(&c.sql()), count(&c.sql_not_indexed()));
        assert_eq!(count(&c.sql()), count(&q.sql_not_indexed()));
        let like = super::patterns_computed(&glob("é%", BinOp::Like));
        assert_eq!(count(&like.sql()), count(&like.sql_not_indexed()));
        for (p, op) in [("x*é", BinOp::Glob), ("_é%", BinOp::Like), ("x%", BinOp::NotLike)] {
            assert!(super::patterns_computed(&glob(p, op)) == glob(p, op), "{p}");
        }
    }

    /// `setops-self`: a query's SELECT as both members of a compound gives
    /// its rows twice (UNION ALL), as a set (UNION, INTERSECT) or none
    /// (EXCEPT), whatever the query: grouped, DISTINCT, with a named
    /// window, which each member then defines in a WINDOW clause of its
    /// own (`Query::sql` writes one, at the end, where it names the last
    /// member's: "no such window" for the first's).
    #[test]
    fn setops_compounds_a_query_with_itself() {
        use crate::expr::{Agg, Named, ROWID};
        let (_, schema) = one_table(2);
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0, c1); INSERT INTO t0 VALUES (1, 'b'), (NULL, 'a'), (1, 'b'), (1.0, 'b'), (2, NULL);").unwrap();
        let plain = Query::simple(0, false, Items::Exprs(vec![Expr::col(0, 0), Expr::col(0, 1)]), None);
        let distinct = Query { distinct: true, ..plain.clone() };
        let count = Expr::Agg(Agg::Count, None);
        let grouped = Query { group: vec![Expr::col(0, 1)], ..Query::simple(0, false, Items::Exprs(vec![Expr::col(0, 1), count]), None) };
        let w = Window {
            func: WinFunc::Count,
            arg: Some(Expr::col(0, 1)),
            default: None,
            filter: None,
            partition: vec![Expr::col(0, 0)],
            order: Vec::new(),
            frame: None,
            named: Named::Partition,
        };
        let window = Query::simple(0, false, Items::Exprs(vec![Expr::col(0, 0), Expr::Window(Box::new(w))]), None);
        assert!(window.sql().contains(" WINDOW "), "{}", window.sql());
        let compound = Query { compound: vec![(SetOp::UnionAll, window.clone())], ..window.clone() };
        assert!(super::run(&db, &compound.sql()).unwrap_err().contains("no such window"), "{}", compound.sql());
        let mut seen = std::collections::BTreeSet::new();
        for q in [plain, distinct, grouped, window] {
            // Every operator, by the draw of a query of another text.
            for k in 0..12 {
                let q = Query { filter: (k > 0).then(|| Expr::bin(crate::expr::BinOp::Ne, Expr::col(0, ROWID), Expr::Lit(crate::model::Value::Int(-k)))), ..q.clone() };
                let mut tally = super::Tally::default();
                super::setops_self(&db, &schema, &q, &mut tally).unwrap();
                let own = tally.variants.contains_key("setops-self-own");
                assert_eq!(tally.comparisons + tally.thinned, 1, "{}", q.sql());
                if tally.thinned == 0 {
                    seen.insert((super::fnv(&q.sql()) % 4, own));
                }
            }
        }
        assert_eq!(seen.iter().map(|s| s.0).collect::<std::collections::BTreeSet<_>>().len(), 4, "every operator");
        assert!(seen.iter().any(|s| s.1) && seen.iter().any(|s| !s.1), "against the query's rows and against themselves");
        // Rows of a compound with itself as each operator may and may not
        // give them.
        let (one, real, a) = (Cell::Int(1), Cell::Real(1f64.to_bits()), Cell::Text(b"a".to_vec()));
        let mut t = super::Tally::default();
        let mut check = |op, r: &[&Cell]| super::self_compound_rows(&mut t, op, "", &r.iter().map(|&c| vec![c.clone()]).collect()).is_ok();
        assert!(check(SetOp::UnionAll, &[&one, &real, &a, &a]), "1 and 1.0 alike");
        assert!(!check(SetOp::UnionAll, &[&one, &a, &a]));
        assert!(check(SetOp::Union, &[&one, &a, &Cell::Blob(b"a".to_vec())]), "text and blob apart");
        assert!(!check(SetOp::Intersect, &[&one, &real]));
        assert!(check(SetOp::Except, &[]) && !check(SetOp::Except, &[&one]));
    }

    /// `window-wrap` reads each SELECT of a query in a CTE with `count(*)`
    /// windows over its rows, a compound's members one by one: the
    /// compound in a CTE gives its columns its first member's affinity,
    /// which converts the second's integer 0 under a REAL column to 0.0.
    /// The counts are the variant's rows' and those of each partition,
    /// whose values SQLite tells apart as a set does (1 and 1.0 alike,
    /// the text 'a' and the blob x'61' apart, NULLs alike).
    #[test]
    fn window_wrap_counts_each_select_s_rows() {
        let (m, schema) = one_table(2);
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t0(c0 REAL, c1); INSERT INTO t0 VALUES (1, 'a'), (1.5, x'61'), (NULL, 'a'), (NULL, 1), (2, 1.0);").unwrap();
        let plain = Query::simple(0, false, Items::Exprs(vec![Expr::col(0, 1), Expr::col(0, 0)]), None);
        let zero = Query::simple(0, false, Items::Exprs(vec![Expr::col(0, 1), Expr::Lit(crate::model::Value::Int(0))]), None);
        let compound = Query { compound: vec![(SetOp::UnionAll, zero.clone())], ..plain.clone() };
        let whole = super::run(&db, &format!("WITH tw(r0, r1) AS ({}) SELECT r1 FROM tw WHERE typeof(r1) = 'real' AND r1 = 0", compound.sql())).unwrap();
        assert_eq!(whole.len(), 5, "the second member's 0 read as 0.0 through the compound: {whole:?}");
        for (q, cores) in [(plain, 1), (zero, 1), (compound, 2)] {
            let mut tally = super::Tally::default();
            super::window_wrap(&db, &m, &schema, &q, &mut tally).unwrap();
            let checked = if tally.thinned == 1 { 0 } else { cores };
            assert_eq!(*tally.variants.get("window-count").unwrap_or(&0), checked, "{}", q.sql());
            let mut tally = super::Tally::default();
            for c in q.cores() {
                super::window_wrap_core(&db, &m, &schema, &c, &mut tally).unwrap();
            }
            assert_eq!(tally.variants.get("window-count"), Some(&cores), "{}", q.sql());
            assert!(tally.variants.get("window-wrap").unwrap_or(&0) <= &cores, "{}", q.sql());
        }
        // The partitions on `c1` as the oracle counts them.
        let rows = super::run(&db, "SELECT c1, count(*) OVER (PARTITION BY c1) FROM t0").unwrap();
        for row in &rows {
            let same = rows.iter().filter(|r| r[0].distinct_key() == row[0].distinct_key()).count() as i64;
            assert_eq!(row[1], Cell::Int(same), "{rows:?}");
        }
        assert_eq!(rows.iter().filter(|r| r[1] == Cell::Int(2)).count(), 4, "'a' twice, 1 and 1.0: {rows:?}");
    }

    /// The optimizations' mask in force on `db` (`SQLITE_TESTCTRL_GETOPT`).
    fn mask_of(db: &Connection) -> u32 {
        let mut n: std::ffi::c_int = -1;
        // SAFETY: the handle is this open connection's; the test control
        // writes the one int it is given.
        unsafe { rusqlite::ffi::sqlite3_test_control(rusqlite::ffi::SQLITE_TESTCTRL_GETOPT, db.handle(), &mut n as *mut std::ffi::c_int) };
        n as u32
    }

    /// `plans-noopt`'s mask takes effect on the query it runs and is reset
    /// after it, on success and on error: a join SQLite plans with a Bloom
    /// filter and an automatic index has no Bloom filter with every
    /// optimization off or BloomFilter alone off (the automatic index is
    /// `PRAGMA automatic_index`'s, no optimization bit), and has it again
    /// after. A statement prepared before the mask keeps its plan (SQLite
    /// reads the mask when it prepares), so `run_without` must prepare
    /// afresh: it does, with the same SQL in rusqlite's statement cache.
    #[test]
    fn optimizations_are_off_under_the_mask_only() {
        use super::{run, run_without, Optimizations, ALL_OPTIMIZATIONS, OPTIMIZATIONS};
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t1(a, b); CREATE TABLE t2(a, b); INSERT INTO t1 VALUES (1, 2); INSERT INTO t2 VALUES (1, 3);").unwrap();
        let eqp = "EXPLAIN QUERY PLAN SELECT * FROM t1, t2 WHERE t1.a = t2.a";
        let plan = |o: super::Outcome| format!("{:?}", o.unwrap());
        let bloom = 1u32 << (OPTIMIZATIONS.iter().find(|o| o.1 == "BloomFilter").unwrap().0 - 1);
        assert_eq!(bloom, 0x0008_0000);
        let default = plan(run(&db, eqp));
        assert!(default.contains("BLOOM FILTER") && default.contains("AUTOMATIC"), "{default}");
        // The statement cache holds the query, prepared with every optimization on.
        let mut cached = db.prepare_cached(eqp).unwrap();
        for mask in [ALL_OPTIMIZATIONS, bloom] {
            let off = plan(run_without(&db, eqp, mask));
            assert!(!off.contains("BLOOM FILTER") && off.contains("AUTOMATIC"), "{mask:#x}: {off}");
            assert_eq!(mask_of(&db), 0);
            assert_eq!(plan(run(&db, eqp)), default);
        }
        // Prepared before the mask: the plan stays the one prepared.
        {
            let _off = Optimizations::off(&db, ALL_OPTIMIZATIONS);
            assert_eq!(mask_of(&db), ALL_OPTIMIZATIONS);
            let rows: Vec<String> = cached.query_map([], |r| r.get::<_, String>(3)).unwrap().map(Result::unwrap).collect();
            assert!(rows.iter().any(|r| r.contains("BLOOM FILTER")), "{rows:?}");
        }
        assert_eq!(mask_of(&db), 0);
        // An error resets the mask too.
        assert!(run_without(&db, "SELECT * FROM nowhere", ALL_OPTIMIZATIONS).is_err());
        assert_eq!(mask_of(&db), 0);
        assert_eq!(plan(run(&db, eqp)), default);
        // The names are 31, distinct bits 1..=32 but 21, each variant named after its optimization.
        let bits: std::collections::BTreeSet<u32> = OPTIMIZATIONS.iter().map(|o| o.0).collect();
        assert_eq!(bits, (1..=32).filter(|&b| b != 21).collect());
        assert!(OPTIMIZATIONS.iter().all(|(_, name, variant)| *variant == format!("plans-noopt-{name}")));
    }

    /// `plans-noopt` agrees with the query as written on correct SQLite: a
    /// few queries by hand whose plans the mask changes (a flattened
    /// subquery, a Bloom filter, constant propagation, min/max, an indexed
    /// ORDER BY, a pushed-down WHERE), and drawn inputs, where it compares.
    #[test]
    fn noopt_agrees_on_correct_queries() {
        use super::{noopt, run, run_without, Mode, Tally};
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE t1(a, b); CREATE INDEX t1a ON t1(a); CREATE TABLE t2(a, b);
             INSERT INTO t1 VALUES (1, 2), (2, 3), (3, NULL), (NULL, 4); INSERT INTO t2 VALUES (1, 3), (3, 5), (3, 6);",
        )
        .unwrap();
        let queries = [
            ("SELECT x.a, x.b FROM (SELECT a, b FROM t1 WHERE b > 2) AS x", Mode::Multiset),
            ("SELECT t1.a, t2.b FROM t1, t2 WHERE t1.a = t2.a", Mode::Multiset),
            ("SELECT * FROM t1 WHERE a = 3 AND b IS a + 1", Mode::Multiset),
            ("SELECT max(a) FROM t1", Mode::Scalar),
            ("SELECT a, b FROM t1 ORDER BY a, b", Mode::Ordered(true)),
            ("SELECT * FROM (SELECT a, b FROM t1 UNION ALL SELECT a, b FROM t2) WHERE a > 1", Mode::Multiset),
        ];
        for (sql, mode) in queries {
            let mut t = Tally::default();
            noopt(&mut t, super::ALL_OPTIMIZATIONS, mode, sql, &run(&db, sql), &|mask| run_without(&db, sql, mask), &|_| false).unwrap();
            assert_eq!(t.variants["plans-noopt"], 1);
        }
        let mut t = Tally::default();
        for (case, queries) in drawn(60) {
            t.add(&super::plans(&case, &queries).unwrap());
        }
        assert!(t.variants["plans-noopt"] > 50, "{:?}", t.variants);
        assert!(t.variants.keys().all(|k| !k.starts_with("plans-noopt-")));
    }

    /// The culprit of a `plans-noopt` disagreement, on a stand-in for
    /// SQLite whose result is wrong with some optimizations on: one that
    /// alone reproduces it names the variant and the run shown; several, or
    /// none alone, leave it `plans-noopt`; each message ends with its mask.
    #[test]
    fn noopt_names_the_one_optimization_that_explains_it() {
        use super::{noopt, Cell, Mode, Outcome, Tally};
        const BLOOM: u32 = 0x0008_0000;
        const PROPAGATE: u32 = 0x0000_8000;
        let orig: Outcome = Ok(vec![vec![Cell::Int(1)]]);
        let failure_with = |all: u32, wrong: &dyn Fn(u32) -> Option<Outcome>| {
            let mut t = Tally::default();
            let r = noopt(&mut t, all, Mode::Multiset, "SELECT 1", &orig, &|mask| wrong(mask).unwrap_or_else(|| orig.clone()), &|_| false);
            assert_eq!((t.comparisons, t.variants.len()), (1, 1), "{:?}", t.variants);
            r.err()
        };
        let failure = |wrong: &dyn Fn(u32) -> Option<Outcome>| failure_with(super::ALL_OPTIMIZATIONS, wrong);
        let lines = |m: Option<String>| {
            let m = m.expect("a failure");
            let v: Vec<String> = m.lines().map(str::to_string).collect();
            (v[0].clone(), v.last().unwrap().clone(), v.len())
        };
        let two = Ok(vec![vec![Cell::Int(2)]]);
        assert_eq!(failure(&|_| None), None);
        let (head, last, n) = lines(failure(&|mask| (mask & BLOOM != 0).then(|| two.clone())));
        assert_eq!((head.as_str(), last.as_str(), n), ("results differ (plans-noopt-BloomFilter)", "  optimizations off: 0x00080000 (BloomFilter)", 6));
        let (head, last, _) = lines(failure(&|mask| (mask & BLOOM != 0).then(|| Err("boom".to_string()))));
        assert_eq!((head.as_str(), last.as_str()), ("the variant fails, the original does not (plans-noopt-BloomFilter)", "  optimizations off: 0x00080000 (BloomFilter)"));
        let (head, last, _) = lines(failure(&|mask| (mask & (BLOOM | PROPAGATE) != 0).then(|| two.clone())));
        assert_eq!((head.as_str(), last.as_str()), ("results differ (plans-noopt)", "  optimizations off: 0xffffffff (all; alone, each of PropagateConst 0x00008000, BloomFilter 0x00080000)"));
        let (head, last, _) = lines(failure(&|mask| (mask == super::ALL_OPTIMIZATIONS).then(|| two.clone())));
        assert_eq!((head.as_str(), last.as_str()), ("results differ (plans-noopt)", "  optimizations off: 0xffffffff (all; none alone)"));
        // A mask that leaves an optimization on never turns it off, nor names it.
        const COROUTINES: u32 = 0x0200_0000;
        assert_eq!(failure_with(!BLOOM, &|mask| (mask & BLOOM != 0).then(|| two.clone())), None);
        let (head, last, _) = lines(failure_with(!COROUTINES, &|mask| (mask & (BLOOM | PROPAGATE | COROUTINES) != 0).then(|| two.clone())));
        assert_eq!((head.as_str(), last.as_str()), ("results differ (plans-noopt)", "  optimizations off: 0xfdffffff (all but Coroutines; alone, each of PropagateConst 0x00008000, BloomFilter 0x00080000)"));
        // The one optimization's run is the one shown.
        let m = failure(&|mask| match mask {
            BLOOM => Some(Ok(vec![vec![Cell::Int(3)]])),
            m if m & BLOOM != 0 => Some(two.clone()),
            _ => None,
        })
        .unwrap();
        assert!(m.contains("    -> Ok([[3]])") && !m.contains("[[2]]"), "{m}");
    }

    /// SQLite's coalesce() for a FULL JOIN's USING or NATURAL column has the
    /// left column's collating sequence (3.53.2 on), a written coalesce() none:
    /// `plans-on` writes the sequence out. In a UTF-16 database RTRIM (UTF-8
    /// only) orders texts by code point, BINARY by UTF-16 units, so '中'
    /// (U+4E2D) sorts after 'A' under one and before it under the other.
    #[test]
    fn plans_on_keeps_a_natural_full_join_column_collation() {
        use crate::expr::{Items, Join, JoinKind, JoinOp, Query, Schema};
        use crate::model::{Coll, Encoding, Key, Model, Table};
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "PRAGMA encoding = 'UTF-16le'; CREATE TABLE t0(c0 COLLATE RTRIM); CREATE TABLE t1(c0);
             INSERT INTO t0 VALUES ('A'), ('中'); INSERT INTO t1 VALUES ('x');",
        )
        .unwrap();
        let (mut t0, t1) = (Table::new(1, Key::Rowid), Table::new(1, Key::Rowid));
        t0.coll[0] = Some(Coll::Rtrim);
        let m = Model { encoding: Encoding::Utf16le, tables: vec![t0, t1], views: Vec::new(), triggers: Vec::new(), savepoints: Vec::new(), foreign_keys: true, rules: Vec::new(), recursive_triggers: false, session: crate::model::Session::default(), vtabs: Vec::new() };
        let schema = Schema::of(&m);
        let mut q = Query::simple(0, false, Items::Star, None);
        q.joins.push(Join { table: 1, kind: JoinKind::Natural(JoinOp::Full) });
        q.order.push(Order { key: OrderKey::Column(0), desc: false, nulls: None, by: By::Number });
        let (_, on) = super::join_variants(&q, &schema).into_iter().find(|(n, _)| *n == "plans-on").unwrap();
        assert!(on.contains("coalesce(a0.c0, a1.c0) COLLATE RTRIM"), "{on}");
        let text = |t: &str| Cell::Text(t.as_bytes().to_vec());
        let want = vec![vec![text("A")], vec![text("x")], vec![text("中")]];
        assert_eq!(super::run(&db, &q.sql()).unwrap(), want);
        assert_eq!(super::run(&db, &on).unwrap(), want);
        let bare = on.replace(" COLLATE RTRIM", "");
        assert_eq!(super::run(&db, &bare).unwrap(), vec![vec![text("中")], vec![text("A")], vec![text("x")]]);
    }

    /// The SQLite bug behind [`super::concat_blob_utf16`] (the shrunk
    /// counterexample of `query_plans_agree` at fuzz 1cc3a23, seed
    /// 410433389429008914): in a UTF-16be database concat() copies a BLOB
    /// argument's size in bytes of its UTF-8 text, so `x'3f'` (whose text is
    /// '') adds its terminator on the first row and nothing after, and the
    /// table and its index visit t1's rows in another order. Valid UTF-16
    /// too: `concat(x'0041')` is 'A' and a NUL. When this fails, SQLite has
    /// fixed it and the rewrite can go. Only a UTF-16 database's query with
    /// an argument that may be a BLOB is rewritten, below the query too, and
    /// as [`super::concat_as_text`] writes it both plans agree.
    #[test]
    fn concat_of_a_blob_in_utf16_is_checked_as_text() {
        use crate::expr::{CastType, Func};
        use crate::model::{Encoding, Key, Model, Table, Value};
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "PRAGMA encoding = 'UTF-16be'; CREATE TABLE t0(c0); CREATE TABLE t1(c0); CREATE INDEX t1_i0 ON t1(c0 DESC);
             INSERT INTO t1 VALUES (NULL), (0);",
        )
        .unwrap();
        let text = |t: &[u8]| vec![Cell::Text(t.to_vec())];
        let table = super::run(&db, "SELECT concat(x'3f', c0) FROM t1 AS a0").unwrap();
        let index = super::run(&db, "SELECT concat(x'3f', c0) FROM t1 AS a0 INDEXED BY t1_i0").unwrap();
        assert_eq!((table, index), (vec![text(b"\0"), text(b"0")], vec![text(b"\x000"), text(b"")]));
        assert_eq!(super::run(&db, "SELECT concat(x'0041'), concat(CAST(x'0041' AS TEXT))").unwrap(), vec![vec![Cell::Text(b"A\0".to_vec()), Cell::Text(b"A".to_vec())]]);
        let model = |encoding| Model { encoding, tables: vec![Table::new(1, Key::Rowid), Table::new(1, Key::Rowid)], views: Vec::new(), triggers: Vec::new(), savepoints: Vec::new(), foreign_keys: true, rules: Vec::new(), recursive_triggers: false, session: crate::model::Session::default(), vtabs: Vec::new() };
        let query = |args: Vec<Expr>| Query::simple(1, false, Items::Exprs(vec![Expr::Call(Func::Concat, args)]), None);
        let found = query(vec![Expr::Lit(Value::Blob(vec![0x3f])), Expr::col(0, 0)]);
        assert!(super::concat_blob_utf16(&model(Encoding::Utf16be), &found));
        assert!(super::concat_blob_utf16(&model(Encoding::Utf16le), &query(vec![Expr::col(0, 0)])));
        assert!(!super::concat_blob_utf16(&model(Encoding::Utf8), &found));
        let texts = query(vec![Expr::Lit(Value::Text("a".into())), Expr::Cast(Box::new(Expr::col(0, 0)), CastType::Text)]);
        assert!(!super::concat_blob_utf16(&model(Encoding::Utf16be), &texts));
        let sql = super::concat_as_text(&found).sql();
        assert_eq!(sql, "SELECT concat(CAST(x'3f' AS TEXT), CAST(c0 AS TEXT)) FROM t1 AS a0");
        let want = vec![text(b""), text(b"0")];
        assert_eq!(super::run(&db, &sql).unwrap(), want);
        assert_eq!(super::run(&db, &sql.replace("a0", "a0 INDEXED BY t1_i0")).unwrap(), vec![text(b"0"), text(b"")]);
        // Below the query: in a scalar subquery of its WHERE clause.
        let mut outer = Query::simple(0, false, Items::Star, None);
        outer.filter = Some(Expr::Scalar(Box::new(found)));
        assert!(super::concat_blob_utf16(&model(Encoding::Utf16be), &outer));
        assert!(!super::concat_blob_utf16(&model(Encoding::Utf16be), &super::concat_as_text(&outer)));
    }

    /// A query [`super::concat_as_text`] rewrites is scoped as drawn
    /// ([`super::as_checked`]). Found as run 20261016-1828 (all eight
    /// oracles, fresh draws too): "a CTE of the query is not valid" for CTE
    /// bodies such as `SELECT max(CAST(concat_ws(',', a0.c0) AS INTEGER))
    /// AS c0 FROM t0 AS a0` over a NOCASE c0 in a UTF-16 database, whose
    /// rewritten `CAST(a0.c0 AS TEXT)` inside the aggregate's argument
    /// `casts_binary` rejects; the rewrite reads alike on SQLite.
    #[test]
    fn a_query_rewritten_for_concat_is_scoped_as_drawn() {
        use crate::expr::{Agg, CastType, Cte, CteBody, Func, Query, Schema, CTE};
        use crate::model::{Coll, Encoding, Key, Model, Table, Value};
        let mut t0 = Table::new(1, Key::Rowid);
        t0.coll[0] = Some(Coll::Nocase);
        let m = Model { encoding: Encoding::Utf16le, tables: vec![t0], views: Vec::new(), triggers: Vec::new(), savepoints: Vec::new(), foreign_keys: true, rules: Vec::new(), recursive_triggers: false, session: crate::model::Session::default(), vtabs: Vec::new() };
        let schema = Schema::of(&m);
        let concat = Expr::Call(Func::ConcatWs, vec![Expr::Lit(Value::Text(",".into())), Expr::col(0, 0)]);
        let item = Expr::Agg(Agg::Max, Some(Box::new(Expr::Cast(Box::new(concat), CastType::Integer))));
        let body = Query::simple(0, false, Items::Exprs(vec![item]), None);
        let mut q = Query::simple(CTE, false, Items::Star, None);
        q.with.push(Cte { body: CteBody::Select(Box::new(body)), hint: None, columns: false });
        assert!(q.valid(&schema), "{}", q.sql());
        assert!(super::concat_blob_utf16(&m, &q));
        let rewritten = super::concat_as_text(&q);
        assert!(rewritten.sql().contains("max(CAST(concat_ws(',', CAST(c0 AS TEXT)) AS INTEGER))"), "{}", rewritten.sql());
        assert!(rewritten.scope(&schema).is_none(), "the rewrite is valid now: scoping it would do");
        let (checked, scoped) = super::as_checked(&m, &schema, &q).unwrap();
        assert_eq!((checked.into_owned(), scoped), (rewritten.clone(), q.scope(&schema).unwrap()));
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("PRAGMA encoding = 'UTF-16le'; CREATE TABLE t0(c0 COLLATE NOCASE); INSERT INTO t0 VALUES ('7'), (x'3800'), ('a'), (NULL);").unwrap();
        assert_eq!(super::run(&db, &q.sql()).unwrap(), super::run(&db, &rewritten.sql()).unwrap());
    }

    /// The SQLite bug behind `noopt_mask` keeping Coroutines on under
    /// `PRAGMA reverse_unordered_selects`: without co-routines SQLite
    /// materializes a window's sorted input and scans it in reverse, so a
    /// running sum runs backwards. When this fails, SQLite has fixed it and
    /// the exception can go.
    #[test]
    fn sqlite_reverses_materialized_window_input_without_coroutines() {
        use super::{noopt_mask, run, run_without, ALL_OPTIMIZATIONS};
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE t1(c0); INSERT INTO t1 VALUES (1), (2), (3);").unwrap();
        let sql = "SELECT c0, sum(c0) OVER (ORDER BY c0) FROM t1";
        let int = |v: &[[i64; 2]]| v.iter().map(|r| r.iter().map(|&x| Cell::Int(x)).collect::<Vec<_>>()).collect::<Vec<_>>();
        let sorted = |mut r: Vec<Vec<Cell>>| {
            r.sort();
            r
        };
        let right = int(&[[1, 1], [2, 3], [3, 6]]);
        assert_eq!(noopt_mask(&db), ALL_OPTIMIZATIONS);
        assert_eq!(sorted(run_without(&db, sql, ALL_OPTIMIZATIONS).unwrap()), right);
        db.execute_batch("PRAGMA reverse_unordered_selects = 1").unwrap();
        assert_eq!(sorted(run(&db, sql).unwrap()), right);
        assert_eq!(noopt_mask(&db), 0xfdff_ffff);
        assert_eq!(sorted(run_without(&db, sql, noopt_mask(&db)).unwrap()), right);
        let wrong = run_without(&db, sql, 0x0200_0000).unwrap();
        assert_eq!(sorted(wrong), int(&[[1, 6], [2, 5], [3, 3]]), "SQLite may have fixed it: remove the exception");
    }
}
