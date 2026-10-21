**Title:** `set_host("host:port")` under `set_max_input_length` applies the host, drops the port, and reports success

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
