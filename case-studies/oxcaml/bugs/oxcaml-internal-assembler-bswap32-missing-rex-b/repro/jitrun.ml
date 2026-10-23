let () =
  Clflags.set_target Clflags.Target.Native;
  Opttoploop.set_paths ();
  Opttoploop.initialize_toplevel_env ();
  Jit.init_top ();
  if not (Opttoploop.use_silently Format.err_formatter Sys.argv.(1)) then exit 2
