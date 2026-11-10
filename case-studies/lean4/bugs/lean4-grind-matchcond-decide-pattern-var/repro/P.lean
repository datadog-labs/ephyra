def p (s : Nat × Bool) : Nat := match s with | (_, true) => 0 | _ => 1
theorem t : p (3, false) = 1 := by grind [p]
