Run: `./repro.sh /path/to/brush [/path/to/bash-5.3]`. It runs a small script under `env -i` (so OLDPWD is not
inherited), prints brush's output (and bash's, if given) next to the expected bash 5.3 output, then exits 1 if
the bug reproduces and 0 if it does not.
