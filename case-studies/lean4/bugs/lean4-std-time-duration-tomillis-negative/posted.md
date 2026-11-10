### Prerequisites

* [x] Check that your issue is not already filed: searched for `toMilliseconds`, `Duration negative` and `Std.Time rounding`. Related: #16119 (added the "rounds toward zero, like the unit conversions" wording).
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

`Duration.toMilliseconds` rounds a negative sub-millisecond part down, so it disagrees with converting the same value through nanoseconds:

```lean
import Std.Time
open Std.Time

def d : Duration := Duration.ofNanoseconds (-1)

#eval d.toMilliseconds
#eval d.toNanoseconds.toMilliseconds
#eval (Duration.ofNanoseconds 1).toMilliseconds
#eval (Timestamp.ofNanosecondsSinceUnixEpoch (-1)).toMillisecondsSinceUnixEpoch
```

```
lean DurMs.lean
-1
0
0
0
```

Larger values are off by one too: `(Duration.ofNanoseconds (-1000001)).toMilliseconds` is `-2`, and `.toNanoseconds.toMilliseconds` of the same duration is `-1`.

In `src/Std/Time/Duration.lean`, `toMilliseconds` uses `duration.nano.ediv 1000000`. The other divisions in that file use `tdiv`, and `Nanosecond.Offset.toMilliseconds` rounds toward zero. `src/Std/Time/Internal/UnitVal.lean` describes the unit conversions as "rounding toward zero".

**Expected behavior:** all four lines print `0`.

**Actual behavior:** the first line prints `-1`.

### Versions

```
4.37.0-nightly-2026-11-04   -1
4.36.0-rc2                   -1
4.35.0                       -1
```

These are on macOS arm64 (official toolchains via elan).

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
