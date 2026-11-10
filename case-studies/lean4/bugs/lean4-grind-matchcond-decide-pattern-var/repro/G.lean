def f0 (x : Nat) := match (x, 1) with | (a, 0) => a | _ => 7
theorem t : f0 3 = 7 := by grind [f0]
