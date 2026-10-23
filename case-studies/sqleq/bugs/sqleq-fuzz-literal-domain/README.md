# sqleq-fuzz: literals outside the generated value domain (filed as #187)

Filed 2026-10-14 20:12 UTC by alpaylan: https://github.com/datadog-labs/sqleq/issues/187
(text: `github-title.txt`, `github-body.md`). A documented limitation of sqleq-fuzz (README, "The
generated value domain"), filed as a feature request: add compared literals and their neighbours to
the domain. Verified on sqleq main bbb86f3: `334 <= c` vs `334 < c` is NO-COUNTEREXAMPLE even with
`--trials 2000 --rows 8`; with `1` it is NOT-EQUIVALENT. The `s*.sql` files are the minimized
fuzz-missed near misses from ephyra-sqleq (`out/nearmiss2-0-20`), each with its PostgreSQL witness
rows and both results.
