external box : int64_u -> int64 = "%box_int64"
external sub128 : x0:int64_u -> x1:int64_u -> y0:int64_u -> y1:int64_u -> #(int64_u * int64_u) = "" "caml_int128_sub" [@@noalloc] [@@builtin]
external add128 : x0:int64_u -> x1:int64_u -> y0:int64_u -> y1:int64_u -> #(int64_u * int64_u) = "" "caml_int128_add" [@@noalloc] [@@builtin]

(* high word of (b:a) - (a:c) and of (b:a) + (a:c) *)
let[@inline never] sub_hi a b c = let #(_, hi) = sub128 ~x0:a ~x1:b ~y0:c ~y1:a in box hi
let[@inline never] add_hi a b c = let #(_, hi) = add128 ~x0:a ~x1:b ~y0:c ~y1:a in box hi

let () = Printf.printf "%Ld %Ld\n" (sub_hi #5L #100L #1L) (add_hi #5L #100L #1L)
