### Prerequisites

* [x] Check that your issue is not already filed: searched for `LEAN_NAT_MAX_SIZE`, `maximum numeral size`, `reduce_pow` and `Nat.pow kernel`. Related: #14849 (added the limit), #15439, #16006.
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

The kernel refuses to evaluate `2 ^ k` for every `k > 2^24`, although the result is about 2 MB and the default numeral budget (`LEAN_NAT_MAX_SIZE`) is 128 MB:

```lean
example : 2 ^ 16777217 % 2 = 0 := by decide +kernel
```

```
lean Pow.lean
Pow.lean:1:37: error: (kernel) the kernel refused to evaluate `Nat.pow` because the result would exceed the maximum numeral size; increase the LEAN_NAT_MAX_SIZE environment variable to allow it
```

The same value, and a larger one, are accepted when computed differently:

```lean
example : 2 ^ 16777216 % 2 = 0 := by decide +kernel          -- accepted
example : (2 ^ 16777216 * 2) % 2 = 0 := by decide +kernel    -- accepted, equals 2 ^ 16777217
example : (1 <<< 16777217) % 2 = 0 := by decide +kernel      -- accepted, equals 2 ^ 16777217
example : (4 ^ 8388609) % 2 = 0 := by decide +kernel         -- accepted, equals 2 ^ 16777218
```

Tactic proofs about large bit widths hit it too:

```lean
example : BitVec.setWidth 16777217 (0#8) = 0#16777217 := by grind   -- same kernel error
example : BitVec.setWidth 16777216 (0#8) = 0#16777216 := by grind   -- accepted
```

The guard in `type_checker::reduce_pow` is

```cpp
if (base > nat(1) && k != 0 && nat_size_in_bytes(base) > g_nat_max_size / k)
```

It counts a small base as 8 bytes per unit of the exponent, so for base 2 it estimates 64 times
the actual size: it refuses as soon as `8 * k > 2^27`, which is `k > 2^24`.

**Expected behavior:** `2 ^ 16777217 % 2 = 0` checks, like the other cases.

**Actual behavior:** the kernel refuses it. With `LEAN_NAT_MAX_SIZE=1073741824` it checks.

### Versions

```
4.37.0-nightly-2026-11-03   refused
4.37.0-nightly-2026-10-30   refused
master b21ac7949a            refused
4.36.0-rc2                   refused
4.35.0                       refused
```

These are on macOS arm64 (official toolchains via elan).

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
