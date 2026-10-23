let[@inline never] g x = x + 1

let f x =
  let r = g x in
  let _ = Int64.to_float 5L in
  r

let () = print_int (f 41)
