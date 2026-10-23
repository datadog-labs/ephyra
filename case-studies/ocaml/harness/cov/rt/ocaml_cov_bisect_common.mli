[@@@ocaml.warning "-a"]
(* Vendored from bisect_ppx src/common/bisect_common.mli (MIT), trimmed. *)
type instrumented_file = {
  filename : string;
  points : int array;
  counts : int array;
}
type coverage = (string, instrumented_file) Hashtbl.t
val coverage : coverage Lazy.t
val coverage_file_identifier : string
val register_file :
  filename:string -> points:int array -> [`Visit of (int -> unit)]
val reset_counters : unit -> unit
val runtime_data_to_string : unit -> string option
val write_coverage : coverage -> string
val random_filename : prefix:string -> string
