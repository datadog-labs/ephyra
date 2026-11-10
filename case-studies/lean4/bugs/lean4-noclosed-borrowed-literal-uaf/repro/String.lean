@[noinline] def mk (_s : String) (n : Nat) : List Nat := List.replicate n 0
@[noinline] def g (n : Nat) : Bool :=
  let r := mk "" n
  (match r with | [] => "ab" | _ :: _ => "").endsWith (("".drop 6).toString.toLower)
def main : IO Unit := IO.println (g 1)
