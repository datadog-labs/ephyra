let f = function (Ok _ | Error _), _ -> 0 | (Ok n | Error n), (0 | 1) -> n
let () = print_int (f (Ok 5, 0)); print_newline ()
