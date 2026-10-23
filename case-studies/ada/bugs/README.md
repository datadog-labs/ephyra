# Bugs: ada

The bugs the case studies found in ada. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [`parse("C\|", file_base)` marks the URL as having an opaque path: host/path setters fail and it cannot be a base](ada-file-drive-letter-opaque-path/) | open | [link](https://github.com/ada-url/ada/issues/1278) |
| [combining-mark, bidi and normalization tables predate the Unicode 17 mapping table: leading marks and RTL letters from Unicode 14+ pass, Unicode 16 compositions are not applied](ada-idna-stale-unicode-tables/) | fixed | [link](https://github.com/ada-url/idna/issues/97) |
| [`url_search_params` keeps ill-formed UTF-8 from percent-decoding instead of decoding it to U+FFFD](ada-search-params-ill-formed-utf8/) | declined | [link](https://github.com/ada-url/ada/issues/1279) |
| [`url_aggregator::set_host`/`set_hostname` exceed `set_max_input_length` with IDNA-expanding hosts (regression from #1262)](ada-set-host-idna-expansion-limit/) | open | [link](https://github.com/ada-url/ada/issues/1326) |
| [`set_host("host:port")` under `set_max_input_length` applies the host, drops the port, and reports success](ada-set-host-port-length-limit/) | open | [link](https://github.com/ada-url/ada/issues/1282) |
| [URLPattern opaque pathname: a leading `//` is parsed as an authority and dropped (`//x/y` becomes `/y`)](ada-urlpattern-opaque-pathname-authority/) | open | [link](https://github.com/ada-url/ada/issues/1284) |
| [URLPattern `generate_pattern_string` drops the braces around a part whose prefix starts with `/` but is longer, so the pathname string no longer compiles to the same pattern](ada-urlpattern-pattern-string-prefix-grouping/) | fixed | [link](https://github.com/ada-url/ada/issues/1327) |
| [`canonicalize_protocol` indexes an empty string_view on a protocol pattern of an escaped colon (aborts in hardened builds)](ada-urlpattern-protocol-colon-oob/) | open | [link](https://github.com/ada-url/ada/issues/1283) |
| [URLPattern search canonicalization uses the non-special query encode set: a search pattern containing `'` never matches a special URL](ada-urlpattern-search-special-query-encode/) | open | [link](https://github.com/ada-url/ada/issues/1285) |
| [URLPattern: a literal backslash in a special-scheme pathname pattern is not canonicalized to "/" (spec's dummy URL is special)](ada-urlpattern-special-pathname-backslash/) | fixed | [link](https://github.com/ada-url/ada/issues/1281) |
| [URLPattern test/exec/match resolve a relative string input against an empty URL when no base is given (should not match)](ada-urlpattern-string-input-default-base/) | fixed | [link](https://github.com/ada-url/ada/issues/1280) |
| [x86-64 builds execute SSSE3 instructions unconditionally since #1216/#1230 (illegal instruction on CPUs without SSSE3)](ada-x86-ssse3-without-dispatch/) | reported by others | [link](https://github.com/ada-url/ada/pull/1321) |
