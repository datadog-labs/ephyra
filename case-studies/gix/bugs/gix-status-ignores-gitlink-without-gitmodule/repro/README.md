Run: `./repro.sh /path/to/gix` (the gitoxide CLI binary, e.g. `cargo build --release --bin gix`).
Exit 1 = bug reproduces (git reports ` M d`, gix prints nothing), 0 = fixed, 2 = setup failed.
On trunk 59ba38b89 with git 2.55.0: exit 1.
