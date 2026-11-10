import Std.Http
open Std.Http

def u : URI := URI.Builder.empty
  |>.setScheme! "http" |>.setUserInfo "a:b" (some "c") |>.setHost! "example.com" |>.build

def creds (u : Option URI) := (u.bind (·.authority) |>.bind (·.userInfo)).map fun i => (i.username?, i.password?)

#eval toString u              -- the printed URI
#eval creds (some u)          -- username/password given to the builder
#eval creds (URI.parse? (toString u))  -- username/password parsed back from the printed URI
