Run against an upstream brush build:

    ./repro.sh /path/to/brush /path/to/bash-5.3

Exit 1 means brush printed nothing for `echo before` (bug present); exit 0 means it printed `before` (fixed).
