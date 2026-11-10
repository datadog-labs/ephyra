import Std.Time
open Std.Time

def fmt : GenericFormat .any := .spec! "hh:mm aaaa"
def t : PlainTime := PlainTime.ofHourMinuteSeconds 13 5 0

#eval t.format "hh:mm aaaa"
#eval fmt.parse "01:05 PM" |>.isOk
#eval fmt.parse "01:05 post meridiem" |>.isOk
