# Bugs: coreutils

The bugs the case studies found in coreutils. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [coreutils-basenc-base64url-decode-rejects-unpadded](coreutils-basenc-base64url-decode-rejects-unpadded/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15664) |
| [coreutils-cat-squeeze-blank-drops-blank-after-spli](coreutils-cat-squeeze-blank-drops-blank-after-spli/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15655) |
| [coreutils-chmod-leading-dash-mode-followed-by-oper](coreutils-chmod-leading-dash-mode-followed-by-oper/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15665) |
| [coreutils-chmod-reference-first-operand-processed-](coreutils-chmod-reference-first-operand-processed-/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15666) |
| [comm: sorted input rejected as "not in sorted order" when a line is a prefix of the next followed by a tab](coreutils-comm-compares-line-terminator/) | fixed | [link](https://github.com/uutils/coreutils/issues/15439) |
| [coreutils-csplit-negative-offset-regex-hangs-after](coreutils-csplit-negative-offset-regex-hangs-after/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15653) |
| [csplit: a `/REGEX/` pattern after a line-number pattern never tests the line the split stopped at](coreutils-csplit-regex-skips-line-after-line-numbe/) | open | [link](https://github.com/uutils/coreutils/issues/15440) |
| [coreutils-csplit-suppress-matched-line-just-past-e](coreutils-csplit-suppress-matched-line-just-past-e/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15584) |
| [coreutils-cut-output-delimiter-repeats-last-field-](coreutils-cut-output-delimiter-repeats-last-field-/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15654) |
| [coreutils-dd-swab-pairs-reset-per-read-block](coreutils-dd-swab-pairs-reset-per-read-block/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15646) |
| [coreutils-expand-column-reset-at-file-and-buffer-b](coreutils-expand-column-reset-at-file-and-buffer-b/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15656) |
| [coreutils-expand-nul-counted-as-one-column](coreutils-expand-nul-counted-as-one-column/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15580) |
| [coreutils-expand-tabs-increment-checked-as-ascendi](coreutils-expand-tabs-increment-checked-as-ascendi/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15579) |
| [coreutils-fmt-first-word-sentence-end-single-space](coreutils-fmt-first-word-sentence-end-single-space/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15585) |
| [coreutils-fold-control-chars-zero-width](coreutils-fold-control-chars-zero-width/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15571) |
| [coreutils-join-trailing-blanks-phantom-field-and-i](coreutils-join-trailing-blanks-phantom-field-and-i/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15576) |
| [coreutils-nl-resets-logical-page-section-between-f](coreutils-nl-resets-logical-page-section-between-f/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15586) |
| [coreutils-nl-unnumbered-line-padding-ignores-separ](coreutils-nl-unnumbered-line-padding-ignores-separ/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15591) |
| [coreutils-numfmt-invalid-field-leaves-whole-line-u](coreutils-numfmt-invalid-field-leaves-whole-line-u/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15575) |
| [coreutils-numfmt-rounded-to-zero-prints-negative-z](coreutils-numfmt-rounded-to-zero-prints-negative-z/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15661) |
| [coreutils-numfmt-to-si-unscaled-value-ignores-roun](coreutils-numfmt-to-si-unscaled-value-ignores-roun/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15574) |
| [coreutils-od-strings-hex-address-width](coreutils-od-strings-hex-address-width/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15592) |
| [coreutils-printf-scientific-rounds-exact-ties-half](coreutils-printf-scientific-rounds-exact-ties-half/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15662) |
| [coreutils-printf-string-star-precision-above-int-m](coreutils-printf-string-star-precision-above-int-m/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15663) |
| [coreutils-seq-equal-width-counts-increment-digits](coreutils-seq-equal-width-counts-increment-digits/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15572) |
| [coreutils-sort-global-i-not-overridden-by-d](coreutils-sort-global-i-not-overridden-by-d/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15578) |
| [coreutils-sort-human-numeric-ignores-f-fold](coreutils-sort-human-numeric-ignores-f-fold/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15577) |
| [coreutils-sort-key-own-reverse-ignored-on-whole-li](coreutils-sort-key-own-reverse-ignored-on-whole-li/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15570) |
| [coreutils-sort-treats-form-feed-and-cr-as-blanks](coreutils-sort-treats-form-feed-and-cr-as-blanks/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15588) |
| [coreutils-sort-version-ignores-i-d-f](coreutils-sort-version-ignores-i-d-f/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15587) |
| [coreutils-split-lines-count-parsed-as-byte-size](coreutils-split-lines-count-parsed-as-byte-size/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15660) |
| [coreutils-split-suffix-length-zero-ignores-default](coreutils-split-suffix-length-zero-ignores-default/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15581) |
| [coreutils-split-suffix-start-too-large-accepted-on](coreutils-split-suffix-start-too-large-accepted-on/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15569) |
| [coreutils-split-verbose-silent-in-number-mode](coreutils-split-verbose-silent-in-number-mode/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15583) |
| [coreutils-tr-string2-construct-validation-mismatch](coreutils-tr-string2-construct-validation-mismatch/) | reported 2026-10-21 | [link](https://github.com/uutils/coreutils/issues/15589) |
| [coreutils-unexpand-dash-operand-taken-as-tab-short](coreutils-unexpand-dash-operand-taken-as-tab-short/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15658) |
| [coreutils-unexpand-line-state-reset-at-file-bounda](coreutils-unexpand-line-state-reset-at-file-bounda/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15657) |
| [coreutils-uniq-skip-fields-treats-cr-ff-as-blanks](coreutils-uniq-skip-fields-treats-cr-ff-as-blanks/) | reported 2026-10-22 | [link](https://github.com/uutils/coreutils/issues/15659) |
