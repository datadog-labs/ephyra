# Posted: https://github.com/ada-url/ada/issues/1284

Filed by alpaylan, 2026-10-08. Text as posted:

**URLPattern opaque pathname: a leading `//` is parsed as an authority and dropped (`//x/y` becomes `/y`)**

For a pattern whose protocol is not special, the pathname is canonicalized with "canonicalize an opaque pathname", which runs the opaque path state, as a state override, on a dummy URL. That state appends every code point to the path (C0 controls percent-encoded); a leading `//` is just two characters. `url_pattern_helpers::canonicalize_opaque_pathname` instead parses the whole URL `"fake:" + value`. When the value starts with `//`, that is `fake://…`, an authority, so the host-looking part disappears from the pathname:

```cpp
ada::url_pattern_init init;
init.protocol = "foo";
init.pathname = "//x/y";
ada::parse_url_pattern<Provider>(std::move(init))->get_pathname();   // ada: "/y"   spec: "//x/y"
// "//x" gives "" and "//" gives ""
```

The effect is visible in plain use. A pattern made from a non-special URL does not match that URL, and matches another one (`repro.cpp`):

```cpp
auto p = ada::parse_url_pattern<Provider>(std::string_view("foo://h//x/y"));
p->get_pathname();                          // "/y"
p->test(std::string_view("foo://h//x/y"));  // false: the URL it was written from
p->test(std::string_view("foo://h/y"));     // true
// new URL("foo://h//x/y").pathname is "//x/y"
```

In Node.js (ada 4.0.0), `new URLPattern("foo://h//x/y")` has the pathname `/y` and the same two `test` results. Deno's `urlpattern` 0.6 gives `//x/y`, as the spec does. urlpattern-polyfill 10.1.0 also gives `/y`; it appears to take the same shortcut. Present in v4.0.0 and on main (4eee4c2).

Spec text (whatwg/urlpattern `spec.bs`, Review Draft September 2026):

> To canonicalize an opaque pathname given a string *value*: … Let *dummyURL* be the result of creating a dummy URL. Set *dummyURL*'s path to the empty string. Let *parseResult* be the result of running URL parsing given *value* with *dummyURL* as url and opaque path state as state override. …

The URL Standard's opaque path state stops at `?` or `#` and otherwise appends each code point. Nothing in it starts an authority.

**Fix** (`fix.patch`). Kept deliberately minimal: when the value starts with `//`, parse `"fake:/." + value` instead of `"fake:" + value`. The `/.` keeps the parser in the path (it is how a path starting with an empty segment is written, and the pathname getter omits it). New test `opaque_pathname_keeps_leading_slashes` checks `//x/y` and `//x`, and that the pattern from `foo://h//x/y` matches that URL and not `foo://h/y`. It fails without the fix. On main 4eee4c2 with the patch, all 404 tests pass.

**Scope of the change, measured.** Over 20,000 generated non-special pathname values (slashes, segments, spaces, C0 and DEL, non-ASCII, `%`), the fix changes exactly the 1,280 that start with `//`, and no others. On 30,000 generated values starting with `//`, ada with the fix agrees with Deno on all but those ending in a space or C0 control. There, ada trims the trailing characters (the whole-URL parse strips leading and trailing C0 controls and spaces) and Deno keeps them. That trimming predates this fix and is unchanged by it. Note also that the opaque path state keeps spaces, while the current shortcut, for values starting with `/`, encodes them as a hierarchical path does (`/ b` becomes `/%20b`). That matches real non-special URLs with hierarchical paths, and Deno, but not the spec's literal text. Both points seem worth a decision upstream; this fix does not change them.

<details>
<summary>repro.cpp</summary>

```cpp
// A URLPattern built from a non-special URL whose path starts with "//"
// loses that part of its path: it does not match the URL it was made from,
// and matches a different one.
#include <cstdio>
#include <string>
#include "ada.h"
int main() {
  using P = ada::url_pattern_regex::std_regex_provider;
  auto p = ada::parse_url_pattern<P>(std::string_view("foo://h//x/y"));
  printf("pathname component: %s\n", std::string(p->get_pathname()).c_str());
  printf("test(foo://h//x/y) = %d\n", int(*p->test(std::string_view("foo://h//x/y"))));
  printf("test(foo://h/y)    = %d\n", int(*p->test(std::string_view("foo://h/y"))));
}
```

</details>

<details>
<summary>Patch (fix.patch)</summary>

```diff
diff --git a/src/url_pattern_helpers.cpp b/src/url_pattern_helpers.cpp
index 8c11f05f..e3efee0e 100644
--- a/src/url_pattern_helpers.cpp
+++ b/src/url_pattern_helpers.cpp
@@ -541,8 +541,14 @@ tl::expected<std::string, errors> canonicalize_opaque_pathname(
   // Set dummyURL's path to the empty string.
   // Let parseResult be the result of running URL parsing given value with
   // dummyURL as url and opaque path state as state override.
+  // A value starting with "//" would make "fake:" + value start an authority,
+  // dropping it ("//x/y" became "/y"). The opaque path state appends those
+  // slashes like any other code point, so keep the parser in the path: "/."
+  // before "//" is how a path that starts with an empty segment is written,
+  // and the pathname getter leaves it out.
+  const std::string prefix = input.starts_with("//") ? "fake:/." : "fake:";
   if (auto url =
-          ada::parse<url_aggregator>("fake:" + std::string(input), nullptr)) {
+          ada::parse<url_aggregator>(prefix + std::string(input), nullptr)) {
     // Return the result of URL path serializing dummyURL.
     return std::string(url->get_pathname());
   }
diff --git a/tests/wpt_urlpattern_tests.cpp b/tests/wpt_urlpattern_tests.cpp
index a1d953f5..094304f4 100644
--- a/tests/wpt_urlpattern_tests.cpp
+++ b/tests/wpt_urlpattern_tests.cpp
@@ -1372,3 +1372,25 @@ TEST(wpt_urlpattern_tests, urlpattern_test_data) {
   }
   SUCCEED();
 }
+
+TEST(wpt_urlpattern_tests, opaque_pathname_keeps_leading_slashes) {
+  // canonicalize an opaque pathname runs the opaque path state, which
+  // appends a leading "//" like any other code points: it must not start an
+  // authority and drop the text that follows.
+  auto pathname = [](std::string_view value) {
+    ada::url_pattern_init init{};
+    init.protocol = "foo";
+    init.pathname = value;
+    auto p = ada::parse_url_pattern<regex_provider>(init);
+    EXPECT_TRUE(p) << value;
+    return p ? std::string(p->get_pathname()) : std::string("(error)");
+  };
+  EXPECT_EQ(pathname("//x/y"), "//x/y");
+  EXPECT_EQ(pathname("//x"), "//x");
+  // A pattern made from a non-special URL matches that URL, and only it.
+  auto p = ada::parse_url_pattern<regex_provider>("foo://h//x/y");
+  ASSERT_TRUE(p);
+  ASSERT_EQ(p->get_pathname(), "//x/y");
+  ASSERT_TRUE(*p->test(std::string_view("foo://h//x/y")));
+  ASSERT_FALSE(*p->test(std::string_view("foo://h/y")));
+}
```

</details>
