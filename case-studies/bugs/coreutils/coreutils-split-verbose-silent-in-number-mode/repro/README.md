- `repro-standalone.sh [path/to/split]`: runs `-n 2`, `-n l/2`, `-n r/2` with `--verbose`; exit 1 = bug present.
  For a multicall build, symlink `coreutils` as `split` and pass that path.
- `repro.sh <build-dir>`: the harness repro (expects `<build-dir>/bin/split`; compares stdout of
  `split -n 2 --verbose </dev/null` with recorded GNU 9.12 output in `expected/`).
