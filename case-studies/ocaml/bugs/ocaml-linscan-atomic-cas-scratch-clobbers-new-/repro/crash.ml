let n = Array.length Sys.argv
let g = Atomic.make 0
let () = ignore (Atomic.compare_and_set g (Atomic.get g) n)
