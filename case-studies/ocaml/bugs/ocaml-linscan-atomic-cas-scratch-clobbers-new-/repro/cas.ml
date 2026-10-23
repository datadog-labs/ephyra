let r = Atomic.make false
let () =
  let ok = Atomic.compare_and_set r false (Sys.opaque_identity true) in
  Printf.printf "%b %b\n" ok (Atomic.get r)
