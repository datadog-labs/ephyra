# ada-asan

[ephyra-ada](../generator)'s properties, unchanged (`tests/oracles.rs`
includes `../generator/tests/oracles/properties.rs`), on ada built with
AddressSanitizer, UBSan and libc++ extensive hardening, every error fatal.
`.cargo/config.toml` sets the `CXXFLAGS` that ephyra-ada's `build.rs` passes
to ada and the shim; Rust code is not instrumented. On Darwin the ASan
runtime carries UBSan's, so linking it is enough.

- A sanitizer abort kills the process. With `EA_SANITIZE` (in `CXXFLAGS`),
  the shim's signal handler first writes the case being run to
  `$EA_CRASH_FILE` (`target/crash.bin`). [src/lib.rs](src/lib.rs):
  `read_crash` decodes it, `replay` runs it again;
  `cargo run --example replay [-- FILE]` does that under the sanitizers.
- `tests/canary.rs` checks that pipeline end to end: a child process reads
  past a heap buffer inside the shim while a known case is current; the test
  expects a sanitizer report and the case back from the crash file.
- `cargo test` runs the canary and the properties; `EPHYRA_TESTS` and
  `EPHYRA_MAX_SIZE` as in ephyra-ada. The first seven properties once ran
  clean at 1,000,000 cases each; thirteen with the current generators at
  150,000 each (max size 250), the two added last
  (`init_test_agrees_with_exec`, `predicates_agree_with_components`) at
  30,000.
- libc++ hardening is what catches
  [ada-url/ada#1283](https://github.com/ada-url/ada/issues/1283)
  ([../bugs/ada-urlpattern-protocol-colon-oob](../bugs/ada-urlpattern-protocol-colon-oob/)):
  an index into an empty `string_view` that stays inside the same string,
  so ASan does not see it.

## Platform

`.cargo/config.toml` is configured for macOS on Apple silicon with the
Command Line Tools' clang 21:

- `build.target = "aarch64-apple-darwin"`: an explicit target keeps the link
  flags off build scripts and proc macros, which rustc loads with `dlopen`,
  too late for ASan. Keep a target, but make it the host's
  (`rustc -vV | grep host`).
- The ASan runtime and its rpath by absolute path:
  `/Library/Developer/CommandLineTools/usr/lib/clang/21/lib/darwin/libclang_rt.asan_osx_dynamic.dylib`.
  It must come from the clang that compiles ada; with another clang,
  replace both paths with `$(clang -print-resource-dir)/lib/darwin`.

On Linux, the runtime is `libclang_rt.asan-x86_64.so` (or
`lib/<target>/libclang_rt.asan.so`) under the same resource directory, and
UBSan's runtime may need linking too; this configuration has only been used
on macOS.
