**Title (ada-url/idna):** combining-mark, bidi and normalization tables predate the Unicode 17 mapping table: leading marks and RTL letters from Unicode 14+ pass, Unicode 16 compositions are not applied

`scripts/pack_tables.py` says: "Normalization and bidi/combining data have no generator in this repo; they are preserved across regenerations by reading the existing table_blob.inc." The mapping table has been regenerated up to Unicode 17, but those tables have not, and they no longer agree with it. ada (and so Node.js) takes these tables from this repository.

**1. Labels may start with a combining mark added in Unicode 14 or later.** UTS 46 rejects a label that begins with a General_Category=Mark code point. The `combining_flat` table holds 290 ranges and lacks the marks added since (U+0897, U+08CA–U+08D2, U+0C3C, U+0CF3, U+0ECE, U+1715, U+1AC1–U+1ADD, U+1AE0–U+1AE7 and more). The mapping table knows them as valid, so they pass:

```js
// Node.js 26.10 (ada 4.0.0)
new URL("http://́a/")   // TypeError: old mark, rejected
new URL("http://࣋a/")   // "http://xn--a-lqd/": Unicode 14 mark, accepted
new URL("http://ࢗa/")   // "http://xn--a-mnd/": Unicode 16 mark, accepted
```

Checking every one of the 2543 Mark code points of Unicode 17 in `http://<mark>a/` against whatwg-url 17.2.0 (tr46 7.0.0): ada accepts 247 that the standard rejects (`leading_marks.rs`). Leading combining marks are excluded because they attach visually to what precedes them, which makes them a spoofing aid.

**2. Unicode 16 canonical compositions are not applied.** Of the 26,506 strings formed by every decomposable code point and its NFD, 20 give a different host in ada than in whatwg-url (`normalization.rs`): all are compositions added in Unicode 16 (Todhri U+105D2/U+105DA, Tulu-Tigalari U+113xx, Gurung Khema U+1611E…, Kirat Rai U+16D6x). For example `http://a\u{1611E}\u{1611E}/` is `xn--a-ln1ma` in ada (not in NFC) and `xn--a-rn1m` (the composed U+16121) in the standard. Two spellings of one name give two hosts.

**3. RTL characters added in Unicode 14 or later are not RTL to the Bidi rule.** The `dir_*` table predates them, so a label with such a character is not an RTL label, and `http://<c>a/` (an RTL character, then the L character `a`) passes where the standard rejects it. Of the 3121 code points of Bidi class R, AL or AN in Unicode 17, 154 differ (`rtl_labels.rs`): Arabic Extended-B U+0870–U+088F, U+08B5, U+08C8–U+08C9, U+061D, U+FBC2 and Old Uyghur U+10F70–U+10F89 (Unicode 14); Garay U+10D40–U+10D8F and Arabic Extended-C U+10EC2–U+10EC7 (Unicode 16); Sidetic U+10940–U+10959 (Unicode 17). Node.js: `new URL("http://\u{10940}a/")` is `http://xn--a-6w4i/`.

**Fix.** Generate the validity tables from the UCD of the same version as the mapping table. `idna-combining-bidi-unicode-17.patch` does it for Unicode 17 with `pack_tables.load_blob_sections`/`write_blob`: `combining_flat` from `DerivedGeneralCategory.txt` (Mn, Mc, Me; 290 → 327 ranges) and `dir_start`/`dir_final`/`dir_value` from `DerivedBidiClass.txt` (1449 → 1611 ranges). Afterwards all 2543 marks and all 3121 RTL code points agree with whatwg-url; the patch applies to idna main (92ea246) and its 60 tests pass, and ada main (4eee4c2) built with the patched idna passes its 403 tests. The 20 normalization differences remain, as described below. The patch only touches the blob; the generator belongs in `scripts/` next to `derived_table.py` (the snippet is short and I can contribute it). The normalization tables (decomposition, CCC, composition, exclusions) need a generator from `UnicodeData.txt` and `CompositionExclusions.txt`, which this patch does not include.

Found by a differential of generated URLs between ada and whatwg-url, then confirmed by exhaustive sweeps over the affected code points.
