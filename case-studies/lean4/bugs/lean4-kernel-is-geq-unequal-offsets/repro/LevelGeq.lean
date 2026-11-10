import Lean
open Lean
#eval (Level.succ (.succ (.param `u))).geq (.succ (.imax (.succ (.param `u)) (.param `u)))
