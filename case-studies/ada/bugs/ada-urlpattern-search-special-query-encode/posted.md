# Posted: https://github.com/ada-url/ada/issues/1285

Filed by alpaylan, 2026-10-08. Text as posted:

**URLPattern search canonicalization uses the non-special query encode set: a search pattern containing `'` never matches a special URL**

"canonicalize a search" runs the query state on the spec's dummy URL, which "create a dummy URL" makes from `https://dummy.invalid/`, a special URL. A special URL's query is percent-encoded with the special-query percent-encode set, which (unlike the plain query set) includes U+0027 (`'`). `url_pattern_helpers::canonicalize_search` uses `character_sets::QUERY_PERCENT_ENCODE`; its comment reads "Note: "fake://dummy.test" is not a special URL, so we use QUERY_PERCENT_ENCODE". So `'` stays literal in the pattern, while the URLs it is meant to match have `%27`:

```cpp
ada::url_pattern_init init;
init.search = "q='x'";
auto p = ada::parse_url_pattern<Provider>(std::move(init));
p->get_search();                           // ada: "q='x'"   spec: "q=%27x%27"
p->test(std::string_view("https://h/?q='x'"));   // ada: false   spec: true
// new URL("https://h/?q='x'").search is "?q=%27x%27"
```

In Node.js (ada 4.0.0): `new URLPattern({search: "q='x'"}).test("https://h/?q='x'")` is `false`. urlpattern-polyfill 10.1.0 and Deno's `urlpattern` 0.6 both give the search component `q=%27x%27` and `true`. The same set is used when processing an init input, so `pattern.exec({search: "?'"})` reports the search input `'` in ada and `%27` in both references. Present in v4.0.0 and on main (4eee4c2).

Spec text (whatwg/urlpattern `spec.bs`, Review Draft September 2026):

> To canonicalize a search given a string *value*: … Let *dummyURL* be the result of creating a dummy URL. Set *dummyURL*'s query to the empty string. Run basic URL parser given *value* with *dummyURL* as url and query state as state override. Return *dummyURL*'s query.

(The dummy URL is special whatever the pattern's protocol, so with the fix a search pattern with `'` matches special URLs but not non-special ones, whose queries keep `'`. Deno behaves the same. That follows the spec as written.)

**Fix** (`fix.patch`). Use `character_sets::SPECIAL_QUERY_PERCENT_ENCODE` in `canonicalize_search`. New test `search_uses_special_query_percent_encode_set` checks the pattern component (`q=%27x%27`), the match against `https://h/?q='x'`, and the init-input case (`%27`). It fails without the fix. On main 4eee4c2 with the patch, all 404 tests pass.

**How it was found.** `exec` on the all-wildcard pattern `{}` reports each component of a URLPatternInit input after processing. That is canonicalization only, independent of the regex engine, so ada could be compared with urlpattern-polyfill and Deno directly. Of about 30,000 generated init inputs on which the two references agree, ada differed on 66 in the search component. With the fix, 65 of them agree; the remaining one is a separate edge case (a search inherited from a `baseURL` whose query itself starts with `?`). The same root cause, a non-special dummy URL where the spec's is special, underlies the separately reported pathname-backslash bug.

Related: #1281 (the pathname-backslash bug mentioned above).

<details>
<summary>Patch (fix.patch)</summary>

```diff
diff --git a/src/url_pattern_helpers.cpp b/src/url_pattern_helpers.cpp
index 8c11f05f..0fad8271 100644
--- a/src/url_pattern_helpers.cpp
+++ b/src/url_pattern_helpers.cpp
@@ -566,18 +566,18 @@ tl::expected<std::string, errors> canonicalize_search(std::string_view input) {
     return "";
   }
 
-  // Percent-encode using QUERY_PERCENT_ENCODE (for non-special URLs)
-  // Note: "fake://dummy.test" is not a special URL, so we use
-  // QUERY_PERCENT_ENCODE
+  // The spec's dummy URL ("create a dummy URL") is https://dummy.invalid/,
+  // a special URL, so its query state uses the special-query percent-encode
+  // set, which also encodes U+0027 (').
   size_t idx = ada::unicode::percent_encode_index(
-      new_value, character_sets::QUERY_PERCENT_ENCODE);
+      new_value, character_sets::SPECIAL_QUERY_PERCENT_ENCODE);
   if (idx == new_value.size()) {
     // No encoding needed
     return new_value;
   }
   // Percent-encode from the first character that needs encoding
   return ada::unicode::percent_encode(
-      new_value, character_sets::QUERY_PERCENT_ENCODE, idx);
+      new_value, character_sets::SPECIAL_QUERY_PERCENT_ENCODE, idx);
 }
 
 tl::expected<std::string, errors> canonicalize_hash(std::string_view input) {
diff --git a/tests/wpt_urlpattern_tests.cpp b/tests/wpt_urlpattern_tests.cpp
index a1d953f5..986f0179 100644
--- a/tests/wpt_urlpattern_tests.cpp
+++ b/tests/wpt_urlpattern_tests.cpp
@@ -1372,3 +1372,25 @@ TEST(wpt_urlpattern_tests, urlpattern_test_data) {
   }
   SUCCEED();
 }
+
+TEST(wpt_urlpattern_tests, search_uses_special_query_percent_encode_set) {
+  // canonicalize a search runs the query state on the spec's dummy URL,
+  // https://dummy.invalid/, which is special: U+0027 (') is percent-encoded,
+  // as in the query of any special URL. Otherwise a search pattern with a
+  // quote never matches a special URL's query.
+  ada::url_pattern_init init{};
+  init.search = "q='x'";
+  auto p = ada::parse_url_pattern<regex_provider>(init);
+  ASSERT_TRUE(p);
+  ASSERT_EQ(p->get_search(), "q=%27x%27");
+  auto t = p->test(std::string_view("https://h/?q='x'"));
+  ASSERT_TRUE(t);
+  ASSERT_TRUE(*t);
+  // Processing an init input canonicalizes its search the same way.
+  ada::url_pattern_init input{};
+  input.search = "?'";
+  auto all = ada::parse_url_pattern<regex_provider>(ada::url_pattern_init{});
+  auto m = all->exec(ada::url_pattern_input(input), nullptr);
+  ASSERT_TRUE(m && *m);
+  ASSERT_EQ((**m).search.input, "%27");
+}
```

</details>
