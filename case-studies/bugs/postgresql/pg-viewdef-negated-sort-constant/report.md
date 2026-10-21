# pgsql-bugs report: pg_get_viewdef prints a negated constant in ORDER BY / GROUP BY as a column position

Status: draft, not submitted. Deparse/dump bug (a view whose dump does not restore, or restores with a different
meaning); no crash, no privilege boundary, so the normal bug form, not security.

## Form fields

- PostgreSQL version: choose `18.6` (the body names 19rc1, 14.24 and master)
- Operating system: `Ubuntu 24.04 x86_64 (also seen on macOS 26 arm64)`
- Short description: `pg_get_viewdef prints ORDER BY (- '3'::int4) as (- 3), which reparses as column position -3`

## Body (plain text)

```
Hello,

pg_get_viewdef() prints an ORDER BY / GROUP BY / DISTINCT ON term that
negates an integer constant as "(- 3)". When that text is parsed again,
the grammar folds "- 3" into the constant -3 (doNegate in gram.y), and a
constant there is a column position, so the text fails, or, when it
lands on a position that exists, means something else. pg_dump uses the
same deparse, so a dump with such a view does not restore.

  CREATE TABLE t (a int);
  INSERT INTO t VALUES (1), (2);
  CREATE VIEW o AS SELECT a FROM t ORDER BY (- '3'::int4);
  CREATE VIEW g AS SELECT count(*) AS n FROM t GROUP BY (- '3'::int4);
  SELECT * FROM g;                        -- n = 2, both views work
  SELECT pg_get_viewdef('o'::regclass);   -- SELECT a FROM t ORDER BY (- 3);
  SELECT pg_get_viewdef('g'::regclass);   -- SELECT count(*) AS n FROM t GROUP BY (- 3);
  SELECT t.a FROM t ORDER BY (- 3);
  ERROR:  ORDER BY position -3 is not in select list
  SELECT count(*) AS n FROM t GROUP BY (- 3);
  ERROR:  GROUP BY position -3 is not in select list

pg_dump of that database writes

  CREATE VIEW public.o AS
   SELECT a
     FROM public.t
    ORDER BY (- 3);

and restoring it with psql into an empty database gives

  psql:dump.sql:44: ERROR:  ORDER BY position -3 is not in select list
  psql:dump.sql:67: ERROR:  GROUP BY position -3 is not in select list

plus "relation does not exist" for the views' ALTER OWNER and for a view
built on top of o, so those views are all lost; pg_restore of a -Fc dump
fails the same way.

Some variants (both the pretty and the plain form print the same):

- DISTINCT ON (- '3'::int4), GROUPING SETS / ROLLUP / CUBE (- '3'::int4)
  and a FROM-subquery's ORDER BY fail the same way; so do a rule action
  (pg_get_ruledef), a materialized view and a BEGIN ATOMIC function body
  (pg_get_functiondef).
- - '0'::int4 gives "ORDER BY position 0 is not in select list".
- - '1.5'::numeric prints as (- 1.5), which comes back as
  "non-integer constant in ORDER BY".
- - (- '3'::int4) prints as (- (- 3)), which folds to 3: with three
  columns in the select list, the reparsed view silently orders by the
  third column. Likewise
    CREATE TABLE t2 (a int, b int);
    INSERT INTO t2 VALUES (1, 10), (2, 20), (1, 30);
    CREATE VIEW v AS SELECT DISTINCT ON (- (- '1'::int4)) a, b FROM t2
      ORDER BY - (- '1'::int4), b DESC;
  returns one row, (1,30); its definition, run again, becomes
  DISTINCT ON (a) ... ORDER BY a, b DESC and returns (1,30) and (2,20).
- Window PARTITION BY / ORDER BY and an aggregate's ORDER BY print the
  same text but are not affected, since there a constant is not a
  position. Negative constants ('-1'::integer), int2, int8, float8 and
  integral numeric constants are printed with a cast and are fine.

Reproduced on 18.6 (REL_18_STABLE dfbc1507cdb), 19rc1 (REL_19_STABLE
1212d5d33b1), 14.24 (REL_14_STABLE 14904052e65) and master a261fc2b722,
Ubuntu 24.04 x86_64, --enable-cassert, with pg_dump of the same version;
also on 18.6 and master on macOS arm64.

Cause: get_rule_sortgroupclause() in ruleutils.c prints a plain Const
with an explicit cast, because "a simple integer constant is ambiguous
(and will be misinterpreted by findTargetlistEntrySQL92()) if we dump it
without any decoration" (added in 2008 for 8.3). A prefix minus over a
Const is an OpExpr and takes the generic branch, where get_oper_expr()
prints the Const undecorated; the parentheses around it do not help,
since "(- 3)" folds too.

The patch below prints a prefix operator over a Const (also nested) with
the Const's cast, "(- 3::integer)". "- 3::integer" parses as -(3::integer),
which doNegate does not fold, so the reparsed tree is the same OpExpr and
the text round-trips. With it, the cases above dump and restore, and the
deparsed text of each reparses to the same text and the same rows. "make
check" passes (239 tests), with a new test in create_view.sql; no
existing expected output changes. Window and aggregate ORDER BY also get
the cast, which they do not need but which is harmless.

diff --git a/src/backend/utils/adt/ruleutils.c b/src/backend/utils/adt/ruleutils.c
index 5e8e1683db0..0d68da89e24 100644
--- a/src/backend/utils/adt/ruleutils.c
+++ b/src/backend/utils/adt/ruleutils.c
@@ -435,6 +435,8 @@ static void get_setop_query(Node *setOp, Query *query,
 static Node *get_rule_sortgroupclause(Index ref, List *tlist,
 									  bool force_colno,
 									  deparse_context *context);
+static bool is_prefix_op_over_const(Node *expr);
+static void get_prefix_op_over_const(Node *expr, deparse_context *context);
 static void get_rule_groupingset(GroupingSet *gset, List *targetlist,
 								 bool omit_parens, deparse_context *context);
 static void get_rule_orderby(List *orderList, List *targetList,
@@ -6583,12 +6585,13 @@ get_rule_sortgroupclause(Index ref, List *tlist, bool force_colno,
 	 * expression is a constant, force it to be dumped with an explicit cast
 	 * as decoration --- this is because a simple integer constant is
 	 * ambiguous (and will be misinterpreted by findTargetlistEntrySQL92()) if
-	 * we dump it without any decoration.  Similarly, if it's just a Var,
-	 * there is risk of misinterpretation if the column name is reassigned in
-	 * the SELECT list, so we may need to force table qualification.  And, if
-	 * it's anything more complex than a simple Var, then force extra parens
-	 * around it, to ensure it can't be misinterpreted as a cube() or rollup()
-	 * construct.
+	 * we dump it without any decoration.  The same goes for a prefix operator
+	 * applied to a constant, since gram.y folds "- 3" into the constant -3.
+	 * Similarly, if it's just a Var, there is risk of misinterpretation if
+	 * the column name is reassigned in the SELECT list, so we may need to
+	 * force table qualification.  And, if it's anything more complex than a
+	 * simple Var, then force extra parens around it, to ensure it can't be
+	 * misinterpreted as a cube() or rollup() construct.
 	 */
 	if (force_colno)
 	{
@@ -6599,6 +6602,8 @@ get_rule_sortgroupclause(Index ref, List *tlist, bool force_colno,
 		 /* do nothing, probably can't happen */ ;
 	else if (IsA(expr, Const))
 		get_const_expr((Const *) expr, context, 1);
+	else if (is_prefix_op_over_const(expr))
+		get_prefix_op_over_const(expr, context);
 	else if (IsA(expr, Var))
 	{
 		/* Tell get_variable to check for name conflict */
@@ -6633,6 +6638,44 @@ get_rule_sortgroupclause(Index ref, List *tlist, bool force_colno,
 	return expr;
 }
 
+/*
+ * Is expr a prefix operator applied, perhaps repeatedly, to a Const?
+ */
+static bool
+is_prefix_op_over_const(Node *expr)
+{
+	while (IsA(expr, OpExpr) && list_length(((OpExpr *) expr)->args) == 1)
+		expr = (Node *) linitial(((OpExpr *) expr)->args);
+	return IsA(expr, Const);
+}
+
+/*
+ * Display such an expression for get_rule_sortgroupclause, with the Const
+ * decorated by an explicit cast.  Otherwise "- 3" would be read back as the
+ * integer constant -3, which a sort/group clause takes as a column number.
+ */
+static void
+get_prefix_op_over_const(Node *expr, deparse_context *context)
+{
+	StringInfo	buf = context->buf;
+	OpExpr	   *opexpr;
+	Node	   *arg;
+
+	if (IsA(expr, Const))
+	{
+		get_const_expr((Const *) expr, context, 1);
+		return;
+	}
+	opexpr = castNode(OpExpr, expr);
+	arg = (Node *) linitial(opexpr->args);
+	appendStringInfo(buf, "(%s ",
+					 generate_operator_name(opexpr->opno,
+											InvalidOid,
+											exprType(arg)));
+	get_prefix_op_over_const(arg, context);
+	appendStringInfoChar(buf, ')');
+}
+
 /*
  * Display a GroupingSet
  */

Found with a query fuzzer.

Regards,
<name>
```

