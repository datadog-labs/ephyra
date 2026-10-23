# ada-coverage

The ada code that [ephyra-ada](../generator)'s cases exercise, read
in-process from an instrumented build of the same vendored ada: the
analogue of [sqlite-coverage](../../sqlite/coverage) for ada, used to score
generator changes. It depends on `../generator`, whose `build.rs` compiles
ada and the shim with the `CXXFLAGS` set in `.cargo/config.toml`
(`-fprofile-instr-generate -fcoverage-mapping`); Rust code is not
instrumented, so the counters are ada's alone.

- [src/lib.rs](src/lib.rs): `is_ada` keeps ada's own functions
  (Itanium-mangled in the `ada` namespace, `_ZN3ada`/`_ZNK3ada`, so the
  shim and standard-library instantiations are left out), `ada_counters()`,
  and `covered(within, f)`, the counters that running `f` executes.
- `tests/smoke.rs`: the counters are readable and parsing reaches them
  (`https://example.com/a` reaches far fewer than `http://0x7f.1/` or
  `http://[::ffff:1.2.3.4]/`). Run single-threaded for exact attribution:
  `cargo test -- --test-threads=1`. Profile files go to `target/profraw/`.
- `cargo run --release --example score -- [SEEDS_PER_GENERATOR]`: the joint
  coverage score of ephyra-ada's generators (URL cases on both
  representations, search params, URLPattern, pattern lists, URLPatternInit
  inputs): distinct ada counters reached from a fixed seed set, the same on
  a disjoint fresh seed range, and a breakdown by area. `EA_GAPS=N` lists
  the N functions missing the most counters (mangled; pipe through
  `c++filt`). The rounds run with it are described in
  [../generator/README.md](../generator/README.md#synthesis-rounds).
- `cargo run --release --example unreached -- [CASES]`: every ada function
  with the share of its counters the grammar executes, least covered first.
- Per-line gaps: `llvm-cov show` on the score binary with the newest
  `target/profraw/` file (merged with `llvm-profdata`).

## Platform

`.cargo/config.toml` links clang's profile runtime by absolute path, as
configured for macOS on Apple silicon with the Command Line Tools' clang 21:

```
/Library/Developer/CommandLineTools/usr/lib/clang/21/lib/darwin/libclang_rt.profile_osx.a
```

The runtime must come from the clang that compiles ada (the `cc` crate's
C++ compiler). With another clang, replace that path with
`$(clang -print-resource-dir)/lib/darwin/libclang_rt.profile_osx.a`. On
Linux the library is `lib/linux/libclang_rt.profile-x86_64.a` (or
`lib/<target>/libclang_rt.profile.a`, depending on the LLVM build) under the
same resource directory; this configuration has only been used on macOS.
