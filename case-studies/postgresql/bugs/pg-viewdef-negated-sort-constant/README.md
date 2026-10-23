# pg_get_viewdef prints a negated integer constant in ORDER BY / GROUP BY as a column position

Found 2026-10-16 by the `views` oracle of ephyra-postgresql (round/views-1,
`views-viewdef`: the deparsed text of a view did not run). Not reported.

## What

A view whose ORDER BY or GROUP BY term is a unary minus of an integer (or
numeric) literal deparses, in both the pretty and the raw form, as `(- 3)`:

    CREATE VIEW o AS SELECT a FROM t ORDER BY (- '3'::int4);
    pg_get_viewdef('o', false):  SELECT a FROM t ORDER BY (- 3);

When the text is parsed again, the grammar folds `- 3` into the constant -3
(`doNegate`), and a constant in ORDER BY / GROUP BY is a column position:

    ERROR:  ORDER BY position -3 is not in select list
    ERROR:  GROUP BY position -3 is not in select list

## Impact

pg_dump uses the same deparser: the dump of such a view contains

    CREATE VIEW public.o AS
     SELECT a
       FROM public.t
      ORDER BY (- 3);

so restoring the dump fails on that view (and on everything that depends on
it). The view itself works and is accepted by CREATE VIEW.

## Where

`src/backend/utils/adt/ruleutils.c`, `get_rule_sortgroupclause()` (master
158b6d5109, ~line 6580). Its comment already guards this ambiguity for a
plain constant ("a simple integer constant is ambiguous (and will be
misinterpreted by findTargetlistEntrySQL92()) if we dump it without any
decoration"), printing a Const with an explicit cast. A unary minus over a
constant is an OpExpr, so it takes the generic branch, and `get_oper_expr`
prints `- 3` with the constant undecorated; the parentheses added around it
do not help, because the grammar folds `(- 3)` too. Printing the operand of
the prefix minus with its cast (`(- '3'::integer)`) would keep it an
expression.

## Versions checked (2026-10-16, macOS arm64, cassert)

- master 158b6d5109: `minimal.sql` gives the two errors above; pg_dump output as shown.
- REL_18_STABLE a85b5523e67 (18.6): the same.

Not yet done: current master, older branches, a duplicate search of
pgsql-bugs / pgsql-hackers, and an actual restore of the dump.
