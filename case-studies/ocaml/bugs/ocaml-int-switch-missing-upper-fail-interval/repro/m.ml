let f x y = match x, y with
  | _, true -> 0
  | (0 | 2), _ -> 1
  | 3, true -> 2
  | -1, _ -> 3
let () = print_int (f 4 false)
