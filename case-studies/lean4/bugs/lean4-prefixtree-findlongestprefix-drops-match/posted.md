### Prerequisites

* [x] Check that your issue is not already filed: searched for `findLongestPrefix`, `PrefixTree`, `NameTrie` and `AsyncConsts`.
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

With keys `a` and `a.b.c`, `findLongestPrefix? a.b.d` returns `none`, although `a` is a key and a prefix of `a.b.d`:

```lean
import Lean.Data.NameTrie
open Lean

-- Keys: `a` ↦ 1 and `a.b.c` ↦ 2. The node for `a.b` exists but holds no value.
def t : NameTrie Nat := ((∅ : NameTrie Nat).insert `a 1).insert `a.b.c 2

#eval t.findLongestPrefix? `a.x    -- some 1
#eval t.findLongestPrefix? `a.b.d  -- none, expected some 1
```

The docstring of `PrefixTreeNode.findLongestPrefix?` (`src/Lean/Data/PrefixTree.lean`) says: "Returns the value of the longest key in `t` that is a prefix of `k`, if any."

The first lookup leaves the trie at the node for `a`, which has a value. The second goes through the node for `a.b`, which has none, and then leaves the trie. In `findLongestPrefix?.loop`, the `[]` case returns `val <|> acc?`, but the `m.get? k = none` case returns only `val` and drops `acc?`, the best match found higher up.

**Expected behavior:** `some 1` for both lookups.

**Actual behavior:** `some 1`, then `none`.

### Versions

```
4.37.0-nightly-2026-11-05   some 1 / none
4.36.0-rc2                   some 1 / none
4.35.0                       some 1 / none
```

These are on macOS arm64 (official toolchains via elan), and Linux x86_64 gives the same results. `PrefixTree.lean` and `NameTrie.lean` have not changed on master since the nightly. The function was added in #6691 (v4.17.0).

### Additional Information

The in-tree caller is `AsyncConsts.findPrefix?` in `src/Lean/Environment.lean`.

A fix is available (one line: `| none => val <|> acc?`).

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
