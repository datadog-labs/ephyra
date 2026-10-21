# pg_get_viewdef(.., true) drops the parentheses around the left operand of IN (subquery)

Found 2026-10-16 by the `views` oracle of ephyra-postgresql (round/views-1,
`views-viewdef-pretty`; burn seeds 49, 102, 155, 210 among the first 300).

## What

In pretty mode (`pg_get_viewdef(v, true)`, psql's `\d+` "View definition",
`pg_get_ruledef`/`pg_get_constraintdef`/`pg_get_indexdef` with pretty),
`(a = 1) IN (SELECT b FROM t)` is printed as `(a = 1 IN ( SELECT ..))`.
IN binds tighter than comparison operators, IS, NOT, AND, OR (and is
nonassoc with BETWEEN, LIKE), so the text reads `a = (1 IN (SELECT ..))`:
it fails to parse or type (`operator does not exist: integer = boolean`),
or, when the types fit, it means something else and gives other rows
(`minimal.sql`: `(b < b) IN (SELECT false)` is true, the printed text gives
false).

## Where

`src/backend/utils/adt/ruleutils.c`, `get_sublink_expr` (line ~11945 on
158b6d5109): the testexpr's left operand is printed with
`get_rule_expr(linitial(opexpr->args), context, true)` (and likewise for the
BoolExpr and RowCompareExpr cases). In pretty mode an OpExpr / NullTest /
BoolExpr / BooleanTest node prints no parentheses of its own
(`if (!PRETTY_PAREN(context))`), leaving it to the parent to call
`get_rule_expr_paren`; `get_sublink_expr` does not, and the parentheses it
adds wrap the whole sublink, not the left operand.

## Versions checked

- master 158b6d5109f (2026-10-13), `~/Projects/postgres/inst` (20devel,
  cassert): reproduces (`psql -f minimal.sql`).
- Only the pretty form: `pg_get_viewdef(v, false)` prints `((a = 1) IN (..))`,
  and pg_dump (non-pretty) dumps and restores both views correctly
  (checked: restored views give the original rows).

## Impact

Display only, but wrong: psql `\d+ view` and any client using the pretty
form show a definition that does not mean the view's query; copying it
back (e.g. to recreate a view) fails or silently changes the result.
pg_dump is not affected.

## Not done

Not reported upstream, mailing lists not searched (per the round's brief).
The oracle skips it narrowly: `ephyra_postgresql::sqlancer::sublink_paren_bug`
(id `viewdef-sublink-paren` for `EPHYRA_FIXED_BUGS`).
