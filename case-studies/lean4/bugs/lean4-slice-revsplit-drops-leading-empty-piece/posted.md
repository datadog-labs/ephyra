### Prerequisites

* [x] Check that your issue is not already filed: searched for `revSplit`, `Slice.split`, `String.Slice split` and `split empty slice`.
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

`String.Slice.revSplit` leaves out the leftmost piece when it is empty. A slice that starts with a separator, or an empty slice, gives one piece fewer than `String.Slice.split`. A separator at the end does give an empty piece, so the two ends behave differently:

```lean
def main : IO Unit := do
  IO.println (repr <| (",x".toSlice.split ',').toList.map (·.copy))
  IO.println (repr <| (",x".toSlice.revSplit ',').toList.map (·.copy))
  IO.println (repr <| ("x,".toSlice.revSplit ',').toList.map (·.copy))
  IO.println (repr <| ("".toSlice.revSplit ',').toList.map (·.copy))
```

```
lean --run RevSplit.lean
# expected:      # actual:
["", "x"]        ["", "x"]
["x", ""]        ["x"]
["", "x"]        ["", "x"]
[""]             []
```

Likewise `"aa".toSlice.split 'a'` gives three empty pieces and `"aa".toSlice.revSplit 'a'` gives two. The `Char → Bool` pattern form behaves the same way.

The `revSplit` docstring matches the `split` docstring apart from the direction ("starting from the end of the slice and traversing towards the start ... If multiple subslices in a row match the pattern, the resulting list will contain empty slices"), so `revSplit` should give the pieces of `split` in reverse order.

In `RevSplitIterator.step` (`src/Init/Data/String/Slice/Pattern.lean`), when the searcher is done, the remaining piece `s.sliceTo currPos` is yielded only if `currPos ≠ s.startPos`. The forward `SplitIterator` yields its remaining piece unconditionally.

**Expected behavior:** `revSplit` gives the same pieces as `split`, in reverse order.

**Actual behavior:** the leftmost empty piece is missing.

### Versions

```
4.37.0-nightly-2026-11-05   ["x"] / []
4.36.0-rc2                   ["x"] / []
4.35.0                       ["x"] / []
```

These are on macOS arm64 (official toolchains via elan), and Linux x86_64 gives the same results. Nothing under `src/Init/Data/String/` has changed on master since the nightly. The code came in with #10514.

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
