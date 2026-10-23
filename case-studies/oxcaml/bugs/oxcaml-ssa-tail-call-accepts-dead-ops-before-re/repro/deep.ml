let rec f n = if n = 0 then 0 else let r = f (n - 1) in let _ = Int64.to_float 5L in r
let () = print_int (f 100_000_000); print_newline ()
