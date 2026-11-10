import Std.Time
open Std.Time
def fmt : GenericFormat .any := .spec! "uuuu-MM-dd hh:mm aaaa"
#eval fmt.parse "2020-01-01 01:05 PM" |>.map (fun _ => ())
