# pgsql-bugs report: nested BETWEEN, parse analysis and cancellation

Status: submitted 2026-10-14 as BUG #19758, https://www.postgresql.org/message-id/19758-d074798b35cc9215%40postgresql.org (version field 18.6).
https://www.postgresql.org/account/submitbug/ (attach cfi.patch or paste it inline). Both tested builds have
assertions on; release-build numbers have not been measured.

## Subject

Nested BETWEEN grows exponentially in parse analysis, and statement_timeout / pg_cancel_backend cannot interrupt it

## Body (plain text)

```
Hello,

A short query with nested BETWEEN expressions makes parse analysis take
time and memory exponential in the nesting depth, and neither
statement_timeout nor pg_cancel_backend() takes effect until parse
analysis has finished.

Versions tested (both built from git with --enable-cassert --enable-debug,
CFLAGS="-O2 -g"; default postgresql.conf except fsync=off, empty .psqlrc):
  - master 158b6d5109f: PostgreSQL 20devel on aarch64-apple-darwin25.6.0,
    compiled by Apple clang version 21.0.0 (clang-2100.3.34.2), 64-bit
  - REL_18_STABLE a85b5523e67: PostgreSQL 18.6 on aarch64-apple-darwin25.6.0,
    same compiler
Platform: macOS 26.7 (Darwin 25.6.0), Apple M5 Pro (arm64), 64 GB RAM.

Reproduction (no tables needed; 521-byte query, depth 20):

  \timing on
  SET statement_timeout = '100ms';
  SELECT 1 WHERE ((((((((((((((((((((true BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false) BETWEEN true AND false);

Actual:
  ERROR:  canceling statement due to statement timeout
  Time: 1642.107 ms (00:01.642)          -- master; 1604.506 ms on 18.6
The backend's RSS reaches about 1.65 GB first. With log_parser_stats = on,
PARSE ANALYSIS reports 1.42 s elapsed, i.e. the timeout is acted on only
once parse analysis returns.

Expected: the statement is cancelled shortly after 100 ms.

The same happens with pg_cancel_backend() from another session: sent at
0.21 s, the query still ran to 1.63 s (the same as without a cancel) and
then reported "canceling statement due to user request". EXPLAIN and
PREPARE of the same query behave the same (PREPARE at depth 20 with a
100 ms timeout: cancelled after 1732 ms; PARSE ANALYSIS 1.40 s).

Scaling (master; 18.6 is within a few percent). analyze = PARSE ANALYSIS
elapsed from log_parser_stats, RSS = backend peak:

  per level: (e BETWEEN true AND false)
    depth 14:  0.017 s,   19 MB
    depth 16:  0.073 s,  111 MB
    depth 18:  0.353 s,  419 MB
    depth 20:  1.445 s, 1652 MB      (2x per level; 25 bytes per level)
  per level: (e BETWEEN SYMMETRIC true AND false)
    depth  6:  0.003 s,   16 MB
    depth  8:  0.048 s,   93 MB
    depth 10:  0.892 s, 1729 MB      (4x per level; 371-byte query)
  per level: (e NOT BETWEEN true AND false)
    depth 18:  0.327 s,  622 MB; depth 20 exceeded 2 GB (I stopped it)

Non-nested use is linear, so ordinary queries are not affected:
16000 side-by-side "(true BETWEEN true AND false) OR ..." (528 kB) take
0.007 s of parse analysis; one BETWEEN with a 4000-term left operand takes
0.002 s.

Cause: in src/backend/parser/parse_expr.c, transformAExprBetween()
expands "a BETWEEN b AND c" into "a >= b AND a <= c" by copyObject()ing the
raw left operand and transforming each copy separately (two copies for
[NOT] BETWEEN, four of the left operand plus two of each bound for
[NOT] BETWEEN SYMMETRIC). When the left operand itself contains a
BETWEEN, the work doubles (or quadruples) at each level. The XXX comment
there already notes the multiple evaluation; the size blowup for nested
cases seems to be a further consequence. transformExprRecurse() calls
check_stack_depth() but never CHECK_FOR_INTERRUPTS(), and there is no
CHECK_FOR_INTERRUPTS() anywhere in src/backend/parser or
src/backend/nodes, so a pending cancel or timeout waits for the whole
transform.

A minimal mitigation is to let the recursion be cancelled:

--- a/src/backend/parser/parse_expr.c
+++ b/src/backend/parser/parse_expr.c
@@ -145,6 +145,9 @@ transformExprRecurse(ParseState *pstate, Node *expr)
 	/* Guard against stack overflow due to overly complex expressions */
 	check_stack_depth();
 
+	/* Also allow complex expressions to be cancelled */
+	CHECK_FOR_INTERRUPTS();
+
 	switch (nodeTag(expr))
 	{
 		case T_ColumnRef:

With this on master, the reproduction above is cancelled after 111 ms
(peak RSS 162 MB). A pg_cancel_backend() sent at 0.22 s ends the query at
0.25 s. PREPARE is cancelled at 137 ms. "make check" passes (239 tests).
This does not remove the exponential growth itself; that would need a
BETWEEN representation that does not duplicate its operands. Transforming
the operand once and then copying the result would not help, because the
copies are just as large.

Severity, to be fair about it: the input is machine-generated (it came
from a query fuzzer; the original was a 240 kB generated query that ran
far past a 10 s statement_timeout until the OS killed the backend), and
hand-written SQL rarely nests BETWEEN inside a BETWEEN operand. But a
few hundred bytes of SQL from any user who can run queries are enough to
make a backend allocate memory faster than 1 GB/s, where
statement_timeout and query cancel do not help. Where the OOM killer
ends such a backend, the postmaster restarts all sessions.

A smaller related effect: nested single-item IN lists, e.g.
"((true IN (true)) IN (true)) ...", grow quadratically, because
transformAExprIn copies the already transformed left operand for each list
element. 4000 levels (48 kB) take 0.43 s and 1.85 GB of parse analysis;
two-item lists (ScalarArrayOpExpr) stay linear (0.0016 s).

Regards,
<name>
```

