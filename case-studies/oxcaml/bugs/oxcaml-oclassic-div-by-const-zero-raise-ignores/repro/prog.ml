let () = let r = ref 0 in incr r; print_int (try 1 / 0 with _ -> !r)
