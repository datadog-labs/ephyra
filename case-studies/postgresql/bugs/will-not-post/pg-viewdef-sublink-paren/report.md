# pgsql-bugs report draft: pretty-mode deparse drops the parentheses around the left operand of IN (subquery)

Status: **duplicate, do not submit as is.** Henson Choi reported the same bug on pgsql-hackers on 2026-10-13,
with a test patch and a fix patch: "[DEPARSE] Pretty-printed sublink test expressions lose needed parentheses",
https://www.postgresql.org/message-id/CAAAe_zCy+D45Qd_br484F+V34y0BVA=hp_7cb=u6HysyMkE3UQ@mail.gmail.com
(no replies at fetch time 2026-10-16; not committed on master a261fc2b7220). The draft is kept for the record.
A short reply on that thread fits better (see "Possible reply"). Display only, no security angle.

## Form fields

- PostgreSQL version: `18.6`
- Operating system: `Ubuntu 24.04 x86_64 (also seen on macOS arm64)`
- Short description: `pg_get_viewdef(..., true) drops needed parentheses around the left operand of IN (subquery)`

## Body (plain text)

```
Hello,

In pretty mode, ruleutils prints the left operand of IN (subquery),
NOT IN, op ANY (subquery) and op ALL (subquery) without parentheses,
even when it is an operator expression that binds less tightly than
IN or the combining operator. The printed text then fails to parse or
type-check, or it parses as a different expression.

  CREATE TABLE t (a int, b bool);
  INSERT INTO t VALUES (1, true);
  CREATE VIEW v AS SELECT (a = 1) IN (SELECT b FROM t) AS r FROM t;
  CREATE VIEW w AS SELECT (b < b) IN (SELECT false) AS r FROM t;
  SELECT pg_get_viewdef('v', true);
  --  SELECT (a = 1 IN ( SELECT t_1.b FROM t t_1)) AS r FROM t;
  SELECT pg_get_viewdef('w', true);
  --  SELECT (b < b IN ( SELECT false)) AS r FROM t;
  SELECT * FROM w;                                 -- t
  SELECT (b < b IN ( SELECT false)) AS r FROM t;   -- f
  SELECT (a = 1 IN ( SELECT t_1.b FROM t t_1)) AS r FROM t;
  -- ERROR:  operator does not exist: integer = boolean

IN binds more tightly than the comparison operators, so the text reads
as b < (b IN (...)). pg_get_viewdef('w', false) prints
((b < b) IN ( SELECT false)), which is correct.

Other left operands that are printed wrongly: <=, >=, <>, IS [NOT]
DISTINCT FROM, NOT, AND, OR, and BETWEEN (printed as AND). For example
(b OR true) IN (SELECT false) prints as (b OR true IN ( SELECT false)),
which gives other rows, and (b < b) <> ALL (SELECT true) prints as
(b < b <> ALL ( SELECT true)), a syntax error. Postfix forms such as
IS NULL, IS TRUE and IS JSON, and Op-level operators such as +, || and
~~ (LIKE), read back correctly. Row forms like (x, y) IN (SELECT ...)
are fine, because the commas separate the elements.

Affected: pg_get_viewdef(..., true) (also for materialized views and
the wrap-column variant), pg_get_ruledef(..., true) for rule actions
and WHERE quals, pg_get_expr(..., true) on policy quals, psql's \d+
"View definition" and the "Rules:" section of \d. Not affected: the
non-pretty forms, pg_dump (a dump/restore keeps the parentheses and the
restored views give the same rows), the pg_policies view, psql's policy
lines, and SQL-standard function bodies. CHECK constraints, index
expressions and predicates, trigger WHEN, generated columns and DEFAULT
reject subqueries, so they can't show it.

So this is a display problem, but psql \d+ shows a definition that does
not mean what the view does, and copying it back to recreate the view
fails or silently changes the results.

Reproduced on master a261fc2b722, REL_19_STABLE 1212d5d33b1 (19rc1),
REL_18_STABLE dfbc1507cdb (18.6) and REL_14_STABLE 14904052e65 (14.24),
all --enable-cassert, Ubuntu 24.04 x86_64. It was first seen on master
and 18.6 on macOS.

Cause: get_sublink_expr() prints the left operand of a single combining
operator with get_rule_expr(). In pretty mode OpExpr, BoolExpr,
NullTest, DistinctExpr etc. don't print their own parentheses and
expect the parent to use get_rule_expr_paren(). The = < > / NOT / AND /
OR cases date back to the pretty-printing code (7.4). The <= >= <> and
IS cases date from 9.5's operator precedence change (c6b3c939b7e),
which moved those operators below IN.

Using get_rule_expr_paren() with the SubLink as parent fixes it.
isSimpleNode() has no SubLink-parent case, so every operator-like left
operand gets parentheses (harmless extras for a + 1 and for a nested
sublink); plain columns, constants and function calls print as before.
"make check" passes, and no existing expected output changes apart from
the new test columns:

--- a/src/backend/utils/adt/ruleutils.c
+++ b/src/backend/utils/adt/ruleutils.c
@@ -11942,7 +11942,15 @@ get_sublink_expr(SubLink *sublink, deparse_context *context)
 			/* single combining operator */
 			OpExpr	   *opexpr = (OpExpr *) sublink->testexpr;
 
-			get_rule_expr(linitial(opexpr->args), context, true);
+			/*
+			 * In pretty mode the lefthand expression may need parentheses:
+			 * IN binds more tightly than comparison operators, and both IN
+			 * and "op ANY/ALL" bind more tightly than IS, NOT, AND and OR.
+			 * (The multiple-operator cases below separate the lefthand
+			 * expressions with commas, so they need no such care.)
+			 */
+			get_rule_expr_paren(linitial(opexpr->args), context, true,
+								(Node *) sublink);
 			opname = generate_operator_name(opexpr->opno,
 											exprType(linitial(opexpr->args)),
 											exprType(lsecond(opexpr->args)));

(The full patch, with a regression test added to tt26v in
create_view.sql, is attached.)

Found with a query fuzzer.

Regards,
<name>
```

