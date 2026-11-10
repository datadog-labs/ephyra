example (x : BitVec 8) (h : x < x) : False := by grind_linarith
example (x : Fin 3) (h : x < x) : False := by grind
