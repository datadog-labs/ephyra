-- `4 * x % 3` is `x % 3`, so the goal is `p → p`.
example (x : Int) : x % 3 ≤ 2 * x % 3 → 4 * x % 3 ≤ 2 * x % 3 := by lia

example (x : Int) : x % 3 ≤ 2 * x % 3 → 4 * x % 3 ≤ 2 * x % 3 := by grind
