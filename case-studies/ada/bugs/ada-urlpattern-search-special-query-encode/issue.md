**Title:** URLPattern search canonicalization uses the non-special query encode set: a search pattern containing `'` never matches a special URL

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
