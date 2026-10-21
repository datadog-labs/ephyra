**Title:** `url_aggregator::set_host`/`set_hostname` exceed `set_max_input_length` with IDNA-expanding hosts (regression from #1262)

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
