# ephyra-ada

A testing engine for [ada](https://github.com/ada-url/ada), the WHATWG URL
parser in C++ used by Node.js, built on ephyra (`../../crates/ephyra`). URLs
come from a typed grammar whose generators `ephyra check` verifies; they run
through ada under differential and metamorphic oracles; the search is scored
by coverage of ada's C++ code ([../ada-coverage](../ada-coverage)), and the
same properties also run on a sanitized build ([../ada-asan](../ada-asan)).
Every failure ends as a minimal input. Found twelve bugs in ada and its
IDNA library (below).

ada is already fuzzed heavily (OSS-Fuzz; libFuzzer harnesses for `parse`,
`can_parse`, IDNA, search params and URLPattern; its own fuzzers compare its
two URL representations) and passes the web-platform-tests. The edge here:
structure-aware generation and verified shrinking, coverage-guided scoring
with targets inside ada, and oracles a byte fuzzer does not have (independent
implementations of the standard, setter round trips, length limits).

## Running

- A C++20 compiler (`build.rs` compiles `vendor/` and `cpp/shim.cpp` with
  the `cc` crate) and Node.js: `npm install` in `reference/` first
  (`node_modules/` is ignored).
- `cargo test` runs the smoke tests and the properties at ephyra's default
  count. Larger runs: `EPHYRA_TESTS=200000 EPHYRA_MAX_SIZE=250 cargo test
  --release --test oracles` (a few minutes, mostly the whatwg-url and
  polyfill round trips). `EPHYRA_SEED` fixes the seed.
- Generators and strategies: `cargo build -p ephyra-cli` at the repository
  root, then `../../target/debug/ephyra check` here.
- Triage: `cargo run --example show -- INPUT [--base BASE] [setter=value]...`
  prints the transcripts of both ada representations, whatwg-url and the
  `url` crate. `examples/show_pattern.rs`, `pattern_triage.rs` (patterns,
  three-way), `init_triage.rs` (URLPatternInit inputs on `{}`),
  `path_diff.rs` and `opaque_diff.rs` (pathname canonicalization),
  `idna_triage.rs` and `list_stats.rs` (how often the pattern-list oracle
  fires) are the other triage tools.
- x86-64 on an ARM Mac: `CARGO_TARGET_DIR=target/x86 cargo test --target
  x86_64-apple-darwin` runs everything under Rosetta (SSE up to 4.2, no AVX),
  which executes ada's SSE2/SSSE3 kernels instead of NEON. The AVX-512
  kernels need an x86 machine with AVX-512 (or Intel SDE).
- `longrun/run.sh`: long bug-hunting runs (all properties, fresh seeds, 3M
  cases per seed and size) on a frozen copy of the test binary
  (`cargo test --release --test oracles --no-run`, copied to
  `longrun/oracles.bin`), one log per seed and size.

## Design

- `src/lib.rs`: `Case` (input, optional base, setter calls), `Setter`,
  `Parts` (the eleven getters), `Step`, `Transcript`.
- `src/ada.rs` + `cpp/shim.cpp`: `ada::run(kind, case, max_input_length)`
  runs a case on `ada::url` or `ada::url_aggregator` and returns the
  transcript and whether `url_aggregator::validate()` held after every step.
  `max_input_length` (0 for ada's default) is process-global in ada; runs
  hold a mutex. ada's development checks are off because their assertions
  abort the process (losing the case); the shim calls `validate()` instead.
  The `ada-url` crate is not used: its API reaches only the aggregator.
- `build.rs`: compiles `vendor/` (see [vendor/README.md](vendor/README.md))
  with URLPattern on `std::regex`, `-std=c++20`, `ADA_DEVELOPMENT_CHECKS=0`.
  `EA_VENDOR=<dir>` compiles another amalgamation (the benchmark below).
- `src/reference.rs` + `reference/run.js`: the same transcript from
  whatwg-url 17.2.0 (with tr46 7.0.0), the standard's reference
  implementation, through one long-lived `node` process speaking JSON lines.
  U+2028/U+2029 are escaped because Node's readline splits lines on them.
  Node's own `URL` is ada, so it is not an independent opinion, and Node's
  binding re-parses the href around setters, which can hide state bugs.
  `src/servo.rs`: the same from the `url` crate 2.5 (servo's, its `quirks`
  module), a third opinion for triage; it deviates from the standard in
  places, so whatwg-url is the reference for differences.
- `src/params.rs`, `src/pattern.rs`, `src/pattern_list.rs`: URLSearchParams
  (init string, then append, delete, set, sort, get, getAll, has and the
  iterators), URLPattern and URLPattern-list (`url_pattern_list`) cases, each
  run on ada through the shim and on references: whatwg-url and the `url`
  crate for search params; urlpattern-polyfill 10.1.0 (`reference/run.js`)
  and Deno's `urlpattern` crate 0.6 (`pattern::deno`, on the Rust `regex`
  crate) for URLPattern. Non-UTF-8 bytes out of ada are shown as `\xNN`
  (`params::text`), never replaced.
- `src/init_inputs.rs`: URLPatternInit inputs run through the all-wildcard
  pattern `{}` (processed init is canonicalization, engine-independent),
  against urlpattern-polyfill and Deno.
- `src/canonical.rs`: a verified complex validity condition. `is_canonical`
  holds when `parse(s).href == s`; `canonical_urls` generates only such
  strings, and `ephyra check` verifies that its shrinker and mutator keep
  them canonical. It densifies the valid region for strong oracles. The
  obvious "setting a component to its own getter value is a no-op" law is
  unsound (the `?`/`#` empty-delimiter asymmetry: a bare `#` or `?` reads
  back as `""`, and setting `""` removes it); setter idempotence (twice ==
  once) is sound and kept.
- `src/idna_direct.rs`: ada's IDNA called directly, against tr46. Not a
  sound oracle and not a kept property: ada's `to_ascii`/`to_unicode` follow
  the URL Standard, tr46 is pure UTS-46 (about 63% by-design differences,
  none visible through the URL API). Kept for exploration; the sound IDNA
  oracle is host parsing against whatwg-url.
- `src/gen.rs`: `cases()`, the grammar (see its module docs): schemes,
  slashes, credentials, hosts of every kind (domains with IDNA-relevant
  Unicode and punycode, IPv4 in every radix and part count, IPv6 with
  compression and embedded IPv4, opaque, percent-encoded), ports, paths with
  dot segments and drive letters, queries, fragments, noise; relative inputs
  with a base; setter calls with values drawn per setter. Our own AST,
  rendered by `Display`, so a counterexample prints as the input itself.
  Shrinking on the final strings, mutation by splicing grammar fragments. No
  predicate: every string is a valid parser input.
- `tests/oracles.rs` includes `tests/oracles/properties.rs` (shared with
  `../ada-asan`, so no inner attributes there). Seventeen properties:
  `representations_agree` (`ada::url` = aggregator, `validate()` holds),
  `representations_agree_under_limits`, `can_parse_agrees_with_parse`,
  `href_round_trips`, `urls_behave_as_their_href` (after every step, the
  remaining setters do the same to the re-parsed href),
  `failed_setters_change_nothing`, `length_limit_holds` (under
  `set_max_input_length` near the case's lengths: no href over the limit;
  each step as without a limit, or failed and unchanged),
  `predicates_agree_with_components` (the `has_*` predicates),
  `ada_agrees_with_reference`, `search_params_agree_with_reference`,
  `canonical_url_is_reference_canonical` (whole-href differential on
  canonical URLs), `setters_are_idempotent`,
  `pattern_agrees_with_reference_majority`, `pattern_list_agrees_with_patterns`,
  `pattern_test_agrees_with_exec`, `init_test_agrees_with_exec` and
  `pattern_components_are_stable`. Each has run at 200,000 cases (max size
  250) on ARM, the newest at 3,000,000; the first thirteen at 30,000 on
  x86-64 under Rosetta and at 150,000 under the sanitizers. Known gaps are
  excused in place, with comments.
- `tests/smoke.rs`: hand-picked cases across all implementations, and
  regression tests for the bugs.

`pattern_agrees_with_reference_majority` compares ada with the two
URLPattern references and only flags ada when both agree against it: the
references disagree with each other and with ada on many cases (each has its
own bugs and its own URL parser). Two references agreeing is evidence, not
proof: for `{protocol:"file", port:"0x50"}` both reject the input for
different reasons (the polyfill rejects trailing junk, Deno rejects any port
on `file`), while ada's `0` is what the spec's port state gives. Clusters
where both agree against ada, triaged as not ada bugs and masked in the
property:

- **std::regex gives up.** A wildcard or group with a `*`/`+` modifier and
  no prefix (`**`, `:a*`, `(.*)*`) compiles, as the spec says, to a nested
  quantifier such as `((?:.*)*)`. libc++ throws `error_complexity` on it
  past about a dozen input bytes, and ada's `std_regex_provider` turns that
  into "no match". `cpp/shim.cpp` runs patterns on a recording regex
  provider, which reports such an answer as 3. The same regexes make the
  polyfill's JS RegExp backtrack for minutes, so the vote skips the case.
- **`file:` inputs with a `//` pathname** (`file:////users/`, pathname
  `//users/` per the URL Standard, as ada has it). The polyfill re-parses
  the pathname as a whole URL, so `//users` becomes a host
  (kenchris/urlpattern-polyfill#128). The `url` crate under Deno drops the
  empty segments (`/users/`; compare servo/rust-url#1118).
- **Opaque hosts the special-host parser changes** (`foo://é/` with host
  `%C3%A9`, `foo://2/`, `x://.1/`). The polyfill canonicalizes an input's
  hostname by setting it on an `https:` URL, which throws on `%`, turns `2`
  into `0.0.0.2` and ignores `.1`. Separately, Deno uses `/` as the segment
  delimiter for non-special pathnames, where the spec's default options have
  none (`foo://*/:x` matches `foo://e/a/`).
- **Deno and `ignoreCase`.** The `urlpattern` crate's `regex::Regex` engine
  drops the flags, so its regexp fallback ignores `ignoreCase` (worked
  around in the harness: `CaseRegex` in `src/pattern.rs`). Its
  `Matcher::matches` also compares a literal prefix and suffix
  case-sensitively (`/*a9` against `/xA9`). Under `ignoreCase`, an input
  whose ada answer depends on it gets no vote.
- **Deno panics** on some crafted patterns (a subtract-with-overflow in its
  constructor-string parser, e.g. `*:]`); `pattern::deno` returns no answer
  then.

Open question: ada compiles a `\:*` protocol pattern (a modifier on an
escaped delimiter) that the polyfill and Deno both reject. Not compared until
it is settled where, if anywhere, the spec rejects it.

## Facts about ada that shape the design

- **Two representations.** `ada::url` (components as separate strings) and
  `ada::url_aggregator` (one buffer with offsets) implement the same
  standard separately, through templated parsing (`parse_url_impl<T>`).
  They must agree on every input and after every setter.
- **Fast paths.** Canonical absolute URLs take a fast path, and
  per-architecture SIMD kernels scan, classify and parse hosts. On ARM, the
  NEON kernels and scalar fallbacks run.
- **main moves fast.** After 4.0.0, main gained many new fast paths: SIMD
  scanners and a host-preserving absolute fast path, 8-byte scheme matching
  and SWAR, a single structural edit for aggregator queries, setters that
  skip their rollback snapshot when a limit cannot be reached, SIMD
  `percent_encode`, faster `percent_decode`, `can_parse` for canonical HTTP
  URLs. Several later commits fix bugs in that same code (tab/newline
  stripping in the fast path, `ada::url::host_end` one byte short,
  `max_input_length` on early returns, NFC detection). New code is the best
  place to look, and a bug found only in 4.0.0 may already be fixed: test
  main.
- **The standard has its own test data.** ada's `tests/wpt/` holds WPT's
  `urltestdata.json`, `setters_tests.json`, `toascii.json` and
  `IdnaTestV2.json`, plus ada's own extra URL, setter and long-URL data:
  seeds, and a source of known-tricky shapes for the grammar.

## Findings

Each folder under [../bugs/ada](../bugs/ada) has the issue as filed
(`issue.md`) and, where there is one, a fix with a test (`fix.patch`: the
test fails without the fix, ada's suite passes with it). `vendor/` carries
the fixes of the open ones.

- [ada-url/ada#1278](https://github.com/ada-url/ada/issues/1278), open:
  [ada-file-drive-letter-opaque-path](../bugs/ada/ada-file-drive-letter-opaque-path/).
  A reference starting with a drive letter and `|` (`C|`, `C|/x`) against a
  `file:` base leaves `has_opaque_path` true: `set_host`, `set_hostname` and
  `set_pathname` fail, and the URL cannot be a base. Found by
  `ada_agrees_with_reference` and `urls_behave_as_their_href`.
- [ada-url/ada#1279](https://github.com/ada-url/ada/issues/1279), declined:
  [ada-search-params-ill-formed-utf8](../bugs/ada/ada-search-params-ill-formed-utf8/).
  `url_search_params` keeps ill-formed UTF-8 from percent-decoding (`%aa`)
  instead of decoding it to U+FFFD as the standard does. Excused exactly in
  `search_params_agree_with_reference`.
- [ada-url/ada#1280](https://github.com/ada-url/ada/issues/1280), fixed by
  #1312:
  [ada-urlpattern-string-input-default-base](../bugs/ada/ada-urlpattern-string-input-default-base/).
  `url_pattern::test`/`exec`/`match` default-constructed
  `result<url_aggregator>`, an empty URL, so a relative string input with no
  base resolved against it and matched instead of failing (a pathname-only
  pattern `{pathname:"/users/:id"}` matched the input `/users/1`).
- [ada-url/ada#1281](https://github.com/ada-url/ada/issues/1281), fixed by
  #1313:
  [ada-urlpattern-special-pathname-backslash](../bugs/ada/ada-urlpattern-special-pathname-backslash/).
  A special-scheme pathname pattern did not convert `\` to `/` (nor
  normalize dot segments): `canonicalize_pathname` parsed against a
  non-special dummy scheme, so `{protocol:"https", pathname:"/a\\b"}` never
  matched `/a/b`.
- [ada-url/ada#1282](https://github.com/ada-url/ada/issues/1282), open:
  [ada-set-host-port-length-limit](../bugs/ada/ada-set-host-port-length-limit/).
  Under `set_max_input_length`, `set_host("host:port")` whose result does
  not fit applies the host, keeps the old port and returns true. Found by
  `length_limit_holds`.
- [ada-url/ada#1283](https://github.com/ada-url/ada/issues/1283), open:
  [ada-urlpattern-protocol-colon-oob](../bugs/ada/ada-urlpattern-protocol-colon-oob/).
  `canonicalize_protocol` checks for empty input before stripping a trailing
  `:`, so a protocol pattern of an escaped colon (`\:?`, `\:*`, `{\:}?://a`)
  indexes an empty `string_view`. Invisible in plain builds and to ASan;
  libc++ hardening, even `fast` mode, aborts.
- [ada-url/ada#1284](https://github.com/ada-url/ada/issues/1284), open:
  [ada-urlpattern-opaque-pathname-authority](../bugs/ada/ada-urlpattern-opaque-pathname-authority/).
  `canonicalize_opaque_pathname` parses `"fake:" + value`, so a leading `//`
  becomes an authority and is dropped: `new URLPattern("foo://h//x/y")` has
  pathname `/y` and does not match its own URL.
- [ada-url/ada#1285](https://github.com/ada-url/ada/issues/1285), open:
  [ada-urlpattern-search-special-query-encode](../bugs/ada/ada-urlpattern-search-special-query-encode/).
  `canonicalize_search` uses the non-special query encode set, but the
  spec's dummy URL is special: `{search:"q='x'"}` never matches
  `https://h/?q='x'` (query `q=%27x%27`). Found by `examples/init_triage.rs`.
- [ada-url/ada#1326](https://github.com/ada-url/ada/issues/1326), open:
  [ada-set-host-idna-expansion-limit](../bugs/ada/ada-set-host-idna-expansion-limit/).
  A regression from #1262: `url_aggregator`'s host setters exceed
  `set_max_input_length` with IDNA-expanding hosts. Found by
  `length_limit_holds` on its first run against the new main.
- [ada-url/ada#1327](https://github.com/ada-url/ada/issues/1327), open:
  [ada-urlpattern-pattern-string-prefix-grouping](../bugs/ada/ada-urlpattern-pattern-string-prefix-grouping/).
  `generate_pattern_string` compares only the first character of a part's
  prefix with `/`, so `{/9*}` serializes as `/9*`, and `/a{/9*}?` as
  `/a/9*?`, which matches different URLs. Found by
  `pattern_agrees_with_reference_majority` in a long run (after 2.6M cases).
- [ada-url/idna#97](https://github.com/ada-url/idna/issues/97), fixed by
  idna#100 and ada#1288:
  [ada-idna-stale-unicode-tables](../bugs/ada/ada-idna-stale-unicode-tables/).
  The combining-mark and normalization tables predated the Unicode 17
  mapping table: 247 marks from Unicode 14+ were accepted at the start of a
  label (`new URL("http://\u08CBa/")` parsed in Node.js), and 20 Unicode 16
  compositions were not applied.
- x86 SSSE3 kernels without runtime dispatch, fixed by
  [ada-url/ada#1321](https://github.com/ada-url/ada/issues/1321) before it was
  filed: [ada-x86-ssse3-without-dispatch](../bugs/ada/ada-x86-ssse3-without-dispatch/).
  Since #1216/#1230, baseline x86-64 builds contained `pshufb` with no
  runtime check (SIGILL on CPUs without SSSE3; not demonstrated).

Known and excused:

- The RFC 5893 Bidi rule is applied only to labels with an RTL character of
  their own, not to every label of a Bidi domain name (`http://ب.-/` is
  accepted): ada-url/ada#1251, closed in favour of ada-url/idna#91.
- Hangul LV+T composition (idna#88) is fixed in ada-url/idna but was not yet
  synced into ada when this was written: `new URL("ws://\uAC00\u11A8")` is
  `xn--rud7310f` in ada and Node.js, `xn--p39a` in the standard.
- The standard itself does not round-trip `file://a/a|` (made by the
  protocol setter from `ws:a/a|`): a parse reads `a|` as the drive letter
  `a:`. whatwg-url agrees with ada.
- tr46 7.0.0 (whatwg-url's IDNA) accepts a label with a virama-licensed ZWNJ
  followed by an unlicensed one (`http://\u1842\ua8c4\u200c\u1842\u200c/`):
  its CONTEXTJ regex runs over a window from the last licensed ZWNJ. ada and
  the `url` crate reject it, as RFC 5892 says. The grammar does not make
  such labels yet.
- `url` crate deviations (triage only): `new URL("\\", "a:/")` loses the
  backslash; the protocol setter refuses `ws:` to `file:`.

## Synthesis rounds

Generator changes are judged in `../ada-coverage` with `cargo run --release
--example score -- <seeds>`: the joint coverage score is the number of
distinct ada counters reached by running a fixed seed set through all five
generators, with a fresh-seed re-score on a disjoint range to catch
overfitting, and a by-area breakdown. `EA_GAPS=N` lists the N functions
missing the most counters (mangled; pipe through `c++filt`). A change is
kept only if both the score and the fresh score rise; rounds are logged in
`results.tsv` (ignored). The first rounds took the score from 4023 to 4184
of 4912 counters (2500 seeds): kept a clean-HTTP fast-path shape, tokenizer
error and edge shapes, and wide static route sets; discarded DNS-length
labels.

gap-round-1 (per-line gaps read with `llvm-cov show` on the score binary and
its newest `target/profraw/` file) took it from 4194/4128 to 4275/4227
(score/fresh) of 4947: references sharing the base's scheme, file bases with
drive letters paired with file references, host-setter edge values,
regexp-group grammar and segment-wildcard regexps, ACE overflow and
double-ACE labels, Hangul, colliding route fan-outs, `blob:` inner URLs, the
search-params iterators. New laws: `can_parse` under a limit, truncated list
captures (the first 8 of a route alone), `init_test_agrees_with_exec`,
`predicates_agree_with_components`. The fresh score moves by about 5 either
way on a pure reshuffle of draws, so small changes are hard to judge.

Remaining gaps not worth chasing: most of `parse_url_impl<url_aggregator,
false>`'s gap is unreachable (`can_parse` parses its base without storing
values, so the base never has a stored search or path there);
`checkers::verify_dns_length` needs the `verifyDNSLength` flag that the URL
and URLPattern entry points never set; `idna::deflate::inflate_raw` is the
one-time table decompressor, driven by which table regions any non-ASCII
input touches, not by grammar. The reachable headroom is in
`parser::parse_url_impl` error-recovery states. Rounds run on a time budget
with no stopping thresholds.

## Bug benchmark (`bench/`)

`bench/mutants.sh` builds ada with one known bug put back (8 variants plus a
control with all fixes in) into `bench/mutants/<id>/`; `bench/run.sh` runs
the properties on each, with saved counterexamples off
(`EPHYRA_REGRESSIONS=0`), and writes one row per variant, seed and property
to `bench/results/` (logs in `bench/logs/`). `FILTER=<names>` runs only some
properties. The split into tuning and held-out variants is in `bench/SPLIT`:
judge harness changes on tuning, look at held-out only as a monitor. Runs
are slow (debug build, 300k cases); run variants in parallel lanes
(`VARIANTS=...`, distinct `LABEL`s).

The variants, results and logs are not in this repository. Generate the
variants first: `ADA_SRC=<an ada clone containing 42963db> bench/mutants.sh`
(bash, python3 and git; it applies the fixes from `../bugs/ada/`).
`bench/FROZEN.sha256` freezes the scripts, the split and the variants:
`cd bench && shasum -a 256 -c FROZEN.sha256` checks them. The amalgamation's
timestamp is the commit's, so the variants regenerate byte for byte. Two
lines of `mutants.sh` were updated for this repository (the ada clone comes
from `ADA_SRC`, the fixes from `../bugs/ada/`), and its hash with them; its
logic is unchanged.

Detection at 300,000 cases, seeds 11/22/33 (passing cases before the
failure):

| variant | 2026-10-12 baseline | 2026-10-13 | by |
|---|---|---|---|
| control | clean | clean | |
| m1278 drive letter | 3/3 (18k-101k) | same | `ada_agrees_with_reference` |
| m1280 string input, no base | 0/3 | 3/3 (17-1065) | `pattern_agrees_with_reference_majority` |
| m1282 set_host limit | 3/3 (3.5k-23k) | same | `length_limit_holds` |
| m1285 special-query `'` | 0/3 | 3/3 (1.5k-2.4k) | majority, after the encode-set characters joined the pattern generator |
| m1326 set_host IDNA limit | 3/3 (26k-188k) | same | `length_limit_holds`, `representations_agree_under_limits` |
| m1281 backslash (held-out) | 0/3 | 3/3 (84-537) | majority |
| m1284 opaque `//` (held-out) | 0/3 | 0/3 | the polyfill has its own bug there, so no majority |
| midna97 tables (held-out) | 3/3 (24k-253k) | same | `ada_agrees_with_reference` |

The m1278 count was under 3,000 on 2026-10-07: the coverage-oriented
generator changes since then diluted the drive-letter shapes.

## Verifying

`cargo test`; `cargo clippy --all-targets -- -D warnings`; rustfmt
(`rustfmt.toml` is the stable subset of ephyra's); `ephyra check` for the
generators; coverage in `../ada-coverage` (`cargo test -- --test-threads=1`).
