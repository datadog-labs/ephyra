# Posted: https://github.com/ada-url/ada/issues/1280

Filed by alpaylan, 2026-10-08. Text as posted:

**URLPattern test/exec/match resolve a relative string input against an empty URL when no base is given (should not match)**

In `url_pattern::test`, `exec` and `match`, when the input is a string and no base URL string is given, ada declares the base URL with `result<url_aggregator> base_url;`. A default-constructed `result` (tl::expected) holds a *value*: an empty, successfully-parsed `url_aggregator`. The relative input is then parsed against that empty base and can succeed, so a relative string input matches a pattern that it should not, because a relative URL with no base is not a valid URL.

```cpp
ada::url_pattern_init init;
init.pathname = "/users/:id";
auto p = ada::parse_url_pattern<Provider>(std::move(init));
auto m = p->exec(std::string_view("/users/1"));   // matches, groups {id: "1"}
// Per the standard, "/users/1" with no base is not a URL: exec must be null.
```

Through Node.js (which uses ada):

```js
new URLPattern({ pathname: "/users/:id" }).test("/users/1")   // true in Node; false in urlpattern-polyfill
new URLPattern({ pathname: "/admin/*" }).exec("/admin/x")      // matches in Node; null elsewhere
new URLPattern("*://*").test("not a url")                      // true in Node; false elsewhere
```

urlpattern-polyfill 10.1.0 returns no match. So does Deno's `urlpattern` crate 0.6, whose matcher takes a parsed URL, and `/users/1` does not parse without a base. (I tested the crate, not the Deno runtime.) With a base given, ada already behaves correctly; the bug is only the no-base string path, where the intended "base is null" is actually "base is an empty URL". In v4.0.0 and on main (4eee4c2).

**Cause.** The URL-string branch of each of `test`, `exec`/`match` starts with:

```cpp
result<url_aggregator> base_url;              // holds a value: an empty URL
if (base_url_string) {
  base_url = ada::parse<url_aggregator>(*base_url_string, nullptr);
  if (!base_url) return ...no match...;
}
auto url = ada::parse<url_aggregator>(input, base_url.has_value() ? &*base_url : nullptr);
```

`base_url.has_value()` is true even when no base string was given, so a non-null pointer to an empty URL is passed and the relative input resolves against it.

**Fix** (`fix.patch`). Initialize the base to an error so "no base given" means no value:

```cpp
result<url_aggregator> base_url = tl::unexpected(errors::type_error);
```

in both the `test` branch and the `exec`/`match` branch. Added test `string_input_without_base_must_parse_alone` in `tests/wpt_urlpattern_tests.cpp`; it fails without the fix, and on main 4eee4c2 with the patch all 404 tests pass.

**How it was found.** A WPT-shaped property (a relative string input with no base must not match) and the differential against urlpattern-polyfill and Deno.

<details>
<summary>Patch (fix.patch)</summary>

```diff
diff --git a/include/ada/url_pattern-inl.h b/include/ada/url_pattern-inl.h
index 8ba8124c..75924a67 100644
--- a/include/ada/url_pattern-inl.h
+++ b/include/ada/url_pattern-inl.h
@@ -318,7 +318,9 @@ result<bool> url_pattern<regex_provider>::test(
   }
 
   // URL string input path
-  result<url_aggregator> base_url;
+  // Let baseURL be null. (A default-constructed result holds a value: an
+  // empty URL that relative inputs would resolve against.)
+  result<url_aggregator> base_url = tl::unexpected(errors::type_error);
   if (base_url_string) {
     base_url = ada::parse<url_aggregator>(*base_url_string, nullptr);
     if (!base_url) {
@@ -434,8 +436,9 @@ result<std::optional<url_pattern_result>> url_pattern<regex_provider>::match(
   } else {
     ADA_ASSERT_TRUE(std::holds_alternative<std::string_view>(input));
 
-    // Let baseURL be null.
-    result<url_aggregator> base_url;
+    // Let baseURL be null. (A default-constructed result holds a value: an
+    // empty URL that relative inputs would resolve against.)
+    result<url_aggregator> base_url = tl::unexpected(errors::type_error);
 
     // If baseURLString was given, then:
     if (base_url_string) {
diff --git a/tests/wpt_urlpattern_tests.cpp b/tests/wpt_urlpattern_tests.cpp
index a1d953f5..313e7e4e 100644
--- a/tests/wpt_urlpattern_tests.cpp
+++ b/tests/wpt_urlpattern_tests.cpp
@@ -1372,3 +1372,31 @@ TEST(wpt_urlpattern_tests, urlpattern_test_data) {
   }
   SUCCEED();
 }
+
+TEST(wpt_urlpattern_tests, string_input_without_base_must_parse_alone) {
+  // A string input is parsed with the base URL string only when one is
+  // given; a relative string with no base is not a URL, so test() is false
+  // and exec() is null, whatever the pattern.
+  auto check = [](std::variant<std::string_view, ada::url_pattern_init> pattern,
+                  std::string_view input) {
+    auto p = ada::parse_url_pattern<regex_provider>(std::move(pattern));
+    ASSERT_TRUE(p);
+    auto t = p->test(input);
+    ASSERT_TRUE(t);
+    ASSERT_FALSE(*t) << input;
+    auto e = p->exec(input);
+    ASSERT_TRUE(e);
+    ASSERT_FALSE(e->has_value()) << input;
+  };
+  ada::url_pattern_init users{};
+  users.pathname = "/users/:id";
+  check(users, "/users/1");
+  ada::url_pattern_init admin{};
+  admin.pathname = "/admin/*";
+  check(admin, "/admin/x");
+  check(std::string_view("*://*"), "not a url");
+  // With a base, the same relative input matches.
+  auto p = ada::parse_url_pattern<regex_provider>(users);
+  std::string_view base = "https://example.com";
+  ASSERT_TRUE(*p->test("/users/1", &base));
+}
```

</details>
