set_option backward.grind.normalizer false in
example (x : BitVec 0) : (0 : BitVec 0) = x ^ 2 := by grind (maxChar := 1)
