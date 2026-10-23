**Title:** sort: global `-i` is not overridden by `-d`

### Describe the bug

`-i` should have no effect when `-d` is also given. uutils sort follows this for key options (`-k1,1di`) but not for the global options `-d -i`, `-i -d` or `-di`: it applies both filters, so it drops tabs and other control characters that `-d` keeps.

The GNU manual (sort invocation) says:

> `-i`: Ignore nonprinting characters. The LC_CTYPE locale determines character types. This option has no effect if the stronger --dictionary-order (-d) option is also given.

```sh
printf '\tK\n7\n' | sort -d -i | od -An -c
# GNU 9.12:  \t   K  \n   7  \n      (same as -d alone: the tab is a blank and is kept)
# uutils:     7  \n  \t   K  \n      (same as -i alone: the tab is dropped)

printf '\tK\n7\n' | sort -k1,1di | od -An -c
# GNU 9.12:  \t   K  \n   7  \n
# uutils:    \t   K  \n   7  \n      (key options: correct)
```

**Versions.** uutils main eee56033d28bf7ef5cc816286b048779d57fb803 (2026-10-21) and the 0.12.0 release binary, macOS 26 aarch64 (the loop found it on Linux x86_64), `LC_ALL=C`, compared with GNU coreutils 9.12.

**Where.** `uumain` in `src/uu/sort/src/sort.rs` copies both global flags into `GlobalSettings` as given (`settings.dictionary_order = ...; settings.ignore_non_printing = ...;`). The key-option parser already clears `ignore_non_printing` when `d` is set.

Found with an automated differential tester that compares uutils with GNU 9.12.

I have a patch with tests and can open a PR.

### Reproduction steps

```sh
export LC_ALL=C
printf '\tK\n7\n' | sort -d -i | od -An -c
# GNU 9.12:  \t   K  \n   7  \n
# uutils:     7  \n  \t   K  \n
```
