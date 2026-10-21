**Title:** `url_search_params` keeps ill-formed UTF-8 from percent-decoding instead of decoding it to U+FFFD

The application/x-www-form-urlencoded parser runs "UTF-8 decode without BOM" on the percent-decoded name and value (https://url.spec.whatwg.org/#concept-urlencoded-parser), so escapes of ill-formed UTF-8 become U+FFFD. ada keeps the raw bytes: from valid ASCII input it builds entries that are not valid UTF-8.

```cpp
ada::url_search_params p("%aa");
p.front().first;   // "\xAA"           (standard: "�", bytes EF BF BD)
p.to_string();     // "%AA="           (standard: "%EF%BF%BD=")

ada::url_search_params q("%aa=1&%ab=2");
q.get_all("\xEF\xBF\xBD").size();   // 0 (standard: 2; both names decode to U+FFFD)
```

| init | ada `to_string()` | whatwg-url 17.2.0 |
|---|---|---|
| `%aa` | `%AA=` | `%EF%BF%BD=` |
| `%C3` | `%C3=` | `%EF%BF%BD=` |
| `%C3%28` | `%C3%28=` | `%EF%BF%BD%28=` |
| `%ED%A0%80` | `%ED%A0%80=` | `%EF%BF%BD%EF%BF%BD%EF%BF%BD=` |
| `%F0%9F%98` | `%F0%9F%98=` | `%EF%BF%BD=` |

This is different from #865, which was about invalid UTF-8 given to ada (the caller's responsibility). Here the input is ASCII and ada produces the ill-formed strings. The comment in `url_search_params::sort()` ("percent_decode can produce arbitrary byte sequences") shows the bytes were noticed, but the decoding step is missing. Consumers of the C++ or C API get strings that are not UTF-8, and names that the standard treats as one key stay distinct.

**Fix** (`fix.patch`). `unicode::form_urlencoded_decode` runs the Encoding Standard's UTF-8 decoder with replacement (one U+FFFD per maximal ill-formed subpart) when an escape decoded to a non-ASCII byte. Input with no such escape takes the old path unchanged. New test `percent_decoded_ill_formed_utf8_becomes_replacement`, whose expected values are whatwg-url's (fails without the fix); on main 4eee4c2 with the patch, all 404 tests pass.

Found by comparing generated URLSearchParams cases (init string, then append/delete/set/sort/get/getAll/has) between ada and whatwg-url.
