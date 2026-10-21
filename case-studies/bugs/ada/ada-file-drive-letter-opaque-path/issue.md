**Title:** `parse("C|", file_base)` marks the URL as having an opaque path: host/path setters fail and it cannot be a base

Resolving a reference that starts with a Windows drive letter against a `file:` base gives a URL whose href is right (`file:///C:`) but whose `has_opaque_path` is `true`. Its path is the list `["C:"]`, not an opaque path. Because of the flag:

- `set_hostname`, `set_host` and `set_pathname` return `false` and change nothing;
- `ada::parse("x", &url)` fails, where the URL Standard gives `file:///C:/x`.

The same href parsed afresh (`ada::parse("file:///C:")`) behaves correctly. Both `ada::url` and `ada::url_aggregator` are affected, as is the C API (`ada_parse_with_base("C|", 2, "file:", 5)` then `ada_set_hostname` returns `false`). Found on ada main at 4eee4c2 (2026-10-05) and present in v4.0.0. Node.js does not show the bug, apparently because it re-parses the href before each setter.

```cpp
auto file = ada::parse<ada::url_aggregator>("file:");
auto url = ada::parse<ada::url_aggregator>("C|", &*file);   // file:///C:
url->has_opaque_path;                                         // true, should be false
url->set_hostname("h");                                       // false, should be true -> file://h/C:
url->set_pathname("/D:/y");                                   // false, should be true -> file:///D:/y
ada::parse<ada::url_aggregator>("x", &*url);                  // failure, should be file:///C:/x
```

Output of the attached `repro.cpp` on main (the second line of each pair parses the same href directly):

```
url        new URL("C|", "file:")   has_opaque_path 1 | set_hostname("h") 0 -> file:///C:   | set_pathname("/D:/y") 0 -> file:///C:    | new URL("x", it) failure
url        new URL("file:///C:")    has_opaque_path 0 | set_hostname("h") 1 -> file://h/C:  | set_pathname("/D:/y") 1 -> file:///D:/y  | new URL("x", it) file:///C:/x
aggregator new URL("C|", "file:")   has_opaque_path 1 | set_hostname("h") 0 -> file:///C:   | set_pathname("/D:/y") 0 -> file:///C:    | new URL("x", it) failure
aggregator new URL("file:///C:")    has_opaque_path 0 | set_hostname("h") 1 -> file://h/C:  | set_pathname("/D:/y") 1 -> file:///D:/y  | new URL("x", it) file:///C:/x
```

whatwg-url 17.2.0 (the reference implementation) gives `file://h/C:`, `file:///D:/y` and `file:///C:/x`.

**Cause.** In the file state, when the base is a `file:` URL and the rest of the input starts with a Windows drive letter, the standard says "set url's path to « »", an empty list. `src/parser.cpp` does that and also sets `url.has_opaque_path = true`:

```cpp
            // Otherwise:
            else {
              // Set url's path to an empty list.
              url.clear_pathname();
              url.has_opaque_path = true;
            }
```

The path state then appends `C:` as an ordinary segment, so the URL serializes as a hierarchical URL while carrying the opaque flag, which the setters and the relative-reference states check.

**Fix.** Drop the assignment (the flag was already copied from the base, and a `file:` URL never has an opaque path). The attached patch does that and adds a typed test, `file_relative_windows_drive_letter_path_is_not_opaque`, which fails on both types without the fix. On main 4eee4c2 with the patch, all 405 tests pass.

Any relative reference starting with a letter and `|` against a `file:` base triggers it: `C|`, `C|/x` and `c|\x` with the bases `file:`, `file:///` and `file:///z` were checked. (`C:` and `c:\x` do not reach this branch: they parse as URLs with the schemes `c:`.)

How it was found: generated cases (input, optional base, setter calls) compared across `ada::url`, `ada::url_aggregator` and whatwg-url, plus a metamorphic check that a URL behaves like the URL its href parses to. Both checks found this independently and shrank it to `new URL("a|", "file:")` followed by `hostname = ""`.
