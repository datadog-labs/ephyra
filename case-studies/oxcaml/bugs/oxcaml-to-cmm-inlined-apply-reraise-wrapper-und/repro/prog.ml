let f g h x = let r = ref x in try g (h x) with _ -> !r
let () = print_int (f (fun _ -> raise Exit) succ 41)
