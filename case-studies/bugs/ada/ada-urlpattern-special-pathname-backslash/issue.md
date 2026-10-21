**Title:** URLPattern: a literal backslash in a special-scheme pathname pattern is not canonicalized to "/" (spec's dummy URL is special)

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
