set_option backward.grind.normalizer false in
example (x : Fin 1) : (0 : Fin 1) = x ^ 2 := by grind (maxChar := 1)