## Verification

Details, commands and raw output in VALIDATION.md; files named below are in this folder.

- Reproduces on master a261fc2b7220 (20devel), REL_19_STABLE 1212d5d33b17 (19rc1), REL_18_STABLE dfbc1507cdb5
  (18.6), REL_14_STABLE 14904052e653 (14.24); fresh cassert builds on workspace, Linux x86_64 (VALIDATION.md
  sections 1-1d; ver-rel19.out, ver-rel18.out, ver-rel14.out). macOS 18.6 a85b5523e67 and master 158b6d5109:
  README.md (earlier session).
- Dump/restore: pg_dump of the same version writes `ORDER BY (- 3)` / `GROUP BY (- 3)` on all four; psql restore
  into an empty db errors on o and g, then "relation does not exist" for ALTER OWNER and the dependent view dep;
  only t is left. pg_restore of -Fc on master: exit 1, "errors ignored on restore: 6". Line numbers 44/67 are from
  the master and 18/19 dumps (43/66 on 14). (VALIDATION.md section 2.)
- Variants (extent.sql, extent-master.out; same on 19, 18, 14): DISTINCT ON, GROUPING SETS/ROLLUP/CUBE, subquery
  ORDER BY, - '0', - '1.5'::numeric (non-integer constant error), - (- '3') -> ORDER BY c and - (- '1') -> ORDER BY a
  with rows reordered, DISTINCT ON (- (- 1)) -> DISTINCT ON (a) with 1 row vs 3 rows; rule action
  (GROUP BY position 2), materialized view, BEGIN ATOMIC function. Window/aggregate ORDER BY fine; '-1'::int4,
  int2, int8, float8, '3'::numeric, + '3' fine. (VALIDATION.md section 3.) The body's DISTINCT ON example with t2 was
  run as written on 18.6 (distinct-on.sql): v gives (1,30); v2 created from its viewdef deparses as
  `DISTINCT ON (a) a, b FROM t2 ORDER BY a, b DESC` and gives (1,30), (2,20).
- History: the Const branch is 59358907757 (2008-01-06, Tom Lane), first in REL8_3_RC2; doNegate folds A_Const
  Integer/Float only (VALIDATION.md section 4).
- Fix: fix.patch (ruleutils.c + create_view test) against master a261fc2b722; patched cassert build: minimal.sql
  gives `(- 3::integer)`, dump restores with ON_ERROR_STOP (exit 0), all 29 extent cases round-trip with the same
  text and rows (extent-fix.out). make check: first run only the new create_view block differed (no change to
  rules.out or other existing expected files); after updating expected, "All 239 tests passed" (fix-make-check.out).
  Rerun after the final comment reflow: "All 239 tests passed" again.
- Duplicate search: 6 postgresql.org list searches, 12 s apart, and git log --grep over master; no report or fix
  for this case. Related only: c399248b3b4 (postgres_fdw ORDER BY Const, 2024) and 43f2e7634d8 (ORDER BY name
  conflict, 2024). (VALIDATION.md section 6.)
- Not checked: pg_dump TAP tests, contrib regression suites, branches 15-17 (code is the same between 14 and 18).
