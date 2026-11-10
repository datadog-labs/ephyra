type _ Effect.t += E : unit Effect.t
let rec deep n = if n = 0 then 0 else 1 + deep (n - 1)
let () =
  let _ = Gc.Memprof.start ~sampling_rate:1.0
    { Gc.Memprof.null_tracker with alloc_minor = fun _ -> ignore (deep 1000); None } in
  Effect.Deep.try_with Effect.perform E
    { effc = fun (type a) (e : a Effect.t) -> match e with
        | E -> Some (fun (k : (a, _) Effect.Deep.continuation) -> Effect.Deep.continue k ())
        | _ -> None };
  print_endline "ok"
