def f (a : Nat) : Nat :=
  match id a, a with
  | 0, _ => 1
  | _+1, 0 => 2
  | _, _+1 => 3

#check @f.eq_1
