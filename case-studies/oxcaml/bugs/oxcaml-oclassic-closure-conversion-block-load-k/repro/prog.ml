type t = A of float# | B of float
let f () = match B 1.0 with A y -> Stdlib_upstream_compatible.Float_u.floor y | B _ -> #0.0
let () = print_float (Stdlib_upstream_compatible.Float_u.to_float (f ()))
