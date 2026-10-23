# vendor

The ada amalgamation (`ada.cpp`, `ada.h`, `ada_c.h`) that `build.rs`
compiles: [ada](https://github.com/ada-url/ada) main at
`42963db48056403f6ec3ff6e6dbea62ed724b6ec` (2026-10-12), generated with
`singleheader/amalgamate.py`, with ada's bundled IDNA (ada-idna v0.7.0,
Unicode 18 tables). [COMMIT](COMMIT) records the same.

On top of main, the source part (tests excluded) of the fix for each issue
still open when it was built, so that the search goes past them:

- [ada-url/ada#1278](https://github.com/ada-url/ada/issues/1278):
  [../../bugs/ada-file-drive-letter-opaque-path/fix.patch](../../bugs/ada-file-drive-letter-opaque-path/fix.patch)
- [ada-url/ada#1282](https://github.com/ada-url/ada/issues/1282):
  [../../bugs/ada-set-host-port-length-limit/fix.patch](../../bugs/ada-set-host-port-length-limit/fix.patch)
- [ada-url/ada#1283](https://github.com/ada-url/ada/issues/1283):
  [../../bugs/ada-urlpattern-protocol-colon-oob/fix.patch](../../bugs/ada-urlpattern-protocol-colon-oob/fix.patch)
- [ada-url/ada#1284](https://github.com/ada-url/ada/issues/1284):
  [../../bugs/ada-urlpattern-opaque-pathname-authority/fix.patch](../../bugs/ada-urlpattern-opaque-pathname-authority/fix.patch)
- [ada-url/ada#1285](https://github.com/ada-url/ada/issues/1285):
  [../../bugs/ada-urlpattern-search-special-query-encode/fix.patch](../../bugs/ada-urlpattern-search-special-query-encode/fix.patch)
- [ada-url/ada#1326](https://github.com/ada-url/ada/issues/1326) (a
  regression from #1262):
  [../../bugs/ada-set-host-idna-expansion-limit/fix.patch](../../bugs/ada-set-host-idna-expansion-limit/fix.patch)
- [ada-url/ada#1327](https://github.com/ada-url/ada/issues/1327):
  [../../bugs/ada-urlpattern-pattern-string-prefix-grouping/fix.patch](../../bugs/ada-urlpattern-pattern-string-prefix-grouping/fix.patch)

Already fixed on that main, so not patched: #1280 (by #1312), #1281 (by
#1313), ada-url/idna#97 (by idna#100 and ada#1288), the x86 SSSE3 dispatch
(by #1321). #1279 was closed as not planned, so its behaviour is live in this
build (the oracle excuses it).

To rebuild: clone ada, check out the commit above, `git apply
--exclude='tests/*'` each patch, then `AMALGAMATE_OUTPUT_PATH=<this
directory> python3 singleheader/amalgamate.py` and delete the demo files it
also writes. `bench/mutants.sh` does exactly this for the benchmark variants.
Regenerate when the remaining issues are fixed upstream.

ada is dual-licensed under [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE) (copied from that commit). `ada.h` bundles
tl::expected, dedicated to the public domain under CC0 (its header says so).
