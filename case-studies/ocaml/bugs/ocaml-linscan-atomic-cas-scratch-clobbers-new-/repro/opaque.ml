let g = Atomic.make false
let () =
  let b = Sys.opaque_identity true in
  ignore (Atomic.compare_and_set g (Atomic.get g) b);
  print_endline (if Atomic.get g then "T" else "F")
