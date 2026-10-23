[@@@ocaml.warning "-a"]
(* Minimal bisect_ppx runtime vendored for compiler coverage builds. *)
val register_file :
  bisect_file:string option ->
  bisect_silent:string option ->
  bisect_sigterm:bool ->
  filename:string ->
  points:int array ->
  [`Visit of (int -> unit)]
