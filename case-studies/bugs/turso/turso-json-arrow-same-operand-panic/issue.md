Filed as #9836 (https://github.com/tursodatabase/turso/issues/9836) (line 1 corrected 2026-10-21).

Title: `x -> x` and `x ->> x` panic (index out of bounds) when both operands are the same expression

Any `->` or `->>` whose two operands are the same expression panics the process:

```sql
SELECT NULL -> NULL;
SELECT 1 ->> 1;
SELECT 'x' -> 'x';
CREATE TABLE t(a); INSERT INTO t VALUES(NULL); SELECT a -> a FROM t;
```

Turso (`tursodb -q -m list`):
```
thread 'main' panicked at core/vdbe/execute.rs:9837:29:
index out of bounds: the len is 3 but the index is 3
```

SQLite 3.50.4 (and trunk): one row, NULL, for each query.

Expected: the operator is evaluated like any other, here returning NULL. It should never panic.

Likely cause: `emit_binary_expr_scalar` (core/translate/expr/binary.rs:103) notices that both operands are equivalent (`exprs_are_equivalent`) and evaluates them once into a single register, passing `lhs == rhs` to `emit_binary_insn`. The `ArrowRight`/`ArrowRightShift` arm of `emit_binary_insn` (binary.rs:706 and 1127) ignores `rhs` and emits `Function { start_reg: lhs, arg_count: 2 }`, which assumes the path is in `lhs + 1`. The VDBE then reads `registers[start_reg + 1]` (execute.rs:9837), past the end of the register file:

```
EXPLAIN SELECT 1 -> 1;
3|Integer|1|2|0||0|r[2]=1
4|Function|0|2|1|->|0|r[1]=func(r[2..3])     <- r[3] never allocated
```

When the shared register isn't the last one, the function would read whatever sits in the next register instead of panicking.

Tested on Turso main 7641fad2 (2026-10-13), compared against SQLite 3.50.4.
