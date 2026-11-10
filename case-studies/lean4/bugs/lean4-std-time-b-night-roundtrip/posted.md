### Prerequisites

* [x] Check that your issue is not already filed: searched for `at night`, `ExtendedDayPeriod`, `day period` and `Std.Time` round trip. Related: #16043 and #16038 (other patterns that do not read back their own output), #16119.
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

With the extended day period `B`, times from 21:00 to 23:59 format as "at night", and parsing that output gives a time 12 hours early:

```lean
import Std.Time
open Std.Time

def fmt : GenericFormat .any := .spec! "hh:mm B"
def t : PlainTime := PlainTime.ofHourMinuteSeconds 22 0 0

#eval t.format "hh:mm B"
#eval (fmt.parse "10:00 at night").map (·.time)
```

```
lean Night.lean
"10:00 at night"
Except.ok (time("10:00:00.000000000"))
```

`BBBB` behaves the same (23:59 formats as "11:59 at night" and parses back as 11:59). Night times after midnight round-trip: 03:00 formats as "03:00 at night" and parses back as 03:00. So does 20:59 ("08:59 in the evening").

In `src/Std/Time/Format/Basic.lean`, `classifyExtendedDayPeriod` returns `.night` both for hours below 6 and for hours 21 to 23, and `markerOfExtendedDayPeriod` always maps `.night` to AM.

**Expected behavior:** `"10:00 at night"` parses as 22:00, the time that formats to it.

**Actual behavior:** it parses as 10:00.

### Versions

```
4.37.0-nightly-2026-11-03   fails
4.36.0-rc2                   fails
4.35.0                       fails
```

These are on macOS arm64 (official toolchains via elan).

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
