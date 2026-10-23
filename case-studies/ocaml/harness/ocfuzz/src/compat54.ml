(* Trunk (5.5+) stdlib functions the generator uses, written in plain OCaml
   from their documentation in trunk stdlib/int.mli, so that OCaml 5.4.0 can
   compile the generated programs (opened with -open Compat54). *)
module Int = struct
  include Stdlib.Int
  (* "the number of 1 bits in the binary representation of [n]" *)
  let popcount n =
    let rec go n c i = if i = Sys.int_size then c else go (n lsr 1) (c + (n land 1)) (i + 1) in
    go n 0 0
  (* "number of leading (most significant) 0 bits"; 0 for negative n,
     Sys.int_size iff n = 0 *)
  let leading_zeros n =
    let rec go n c = if n = 0 then c else go (n lsr 1) (c - 1) in
    go n Sys.int_size
  (* "largest integer i <= Sys.int_size such that 2^i divides n evenly" *)
  let trailing_zeros n =
    if n = 0 then Sys.int_size
    else let rec go n c = if n land 1 = 1 then c else go (n lsr 1) (c + 1) in go n 0
end
