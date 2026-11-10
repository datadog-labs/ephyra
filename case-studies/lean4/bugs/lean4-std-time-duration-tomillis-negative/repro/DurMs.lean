import Std.Time
open Std.Time

def d : Duration := Duration.ofNanoseconds (-1)

#eval d.toMilliseconds
#eval d.toNanoseconds.toMilliseconds
#eval (Duration.ofNanoseconds 1).toMilliseconds
#eval (Timestamp.ofNanosecondsSinceUnixEpoch (-1)).toMillisecondsSinceUnixEpoch
