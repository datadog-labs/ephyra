# Posted: https://github.com/ada-url/ada/issues/1281

Filed by alpaylan, 2026-10-08. Text as posted:

**URLPattern: a literal backslash in a special-scheme pathname pattern is not canonicalized to "/" (spec's dummy URL is special)**

For a pattern whose protocol matches a special scheme, the pathname is canonicalized with "canonicalize a pathname". The spec runs the URL path parser on a dummy URL from "create a dummy URL", which is `https://dummy.invalid/`, a special URL. So a literal backslash in the pathname becomes `/`, as it does in any special URL path. ada's `url_pattern_helpers::canonicalize_pathname` uses the non-special dummy `fake://fake-url` instead. The backslash stays, and the pattern never matches the URLs it is written for.

Spec text (whatwg/urlpattern `spec.bs`, Review Draft September 2026):

> To canonicalize a pathname given a string *value*: … Let *dummyURL* be the result of creating a dummy URL. Empty *dummyURL*'s path. Run basic URL parser given *modified value* with *dummyURL* as url and path start state as state override. …
>
> To create a dummy URL: Let *dummyInput* be "`https://dummy.invalid/`". Return the result of running the basic URL parser on *dummyInput*.

**Example.** A literal backslash in a pattern is written as the escape `\\` (a lone `\` escapes the next character). So the pathname value below holds the two characters `\\`:

```cpp
ada::url_pattern_init init;
init.protocol = "https";
init.hostname = "h";
init.pathname = "/a\\\\b";   // the string /a\\b: a literal backslash between a and b
auto p = ada::parse_url_pattern<Provider>(std::move(init));
p->get_pathname();            // ada: "/a\\b" (literal backslash kept); spec: "/a/b"
p->test("https://h/a/b");     // ada: false; spec: true
```

In Node.js (ada 4.0.0): `new URLPattern({protocol: "https", hostname: "h", pathname: "/a\\\\b"})` has the pathname `/a\\b`, and `.test("https://h/a/b")` is `false`. The URL `https://h/a\b` itself parses to the path `/a/b`, so the pattern can never match anything.

Measured with controlled bytes; the column shows the pathname component for the init value `/a\\b`:

| protocol | ada | urlpattern-polyfill 10.1.0 | Deno `urlpattern` 0.6 |
|---|---|---|---|
| https | `/a\\b` | `/a/b` | `/a/b` |
| ftp   | `/a\\b` | `/a/b` | `/a/b` |
| foo (non-special) | `/a\\b` | `/a\\b` | `/a/b` |

`/a/../b` gives `/b` everywhere, and a lone `\` (an escape, not a backslash) is handled the same everywhere. The non-special row goes through "canonicalize an opaque pathname", which this report does not touch. Present in v4.0.0 and on main (4eee4c2).

**Fix** (`fix.patch`). Use the spec's dummy URL in `canonicalize_pathname`:

```cpp
-  auto url = ada::parse<url_aggregator>("fake://fake-url", nullptr);
+  auto url = ada::parse<url_aggregator>("https://dummy.invalid/", nullptr);
```

New test `special_scheme_pathname_converts_backslash` checks that `/a\\b` becomes `/a/b` for https and ftp, and that `/a/../b` becomes `/b`. It fails without the fix. On main 4eee4c2 with the patch, all 404 tests pass.

**Validation.** With the fix, `path_diff.rs` generated 40,000 special-scheme pathname patterns from segments, `.`/`..` and percent escapes, joined by `/`, `\`, `//`, `/\` and `\\`. It found no case where ada's pathname component differs from both references while the references agree.

**How it was found.** Differential testing of URLPattern against the two references. Shell and source-code escaping make this one easy to misread, so every value above was produced from controlled bytes.

`path_diff.rs` is a generator in my test harness, not a standalone program; I can share it if useful.

<details>
<summary>Patch (fix.patch)</summary>

```diff
diff --git a/src/url_pattern_helpers.cpp b/src/url_pattern_helpers.cpp
index 8c11f05f..d5cc7b64 100644
--- a/src/url_pattern_helpers.cpp
+++ b/src/url_pattern_helpers.cpp
@@ -511,7 +511,10 @@ tl::expected<std::string, errors> canonicalize_pathname(
   // query/fragment, so the pathname was silently truncated at the first literal
   // '?' or '#'. set_pathname runs the parser in path state override, matching
   // how canonicalize_hostname uses set_hostname.
-  auto url = ada::parse<url_aggregator>("fake://fake-url", nullptr);
+  // The spec's "create a dummy URL" is "https://dummy.invalid/": a special
+  // URL, so the path parser turns "\\" into "/" as it does in a special URL.
+  // A non-special dummy kept a literal backslash.
+  auto url = ada::parse<url_aggregator>("https://dummy.invalid/", nullptr);
   ADA_ASSERT_TRUE(url);
   if (!url->set_pathname(modified_value)) {
     // If parseResult is failure, then throw a TypeError.
diff --git a/tests/wpt_urlpattern_tests.cpp b/tests/wpt_urlpattern_tests.cpp
index a1d953f5..d8b15b30 100644
--- a/tests/wpt_urlpattern_tests.cpp
+++ b/tests/wpt_urlpattern_tests.cpp
@@ -1372,3 +1372,24 @@ TEST(wpt_urlpattern_tests, urlpattern_test_data) {
   }
   SUCCEED();
 }
+
+TEST(wpt_urlpattern_tests, special_scheme_pathname_converts_backslash) {
+  // canonicalize_pathname (used for a special protocol) must canonicalize
+  // the pathname as a special-scheme URL path: a literal backslash becomes
+  // "/". A non-special dummy scheme kept it literal. The pattern text of
+  // "/a\\b" is "/a\b" (one literal backslash after escape processing).
+  auto compile = [](std::string_view protocol, std::string_view pathname) {
+    ada::url_pattern_init init{};
+    init.protocol = protocol;
+    init.hostname = "h";
+    init.pathname = pathname;
+    auto p = ada::parse_url_pattern<regex_provider>(init);
+    EXPECT_TRUE(p) << pathname;
+    return p ? std::string(p->get_pathname()) : std::string("(error)");
+  };
+  // Two source backslashes -> one literal backslash -> "/" for a special scheme.
+  EXPECT_EQ(compile("https", "/a\\\\b"), "/a/b");
+  EXPECT_EQ(compile("ftp", "/a\\\\b"), "/a/b");
+  // Dot segments are normalized as a special path.
+  EXPECT_EQ(compile("https", "/a/../b"), "/b");
+}
```

</details>
