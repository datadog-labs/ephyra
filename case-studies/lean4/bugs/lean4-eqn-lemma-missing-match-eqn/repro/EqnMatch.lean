def f : Nat :=
  Nat.divExact (2 * (match 0 with | 0 => 1 | _ => 3)) _ (Nat.dvd_mul_left _ 2)

theorem t : f = 2 := by simp [f]
