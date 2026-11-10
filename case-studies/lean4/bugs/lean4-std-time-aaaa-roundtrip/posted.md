### Prerequisites

* [x] Check that your issue is not already filed: searched for `meridiem`, `aaaa`, `AM/PM` and `Std.Time` round trip. Related: #16043 and #16038 (other patterns that do not read back their own output), #16119.
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

The pattern letter `aaaa` is documented as the full AM/PM form ("post meridiem"). Formatting prints the short form, and parsing accepts only the full form:

```lean
import Std.Time
open Std.Time

def fmt : GenericFormat .any := .spec! "hh:mm aaaa"
def t : PlainTime := PlainTime.ofHourMinuteSeconds 13 5 0

#eval t.format "hh:mm aaaa"
#eval fmt.parse "01:05 PM" |>.isOk
#eval fmt.parse "01:05 post meridiem" |>.isOk
```

```
lean Aaaa.lean
"01:05 PM"
false
true
```

With a date, the error is explicit: parsing `"2020-01-01 01:05 PM"` with `"uuuu-MM-dd hh:mm aaaa"` gives `offset 17: expected: post meridiem`.

The docs say `aaaa` "Displays the full form (e.g., "ante meridiem", "post meridiem")" (`src/Std/Time.lean`). In `src/Std/Time/Format/Basic.lean`, the formatter uses `formatMarkerShort` for `.a .full` and the parser uses `parseMarkerLong`. `formatMarkerLong` is defined but never used.

**Expected behavior:** `aaaa` formats `"01:05 post meridiem"`, which the same pattern parses.

**Actual behavior:** it formats `"01:05 PM"`, which the same pattern rejects.

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
