def g (s : Float) (n : Nat) : Nat :=
  match s, n with
  | 5, 0 => 6
  | 5.0, _ => 7
  | _, _ => 0
#eval g 5 1
def h (s : Float) (n : Nat) : Nat :=
  match s, n with
  | 1.0, 1 => 8
  | 1, _ => 9
  | _, _ => 0
#eval h 1 0
example : (16777216 : Float32) = 16777217 := by decide
example : (16777216 : Float32) = 16777217 := rfl
