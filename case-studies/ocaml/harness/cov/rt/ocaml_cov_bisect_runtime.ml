[@@@ocaml.warning "-a"]
(* Minimal bisect_ppx native runtime (adapted from bisect_ppx
   src/runtime/native/runtime.ml, MIT), vendored into the compiler's utils/
   for coverage builds.  Differences from upstream:
   - no Unix dependency (uses open_out_gen with Open_excl);
   - coverage is written ONLY if BISECT_FILE is set in the environment
     (so the instrumented compiler does not litter the cwd);
   - BISECT_SILENT defaults to YES (no messages; set BISECT_SILENT=ERR to see
     errors on stderr). *)

module Common = Ocaml_cov_bisect_common

let verbose msg =
  match Sys.getenv_opt "BISECT_SILENT" with
  | Some s when String.uppercase_ascii s = "ERR" -> prerr_endline (" *** " ^ msg)
  | _ -> ()

let dump () =
  match Sys.getenv_opt "BISECT_FILE" with
  | None | Some "" -> ()
  | Some prefix ->
    let prefix =
      if Filename.is_implicit prefix then
        Filename.concat Filename.current_dir_name prefix
      else prefix in
    let data =
      Common.coverage |> Lazy.force |> Common.write_coverage in
    let rec create attempts =
      let filename = Common.random_filename ~prefix in
      match
        open_out_gen [Open_wronly; Open_creat; Open_excl; Open_binary]
          0o644 filename
      with
      | oc -> Some oc
      | exception Sys_error e ->
        if attempts > 0 && Sys.file_exists filename then create (attempts - 1)
        else (verbose ("Bisect runtime was unable to create file: " ^ e); None)
    in
    match create 100 with
    | None -> ()
    | Some oc ->
      (try output_string oc data; flush oc
       with _ -> verbose "Bisect runtime was unable to write file.");
      close_out_noerr oc

let written = ref false

let dump_at_exit () =
  if not !written then begin
    written := true;
    (try dump () with _ -> ())
  end

let register_dump : unit Lazy.t = lazy (at_exit dump_at_exit)

let register_file ~bisect_file:_ ~bisect_silent:_ ~bisect_sigterm:_
    ~filename ~points =
  Lazy.force register_dump;
  Common.register_file ~filename ~points
