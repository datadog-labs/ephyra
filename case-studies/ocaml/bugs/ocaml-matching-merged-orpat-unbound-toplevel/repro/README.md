Run: `./repro.sh <prefix>` where `<prefix>/bin/ocaml` is an OCaml install (e.g. trunk built with default
`./configure --prefix=<prefix> && make && make install`). Exit 1 = the toplevel hits
"Fatal error: n unbound at toplevel" (bug present), 0 = not present, 2 = could not run.
