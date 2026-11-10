def f (a : Nat) : Nat :=
  match id a, a with
  | 0, _ => 1
  | _+1, 0 => 2
  | _, _+1 => 3

#eval f 0
example : f 0 = 1 := by simp [f]
example : f 0 = 1 := by unfold f; rfl
example : f 0 = 1 := by rw [f]
