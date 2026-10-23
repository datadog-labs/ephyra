**Title:** `canonicalize_protocol` indexes an empty string_view on a protocol pattern of an escaped colon (aborts in hardened builds)

`url_pattern_helpers::canonicalize_protocol` checks for an empty input before stripping a trailing `:`, not after. When the URLPattern parser's encoding callback passes it exactly `":"`, the view is empty after `remove_suffix(1)`, and the scheme-character check then calls `input[0]` on an empty `std::string_view`, which is a precondition violation:

```cpp
tl::expected<std::string, errors> canonicalize_protocol(std::string_view input) {
  if (input.empty()) [[unlikely]] {   // ":" is not empty here
    return "";
  }
  if (input.ends_with(":")) {
    input.remove_suffix(1);           // now empty
  }
  if (scheme::is_special(input)) { ... }
  uint8_t first_flags = char_class_table[static_cast<uint8_t>(input[0])];  // index 0 of an empty view
```

**Reach.** Any protocol pattern whose pending fixed value is a lone escaped colon: init `{protocol: "\\:"}`, `"\\:?"`, `"\\:*"`, `"\\:+"`, or the constructor string `"{\\:}?://a"`. In Node.js (ada 4.0.0) that is `new URLPattern({ protocol: "\\:" })`. Present in v4.0.0 and on main (4eee4c2).

**Impact, precisely.**
- *Plain builds (no library assertions):* no visible effect. The empty view still points at the `:` that was just stripped, so `input[0]` reads that same byte inside the caller's string. It always reads `:`, which is not a scheme character, so the call returns a type error and the pattern is rejected. That is also the result the spec requires (below). AddressSanitizer does not report it, because the byte is inside a live allocation.
- *Hardened builds:* the process aborts. With libc++ hardening, including the `fast` mode recommended for production (`-D_LIBCPP_HARDENING_MODE=_LIBCPP_HARDENING_MODE_FAST`), `string_view::operator[]` traps on the first such pattern (`repro.cpp`; exit by SIGTRAP). So any service built that way, which compiles URLPatterns from untrusted strings, can be crashed by one pattern. libstdc++'s `_GLIBCXX_ASSERTIONS`, which some distributions enable by default, asserts in the same accessor; I have not been able to test it here.

So this is a crash in hardened deployments, not memory corruption or a leak.

**What the spec says.** "canonicalize a protocol" parses the value followed by `://dummy.invalid/`, and throws a TypeError on failure. For `:` that is `:://dummy.invalid/`, which fails to parse, so `:` must be an error. A `:` in an init value never reaches this point: "process protocol for init" strips one trailing `:` first, and ada's `url_pattern_init::process_protocol` does the same before calling `canonicalize_protocol`. So the empty-after-stripping case comes only from the encoding callback.

**Fix** (`fix.patch`). Return a type error when the input is empty after stripping:

```cpp
  if (input.ends_with(":")) {
    input.remove_suffix(1);
  }
  if (input.empty()) [[unlikely]] {
    return tl::unexpected(errors::type_error);
  }
```

Observable results in a plain build are unchanged, because the stray read already led to an error. The init value `":"` still canonicalizes to the empty protocol. urlpattern-polyfill 10.1.0 and Deno's `urlpattern` 0.6 also reject `\:?`, `\:*` and `\:+`; for `\:` Deno rejects it and the polyfill accepts it.

New test `protocol_bare_colon_does_not_read_out_of_bounds` (`tests/wpt_urlpattern_tests.cpp`) asserts these patterns are errors and the init `":"` is the empty protocol. It catches the bug only in a build with library assertions: in a plain build the test passes with or without the fix, because the stray read happens to give the right answer. ada's CI runs ASan, UBSan and `-fsanitize=bounds`, none of which detect it; a CI job with libc++ hardening or `_GLIBCXX_ASSERTIONS` would. On main 4eee4c2 with the patch, all 404 tests pass.

**How it was found.** Generated URLPattern cases, run on ada built with ASan, UBSan and libc++ extensive hardening; the hardening check fired and the harness saved the case. It was reduced by hand. The stack trace came from a libc++ verbose-abort override (`backtrace_handler.cpp`), with no debugger:

```
string_view:417: libc++ Hardening assertion __pos < size() failed: string_view[] index out of bounds
  ada::url_pattern_helpers::canonicalize_protocol
  ada::url_pattern_helpers::url_pattern_parser<...>::maybe_add_part_from_the_pending_fixed_value
  ada::url_pattern_helpers::parse_pattern_string<...>
  ...
  ada::parse_url_pattern<...>
```
