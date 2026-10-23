let[@inline never] test (a : int32) =
  let g y = let z = Int32.to_int a in y + 1 in
  (Sys.opaque_identity g) 3
let () = print_int (test 7l); print_newline ()
