**Title:** URLPattern opaque pathname: a leading `//` is parsed as an authority and dropped (`//x/y` becomes `/y`)

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