## Possible reply on the hackers thread (instead of a new report)

Points the thread doesn't make, all checked here: REL_14_STABLE (14.24) is affected too (the thread proposes a
back-patch to 15); the =/</>/NOT/AND/OR cases date back to 7.4's pretty mode (52347b66370), and 9.5's
c6b3c939b7e added the <=/>=/<>/IS cases; pretty pg_get_ruledef (so psql \d "Rules:") and
pg_get_expr(polqual, polrelid, true) are affected as well. pg_policies and psql's policy lines are not.

## Verification

All on workspace-ephyra-fuzz-pg (Ubuntu x86_64), fresh cassert builds from ~/verify-builds/build2.sh, throwaway
clusters (run.sh). Raw outputs are in raw/, and VALIDATION.md has each run.

- Reproduction (minimal.sql: pretty text wrong; re-running it errors or gives f where the view gives t): master
  a261fc2b7220, REL_19_STABLE 1212d5d33b17, REL_18_STABLE dfbc1507cdb5 (18.6), REL_14_STABLE 14904052e653
  (14.24). raw/min-r19.out, min-r18.out, min-r14.out; VALIDATION.md section 1.
- Extent (46 cases, pretty text re-executed and rows compared; non-pretty text matched every time):
  extent.sql; raw/extent-master.tsv and -r19/-r18/-r14. Verdicts are identical on all four branches, except
  that IS JSON doesn't exist on 14.
  FAILS: =, <=, <> on ints, IS [NOT] DISTINCT FROM on ints, BETWEEN, COLLATE'd comparison, (a = 1) NOT IN,
  (a = 1) < ANY, (b < b) <> ALL.
  CHANGED: < and > on bools, IS DISTINCT FROM on bools, NOT, AND, OR, NOT IN with < or OR, OR <> ANY,
  NOT < ANY, AND < ALL, OR <> ALL.
  fine: IS [NOT] NULL, IS TRUE/UNKNOWN, IS JSON, LIKE, IN list, +, nested sublink, every row form.
- Deparse users (deparsers.sql; raw/deparsers-*.out): pretty pg_get_ruledef (action, WHERE, _RETURN),
  pg_get_viewdef on a matview and with wrap 80, pg_get_expr(polqual,..,true), psql \d+ view and \d rules are
  wrong. pg_policies, psql policy lines, non-pretty ruledef and pg_get_functiondef are correct. CHECK, index
  expression/predicate, trigger WHEN, generated column and DEFAULT reject subqueries. Same on all four branches.
- pg_dump (dump.sh: dump, restore with ON_ERROR_STOP into a new db, compare rows and the rule's effect):
  identical on all four branches (raw/dump-*.out).
- History: git log -L on get_sublink_expr; gram.y precedence in REL9_4 vs REL9_5/master; 52347b66370,
  c6b3c939b7e. VALIDATION.md section 3.
- Fix: fix.patch against a261fc2b7220. Patched build: minimal.sql correct, all 46 extent cases fine
  (raw/extent-fix.tsv, fix-run.out). make check: run 1 failed only on create_view (the new tt26v columns c12-c14,
  no other expected output changed); after updating expected, "All 239 tests passed" (raw/make-check-fix.log).
  Unpatched master prints c12-c14 wrongly. check-world and contrib not run. The patched line has the same context
  on REL_14/18/19, but no back-branch patched build was made.
- Duplicate search: 6 postgresql.org searches 15 s apart. 3 of them hit the Henson Choi hackers thread of
  2026-10-13 (same bug, patches attached), the other 3 had no hits. VALIDATION.md section 5;
  hackers-thread-2026-10-13.html.
