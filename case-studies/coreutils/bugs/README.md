# Bugs: coreutils

The bugs the case studies found in coreutils. Each folder has the report as
it was filed upstream, the reproduction, and where we had one, the fix and
a test. Status as of this export; the upstream link is authoritative.

| Bug | Status | Upstream |
|---|---|---|
| [basenc: `--base64url -d` rejects input whose last group has no `=` padding](coreutils-basenc-base64url-decode-rejects-unpadded/) | fixed | [link](https://github.com/uutils/coreutils/issues/15664) |
| [cat: `-s` drops a single blank line when the line before it was continued from an earlier file or read](coreutils-cat-squeeze-blank-drops-blank-after-spli/) | fixed | [link](https://github.com/uutils/coreutils/issues/15655) |
| [chmod: modes like `-=r`, `-+x` and `-,u+x` are rejected as unknown options](coreutils-chmod-leading-dash-mode-followed-by-oper/) | fixed | [link](https://github.com/uutils/coreutils/issues/15665) |
| [chmod --reference: the first FILE operand is processed last](coreutils-chmod-reference-first-operand-processed-/) | fixed | [link](https://github.com/uutils/coreutils/issues/15666) |
| [comm: sorted input rejected as "not in sorted order" when a line is a prefix of the next followed by a tab](coreutils-comm-compares-line-terminator/) | fixed | [link](https://github.com/uutils/coreutils/issues/15439) |
| [csplit: hangs on a `/REGEX/-N` pattern after a line-number pattern that names the line a regex stopped at](coreutils-csplit-negative-offset-regex-hangs-after/) | open | [link](https://github.com/uutils/coreutils/issues/15653) |
| [csplit: a `/REGEX/` pattern after a line-number pattern never tests the line the split stopped at](coreutils-csplit-regex-skips-line-after-line-numbe/) | open | [link](https://github.com/uutils/coreutils/issues/15440) |
| [csplit: with `--suppress-matched`, a line number one past the end of input is rejected as out of range](coreutils-csplit-suppress-matched-line-just-past-e/) | fixed | [link](https://github.com/uutils/coreutils/issues/15584) |
| [cut: with `--output-delimiter`, the last field is printed twice when the line has no terminator](coreutils-cut-output-delimiter-repeats-last-field-/) | fixed | [link](https://github.com/uutils/coreutils/issues/15654) |
| [dd: `conv=swab` pairs bytes within each input block, so the result depends on `bs=` and on how a pipe delivers data](coreutils-dd-swab-pairs-reset-per-read-block/) | open | [link](https://github.com/uutils/coreutils/issues/15646) |
| [expand: tab stops are misplaced after a file without a trailing newline, and on lines longer than 8 KiB](coreutils-expand-column-reset-at-file-and-buffer-b/) | open | [link](https://github.com/uutils/coreutils/issues/15656) |
| [expand: a NUL byte takes one column, so the next tab stop comes one column early](coreutils-expand-nul-counted-as-one-column/) | fixed | [link](https://github.com/uutils/coreutils/issues/15580) |
| [expand: `--tabs=2,/2` and `--tabs=4,+2` are rejected with "tab sizes must be ascending"](coreutils-expand-tabs-increment-checked-as-ascendi/) | fixed | [link](https://github.com/uutils/coreutils/issues/15579) |
| [fmt: one space instead of two after a sentence-ending first word at the end of a line](coreutils-fmt-first-word-sentence-end-single-space/) | fixed | [link](https://github.com/uutils/coreutils/issues/15585) |
| [fold: control characters such as `\f`, ESC and DEL take no column, so long lines are not folded](coreutils-fold-control-chars-zero-width/) | open | [link](https://github.com/uutils/coreutils/issues/15571) |
| [join: with `-i`, every line of a matching group is printed with the first line's join field](coreutils-join-trailing-blanks-phantom-field-and-i/) | fixed | [link](https://github.com/uutils/coreutils/issues/15576) |
| [nl: header/footer section is reset to body at the start of each input file](coreutils-nl-resets-logical-page-section-between-f/) | fixed | [link](https://github.com/uutils/coreutils/issues/15586) |
| [nl: unnumbered lines are padded with width+1 spaces, ignoring the length of `-s`](coreutils-nl-unnumbered-line-padding-ignores-separ/) | fixed | [link](https://github.com/uutils/coreutils/issues/15591) |
| [numfmt: with `--invalid=fail/warn/ignore`, one invalid field leaves the whole line unconverted](coreutils-numfmt-invalid-field-leaves-whole-line-u/) | fixed | [link](https://github.com/uutils/coreutils/issues/15575) |
| [numfmt: a negative number that rounds to zero is printed as `-0`](coreutils-numfmt-rounded-to-zero-prints-negative-z/) | fixed | [link](https://github.com/uutils/coreutils/issues/15661) |
| [numfmt: `--to=si`/`--to=iec` ignores `--round` for values from 10 to 999 that get no suffix](coreutils-numfmt-to-si-unscaled-value-ignores-roun/) | open | [link](https://github.com/uutils/coreutils/issues/15574) |
| [od: `-S` with `-Ax` prints 7-digit hex offsets instead of 6](coreutils-od-strings-hex-address-width/) | fixed | [link](https://github.com/uutils/coreutils/issues/15592) |
| [printf: `%e` and `%g` round an exact halfway value up instead of to even (`%.0e` of 2.5 gives `3e+00`)](coreutils-printf-scientific-rounds-exact-ties-half/) | open | [link](https://github.com/uutils/coreutils/issues/15662) |
| [printf: `%.*s` accepts a `*` precision above 2147483647 instead of failing with "invalid precision"](coreutils-printf-string-star-precision-above-int-m/) | fixed | [link](https://github.com/uutils/coreutils/issues/15663) |
| [seq: `-w` pads to the width of the increment, which is never printed](coreutils-seq-equal-width-counts-increment-digits/) | fixed | [link](https://github.com/uutils/coreutils/issues/15572) |
| [sort: global `-i` is not overridden by `-d`](coreutils-sort-global-i-not-overridden-by-d/) | fixed | [link](https://github.com/uutils/coreutils/issues/15578) |
| [sort: `-h` ignores `-f`, so lowercase suffixes such as `m`, `g`, `e` are not folded](coreutils-sort-human-numeric-ignores-f-fold/) | fixed | [link](https://github.com/uutils/coreutils/issues/15577) |
| [sort: a key's own `r` is ignored when the key is the whole line (`-k1r`)](coreutils-sort-key-own-reverse-ignored-on-whole-li/) | fixed | [link](https://github.com/uutils/coreutils/issues/15570) |
| [sort: `-b` and `-M` still skip leading form feed and carriage return (follow-up to #14119)](coreutils-sort-treats-form-feed-and-cr-as-blanks/) | fixed | [link](https://github.com/uutils/coreutils/issues/15588) |
| [sort: `-V` ignores `-d` and `-i` (follow-up to #15463)](coreutils-sort-version-ignores-i-d-f/) | open | [link](https://github.com/uutils/coreutils/issues/15587) |
| [split: the `-l` count is parsed like a byte SIZE (`-l 010` means 8 lines, `-l 1k` is accepted)](coreutils-split-lines-count-parsed-as-byte-size/) | open | [link](https://github.com/uutils/coreutils/issues/15660) |
| [split: `-a 0` with `-n N` uses a 1-character suffix instead of the default length 2](coreutils-split-suffix-length-zero-ignores-default/) | fixed | [link](https://github.com/uutils/coreutils/issues/15581) |
| [split: a suffix start value too large for the suffix length is accepted when input is empty (regression on main)](coreutils-split-suffix-start-too-large-accepted-on/) | fixed | [link](https://github.com/uutils/coreutils/issues/15569) |
| [split: `--verbose` prints nothing with `-n`/`--number`](coreutils-split-verbose-silent-in-number-mode/) | open | [link](https://github.com/uutils/coreutils/issues/15583) |
| [tr: `[c*]` accepted with `-d -s`, `[=c=]` accepted in string2 when translating, and a case class past the end of string1 rejected](coreutils-tr-string2-construct-validation-mismatch/) | partly fixed | [link](https://github.com/uutils/coreutils/issues/15589) |
| [expand/unexpand: a bare `-` operand is taken as an obsolete `-N` tab option, so standard input is skipped](coreutils-unexpand-dash-operand-taken-as-tab-short/) | fixed | [link](https://github.com/uutils/coreutils/issues/15658) |
| [unexpand: a line that continues from one FILE into the next is handled as two lines](coreutils-unexpand-line-state-reset-at-file-bounda/) | fixed | [link](https://github.com/uutils/coreutils/issues/15657) |
| [uniq: `-f` treats carriage return and form feed as field separators](coreutils-uniq-skip-fields-treats-cr-ff-as-blanks/) | fixed | [link](https://github.com/uutils/coreutils/issues/15659) |
