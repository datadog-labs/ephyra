**Title:** chmod: modes like `-=r`, `-+x` and `-,u+x` are rejected as unknown options

### Describe the bug

`chmod` accepts a symbolic mode that starts with `-` when the next character is a permission letter or a digit (`chmod -w f`, `chmod -x f`). It rejects one where `-` is followed by another operator or by a comma, such as `-=r`, `-+x` or `-,u+x`: it treats these as unknown options and exits 1 without changing the file. GNU applies the mode.

The GNU `chmod --help` text gives the grammar (uutils prints the same line, with `[-+=]?[0-7]+` at the end):

> Each MODE is of the form '[ugoa]*([-+=]([rwxXst]*|[ugo]))+|[-+=][0-7]+'.

Under this grammar `-=r` is an empty `-` followed by `=r`, and `-,u+x` is two modes separated by a comma. The GNU manual (chmod invocation) says that a leading `-` without `--` works:

> If you really want mode to have a leading '-', you should use -- first, e.g., 'chmod -- -w file'. Typically, though, 'chmod a-w file' is preferable, and chmod -w file (without the --) complains if it behaves differently from what 'chmod a-w file' would do.

uutils already accepts these modes after `--` (`chmod -- -=r f` works); only the form without `--` fails. GNU coreutils 9.12, with `umask 022`:

```sh
touch f
chmod 644 f; chmod -=r f; echo "exit=$?"; stat -c %A f
# GNU 9.12: exit=0, -r--r--r--
# uutils:   error: unexpected argument '-=' found, exit=1, -rw-r--r--

chmod 644 f; chmod -+x f; echo "exit=$?"; stat -c %A f
# GNU 9.12: exit=0, -rwxr-xr-x
# uutils:   error: unexpected argument '-+' found, exit=1, -rw-r--r--

chmod 644 f; chmod -,u+x f; echo "exit=$?"; stat -c %A f
# GNU 9.12: exit=0, -rwxr--r--
# uutils:   error: unexpected argument '-,' found, exit=1, -rw-r--r--
```

**Versions.** uutils main 342b6f7bd10adee39809516f56d86425422cc9ec (2026-10-22) and the 0.12.0 release binary, macOS 26 aarch64, `LC_ALL=C`, compared with GNU coreutils 9.12 (also seen on Linux x86_64).

**Where.** `extract_negative_modes()` in `src/uu/chmod/src/chmod.rs` treats an argument as a mode only when its second character is one of `rwxXstugo` or `0`-`7`. Any other argument starting with `-` goes to clap, which rejects it.

Related: open PR #15378 ("mode: reject empty clauses and bare octal inside comma lists") changes the mode parser; I did not check how it handles these modes. #14301 (invalid-mode error text) is a different problem.

Found with an automated differential tester that compares uutils with GNU 9.12. I reproduced it by hand on main and on 0.12.0.

### Reproduction steps

```sh
export LC_ALL=C
umask 022
cd "$(mktemp -d)"

touch f; chmod 644 f
chmod -=r f; echo "exit=$?"; stat -c %A f
# GNU 9.12: exit=0, -r--r--r--
# uutils:   error: unexpected argument '-=' found, exit=1, -rw-r--r--
```

I have a patch with tests and can open a PR.
