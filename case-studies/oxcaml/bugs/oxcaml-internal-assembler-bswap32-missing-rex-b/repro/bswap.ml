let[@inline never] f s =
  let a = String.get_int32_be s 0 in
  let b = String.get_int32_be s 1 in
  let c = String.get_int32_be s 2 in
  let d = String.get_int32_be s 3 in
  let e = String.get_int32_be s 4 in
  let g = String.get_int32_be s 5 in
  Int32.(to_int a + to_int b + to_int c + to_int d + to_int e + to_int g)
let () = print_int (f "abcdefghij"); print_newline ()
