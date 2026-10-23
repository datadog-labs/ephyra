**Title:** dd: `conv=swab` pairs bytes within each input block, so the result depends on `bs=` and on how a pipe delivers data

### Describe the bug

`dd conv=swab` swaps byte pairs inside each input block separately. When a block has an odd length, its last byte is never paired with the first byte of the next block, and every later pair is off by one. The output then depends on `bs=`/`ibs=`, and for a pipe on how the data happened to arrive.

The GNU manual (dd invocation) lists the steps dd takes for each block:

> If the `swab` conversion is given, swap each pair of input bytes. If the input data length is odd, preserve the last input byte (since there is nothing to swap it with).
>
> If any of the conversions `swab`, `block`, `unblock`, `lcase`, `ucase`, `ascii`, `ebcdic` and `ibm` are given, do these conversions. These conversions operate independently of input blocking, and might deal with records that span block boundaries.

GNU coreutils 9.12:

```sh
printf abcdefg | dd conv=swab bs=3 status=none
# GNU 9.12: badcfeg
# uutils:   bacedfg

printf ab | dd conv=swab bs=1 status=none
# GNU 9.12: ba
# uutils:   ab

(printf abc; sleep 0.3; printf defg) | dd conv=swab status=none
# GNU 9.12: badcfeg
# uutils:   bacedgf
```

With the default block size and a regular file the result is right (`printf abcdefg | dd conv=swab status=none` prints `badcfeg` in both), so the third case is the one most likely to bite: the same bytes give a different result depending on pipe timing.

**Versions.** uutils main 342b6f7bd10adee39809516f56d86425422cc9ec (2026-10-22) and the 0.12.0 release binary, macOS 26 aarch64, `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `read_helper` in `src/uu/dd/src/dd.rs` calls `perform_swab` on each block's buffer, and no unpaired byte is carried over to the next read.

Related: none found. #13458 (closed, short reads with `obs > ibs`) is also about block boundaries but is a different problem.

Found with an automated differential tester that compares uutils with GNU 9.12. I reproduced it by hand on main and on 0.12.0.

### Reproduction steps

```sh
export LC_ALL=C

printf abcdefg | dd conv=swab bs=3 status=none; echo
# GNU 9.12: badcfeg
# uutils:   bacedfg
```
