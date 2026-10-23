type t = A | B | C
let f v = let _ = try (match v with A -> 0 | B -> 1 | C -> truncate (sqrt 2.0)) with Exit -> 0 in print_endline "ok"
let () = f A; f B; f C
