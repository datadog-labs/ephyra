# Repro

    ./repro.sh /path/to/brush

Runs `printf '%d\n' B 2>/dev/null; echo rc=$?` and `f() { printf '%x\n' 12abc; }; f 2>/dev/null` under brush.
Exit 1: the bug reproduces (a diagnostic reaches brush's stderr despite the redirection, or rc is not 1).
Exit 0: brush behaves like bash 5.3. Exit 2: bad arguments.
Results: brush 7d4c4da exits 1; 7d4c4da + fix.diff exits 0; bash 5.3.20 (`./repro.sh /path/to/bash`) exits 0.
