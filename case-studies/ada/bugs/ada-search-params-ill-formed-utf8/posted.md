# Posted: https://github.com/ada-url/ada/issues/1279

Filed by alpaylan, 2026-10-08. Text as posted:

**`url_search_params` keeps ill-formed UTF-8 from percent-decoding instead of decoding it to U+FFFD**

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

<details>
<summary>Patch (fix.patch)</summary>

```diff
diff --git a/include/ada/unicode.h b/include/ada/unicode.h
index df2e1138..8ff1a318 100644
--- a/include/ada/unicode.h
+++ b/include/ada/unicode.h
@@ -199,7 +199,9 @@ std::string percent_decode(std::string_view input, size_t first_percent);
 
 /**
  * Decode an application/x-www-form-urlencoded component: map '+' to space,
- * then percent-decode. Single allocation; no intermediate string.
+ * percent-decode, then UTF-8 decode with replacement as the standard does,
+ * so that escapes of ill-formed UTF-8 (`%FF`) become U+FFFD and the result
+ * is valid UTF-8.
  *
  * @param input A form-urlencoded component (key or value).
  * @return The decoded string.
diff --git a/src/unicode.cpp b/src/unicode.cpp
index 754dc644..76035c2a 100644
--- a/src/unicode.cpp
+++ b/src/unicode.cpp
@@ -530,6 +530,62 @@ constexpr static std::array<uint8_t, 256> unhex_table = []() consteval {
   return t;
 }();
 
+namespace {
+// The Encoding Standard's UTF-8 decoder with replacement, re-encoded as
+// UTF-8: well-formed sequences are kept, and each maximal ill-formed
+// subsequence becomes U+FFFD.
+// https://encoding.spec.whatwg.org/#utf-8-decoder
+std::string utf8_decode_without_bom(std::string_view input) {
+  constexpr std::string_view replacement = "\xEF\xBF\xBD";
+  std::string out;
+  out.reserve(input.size());
+  size_t i = 0;
+  while (i < input.size()) {
+    const uint8_t b = static_cast<uint8_t>(input[i]);
+    size_t needed = 0;
+    uint8_t lower = 0x80, upper = 0xBF;
+    if (b <= 0x7F) {
+      out.push_back(static_cast<char>(b));
+      i++;
+      continue;
+    } else if (b >= 0xC2 && b <= 0xDF) {
+      needed = 1;
+    } else if (b >= 0xE0 && b <= 0xEF) {
+      needed = 2;
+      if (b == 0xE0) lower = 0xA0;
+      if (b == 0xED) upper = 0x9F;
+    } else if (b >= 0xF0 && b <= 0xF4) {
+      needed = 3;
+      if (b == 0xF0) lower = 0x90;
+      if (b == 0xF4) upper = 0x8F;
+    } else {
+      out.append(replacement);
+      i++;
+      continue;
+    }
+    size_t seen = 0;
+    while (seen < needed && i + 1 + seen < input.size()) {
+      const uint8_t c = static_cast<uint8_t>(input[i + 1 + seen]);
+      if (c < lower || c > upper) {
+        break;
+      }
+      lower = 0x80;
+      upper = 0xBF;
+      seen++;
+    }
+    if (seen == needed) {
+      out.append(input.substr(i, needed + 1));
+    } else {
+      // The lead byte and the continuation bytes that fit form the maximal
+      // subpart; the byte that broke it is read again.
+      out.append(replacement);
+    }
+    i += seen + 1;
+  }
+  return out;
+}
+}  // namespace
+
 std::string form_urlencoded_decode(const std::string_view input) {
   const size_t len = input.size();
   if (len == 0) [[unlikely]] {
@@ -554,6 +610,9 @@ std::string form_urlencoded_decode(const std::string_view input) {
   std::string out(len, '\0');
   char* d = out.data();
   char* const d0 = d;
+  // Whether an escape decoded to a non-ASCII byte, which may leave the
+  // output ill-formed UTF-8 (the input is valid UTF-8).
+  bool decoded_non_ascii = false;
 
   const size_t prefix = static_cast<size_t>(p - src);
   std::memcpy(d, src, prefix);
@@ -573,6 +632,7 @@ std::string form_urlencoded_decode(const std::string_view input) {
           break;
         }
         *d++ = static_cast<char>((hi << 4) | lo);
+        decoded_non_ascii |= hi >= 8;
         p += 3;
       }
       if (p < end && *p == '%') {
@@ -593,6 +653,10 @@ std::string form_urlencoded_decode(const std::string_view input) {
   }
 
   out.resize(static_cast<size_t>(d - d0));
+  // The parser UTF-8 decodes the percent-decoded bytes, with replacement.
+  if (decoded_non_ascii) {
+    return utf8_decode_without_bom(out);
+  }
   return out;
 }
 
diff --git a/tests/url_search_params.cpp b/tests/url_search_params.cpp
index bf2331bf..1cadf042 100644
--- a/tests/url_search_params.cpp
+++ b/tests/url_search_params.cpp
@@ -581,3 +581,27 @@ TEST(url_search_params, sort_surrogate_lhs_exhausted_pending) {
   ASSERT_EQ(keys.next(), "A\360\237\214\210B");
   SUCCEED();
 }
+
+TEST(url_search_params, percent_decoded_ill_formed_utf8_becomes_replacement) {
+  // The application/x-www-form-urlencoded parser UTF-8 decodes the
+  // percent-decoded name and value without BOM: each maximal ill-formed
+  // subsequence becomes U+FFFD, and the entries are valid UTF-8.
+  auto check = [](std::string_view init, std::string_view key,
+                  std::string_view serialized) {
+    ada::url_search_params params(init);
+    ASSERT_EQ(params.size(), 1);
+    ASSERT_EQ(params.front().first, key);
+    ASSERT_EQ(params.to_string(), serialized);
+  };
+  check("%aa", "\xEF\xBF\xBD", "%EF%BF%BD=");
+  check("%C3", "\xEF\xBF\xBD", "%EF%BF%BD=");
+  check("%C3%28", "\xEF\xBF\xBD(", "%EF%BF%BD%28=");
+  check("%E2%82", "\xEF\xBF\xBD", "%EF%BF%BD=");
+  check("%ED%A0%80", "\xEF\xBF\xBD\xEF\xBF\xBD\xEF\xBF\xBD",
+        "%EF%BF%BD%EF%BF%BD%EF%BF%BD=");
+  check("%F0%9F%98", "\xEF\xBF\xBD", "%EF%BF%BD=");
+  check("%C3%A9", "\xC3\xA9", "%C3%A9=");
+  // Distinct ill-formed escapes are the same key once decoded.
+  ada::url_search_params params("%aa=1&%ab=2");
+  ASSERT_EQ(params.get_all("\xEF\xBF\xBD").size(), 2);
+}
```

</details>
