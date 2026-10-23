type t = A of int | B of int | C of int
let f x = let _ = try (match Sys.opaque_identity (B x) with A _ -> 0 | B y -> y | C _ -> truncate (sqrt 2.0)) with Exit -> 0 in x
