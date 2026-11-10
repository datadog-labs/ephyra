let a = Bigarray.(Array1.create float32 c_layout 1)
let f x = a.{0} <- x; x +. 1.
let () = let y = f 2.5 in Printf.printf "%g %g\n" y a.{0}
