let s = "abcdefghijklmnopqrstuvwxyz"
let[@inline never] f () =
  let x0 = String.get_int32_be s 0 in
  let x1 = String.get_int32_be s 1 in
  let x2 = String.get_int32_be s 2 in
  let x3 = String.get_int32_be s 3 in
  let x4 = String.get_int32_be s 4 in
  let x5 = String.get_int32_be s 5 in
  let x6 = String.get_int32_be s 6 in
  1 * Int32.to_int x0 + 2 * Int32.to_int x1 + 3 * Int32.to_int x2 + 4 * Int32.to_int x3 + 5 * Int32.to_int x4 + 6 * Int32.to_int x5 + 7 * Int32.to_int x6
let () = print_int (f ()); print_newline ()
