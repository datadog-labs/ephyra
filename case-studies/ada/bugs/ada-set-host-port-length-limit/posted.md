# Posted: https://github.com/ada-url/ada/issues/1282

Filed by alpaylan, 2026-10-08. Text as posted:

**`set_host("host:port")` under `set_max_input_length` applies the host, drops the port, and reports success**

When the result of `set_host("host:port")` does not fit `ada::get_max_input_length()`, the setter should fail and leave the URL unchanged, as the other setters do (`url_unchanged_after_rejected_set` tests that). Instead the host is changed, the port is left as it was (or absent), and the setter returns `true`:

```cpp
auto url = ada::parse<ada::url_aggregator>("https://bank.example:1/");
ada::set_max_input_length(25);
url->set_host("evil.example:9443");   // true
url->get_href();                      // "https://evil.example:1/": the new host with the old port
```

```cpp
auto url = ada::parse<ada::url_aggregator>("http://example.com/");
ada::set_max_input_length(20);
url->set_host("example.org:8080");    // true
url->get_href();                      // "http://example.org/": port 8080 silently dropped
```

The result is a host/port pair the caller never asked for, reported as success. Both `ada::url` and `ada::url_aggregator` behave this way, on main (4eee4c2) and in v4.0.0. `repro.cpp` prints both cases for both types.

**Cause.** `set_host_or_hostname` parses the host, then calls `set_port(port_buffer)` and ignores its result before `check_url_size()`:

```cpp
      if (!port_buffer.empty()) {
        set_port(port_buffer);
      }
      return check_url_size();
```

`set_port` enforces the limit itself: the port does not fit, so it rolls back its own change and returns `false`. `check_url_size()` then sees a buffer that fits (without the port) and returns `true`, so the full snapshot `saved_url` is never restored.

**Fix** (`fix.patch`). `set_port` is split into `apply_port`, which does everything but the limit check (and changes nothing when it fails), and `set_port`, which adds the check. The host setter calls `apply_port`, so `check_url_size()` checks the whole result and rolls back the host with the port. A port the standard rejects (`"host:8a"`) behaves as before. New typed test `set_host_port_that_does_not_fit_rolls_back` (fails on both types without the fix); on main 4eee4c2 with the patch, all 405 tests pass.

Found by a property that runs generated cases (input, base, setter calls) under limits near the lengths the case reaches, and checks that each setter either does what it does without a limit or fails and changes nothing.

<details>
<summary>repro.cpp</summary>

```cpp
#include <cstdio>
#include <string>
#include "ada.h"
template <class T> void check(const char* name, const char* input, const char* host, uint32_t limit) {
  auto u = ada::parse<T>(input);
  ada::set_max_input_length(limit);
  bool ok = u->set_host(host);
  ada::set_max_input_length(UINT32_MAX);
  printf("%-10s limit %2u: %s .host = \"%s\" -> %d, href %s\n", name, limit, input, host, ok,
         std::string(u->get_href()).c_str());
}
int main() {
  check<ada::url>("url", "https://bank.example:1/", "evil.example:9443", 25);
  check<ada::url_aggregator>("aggregator", "https://bank.example:1/", "evil.example:9443", 25);
  check<ada::url>("url", "http://example.com/", "example.org:8080", 20);
  check<ada::url_aggregator>("aggregator", "http://example.com/", "example.org:8080", 20);
}
```

</details>

<details>
<summary>Patch (fix.patch)</summary>

