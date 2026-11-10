### Prerequisites

* [x] Check that your issue is not already filed: searched for `userinfo`, `setUserInfo`, `URI username colon` and `URI password`. Related: #15246, #16021, #16058 (other Std.Http URI round trips).
* [x] Reduce the issue to a minimal, self-contained, reproducible test case. Avoid dependencies to Mathlib or Batteries.
* [x] Test your test case against the latest nightly release.

### Description

A username containing `:` is printed unencoded, so parsing the printed URI splits the credentials differently:

```lean
import Std.Http
open Std.Http

def u : URI := URI.Builder.empty
  |>.setScheme! "http" |>.setUserInfo "a:b" (some "c") |>.setHost! "example.com" |>.build

def creds (u : Option URI) := (u.bind (·.authority) |>.bind (·.userInfo)).map fun i => (i.username?, i.password?)

#eval toString u
#eval creds (some u)
#eval creds (URI.parse? (toString u))
```

```
lean UserInfo.lean
"http://a:b:c@example.com/"
some (some "a:b", some "c")
some (some "a", some "b:c")
```

The builder's docstring says it "Takes raw (unencoded) strings and handles encoding automatically when building the final URI", and `setUserInfo`'s says "The strings will be automatically percent-encoded." In userinfo the first `:` separates the username from the password, so a `:` in the username has to be percent-encoded (`a%3Ab`).

**Expected behavior:** `"http://a%3Ab:c@example.com/"`, which parses back as username `a:b` and password `c`.

**Actual behavior:** `"http://a:b:c@example.com/"`, which parses back as username `a` and password `b:c`.

### Versions

```
4.37.0-nightly-2026-11-04   fails
4.35.0                       fails
```

These are on macOS arm64 (official toolchains via elan). The URI commits on master since the nightly do not touch userinfo.

### Impact

Add :+1: to issues you consider important. If others are impacted by this issue, please ask
them to add :+1: to it.

Found by an automated differential tester ([ephyra](https://github.com/datadog-labs/ephyra)); this report was drafted with AI assistance.
