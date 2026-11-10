def f (s : Float32) (n : Nat) : Nat :=
  match s, n with
  | 16777216, 0 => 6
  | 16777217, _ => 7
  | _, _ => 0
#eval f 16777217 1
example : f 16777217 1 = 7 := by decide
