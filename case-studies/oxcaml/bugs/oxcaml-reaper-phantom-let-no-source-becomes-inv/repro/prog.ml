let f a = fun y -> let _z = a + 1 in y + 1
let () = print_int (f 7 3); print_newline ()
