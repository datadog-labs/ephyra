@[noinline] def mkN (_s : Nat) (n : Nat) : List Nat := List.replicate n 0
@[noinline] def h (n : Nat) : Nat :=
  let r := mkN 100000000000000000000000 n
  (match r with | [] => 7 | _ :: _ => 100000000000000000000000) + (100000000000000000000000 + n)
def main : IO Unit := IO.println (h 1)
