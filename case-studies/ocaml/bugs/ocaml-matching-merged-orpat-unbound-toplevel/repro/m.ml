let f x = match x with (`B m | `C m), _ -> 1 | (`B n | `C n), (1 | 2) -> n
let () = print_int (f (`B 5, 1)); print_newline ()
