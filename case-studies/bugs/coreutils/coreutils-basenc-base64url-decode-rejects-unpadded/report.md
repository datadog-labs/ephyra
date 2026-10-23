**Title:** basenc: `--base64url -d` rejects input whose last group has no `=` padding

### Describe the bug

`basenc --base64url -d` fails with `invalid input` when the last group of the input is not padded with `=`. `basenc --base64 -d` accepts the same unpadded input, and GNU coreutils 9.12 decodes it with both options.

The GNU manual (basenc invocation) describes `--base64url` as base64 with a different alphabet:

> `--base64url`: Encode into (or decode from with `-d/--decode`) file-and-url-safe base64 form (using `_` and `-` instead of `+` and `/`).

and says that "The `-w/--wrap`, `-i/--ignore-garbage`, `-d/--decode` options of this command are precisely the same as for `base64`." GNU coreutils 9.12:

```sh
printf Pz8 | basenc --base64url -d
# GNU 9.12: ??                          (exit 0)
# uutils:   basenc: invalid input       (exit 1)

printf Pz8 | basenc --base64 -d
# GNU 9.12: ??                          (exit 0)
# uutils:   ??                          (exit 0)

printf Pz8=Pz8 | basenc --base64url -d
# GNU 9.12: ????                        (exit 0)
# uutils:   basenc: invalid input       (exit 1)
```

Padded input (`Pz8=`) works. Every base64url string without padding fails, so base64url data as it is usually written (for example JWT segments, which never carry `=`) cannot be decoded.

**Versions.** uutils main 342b6f7bd10adee39809516f56d86425422cc9ec (2026-10-22) and the 0.12.0 release binary (which prints `basenc: error: invalid input`), macOS 26 aarch64, `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** In `src/uu/base32/src/base_common.rs`, `Format::Base64` decodes through `Base64SimdWrapper`, which accepts an unpadded tail, while `Format::Base64Url` uses `EncodingWrapper` with the padded `BASE64URL` encoding.

Related: #12204 (an inner `=` in base64/base32 input) may matter for the third case; #5698 (basenc tracking issue). Open PR #15485 ("basenc: preserve decoded prefixes on invalid input", for #6008) touches unpadded base64url tails, but with it (head 93eff7580) `printf Pw | basenc --base64url -d` still prints `?` followed by `basenc: error: invalid input` and exits 1.

Found with an automated differential tester that compares uutils with GNU 9.12. I reproduced it by hand on main and on 0.12.0.

### Reproduction steps

```sh
export LC_ALL=C

printf Pw | basenc --base64url -d; echo " exit=$?"
# GNU 9.12: ? exit=0
# uutils:   basenc: invalid input, exit=1

printf Pw | basenc --base64 -d; echo " exit=$?"
# GNU 9.12: ? exit=0
# uutils:   ? exit=0
```
