repro.sh <numfmt-command>   standalone; e.g. `repro.sh ./bin/numfmt` or `repro.sh "target/debug/coreutils numfmt"`.
Exit 1 = bug reproduces, 0 = matches GNU 9.12 on all five cases.
harness-minimal/ is the loop's minimal case (`repro.sh <build-dir>` expects <build-dir>/bin/numfmt).
