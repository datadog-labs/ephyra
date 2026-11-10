# Posted: https://github.com/ada-url/ada/issues/1326

Filed by alpaylan, 2026-10-13. Text as posted:

**`url_aggregator::set_host`/`set_hostname` exceed `set_max_input_length` with IDNA-expanding hosts (regression from #1262)**

Since #1262 ("perf: avoid url_aggregator copy in set_host_or_hostname"), `url_aggregator::set_host_or_hostname` takes its rollback snapshot only when `needs_rollback_snapshot(input.size())` says the limit can be reached, and checks the final length only when a snapshot exists:

```cpp
  if (needs_rollback_snapshot(input.size())) {
    saved_url = *this;
  }
  ...
    if (saved_url && buffer.size() > ada::get_max_input_length()) {
```

`needs_rollback_snapshot` bounds the growth by `buffer.size() + 3 * input + 16`, which assumes percent-encoding is the worst case. Domain to ASCII can grow a non-ASCII host further: U+337F (3 bytes of UTF-8) becomes the 17-byte label `xn--6oqv20b1zgzxr`. So such a host skips the snapshot, the final check is skipped too, and the setter returns `true` with an href longer than the limit:

```cpp
auto u = ada::parse<ada::url_aggregator>("ws:a");          // ws://a/
ada::set_max_input_length(59);
u->set_hostname("㈞㎯㌙㍿");               // 12 bytes of input
// true, href is 60 bytes (> 59). ada::url returns false and leaves the URL unchanged.
```

`repro.cpp` prints both types. Bisected: the commit before #1262 rejects it with both types, #1262 and main (42963db) accept it with `url_aggregator`. Not in a release (v4.0.0 rejects it).

**Fix** (`fix.patch`). Also take the snapshot when the host input contains a non-ASCII byte. ASCII input cannot outgrow the existing bound: an ACE label keeps its length, IPv4 normalization adds at most 6 bytes, and a percent escape costs 3 input bytes for at most one decoded code point. New typed test `set_hostname_idna_expansion_respects_limit` (both `set_hostname` and `set_host`), which fails on `url_aggregator` without the fix. On main 42963db with the patch, all 412 tests pass.

**How it was found.** A property that runs generated setter calls under limits near the lengths they reach, and requires every href to fit the limit and every setter to either succeed as it does without a limit or fail and change nothing. It failed on its first run against main after #1262, with this hostname shrunk to four code points.

Related: #1262 (the change), #1282 (another `set_host` length-limit case).

<details>
<summary>repro.cpp</summary>

```cpp
// set_hostname with IDNA-expanding characters under a length limit.
#include <cstdio>
#include <string>
#include "ada.h"
template <class T> void check(const char* name) {
  auto u = ada::parse<T>("ws:a");
  ada::set_max_input_length(59);
  bool ok = u->set_hostname("\xe3\x88\x9e\xe3\x8e\xaf\xe3\x8c\x99\xe3\x8d\xbf");  // ㈞㎯㌙㍿
  ada::set_max_input_length(UINT32_MAX);
  std::string href(u->get_href());
  printf("%-10s set_hostname -> %d, href %zu bytes (limit 59)%s\n", name, ok, href.size(), href.size() > 59 ? "  OVER THE LIMIT" : "");
}
int main() { check<ada::url>("url"); check<ada::url_aggregator>("aggregator"); }
```

</details>

<details>
<summary>Patch (fix.patch)</summary>

```diff
diff --git a/src/url_aggregator.cpp b/src/url_aggregator.cpp
index 2e3372b6..b0da5be8 100644
--- a/src/url_aggregator.cpp
+++ b/src/url_aggregator.cpp
@@ -677,10 +677,15 @@ bool url_aggregator::set_host_or_hostname(const std::string_view input) {
   // parse_host no longer mutates the buffer on failure (IPv4 is checked
   // before update_base_hostname), so the only rollback needed is for the
   // length limit. Snapshot only when the bound can be reached, like the
-  // other setters do via needs_rollback_snapshot.
+  // other setters do via needs_rollback_snapshot. That bound assumes
+  // percent-encoding (3x) is the worst growth, but domain to ASCII can grow
+  // a non-ASCII host further (U+337F, 3 bytes, becomes the 17-byte label
+  // "xn--6oqv20b1zgzxr"), so a non-ASCII host always takes the snapshot.
   const bool was_valid = is_valid;
   std::optional<url_aggregator> saved_url;
-  if (needs_rollback_snapshot(input.size())) {
+  const bool non_ascii = std::ranges::any_of(
+      input, [](char c) { return static_cast<unsigned char>(c) >= 0x80; });
+  if (needs_rollback_snapshot(input.size()) || non_ascii) {
     saved_url = *this;
   }
 
diff --git a/tests/max_input_length.cpp b/tests/max_input_length.cpp
index d1005aa7..2d0cd155 100644
--- a/tests/max_input_length.cpp
+++ b/tests/max_input_length.cpp
@@ -349,3 +349,21 @@ TEST(max_input_length_helpers, url_search_params_accepts_under_limit) {
   ASSERT_EQ(params.get("foo"), "bar");
   ada::set_max_input_length(std::numeric_limits<uint32_t>::max());
 }
+
+TYPED_TEST(max_input_length_tests, set_hostname_idna_expansion_respects_limit) {
+  // Domain to ASCII can grow a host by more than 3x its input (U+337F, 3
+  // bytes, becomes the 17-byte label "xn--6oqv20b1zgzxr"). A host setter
+  // whose result does not fit the limit must fail and leave the URL
+  // unchanged, however much IDNA grew it.
+  auto url = ada::parse<TypeParam>("ws:a");
+  ASSERT_TRUE(url);
+  std::string before(url->get_href());
+  ada::set_max_input_length(59);
+  // U+321E U+33AF U+3319 U+337F: 12 bytes of input, 51 bytes of host.
+  bool ok = url->set_hostname("\xe3\x88\x9e\xe3\x8e\xaf\xe3\x8c\x99\xe3\x8d\xbf");
+  bool ok_host = url->set_host("\xe3\x88\x9e\xe3\x8e\xaf\xe3\x8c\x99\xe3\x8d\xbf");
+  ada::set_max_input_length(small_limit);
+  ASSERT_FALSE(ok);
+  ASSERT_FALSE(ok_host);
+  ASSERT_EQ(url->get_href(), before);
+}
```

</details>