## Verification

The commands below run from `verify/` (outputs are in that folder and `raw/`). Every heavy run went through
`wd.py`, which kills the backend above 2 GB RSS or after 60 s. Each depth ran in a fresh session.

| Claim | Command | Output |
|---|---|---|
| Versions and platform | `select version()`, `pg_config --configure`, `sw_vers`, `sysctl` | in the report header |
| Scaling for BETWEEN, SYMMETRIC and NOT BETWEEN on both builds | `sweep.py {master,18} between 14..20`, `notbetween 14 16 18 20`, `betweensym 6..10` | `sweep_between.txt` |
| Timeout fires at 1642 ms (master) and 1604 ms (18.6), after 1.42 s of parse analysis | `wd.py T ../minimal.sql`, plus a run with `log_parser_stats` | `timeout_cancel.txt`, `raw/*-to-stats.out` |
| pg_cancel_backend waits (cancel sent at 0.21 s, query ends at 1.63 s) | `wd.py T b20.sql --cancel-after 0.2` | `timeout_cancel.txt` |
| EXPLAIN and BETWEEN SYMMETRIC at depth 10 are also cancelled late | `wd.py` with `statement_timeout='100ms'` | `timeout_cancel.txt` |
| PREPARE is cancelled at 1732 ms (master) and 1788 ms (18.6), after 1.40–1.44 s of parse analysis; memory is freed afterwards | `wd.py T prepare_depth20.sql` | `prepare.txt`, `raw/*-p20.out` |
| Side-by-side BETWEENs and a large left operand stay linear; IN (x) is quadratic, two-item IN is linear | `wsweep.sh T {or,orsym,biglhs,biglhssym,in1,in2} N...` | `wide_and_in.txt` |
| With the patch: 111 ms, cancel at 0.25 s, PREPARE at 137 ms | the same `wd.py` runs against the patched build | `timeout_cancel.txt`, `prepare.txt` |
| Original 240 KB query: master fires at 100 ms but not at 300–700 ms (killed at 2 GB); the patched build fires at every setting | `wd.py T ../query.sql` after loading `setup.sql` | `original_query.txt` |
| `make check` with the patch passes all 239 tests | `make check` in a /tmp worktree | `make_check_cfi.log` |
| No interrupt check in parser or nodes | `grep -rn CHECK_FOR_INTERRUPTS src/backend/parser src/backend/nodes` (empty); `git log -S` (empty) | |
| History | `git show 34af082f95a` (9.5) and `6a75562ed16`; no later change to BETWEEN or interrupt checks in parse_expr.c since 2015 | |
| Exponential growth, rechecked in the main session (master) | EXPLAIN at depths 12, 14, 16: 7, 21, 93 ms | |

Corrections to triage.md from this write-up: the original query is cancellable at very small timeouts (100 ms
fired at 0.34 s) but not at 300 ms and above; NOT BETWEEN also inflates planning; 18.6 now checked and identical;
the pre-2015 history claim is untested and left out.

## Duplicate search

See the end of this file, filled in by the main session.

### Done (main session, 2026-10-13)

- https://www.postgresql.org/search/?m=1&q=BETWEEN+exponential: 20 weak matches (scores 0.00 to 0.01), none about
  BETWEEN, nested BETWEEN or parse-analysis blowup (autovacuum scheduling, row pattern recognition, SQL/PGQ, repeat()
  speedup).
- Earlier, from triage: E1YPbkd-0002f5-I7@gemulon.postgresql.org (commit 34af082f95a) and
  16054.1424619402@sss.pgh.pa.us acknowledge the operand duplication but do not report this blowup.

### Still to do by hand (the tool's safety filter refused these requests)

- https://www.postgresql.org/search/?m=1&q=transformAExprBetween
- https://www.postgresql.org/search/?m=1&q=parse+analysis+CHECK_FOR_INTERRUPTS+statement_timeout
