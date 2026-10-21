# Issue for qed-solver/prover: filed as #14

Filed 2026-10-14 13:35 UTC by alpaylan: https://github.com/qed-solver/prover/issues/14

Work tag `qed/bug/sort-panics`. Verified 2026-10-14 on upstream main 9e9c262 with qed-parser
684daf2 (Nix builds: prover /nix/store/0fwjvgy83yk0q2bbfixnbs0p2cyp8hss-qed-prover-0.1.0,
parser /nix/store/w4x7qc7lxnbw805zzdnmq0yhfmp48hjc-qed-parser): both repros panic at the lines
below, also with the #13 fix applied (independent bug); the control without the type mix proves.
Repro files: `min/qedparser_in_int_decimal.sql`, `min/qedparser_unionall_int_decimal.sql`.
Duplicates: none (open #8 is a different site and mechanism). Details in `triage.md`. The
sqleq-side counterpart (opaque functions typed VARBINARY) is internal, see `triage.md`.

---

**Title:** Panic on INTEGER vs DECIMAL in `IN (subquery)` (normal.rs:440) and `UNION` (partial.rs:185)

**Body:**

qed-prover panics, instead of returning a result, when an `IN`/`= ANY` subquery or a `UNION` mixes an INTEGER column with a DECIMAL one. The SQL is ordinary and the JSON comes straight from qed-parser.

## Reproduction

Both go through the shipped pipeline (`qed-parser repro.sql && qed-prover repro.json`); no IR is edited by hand.

**A. `IN` subquery**

```sql
CREATE TABLE t (a DECIMAL, b DECIMAL, i INTEGER);
SELECT i FROM t WHERE i IN (SELECT a FROM t);
SELECT i FROM t WHERE i IN (SELECT b FROM t);
```

```
thread 'main' panicked at src/pipeline/normal.rs:440:9:
assertion `left == right` failed: v!62 and v!66 have different types.
```

`i = ANY (SELECT a FROM t)` and `a IN (SELECT i FROM t)` panic the same way.

**B. `UNION`**

```sql
CREATE TABLE t (a DECIMAL, b DECIMAL, i INTEGER);
SELECT i FROM t UNION ALL SELECT a FROM t;
SELECT i FROM t UNION ALL SELECT b FROM t;
```

```
thread 'main' panicked at src/pipeline/partial.rs:185:9:
assertion `left == right` failed: Wrong type for $1
```

`UNION` (distinct) panics the same way.

Expected: a result (here `NotProvable`), or at worst an unsupported/error result, not a panic.

## Cause

qed-parser emits these operands with their own types and no CAST:

- `{"operator":"IN","operand":[{"column":0,"type":"INTEGER"}],"query":{... "target":[{"column":1,"type":"DECIMAL"}]}}`
- `{"union":[{project → INTEGER}, {project → DECIMAL}]}`

For a scalar comparison it does insert the CAST (`i = a` becomes `CAST(i AS DECIMAL) = CAST(a AS DECIMAL)`), so the prover never sees a mixed `=` there. The prover then requires equal sorts:

- **A:** `IN` is evaluated as `quant_cmp("SOME", "=", ...)` (normal.rs:757 → 487), which calls `self.equal(ty, v, x)` with the needle (Int) and a fresh variable of the subquery column's sort (Real). `equal` begins with `assert_eq!(a1.get_sort(), a2.get_sort(), ...)` (normal.rs:440).
- **B:** `Relation::scope()` for `Union`/`Intersect` is `rels[0].scope(schemas)` (relation.rs:92). The column is therefore declared INTEGER while the second branch produces Real values, and the column reference above it fails `assert_eq!(self.0[l].ty(), ty, "Wrong type for {}", ..)` (partial.rs:185).

## Possible fixes (untested)

- Parser: CAST the IN needle/subquery column and each UNION branch to the least-restrictive type, as is already done for scalar comparisons.
- Prover: in `quant_cmp`/`equal`, promote an Int operand with `int_to_real` when the other side is Real (as the `CAST` case at normal.rs:723 already does). For set operations, cast each branch's columns to a common scope instead of taking `rels[0]`'s.
- Alternatively, reject such input with an error rather than an assertion.

## Environment

`qed-prover` at `9e9c262` and `qed-parser` at `684daf2`, both built with Nix. Found by differential testing (same campaign as #13).
