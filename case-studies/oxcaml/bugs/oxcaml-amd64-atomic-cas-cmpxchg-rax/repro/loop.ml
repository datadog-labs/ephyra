let[@inline never] wait_then_set r (seen : int) (v : int) =
  let n = ref 0 in
  while not (Atomic.compare_and_set r seen v) do incr n; if !n > 1000 then exit 3 done;
  !n
let () =
  let r = Atomic.make 5 in
  let n = wait_then_set r 1 7 in
  Printf.printf "returned after %d failures, r=%d\n" n (Atomic.get r)
