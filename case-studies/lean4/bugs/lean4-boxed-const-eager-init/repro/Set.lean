@[inline] def bump (xs : Array UInt64) (i : Nat) : Array UInt64 :=
  if h : i < xs.size then xs.set i (xs[i] + 1) else xs

def main : IO Unit := IO.println (bump #[4] 1)
