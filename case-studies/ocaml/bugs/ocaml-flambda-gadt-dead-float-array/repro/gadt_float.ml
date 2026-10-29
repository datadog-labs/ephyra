type _ t = Int : int t | Float : float t
let () =
  let f (type a) (t : a t) (x : a) =
    match t with Float -> ignore [| x |] | Int -> ()
  in
  f Int 0;
  print_endline "ok"
