def f (_u : Unit) : List Nat :=
  let xs : Array UInt64 := #[12, 17, 11]
  (Array.modifyM (m := Id) xs 3 (fun (x : UInt64) => (pure (x + 1) : Id UInt64))).run.toList.map UInt64.toNat
def main : IO Unit := IO.println (f ())
