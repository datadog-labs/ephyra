def f2 (x : Nat) (y : UInt8) : Nat :=
  match (x, y) with
  | (a, 0) => a
  | _ => 7
example : f2 3 1 = 7 := by grind [f2]
