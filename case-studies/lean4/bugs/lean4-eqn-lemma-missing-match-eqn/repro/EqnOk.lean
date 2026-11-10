def f : Nat := 2 * (match 0 with | 0 => 1 | _ => 3)
theorem t : f = 2 := by simp [f]
def g : Nat :=
  Nat.divExact (2 * (match true with | true => 1 | false => 3)) _ (Nat.dvd_mul_left _ 2)
theorem tg : g = 2 := by simp [g]
