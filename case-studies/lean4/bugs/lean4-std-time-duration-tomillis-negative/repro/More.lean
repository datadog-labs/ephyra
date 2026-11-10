import Std.Time
open Std.Time
#eval (Duration.ofNanoseconds (-1000001)).toMilliseconds
#eval (Duration.ofNanoseconds (-1000001)).toNanoseconds.toMilliseconds
