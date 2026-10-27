(* Prelude shared by every generated program (module H). Printers and total
   ("safe") helpers so that generated pure expressions never raise. *)
exception E0
exception E1 of int
exception E2 of string * float

type r = { mutable ri : int; rf : float; mutable rl : int64 }
type fr = { mutable fx : float; fy : float }
type v = V0 | V1 | V2 of int | V3 of float * int | V4 of int64
       | V5 of { mutable vx : int; vy : float }
type pv = [ `A | `B of int | `C of float * int | `D of string ]
module type S = sig val k : int val f : int -> int end

let p_int x = print_string (string_of_int x); print_char ' '
let p_bool x = print_string (if x then "T " else "F ")
(* Float formatting: %h is exact; nan sign/payload is unspecified, so print "nan". *)
let p_float x =
  if Float.is_nan x then print_string "nan " else (print_string (Printf.sprintf "%h" x); print_char ' ')
let p_i32 x = print_string (Int32.to_string x); print_string "l "
let p_i64 x = print_string (Int64.to_string x); print_string "L "
let p_nat x = print_string (Nativeint.to_string x); print_string "n "
let p_str x = print_string (Printf.sprintf "%S" x); print_char ' '
let p_opt f = function None -> print_string "None " | Some x -> print_string "Some("; f x; print_string ") "
let p_pair f g (a, b) = print_char '('; f a; print_char ','; g b; print_string ") "
let p_list f l = print_char '['; List.iter f l; print_string "] "
let p_arr f a = print_string "[|"; Array.iter f a; print_string "|] "
let p_ref f r = print_string "ref("; f !r; print_string ") "
let p_r r = print_char '{'; p_int r.ri; p_float r.rf; p_i64 r.rl; print_string "} "
let p_fr r = print_char '{'; p_float r.fx; p_float r.fy; print_string "} "
let p_v = function
  | V0 -> print_string "V0 " | V1 -> print_string "V1 "
  | V2 i -> print_string "V2 "; p_int i
  | V3 (f, i) -> print_string "V3 "; p_float f; p_int i
  | V4 l -> print_string "V4 "; p_i64 l
  | V5 { vx; vy } -> print_string "V5 "; p_int vx; p_float vy
let p_pv : pv -> unit = function
  | `A -> print_string "`A " | `B i -> print_string "`B "; p_int i
  | `C (f, i) -> print_string "`C "; p_float f; p_int i
  | `D s -> print_string "`D "; p_str s
let p_atom f a = print_string "atom("; f (Atomic.get a); print_string ") "

let p_exn = function
  | E0 -> print_string "E0"
  | E1 i -> print_string "E1 "; p_int i
  | E2 (s, f) -> print_string "E2 "; p_str s; p_float f
  | e -> print_string (Printexc.to_string e)

let run name f =
  print_string "\n=="; print_string name; print_char '\n';
  (try f () with e -> print_string "\n!exn "; p_exn e)

(* total helpers *)
let[@inline] get a i = a.((i land max_int) mod Array.length a)
let[@inline] set a i x = a.((i land max_int) mod Array.length a) <- x
let[@inline] nz x = x lor 1
let[@inline] clampf lo hi x = if Float.is_nan x then 0. else Float.max lo (Float.min hi x)
let[@inline] f2i x = int_of_float (clampf (-4.5e15) 4.5e15 x)
let[@inline] f2i32 x = Int32.of_float (clampf (-2.0e9) 2.0e9 x)
let[@inline] f2i64 x = Int64.of_float (clampf (-4.5e15) 4.5e15 x)
let[@inline] f2nat x = Nativeint.of_float (clampf (-4.5e15) 4.5e15 x)
let[@inline] bits64 x = if Float.is_nan x then 0L else Int64.bits_of_float x
let[@inline] bits32 x = if Float.is_nan x then 0l else Int32.bits_of_float x
let[@inline] sget s i = if s = "" then 0 else Char.code (String.unsafe_get s ((i land max_int) mod String.length s))
let[@inline] ssub s i = let n = String.length s in let i = (i land max_int) mod (n + 1) in String.sub s i (n - i)

(* prefix of a (possibly cyclic) list *)
let rec take n l = if n <= 0 then [] else match l with [] -> [] | x :: r -> x :: take (n - 1) r

(* effects: fixed handlers, generated code performs *)
type _ Effect.t += Eff : int -> int Effect.t | Eff2 : float -> float Effect.t
let handle_cont f =
  Effect.Deep.match_with f ()
    { retc = (fun x -> x); exnc = raise;
      effc = (fun (type a) (e : a Effect.t) -> match e with
        | Eff n -> Some (fun (k : (a, _) Effect.Deep.continuation) -> Effect.Deep.continue k (n + 1))
        | Eff2 x -> Some (fun (k : (a, _) Effect.Deep.continuation) -> Effect.Deep.continue k (x *. 2.))
        | _ -> None) }
let handle_disc f =
  Effect.Deep.try_with f ()
    { effc = (fun (type a) (e : a Effect.t) -> match e with
        | Eff n -> Some (fun (k : (a, _) Effect.Deep.continuation) ->
            if n land 1 = 0 then Effect.Deep.continue k (n * 3) else Effect.Deep.discontinue k (E1 n))
        | _ -> None) }
let handle_shallow f =
  let open Effect.Shallow in
  let rec go : type a. (a, _) continuation -> a -> _ = fun k v ->
    continue_with k v
      { retc = (fun x -> x); exnc = raise;
        effc = (fun (type b) (e : b Effect.t) -> match e with
          | Eff n -> Some (fun (k : (b, _) continuation) -> go k (n - 1))
          | Eff2 x -> Some (fun (k : (b, _) continuation) -> go k (x +. 1.))
          | _ -> None) }
  in
  go (fiber f) ()

(* recursive value definitions (lambda/value_rec_compiler.ml): cyclic records
   and variants, observed through bounded walks *)
type knot = { kv : int; kr : fr; kn : knot; kg : int -> int }
type kv = K0 | K1 of int * kv | K2 of { k2 : int; k2n : kv; k2f : int -> int }
let rec knot0 = { kv = 7; kr = { fx = 0.5; fy = -1. }; kn = knot0; kg = (fun x -> x + 1) }
let rec ksum n k = if n <= 0 then 0 else k.kv + k.kg 0 + ksum (n - 1) k.kn
let rec kvsum n v =
  if n <= 0 then 0
  else match v with
    | K0 -> 1
    | K1 (i, w) -> i + kvsum (n - 1) w
    | K2 r -> r.k2 + r.k2f 0 + kvsum (n - 1) r.k2n