```diff
diff --git a/include/ada/url.h b/include/ada/url.h
index 4e79e9ca..e55c8602 100644
--- a/include/ada/url.h
+++ b/include/ada/url.h
@@ -453,6 +453,15 @@ struct url : url_base {
    */
   [[nodiscard]] inline bool cannot_have_credentials_or_port() const;
 
+  /**
+   * The port setter without the length limit: parses `input` as the port
+   * state does with a state override and sets the port, or returns false and
+   * changes nothing. set_port adds the limit; the host setter, which checks
+   * the limit for the whole value, uses this so that a port that does not
+   * fit rolls back the host with it.
+   */
+  bool apply_port(std::string_view input);
+
   ada_really_inline size_t parse_port(
       std::string_view view, bool check_trailing_content) noexcept override;
 
diff --git a/include/ada/url_aggregator.h b/include/ada/url_aggregator.h
index ff069fff..67c0ada7 100644
--- a/include/ada/url_aggregator.h
+++ b/include/ada/url_aggregator.h
@@ -365,6 +365,15 @@ struct url_aggregator : url_base {
    */
   constexpr void reserve(uint32_t capacity);
 
+  /**
+   * The port setter without the length limit: parses `input` as the port
+   * state does with a state override and sets the port, or returns false and
+   * changes nothing. set_port adds the limit; the host setter, which checks
+   * the limit for the whole value, uses this so that a port that does not
+   * fit rolls back the host with it.
+   */
+  bool apply_port(std::string_view input);
+
   ada_really_inline size_t parse_port(std::string_view view,
                                       bool check_trailing_content) override;
 
diff --git a/src/url.cpp b/src/url.cpp
index 0ea943ea..787d6ad7 100644
--- a/src/url.cpp
+++ b/src/url.cpp
@@ -671,7 +671,9 @@ bool url::set_host_or_hostname(const std::string_view input) {
       // state.
       std::string_view port_buffer = new_host.substr(location + 1);
       if (!port_buffer.empty()) {
-        set_port(port_buffer);
+        // The limit is checked for the whole value below: a port that does
+        // not fit must roll back the host too.
+        apply_port(port_buffer);
       }
       return check_url_size();
     }
@@ -768,6 +770,18 @@ bool url::set_password(const std::string_view input) {
 }
 
 bool url::set_port(const std::string_view input) {
+  std::optional<uint16_t> previous_port = port;
+  if (!apply_port(input)) {
+    return false;
+  }
+  if (get_href_size() > ada::get_max_input_length()) {
+    port = std::move(previous_port);
+    return false;
+  }
+  return true;
+}
+
+bool url::apply_port(const std::string_view input) {
   if (cannot_have_credentials_or_port()) {
     return false;
   }
@@ -799,10 +813,6 @@ bool url::set_port(const std::string_view input) {
   std::optional<uint16_t> previous_port = port;
   parse_port(digits_to_parse);
   if (is_valid) {
-    if (get_href_size() > ada::get_max_input_length()) {
-      port = std::move(previous_port);
-      return false;
-    }
     return true;
   }
   port = std::move(previous_port);
diff --git a/src/url_aggregator.cpp b/src/url_aggregator.cpp
index 348777e3..b3c2814f 100644
--- a/src/url_aggregator.cpp
+++ b/src/url_aggregator.cpp
@@ -313,6 +313,26 @@ bool url_aggregator::set_port(const std::string_view input) {
   ada_log("url_aggregator::set_port ", input);
   ADA_ASSERT_TRUE(validate());
   ADA_ASSERT_TRUE(!helpers::overlaps(input, buffer));
+  // A valid port can only grow the buffer by a few bytes, and apply_port
+  // changes nothing when it fails.
+  std::optional<url_aggregator> saved_url;
+  if (needs_rollback_snapshot(input.size())) {
+    saved_url = *this;
+  }
+  if (!apply_port(input)) {
+    return false;
+  }
+  if (saved_url && buffer.size() > ada::get_max_input_length()) {
+    *this = std::move(*saved_url);
+    return false;
+  }
+  return true;
+}
+
+bool url_aggregator::apply_port(const std::string_view input) {
+  ada_log("url_aggregator::apply_port ", input);
+  ADA_ASSERT_TRUE(validate());
+  ADA_ASSERT_TRUE(!helpers::overlaps(input, buffer));
   if (cannot_have_credentials_or_port()) {
     return false;
   }
@@ -341,22 +361,13 @@ bool url_aggregator::set_port(const std::string_view input) {
       std::string_view(trimmed.data(), first_non_digit - trimmed.begin());
 
   // parse_port only touches the buffer once the digits are already known to
-  // be in range, so an invalid port never mutates anything and needs no
-  // rollback; a valid one can only grow the buffer by a few bytes.
-  std::optional<url_aggregator> saved_url;
-  if (needs_rollback_snapshot(digits_to_parse.size())) {
-    saved_url = *this;
-  }
+  // be in range, so an invalid port never mutates anything.
   parse_port(digits_to_parse);
   if (!is_valid) {
     is_valid = true;
     ADA_ASSERT_TRUE(validate());
     return false;
   }
-  if (saved_url && buffer.size() > ada::get_max_input_length()) {
-    *this = std::move(*saved_url);
-    return false;
-  }
   return true;
 }
 
@@ -710,7 +721,9 @@ bool url_aggregator::set_host_or_hostname(const std::string_view input) {
       // state.
       std::string_view port_buffer = new_host.substr(location + 1);
       if (!port_buffer.empty()) {
-        set_port(port_buffer);
+        // The limit is checked for the whole value below: a port that does
+        // not fit must roll back the host too.
+        apply_port(port_buffer);
       }
       return check_url_size();
     }
diff --git a/tests/max_input_length.cpp b/tests/max_input_length.cpp
index d1005aa7..2d5c6490 100644
--- a/tests/max_input_length.cpp
+++ b/tests/max_input_length.cpp
@@ -349,3 +349,21 @@ TEST(max_input_length_helpers, url_search_params_accepts_under_limit) {
   ASSERT_EQ(params.get("foo"), "bar");
   ada::set_max_input_length(std::numeric_limits<uint32_t>::max());
 }
+
+TYPED_TEST(max_input_length_tests, set_host_port_that_does_not_fit_rolls_back) {
+  // set_host("host:port") sets the host, then the port. When the result
+  // does not fit the limit, the whole setter fails and the URL is unchanged:
+  // the new host must not stay with the old port, or with none.
+  auto check = [](std::string_view input, std::string_view host,
+                  uint32_t limit) {
+    auto url = ada::parse<TypeParam>(input);
+    ASSERT_TRUE(url);
+    ada::set_max_input_length(limit);
+    ASSERT_FALSE(url->set_host(host));
+    ASSERT_EQ(url->get_href(), input);
+    ada::set_max_input_length(small_limit);
+  };
+  check("https://bank.example:1/", "evil.example:9443", 25);
+  check("http://example.com/", "example.org:8080", 20);
+  check("a:/", "a:0", 7);
+}
```

</details>
