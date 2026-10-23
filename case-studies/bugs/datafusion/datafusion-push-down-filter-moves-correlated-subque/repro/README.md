# Repro: `= ANY (subquery)` in JOIN ON on an aliased table -> "No field named a.c0"

- `repro.sql`: paste into `datafusion-cli` (or `datafusion-cli -f repro.sql`). The first SELECT fails,
  and the next two (no alias / WHERE form) return the expected rows.
- `repro.sh <datafusion-cli binary | datafusion checkout>`: runs the failing query on its own.
  Exit 1 = bug reproduces (schema error), 0 = correct rows `(1,x,1) (1,x,3)`, 2 = cannot decide.
  With a checkout it first runs `cargo build --release -p datafusion-cli`.

Checked results:
- trunk 63a2801c1b0b (2026-10-16): exit 1
- trunk 63a2801c1b0b + ../fix.diff: exit 0
