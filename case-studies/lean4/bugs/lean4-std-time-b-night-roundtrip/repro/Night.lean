import Std.Time
open Std.Time

def fmt : GenericFormat .any := .spec! "hh:mm B"
def t : PlainTime := PlainTime.ofHourMinuteSeconds 22 0 0

#eval t.format "hh:mm B"
#eval (fmt.parse "10:00 at night").map (·.time)
