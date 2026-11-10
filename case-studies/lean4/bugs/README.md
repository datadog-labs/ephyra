# Bugs: lean4

The bugs the case studies found in lean4. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [Boxed constant is initialized eagerly (added to an existing issue, #15746)](lean4-boxed-const-eager-init/) | reported by others | [link](https://github.com/leanprover/lean4/issues/15746#issuecomment-6563437308) |
| [contradiction fails with an internal mkNoConfusion error on an offset of zero](lean4-contradiction-mknoconfusion-offset-zero/) | fixed | [link](https://github.com/leanprover/lean4/issues/16285) |
| [Equation lemmas fail to generate for a match on id a, a](lean4-eqn-lemma-drops-match-discr-hyp/) | open | [link](https://github.com/leanprover/lean4/issues/16259) |
| [Kernel rejects an equation lemma: unknown constant for the match equation (regression in 4.36.0-rc1)](lean4-eqn-lemma-missing-match-eqn/) | fixed | [link](https://github.com/leanprover/lean4/issues/16246) |
| [failed to generate equational theorem when a match discriminant hypothesis is unused in a closed proof](lean4-eqn-proof-closed-discr-hyp-unused/) | open | [link](https://github.com/leanprover/lean4/issues/16300) |
| [Segfault on a second withImportModules in one process: findOLean's root cache points into freed memory](lean4-findolean-cache-freed-region/) | fixed | [link](https://github.com/leanprover/lean4/issues/16267) |
| [grind.debug: PANIC in checkEqc on -x = -1 for Int8/Int16/Int32/Int64](lean4-grind-debug-checkeqc-int-neg/) | fixed | [link](https://github.com/leanprover/lean4/issues/16229) |
| [grind_linarith: internal error on negative Int8..Int64 literals](lean4-grind-linarith-neg-sint-literal/) | fixed | [link](https://github.com/leanprover/lean4/issues/16286) |
| [grind_linarith: internal error 'structure is not a preorder' on Fin n and BitVec](lean4-grind-linarith-no-ordered-add-preorder/) | fixed | [link](https://github.com/leanprover/lean4/issues/16287) |
| [grind builds a proof the kernel rejects for match conditions where a literal meets a pattern variable](lean4-grind-matchcond-decide-pattern-var/) | fixed | [link](https://github.com/leanprover/lean4/issues/16168) |
| [Kernel rejects a structure with an imax universe as too big, though it fits](lean4-kernel-is-geq-unequal-offsets/) | open | [link](https://github.com/leanprover/lean4/issues/16242) |
| [Kernel refuses 2 ^ 16777217 as over the numeral size limit, though the result is 2 MB](lean4-kernel-pow-size-guard-64x/) | open | [link](https://github.com/leanprover/lean4/issues/16230) |
| [With compiler.postponeCompile, leanir reads the placeholder .ir.sig of plain imports: Unknown constant _private...](lean4-leanir-placeholder-ir-sig-plain-import/) | open | [link](https://github.com/leanprover/lean4/issues/16201) |
| [lia/grind: kernel rejects the proof of x % 3 <= 2 * x % 3 -> 4 * x % 3 <= 2 * x % 3](lean4-lia-backtrack-kernel-mismatch/) | fixed | [link](https://github.com/leanprover/lean4/issues/16228) |
| [PANIC "getOptions called inside a computation recording its dependencies" when logAt runs inside synthInstance](lean4-logat-getoptions-panic-synthinstance/) | open | [link](https://github.com/leanprover/lean4/issues/16197) |
| [match on Float/Float32 literal patterns treats equal-valued literals as different](lean4-match-float-literal-equal-value/) | fixed | [link](https://github.com/leanprover/lean4/issues/16198) |
| [With compiler.extract_closed false, borrow inference treats string and big Nat literals as borrowed: use-after-free / wrong value](lean4-noclosed-borrowed-literal-uaf/) | open | [link](https://github.com/leanprover/lean4/issues/16160) |
| [NameTrie.findLongestPrefix? returns none through a key-less node](lean4-prefixtree-findlongestprefix-drops-match/) | open | [link](https://github.com/leanprover/lean4/issues/16299) |
| [String.Slice.revSplit drops the empty piece at the start of the slice](lean4-slice-revsplit-drops-leading-empty-piece/) | open | [link](https://github.com/leanprover/lean4/issues/16305) |
| [Std.Http: a URI with no authority and a path starting with // does not round-trip](lean4-std-http-uri-hostless-double-slash/) | fixed | [link](https://github.com/leanprover/lean4/issues/16268) |
| [Std.Http: URI.Builder.setUserInfo does not percent-encode : in the username](lean4-std-http-userinfo-colon-unencoded/) | open | [link](https://github.com/leanprover/lean4/issues/16269) |
| [Std.Time: pattern aaaa formats PM but parses only post meridiem](lean4-std-time-aaaa-roundtrip/) | fixed | [link](https://github.com/leanprover/lean4/issues/16247) |
| [Std.Time: pattern B formats 22:00 as 10:00 at night but parses it back as 10:00](lean4-std-time-b-night-roundtrip/) | fixed | [link](https://github.com/leanprover/lean4/issues/16248) |
| [Std.Time: Duration.toMilliseconds rounds negative durations down](lean4-std-time-duration-tomillis-negative/) | open | [link](https://github.com/leanprover/lean4/issues/16257) |
| [grind (maxChar := 1) with the Sym.simp normalizer builds a proof the kernel rejects](lean4-sym-arith-charone-kernel-mismatch/) | fixed | [link](https://github.com/leanprover/lean4/issues/16231) |
