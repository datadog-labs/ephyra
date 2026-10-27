type t = A | B | C of float * int
let[@inline never] f (x : int) = x
let a = Atomic.make 0
let () =
  let n = Sys.opaque_identity 2 in
  let x = Sys.opaque_identity 1 in
  let old = Atomic.exchange a x in
  for _ = x downto n land 3 do
    match Sys.opaque_identity (if n = 0 then C (0., old) else A) with
      | B when Float.is_nan (Float.fma 1. 1. 1.) -> ()
      | A -> ignore (f 2); raise Exit
  done;
  print_int (Atomic.get a)
