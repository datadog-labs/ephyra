# Posted: https://github.com/ada-url/ada/issues/1283

Filed by alpaylan, 2026-10-08. Text as posted:

**`canonicalize_protocol` indexes an empty string_view on a protocol pattern of an escaped colon (aborts in hardened builds)**

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

<details>
<summary>repro.cpp</summary>

```cpp
// A protocol pattern of an escaped colon makes canonicalize_protocol index
// an empty string_view. Without library assertions this prints "error" three
// times; with libc++ hardening it aborts on the first pattern:
//
//   clang++ -std=c++20 -D_LIBCPP_HARDENING_MODE=_LIBCPP_HARDENING_MODE_FAST \
//     -DADA_INCLUDE_URL_PATTERN=1 -DADA_USE_UNSAFE_STD_REGEX_PROVIDER \
//     -I<ada singleheader> <ada singleheader>/ada.cpp repro.cpp
#include <cstdio>
#include "ada.h"
int main() {
  using P = ada::url_pattern_regex::std_regex_provider;
  for (const char* src : {"\\:", "\\:?", "\\:*"}) {
    ada::url_pattern_init init; init.protocol = src;
    auto p = ada::parse_url_pattern<P>(std::move(init));
    printf("protocol %-4s -> %s\n", src, p ? "pattern" : "error");
  }
}
```

</details>

<details>
<summary>backtrace_handler.cpp</summary>

```cpp
// Override libc++'s verbose-abort hook to print a backtrace, so a hardening
// assertion shows where it fired without a debugger.
#include <cstdarg>
#include <cstdio>
#include <cstdlib>
#include <execinfo.h>
extern "C" void __attribute__((noreturn)) _ZNSt3__122__libcpp_verbose_abortEPKcz(const char* fmt, ...) {
  va_list ap; va_start(ap, fmt); vfprintf(stderr, fmt, ap); va_end(ap);
  fprintf(stderr, "\n");
  void* bt[64]; int n = backtrace(bt, 64);
  backtrace_symbols_fd(bt, n, 2);
  abort();
}
```

</details>

<details>
<summary>Patch (fix.patch)</summary>

```diff
diff --git a/src/url_pattern_helpers.cpp b/src/url_pattern_helpers.cpp
index 8c11f05f..1cb9693a 100644
--- a/src/url_pattern_helpers.cpp
+++ b/src/url_pattern_helpers.cpp
@@ -241,6 +241,15 @@ tl::expected<std::string, errors> canonicalize_protocol(
     input.remove_suffix(1);
   }
 
+  // A bare ":" is empty once the suffix is removed. process_protocol strips
+  // the trailing ":" of an init value before calling this, so only the
+  // encoding callback gets here with ":", and the spec's canonicalize a
+  // protocol fails on it (":://dummy.invalid/" does not parse). Reading
+  // input[0] below would be out of bounds.
+  if (input.empty()) [[unlikely]] {
+    return tl::unexpected(errors::type_error);
+  }
+
   // Fast path: special schemes are already canonical
   if (scheme::is_special(input)) {
     return std::string(input);
diff --git a/tests/wpt_urlpattern_tests.cpp b/tests/wpt_urlpattern_tests.cpp
index a1d953f5..38ee35bc 100644
--- a/tests/wpt_urlpattern_tests.cpp
+++ b/tests/wpt_urlpattern_tests.cpp
@@ -1372,3 +1372,24 @@ TEST(wpt_urlpattern_tests, urlpattern_test_data) {
   }
   SUCCEED();
 }
+
+TEST(wpt_urlpattern_tests, protocol_bare_colon_does_not_read_out_of_bounds) {
+  // canonicalize_protocol strips a trailing ":" before checking the first
+  // scheme character. The encoding callback can pass it a bare ":" (from an
+  // escaped colon in a protocol pattern), which is empty once stripped: it
+  // must not be read. The spec's canonicalize a protocol fails on ":"
+  // (":://dummy.invalid/" does not parse), so these patterns are errors.
+  for (auto src : {"\\:", "\\:?", "\\:*", "\\:+"}) {
+    ada::url_pattern_init init{};
+    init.protocol = src;
+    ASSERT_FALSE(ada::parse_url_pattern<regex_provider>(init)) << src;
+  }
+  ASSERT_FALSE(ada::parse_url_pattern<regex_provider>("{\\:}?://a"));
+  // An init protocol of ":" is stripped by process protocol for init first,
+  // and canonicalizes to the empty protocol.
+  ada::url_pattern_init colon{};
+  colon.protocol = ":";
+  auto p = ada::parse_url_pattern<regex_provider>(colon);
+  ASSERT_TRUE(p);
+  ASSERT_EQ(p->get_protocol(), "");
+}
```

</details>
