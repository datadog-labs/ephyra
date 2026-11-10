example (n : Nat) (h : 0 + 0 = n + 1) : False := by contradiction
example (n : Nat) (h : n + 1 = 0 + 0) : False := by contradiction
example (h : 2 + 2 = 3) : False := by contradiction
example (h : 0 + 1 = 0) : False := by contradiction
example (h : 0 + 0 = 1) : False := by simp at h
example (h : 0 + 0 = 1) : False := by omega
