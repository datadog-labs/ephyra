let g = Atomic.make 0
let () =
  let v = Array.length Sys.argv in
  ignore (Atomic.compare_and_set g (Atomic.get g) v);
  print_int (Atomic.get g); print_newline ()
