def f (a : Nat) : Nat :=
  match 1 - a, a with
  | n+1, 0 => n
  | _, _ => 0

#check @f.eq_1
example : f 0 = 0 := by simp [f]
