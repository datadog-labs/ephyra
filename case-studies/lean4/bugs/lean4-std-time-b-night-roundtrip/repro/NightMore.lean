import Std.Time
open Std.Time
def rt (p : String) (t : PlainTime) : String :=
  let s := t.format p
  match (GenericFormat.spec p : Except String (GenericFormat .any)) with
  | .error e => e
  | .ok f => s!"{s} -> {repr ((f.parse s).map (·.time))}"
#eval rt "hh:mm B" (PlainTime.ofHourMinuteSeconds 3 0 0)
#eval rt "hh:mm BBBB" (PlainTime.ofHourMinuteSeconds 23 59 0)
#eval rt "KK:mm B" (PlainTime.ofHourMinuteSeconds 21 0 0)
#eval rt "hh:mm B" (PlainTime.ofHourMinuteSeconds 20 59 0)
