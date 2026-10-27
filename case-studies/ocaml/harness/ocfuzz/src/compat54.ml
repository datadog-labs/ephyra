(* Trunk (5.5+) stdlib functions the generator uses that the OCaml 5.4.0
   release lacks, written in plain OCaml from their documentation in trunk
   stdlib/int.mli and stdlib/atomic.mli. Only the ref oracle's 5.4.0 compile
   sees this module (-open Compat54). *)
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

module Atomic = struct
  include Stdlib.Atomic
  (* "[update f r] computes a new value for [r] by applying [f] to its
     current value" (retried with compare_and_set) *)
  let rec update f r =
    let v = get r in
    if not (compare_and_set r v (f v)) then update f r
  module Loc = struct
    include Stdlib.Atomic.Loc
    let rec update f r =
      let v = get r in
      if not (compare_and_set r v (f v)) then update f r
  end
  (* "The API below mirrors the API to access atomic references" *)
  module Array = struct
    type 'a t = 'a Stdlib.Atomic.t array
    let make n v = Stdlib.Array.init n (fun _ -> Stdlib.Atomic.make v)
    let init n f = Stdlib.Array.init n (fun i -> Stdlib.Atomic.make (f i))
    let length = Stdlib.Array.length
    let unsafe_length = length
    let get a i = Stdlib.Atomic.get a.(i)
    let set a i v = Stdlib.Atomic.set a.(i) v
    let exchange a i v = Stdlib.Atomic.exchange a.(i) v
    let compare_and_set a i e v = compare_and_set a.(i) e v
    let fetch_and_add a i n = Stdlib.Atomic.fetch_and_add a.(i) n
    let update f a i = update f a.(i)
    let unsafe_get = get
    let unsafe_set = set
    let unsafe_exchange = exchange
    let unsafe_compare_and_set = compare_and_set
    let unsafe_fetch_and_add = fetch_and_add
    let unsafe_update = update
  end
end
