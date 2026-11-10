type t4 = #(int * int * int * int)
type t16 = #(t4 * t4 * t4 * t4)
type t64 = #(t16 * t16 * t16 * t16)
type t256 = #(t64 * t64 * t64 * t64)
let f (_ : t256) (y : int) = y
let () = print_endline "ok"
