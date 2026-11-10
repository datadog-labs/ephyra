def g (a : Nat) : Nat :=
  match 7 - 0, a with
  | 0, _ => 1
  | _+1, 0 => 2
  | _, _+1 => 3
example : g 0 = 2 := by simp [g]
