**Title:** x86-64 builds execute SSSE3 instructions unconditionally since #1216/#1230 (illegal instruction on CPUs without SSSE3)

On x86-64 with GCC or Clang, `ADA_PARSER_SSSE3` and `ADA_UNICODE_SSSE3` are set to 1 even when the translation unit is compiled for baseline x86-64 (SSE2): the kernels are marked `__attribute__((target("ssse3")))` (`ADA_PARSER_NEED_SSSE3_TARGET`, `ADA_UNICODE_NEED_SSSE3_TARGET`). That attribute lets the compiler emit SSSE3 inside those functions. It does not check at run time that the CPU has SSSE3, and nothing in ada does (no `__builtin_cpu_supports`/cpuid). The kernels are called unconditionally.

A `-march=x86-64` build of main (4eee4c2) contains `pshufb`/`palignr` in five hot functions (`objdump-baseline.txt`): `scan_plain_host`, `scan_path_run`, `scan_query_run`, `scan_hash_run` and `percent_encode_to_ssse3`. On an x86-64 CPU without SSSE3, executing these instructions raises SIGILL, so parsing URLs that reach these scanners would crash. CPUs without SSSE3 include AMD K8 and K10 (Athlon 64, Phenom II, Athlon II) and QEMU's default `qemu64` CPU model. Programs and distributions that build for the x86-64 baseline expect the library to run there.

Introduced by #1216 (2026-08-17, nibble-table SIMD classification) in the parser scanners, extended by later commits, and added to `percent_encode` by #1230 (2026-08-21). Counting `pshufb`/`palignr` in a `-march=x86-64` build: v4.0.0 has 0, the commit before #1216 has 0, #1216 has 16, the commit before #1230 has 32, #1230 and main have 41 (9 of them in `percent_encode_to_ssse3`). Not in a release yet (v4.0.0 is 2026-07-27).

I could not run on such a CPU (this was found on Apple silicon; Rosetta 2 has SSSE3), so the crash itself is not demonstrated here, only the instructions and the missing dispatch.

**Fix options.** (a) Use the SSSE3 kernels only when `__SSSE3__` is defined, falling back to the SSE2 or scalar code otherwise (drop the two `NEED_SSSE3_TARGET` branches); or (b) keep them and select them at run time once (`__builtin_cpu_supports("ssse3")`, cached), as simdutf and simdjson do.

The SSSE3 kernels themselves agree with the scalar and NEON paths: all properties (parse, setters, round trips, length limits, URLSearchParams, against whatwg-url) pass at 300,000 cases each in an x86-64 build run under Rosetta.
