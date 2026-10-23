**Title:** chmod --reference: the first FILE operand is processed last

### Describe the bug

With `--reference=REF_FILE`, uutils `chmod` processes the first FILE operand after all the others. Without `--reference` the order is right. GNU coreutils 9.12 processes the operands in command-line order in both cases.

The GNU manual (chmod invocation) gives the synopsis `chmod [OPTION]... {MODE | --reference=REF_FILE} FILE...` and says `--reference` will "Change the mode of each FILE to be the same as that of REF_FILE." So with `--reference` every operand is a FILE, and they should be handled like the FILEs after a MODE.

The order shows up in `-v`/`-c` output. It also changes the result when a directory is listed after a file inside it: uutils changes the directory first, takes away its search (x) permission, and then cannot reach the file. GNU coreutils 9.12:

```sh
touch ref a b; chmod 644 ref; chmod 600 a b
chmod -v --reference=ref a b
# GNU 9.12: mode of 'a' changed from 0600 (rw-------) to 0644 (rw-r--r--)
#           mode of 'b' changed from 0600 (rw-------) to 0644 (rw-r--r--)
# uutils:   mode of 'b' changed from 0600 (rw-------) to 0644 (rw-r--r--)
#           mode of 'a' changed from 0600 (rw-------) to 0644 (rw-r--r--)

mkdir dir; touch dir/file; chmod 755 dir
chmod --reference=ref dir/file dir; echo "exit=$?"
# GNU 9.12: exit=0
# uutils:   chmod: cannot access 'dir/file': Permission denied, exit=1
```

With three operands `a b c`, uutils reports `b`, `c`, `a`. `chmod -v 644 a b` (no `--reference`) reports `a` then `b`, as GNU does. The second example needs a non-root user.

**Versions.** uutils main 342b6f7bd10adee39809516f56d86425422cc9ec (2026-10-22) and the 0.12.0 release binary, macOS 26 aarch64, `LC_ALL=C`, compared with GNU coreutils 9.12 (also seen on Linux x86_64).

**Where.** `uumain()` in `src/uu/chmod/src/chmod.rs`: when `--reference` is given, clap has parsed the first FILE as MODE, and the code appends it to the end of the file list (`files.push(...)`) instead of putting it first.

Related: none found. #9790 (closed) was about chmod error handling with several files, not about order.

Found with an automated differential tester that compares uutils with GNU 9.12. I reproduced it by hand on main and on 0.12.0.

### Reproduction steps

```sh
export LC_ALL=C
cd "$(mktemp -d)"

touch ref a b; chmod 644 ref; chmod 600 a b
chmod -v --reference=ref a b
# GNU 9.12: 'a' first, then 'b'
# uutils:   'b' first, then 'a'
```

I have a patch with tests and can open a PR.
